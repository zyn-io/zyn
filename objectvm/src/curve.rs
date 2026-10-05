//! Bonding-curve launches: anyone makes a token, it trades at once on a
//! fixed curve, and when the curve sells out it graduates into a pool whose
//! liquidity nobody can pull (DECISIONS §141).
//!
//! # One curve for every token
//!
//! Supply is 1,000,000,000 tokens in [`LOTS`] lots of [`LOT_TOKENS`]. Lot `i`
//! (counting from zero) costs `BASE + SLOPE·i` zatoshi of the quote asset, so
//! lots `[q, q+d)` cost
//!
//! ```text
//!     cost(q, d) = BASE·d + SLOPE·d·(2q + d − 1)/2        (exact: d·(2q+d−1) is even)
//! ```
//!
//! The curve sells up to lot [`GRADUATE_AT`] and graduates there: 17.00005
//! ZEC raised, the price 7.62× where it started, a 47 ZEC fully diluted value
//! — pump.fun's shape in dollars at today's ZEC price. The lots it did not
//! sell and everything it raised then open a pool at, to 0.0011%, the curve's
//! last price. The numbers are consensus, not the creator's: a creator who
//! could tune them could tune them against buyers.
//!
//! # Quote assets (§166)
//!
//! A curve priced in another bridged coin has the same shape, sized for that
//! coin: its [`QuoteTerms`] fix a *unit*, the ledger amount one "zatoshi" of
//! the shape stands for, and the bond. ZEC.zy's unit is [`ZAT`]: 47 ZEC at
//! graduation. ETH.zy's might be sized to 23 ETH, BOLD.zy's to 63,000 BOLD.
//! The operator sets each from a graduation market cap in that coin; no price
//! feed is read. A curve keeps the unit it was made with. Until the operator
//! names a quote asset, every deposited asset prices curves at ZEC's size, as
//! before; once one is named, only named ones do.
//!
//! # Batches, sells first
//!
//! Orders are escrowed for the epoch and settle at its seal, like swaps. The
//! sells settle first, all at one price — the average over the lots they
//! return — then the buys, all at one price over the lots they take, each
//! buyer's share in proportion to what they put in. The rule is the VM's,
//! not the order they arrived in, so the sequencer gains nothing by
//! reordering and nobody can be sandwiched. Every division rounds toward the
//! curve, so the reserve never holds less than it would take to buy back
//! every lot sold.
//!
//! # Fees
//!
//! A fee, if the operator set one ([`CurveTerms`]), is charged on the quote
//! side of every trade — on top of a buy, out of a sell's proceeds — and
//! splits 50% to the ZYN pot, 40% to the token's own graduation liquidity and
//! 10% to its creator (§69.8's split). The creator's share, and the bond they
//! put up to create the token, are paid only if it graduates: a launch that
//! goes nowhere earns its creator nothing.

use zyn_vm::commit::{merkle_root, Encoder, Hash};
use zyn_vm::derive::Address;
use zyn_vm::read::Decoder;
use zyn_vm::spec::AccountId;
use zyn_vm::Fixed;

use crate::market::{fee_of, geometric_mean, lp_asset, pool_id, resale_split, token_of, Pool, BPS, MINIMUM_LIQUIDITY};
use crate::{ObjectVm, Output, Receipt};

/// Lots in a token's supply.
pub const LOTS: u32 = 80_000;
/// Tokens in a lot.
pub const LOT_TOKENS: i128 = 12_500;
/// One lot, in the ledger's units (1e18 per token).
pub const LOT: Fixed = Fixed(LOT_TOKENS * zyn_vm::fixed::WAD);
/// Every token's supply.
pub const SUPPLY: Fixed = Fixed(LOTS as i128 * LOT_TOKENS * zyn_vm::fixed::WAD);
/// The first lot's price, in zatoshi.
pub const BASE_ZAT: i128 = 7_721;
/// How much each lot sold adds to the next one's price, in zatoshi.
pub const SLOPE_ZAT: i128 = 1;
/// Lots the curve sells before it graduates.
pub const GRADUATE_AT: u32 = 51_098;
/// A zatoshi in the ledger's units: ZEC.zy's unit.
pub const ZAT: i128 = 10_000_000_000;
/// The smallest buy, 0.001 of the quote asset: below it a lot is not
/// affordable anyway, and the batch has a limit worth spending on real orders.
/// In units: a curve sized for another coin scales it with its unit.
pub const MIN_BUY_UNITS: i128 = 100_000;
pub const MIN_BUY: Fixed = Fixed(MIN_BUY_UNITS * ZAT);
/// A curve's fully diluted value at graduation, in units: the next lot's
/// price per token times the supply, `(BASE + SLOPE·GRADUATE_AT) · LOTS`.
/// At ZEC's unit, 47.0552 ZEC.
pub const GRADUATION_MC_UNITS: i128 = (BASE_ZAT + SLOPE_ZAT * GRADUATE_AT as i128) * LOTS as i128;
/// Bounds on a quote's unit, so that no batch's arithmetic can overflow and
/// no lot rounds to nothing: 10⁶ to 10¹⁸ ledger units (ZEC's is 10¹⁰).
pub const MIN_UNIT: i128 = 1_000_000;
pub const MAX_UNIT: i128 = 1_000_000_000_000_000_000;
/// The bond's ceiling, in units: 1 ZEC at ZEC's unit ([`MAX_BOND`]).
pub const MAX_BOND_UNITS: i128 = 100_000_000;
/// A graduated pool's fee, kept by the pool — whose shares nobody holds, so
/// it deepens the pool rather than paying anyone.
pub const POOL_FEE_BPS: u32 = 30;
/// Ceilings the operator's terms cannot exceed.
pub const MAX_CURVE_FEE_BPS: u32 = 200;
pub const MAX_BOND: Fixed = Fixed(zyn_vm::fixed::WAD);
/// Orders one curve takes in one epoch.
pub const MAX_ORDERS_PER_CURVE: usize = 256;

fn id(domain: &[u8], parts: &[&[u8]]) -> Hash {
    let mut e = Encoder::new();
    e.bytes(domain);
    for p in parts {
        e.bytes(p);
    }
    e.leaf()
}

/// Where a curve's creator is paid at graduation: from a nonce only they hold.
pub fn curve_source(creator: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.curve.v1", &[creator, nonce])
}

/// An order's id, and so where what it is owed is claimable.
pub fn curve_order_id(trader: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.curve-order.v1", &[trader, nonce])
}

/// The quote asset lots `[q, q+d)` cost, in the ledger's units, on a curve
/// sized for ZEC.
pub fn cost(q: u32, d: u32) -> i128 {
    cost_at(q, d, ZAT)
}

/// The same on a curve whose unit is `unit`.
pub fn cost_at(q: u32, d: u32, unit: i128) -> i128 {
    let (q, d) = (q as i128, d as i128);
    (BASE_ZAT * d + SLOPE_ZAT * (d * (2 * q + d - 1) / 2)) * unit
}

/// The most lots `budget` buys starting at lot `q`, at most `cap`, sized
/// for ZEC.
pub fn lots_for(q: u32, budget: i128, cap: u32) -> u32 {
    lots_for_at(q, budget, cap, ZAT)
}

/// The same on a curve whose unit is `unit`.
pub fn lots_for_at(q: u32, budget: i128, cap: u32, unit: i128) -> u32 {
    let (mut lo, mut hi) = (0u32, cap);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if cost_at(q, mid, unit) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// What the operator charges. Absent until set, so nothing is charged
/// before it and a chain that never set it replays unchanged.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CurveTerms {
    pub fee_bps: u32,
    /// Put up to create a token; returned to its creator at graduation.
    pub bond: Fixed,
}

/// How curves priced in one asset are sized (§166): set by the operator per
/// quote asset, fixed per curve at creation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QuoteTerms {
    /// The ledger amount one unit of the curve's shape stands for.
    pub unit: i128,
    /// What a creator puts up, in this asset.
    pub bond: Fixed,
}

impl QuoteTerms {
    /// The unit that makes a curve graduate at `mc` (ledger units of the
    /// quote) fully diluted, rounded down.
    pub fn unit_for(mc: Fixed) -> i128 {
        mc.0 / GRADUATION_MC_UNITS
    }

    /// Whether these terms are within what the ledger takes.
    pub fn valid(&self) -> bool {
        (MIN_UNIT..=MAX_UNIT).contains(&self.unit)
            && self.bond.0 >= 0
            && self.bond.0 <= MAX_BOND_UNITS.saturating_mul(self.unit)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Curve {
    pub creator: AccountId,
    pub source: Hash,
    pub token: Address,
    pub quote: Address,
    /// `H(name ‖ symbol ‖ image …)`, published by the creator off chain.
    pub meta: Hash,
    pub created_at: u64,
    /// The fee in force when the token was made. Fixed for its life: holders
    /// bought into these terms.
    pub fee_bps: u32,
    /// Lots sold and not sold back.
    pub sold: u32,
    /// What the lots sold paid, and what sells are paid from.
    pub reserve: Fixed,
    /// Fees kept for the pool this graduates into.
    pub graduation: Fixed,
    /// Fees kept for the creator, paid at graduation.
    pub creator_fees: Fixed,
    pub bond: Fixed,
    pub graduated: bool,
    /// The ledger amount one unit of the shape stands for: [`ZAT`] for a
    /// curve sized for ZEC, which every curve made before §166 is.
    pub unit: i128,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CurveOrder {
    pub id: Hash,
    pub owner: AccountId,
    pub token: Address,
    pub buy: bool,
    /// The quote asset put in (a buy) or the tokens put in (a sell; whole lots).
    pub amount_in: Fixed,
    /// Tokens at least (a buy) or quote at least, after the fee (a sell).
    pub min_out: Fixed,
}

impl Curve {
    /// The price of the next lot, per token, in the ledger's units.
    pub fn price_per_token(&self) -> Fixed {
        Fixed(cost_at(self.sold, 1, self.unit) / LOT_TOKENS)
    }

    /// The smallest buy this curve takes.
    pub fn min_buy(&self) -> Fixed {
        Fixed(MIN_BUY_UNITS * self.unit)
    }

    /// What the curve holds of its quote asset.
    pub fn quote_held(&self) -> Option<i128> {
        self.reserve
            .0
            .checked_add(self.graduation.0)?
            .checked_add(self.creator_fees.0)?
            .checked_add(self.bond.0)
    }
}

pub(crate) fn curve_leaf(c: &Curve) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.curve-leaf.v1")
        .bytes(&c.token)
        .bytes(&c.quote)
        .bytes(&c.meta)
        .u64(c.created_at)
        .u32(c.fee_bps)
        .u32(c.sold)
        .fixed(c.reserve)
        .fixed(c.graduation)
        .fixed(c.creator_fees)
        .fixed(c.bond)
        .bool(c.graduated)
        // The creator is private, like a launch's: committed with its source.
        .bytes(&id(b"objectvm.curve-creator.v1", &[&c.creator, &c.source]));
    // A curve sized for another coin says so; one sized for ZEC hashes as it
    // always did, so every root sealed before §166 keeps.
    if c.unit != ZAT {
        e.bytes(b"objectvm.curve-unit.v1").fixed(Fixed(c.unit));
    }
    e.leaf()
}

fn quote_leaf(quote: &Address, q: &QuoteTerms) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.curve-quote-leaf.v1").bytes(quote).fixed(Fixed(q.unit)).fixed(q.bond);
    e.leaf()
}

pub(crate) fn curve_order_leaf(o: &CurveOrder) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.curve-order-leaf.v1")
        .bytes(&o.id)
        .bytes(&o.owner)
        .bytes(&o.token)
        .bool(o.buy)
        .fixed(o.amount_in)
        .fixed(o.min_out);
    e.leaf()
}

/// The curve section of the market's leaf. Public: what a token costs, what
/// its curve holds and what it will graduate with are things every buyer
/// should be able to check. Orders are blinded by their ids, as swaps are.
pub(crate) fn section(
    e: &mut Encoder,
    terms: Option<CurveTerms>,
    quotes: &std::collections::BTreeMap<Address, QuoteTerms>,
    curves: &std::collections::BTreeMap<Address, Curve>,
    orders: &[CurveOrder],
) {
    let t = terms.unwrap_or_default();
    e.bytes(b"objectvm.curves.v1").bool(terms.is_some()).u32(t.fee_bps).fixed(t.bond);
    let leaves: Vec<Hash> = curves.values().map(curve_leaf).collect();
    let order_leaves: Vec<Hash> = orders.iter().map(curve_order_leaf).collect();
    e.u32(curves.len() as u32).bytes(&merkle_root(&leaves)).bytes(&merkle_root(&order_leaves));
    // Joined once a quote asset is named (§166), so older roots keep.
    if !quotes.is_empty() {
        let q: Vec<Hash> = quotes.iter().map(|(a, t)| quote_leaf(a, t)).collect();
        e.bytes(b"objectvm.curve-quotes.v1").u32(q.len() as u32).bytes(&merkle_root(&q));
    }
}

/// MARKET11's tail (§166): the named quote assets, and the unit of every
/// curve not sized for ZEC.
pub(crate) fn encode_quotes(
    e: &mut Encoder,
    quotes: &std::collections::BTreeMap<Address, QuoteTerms>,
    curves: &std::collections::BTreeMap<Address, Curve>,
) {
    e.u32(quotes.len() as u32);
    for (a, q) in quotes {
        e.bytes(a).fixed(Fixed(q.unit)).fixed(q.bond);
    }
    let sized: Vec<&Curve> = curves.values().filter(|c| c.unit != ZAT).collect();
    e.u32(sized.len() as u32);
    for c in sized {
        e.bytes(&c.token).fixed(Fixed(c.unit));
    }
}

pub(crate) fn decode_quotes(
    d: &mut Decoder,
    curves: &mut std::collections::BTreeMap<Address, Curve>,
) -> Option<std::collections::BTreeMap<Address, QuoteTerms>> {
    let mut quotes = std::collections::BTreeMap::new();
    for _ in 0..d.u32().ok()? {
        let a = d.hash().ok()?;
        quotes.insert(a, QuoteTerms { unit: d.fixed().ok()?.0, bond: d.fixed().ok()? });
    }
    for _ in 0..d.u32().ok()? {
        let token = d.hash().ok()?;
        let unit = d.fixed().ok()?.0;
        curves.get_mut(&token)?.unit = unit;
    }
    Some(quotes)
}

pub(crate) fn encode(
    e: &mut Encoder,
    terms: Option<CurveTerms>,
    curves: &std::collections::BTreeMap<Address, Curve>,
    orders: &[CurveOrder],
) {
    let t = terms.unwrap_or_default();
    e.bool(terms.is_some()).u32(t.fee_bps).fixed(t.bond);
    e.u32(curves.len() as u32);
    for c in curves.values() {
        e.bytes(&c.token)
            .bytes(&c.creator)
            .bytes(&c.source)
            .bytes(&c.quote)
            .bytes(&c.meta)
            .u64(c.created_at)
            .u32(c.fee_bps)
            .u32(c.sold)
            .fixed(c.reserve)
            .fixed(c.graduation)
            .fixed(c.creator_fees)
            .fixed(c.bond)
            .bool(c.graduated);
    }
    e.u32(orders.len() as u32);
    for o in orders {
        e.bytes(&o.id)
            .bytes(&o.owner)
            .bytes(&o.token)
            .bool(o.buy)
            .fixed(o.amount_in)
            .fixed(o.min_out);
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn decode(
    d: &mut Decoder,
) -> Option<(Option<CurveTerms>, std::collections::BTreeMap<Address, Curve>, Vec<CurveOrder>)> {
    let has = d.u8().ok()? == 1;
    let fee_bps = d.u32().ok()?;
    let bond = d.fixed().ok()?;
    let terms = has.then_some(CurveTerms { fee_bps, bond });
    let mut curves = std::collections::BTreeMap::new();
    for _ in 0..d.u32().ok()? {
        let token = d.hash().ok()?;
        curves.insert(
            token,
            Curve {
                token,
                creator: d.account().ok()?,
                source: d.hash().ok()?,
                quote: d.hash().ok()?,
                meta: d.hash().ok()?,
                created_at: d.u64().ok()?,
                fee_bps: d.u32().ok()?,
                sold: d.u32().ok()?,
                reserve: d.fixed().ok()?,
                graduation: d.fixed().ok()?,
                creator_fees: d.fixed().ok()?,
                bond: d.fixed().ok()?,
                graduated: d.u8().ok()? == 1,
                unit: ZAT,
            },
        );
    }
    let mut orders = Vec::new();
    for _ in 0..d.u32().ok()? {
        orders.push(CurveOrder {
            id: d.hash().ok()?,
            owner: d.account().ok()?,
            token: d.hash().ok()?,
            buy: d.u8().ok()? == 1,
            amount_in: d.fixed().ok()?,
            min_out: d.fixed().ok()?,
        });
    }
    Some((terms, curves, orders))
}

/// How one batch of sells settles: `(order index, gross quote, fee)` for
/// each order filled, and the indices refunded because their minimum was
/// not met.
pub(crate) struct SellFill {
    pub filled: Vec<(usize, i128, i128)>,
    pub refunded: Vec<usize>,
    pub lots: u32,
}

/// Sells, all at one price: the average over the lots they return.
pub(crate) fn clear_sells(sold: u32, fee_bps: u32, unit: i128, orders: &[CurveOrder]) -> SellFill {
    let mut active: Vec<bool> = vec![true; orders.len()];
    loop {
        let lots: u64 = orders
            .iter()
            .zip(&active)
            .filter(|(_, a)| **a)
            .map(|(o, _)| (o.amount_in.0 / LOT.0) as u64)
            .sum();
        // More lots than were ever sold cannot be anybody's to sell. It
        // cannot happen — tokens exist only for lots sold — but if it did,
        // paying it would take from the reserve what it owes everyone else.
        if lots > sold as u64 {
            return SellFill { filled: Vec::new(), refunded: (0..orders.len()).collect(), lots: 0 };
        }
        let lots = lots as u32;
        let total = if lots == 0 { 0 } else { cost_at(sold - lots, lots, unit) };
        let mut filled = Vec::new();
        let mut unmet = None;
        for (i, o) in orders.iter().enumerate() {
            if !active[i] {
                continue;
            }
            let mine = (o.amount_in.0 / LOT.0) as i128;
            // Rounded down: the dust stays in the reserve.
            let gross = if lots == 0 { 0 } else { total * mine / lots as i128 };
            let fee = fee_of(Fixed(gross), fee_bps).map(|f| f.0).unwrap_or(0);
            if gross - fee < o.min_out.0 {
                unmet = Some(i);
                break;
            }
            filled.push((i, gross, fee));
        }
        match unmet {
            Some(i) => active[i] = false,
            None => {
                let refunded = (0..orders.len()).filter(|i| !active[*i]).collect();
                return SellFill { filled, refunded, lots };
            }
        }
    }
}

/// How one batch of buys settles: `(order index, lots, paid, fee)` for each
/// order filled, and the indices refunded whole.
pub(crate) struct BuyFill {
    pub filled: Vec<(usize, u32, i128, i128)>,
    pub refunded: Vec<usize>,
    pub lots: u32,
}

/// `a / b` rounded up, for `a ≥ 0` and `b > 0`.
fn ceil_div(a: i128, b: i128) -> i128 {
    (a + b - 1) / b
}

/// The quote a buy has left for lots once the fee on top is set aside.
fn budget(amount_in: i128, fee_bps: u32) -> i128 {
    amount_in * BPS / (BPS + fee_bps as i128)
}

/// Buys, all at one price: the average over the lots they take, shared in
/// proportion to what each put in, never past graduation.
pub(crate) fn clear_buys(sold: u32, fee_bps: u32, unit: i128, orders: &[CurveOrder]) -> BuyFill {
    let cap = GRADUATE_AT.saturating_sub(sold);
    let mut active: Vec<bool> = vec![true; orders.len()];
    loop {
        let total_budget: i128 = orders
            .iter()
            .zip(&active)
            .filter(|(_, a)| **a)
            .map(|(o, _)| budget(o.amount_in.0, fee_bps))
            .sum();
        let lots = if total_budget <= 0 { 0 } else { lots_for_at(sold, total_budget, cap, unit) };
        let total = cost_at(sold, lots, unit);
        let mut filled = Vec::new();
        let mut unmet = None;
        for (i, o) in orders.iter().enumerate() {
            if !active[i] {
                continue;
            }
            let b = budget(o.amount_in.0, fee_bps);
            let mut mine = if lots == 0 { 0 } else { (b * lots as i128 / total_budget) as u32 };
            // Rounded up: the curve is never short for a lot it sold.
            let paid_for = |n: u32| if lots == 0 { 0 } else { ceil_div(total * n as i128, lots as i128) };
            let fee_for = |p: i128| fee_of(Fixed(p), fee_bps).map(|f| f.0).unwrap_or(0);
            while mine > 0 && paid_for(mine) + fee_for(paid_for(mine)) > o.amount_in.0 {
                mine -= 1;
            }
            if (mine as i128) * LOT.0 < o.min_out.0 {
                unmet = Some(i);
                break;
            }
            let paid = paid_for(mine);
            filled.push((i, mine, paid, fee_for(paid)));
        }
        match unmet {
            Some(i) => active[i] = false,
            None => {
                let taken = filled.iter().map(|f| f.1).sum();
                let refunded = (0..orders.len()).filter(|i| !active[*i]).collect();
                return BuyFill { filled, refunded, lots: taken };
            }
        }
    }
}

impl ObjectVm {
    pub(crate) fn apply_create_curve(
        &mut self,
        creator: &AccountId,
        nonce: &Hash,
        quote: &Address,
        meta: &Hash,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let token = token_of(creator, nonce);
        if self.market.curves.contains_key(&token)
            || self.market.launches.contains_key(&token)
            || self.issued.contains_key(&token)
        {
            return Err("that token exists");
        }
        let source = curve_source(creator, nonce);
        self.fresh_source(&source)?;
        if !self.issued.contains_key(quote) || self.market.is_derived(quote) {
            return Err("a curve is priced in an asset people can deposit");
        }
        let terms = self.market.curve_terms.unwrap_or_default();
        // Sized for its quote asset once the operator names any (§166);
        // before that, every asset prices at ZEC's size, as it always did.
        let (unit, bond) = match self.market.curve_quotes.get(quote) {
            Some(q) => (q.unit, q.bond),
            None if self.market.curve_quotes.is_empty() => (ZAT, terms.bond),
            None => return Err("coins are not launched in that asset"),
        };
        if bond.0 > 0 {
            let f = self.plan_funding(creator, inputs, change, &[(*quote, bond)])?;
            self.apply_funding(creator, inputs, f);
        } else if !inputs.is_empty() || !change.is_empty() {
            return Err("there is no bond to pay");
        } else {
            *self.accounts.entry(*creator).or_insert(0) += 1;
        }
        self.market.curves.insert(
            token,
            Curve {
                creator: *creator,
                source,
                token,
                quote: *quote,
                meta: *meta,
                created_at: self.epoch,
                fee_bps: terms.fee_bps,
                sold: 0,
                reserve: Fixed::ZERO,
                graduation: Fixed::ZERO,
                creator_fees: Fixed::ZERO,
                bond,
                graduated: false,
                unit,
            },
        );
        Ok(Receipt::CurveCreated { token })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_curve_trade(
        &mut self,
        trader: &AccountId,
        token: &Address,
        buy: bool,
        amount_in: Fixed,
        min_out: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[Output],
        nonce: &Hash,
    ) -> Result<Receipt, &'static str> {
        let c = *self.market.curves.get(token).ok_or("no such curve")?;
        if c.graduated {
            return Err("that token has graduated; it trades in its pool");
        }
        if min_out.0 < 0 {
            return Err("a minimum below nothing");
        }
        if buy {
            if amount_in < c.min_buy() {
                return Err("a buy is at least 0.001");
            }
        } else if amount_in.0 <= 0 || amount_in.0 % LOT.0 != 0 {
            return Err("a sell is a whole number of lots (12,500 tokens each)");
        }
        let oid = curve_order_id(trader, nonce);
        self.fresh_source(&oid)?;
        if self.market.curve_orders.iter().filter(|o| &o.token == token).count() >= MAX_ORDERS_PER_CURVE {
            return Err("this curve's batch is full; try next epoch");
        }
        let asset = if buy { c.quote } else { c.token };
        let f = self.plan_funding(trader, inputs, change, &[(asset, amount_in)])?;
        self.apply_funding(trader, inputs, f);
        self.market.curve_orders.push(CurveOrder {
            id: oid,
            owner: *trader,
            token: *token,
            buy,
            amount_in,
            min_out,
        });
        Ok(Receipt::Ordered { order: oid })
    }

    /// Split one fee three ways: the ZYN pot, the token's graduation
    /// liquidity, and its creator's escrow.
    fn route_curve_fee(&mut self, token: &Address, fee: i128) {
        if fee <= 0 {
            return;
        }
        let (pot, graduation, creator) = resale_split(Fixed(fee));
        let quote = self.market.curves[token].quote;
        let _ = self.market.to_pol(quote, pot);
        let c = self.market.curves.get_mut(token).expect("routed for a curve that exists");
        c.graduation = Fixed(c.graduation.0 + graduation.0);
        c.creator_fees = Fixed(c.creator_fees.0 + creator.0);
    }

    /// Settle every curve's batch, sells before buys, and graduate any curve
    /// that sold out.
    pub(crate) fn settle_curves(&mut self) {
        let orders = std::mem::take(&mut self.market.curve_orders);
        let tokens: Vec<Address> = self.market.curves.keys().copied().collect();
        for token in tokens {
            let batch: Vec<CurveOrder> = orders.iter().filter(|o| o.token == token).copied().collect();
            if batch.is_empty() {
                continue;
            }
            let c = self.market.curves[&token];
            let sells: Vec<CurveOrder> = batch.iter().filter(|o| !o.buy).copied().collect();
            let buys: Vec<CurveOrder> = batch.iter().filter(|o| o.buy).copied().collect();

            let s = clear_sells(c.sold, c.fee_bps, c.unit, &sells);
            let mut paid_out: i128 = 0;
            for (i, gross, fee) in &s.filled {
                let o = &sells[*i];
                paid_out += gross;
                self.market.pay(&o.id, 0, o.owner, c.quote, Fixed(gross - fee));
                self.route_curve_fee(&token, *fee);
            }
            for i in &s.refunded {
                let o = &sells[*i];
                self.market.pay(&o.id, 1, o.owner, c.token, o.amount_in);
            }
            if s.lots > 0 {
                let _ = self.burn(token, Fixed(s.lots as i128 * LOT.0));
            }
            {
                let cur = self.market.curves.get_mut(&token).expect("curve exists");
                cur.sold -= s.lots;
                cur.reserve = Fixed(cur.reserve.0 - paid_out);
            }

            let sold = self.market.curves[&token].sold;
            let b = clear_buys(sold, c.fee_bps, c.unit, &buys);
            let mut paid_in: i128 = 0;
            for (i, lots, paid, fee) in &b.filled {
                let o = &buys[*i];
                paid_in += paid;
                self.market.pay(&o.id, 0, o.owner, c.token, Fixed(*lots as i128 * LOT.0));
                self.market.pay(&o.id, 1, o.owner, c.quote, Fixed(o.amount_in.0 - paid - fee));
                self.route_curve_fee(&token, *fee);
            }
            for i in &b.refunded {
                let o = &buys[*i];
                self.market.pay(&o.id, 1, o.owner, c.quote, o.amount_in);
            }
            if b.lots > 0 {
                let _ = self.issue(token, Fixed(b.lots as i128 * LOT.0));
            }
            let graduates = {
                let cur = self.market.curves.get_mut(&token).expect("curve exists");
                cur.sold += b.lots;
                cur.reserve = Fixed(cur.reserve.0 + paid_in);
                cur.sold >= GRADUATE_AT
            };
            if graduates {
                self.graduate(&token);
            }
        }
    }

    /// The curve's unsold lots and everything it holds for the pool open a
    /// pool whose every share is locked; the creator's escrow and bond are
    /// paid. Atomic: if the pool cannot be made, nothing moves and the curve
    /// stays as it is (sells still work; buys find no lots).
    fn graduate(&mut self, token: &Address) {
        let c = self.market.curves[token];
        let pid = pool_id(&c.token, &c.quote);
        if self.market.pools.contains_key(&pid) {
            return;
        }
        let tokens = Fixed((LOTS - c.sold) as i128 * LOT.0);
        let quote = Fixed(c.reserve.0 + c.graduation.0);
        let (a, b, ra, rb) = if c.token < c.quote { (c.token, c.quote, tokens, quote) } else { (c.quote, c.token, quote, tokens) };
        let Some(shares) = geometric_mean(ra, rb).filter(|s| *s > MINIMUM_LIQUIDITY) else {
            return;
        };
        if self.issue(c.token, tokens).is_err() || self.issue(lp_asset(&pid), shares).is_err() {
            return;
        }
        self.market.pools.insert(
            pid,
            Pool { a, b, reserve_a: ra, reserve_b: rb, shares, locked: shares, fee_bps: POOL_FEE_BPS },
        );
        self.market.pay(&c.source, 0, c.creator, c.quote, Fixed(c.creator_fees.0 + c.bond.0));
        self.market.curves.insert(
            *token,
            Curve {
                reserve: Fixed::ZERO,
                graduation: Fixed::ZERO,
                creator_fees: Fixed::ZERO,
                bond: Fixed::ZERO,
                graduated: true,
                ..c
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_raises_seventeen_zec_and_opens_its_pool_at_its_last_price() {
        let raised = cost(0, GRADUATE_AT);
        assert_eq!(raised, 1_700_004_911 * ZAT); // 17.00004911
        let last = cost(GRADUATE_AT, 1); // the next lot's price
        assert_eq!(last, 58_819 * ZAT);
        let rest = (LOTS - GRADUATE_AT) as i128;
        // The pool opens at raised/rest per lot: within 0.01% of the curve's last.
        assert!((raised - last * rest).abs() * 10_000 < last * rest);
        assert_eq!(SUPPLY.0, 1_000_000_000 * zyn_vm::fixed::WAD);
    }

    #[test]
    fn cost_is_additive_so_splitting_a_purchase_costs_the_same() {
        for (q, a, b) in [(0, 1, 1), (10, 500, 1_234), (40_000, 7, 11_098)] {
            assert_eq!(cost(q, a) + cost(q + a, b), cost(q, a + b));
        }
    }

    #[test]
    fn lots_for_is_the_most_a_budget_buys() {
        for (q, budget) in [(0u32, 1i128), (0, cost(0, 800)), (30_000, cost(30_000, 17) + 1), (50_000, i128::MAX / 4)] {
            let n = lots_for(q, budget, GRADUATE_AT - q.min(GRADUATE_AT));
            assert!(cost(q, n) <= budget);
            if q + n < GRADUATE_AT {
                assert!(cost(q, n + 1) > budget, "one more lot is out of reach");
            }
        }
    }

    fn order(n: u8, buy: bool, amount: i128, min_out: i128) -> CurveOrder {
        CurveOrder { id: [n; 32], owner: [n; 32], token: [9; 32], buy, amount_in: Fixed(amount), min_out: Fixed(min_out) }
    }

    #[test]
    fn buyers_share_one_price_and_the_curve_is_never_short() {
        let orders = [order(1, true, 3 * 100_000_000 * ZAT / 100, 0), order(2, true, 100_000_000 * ZAT / 100, 0)];
        let f = clear_buys(1_000, 100, ZAT, &orders);
        let paid: i128 = f.filled.iter().map(|x| x.2).sum();
        assert!(paid >= cost(1_000, f.lots), "buyers pay at least what the lots cost");
        // Three times the money, three times the lots (to a lot).
        let (a, b) = (f.filled[0].1 as i128, f.filled[1].1 as i128);
        assert!((a - 3 * b).abs() <= 3, "{a} vs 3×{b}");
        for (i, _, p, fee) in &f.filled {
            assert!(p + fee <= orders[*i].amount_in.0, "nobody pays more than they put in");
        }
    }

    #[test]
    fn order_within_the_batch_changes_nothing() {
        let a = [order(1, true, 50_000_000 * ZAT, 0), order(2, true, 20_000_000 * ZAT, 0), order(3, true, 7_000_000 * ZAT, 0)];
        let b = [a[2], a[0], a[1]];
        let fa = clear_buys(5_000, 100, ZAT, &a);
        let fb = clear_buys(5_000, 100, ZAT, &b);
        let by = |f: &BuyFill, o: &[CurveOrder]| {
            let mut v: Vec<_> = f.filled.iter().map(|x| (o[x.0].id, x.1, x.2)).collect();
            v.sort();
            v
        };
        assert_eq!(by(&fa, &a), by(&fb, &b));
    }

    #[test]
    fn a_batch_never_buys_past_graduation() {
        let whale = [order(1, true, 1_000 * 100_000_000 * ZAT, 0)];
        let f = clear_buys(GRADUATE_AT - 10, 0, ZAT, &whale);
        assert_eq!(f.lots, 10);
        assert_eq!(f.filled[0].2, cost(GRADUATE_AT - 10, 10));
    }

    #[test]
    fn sellers_get_one_price_and_the_reserve_keeps_the_dust() {
        let sells = [order(1, false, 3 * LOT.0, 0), order(2, false, 4 * LOT.0, 0)];
        let f = clear_sells(20_000, 100, ZAT, &sells);
        assert_eq!(f.lots, 7);
        let gross: i128 = f.filled.iter().map(|x| x.1).sum();
        assert!(gross <= cost(20_000 - 7, 7));
        // Same lots, same money: 4/3 of the other, to the rounding.
        assert!((f.filled[1].1 * 3 - f.filled[0].1 * 4).abs() <= 7);
    }

    #[test]
    fn an_unmet_minimum_is_refunded_and_the_rest_reprice() {
        let orders = [order(1, true, 10_000_000 * ZAT, 0), order(2, true, 10_000_000 * ZAT, 10_000 * LOT.0)];
        let f = clear_buys(0, 0, ZAT, &orders);
        assert_eq!(f.refunded, vec![1]);
        assert_eq!(f.filled.len(), 1);
    }
}
