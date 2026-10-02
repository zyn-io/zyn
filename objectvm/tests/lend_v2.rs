//! Zyn Lend's rules v2 (DECISIONS §160), and proof that they change nothing
//! before they are switched on.
//!
//! Testnet 212's journal holds lending from before guarded mode, guarded mode
//! itself, and (from §160) rules v2. Replicas and ZynScan replay it from
//! genesis and check every root, so every history the old rules wrote must
//! still reach the same roots: `old_histories_replay_to_the_roots_they_sealed`
//! pins them, as computed by the code before §160.

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
const ALICE: AccountId = [0xA1; 32];
const BOB: AccountId = [0xB0; 32];
const CAROL: AccountId = [0xC0; 32];
const DAVE: AccountId = [0xD0; 32];

const WAD: i128 = 1_000_000_000_000_000_000;
const fn usd(n: i128) -> Fixed {
    Fixed(n * WAD)
}

struct Chain {
    vm: ObjectVm,
    seq: u64,
    nonce: u8,
    salt: u64,
    /// Every sealed root, folded: one hash pins a whole history.
    roots: Hash,
}

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

impl Chain {
    fn new() -> Chain {
        Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, nonce: 0, salt: 0, roots: [0u8; 32] }
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
        let r = r.into_iter().next().unwrap();
        if let Receipt::Sealed(cp) = &r {
            let mut e = zyn_vm::commit::Encoder::new();
            e.bytes(&self.roots).bytes(&cp.state_root);
            self.roots = e.leaf();
        }
        r
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

    /// Prices in cents, for moves finer than a dollar.
    fn prices_c(&mut self, at: u64, zec_cents: i128, sol_cents: i128) {
        self.ok(Op::LendPrices { program: program::lend_prices(), at, prices: vec![(ZEC, Fixed(zec_cents * WAD / 100)), (SOL, Fixed(sol_cents * WAD / 100))] });
    }


    fn tick(&mut self, at: u64) {
        self.ok(Op::LendTime { program: program::lend_prices(), at });
    }

    fn now(&self) -> u64 {
        self.vm.market.lend.time
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

    fn bid(&mut self, who: AccountId, pos: &Hash, pay: Fixed) -> Result<Hash, &'static str> {
        let (inputs, change) = self.fund(who, SOL, pay);
        let nonce = self.fresh();
        match self.apply(Op::LendBid { bidder: who, program: program::lend_bid(), lot: lot_source(pos), pay, nonce, inputs, change }) {
            Receipt::Refused(why) => Err(why),
            _ => Ok(lend_source(&who, &nonce)),
        }
    }

    fn repay(&mut self, who: AccountId, pos: &Hash, amount: Fixed) -> Result<Hash, &'static str> {
        let (inputs, change) = self.fund(who, SOL, amount);
        let nonce = self.fresh();
        match self.apply(Op::LendRepay { borrower: who, program: program::lend_repay(), position: *pos, amount, nonce, inputs, change }) {
            Receipt::Refused(why) => Err(why),
            _ => Ok(lend_source(&who, &nonce)),
        }
    }

    fn remove(&mut self, who: AccountId, pos: &Hash, amount: Fixed) -> Result<Hash, &'static str> {
        let nonce = self.fresh();
        match self.apply(Op::LendRemoveCollateral { borrower: who, program: program::lend_collateral(), position: *pos, amount, nonce }) {
            Receipt::Refused(why) => Err(why),
            _ => Ok(lend_source(&who, &nonce)),
        }
    }

    fn add(&mut self, who: AccountId, pos: &Hash, asset: [u8; 32], amount: Fixed) -> Result<(), &'static str> {
        let (inputs, change) = self.fund(who, asset, amount);
        match self.apply(Op::LendAddCollateral { borrower: who, program: program::lend_collateral(), position: *pos, amount, inputs, change }) {
            Receipt::Refused(why) => Err(why),
            _ => Ok(()),
        }
    }

    fn withdraw(&mut self, who: AccountId, asset: [u8; 32], shares: Fixed) -> Result<Hash, &'static str> {
        let (inputs, change) = self.fund(who, lend_asset(&asset), shares);
        let nonce = self.fresh();
        match self.apply(Op::LendWithdraw { lender: who, program: program::lend_withdraw(), asset, shares, nonce, inputs, change }) {
            Receipt::Refused(why) => Err(why),
            _ => Ok(lend_source(&who, &nonce)),
        }
    }

    fn pool(&self, asset: [u8; 32]) -> zyn_lend::Pool {
        self.vm.market.lend.listed[&asset].pool
    }

    fn limits(&mut self, asset: [u8; 32], collateral_cap: i64, borrow_cap: i64, min_usd: i128) {
        self.ok(Op::LendLimits {
            program: program::lend_list(),
            asset,
            collateral_cap: Fixed::whole(collateral_cap),
            borrow_cap: Fixed::whole(borrow_cap),
            min_borrow_usd: usd(min_usd),
        });
    }
}

fn hex(h: &Hash) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// ZEC ($50) and SOL ($150) listed, a ZEC/SOL pool of `pool_zec` ZEC to a
/// third as much SOL, Alice lending 100 SOL.
fn market_with(pool_zec: i64) -> Chain {
    let mut c = Chain::new();
    for who in [ALICE, BOB, CAROL, DAVE] {
        c.mint(who, ZEC, 5_000);
        c.mint(who, SOL, 1_000);
    }
    let (ia, mut ch) = c.fund(DAVE, ZEC, Fixed::whole(pool_zec));
    let (ib, chb) = c.fund(DAVE, SOL, Fixed::whole(pool_zec / 3));
    ch.extend(chb);
    let nonce = c.fresh();
    let (a, b, amt_a, amt_b) = if ZEC < SOL { (ZEC, SOL, pool_zec, pool_zec / 3) } else { (SOL, ZEC, pool_zec / 3, pool_zec) };
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
    c
}

fn market() -> Chain {
    market_with(300)
}

// ------------------------------------------------------------ old histories

/// Lending before guarded mode: interest, repay, close, an auction filled,
/// an auction left to its floor.
fn history_open() -> Chain {
    let mut c = market();
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    c.claim(BOB, &[pos]);
    c.prices(1_000 + 31_536_000, 50, 150);
    let owed = c.pool(SOL).owed(c.vm.market.lend.positions[&pos].debt_shares.0).unwrap();
    let src = c.repay(BOB, &pos, Fixed(owed + WAD)).unwrap();
    c.claim(BOB, &[src]);
    let src = c.remove(BOB, &pos, Fixed::whole(100)).unwrap();
    c.claim(BOB, &[src]);
    c.seal();
    let t = 1_000 + 31_536_000;
    // An auction, filled by one bid.
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    c.prices(t + 10, 35, 150);
    c.seal();
    let lot = c.vm.market.lend.lots[&pos];
    let owed = c.pool(SOL).owed(lot.debt_shares.0).unwrap();
    let b = c.bid(CAROL, &pos, Fixed(owed * 12 / 10)).unwrap();
    c.seal();
    c.claim(CAROL, &[b]);
    // An auction nobody fills, to its floor.
    c.prices(t + 20, 50, 150);
    let pos2 = c.open(CAROL, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    c.prices(t + 30, 35, 150);
    for _ in 0..25 {
        c.seal();
    }
    assert!(c.vm.market.lend.lots.contains_key(&pos2));
    c
}

/// Guarded mode: caps, staleness, the breaker, a booked shortfall and its
/// recovery, a backstop through the pool batch, insurance.
fn history_guarded() -> Chain {
    let mut c = market();
    c.limits(ZEC, 10_000, 10_000, 1);
    c.limits(SOL, 10_000, 10_000, 1);
    assert_eq!(c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed(WAD / 1_000)), Err("that loan is below the smallest this pool makes"));
    for _ in 0..13 {
        c.seal();
    }
    assert!(c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(10)).is_err(), "stale");
    c.prices(5_000, 50, 150);
    c.open(DAVE, ZEC, Fixed::whole(200), SOL, Fixed::whole(10)).unwrap();
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    // A jump held, then confirmed: liquidated, shortfall booked.
    c.prices(6_000, 20, 150);
    c.seal();
    c.prices(6_010, 20, 150);
    c.seal();
    let lot = c.vm.market.lend.lots[&pos];
    let price = Lend::lot_price(&lot, c.vm.epoch).unwrap();
    let pay = Fixed(zyn_lend::math::wmul_up(lot.collateral.0, price).unwrap());
    let b = c.bid(CAROL, &pos, pay).unwrap();
    c.seal();
    c.claim(CAROL, &[b]);
    // A lot nobody bids on, sold by the backstop.
    c.prices(7_000, 50, 150);
    c.prices(7_005, 50, 150);
    let pos = c.open(CAROL, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    c.prices(7_010, 35, 150);
    c.prices(7_020, 35, 150);
    for i in 0..60 {
        if i % 10 == 0 {
            c.prices(8_000 + i, 35, 150);
        }
        c.seal();
        if !c.vm.market.lend.lots.contains_key(&pos) {
            break;
        }
    }
    assert!(!c.vm.market.lend.lots.contains_key(&pos));
    // A year of interest fills insurance.
    c.prices(8_100 + 31_536_000, 50, 150);
    c.seal();
    assert!(c.vm.market.lend.insurance.get(&SOL).map_or(0, |x| x.0) > 0, "insurance filled");
    c
}

/// Every root these histories sealed, under the code before §160. If one of
/// these moves, a replica replaying testnet 212 stops agreeing with it.
#[test]
fn old_histories_replay_to_the_roots_they_sealed() {
    let open = history_open();
    let guarded = history_guarded();
    assert_eq!(open.vm.market.lend.rules, 0);
    assert_eq!(guarded.vm.market.lend.rules, 0);
    assert_eq!(hex(&open.roots), "ddcc3d42f6ecc989fe3279c5e0f01f219014cbbdb770030c4786824d013b8a4a");
    assert_eq!(hex(&guarded.roots), "32ce7651773a19abb3c2b5e69eac8c884251f4761b533d5610f7430c628d8a76");
}

/// Until rules v2 are on, the state saves as MARKET09, exactly as before
/// §160: a node rolled back to an older binary still reads it. After, it
/// saves as MARKET10.
#[test]
fn the_state_saves_as_before_until_rules_v2_are_on() {
    let tag = |vm: &ObjectVm| {
        let bytes = vm.encode();
        let at = bytes.windows(7).position(|w| w == b"MARKET1" || w == b"MARKET0").expect("market tag");
        String::from_utf8(bytes[at..at + 8].to_vec()).unwrap()
    };
    let c = history_guarded();
    assert_eq!(tag(&c.vm), "MARKET09");
    let c = v2();
    assert_eq!(tag(&c.vm), "MARKET10");
}

// ------------------------------------------------------------------ rules v2

const EPOCH_SPAM: usize = 200;

/// `market()` with a deep ZEC/SOL pool (3,000 ZEC), guarded, then rules v2.
fn v2() -> Chain {
    let mut c = market_with(3_000);
    c.mint(DAVE, SOL, 1_000);
    c.limits(ZEC, 10_000, 10_000, 1);
    c.limits(SOL, 10_000, 10_000, 1);
    c.ok(Op::LendRules { program: program::lend_list(), version: 2 });
    assert!(c.vm.market.lend.v2());
    assert_eq!(c.vm.market.lend.priced_at[&ZEC], 1_000);
    c
}

/// Post prices `secs` after the clock, twice when it is a jump the breaker
/// holds, so they are confirmed.
fn confirm(c: &mut Chain, secs: u64, zec_cents: i128, sol_cents: i128) {
    let t = c.now() + secs;
    c.prices_c(t, zec_cents, sol_cents);
    c.prices_c(t + 1, zec_cents, sol_cents);
    assert!(c.vm.market.lend.pending.is_empty());
}

#[test]
fn rules_v2_come_after_guarded_mode_once_and_need_sound_risk() {
    let mut c = market();
    let rules = Op::LendRules { program: program::lend_list(), version: 2 };
    assert_eq!(c.refused(rules.clone()), "rules v2 come after guarded mode's limits");
    assert_eq!(c.refused(Op::LendTime { program: program::lend_prices(), at: 2_000 }), "the lending clock is posted under rules v2 only");
    c.limits(ZEC, 10_000, 10_000, 1);
    c.limits(SOL, 10_000, 10_000, 1);
    // ETH's old 80% line with a 5% penalty needs 84% of an 85% floor.
    c.mint(DAVE, [0x77; 32], 10);
    c.ok(Op::LendList { program: program::lend_list(), asset: [0x77; 32], max_ltv_bps: 7_500, liquidation_bps: 8_000, penalty_bps: 500 });
    assert_eq!(c.refused(rules.clone()), "a listed collateral's line and penalty do not fit under the auction floor; list it again first");
    c.ok(Op::LendList { program: program::lend_list(), asset: [0x77; 32], max_ltv_bps: 7_200, liquidation_bps: 7_700, penalty_bps: 500 });
    assert_eq!(c.refused(Op::LendRules { program: program::lend_list(), version: 3 }), "no such lending rules");
    c.ok(rules.clone());
    assert_eq!(c.refused(rules), "those lending rules are already in force");
    assert_eq!(
        c.refused(Op::LendList { program: program::lend_list(), asset: [0x77; 32], max_ltv_bps: 7_500, liquidation_bps: 8_000, penalty_bps: 500 }),
        "under rules v2 a liquidated debt and its penalty must fit under the auction floor"
    );
    assert_eq!(c.refused(Op::LendTime { program: program::lend_prices(), at: 999 }), "the lending clock cannot run backwards");
}

/// Item 1: sealing is free, the clock is not. Epochs spammed by anyone age
/// neither a price nor an auction.
#[test]
fn spammed_epochs_age_neither_prices_nor_auctions() {
    let mut c = v2();
    for _ in 0..EPOCH_SPAM {
        c.seal();
    }
    assert!(c.vm.market.lend.fresh(&ZEC, c.vm.epoch), "two hundred epochs, no time");
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    confirm(&mut c, 10, 3_500, 15_000);
    c.seal();
    let lot = c.vm.market.lend.lots[&pos];
    let start = Lend::lot_price_at(&lot, c.now()).unwrap();
    for _ in 0..EPOCH_SPAM {
        c.seal();
    }
    let l = &c.vm.market.lend;
    assert_eq!(l.auction_price(&l.lots[&pos], c.vm.epoch), Some(start), "the auction did not move");
    assert_eq!(start, (35 * WAD / 150) * 10_200 / 10_000);
    // Only the operator's clock moves it: a point a minute.
    let t = c.now();
    c.tick(t + 59);
    assert_eq!(c.vm.market.lend.auction_price(&lot, c.vm.epoch), Some(start));
    c.tick(t + 60);
    assert_eq!(c.vm.market.lend.auction_price(&lot, c.vm.epoch), Some((35 * WAD / 150) * 10_100 / 10_000));
    c.tick(t + objectvm::lend::AUCTION_FLOOR_AFTER_SECS);
    assert_eq!(c.vm.market.lend.auction_price(&lot, c.vm.epoch), Some((35 * WAD / 150) * 8_500 / 10_000));
    // And prices go stale on it, not on epochs: five minutes unposted.
    let t = c.now() + 1;
    c.prices_c(t, 3_500, 15_000);
    c.tick(t + 300);
    assert!(c.vm.market.lend.fresh(&ZEC, c.vm.epoch));
    c.tick(t + 301);
    assert!(!c.vm.market.lend.fresh(&ZEC, c.vm.epoch));
    assert_eq!(c.open(CAROL, ZEC, Fixed::whole(100), SOL, Fixed::whole(1)), Err("prices are stale or still moving; borrowing waits for the next confirmed price"));
}

/// Item 2: each asset's price ages on its own.
#[test]
fn each_assets_price_ages_on_its_own() {
    let mut c = v2();
    let t = c.now();
    c.ok(Op::LendPrices { program: program::lend_prices(), at: t + 200, prices: vec![(SOL, usd(150))] });
    c.tick(t + 301);
    let l = &c.vm.market.lend;
    assert!(!l.fresh(&ZEC, 0) && l.fresh(&SOL, 0));
    assert_eq!(c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)), Err("prices are stale or still moving; borrowing waits for the next confirmed price"));
    c.ok(Op::LendPrices { program: program::lend_prices(), at: t + 302, prices: vec![(ZEC, usd(50))] });
    assert!(c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).is_ok());
}

/// Item 3: a dust bid is refused, and a full lot keeps its largest bids.
#[test]
fn dust_cannot_fill_a_lot_and_a_full_lot_keeps_the_largest_bids() {
    let mut c = v2();
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    confirm(&mut c, 10, 3_500, 15_000);
    c.seal();
    let lot = c.vm.market.lend.lots[&pos];
    // The pool's $1 minimum loan is less than 1% of 15 SOL: 1/150 SOL.
    let min = c.vm.lot_min_bid(&lot).unwrap();
    assert_eq!(min, WAD / 150);
    assert_eq!(c.bid(CAROL, &pos, Fixed(1)), Err("a bid below this lot's smallest"));
    assert_eq!(c.bid(CAROL, &pos, Fixed(min - 1)), Err("a bid below this lot's smallest"));
    let mut first = None;
    for _ in 0..objectvm::lend::MAX_BIDS_PER_LOT {
        let b = c.bid(CAROL, &pos, Fixed(min)).unwrap();
        first.get_or_insert(b);
    }
    assert_eq!(c.bid(DAVE, &pos, Fixed(min)), Err("this lot's bids are full; outbid the smallest"));
    let last = c.vm.market.lend.bids.last().unwrap().id;
    let big = c.bid(DAVE, &pos, Fixed(5 * WAD)).unwrap();
    let bids = &c.vm.market.lend.bids;
    assert_eq!(bids.len(), objectvm::lend::MAX_BIDS_PER_LOT);
    assert!(bids.iter().any(|b| b.id == big) && bids.iter().any(|b| Some(b.id) == first));
    // The newest of the smallest went, and its escrow is a refund.
    assert!(!bids.iter().any(|b| b.id == last));
    assert_eq!(c.vm.market.claims[&market::claim_id(&last, 1)].amount.0, min);
    c.claim(CAROL, &[last]);
}

/// Item 4: a lot follows the price down, books its shortfall at what the
/// backstop could fetch, and owes no interest.
#[test]
fn a_lot_follows_the_price_books_a_realistic_shortfall_and_stops_accruing() {
    let mut c = v2();
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    confirm(&mut c, 10, 3_500, 15_000);
    c.seal();
    let lot = c.vm.market.lend.lots[&pos];
    assert_eq!(lot.rate.0, 35 * WAD / 150);
    // 100 ZEC at 70% of $35 is 16.3 SOL: the 15 owed is covered, nothing booked.
    assert_eq!(lot.written_off.0 + lot.insured.0, 0);
    let tab = lot.tab.0;
    assert!(tab >= 15 * WAD);
    // ZEC keeps falling, to $25: the lot is re-marked and the gap booked.
    let value = c.pool(SOL).value().unwrap();
    confirm(&mut c, 10, 2_500, 15_000);
    c.seal();
    let lot = c.vm.market.lend.lots[&pos];
    assert_eq!(lot.rate.0, 25 * WAD / 150, "re-marked to today's price");
    // 100 ZEC · 25/150 · 70% = 11.67 SOL against ~15: ~3.3 SOL booked.
    let booked = lot.written_off.0 + lot.insured.0;
    let expect = tab - 100 * (25 * WAD / 150) * 7_000 / 10_000 / 1;
    assert!((booked - expect).abs() < WAD / 1_000, "{booked} vs {expect}");
    assert!(c.pool(SOL).value().unwrap() < value, "lenders see it now");
    // A booked loss the lot may still recover keeps new deposits out.
    let (inputs, change) = c.fund(CAROL, SOL, Fixed::whole(1));
    let nonce = c.fresh();
    assert_eq!(
        c.refused(Op::LendSupply { lender: CAROL, program: program::lend_supply(), asset: SOL, amount: Fixed::whole(1), nonce, inputs, change }),
        "deposits into this pool wait while an auction recovers a booked loss"
    );
    // A year on the clock: the lot's debt does not grow, and no reserve is
    // taken from interest it will never pay.
    let owed = c.pool(SOL).owed(lot.debt_shares.0).unwrap();
    let (pot, ins) = (c.vm.market.pol.get(&SOL).map_or(0, |x| x.0), c.vm.market.lend.insurance.get(&SOL).map_or(0, |x| x.0));
    let t = c.now();
    c.tick(t + 31_536_000);
    c.prices_c(t + 31_536_001, 2_500, 15_000);
    c.seal();
    let lot = c.vm.market.lend.lots.get(&pos).copied().expect("still selling");
    assert!((c.pool(SOL).owed(lot.debt_shares.0).unwrap() - owed).abs() <= 1, "frozen");
    assert_eq!(c.vm.market.pol.get(&SOL).map_or(0, |x| x.0), pot);
    assert_eq!(c.vm.market.lend.insurance.get(&SOL).map_or(0, |x| x.0), ins);
}

/// Item 4: the backstop sells only what covers the lot, and the rest of the
/// collateral goes back to its owner.
#[test]
fn the_backstop_sells_only_what_covers_the_lot() {
    let mut c = v2();
    let pos = c.open(BOB, ZEC, Fixed::whole(300), SOL, Fixed::whole(15)).unwrap();
    c.claim(BOB, &[pos]);
    // $12: 300 ZEC is $3,600 at the 60% line = $2,160 < $2,250.
    confirm(&mut c, 10, 1_200, 15_000);
    c.seal();
    let lot = c.vm.market.lend.lots[&pos];
    let opened = lot.opened_at;
    // Keep prices live; nobody bids.
    let mut sold_at = None;
    for i in 1..=50u64 {
        c.prices_c(opened + i * 60, 1_200, 15_000);
        c.seal();
        let l = c.vm.market.lend.lots.get(&pos);
        if sold_at.is_none() && l.map_or(true, |l| l.proceeds.0 > 0) {
            sold_at = Some(c.now());
        }
        if l.is_none() {
            break;
        }
    }
    let at = sold_at.expect("the backstop sold");
    assert!(at >= opened + objectvm::lend::BACKSTOP_AFTER_SECS, "not before its time");
    assert!(!c.vm.market.lend.lots.contains_key(&pos), "covered and settled");
    assert_eq!(c.pool(SOL).debt_shares, 0);
    // What the backstop did not need came back to Bob.
    let back = c.vm.market.claims.get(&market::claim_id(&objectvm::lend::leftover_source(&pos), 1)).map_or(0, |x| x.amount.0);
    assert!(back > 0 && back < 300 * WAD, "returned {back}");
    c.claim(BOB, &[objectvm::lend::leftover_source(&pos)]);
}

/// Item 4: a backstop the pool will not fill books what is left.
#[test]
fn a_backstop_the_pool_will_not_fill_books_the_rest() {
    let mut c = v2();
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    confirm(&mut c, 10, 3_500, 15_000);
    c.seal();
    // Dave dumps ZEC into the pool: it now pays far under 70% of $35/$150.
    let pid = market::pool_id(&ZEC, &SOL);
    let p = c.vm.market.pools[&pid];
    let (inputs, change) = c.fund(DAVE, ZEC, Fixed::whole(1_900));
    let nonce = c.fresh();
    c.ok(Op::Swap { trader: DAVE, program: program::amm_swap(), pool: pid, sell_a: p.a == ZEC, amount_in: Fixed::whole(1_900), min_out: Fixed::ZERO, inputs, change, nonce });
    c.seal();
    let opened = c.vm.market.lend.lots[&pos].opened_at;
    let value = c.pool(SOL).value().unwrap();
    for i in 1..=40u64 {
        c.prices_c(opened + i * 60, 3_500, 15_000);
        c.seal();
    }
    let lot = c.vm.market.lend.lots[&pos];
    assert_eq!(lot.proceeds.0, 0, "nothing sold");
    assert_eq!(lot.collateral.0, 100 * WAD, "the collateral came back from the batch");
    assert_eq!(lot.debt_shares.0, 0, "the whole debt is booked");
    assert!(lot.written_off.0 >= 15 * WAD);
    assert!(c.pool(SOL).value().unwrap() < value);
    // A bidder at the floor still recovers it for the lenders.
    let price = c.vm.market.lend.auction_price(&lot, c.vm.epoch).unwrap();
    let pay = Fixed(zyn_lend::math::wmul_up(lot.collateral.0, price).unwrap());
    let before = c.pool(SOL).value().unwrap();
    c.bid(CAROL, &pos, pay).unwrap();
    c.seal();
    assert!(c.pool(SOL).value().unwrap() > before, "recovered");
}

/// Item 5: in a crash the worse price liquidates while the breaker holds,
/// three posts on one side confirm it, and lenders cannot leave meanwhile.
#[test]
fn a_crash_is_acted_on_while_the_breaker_holds_it() {
    let mut c = v2();
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(15)).unwrap();
    let carol = c.open(CAROL, ZEC, Fixed::whole(500), SOL, Fixed::whole(15)).unwrap();
    let t = c.now();
    // ZEC −30%, then −35%, −40%: never within 2% of the last, always down.
    c.prices_c(t + 30, 3_500, 15_000);
    assert_eq!(c.vm.market.lend.prices[&ZEC], usd(50), "held");
    // Lenders of SOL wait; Carol cannot take out ZEC collateral.
    let shares = c.balance(ALICE, lend_asset(&SOL));
    assert_eq!(c.withdraw(ALICE, SOL, Fixed(shares / 10)), Err("withdrawals from this pool wait for a held price to be confirmed"));
    assert_eq!(c.remove(CAROL, &carol, Fixed::whole(1)), Err("prices are stale or still moving; taking collateral out waits for the next confirmed price"));
    c.seal();
    assert!(c.vm.market.lend.lots.contains_key(&pos), "liquidated on the held price, the worse one");
    assert!(c.vm.market.lend.positions.contains_key(&carol));
    c.prices_c(t + 60, 3_250, 15_000);
    assert!(c.vm.market.lend.pending.contains_key(&ZEC));
    c.prices_c(t + 90, 3_000, 15_000);
    assert_eq!(c.vm.market.lend.prices[&ZEC].0, 30 * WAD, "three posts down: confirmed");
    assert!(c.vm.market.lend.pending.is_empty());
    assert!(c.withdraw(ALICE, SOL, Fixed(shares / 10)).is_ok());
    // A held move in the other direction starts its own count.
    c.prices_c(t + 120, 4_000, 15_000);
    c.prices_c(t + 150, 2_000, 15_000);
    assert_eq!(c.vm.market.lend.held_posts[&ZEC], 1);
}

/// Items 6 and 7: caps and the depth limit hold when collateral is added;
/// collateral comes out only where the pool agrees with the price.
#[test]
fn collateral_obeys_caps_depth_and_the_pool_both_ways() {
    let mut c = v2();
    // 25% of the pool's 3,000 ZEC: 750 ZEC may back SOL loans.
    assert_eq!(c.vm.depth_room(&ZEC, &SOL).map(|x| x.1), Some(750 * WAD));
    assert_eq!(c.open(BOB, ZEC, Fixed::whole(751), SOL, Fixed::whole(1)), Err("this pair is at its depth limit: collateral may be at most a quarter of Zyn's pool"));
    let pos = c.open(BOB, ZEC, Fixed::whole(700), SOL, Fixed::whole(1)).unwrap();
    assert_eq!(c.add(BOB, &pos, ZEC, Fixed::whole(51)), Err("this pair is at its depth limit: collateral may be at most a quarter of Zyn's pool"));
    assert!(c.add(BOB, &pos, ZEC, Fixed::whole(50)).is_ok());
    c.limits(ZEC, 760, 10_000, 1);
    let pool = c.vm.depth_room(&ZEC, &SOL).unwrap();
    assert_eq!(pool.2, 0);
    // The cap binds on adding too (the depth limit is lifted by a deeper pool).
    let pid = market::pool_id(&ZEC, &SOL);
    let p = c.vm.market.pools[&pid];
    let (ia, mut ch) = c.fund(DAVE, ZEC, Fixed::whole(300));
    let (ib, chb) = c.fund(DAVE, SOL, Fixed::whole(100));
    ch.extend(chb);
    let nonce = c.fresh();
    let (amount_a, amount_b) = if p.a == ZEC { (Fixed::whole(300), Fixed::whole(100)) } else { (Fixed::whole(100), Fixed::whole(300)) };
    c.ok(Op::AddLiquidity { provider: DAVE, program: program::add_liquidity(), pool: pid, amount_a, amount_b, min_shares: Fixed::ZERO, inputs: [ia, ib].concat(), change: ch, nonce });
    assert_eq!(c.add(BOB, &pos, ZEC, Fixed::whole(11)), Err("this collateral is at its cap"));
    // The pool moves away from the posted price: collateral stays in.
    let t = c.now();
    c.prices_c(t + 10, 6_000, 15_000);
    c.prices_c(t + 11, 6_000, 15_000);
    assert_eq!(c.remove(BOB, &pos, Fixed::whole(1)), Err("taking collateral out is paused: Zyn's pool and the posted price disagree"));
    c.prices_c(t + 12, 5_100, 15_000);
    assert!(c.remove(BOB, &pos, Fixed::whole(1)).is_ok());
}

/// Item 9: interest runs on the clock, at v2's curve: 8% at 80% used.
#[test]
fn interest_runs_on_the_clock_at_the_v2_curve() {
    let mut c = v2();
    c.open(BOB, ZEC, Fixed::whole(450), SOL, Fixed::whole(80)).unwrap();
    let before = c.pool(SOL).debt().unwrap();
    let t = c.now();
    c.tick(t + 31_536_000);
    let interest = c.pool(SOL).debt().unwrap() - before;
    let expect = 80 * WAD * 8 / 100;
    assert!((interest - expect).abs() < WAD / 100, "{interest} vs {expect}");
    assert!(!c.vm.market.lend.fresh(&ZEC, c.vm.epoch), "the clock aged the prices; it did not post any");
}

/// Item 11: anyone may add to a pool's insurance.
#[test]
fn anyone_may_add_to_insurance() {
    let mut c = market();
    let (inputs, change) = c.fund(CAROL, SOL, Fixed::whole(2));
    let op = Op::LendInsure { donor: CAROL, program: program::lend_insure(), asset: SOL, amount: Fixed::whole(2), inputs, change };
    assert_eq!(c.refused(op), "insurance takes deposits under rules v2 only");
    let mut c = v2();
    let (inputs, change) = c.fund(CAROL, SOL, Fixed::whole(2));
    c.ok(Op::LendInsure { donor: CAROL, program: program::lend_insure(), asset: SOL, amount: Fixed::whole(2), inputs, change });
    assert_eq!(c.vm.market.lend.insurance[&SOL], Fixed::whole(2));
    assert_eq!(c.balance(CAROL, SOL), 998 * WAD);
}

/// Item 12: a part payment leaves at least the smallest loan, or nothing.
#[test]
fn a_part_payment_leaves_the_smallest_loan_or_nothing() {
    let mut c = v2();
    c.limits(SOL, 10_000, 10_000, 100);
    let pos = c.open(BOB, ZEC, Fixed::whole(100), SOL, Fixed::whole(1)).unwrap();
    c.claim(BOB, &[pos]);
    // 1 SOL is $150; paying 0.5 leaves $75, under the $100 minimum.
    assert_eq!(c.repay(BOB, &pos, Fixed(WAD / 2)), Err("repay all of it, or leave at least the smallest loan"));
    assert!(c.repay(BOB, &pos, Fixed(WAD / 5)).is_ok());
    let owed = c.pool(SOL).owed(c.vm.market.lend.positions[&pos].debt_shares.0).unwrap();
    assert!(c.repay(BOB, &pos, Fixed(owed)).is_ok());
    assert_eq!(c.vm.market.lend.positions[&pos].debt_shares.0, 0);
}

/// Item 1, the node's half: refused intents are sealed with the rest but do
/// not bring the 64-intent seal closer. Node policy, not consensus: seals
/// are journaled, and replicas replay them as they were.
#[test]
fn refused_intents_do_not_hurry_a_seal() {
    use zyn::epoch::{Economics, EpochPolicy};
    use zyn::node::Node;
    let policy = EpochPolicy { intents_per_epoch: 4, epochs_per_anchor: u64::MAX, max_seconds_per_epoch: 10, max_seconds_per_anchor: 0 };
    let mut n: Node<ObjectVm> = Node::new(212, Limits { max_io: 16 }, policy, Economics::flat(1)).with_refusals_uncounted();
    for i in 0..20u64 {
        let step = n.submit_operator(Op::LendTime { program: program::lend_prices(), at: i }, 0);
        assert!(step.receipts.iter().all(ObjectVm::rejected));
        assert!(step.sealed.is_none(), "a refused intent sealed the epoch");
    }
    for k in 1..=4u8 {
        let step = n.submit_operator(Op::Mint { asset: ZEC, amount: Fixed::whole(1), owner: ALICE, nonce: [k; 32] }, 0);
        assert_eq!(step.sealed.is_some(), k == 4);
    }
    // Refused intents alone still seal on the clock.
    n.submit_operator(Op::LendTime { program: program::lend_prices(), at: 0 }, 1);
    assert!(n.seal_if_due(5).is_none());
    assert!(n.seal_if_due(11).is_some());
}

/// The node's start under `ZYN_LEND_RULES=2`, on a chain like testnet's:
/// guarded, ETH listed at its old risk. List again at v2's risk, switch, post
/// the clock: prices from before a long restart are stale until the first
/// post after it (item 14).
#[test]
fn the_nodes_switch_on_a_guarded_chain_with_old_eth_risk() {
    let mut c = history_guarded();
    const ETH: [u8; 32] = [0xE0; 32];
    c.mint(DAVE, ETH, 10);
    c.ok(Op::LendList { program: program::lend_list(), asset: ETH, max_ltv_bps: 7_500, liquidation_bps: 8_000, penalty_bps: 500 });
    c.limits(ETH, 2, 1, 1);
    let risk = zyn_lend::params::v2::ETH;
    c.ok(Op::LendList { program: program::lend_list(), asset: ETH, max_ltv_bps: risk.max_ltv_bps, liquidation_bps: risk.liquidation_bps, penalty_bps: risk.penalty_bps });
    c.ok(Op::LendRules { program: program::lend_list(), version: 2 });
    let t = c.now();
    c.tick(t + 3_600);
    assert!(!c.vm.market.lend.fresh(&ZEC, c.vm.epoch), "an hour down: stale");
    c.prices(t + 3_601, 50, 150);
    assert!(c.vm.market.lend.fresh(&ZEC, c.vm.epoch));
    c.seal();
    let back = ObjectVm::decode(&c.vm.encode()).unwrap();
    assert_eq!(back.state_root(), c.vm.state_root());
}

// ------------------------------------------------------------- BOLD (§162)

/// One deposit through the bridge and its claim: `whole` in, less the
/// bridge fee, as a note of `who`'s.
fn bridge_in(c: &mut Chain, who: AccountId, asset: [u8; 32], whole: i64) {
    use objectvm::bridge::{deposit_commitment, deposit_key};
    let secret = c.fresh();
    let commitment = deposit_commitment(&who, &secret);
    let external_ref = c.fresh();
    c.ok(Op::DepositCommitted { program: program::deposit(), asset, gross: Fixed::whole(whole), external_ref, commitment });
    let amount = c.vm.market.waiting[&deposit_key(&commitment, &asset)].amount;
    let output = c.note(who, asset, amount);
    c.ok(Op::ClaimDeposit { owner: who, program: program::claim(), secret, asset, output });
}

/// BOLD.zy listed on a ledger already under rules v2 — once it exists, not
/// before — then lent, and borrowed against ETH.zy (and ETH.zy against it)
/// through Zyn's ETH/BOLD pool. Priced at its traded value: a depeg
/// liquidates like any other fall.
#[test]
fn bold_lists_under_rules_v2_and_lends_against_eth() {
    use objectvm::bridge::asset_of;
    let mut c = v2();
    let sepolia = b"eip155:11155111".to_vec();
    let test_bold: Vec<u8> = (0..20).map(|i| u8::from_str_radix(&"d58c9e3eebe83fda55d59c186e86b37aa5d0f79a"[2 * i..2 * i + 2], 16).unwrap()).collect();
    let eth = asset_of(&sepolia, b"");
    let bold = asset_of(&sepolia, &test_bold);
    assert_eq!(hex(&bold), "8c1a1b0616cef63250e9f5bab8b20908320cc47ead0f60215a229d34b5c93ac2");
    for origin in [Vec::new(), test_bold.clone()] {
        c.ok(Op::RegisterBridged { program: program::bridge_register(), network: sepolia.clone(), origin, decimals: 18 });
    }
    let risk = zyn_lend::params::v2::BOLD;
    let list = |asset: [u8; 32], r: zyn_lend::Risk| Op::LendList { program: program::lend_list(), asset, max_ltv_bps: r.max_ltv_bps, liquidation_bps: r.liquidation_bps, penalty_bps: r.penalty_bps };
    // Registered, never deposited: nothing to lend yet (the node waits for it).
    assert_eq!(c.refused(list(bold, risk)), "only an asset that exists and was not made here can be lent");
    for who in [ALICE, BOB, CAROL, DAVE] {
        bridge_in(&mut c, who, bold, 1_000_000);
        bridge_in(&mut c, who, eth, 300);
    }
    // Zyn's ETH/BOLD pool: 200 ETH to 800,000 BOLD.
    let (ia, mut ch) = c.fund(DAVE, eth, Fixed::whole(200));
    let (ib, chb) = c.fund(DAVE, bold, Fixed::whole(800_000));
    ch.extend(chb);
    let nonce = c.fresh();
    let (a, b, amount_a, amount_b) = if eth < bold { (eth, bold, 200, 800_000) } else { (bold, eth, 800_000, 200) };
    c.ok(Op::CreatePool { provider: DAVE, program: program::add_liquidity(), asset_a: a, asset_b: b, amount_a: Fixed::whole(amount_a), amount_b: Fixed::whole(amount_b), fee_bps: 30, inputs: [ia, ib].concat(), change: ch, nonce });
    // Rules v2 refuse BOLD at numbers that do not fit under the floor.
    assert_eq!(c.refused(list(bold, zyn_lend::Risk { liquidation_bps: 7_900, ..risk })), "under rules v2 a liquidated debt and its penalty must fit under the auction floor");
    c.ok(list(bold, risk));
    c.ok(list(eth, zyn_lend::params::v2::ETH));
    c.limits(bold, 5_000, 2_500, 1);
    c.limits(eth, 10, 5, 1);
    assert_eq!(c.vm.market.lend.listed[&bold].risk, risk);
    let post = |c: &mut Chain, eth_usd: i128, bold_micro: i128| {
        let at = c.now() + 1;
        c.ok(Op::LendPrices { program: program::lend_prices(), at, prices: vec![(eth, usd(eth_usd)), (bold, Fixed(bold_micro * WAD / 1_000_000))] });
    };
    post(&mut c, 4_000, 995_800);
    // Alice lends BOLD, Dave lends ETH.
    let src = c.supply(ALICE, bold, 3_000);
    c.claim(ALICE, &[src]);
    let src = c.supply(DAVE, eth, 5);
    c.claim(DAVE, &[src]);
    // Bob borrows 2,000 BOLD on 1 ETH ($4,000): 49.8%, under ETH's 72%.
    let bob = c.open(BOB, eth, Fixed::whole(1), bold, Fixed::whole(2_000)).unwrap();
    c.claim(BOB, &[bob]);
    assert_eq!(c.open(BOB, eth, Fixed::whole(1), bold, Fixed::whole(600)), Err("this pool is at its borrowing cap"), "2,500 BOLD borrow cap");
    // Carol borrows 0.5 ETH on 3,000 BOLD ($2,987): 67%, under BOLD's 75%.
    assert!(c.open(CAROL, bold, Fixed::whole(3_000), eth, Fixed(WAD * 3 / 4)).is_err(), "75% of $2,987 is less than 0.75 ETH");
    let carol = c.open(CAROL, bold, Fixed::whole(3_000), eth, Fixed(WAD / 2)).unwrap();
    c.claim(CAROL, &[carol]);
    c.seal();
    assert!(c.vm.market.lend.positions.contains_key(&carol));
    // BOLD trades at 85 cents: $2,550 behind a $2,000 debt is 78.4%, past
    // BOLD's 78% line. A constant dollar would never have seen it.
    post(&mut c, 4_000, 850_000);
    post(&mut c, 4_000, 850_000);
    c.seal();
    assert!(c.vm.market.lend.lots.contains_key(&carol), "liquidated on BOLD's own price");
    assert!(c.vm.market.lend.positions.contains_key(&bob), "Bob's BOLD debt got cheaper, not riskier");
}
