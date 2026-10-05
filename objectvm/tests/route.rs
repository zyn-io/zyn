//! Atomic routes end to end (DECISIONS §167): a coin priced in ETH into a
//! coin priced in ZEC through an ETH/ZEC pool in one seal; all or nothing
//! when the minimum is missed; one price per venue; routes that would need a
//! venue to clear twice refused — on a real ledger, the books checked after
//! every intent.

use objectvm::curve::{self, curve_order_id, LOT};
use objectvm::market::{self, claim_id, order_id, token_of};
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
}

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

impl Chain {
    fn new() -> Chain {
        let mut c = Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, salt: 0 };
        c.ok(Op::SetCurveTerms { program: program::configure_curves(), fee_bps: 100, bond: Fixed::ZERO });
        c.ok(Op::SetCurveQuote { program: program::configure_curves(), quote: ZEC, graduation_mc: Fixed(curve::GRADUATION_MC_UNITS * curve::ZAT), bond: Fixed::ZERO });
        c.ok(Op::SetCurveQuote { program: program::configure_curves(), quote: ETH, graduation_mc: Fixed::whole(23), bond: Fixed::ZERO });
        c
    }

    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let r = self.vm.apply(self.seq, &op);
        assert_eq!(self.vm.conserved(), Ok(()), "after {op:?}");
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

/// One route, alone in its seal, gets exactly what the same three trades
/// get one epoch at a time — and it all happens in one seal.
#[test]
fn a_route_from_an_eth_coin_to_a_zec_coin_settles_in_one_seal() {
    let (mut c, a, b, p) = setting();
    let lots = 40i128;
    let sell = Fixed(lots * LOT.0);
    let hops = vec![Hop::CurveSell { token: a }, Hop::Pool { pool: p, sell_a: eth_to_zec(&c, &p) }, Hop::CurveBuy { token: b }];

    // The same three trades, one epoch each, on a copy.
    let mut twin = c.clone();
    let (inputs, change) = twin.fund(ALICE, a, sell);
    let n1 = twin.fresh();
    twin.ok(Op::CurveTrade { trader: ALICE, program: program::curve_trade(), token: a, buy: false, amount_in: sell, min_out: Fixed::ZERO, inputs, change, nonce: n1 });
    twin.seal();
    let eth = twin.vm.market.claims[&claim_id(&curve_order_id(&ALICE, &n1), 0)].amount;
    twin.claim(ALICE, &[curve_order_id(&ALICE, &n1)]);
    let (inputs, change) = twin.fund(ALICE, ETH, eth);
    let n2 = twin.fresh();
    twin.ok(Op::Swap { trader: ALICE, program: program::amm_swap(), pool: p, sell_a: eth_to_zec(&c, &p), amount_in: eth, min_out: Fixed::ZERO, inputs, change, nonce: n2 });
    twin.seal();
    let zec = twin.vm.market.claims[&claim_id(&order_id(&ALICE, &n2), 0)].amount;
    twin.claim(ALICE, &[order_id(&ALICE, &n2)]);
    let b_before = twin.balance(ALICE, b);
    let (inputs, change) = twin.fund(ALICE, ZEC, zec);
    let n3 = twin.fresh();
    twin.ok(Op::CurveTrade { trader: ALICE, program: program::curve_trade(), token: b, buy: true, amount_in: zec, min_out: Fixed::ZERO, inputs, change, nonce: n3 });
    twin.seal();
    twin.claim(ALICE, &[curve_order_id(&ALICE, &n3)]);
    let by_hand = Fixed(twin.balance(ALICE, b).0 - b_before.0);
    assert!(by_hand.0 > 0);

    // The route: one intent, one seal, one claim.
    let r = c.route(ALICE, hops, a, sell, Fixed(by_hand.0)).expect("placed");
    let epoch = c.vm.epoch;
    c.seal();
    assert_eq!(c.vm.epoch, epoch + 1, "one seal");
    let got = c.vm.market.claims[&claim_id(&r, 0)];
    assert_eq!((got.asset, got.amount), (b, by_hand), "the same as the three trades by hand");
    assert!(c.vm.market.claims.get(&claim_id(&r, 1)).is_none(), "nothing refunded");
    // No leg left anything behind: only the route's own claims exist for it.
    for i in 0..3 {
        for k in 0..2 {
            assert!(c.vm.market.claims.get(&claim_id(&objectvm::route::leg_id(&r, i), k)).is_none());
        }
    }
    let before = c.balance(ALICE, b);
    c.claim(ALICE, &[r]);
    assert_eq!(c.balance(ALICE, b).0 - before.0, by_hand.0);
    // The venues moved as the trades by hand moved them.
    assert_eq!(c.vm.market.pools[&p], twin.vm.market.pools[&p]);
    assert_eq!(c.vm.market.curves[&a].sold, twin.vm.market.curves[&a].sold);
    assert_eq!(c.vm.market.curves[&b].sold, twin.vm.market.curves[&b].sold);
}

/// A route that would deliver less than its minimum is cleared out of the
/// batch and refunded whole; everyone else in that batch settles exactly as
/// if it had never been placed.
#[test]
fn a_route_short_of_its_minimum_changes_nothing_and_is_refunded_whole() {
    let (mut c, a, b, p) = setting();
    let sell = Fixed(40 * LOT.0);
    let hops = vec![Hop::CurveSell { token: a }, Hop::Pool { pool: p, sell_a: eth_to_zec(&c, &p) }, Hop::CurveBuy { token: b }];
    // Bob swaps in the same pool in the same epoch, on both chains.
    let mut without = c.clone();
    let e2z = eth_to_zec(&c, &p);
    for ch in [&mut c, &mut without] {
        let (inputs, change) = ch.fund(BOB, ETH, Fixed::whole(1));
        let nonce = [0x77; 32];
        ch.ok(Op::Swap { trader: BOB, program: program::amm_swap(), pool: p, sell_a: e2z, amount_in: Fixed::whole(1), min_out: Fixed::ZERO, inputs, change, nonce });
    }
    let held = c.balance(ALICE, a);
    let r = c.route(ALICE, hops, a, sell, Fixed::whole(1_000_000_000)).expect("placed");
    assert_eq!(c.balance(ALICE, a).0, held.0 - sell.0, "escrowed while it waits");
    c.seal();
    without.seal();
    let back = c.vm.market.claims[&claim_id(&r, 1)];
    assert_eq!((back.asset, back.amount), (a, sell), "refunded whole, in what it put in");
    assert!(c.vm.market.claims.get(&claim_id(&r, 0)).is_none(), "nothing delivered");
    assert_eq!(c.vm.market.pools[&p], without.vm.market.pools[&p], "the pool cleared as if the route never was");
    assert_eq!(c.vm.market.curves[&a], without.vm.market.curves[&a]);
    assert_eq!(c.vm.market.curves[&b], without.vm.market.curves[&b]);
    assert_eq!(c.vm.market.claims[&claim_id(&order_id(&BOB, &[0x77; 32]), 0)], without.vm.market.claims[&claim_id(&order_id(&BOB, &[0x77; 32]), 0)]);
    c.claim(ALICE, &[r]);
    assert_eq!(c.balance(ALICE, a), held);
}

/// Two routes through the same venues in opposite orders would need a venue
/// to clear both before and after the other: the second waits an epoch.
#[test]
fn a_route_that_would_make_a_venue_clear_twice_waits_for_the_next_epoch() {
    let (mut c, _a, _b, p) = setting();
    // A second pool, ZEC/USD.
    c.mint(DAVE, USD, 100);
    c.mint(CAROL, USD, 10);
    let q = c.pool(DAVE, ZEC, USD, 20, 100);
    let zec_side_q = c.vm.market.pools[&q].a == ZEC;
    let eth_side_p = eth_to_zec(&c, &p);
    // ETH → ZEC → USD, and USD → ZEC → ETH.
    c.route(ALICE, vec![Hop::Pool { pool: p, sell_a: eth_side_p }, Hop::Pool { pool: q, sell_a: zec_side_q }], ETH, Fixed::whole(1), Fixed::ZERO).expect("first");
    let why = c.route(CAROL, vec![Hop::Pool { pool: q, sell_a: !zec_side_q }, Hop::Pool { pool: p, sell_a: !eth_side_p }], USD, Fixed::whole(1), Fixed::ZERO).unwrap_err();
    assert!(why.contains("other way"), "{why}");
    c.seal();
    let r = c.route(CAROL, vec![Hop::Pool { pool: q, sell_a: !zec_side_q }, Hop::Pool { pool: p, sell_a: !eth_side_p }], USD, Fixed::whole(1), Fixed::ZERO).expect("next epoch");
    c.seal();
    assert_eq!(c.vm.market.claims[&claim_id(&r, 0)].asset, ETH, "USD in, ETH out");
}

/// Two routes through one pool in one epoch share its one price.
#[test]
fn routes_through_one_pool_share_its_one_price() {
    let (mut c, _a, b, p) = setting();
    let hops = vec![Hop::Pool { pool: p, sell_a: eth_to_zec(&c, &p) }, Hop::CurveBuy { token: b }];
    c.mint(BOB, ETH, 3);
    let r1 = c.route(ALICE, hops.clone(), ETH, Fixed::whole(3), Fixed::ZERO).unwrap();
    let r2 = c.route(BOB, hops, ETH, Fixed::whole(1), Fixed::ZERO).unwrap();
    c.seal();
    let (g1, g2) = (c.vm.market.claims[&claim_id(&r1, 0)].amount.0 / LOT.0, c.vm.market.claims[&claim_id(&r2, 0)].amount.0 / LOT.0);
    assert!(g1 > 0 && (g1 - 3 * g2).abs() <= 3, "three times the ETH, three times the lots: {g1} vs 3×{g2}");
}

#[test]
fn a_route_must_flow() {
    let (mut c, a, b, p) = setting();
    let e2z = eth_to_zec(&c, &p);
    let lot = Fixed(LOT.0);
    // No hops, or five: the wire does not even carry them.
    for hops in [vec![], vec![Hop::Pool { pool: p, sell_a: e2z }; 5]] {
        let op = Op::Route { trader: ALICE, program: program::route(), hops, amount_in: Fixed::whole(1), min_out: Fixed::ZERO, inputs: Vec::new(), change: Vec::new(), nonce: [9; 32] };
        let wire = ObjectVm::encode_intent(&op);
        assert_eq!(ObjectVm::decode_intent(&mut zyn_vm::read::Decoder::new(&wire)), None);
        assert!(ObjectVm::rejected(&c.vm.clone().apply(1_000, &op)[0]));
    }
    let cases: Vec<(Vec<Hop>, [u8; 32], Fixed, &str)> = vec![
        (vec![Hop::Pool { pool: p, sell_a: e2z }, Hop::CurveSell { token: a }], ETH, Fixed::whole(1), "only start"),
        (vec![Hop::CurveBuy { token: b }, Hop::Pool { pool: p, sell_a: !e2z }], ZEC, Fixed::whole(1), "only end"),
        (vec![Hop::CurveSell { token: a }, Hop::Pool { pool: p, sell_a: !e2z }], a, lot, "take what"),
        (vec![Hop::Pool { pool: p, sell_a: e2z }, Hop::Pool { pool: p, sell_a: !e2z }], ETH, Fixed::whole(1), "each venue once"),
        (vec![Hop::CurveSell { token: a }], a, Fixed(lot.0 / 2), "whole number of lots"),
        (vec![Hop::CurveBuy { token: b }], ZEC, Fixed(1_000), "at least"),
    ];
    for (hops, start, amount, want) in cases {
        let nonce = c.fresh();
        let why = c.refused(Op::Route { trader: ALICE, program: program::route(), hops, amount_in: amount, min_out: Fixed::ZERO, inputs: Vec::new(), change: Vec::new(), nonce });
        assert!(why.contains(want), "{want}: {why}");
        let _ = start;
    }
    let nonce = c.fresh();
    let why = c.refused(Op::Route { trader: ALICE, program: program::amm_swap(), hops: vec![Hop::Pool { pool: p, sell_a: e2z }], amount_in: Fixed::whole(1), min_out: Fixed::ZERO, inputs: Vec::new(), change: Vec::new(), nonce });
    assert!(why.contains("wrong program"), "{why}");
}

/// Nothing about routes is saved or hashed until one is placed, and once
/// its seal is past the ledger saves as it did before.
#[test]
fn routes_leave_no_trace_outside_their_epoch() {
    let (mut c, _a, b, p) = setting();
    let has = |c: &Chain| c.vm.encode().windows(8).any(|w| w == b"MARKET12");
    assert!(!has(&c));
    c.route(ALICE, vec![Hop::Pool { pool: p, sell_a: eth_to_zec(&c, &p) }, Hop::CurveBuy { token: b }], ETH, Fixed::whole(1), Fixed::ZERO).unwrap();
    assert!(has(&c));
    c.seal();
    assert!(!has(&c));
    assert!(c.vm.market.routes.is_empty());
}
