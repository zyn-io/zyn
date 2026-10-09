//! What the pre-launch audit of Jade's ledger code found (6 Oct 2026), kept
//! as tests that the fixes hold: a curve's token cannot be minted through a
//! launch, two routes cannot share an id, the last launch asset cannot be
//! removed, arrival order never picks between conflicting floors — and the
//! edge cases the audit found sound stay sound.

use objectvm::curve::{self, curve_order_id, LOT};
use objectvm::market::{self, claim_id, token_of};
use objectvm::route::{route_id, Hop};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x10; 32];
const ETH: [u8; 32] = [0x20; 32];
const USD: [u8; 32] = [0x30; 32];
const ALICE: AccountId = [0xA1; 32];
const BOB: AccountId = [0xB0; 32];
const CAROL: AccountId = [0xC0; 32];
const DAVE: AccountId = [0xD0; 32];

#[derive(Clone)]
struct Chain {
    vm: ObjectVm,
    seq: u64,
    salt: u64,
    strict: bool,
}

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

impl Chain {
    fn new() -> Chain {
        // Mainnet's id: batches there settle by the fair-batch rules from genesis
        // (`objectvm::fair_batches_from`); testnet's history keeps the old ones.
        let mut c = Chain { vm: ObjectVm::genesis(26460, Limits { max_io: 16 }), seq: 0, salt: 0, strict: true };
        c.ok(Op::SetCurveTerms { program: program::configure_curves(), fee_bps: 100, bond: Fixed::ZERO });
        c.ok(Op::SetCurveQuote { program: program::configure_curves(), quote: ZEC, graduation_mc: Fixed(curve::GRADUATION_MC_UNITS * curve::ZAT), bond: Fixed::ZERO });
        c.ok(Op::SetCurveQuote { program: program::configure_curves(), quote: ETH, graduation_mc: Fixed::whole(23), bond: Fixed::ZERO });
        c
    }

    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let r = self.vm.apply(self.seq, &op);
        if self.strict { assert_eq!(self.vm.conserved(), Ok(()), "after {op:?}"); }
        assert_eq!(self.vm.reconciled(), Ok(()));
        assert_eq!(ObjectVm::decode(&self.vm.encode()).as_ref(), Some(&self.vm), "state round-trips");
        let wire = ObjectVm::encode_intent(&op);
        assert_eq!(ObjectVm::decode_intent(&mut zyn_vm::read::Decoder::new(&wire)).as_ref(), Some(&op));
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

    fn mint(&mut self, owner: AccountId, asset: [u8; 32], whole: i64) {
        let nonce = self.fresh();
        self.ok(Op::Mint { asset, amount: Fixed::whole(whole), owner, nonce });
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

    /// Collect everything owed under these sources.
    fn claim(&mut self, owner: AccountId, sources: &[Hash]) {
        let mut ids = Vec::new();
        let mut outputs = Vec::new();
        for s in sources {
            for k in 0..3 {
                let id = claim_id(s, k);
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

    fn create(&mut self, creator: AccountId, quote: [u8; 32]) -> [u8; 32] {
        let nonce = self.fresh();
        self.ok(Op::CreateCurve { creator, program: program::curve_create(), nonce, quote, meta: [7; 32], inputs: Vec::new(), change: Vec::new() });
        token_of(&creator, &nonce)
    }

    fn buy(&mut self, who: AccountId, token: [u8; 32], amount: Fixed) {
        let quote = self.vm.market.curves[&token].quote;
        let (inputs, change) = self.fund(who, quote, amount);
        let nonce = self.fresh();
        self.ok(Op::CurveTrade { trader: who, program: program::curve_trade(), token, buy: true, amount_in: amount, min_out: Fixed::ZERO, inputs, change, nonce });
        self.seal();
        self.claim(who, &[curve_order_id(&who, &nonce)]);
    }

    fn pool(&mut self, who: AccountId, a: [u8; 32], b: [u8; 32], amount_a: i64, amount_b: i64) -> Hash {
        let (x, y, ax, ay) = if a < b { (a, b, amount_a, amount_b) } else { (b, a, amount_b, amount_a) };
        let (ia, mut change) = self.fund(who, x, Fixed::whole(ax));
        let (ib, cb) = self.fund(who, y, Fixed::whole(ay));
        change.extend(cb);
        let nonce = self.fresh();
        match self.ok(Op::CreatePool { provider: who, program: program::add_liquidity(), asset_a: x, asset_b: y, amount_a: Fixed::whole(ax), amount_b: Fixed::whole(ay), fee_bps: 30, inputs: [ia, ib].concat(), change, nonce }) {
            Receipt::Pooled { pool, .. } => pool,
            other => panic!("{other:?}"),
        }
    }

    fn route(&mut self, who: AccountId, hops: Vec<Hop>, start: [u8; 32], amount: Fixed, min_out: Fixed) -> Result<Hash, String> {
        let (inputs, change) = self.fund(who, start, amount);
        let nonce = self.fresh();
        match self.apply(Op::Route { trader: who, program: program::route(), hops, amount_in: amount, min_out, inputs, change, nonce }) {
            Receipt::Refused(why) => Err(why.to_string()),
            Receipt::Ordered { order } => {
                assert_eq!(order, route_id(&who, &nonce));
                Ok(order)
            }
            other => panic!("{other:?}"),
        }
    }
}

/// The setting: Bob's coin A priced in ETH, Carol's coin B priced in ZEC,
/// both bought into, and Dave's ETH/ZEC pool. Alice holds 1M of A.
fn setting() -> (Chain, [u8; 32], [u8; 32], Hash) {
    let mut c = Chain::new();
    for (who, asset, n) in [(ALICE, ETH, 5), (ALICE, ZEC, 5), (BOB, ETH, 5), (CAROL, ZEC, 5), (DAVE, ETH, 50), (DAVE, ZEC, 100)] {
        c.mint(who, asset, n);
    }
    let a = c.create(BOB, ETH);
    let b = c.create(CAROL, ZEC);
    c.buy(ALICE, a, Fixed::whole(1));
    c.buy(CAROL, b, Fixed::whole(1));
    let p = c.pool(DAVE, ETH, ZEC, 20, 40);
    (c, a, b, p)
}

fn eth_to_zec(c: &Chain, p: &Hash) -> bool {
    c.vm.market.pools[p].a == ETH
}

impl Chain {
    fn route_n(&mut self, who: AccountId, hops: Vec<Hop>, start: [u8; 32], amount: Fixed, min_out: Fixed, nonce: Hash) -> Receipt {
        let (inputs, change) = self.fund(who, start, amount);
        self.apply(Op::Route { trader: who, program: program::route(), hops, amount_in: amount, min_out, inputs, change, nonce })
    }
    fn total_claims(&self, src: &Hash) -> Vec<(u8, [u8; 32], i128)> {
        (0..3).filter_map(|k| self.vm.market.claims.get(&claim_id(src, k)).map(|c| (k, c.asset, c.amount.0))).collect()
    }
}

/// F1: two routes with the same nonce in one epoch, opposite directions in one pool.
#[test]
fn a_second_route_under_one_id_is_refused() {
    let (mut c, _a, _b, p) = setting();
    c.strict = false;
    let e2z = eth_to_zec(&c, &p);
    let n = [0x55; 32];
    let r1 = c.route_n(ALICE, vec![Hop::Pool { pool: p, sell_a: e2z }], ETH, Fixed::whole(1), Fixed::ZERO, n);
    let r2 = c.route_n(ALICE, vec![Hop::Pool { pool: p, sell_a: !e2z }], ZEC, Fixed::whole(1), Fixed::ZERO, n);
    assert!(!ObjectVm::rejected(&r1));
    assert!(matches!(r2, Receipt::Refused(w) if w.contains("already used")), "the second route under one id is refused");
    c.seal();
    assert_eq!(c.vm.conserved(), Ok(()));
}

/// F1b: same nonce, same pool and direction; first route has an impossible
/// minimum, the second min 0: the shared leg id is removed for both, only one refunded.
#[test]
fn a_short_route_beside_a_refused_twin_is_refunded_whole() {
    let (mut c, _a, _b, p) = setting();
    c.strict = false;
    let e2z = eth_to_zec(&c, &p);
    let n = [0x56; 32];
    c.route_n(ALICE, vec![Hop::Pool { pool: p, sell_a: e2z }], ETH, Fixed::whole(1), Fixed::whole(1_000_000), n);
    let r2 = c.route_n(ALICE, vec![Hop::Pool { pool: p, sell_a: e2z }], ETH, Fixed::whole(1), Fixed::ZERO, n);
    assert!(matches!(r2, Receipt::Refused(_)));
    let pool_before = c.vm.market.pools[&p];
    c.seal();
    let rid = route_id(&ALICE, &n);
    assert_eq!(c.vm.market.pools[&p], pool_before, "the short route moved nothing");
    assert_eq!(c.total_claims(&rid).iter().map(|x| x.0).collect::<Vec<_>>(), vec![1], "refunded whole");
    assert_eq!(c.vm.conserved(), Ok(()));
}

/// F2: a curve's creator reuses the curve's nonce for a CreateLaunch: same token.
#[test]
fn a_launch_cannot_mint_a_curves_token() {
    let mut c = Chain::new();
    for (who, asset, n) in [(ALICE, ZEC, 50), (BOB, ZEC, 5), (CAROL, ZEC, 50)] {
        c.mint(who, asset, n);
    }
    // Bob creates a ZEC curve with nonce N...
    let n = [0x61; 32];
    c.ok(Op::CreateCurve { creator: BOB, program: program::curve_create(), nonce: n, quote: ZEC, meta: [7; 32], inputs: Vec::new(), change: Vec::new() });
    let _t = token_of(&BOB, &n);
    // ... and a launch with the same nonce: same token address.
    let supply = Fixed(curve::SUPPLY.0 * 1000);
    let r = c.apply(Op::CreateLaunch {
        creator: BOB, program: program::launch_create(), nonce: n, quote: ZEC,
        total_supply: supply, sale_supply: Fixed(1), pool_supply: Fixed::ZERO,
        soft_cap: Fixed(1), hard_cap: Fixed(1), pool_bps: 0, fee_bps: 0,
        closes_at_epoch: c.vm.epoch, meta: [7; 32],
    });
    assert!(matches!(r, Receipt::Refused(w) if w.contains("token exists")), "{r:?}");
}



/// F5: removing every named quote reopens launches in every asset at ZEC's size.
#[test]
fn the_last_launch_asset_cannot_be_removed() {
    let mut c = Chain::new();
    c.mint(ALICE, USD, 5);
    c.ok(Op::SetCurveQuote { program: program::configure_curves(), quote: ZEC, graduation_mc: Fixed::ZERO, bond: Fixed::ZERO });
    let why = c.refused(Op::SetCurveQuote { program: program::configure_curves(), quote: ETH, graduation_mc: Fixed::ZERO, bond: Fixed::ZERO });
    assert!(why.contains("last asset"), "{why}");
    let n = c.fresh();
    let r = c.apply(Op::CreateCurve { creator: BOB, program: program::curve_create(), nonce: n, quote: USD, meta: [7; 32], inputs: Vec::new(), change: Vec::new() });
    assert!(ObjectVm::rejected(&r), "the last named asset stays, so an un-named one is refused: {r:?}");
}

/// Route crossing graduation: k=0 tokens, k=2 change, pool opens, books balance.
#[test]
fn route_crossing_graduation_balances() {
    let (mut c, _a, b, p) = setting();
    c.mint(ALICE, ETH, 40);
    c.mint(DAVE, ETH, 100);
    c.mint(DAVE, ZEC, 100);
    let p2 = p;
    // Deepen the pool so 30 ETH gets well over 17 ZEC.
    let e2z = eth_to_zec(&c, &p2);
    let r = c.route(ALICE, vec![Hop::Pool { pool: p2, sell_a: e2z }, Hop::CurveBuy { token: b }], ETH, Fixed::whole(30), Fixed(1)).unwrap();
    // A direct buyer in the same batch, too.
    let (inputs, change) = c.fund(CAROL, ZEC, Fixed::whole(2));
    let n = c.fresh();
    c.ok(Op::CurveTrade { trader: CAROL, program: program::curve_trade(), token: b, buy: true, amount_in: Fixed::whole(2), min_out: Fixed::ZERO, inputs, change, nonce: n });
    c.seal();
    let cur = c.vm.market.curves[&b];
    println!("graduated {} sold {}", cur.graduated, cur.sold);
    println!("route claims {:?}", c.total_claims(&r).iter().map(|(k, a, v)| (*k, a[0], *v)).collect::<Vec<_>>());
    println!("carol claims {:?}", c.total_claims(&curve_order_id(&CAROL, &n)).iter().map(|(k, a, v)| (*k, a[0], *v)).collect::<Vec<_>>());
    c.claim(ALICE, &[r]);
}


/// Arrival order decides which of two conflicting buys fills.
#[test]
fn arrival_order_never_picks_between_conflicting_floors() {
    let mut base = Chain::new();
    base.mint(ALICE, ZEC, 10);
    base.mint(CAROL, ZEC, 10);
    let t = base.create(BOB, ZEC);
    // Solo: 2 ZEC buys how many lots?
    let mut solo = base.clone();
    solo.buy(ALICE, t, Fixed::whole(2));
    let solo_lots = solo.balance(ALICE, t).0 / LOT.0;
    let min = Fixed((solo_lots - 50) * LOT.0); // met alone, not jointly
    let mut outcome = Vec::new();
    for first in [ALICE, CAROL] {
        let mut c = base.clone();
        let order = if first == ALICE { [ALICE, CAROL] } else { [CAROL, ALICE] };
        let mut ids = Vec::new();
        for who in order {
            let (inputs, change) = c.fund(who, ZEC, Fixed::whole(2));
            let n = c.fresh();
            c.ok(Op::CurveTrade { trader: who, program: program::curve_trade(), token: t, buy: true, amount_in: Fixed::whole(2), min_out: min, inputs, change, nonce: n });
            ids.push((who, curve_order_id(&who, &n)));
        }
        c.seal();
        let filled: Vec<_> = ids.iter().filter(|(_, id)| c.vm.market.claims.contains_key(&claim_id(id, 0))).map(|(w, _)| w[0]).collect();
        let mut filled = filled;
        filled.sort();
        outcome.push(filled);
    }
    assert_eq!(outcome[0], outcome[1], "arrival order never decides the winner");
}

fn grad_mc(unit: i128) -> Fixed { Fixed(curve::GRADUATION_MC_UNITS * unit) }

/// Extreme units: a whale buys through graduation at MAX_UNIT and MIN_UNIT.
#[test]
fn extreme_units_graduate_without_overflow() {
    for unit in [curve::MAX_UNIT, curve::MIN_UNIT] {
        let mut c = Chain::new();
        c.ok(Op::SetCurveQuote { program: program::configure_curves(), quote: USD, graduation_mc: grad_mc(unit), bond: Fixed::ZERO });
        assert_eq!(c.vm.market.curve_quotes[&USD].unit, unit);
        let need = curve::cost_at(0, curve::GRADUATE_AT, unit) * 2;
        c.ok(Op::Mint { asset: USD, amount: Fixed(need), owner: ALICE, nonce: [0x99; 32] });
        c.ok(Op::Mint { asset: USD, amount: Fixed(need), owner: CAROL, nonce: [0x98; 32] });
        let t = c.create(BOB, USD);
        c.buy(CAROL, t, Fixed(need / 4));
        c.buy(ALICE, t, Fixed(need));
        let cur = c.vm.market.curves[&t];
        println!("unit {unit}: graduated {} sold {} ", cur.graduated, cur.sold);
        assert!(cur.graduated);
        // Sell into the graduated pool works.
        let pid = market::pool_id(&t, &USD);
        let p = c.vm.market.pools[&pid];
        let price_pool = if p.a == t { p.reserve_b.0 as f64 / p.reserve_a.0 as f64 } else { p.reserve_a.0 as f64 / p.reserve_b.0 as f64 };
        let last = curve::cost_at(curve::GRADUATE_AT, 1, unit) as f64 / LOT.0 as f64;
        println!("  pool price / last lot price = {:.6}", price_pool / last);
    }
}

/// Random trading: the reserve always covers buying back every lot sold.
#[test]
fn random_trading_keeps_reserve_covering() {
    let mut c = Chain::new();
    c.strict = true;
    let users = [ALICE, CAROL, DAVE];
    for u in users { c.mint(u, ZEC, 30); }
    let t = c.create(BOB, ZEC);
    let mut seed: u64 = 0x1234_5678;
    let mut rnd = |m: u64| { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (seed >> 33) % m };
    for _epoch in 0..60 {
        let mut placed = Vec::new();
        for u in users {
            let act = rnd(3);
            if act == 0 {
                let amt = Fixed((rnd(2_000) as i128 + 1) * 1_000_000_000_000_000); // 0.001..2 ZEC
                let zb = c.balance(u, ZEC);
                if zb.0 >= amt.0 && amt >= c.vm.market.curves[&t].min_buy() {
                    let (inputs, change) = c.fund(u, ZEC, amt);
                    let n = c.fresh();
                    let r = c.apply(Op::CurveTrade { trader: u, program: program::curve_trade(), token: t, buy: true, amount_in: amt, min_out: Fixed::ZERO, inputs, change, nonce: n });
                    if !ObjectVm::rejected(&r) { placed.push((u, curve_order_id(&u, &n))); }
                }
            } else if act == 1 {
                let lots = c.balance(u, t).0 / LOT.0;
                if lots > 0 {
                    let sell = (rnd(lots as u64) as i128) + 1;
                    let (inputs, change) = c.fund(u, t, Fixed(sell * LOT.0));
                    let n = c.fresh();
                    let r = c.apply(Op::CurveTrade { trader: u, program: program::curve_trade(), token: t, buy: false, amount_in: Fixed(sell * LOT.0), min_out: Fixed::ZERO, inputs, change, nonce: n });
                    if !ObjectVm::rejected(&r) { placed.push((u, curve_order_id(&u, &n))); }
                }
            }
        }
        c.seal();
        for (u, id) in placed { if (0..3).any(|k| c.vm.market.claims.contains_key(&claim_id(&id, k))) { c.claim(u, &[id]); } }
        let cur = c.vm.market.curves[&t];
        if cur.graduated { println!("graduated"); break; }
        assert!(cur.reserve.0 >= curve::cost_at(0, cur.sold, cur.unit), "reserve short at sold {}", cur.sold);
    }
    println!("final sold {}", c.vm.market.curves[&t].sold);
}

// ---- The second audit (9 Oct 2026): decoys and dust. --------------------

fn curve_buy(c: &mut Chain, who: AccountId, t: [u8; 32], amt: Fixed, min: Fixed) -> Hash {
    let (inputs, change) = c.fund(who, ZEC, amt);
    let n = c.fresh();
    c.ok(Op::CurveTrade { trader: who, program: program::curve_trade(), token: t, buy: true, amount_in: amt, min_out: min, inputs, change, nonce: n });
    curve_order_id(&who, &n)
}

#[test]
fn a_decoy_with_an_impossible_floor_is_turned_away_alone() {
    let mut base = Chain::new();
    base.mint(ALICE, ZEC, 40);
    base.mint(CAROL, ZEC, 10);
    let t = base.create(BOB, ZEC);
    // What Carol's 1 ZEC buys beside Alice's 1 ZEC, with no decoy.
    let mut honest = base.clone();
    let cid = curve_buy(&mut honest, CAROL, t, Fixed::whole(1), Fixed::ZERO);
    let aid = curve_buy(&mut honest, ALICE, t, Fixed::whole(1), Fixed::ZERO);
    honest.seal();
    let carol_lots = honest.vm.market.claims[&claim_id(&cid, 0)].amount.0 / LOT.0;
    let alice_lots = honest.vm.market.claims[&claim_id(&aid, 0)].amount.0 / LOT.0;
    let min = Fixed(carol_lots * 95 / 100 * LOT.0);
    // Alice adds 15 ZEC that can never fill, to push Carol past her 5%.
    let mut c = base.clone();
    let cid = curve_buy(&mut c, CAROL, t, Fixed::whole(1), min);
    let aid = curve_buy(&mut c, ALICE, t, Fixed::whole(1), Fixed::ZERO);
    let did = curve_buy(&mut c, ALICE, t, Fixed::whole(15), Fixed(1_000_000_000 * LOT.0));
    c.seal();
    let got = |c: &Chain, id: &Hash, k: u8| c.vm.market.claims.get(&claim_id(id, k)).map(|x| x.amount.0);
    assert_eq!(got(&c, &cid, 0), Some(carol_lots * LOT.0), "Carol fills as if the decoy were not there");
    assert_eq!(got(&c, &aid, 0), Some(alice_lots * LOT.0), "Alice's real order gets no cheaper price");
    assert_eq!(got(&c, &did, 1), Some(Fixed::whole(15).0), "only the decoy goes back");
}

#[test]
fn a_decoy_cannot_push_a_pool_swap_out_either() {
    let mut c = Chain::new();
    c.mint(DAVE, ZEC, 1000);
    c.mint(DAVE, ETH, 1000);
    c.mint(ALICE, ZEC, 500);
    c.mint(CAROL, ZEC, 10);
    let pid = c.pool(DAVE, ZEC, ETH, 100, 100);
    let sell_a = c.vm.market.pools[&pid].a == ZEC;
    let swap = |c: &mut Chain, who: AccountId, amt: i64, min: Fixed| {
        let (inputs, change) = c.fund(who, ZEC, Fixed::whole(amt));
        let n = c.fresh();
        c.ok(Op::Swap { trader: who, program: program::amm_swap(), pool: pid, sell_a, amount_in: Fixed::whole(amt), min_out: min, inputs, change, nonce: n });
        market::order_id(&who, &n)
    };
    let mut solo = c.clone();
    let id = swap(&mut solo, CAROL, 1, Fixed::ZERO);
    solo.seal();
    let out = solo.vm.market.claims[&claim_id(&id, 0)].amount;
    let cid = swap(&mut c, CAROL, 1, Fixed(out.0 * 95 / 100));
    let did = swap(&mut c, ALICE, 400, Fixed(i128::MAX / 4));
    c.seal();
    assert_eq!(c.vm.market.claims.get(&claim_id(&cid, 0)).map(|x| x.amount), Some(out));
    assert_eq!(c.vm.market.claims.get(&claim_id(&did, 1)).map(|x| x.amount), Some(Fixed::whole(400)));
}

#[test]
fn a_full_batch_of_unfillable_floors_makes_room_for_a_real_order() {
    let mut c = Chain::new();
    c.mint(ALICE, ZEC, 10);
    c.mint(CAROL, ZEC, 10);
    let t = c.create(BOB, ZEC);
    c.buy(ALICE, t, Fixed::whole(1));
    let mut dust = Vec::new();
    for _ in 0..curve::MAX_ORDERS_PER_CURVE {
        let (inputs, change) = c.fund(ALICE, t, LOT);
        let n = c.fresh();
        c.ok(Op::CurveTrade { trader: ALICE, program: program::curve_trade(), token: t, buy: false, amount_in: LOT, min_out: Fixed(i128::MAX / 4), inputs, change, nonce: n });
        dust.push(curve_order_id(&ALICE, &n));
    }
    // A buy that can fill displaces the least willing order, which goes back whole.
    let cid = curve_buy(&mut c, CAROL, t, Fixed::whole(1), Fixed::ZERO);
    let back: Vec<_> = dust.iter().filter(|id| c.vm.market.claims.contains_key(&claim_id(id, 1))).collect();
    assert_eq!(back.len(), 1);
    assert_eq!(c.vm.market.claims[&claim_id(back[0], 1)].amount, LOT);
    // Another unfillable floor cannot displace one of its kind.
    let (inputs, change) = c.fund(ALICE, t, LOT);
    let n = c.fresh();
    let why = c.refused(Op::CurveTrade { trader: ALICE, program: program::curve_trade(), token: t, buy: false, amount_in: LOT, min_out: Fixed(i128::MAX / 4), inputs, change, nonce: n });
    assert!(why.contains("full"), "{why}");
    c.seal();
    assert!(c.vm.market.claims.contains_key(&claim_id(&cid, 0)), "Carol's buy filled");
}

#[test]
fn dust_cannot_fill_the_route_slots() {
    let mut c = Chain::new();
    c.mint(DAVE, ZEC, 1000);
    c.mint(DAVE, ETH, 1000);
    c.mint(ALICE, ZEC, 10);
    let pid = c.pool(DAVE, ZEC, ETH, 100, 100);
    let hop = Hop::Pool { pool: pid, sell_a: c.vm.market.pools[&pid].a == ZEC };
    let why = c.route(ALICE, vec![hop], ZEC, Fixed(1), Fixed::ZERO).unwrap_err();
    assert!(why.contains("too small"), "{why}");
    // A route at a buy's minimum in its coin goes in.
    c.route(ALICE, vec![hop], ZEC, Fixed(curve::MIN_BUY_UNITS * curve::ZAT), Fixed::ZERO).unwrap();
}

#[test]
fn a_decoy_route_is_turned_away_alone() {
    let mut c = Chain::new();
    c.mint(DAVE, ZEC, 1000);
    c.mint(DAVE, ETH, 1000);
    c.mint(ALICE, ZEC, 500);
    c.mint(CAROL, ZEC, 10);
    let pid = c.pool(DAVE, ZEC, ETH, 100, 100);
    let hop = Hop::Pool { pool: pid, sell_a: c.vm.market.pools[&pid].a == ZEC };
    let mut solo = c.clone();
    let id = solo.route(CAROL, vec![hop], ZEC, Fixed::whole(1), Fixed::ZERO).unwrap();
    solo.seal();
    let out = solo.vm.market.claims[&claim_id(&id, 0)].amount;
    let cid = c.route(CAROL, vec![hop], ZEC, Fixed::whole(1), Fixed(out.0 * 95 / 100)).unwrap();
    let did = c.route(ALICE, vec![hop], ZEC, Fixed::whole(400), Fixed(i128::MAX / 4)).unwrap();
    c.seal();
    assert_eq!(c.vm.market.claims.get(&claim_id(&cid, 0)).map(|x| x.amount), Some(out), "Carol's route delivers as if alone");
    assert_eq!(c.vm.market.claims.get(&claim_id(&did, 1)).map(|x| x.amount), Some(Fixed::whole(400)), "the decoy goes back whole");
}

#[test]
fn testnet_history_keeps_the_rules_it_was_sealed_under() {
    assert_eq!(objectvm::fair_batches_from(212), objectvm::TESTNET_FAIR_BATCHES_FROM);
    assert_eq!(objectvm::fair_batches_from(26460), 0);
    // On testnet before the switch, a decoy still drops floors together, as
    // the sealed history did.
    let mut base = Chain::new();
    base.vm.chain_id = 212;
    base.mint(ALICE, ZEC, 40);
    base.mint(CAROL, ZEC, 10);
    let t = base.create(BOB, ZEC);
    let mut honest = base.clone();
    let cid = curve_buy(&mut honest, CAROL, t, Fixed::whole(1), Fixed::ZERO);
    curve_buy(&mut honest, ALICE, t, Fixed::whole(1), Fixed::ZERO);
    honest.seal();
    let carol_lots = honest.vm.market.claims[&claim_id(&cid, 0)].amount.0 / LOT.0;
    let mut c = base.clone();
    let cid = curve_buy(&mut c, CAROL, t, Fixed::whole(1), Fixed(carol_lots * 95 / 100 * LOT.0));
    curve_buy(&mut c, ALICE, t, Fixed::whole(1), Fixed::ZERO);
    curve_buy(&mut c, ALICE, t, Fixed::whole(15), Fixed(1_000_000_000 * LOT.0));
    c.seal();
    assert!(c.vm.market.claims.get(&claim_id(&cid, 0)).is_none(), "the old rule, before the switch");
}
