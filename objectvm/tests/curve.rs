//! Bonding-curve launches end to end (DECISIONS §141): make a token, buy and
//! sell it on its curve in sealed batches, graduate it into a locked pool,
//! pay the creator — on a real ledger, with the books checked after every
//! intent.

use objectvm::curve::{self, cost, curve_order_id, curve_source, GRADUATE_AT, LOT, LOTS, POOL_FEE_BPS};
use objectvm::market::{self, token_of};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x10; 32];
const ALICE: AccountId = [0xA1; 32];
const BOB: AccountId = [0xB0; 32];
const CAROL: AccountId = [0xC0; 32];
/// 0.01 of the quote asset.
const CENT: Fixed = Fixed(10_000_000_000_000_000);

struct Chain {
    vm: ObjectVm,
    seq: u64,
    salt: u64,
}

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

impl Chain {
    fn new() -> Chain {
        Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, salt: 0 }
    }

    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let r = self.vm.apply(self.seq, &op);
        assert_eq!(self.vm.conserved(), Ok(()), "after {op:?}");
        assert_eq!(self.vm.reconciled(), Ok(()));
        assert_eq!(ObjectVm::decode(&self.vm.encode()).as_ref(), Some(&self.vm), "state round-trips");
        let wire = ObjectVm::encode_intent(&op);
        assert_eq!(ObjectVm::decode_intent(&mut zyn_vm::read::Decoder::new(&wire)).as_ref(), Some(&op));
        // The reserve always covers buying back every lot sold.
        for c in self.vm.market.curves.values().filter(|c| !c.graduated) {
            assert!(c.reserve.0 >= cost(0, c.sold), "reserve {} below the integral {}", c.reserve.0, cost(0, c.sold));
        }
        r.into_iter().next().unwrap()
    }

    fn ok(&mut self, op: Op) -> Receipt {
        let r = self.apply(op);
        assert!(!ObjectVm::rejected(&r), "{r:?}");
        r
    }

    fn refused(&mut self, op: Op) -> String {
        match self.apply(op) {
            Receipt::Refused(why) => why.to_string(),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn seal(&mut self) {
        assert!(matches!(self.apply(Op::Seal), Receipt::Sealed(_)));
    }

    fn fresh(&mut self) -> Hash {
        self.salt += 1;
        let mut h = [0x3c; 32];
        h[..8].copy_from_slice(&self.salt.to_be_bytes());
        h
    }

    fn mint(&mut self, owner: AccountId, whole: i64) {
        let nonce = self.fresh();
        self.ok(Op::Mint { asset: ZEC, amount: Fixed::whole(whole), owner, nonce });
    }

    fn note(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> Output {
        Output { note: Note { asset, amount, owner, nonce: self.fresh() }, enc: enc() }
    }

    fn balance(&self, owner: AccountId, asset: [u8; 32]) -> Fixed {
        Fixed(self.vm.live.values().filter(|n| n.owner == owner && n.asset == asset).map(|n| n.amount.0).sum())
    }

    fn fund(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> (Vec<(Hash, Hash)>, Vec<Output>) {
        let mut picked = Vec::new();
        let mut sum = 0i128;
        let notes: Vec<(Hash, Note)> = self.vm.live.iter().map(|(c, n)| (*c, *n)).collect();
        for (c, n) in notes {
            if sum >= amount.0 {
                break;
            }
            if n.owner == owner && n.asset == asset {
                picked.push((c, nullifier(&[owner[0]; 32], &c)));
                sum += n.amount.0;
            }
        }
        assert!(sum >= amount.0, "too little to fund that");
        let change = if sum > amount.0 { vec![self.note(owner, asset, Fixed(sum - amount.0))] } else { Vec::new() };
        (picked, change)
    }

    fn claim(&mut self, owner: AccountId, sources: &[Hash]) {
        let mut ids = Vec::new();
        let mut outputs = Vec::new();
        for s in sources {
            for k in 0..3 {
                let id = market::claim_id(s, k);
                if let Some(c) = self.vm.market.claims.get(&id).copied() {
                    assert_eq!(c.owner, owner);
                    ids.push(id);
                    outputs.push(self.note(owner, c.asset, c.amount));
                }
            }
        }
        assert!(!ids.is_empty(), "nothing to claim");
        self.ok(Op::Claim { owner, program: program::claim(), claims: ids, outputs });
    }

    fn terms(&mut self, fee_bps: u32, bond: Fixed) {
        self.ok(Op::SetCurveTerms { program: program::configure_curves(), fee_bps, bond });
    }

    /// Make a token; returns it and the creator's nonce.
    fn create(&mut self, creator: AccountId) -> ([u8; 32], Hash) {
        let bond = self.vm.market.curve_terms.map(|t| t.bond).unwrap_or(Fixed::ZERO);
        let (inputs, change) = if bond.0 > 0 { self.fund(creator, ZEC, bond) } else { (Vec::new(), Vec::new()) };
        let nonce = self.fresh();
        let r = self.ok(Op::CreateCurve { creator, program: program::curve_create(), nonce, quote: ZEC, meta: [7; 32], inputs, change });
        assert_eq!(r, Receipt::CurveCreated { token: token_of(&creator, &nonce) });
        (token_of(&creator, &nonce), nonce)
    }

    /// Place a trade; returns the order's claim source.
    fn trade(&mut self, who: AccountId, token: [u8; 32], buy: bool, amount: Fixed, min_out: Fixed) -> Hash {
        let (inputs, change) = self.fund(who, if buy { ZEC } else { token }, amount);
        let nonce = self.fresh();
        self.ok(Op::CurveTrade { trader: who, program: program::curve_trade(), token, buy, amount_in: amount, min_out, inputs, change, nonce });
        curve_order_id(&who, &nonce)
    }
}

/// The whole life of a token with the adopted fee package: 1% a trade and a
/// 0.05 bond.
#[test]
fn a_token_is_made_traded_and_graduates_into_a_locked_pool() {
    let mut c = Chain::new();
    c.mint(ALICE, 60);
    c.mint(BOB, 2);
    c.mint(CAROL, 2);
    c.terms(100, Fixed(5 * CENT.0));

    // Bob makes a token; the bond leaves his wallet and waits in the curve.
    let (token, bob_nonce) = c.create(BOB);
    assert_eq!(c.balance(BOB, ZEC), Fixed(2 * 100 * CENT.0 - 5 * CENT.0));
    assert_eq!(c.vm.market.curves[&token].bond, Fixed(5 * CENT.0));

    // Two buyers in one epoch: one price, lots in proportion to what they put in.
    let a = c.trade(CAROL, token, true, Fixed(30 * CENT.0), Fixed::ZERO);
    let b = c.trade(BOB, token, true, Fixed(10 * CENT.0), Fixed::ZERO);
    c.seal();
    c.claim(CAROL, &[a]);
    c.claim(BOB, &[b]);
    let (carol, bob) = (c.balance(CAROL, token).0 / LOT.0, c.balance(BOB, token).0 / LOT.0);
    assert!(carol > 0 && (carol - 3 * bob).abs() <= 3, "{carol} vs 3×{bob}");
    let sold = c.vm.market.curves[&token].sold;
    assert_eq!(sold as i128, carol + bob);
    assert_eq!(c.vm.issued[&token].0, sold as i128 * LOT.0, "tokens exist only for lots sold");

    // The fee split: 50% pot, 40% graduation, 10% creator (§69.8).
    let cv = c.vm.market.curves[&token];
    let pot = c.vm.market.pol[&ZEC];
    let fees = pot.0 + cv.graduation.0 + cv.creator_fees.0;
    assert!(fees > 0);
    // Each fee is split on its own and rounded down, so allow a unit per fee.
    assert!((cv.creator_fees.0 - fees / 10).abs() <= 2, "creator {} of {fees}", cv.creator_fees.0);
    assert!((cv.graduation.0 - fees * 40 / 100).abs() <= 2, "graduation {} of {fees}", cv.graduation.0);

    // Carol sells some back and is paid from the reserve.
    let before = c.balance(CAROL, ZEC);
    let s = c.trade(CAROL, token, false, Fixed(10 * LOT.0), Fixed::ZERO);
    c.seal();
    c.claim(CAROL, &[s]);
    assert!(c.balance(CAROL, ZEC) > before);
    assert_eq!(c.vm.market.curves[&token].sold, sold - 10);

    // The pool cannot be taken before graduation: refused on the token alone.
    let pool_nonce = c.fresh();
    let (x, y) = if token < ZEC { (token, ZEC) } else { (ZEC, token) };
    let why = c.refused(Op::CreatePool {
        provider: ALICE,
        program: program::add_liquidity(),
        asset_a: x,
        asset_b: y,
        amount_a: Fixed::whole(1),
        amount_b: Fixed::whole(1),
        fee_bps: 30,
        inputs: Vec::new(),
        change: Vec::new(),
        nonce: pool_nonce,
    });
    assert!(why.contains("still on its curve"), "{why}");

    // A whale takes the rest: the batch stops at graduation and refunds the rest.
    let w = c.trade(ALICE, token, true, Fixed::whole(40), Fixed::ZERO);
    c.seal();
    let cv = c.vm.market.curves[&token];
    assert!(cv.graduated);
    assert_eq!(cv.sold, GRADUATE_AT);
    c.claim(ALICE, &[w]);
    assert!(c.balance(ALICE, ZEC) > Fixed::whole(20), "the unfilled part came back");

    // The pool: every unsold lot and all the curve raised, every share locked.
    let pid = market::pool_id(&token, &ZEC);
    let p = c.vm.market.pools[&pid];
    assert_eq!(p.locked, p.shares, "nobody holds a share of it");
    assert_eq!(p.fee_bps, POOL_FEE_BPS);
    let (tok, quote) = if p.a == token { (p.reserve_a, p.reserve_b) } else { (p.reserve_b, p.reserve_a) };
    assert_eq!(tok.0, (LOTS - GRADUATE_AT) as i128 * LOT.0);
    assert!(quote.0 >= cost(0, GRADUATE_AT), "the pool has at least what the curve raised");
    assert_eq!(c.vm.issued[&token].0, curve::SUPPLY.0, "the whole supply exists now, and no more");

    // The creator's escrow and bond are paid only now.
    let before = c.balance(BOB, ZEC);
    c.claim(BOB, &[curve_source(&BOB, &bob_nonce)]);
    assert!(c.balance(BOB, ZEC).0 >= before.0 + 5 * CENT.0);

    // From here it trades in the pool, not on the curve.
    let (inputs, change) = c.fund(CAROL, ZEC, CENT);
    let n = c.fresh();
    let why = c.refused(Op::CurveTrade { trader: CAROL, program: program::curve_trade(), token, buy: true, amount_in: CENT, min_out: Fixed::ZERO, inputs, change, nonce: n });
    assert!(why.contains("graduated"), "{why}");
}

/// No terms, no charges: a token costs nothing to make, trades pay no fee,
/// and a chain that never touched curves keeps its roots.
#[test]
fn without_terms_nothing_is_charged() {
    let mut c = Chain::new();
    c.mint(ALICE, 5);
    let before = c.vm.state_root();
    assert!(c.vm.market.is_empty());
    let (token, _) = c.create(ALICE);
    assert_ne!(c.vm.state_root(), before);
    let o = c.trade(ALICE, token, true, Fixed::whole(1), Fixed::ZERO);
    c.seal();
    c.claim(ALICE, &[o]);
    assert!(c.vm.market.pol.is_empty(), "no fee without terms");
    let cv = c.vm.market.curves[&token];
    assert_eq!(cv.reserve.0, cost(0, cv.sold), "a lone buyer pays exactly the curve");
}

#[test]
fn the_curve_refuses_what_it_cannot_settle_honestly() {
    let mut c = Chain::new();
    c.mint(ALICE, 5);
    c.terms(100, Fixed::ZERO);
    let (token, _) = c.create(ALICE);
    // Too small a buy.
    let (inputs, change) = c.fund(ALICE, ZEC, Fixed(CENT.0 / 100));
    let n = c.fresh();
    let why = c.refused(Op::CurveTrade { trader: ALICE, program: program::curve_trade(), token, buy: true, amount_in: Fixed(CENT.0 / 100), min_out: Fixed::ZERO, inputs, change, nonce: n });
    assert!(why.contains("at least"), "{why}");
    // Terms above the ceiling.
    let why = c.refused(Op::SetCurveTerms { program: program::configure_curves(), fee_bps: 201, bond: Fixed::ZERO });
    assert!(why.contains("ceiling"), "{why}");
    // A token nobody made.
    let n = c.fresh();
    let why = c.refused(Op::CurveTrade { trader: ALICE, program: program::curve_trade(), token: [0x99; 32], buy: true, amount_in: CENT, min_out: Fixed::ZERO, inputs: Vec::new(), change: Vec::new(), nonce: n });
    assert!(why.contains("no such curve"), "{why}");
    // A sell that is not whole lots.
    let o = c.trade(ALICE, token, true, Fixed::whole(1), Fixed::ZERO);
    c.seal();
    c.claim(ALICE, &[o]);
    let (inputs, change) = c.fund(ALICE, token, Fixed(LOT.0 / 2));
    let n = c.fresh();
    let why = c.refused(Op::CurveTrade { trader: ALICE, program: program::curve_trade(), token, buy: false, amount_in: Fixed(LOT.0 / 2), min_out: Fixed::ZERO, inputs, change, nonce: n });
    assert!(why.contains("whole number of lots"), "{why}");
    // Curve tokens cannot leave for Zcash: they are made here.
    assert!(c.vm.market.is_derived(&token));
}

/// A buy whose minimum the batch price does not meet comes back whole.
#[test]
fn slippage_protection_refunds_whole() {
    let mut c = Chain::new();
    c.mint(ALICE, 5);
    c.mint(BOB, 5);
    let (token, _) = c.create(ALICE);
    let greedy = c.trade(BOB, token, true, CENT, Fixed(10_000 * LOT.0));
    c.seal();
    c.claim(BOB, &[greedy]);
    assert_eq!(c.balance(BOB, ZEC), Fixed::whole(5), "refunded whole");
    assert_eq!(c.balance(BOB, token), Fixed::ZERO);
}
