//! Bonding-curve launches end to end (DECISIONS §141): make a token, buy and
//! sell it on its curve in sealed batches, graduate it into a locked pool,
//! pay the creator — on a real ledger, with the books checked after every
//! intent.

use objectvm::curve::{self, cost, cost_at, curve_order_id, curve_source, QuoteTerms, GRADUATE_AT, LOT, LOTS, POOL_FEE_BPS, ZAT};
use objectvm::market::{self, token_of};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x10; 32];
const ETH: [u8; 32] = [0x20; 32];
const BOLD: [u8; 32] = [0x30; 32];
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
            assert!(c.reserve.0 >= cost_at(0, c.sold, c.unit), "reserve {} below the integral {}", c.reserve.0, cost_at(0, c.sold, c.unit));
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
        self.mint_of(owner, ZEC, whole);
    }

    fn mint_of(&mut self, owner: AccountId, asset: [u8; 32], whole: i64) {
        let nonce = self.fresh();
        self.ok(Op::Mint { asset, amount: Fixed::whole(whole), owner, nonce });
    }

    fn quote(&mut self, quote: [u8; 32], graduation_mc: Fixed, bond: Fixed) -> Receipt {
        self.apply(Op::SetCurveQuote { program: program::configure_curves(), quote, graduation_mc, bond })
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
        self.create_in(creator, ZEC)
    }

    fn create_in(&mut self, creator: AccountId, quote: [u8; 32]) -> ([u8; 32], Hash) {
        let bond = match self.vm.market.curve_quotes.get(&quote) {
            Some(q) => q.bond,
            None => self.vm.market.curve_terms.map(|t| t.bond).unwrap_or(Fixed::ZERO),
        };
        let (inputs, change) = if bond.0 > 0 { self.fund(creator, quote, bond) } else { (Vec::new(), Vec::new()) };
        let nonce = self.fresh();
        let r = self.ok(Op::CreateCurve { creator, program: program::curve_create(), nonce, quote, meta: [7; 32], inputs, change });
        assert_eq!(r, Receipt::CurveCreated { token: token_of(&creator, &nonce) });
        (token_of(&creator, &nonce), nonce)
    }

    /// Place a trade; returns the order's claim source.
    fn trade(&mut self, who: AccountId, token: [u8; 32], buy: bool, amount: Fixed, min_out: Fixed) -> Hash {
        let quote = self.vm.market.curves[&token].quote;
        let (inputs, change) = self.fund(who, if buy { quote } else { token }, amount);
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

// --- Quote assets (§166) ------------------------------------------------------

/// A whole coin of an 18-decimal asset, times `n`, divided by `d`.
fn coins(n: i128, d: i128) -> Fixed {
    Fixed(n * zyn_vm::fixed::WAD / d)
}

/// ETH named at 23 ETH fully diluted: an ETH coin has ZEC's shape at that
/// size, raises it, graduates into an ETH pool and pays its bond in ETH.
#[test]
fn a_coin_in_eth_is_sized_for_eth_and_graduates_at_its_size() {
    let mut c = Chain::new();
    c.terms(100, Fixed(5 * CENT.0));
    assert!(matches!(c.quote(ZEC, Fixed(curve::GRADUATION_MC_UNITS * ZAT), Fixed(5 * CENT.0)), Receipt::CurveQuoteSet { unit: ZAT, .. }));
    let r = c.quote(ETH, Fixed::whole(23), coins(25, 1000));
    let unit = QuoteTerms::unit_for(Fixed::whole(23));
    assert_eq!(r, Receipt::CurveQuoteSet { quote: ETH, unit, bond: coins(25, 1000) });
    assert_eq!(unit, 4_887_876_366, "23 ETH / 4,705,520,000 units");
    c.mint_of(BOB, ETH, 1);
    c.mint_of(ALICE, ETH, 30);

    let (token, bob_nonce) = c.create_in(BOB, ETH);
    let cv = c.vm.market.curves[&token];
    assert_eq!((cv.quote, cv.unit, cv.bond), (ETH, unit, coins(25, 1000)));
    assert_eq!(c.balance(BOB, ETH), coins(975, 1000));
    assert_eq!(cv.price_per_token().0, cost_at(0, 1, unit) / curve::LOT_TOKENS);

    // A whale takes it to graduation: ~8.3 ETH raised, the rest refunded.
    let w = c.trade(ALICE, token, true, Fixed::whole(20), Fixed::ZERO);
    c.seal();
    let cv = c.vm.market.curves[&token];
    assert!(cv.graduated);
    let raised = cost_at(0, GRADUATE_AT, unit);
    assert!((8_300..8_320).contains(&(raised / (zyn_vm::fixed::WAD / 1000))), "raised {raised}");
    c.claim(ALICE, &[w]);
    assert!(c.balance(ALICE, ETH) > Fixed::whole(21), "the unfilled part came back");
    let p = c.vm.market.pools[&market::pool_id(&token, &ETH)];
    let quote = if p.a == ETH { p.reserve_a } else { p.reserve_b };
    assert!(quote.0 >= raised, "the pool has at least what the curve raised");
    // Its fully diluted value at the curve's last price: 23 ETH, to a unit.
    let mc = cost_at(GRADUATE_AT, 1, unit) * LOTS as i128;
    assert!((Fixed::whole(23).0 - mc) < curve::GRADUATION_MC_UNITS, "{mc}");
    // Fees went to the pot in ETH; the bond comes back in ETH.
    assert!(c.vm.market.pol[&ETH].0 > 0);
    let before = c.balance(BOB, ETH);
    c.claim(BOB, &[curve_source(&BOB, &bob_nonce)]);
    assert!(c.balance(BOB, ETH).0 >= before.0 + coins(25, 1000).0);
}

/// Before any quote is named, every deposited asset prices curves at ZEC's
/// size, as on chains made before §166. Once one is named, only named ones
/// take new coins — and coins already made keep their size and keep trading.
#[test]
fn once_a_quote_is_named_only_named_quotes_take_new_coins() {
    let mut c = Chain::new();
    c.mint_of(ALICE, ETH, 30);
    c.mint(ALICE, 5);
    let (old, _) = c.create_in(ALICE, ETH);
    assert_eq!(c.vm.market.curves[&old].unit, ZAT, "before §166's terms: ZEC's size");
    assert!(!c.vm.encode().windows(8).any(|w| w == b"MARKET11"), "nothing new is saved before a quote is named");

    c.quote(ZEC, Fixed(curve::GRADUATION_MC_UNITS * ZAT), Fixed::ZERO);
    assert!(c.vm.encode().windows(8).any(|w| w == b"MARKET11"));
    let n = c.fresh();
    let why = c.refused(Op::CreateCurve { creator: ALICE, program: program::curve_create(), nonce: n, quote: ETH, meta: [7; 32], inputs: Vec::new(), change: Vec::new() });
    assert!(why.contains("not launched in that asset"), "{why}");
    c.create_in(ALICE, ZEC);
    // The ETH coin made before still trades, at the size it was made with.
    let o = c.trade(ALICE, old, true, Fixed::whole(1), Fixed::ZERO);
    c.seal();
    c.claim(ALICE, &[o]);
    let cv = c.vm.market.curves[&old];
    assert_eq!(cv.unit, ZAT);
    assert_eq!(cv.reserve.0, cost(0, cv.sold));

    // A zero size stops new coins in it again.
    assert!(matches!(c.quote(ZEC, Fixed::ZERO, Fixed::ZERO), Receipt::CurveQuoteSet { unit: 0, .. }));
    assert!(c.vm.market.curve_quotes.is_empty());
    let why = match c.quote(ZEC, Fixed::ZERO, Fixed::ZERO) { Receipt::Refused(w) => w, r => panic!("{r:?}") };
    assert!(why.contains("not launched"), "{why}");
}

/// Sized for BOLD (63,000 BOLD fully diluted), the smallest buy scales too:
/// 1 BOLD is below it, 2 BOLD is not.
#[test]
fn a_coin_in_bold_scales_its_smallest_buy() {
    let mut c = Chain::new();
    c.quote(BOLD, Fixed::whole(63_000), Fixed::whole(67));
    let unit = c.vm.market.curve_quotes[&BOLD].unit;
    assert_eq!(unit, 13_388_530_916_880, "63,000 BOLD / 4,705,520,000");
    c.mint_of(ALICE, BOLD, 200);
    let (token, _) = c.create_in(ALICE, BOLD);
    assert_eq!(c.vm.market.curves[&token].min_buy().0, 100_000 * unit);
    let (inputs, change) = c.fund(ALICE, BOLD, Fixed::whole(1));
    let n = c.fresh();
    let why = c.refused(Op::CurveTrade { trader: ALICE, program: program::curve_trade(), token, buy: true, amount_in: Fixed::whole(1), min_out: Fixed::ZERO, inputs, change, nonce: n });
    assert!(why.contains("at least"), "{why}");
    let o = c.trade(ALICE, token, true, Fixed::whole(2), Fixed::ZERO);
    c.seal();
    c.claim(ALICE, &[o]);
    assert!(c.vm.market.curves[&token].sold > 0);
}

#[test]
fn quote_terms_are_the_operators_and_bounded() {
    let mut c = Chain::new();
    c.mint(ALICE, 5);
    let why = |r: Receipt| match r { Receipt::Refused(w) => w.to_string(), r => panic!("expected a refusal, got {r:?}") };
    let r = c.apply(Op::SetCurveQuote { program: program::curve_create(), quote: ETH, graduation_mc: Fixed::whole(23), bond: Fixed::ZERO });
    assert!(why(r).contains("wrong program"));
    // A coin on a curve is not something coins launch in.
    let (token, _) = c.create(ALICE);
    assert!(why(c.quote(token, Fixed::whole(23), Fixed::ZERO)).contains("not a share or a coin"));
    // Too small to size, too large to settle, a negative size, too big a bond.
    assert!(why(c.quote(ETH, Fixed(curve::GRADUATION_MC_UNITS * 1_000), Fixed::ZERO)).contains("outside"));
    assert!(why(c.quote(ETH, Fixed(curve::GRADUATION_MC_UNITS.saturating_mul(curve::MAX_UNIT) * 2), Fixed::ZERO)).contains("outside"));
    assert!(why(c.quote(ETH, Fixed::whole(-23), Fixed::ZERO)).contains("outside"));
    assert!(why(c.quote(ETH, Fixed::whole(23), Fixed::whole(1))).contains("outside"), "a bond above 1 ZEC's worth of the size");
    assert!(matches!(c.quote(ETH, Fixed::whole(23), coins(48, 100)), Receipt::CurveQuoteSet { .. }));
}
