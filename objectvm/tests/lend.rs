//! Zyn Lend on the ledger, end to end.
//!
//! Every intent goes through `apply`, and after every one: supply is
//! conserved (live objects plus what the market and lending hold equal what
//! was issued), every object is live or retired, the state survives its own
//! encoding, and the intent survives its own wire format. A lending module
//! that paid the right people in one test and leaked a coin in the next would
//! fail here on the step it leaked.

use objectvm::lend::{lend_asset, lend_source, lot_source, Lend};
use objectvm::market;
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x10; 32];
const SOL: [u8; 32] = [0x50; 32];
const ALICE: AccountId = [0xA1; 32]; // lends SOL
const BOB: AccountId = [0xB0; 32]; // borrows SOL against ZEC
const CAROL: AccountId = [0xC0; 32]; // bids in the auction
const DAVE: AccountId = [0xD0; 32]; // made the ZEC/SOL pool

const WAD: i128 = 1_000_000_000_000_000_000;
const fn usd(n: i128) -> Fixed {
    Fixed(n * WAD)
}

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
        Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, nonce: 0, salt: 0 }
    }

    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let r = self.vm.apply(self.seq, &op);
        assert_eq!(self.vm.conserved(), Ok(()), "after {op:?}");
        assert_eq!(self.vm.reconciled(), Ok(()));
        let back = ObjectVm::decode(&self.vm.encode()).expect("state decodes");
        assert_eq!(back, self.vm, "state round trip");
        let wire = ObjectVm::encode_intent(&op);
        let mut d = zyn_vm::read::Decoder::new(&wire);
        assert_eq!(ObjectVm::decode_intent(&mut d).as_ref(), Some(&op), "intent round trip");
        r.into_iter().next().unwrap()
    }

    fn ok(&mut self, op: Op) -> Receipt {
        let r = self.apply(op);
        assert!(!ObjectVm::rejected(&r), "{r:?}");
        r
    }

    fn refused(&mut self, op: Op) -> &'static str {
        match self.apply(op) {
            Receipt::Refused(why) => why,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn seal(&mut self) {
        assert!(matches!(self.apply(Op::Seal), Receipt::Sealed(_)));
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
        self.ok(Op::Mint { asset, amount: Fixed::whole(whole), owner, nonce });
    }

    fn note(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> Output {
        Output { note: Note { asset, amount, owner, nonce: self.fresh() }, enc: enc() }
    }

    fn balance(&self, owner: AccountId, asset: [u8; 32]) -> i128 {
        self.vm.live.values().filter(|n| n.owner == owner && n.asset == asset).map(|n| n.amount.0).sum()
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
        assert!(sum >= amount.0, "{owner:?} holds too little");
        let change = if sum > amount.0 { vec![self.note(owner, asset, Fixed(sum - amount.0))] } else { Vec::new() };
        (picked, change)
    }

    /// Collect every claim `owner` can derive from these sources.
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
        self.ok(Op::Claim { owner, program: program::claim(), claims: ids, outputs });
    }

    fn prices(&mut self, at: u64, zec: i128, sol: i128) {
        self.ok(Op::LendPrices { program: program::lend_prices(), at, prices: vec![(ZEC, usd(zec)), (SOL, usd(sol))] });
    }

    fn supply(&mut self, who: AccountId, asset: [u8; 32], whole: i64) -> Hash {
        let (inputs, change) = self.fund(who, asset, Fixed::whole(whole));
        let nonce = self.fresh();
        self.ok(Op::LendSupply { lender: who, program: program::lend_supply(), asset, amount: Fixed::whole(whole), nonce, inputs, change });
        lend_source(&who, &nonce)
    }

    fn open(&mut self, who: AccountId, coll: [u8; 32], coll_amt: Fixed, borrow: [u8; 32], amt: Fixed) -> Result<Hash, &'static str> {
        let (inputs, change) = self.fund(who, coll, coll_amt);
        let nonce = self.fresh();
        let op = Op::LendOpen { borrower: who, program: program::lend_open(), collateral_asset: coll, collateral: coll_amt, borrow_asset: borrow, amount: amt, nonce, inputs, change };
        match self.apply(op) {
            Receipt::Refused(why) => Err(why),
            _ => Ok(lend_source(&who, &nonce)),
        }
    }

    fn pool(&self, asset: [u8; 32]) -> zyn_lend::Pool {
        self.vm.market.lend.listed[&asset].pool
    }
}

/// A chain with ZEC ($50) and SOL ($150) listed, a ZEC/SOL pool at that
/// price (3 ZEC per SOL), Alice lending 100 SOL.
fn market() -> Chain {
    let mut c = Chain::new();
    for who in [ALICE, BOB, CAROL, DAVE] {
        c.mint(who, ZEC, 1_000);
        c.mint(who, SOL, 1_000);
    }
    // Dave's pool: 300 ZEC to 100 SOL.
    let (ia, mut ch) = c.fund(DAVE, ZEC, Fixed::whole(300));
    let (ib, chb) = c.fund(DAVE, SOL, Fixed::whole(100));
    ch.extend(chb);
    let nonce = c.fresh();
    let (a, b, amt_a, amt_b) = if ZEC < SOL { (ZEC, SOL, 300, 100) } else { (SOL, ZEC, 100, 300) };
    c.ok(Op::CreatePool {
        provider: DAVE,
        program: program::add_liquidity(),
        asset_a: a,
        asset_b: b,
        amount_a: Fixed::whole(amt_a),
        amount_b: Fixed::whole(amt_b),
        fee_bps: 30,
        inputs: [ia, ib].concat(),
        change: ch,
        nonce,
    });
    c.ok(Op::LendList { program: program::lend_list(), asset: ZEC, max_ltv_bps: 5_500, liquidation_bps: 6_000, penalty_bps: 800 });
    c.ok(Op::LendList { program: program::lend_list(), asset: SOL, max_ltv_bps: 6_500, liquidation_bps: 7_000, penalty_bps: 700 });
    c.prices(1_000, 50, 150);
    let src = c.supply(ALICE, SOL, 100);
    c.claim(ALICE, &[src]);
    assert!(c.balance(ALICE, lend_asset(&SOL)) > 0, "Alice holds lSOL");
    c
}

#[test]
fn lend_borrow_repay_and_leave_with_interest() {
    let mut c = market();
    let alice_sol = c.balance(ALICE, SOL);
    // Bob: 100 ZEC ($5,000) for 15 SOL ($2,250) — 45%, under ZEC's 55%.
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    c.claim(BOB, &[pos]);
    assert_eq!(c.balance(BOB, SOL), 1_015 * WAD);
    // A year passes (prices posted with the time), interest accrues.
    c.prices(1_000 + 31_536_000, 50, 150);
    let debt_shares = c.vm.market.lend.positions[&pos].debt_shares.0;
    let owed = c.pool(SOL).owed(debt_shares).unwrap();
    assert!(owed > 15 * WAD, "interest accrued: {owed}");
    // Bob repays everything, and a little more; the excess comes back.
    let (inputs, change) = c.fund(BOB, SOL, Fixed(owed + WAD));
    let nonce = c.fresh();
    c.ok(Op::LendRepay { borrower: BOB, program: program::lend_repay(), position: pos, amount: Fixed(owed + WAD), nonce, inputs, change });
    c.claim(BOB, &[lend_source(&BOB, &nonce)]);
    assert_eq!(c.vm.market.lend.positions[&pos].debt_shares.0, 0);
    // With no debt, all the collateral can come out, and the position is gone.
    let nonce = c.fresh();
    c.ok(Op::LendRemoveCollateral { borrower: BOB, program: program::lend_collateral(), position: pos, amount: Fixed::whole(100), nonce });
    c.claim(BOB, &[lend_source(&BOB, &nonce)]);
    assert!(!c.vm.market.lend.positions.contains_key(&pos));
    assert_eq!(c.balance(BOB, ZEC), 1_000 * WAD, "Bob has his ZEC back");
    // The seal pays the reserve into the ZYN pot.
    c.seal();
    assert!(c.vm.market.pol.get(&SOL).map(|p| p.0).unwrap_or(0) > 0, "the ZYN pot got its share");
    // Alice leaves with more than she lent.
    let shares = c.balance(ALICE, lend_asset(&SOL));
    let (inputs, change) = c.fund(ALICE, lend_asset(&SOL), Fixed(shares));
    let nonce = c.fresh();
    c.ok(Op::LendWithdraw { lender: ALICE, program: program::lend_withdraw(), asset: SOL, shares: Fixed(shares), nonce, inputs, change });
    c.claim(ALICE, &[lend_source(&ALICE, &nonce)]);
    let earned = c.balance(ALICE, SOL) - alice_sol;
    assert!(earned > 100 * WAD, "Alice earned interest: {earned}");
    assert_eq!(c.balance(ALICE, lend_asset(&SOL)), 0);
}

#[test]
fn the_rules_refuse_what_they_should() {
    let mut c = market();
    // Over ZEC's 55%: 100 ZEC ($5,000) against 20 SOL ($3,000) is 60%.
    assert_eq!(c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(20)), Err("that is over the most this collateral may borrow"));
    // More than the pool has free.
    assert_eq!(c.open(BOB, ZEC, Fixed::whole(900), SOL, Fixed::whole(150)), Err("the pool does not have that much free"));
    // Same asset both sides.
    assert_eq!(c.open(BOB, SOL, Fixed::whole(10), SOL, Fixed::whole(1)), Err("collateral and the borrowed asset must differ"));
    // Prices far from the pool pause the pair: SOL posted at $400, pool says $150.
    c.prices(2_000, 50, 400);
    assert_eq!(
        c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(1)),
        Err("borrowing in this pair is paused: Zyn's pool and the posted price disagree")
    );
    // Prices cannot go back in time.
    assert_eq!(c.refused(Op::LendPrices { program: program::lend_prices(), at: 1_500, prices: vec![] }), "prices older than the last ones");
    c.prices(3_000, 50, 150);
    // Somebody else's position cannot be touched.
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(10)).unwrap();
    let nonce = c.fresh();
    assert_eq!(
        c.refused(Op::LendRemoveCollateral { borrower: CAROL, program: program::lend_collateral(), position: pos, amount: Fixed::whole(1), nonce }),
        "not the signer's position"
    );
    // Nor can Bob take collateral out past his maximum.
    let nonce = c.fresh();
    assert_eq!(
        c.refused(Op::LendRemoveCollateral { borrower: BOB, program: program::lend_collateral(), position: pos, amount: Fixed::whole(60), nonce }),
        "that would leave the position over its maximum"
    );
    // The receipt token cannot be listed as collateral, nor withdrawn to Zcash.
    assert_eq!(
        c.refused(Op::LendList { program: program::lend_list(), asset: lend_asset(&SOL), max_ltv_bps: 5_000, liquidation_bps: 6_000, penalty_bps: 500 }),
        "only an asset that exists and was not made here can be lent"
    );
    assert!(c.vm.market.is_derived(&lend_asset(&SOL)));
}

#[test]
fn a_position_past_its_line_is_auctioned_and_the_lenders_are_whole() {
    let mut c = market();
    // Bob borrows 15 SOL against 100 ZEC at $50: 45%.
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    c.claim(BOB, &[pos]);
    // ZEC falls to $35: 100 ZEC is $3,500; debt $2,250 is 64% — past the 60% line.
    c.prices(2_000, 35, 150);
    c.seal();
    assert!(!c.vm.market.lend.positions.contains_key(&pos), "no longer a position");
    let lot = c.vm.market.lend.lots[&pos];
    assert_eq!(lot.collateral.0, 100 * WAD);
    let lot_id = lot_source(&pos);
    // The lot opens next epoch at 102% of the posted rate: 35/150 SOL per ZEC.
    let price = Lend::lot_price(&lot, c.vm.epoch).unwrap();
    assert_eq!(price, (35 * WAD / 150) * 10_200 / 10_000);
    // Carol bids enough SOL to cover the debt and penalty, and some.
    let owed = c.pool(SOL).owed(lot.debt_shares.0).unwrap();
    let pay = Fixed(owed * 12 / 10);
    let (inputs, change) = c.fund(CAROL, SOL, pay);
    let nonce = c.fresh();
    c.ok(Op::LendBid { bidder: CAROL, program: program::lend_bid(), lot: lot_id, pay, nonce, inputs, change });
    c.seal();
    // The lot sold what covered debt plus penalty, then closed.
    assert!(!c.vm.market.lend.lots.contains_key(&pos), "the lot settled");
    assert_eq!(c.pool(SOL).debt_shares, 0, "the pool is repaid");
    c.claim(CAROL, &[lend_source(&CAROL, &nonce)]);
    let carol_zec = c.balance(CAROL, ZEC) - 1_000 * WAD;
    assert!(carol_zec > 0 && carol_zec < 100 * WAD, "Carol bought part of the lot: {carol_zec}");
    // Bob gets the collateral that was not needed.
    c.claim(BOB, &[lot_source(&pos)]);
    assert_eq!(c.balance(BOB, ZEC), 900 * WAD + (100 * WAD - carol_zec), "every ZEC is Carol's or Bob's");
    // And Alice's pool now holds more than she lent: interest, plus the penalty.
    let value = c.pool(SOL).value().unwrap();
    assert!(value > 100 * WAD, "lenders were made whole and then some: {value}");
}

#[test]
fn an_auction_nobody_fills_keeps_falling_to_its_floor() {
    let mut c = market();
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    c.prices(2_000, 35, 150);
    c.seal();
    let lot = c.vm.market.lend.lots[&pos];
    let start = Lend::lot_price(&lot, lot.opened).unwrap();
    for _ in 0..40 {
        c.seal();
    }
    let floor = Lend::lot_price(&lot, c.vm.epoch).unwrap();
    assert_eq!(floor, (35 * WAD / 150) * 8_500 / 10_000);
    assert!(floor < start);
    // Still unsold, still counted: nothing has leaked while it waited.
    assert!(c.vm.market.lend.lots.contains_key(&pos));
}

#[test]
fn two_bidders_in_one_epoch_pay_the_same_price() {
    let mut c = market();
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    c.prices(2_000, 35, 150);
    c.seal();
    let lot_id = lot_source(&pos);
    let owed = c.pool(SOL).owed(c.vm.market.lend.lots[&pos].debt_shares.0).unwrap();
    // Together they bid far more than the lot needs: each is filled pro rata
    // and refunded the rest, at one price.
    let mut sources = Vec::new();
    for who in [CAROL, DAVE] {
        let pay = Fixed(owed);
        let (inputs, change) = c.fund(who, SOL, pay);
        let nonce = c.fresh();
        c.ok(Op::LendBid { bidder: who, program: program::lend_bid(), lot: lot_id, pay, nonce, inputs, change });
        sources.push((who, lend_source(&who, &nonce)));
    }
    let (carol0, dave0) = (c.balance(CAROL, ZEC), c.balance(DAVE, ZEC));
    c.seal();
    for (who, src) in &sources {
        c.claim(*who, &[*src]);
    }
    let carol = c.balance(CAROL, ZEC) - carol0;
    let dave = c.balance(DAVE, ZEC) - dave0;
    assert!(carol > 0 && (carol - dave).abs() <= 1, "equal bids, equal fills: {carol} vs {dave}");
    assert!(!c.vm.market.lend.lots.contains_key(&pos));
}
