//! The market end to end: pools, batch swaps, liquidity, claims, launches.
//!
//! Every intent goes through `apply` on a real ledger, and after every one the
//! books are checked two ways — live objects plus what the market holds equal
//! issued supply (`conserved`), and every object ever created is either live
//! or retired (`reconciled`). A market that moved value correctly in one test
//! and leaked it in the next would pass a suite that only checked outcomes.

use objectvm::market::{self, LaunchState};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x10; 32];
const USD: [u8; 32] = [0xF0; 32];
const ALICE: AccountId = [0xA1; 32];
const BOB: AccountId = [0xB0; 32];
const CAROL: AccountId = [0xC0; 32];

struct Chain {
    vm: ObjectVm,
    seq: u64,
    nonce: u8,
    salt: u64,
}

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

impl Chain {
    fn new() -> Chain {
        Chain {
            vm: ObjectVm::genesis(212, Limits { max_io: 16 }),
            seq: 0,
            nonce: 0,
            salt: 0,
        }
    }

    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let r = self.vm.apply(self.seq, &op);
        assert_eq!(self.vm.conserved(), Ok(()), "after {op:?}");
        assert_eq!(self.vm.reconciled(), Ok(()));
        // Every state must survive its own encoding: this is what a restart
        // or a replica reads back.
        let back = ObjectVm::decode(&self.vm.encode()).expect("state decodes");
        assert_eq!(back, self.vm);
        // And every intent its own wire format: this is what the journal holds.
        let wire = ObjectVm::encode_intent(&op);
        let mut d = zyn_vm::read::Decoder::new(&wire);
        assert_eq!(ObjectVm::decode_intent(&mut d).as_ref(), Some(&op));
        r.into_iter().next().unwrap()
    }

    fn ok(&mut self, op: Op) -> Receipt {
        let r = self.apply(op);
        assert!(!ObjectVm::rejected(&r), "{r:?}");
        r
    }

    fn seal(&mut self) {
        let r = self.apply(Op::Seal);
        assert!(matches!(r, Receipt::Sealed(_)));
    }

    fn fresh(&mut self) -> Hash {
        self.nonce = self.nonce.wrapping_add(1);
        self.salt += 1;
        let mut h = [self.nonce; 32];
        h[..8].copy_from_slice(&self.salt.to_be_bytes());
        h
    }

    fn mint(&mut self, owner: AccountId, asset: [u8; 32], whole: i64) {
        let nonce = self.fresh();
        self.ok(Op::Mint {
            asset,
            amount: Fixed::whole(whole),
            owner,
            nonce,
        });
    }

    fn note(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> Output {
        Output {
            note: Note {
                asset,
                amount,
                owner,
                nonce: self.fresh(),
            },
            enc: enc(),
        }
    }

    fn balance(&self, owner: AccountId, asset: [u8; 32]) -> Fixed {
        Fixed(
            self.vm
                .live
                .values()
                .filter(|n| n.owner == owner && n.asset == asset)
                .map(|n| n.amount.0)
                .sum(),
        )
    }

    /// Enough of `owner`'s `asset` to cover `amount`, and change for the rest.
    fn fund(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> (Vec<(Hash, Hash)>, Vec<Output>) {
        let mut picked = Vec::new();
        let mut sum = 0i128;
        let notes: Vec<(Hash, Note)> = self.vm.live.iter().map(|(c, n)| (*c, *n)).collect();
        for (c, n) in notes {
            if sum >= amount.0 {
                break;
            }
            if n.owner == owner && n.asset == asset && !self.vm.offers.contains_key(&c) {
                picked.push((c, nullifier(&[owner[0]; 32], &c)));
                sum += n.amount.0;
            }
        }
        assert!(sum >= amount.0, "{owner:?} holds too little");
        let change = if sum > amount.0 {
            vec![self.note(owner, asset, Fixed(sum - amount.0))]
        } else {
            Vec::new()
        };
        (picked, change)
    }

    /// Redeem every claim `owner` can derive from these sources.
    fn claim(&mut self, owner: AccountId, sources: &[Hash]) {
        let mut ids = Vec::new();
        let mut outputs = Vec::new();
        for s in sources {
            for k in 0..3 {
                let id = market::claim_id(s, k);
                if let Some(c) = self.vm.market.claims.get(&id).copied() {
                    assert_eq!(c.owner, owner, "a derived claim is somebody else's");
                    ids.push(id);
                    outputs.push(self.note(owner, c.asset, c.amount));
                }
            }
        }
        assert!(!ids.is_empty(), "nothing to claim");
        self.ok(Op::Claim {
            owner,
            program: program::claim(),
            claims: ids,
            outputs,
        });
    }

    fn create_pool(&mut self, who: AccountId, a: i64, b: i64, fee: u32) -> (Hash, Hash) {
        let (inputs_a, mut change) = self.fund(who, ZEC, Fixed::whole(a));
        let (inputs_b, change_b) = self.fund(who, USD, Fixed::whole(b));
        change.extend(change_b);
        let nonce = self.fresh();
        let r = self.ok(Op::CreatePool {
            provider: who,
            program: program::add_liquidity(),
            asset_a: ZEC,
            asset_b: USD,
            amount_a: Fixed::whole(a),
            amount_b: Fixed::whole(b),
            fee_bps: fee,
            inputs: [inputs_a, inputs_b].concat(),
            change,
            nonce,
        });
        match r {
            Receipt::Pooled { pool, source, .. } => (pool, source),
            other => panic!("{other:?}"),
        }
    }

    fn swap(&mut self, who: AccountId, pool: Hash, sell_a: bool, amount: Fixed, min_out: Fixed) -> Hash {
        let asset = if sell_a { ZEC } else { USD };
        let (inputs, change) = self.fund(who, asset, amount);
        let nonce = self.fresh();
        match self.ok(Op::Swap {
            trader: who,
            program: program::amm_swap(),
            pool,
            sell_a,
            amount_in: amount,
            min_out,
            inputs,
            change,
            nonce,
        }) {
            Receipt::Ordered { order } => {
                assert_eq!(order, market::order_id(&who, &nonce));
                order
            }
            other => panic!("{other:?}"),
        }
    }
}

fn product(p: &market::Pool) -> (u128, u128) {
    let (a, b) = (p.reserve_a.0 as u128, p.reserve_b.0 as u128);
    // Compare as f64 is not exact enough; compare as a 256-bit pair.
    let (a1, a0) = (a >> 64, a & u64::MAX as u128);
    let (b1, b0) = (b >> 64, b & u64::MAX as u128);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let mid = (p00 >> 64) + (p01 & u64::MAX as u128) + (p10 & u64::MAX as u128);
    (a1 * b1 + (p01 >> 64) + (p10 >> 64) + (mid >> 64), (p00 & u64::MAX as u128) | (mid << 64))
}

/// A chain that never touches the market keeps the roots it already
/// anchored: no new section, no new bytes in its saved state.
#[test]
fn a_chain_without_a_market_keeps_its_roots() {
    let mut c = Chain::new();
    c.mint(ALICE, ZEC, 5);
    assert_eq!(c.vm.sections().len(), 7);
    let saved = c.vm.encode();
    assert!(!saved.windows(8).any(|w| w == b"MARKET01"));
    c.seal();
    assert_eq!(c.vm.sections().len(), 7);
}

#[test]
fn a_pool_opens_and_its_shares_are_claimable() {
    let mut c = Chain::new();
    c.mint(ALICE, ZEC, 100);
    c.mint(ALICE, USD, 400);
    let (pool, source) = c.create_pool(ALICE, 100, 400, 30);
    assert_eq!(c.vm.sections().len(), 8, "the market joins the root once used");
    let p = c.vm.market.pools[&pool];
    assert_eq!(p.shares, Fixed::whole(200), "√(100·400)");
    assert_eq!(p.locked, market::MINIMUM_LIQUIDITY);
    assert_eq!(c.balance(ALICE, ZEC), Fixed::ZERO);

    c.claim(ALICE, &[source]);
    let lp = market::lp_asset(&pool);
    assert_eq!(c.balance(ALICE, lp), Fixed(Fixed::whole(200).0 - market::MINIMUM_LIQUIDITY.0));
}

/// The swap flow, seen from the trader: escrow now, price at the seal, the
/// proceeds a claim only they can find.
#[test]
fn a_swap_clears_at_the_seal_and_pays_by_claim() {
    let mut c = Chain::new();
    c.mint(ALICE, ZEC, 1000);
    c.mint(ALICE, USD, 1000);
    c.mint(BOB, ZEC, 100);
    let (pool, _) = c.create_pool(ALICE, 1000, 1000, 0);
    let before = product(&c.vm.market.pools[&pool]);

    let order = c.swap(BOB, pool, true, Fixed::whole(100), Fixed::whole(80));
    assert_eq!(c.balance(BOB, ZEC), Fixed::ZERO, "escrowed at once");
    assert_eq!(c.vm.market.pools[&pool].reserve_a, Fixed::whole(1000), "priced only at the seal");

    c.seal();
    assert!(c.vm.market.orders.is_empty());
    let after = product(&c.vm.market.pools[&pool]);
    assert!(after >= before, "the pool never loses value");

    c.claim(BOB, &[order]);
    assert_eq!(c.balance(BOB, USD), Fixed(83_333_333_333_333_333_333));
}

/// A price worse than the trader accepted refunds them in full, and the
/// batch clears for everybody else as if they had not been there.
#[test]
fn an_order_whose_minimum_is_not_met_is_refunded() {
    let mut c = Chain::new();
    c.mint(ALICE, ZEC, 1000);
    c.mint(ALICE, USD, 1000);
    c.mint(BOB, ZEC, 100);
    c.mint(CAROL, USD, 50);
    let (pool, _) = c.create_pool(ALICE, 1000, 1000, 30);
    let greedy = c.swap(BOB, pool, true, Fixed::whole(100), Fixed::whole(99));
    let fair = c.swap(CAROL, pool, false, Fixed::whole(50), Fixed::ZERO);
    c.seal();

    c.claim(BOB, &[greedy]);
    assert_eq!(c.balance(BOB, ZEC), Fixed::whole(100), "refunded whole");
    assert_eq!(c.balance(BOB, USD), Fixed::ZERO);
    c.claim(CAROL, &[fair]);
    assert!(c.balance(CAROL, ZEC).0 > 0);
}

/// Somebody else's claim cannot be redeemed, a claim cannot be redeemed
/// twice, and a redemption cannot be worth more than the claim.
#[test]
fn a_claim_pays_its_owner_once_and_exactly() {
    let mut c = Chain::new();
    c.mint(ALICE, ZEC, 1000);
    c.mint(ALICE, USD, 1000);
    c.mint(BOB, ZEC, 10);
    let (pool, _) = c.create_pool(ALICE, 1000, 1000, 0);
    let order = c.swap(BOB, pool, true, Fixed::whole(10), Fixed::ZERO);
    c.seal();
    let id = market::claim_id(&order, 0);
    let owed = c.vm.market.claims[&id];

    let thief = c.note(CAROL, owed.asset, owed.amount);
    let r = c.apply(Op::Claim { owner: CAROL, program: program::claim(), claims: vec![id], outputs: vec![thief] });
    assert!(ObjectVm::rejected(&r));

    let greedy = c.note(BOB, owed.asset, Fixed(owed.amount.0 + 1));
    let r = c.apply(Op::Claim { owner: BOB, program: program::claim(), claims: vec![id], outputs: vec![greedy] });
    assert!(ObjectVm::rejected(&r));

    c.claim(BOB, &[order]);
    let again = c.note(BOB, owed.asset, owed.amount);
    let r = c.apply(Op::Claim { owner: BOB, program: program::claim(), claims: vec![id], outputs: vec![again] });
    assert!(ObjectVm::rejected(&r));
}

#[test]
fn liquidity_goes_in_at_the_ratio_and_its_share_of_the_invariant_never_falls() {
    let mut c = Chain::new();
    c.mint(ALICE, ZEC, 1000);
    c.mint(ALICE, USD, 4000);
    c.mint(BOB, ZEC, 200);
    c.mint(BOB, USD, 200);
    c.mint(CAROL, ZEC, 500);
    let (pool, alice_src) = c.create_pool(ALICE, 1000, 4000, 100);
    c.claim(ALICE, &[alice_src]);

    // Bob offers 200/200 at a 1:4 pool: 50 ZEC is all that fits; the rest
    // comes back.
    let (ia, mut ch) = c.fund(BOB, ZEC, Fixed::whole(200));
    let (ib, chb) = c.fund(BOB, USD, Fixed::whole(200));
    ch.extend(chb);
    let nonce = c.fresh();
    let r = c.ok(Op::AddLiquidity {
        provider: BOB,
        program: program::add_liquidity(),
        pool,
        amount_a: Fixed::whole(200),
        amount_b: Fixed::whole(200),
        min_shares: Fixed::whole(1),
        inputs: [ia, ib].concat(),
        change: ch,
        nonce,
    });
    let Receipt::Pooled { source: bob_src, .. } = r else { panic!() };
    c.claim(BOB, &[bob_src]);
    assert_eq!(c.balance(BOB, ZEC), Fixed::whole(150));
    assert_eq!(c.balance(BOB, USD), Fixed::ZERO);

    // Trading earns the pool its fee.
    c.swap(CAROL, pool, true, Fixed::whole(500), Fixed::ZERO);
    c.seal();

    // Alice leaves with more value than she brought, at the new price.
    let lp = market::lp_asset(&pool);
    let shares = c.balance(ALICE, lp);
    let (inputs, change) = c.fund(ALICE, lp, shares);
    let nonce = c.fresh();
    let r = c.ok(Op::RemoveLiquidity {
        provider: ALICE,
        program: program::remove_liquidity(),
        pool,
        shares,
        min_a: Fixed::ZERO,
        min_b: Fixed::ZERO,
        inputs,
        change,
        nonce,
    });
    let Receipt::Unpooled { source, .. } = r else { panic!() };
    c.claim(ALICE, &[source]);
    // What a share is worth in the pool's invariant never falls: fees and
    // the batch price both only add to √(x·y). She put in 1000·4000 for her
    // shares and takes out at least that product.
    let (a, b) = (c.balance(ALICE, ZEC).0 as f64 / 1e18, c.balance(ALICE, USD).0 as f64 / 1e18);
    assert!(a * b >= 1000.0 * 4000.0 * 0.999_999, "{a} × {b}");
    assert_eq!(c.balance(ALICE, lp), Fixed::ZERO);
    let p = c.vm.market.pools[&pool];
    // …and the locked minimum stays behind for good.
    assert!(p.shares >= market::MINIMUM_LIQUIDITY);
}

/// Pool shares and launched tokens exist only here. Letting one "leave for
/// Zcash" would record a debt the vault holds nothing to pay.
#[test]
fn market_assets_cannot_leave_for_zcash() {
    let mut c = Chain::new();
    c.mint(ALICE, ZEC, 100);
    c.mint(ALICE, USD, 100);
    let (pool, src) = c.create_pool(ALICE, 100, 100, 0);
    c.claim(ALICE, &[src]);
    let lp = market::lp_asset(&pool);
    let (inputs, _) = c.fund(ALICE, lp, Fixed::whole(1));
    let all = c.balance(ALICE, lp);
    let change = vec![c.note(ALICE, lp, Fixed(all.0 - Fixed::whole(1).0))];
    let r = c.apply(Op::Withdraw {
        holder: ALICE,
        program: program::withdraw(),
        inputs,
        asset: lp,
        amount: Fixed::whole(1),
        destination: [9; 32],
        outputs: change,
    });
    assert_eq!(r, Receipt::Refused("only deposited assets leave for Zcash"));
}

fn launch(c: &mut Chain, creator: AccountId, soft: i64, hard: i64, pool_bps: u32) -> (Hash, Hash) {
    let nonce = c.fresh();
    let closes = c.vm.epoch + 1;
    let r = c.ok(Op::CreateLaunch {
        creator,
        program: program::launch_create(),
        nonce,
        quote: ZEC,
        total_supply: Fixed::whole(1_000_000),
        sale_supply: Fixed::whole(600_000),
        pool_supply: if pool_bps > 0 { Fixed::whole(200_000) } else { Fixed::ZERO },
        soft_cap: Fixed::whole(soft),
        hard_cap: Fixed::whole(hard),
        pool_bps,
        fee_bps: 30,
        closes_at_epoch: closes,
        meta: [0x4d; 32],
    });
    let Receipt::Launched { token } = r else { panic!() };
    assert_eq!(token, market::token_of(&creator, &nonce));
    (token, market::launch_source(&creator, &nonce))
}

fn commit(c: &mut Chain, buyer: AccountId, token: Hash, whole: i64) -> Hash {
    let (inputs, change) = c.fund(buyer, ZEC, Fixed::whole(whole));
    let nonce = c.fresh();
    let r = c.ok(Op::Commit {
        buyer,
        program: program::launch_commit(),
        launch: token,
        amount: Fixed::whole(whole),
        inputs,
        change,
        nonce,
    });
    let Receipt::Committed { commit } = r else { panic!() };
    commit
}

/// Oversubscribed two to one: everyone pays the same price, gets half their
/// commitment's worth, and half back. The pool is seeded and nobody can pull
/// its liquidity.
#[test]
fn an_oversubscribed_sale_allocates_pro_rata_at_one_price() {
    let mut c = Chain::new();
    c.mint(BOB, ZEC, 300);
    c.mint(CAROL, ZEC, 100);
    let (token, creator_src) = launch(&mut c, ALICE, 50, 200, 5000);
    let b = commit(&mut c, BOB, token, 300);
    let k = commit(&mut c, CAROL, token, 100);
    assert_eq!(c.vm.market.launches[&token].raised, Fixed::whole(400));

    c.seal(); // closes_at_epoch not reached: still open
    assert_eq!(c.vm.market.launches[&token].state, LaunchState::Open);
    c.seal();
    assert_eq!(c.vm.market.launches[&token].state, LaunchState::Graduated);

    c.claim(BOB, &[b]);
    c.claim(CAROL, &[k]);
    assert_eq!(c.balance(BOB, token), Fixed::whole(450_000));
    assert_eq!(c.balance(CAROL, token), Fixed::whole(150_000));
    assert_eq!(c.balance(BOB, ZEC), Fixed::whole(150));
    assert_eq!(c.balance(CAROL, ZEC), Fixed::whole(50));

    // Half the 200 accepted seeds the pool, half goes to the creator, with
    // the creator's 200k tokens.
    c.claim(ALICE, &[creator_src]);
    assert_eq!(c.balance(ALICE, ZEC), Fixed::whole(100));
    assert_eq!(c.balance(ALICE, token), Fixed::whole(200_000));
    let pool = c.vm.market.pools[&market::pool_id(&token, &ZEC)];
    assert_eq!(pool.locked, pool.shares, "every share locked");
    assert_eq!(c.balance(ALICE, market::lp_asset(&market::pool_id(&token, &ZEC))), Fixed::ZERO);

    // And it trades.
    let pid = market::pool_id(&token, &ZEC);
    let sell_a = pool.a == token;
    let (inputs, change) = c.fund(BOB, token, Fixed::whole(1000));
    let nonce = c.fresh();
    c.ok(Op::Swap {
        trader: BOB,
        program: program::amm_swap(),
        pool: pid,
        sell_a,
        amount_in: Fixed::whole(1000),
        min_out: Fixed::ZERO,
        inputs,
        change,
        nonce,
    });
    c.seal();
    c.claim(BOB, &[market::order_id(&BOB, &nonce)]);
    assert!(c.balance(BOB, ZEC) > Fixed::whole(150));
}

/// Under the soft cap nothing happened: every commitment back, and the token
/// burned — including the creator's share of it.
#[test]
fn a_sale_under_its_soft_cap_refunds_everyone_and_burns_the_token() {
    let mut c = Chain::new();
    c.mint(BOB, ZEC, 30);
    let (token, creator_src) = launch(&mut c, ALICE, 50, 200, 0);
    let b = commit(&mut c, BOB, token, 30);
    c.seal();
    c.seal();
    assert_eq!(c.vm.market.launches[&token].state, LaunchState::Failed);
    c.claim(BOB, &[b]);
    assert_eq!(c.balance(BOB, ZEC), Fixed::whole(30));
    assert_eq!(c.vm.issued[&token], Fixed::ZERO);
    assert!((0..3).all(|k| !c.vm.market.claims.contains_key(&market::claim_id(&creator_src, k))));
}

/// A sale takes commitments only while it is open, and a token cannot be
/// pooled before its sale has decided what it is worth.
#[test]
fn a_sale_takes_commitments_only_while_open() {
    let mut c = Chain::new();
    c.mint(BOB, ZEC, 30);
    c.mint(ALICE, ZEC, 10);
    let (token, _) = launch(&mut c, ALICE, 0, 200, 0);
    c.seal();
    c.seal();
    let (inputs, change) = c.fund(BOB, ZEC, Fixed::whole(1));
    let nonce = c.fresh();
    let r = c.apply(Op::Commit {
        buyer: BOB,
        program: program::launch_commit(),
        launch: token,
        amount: Fixed::whole(1),
        inputs,
        change,
        nonce,
    });
    assert_eq!(r, Receipt::Refused("that sale is closed"));
}

/// Who traded is committed, so a replica catches a sequencer that swapped
/// one trader for another — but only through leaves blinded by the trader's
/// nonce, which is what `Market::leaf` keeps out of the clear.
#[test]
fn the_market_section_does_not_carry_traders_in_the_clear() {
    let root_for = |who: AccountId| {
        let mut c = Chain::new();
        c.mint(ALICE, ZEC, 100);
        c.mint(ALICE, USD, 100);
        c.mint(who, ZEC, 5);
        let (pool, _) = c.create_pool(ALICE, 100, 100, 0);
        c.swap(who, pool, true, Fixed::whole(5), Fixed::ZERO);
        c.vm.market.leaf()
    };
    assert_ne!(root_for(BOB), root_for(CAROL));
}
