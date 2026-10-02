//! Lending on the object ledger: Zyn Lend (`zyn-io/zyn-lend`).
//!
//! The rules — pools, interest, health, how liquidation proceeds are split —
//! live in the `zyn-lend` crate, open source. This module holds the state and
//! connects it to notes, claims and the seal.
//!
//! ```text
//!   listed     one pool per asset, with its collateral risk        public
//!   prices     posted by the operator with a timestamp             public
//!   positions  one collateral, one borrowed asset, one owner      blinded
//!   lots       liquidated collateral in a descending auction       public, no owner
//!   bids       this epoch's bids on lots                          blinded
//! ```
//!
//! # Time
//!
//! The ledger has no clock, only epochs. Interest runs on the timestamp the
//! operator posts with its prices: every pool accrues to it when prices
//! arrive. The operator's price loop is also what keeps epochs sealing.
//!
//! Under rules v2 (DECISIONS §160) the operator also posts the time alone
//! (`LendTime`, every few seconds), and everything that used to count epochs
//! counts that clock instead: a price's age, an auction's step, the backstop.
//! Epochs seal on the 64th intent too, refused ones included, so anyone could
//! make them pass; nobody but the operator moves the clock.
//!
//! # Liquidation (DECISIONS §12f)
//!
//! Not an instant sale into a pool: a **Dutch auction**. A position past its
//! line becomes a lot at the seal, priced at 102% of the posted rate and
//! falling a point per epoch to a floor of 85%. Anyone bids in the borrowed
//! asset; all of an epoch's bids on a lot clear at that epoch's one price,
//! pro rata if they want more than is offered. A lot sells only what covers
//! the debt and its penalty; the rest of the collateral goes back to the
//! owner. Bidders see the lot, never whose position it was.
//!
//! # Payouts
//!
//! The ledger cannot write a note for someone (it cannot encrypt to them), so
//! everything it pays — receipt shares, withdrawals, borrowed coins, auction
//! fills, refunds, what a liquidation leaves over — is a claim, collected
//! with `Op::Claim` like any other.

use std::collections::BTreeMap;

use zyn_lend::{position as rules, Curve, Pool, Risk, CURVE, CURVE_V2, WAD};
use zyn_vm::commit::{merkle_root, Encoder, Hash};
use zyn_vm::derive::Address;
use zyn_vm::read::Decoder;
use zyn_vm::spec::AccountId;
use zyn_vm::Fixed;

use crate::market::{claim_id, id, pool_id};
use crate::{ObjectVm, Output, Receipt};

/// An auction starts this far above the posted rate…
pub const AUCTION_START_BPS: i128 = 10_200;
/// …falls this much per epoch…
pub const AUCTION_STEP_BPS: i128 = 100;
/// …and stops here.
pub const AUCTION_FLOOR_BPS: i128 = 8_500;
/// Bids on one lot in one epoch.
pub const MAX_BIDS_PER_LOT: usize = 64;

// ---- Guarded mode (on from the first `LendLimits`) --------------------------
//
// Everything below applies only once the operator has set limits. Intents
// applied before that replay exactly as they did, so every root a chain has
// already committed still reproduces; the state-root section only grows once
// guarded mode is on.

/// Prices older than this many epochs are stale: no borrowing, no taking
/// collateral out, and no liquidation on them. The node seals on its clock
/// while lending is listed, so epochs keep counting time when nobody trades.
pub const MAX_PRICE_AGE_EPOCHS: u64 = 12;
/// A posted price this far from the last confirmed one is held, not used,
/// until the next post confirms it…
pub const PRICE_JUMP_BPS: i128 = 1_500;
/// …by landing within this of the held one.
pub const PRICE_CONFIRM_BPS: i128 = 200;
/// A lot unsold this many epochs after it opened (it reached its floor at
/// epoch 17) goes to Zyn's pool batch…
pub const BACKSTOP_AFTER_EPOCHS: u64 = 37;
/// …but never for less than this share of its opening rate.
pub const BACKSTOP_MIN_BPS: i128 = 7_000;
/// The reserve share of interest fills an asset's insurance to this share of
/// its pool's debt before any of it goes to the ZYN pot.
pub const INSURANCE_TARGET_BPS: i128 = 500;

// ---- Rules v2 (on from `LendRules { version: 2 }`, DECISIONS §160) ----------
//
// Like guarded mode: nothing below applies before the operator switches it
// on, and the state root's lend section grows (`objectvm.lend.v3`) only then,
// so every root sealed before keeps.

pub const RULES_V2: u32 = 2;
/// A price not posted for this many seconds of the ledger's clock is stale.
pub const MAX_PRICE_AGE_SECS: u64 = 300;
/// An auction falls `AUCTION_STEP_BPS` per this many seconds…
pub const AUCTION_STEP_SECS: u64 = 60;
/// …so it reaches its floor this long after it opens…
pub const AUCTION_FLOOR_AFTER_SECS: u64 = ((AUCTION_START_BPS - AUCTION_FLOOR_BPS) / AUCTION_STEP_BPS) as u64 * AUCTION_STEP_SECS;
/// …and goes to the pool batch 20 minutes after that.
pub const BACKSTOP_AFTER_SECS: u64 = AUCTION_FLOOR_AFTER_SECS + 20 * 60;
/// A held move is confirmed by this many posts in a row on the same side of
/// the confirmed price, so a market that keeps falling is not held forever.
pub const PRICE_CONFIRM_POSTS: u32 = 3;
/// The smallest bid is this share of what the lot owes (or the pool's
/// smallest loan, if that is less), so dust cannot fill a lot's bid slots.
pub const MIN_BID_BPS: i128 = 100;
/// Collateral of one asset backing loans of another may be at most this
/// share of its reserve in Zyn's pool for the pair: what an auction and the
/// backstop can actually sell.
pub const DEPTH_LIMIT_BPS: i128 = 2_500;

/// What the operator allows per asset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    /// Most of this asset held as collateral, across every position and lot.
    pub collateral_cap: Fixed,
    /// Most of this asset's pool that may be borrowed.
    pub borrow_cap: Fixed,
    /// The smallest loan of this asset, in WAD dollars: a lot too small to be
    /// worth bidding on is bad debt waiting to happen.
    pub min_borrow_usd: Fixed,
}

/// Who owns a backstop order while it is in the batch: nobody. Its claims are
/// swept back into the lot in the same seal, so none is ever left for anyone.
pub const BACKSTOP_OWNER: AccountId = [0u8; 32];

pub fn backstop_order(position: &Hash, epoch: u64) -> Hash {
    id(b"objectvm.lend-backstop.v1", &[position, &epoch.to_be_bytes()])
}

/// The receipt asset for lending `underlying`: lZEC, lETH, lSOL.
pub fn lend_asset(underlying: &Address) -> Address {
    id(b"objectvm.lend.v1", &[underlying])
}

/// Every lend intent's claims and records derive from this: only the owner,
/// who knows the nonce, can find them.
pub fn lend_source(owner: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.lend-src.v1", &[owner, nonce])
}

/// A liquidation lot's public id: bidders name it, and it never gives away
/// the position it came from.
pub fn lot_source(position: &Hash) -> Hash {
    id(b"objectvm.lend-lot.v1", &[position])
}

/// Where a liquidation's leftovers are claimable: from the position, which
/// only its owner knew. Not the lot's public id — if it were, anyone could
/// watch the claim and see when it is collected, and which other lots' claims
/// vanish in the same collection, tying one owner's positions together.
pub fn leftover_source(position: &Hash) -> Hash {
    id(b"objectvm.lend-left.v1", &[position])
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Listed {
    pub risk: Risk,
    pub pool: Pool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Position {
    pub owner: AccountId,
    pub collateral_asset: Address,
    pub collateral: Fixed,
    pub borrow_asset: Address,
    pub debt_shares: Fixed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Lot {
    pub owner: AccountId,
    pub collateral_asset: Address,
    /// Collateral still for sale.
    pub collateral: Fixed,
    pub borrow_asset: Address,
    pub debt_shares: Fixed,
    /// Borrowed-asset units per collateral unit at the posted prices when the
    /// lot opened (WAD).
    pub rate: Fixed,
    pub opened: u64,
    /// What bids have paid so far, in the borrowed asset.
    pub proceeds: Fixed,
    /// Rules v2: the ledger time it opened, which its price steps from…
    pub opened_at: u64,
    /// …and its debt, fixed when it opened: a lot owes no more interest.
    pub tab: Fixed,
    /// Guarded mode: debt written off when the lot opened, because even its
    /// floor price would not cover it, and paid from insurance. Recovered
    /// first if the auction does better — lenders, then the insurance.
    pub written_off: Fixed,
    pub insured: Fixed,
    /// A backstop order of this lot's collateral in the current batch.
    pub backstop: Option<Hash>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bid {
    pub id: Hash,
    pub owner: AccountId,
    pub lot: Hash,
    pub pay: Fixed,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Lend {
    pub listed: BTreeMap<Address, Listed>,
    pub prices: BTreeMap<Address, Fixed>,
    pub time: u64,
    pub positions: BTreeMap<Hash, Position>,
    pub lots: BTreeMap<Hash, Lot>,
    pub bids: Vec<Bid>,
    /// On from the first `LendLimits`; see the constants above.
    pub guarded: bool,
    /// The epoch the last prices were posted in.
    pub priced_epoch: u64,
    /// Posted prices held back by the circuit breaker, awaiting confirmation.
    pub pending: BTreeMap<Address, Fixed>,
    pub limits: BTreeMap<Address, Limits>,
    /// Per asset: what covers bad debt before lenders do.
    pub insurance: BTreeMap<Address, Fixed>,
    /// 0, or [`RULES_V2`] from `LendRules`.
    pub rules: u32,
    /// Rules v2: when each asset's price was last posted, on the ledger clock.
    pub priced_at: BTreeMap<Address, u64>,
    /// Rules v2: posts in a row the breaker has held on the same side.
    pub held_posts: BTreeMap<Address, u32>,
}

impl Lend {
    pub fn is_empty(&self) -> bool {
        self.listed.is_empty() && self.positions.is_empty() && self.lots.is_empty() && self.bids.is_empty() && !self.guarded
    }

    pub fn v2(&self) -> bool {
        self.rules >= RULES_V2
    }

    /// The rate curve in force.
    pub fn curve(&self) -> &'static Curve {
        if self.v2() {
            &CURVE_V2
        } else {
            &CURVE
        }
    }

    /// Whether `asset`'s price may be acted on this epoch: posted, not held
    /// by the circuit breaker, not stale. Before guarded mode, any posted
    /// price will do, as it always did.
    pub fn fresh(&self, asset: &Address, epoch: u64) -> bool {
        if !self.prices.contains_key(asset) {
            return false;
        }
        if self.v2() {
            return self.live(asset) && !self.pending.contains_key(asset);
        }
        !self.guarded || (!self.pending.contains_key(asset) && epoch <= self.priced_epoch + MAX_PRICE_AGE_EPOCHS)
    }

    /// Rules v2: posted within [`MAX_PRICE_AGE_SECS`] of the ledger clock,
    /// held or not. Liquidation runs on a live price; borrowing needs a
    /// fresh one.
    pub fn live(&self, asset: &Address) -> bool {
        self.prices.contains_key(asset) && self.priced_at.get(asset).is_some_and(|t| t.saturating_add(MAX_PRICE_AGE_SECS) >= self.time)
    }

    /// The price that is worse for lenders, of the confirmed one and one the
    /// breaker holds: lower for collateral, higher for debt. What health and
    /// liquidation use under rules v2, so a crash is acted on while it is
    /// still being confirmed.
    pub fn worse_price(&self, asset: &Address, collateral: bool) -> Option<i128> {
        let c = self.prices.get(asset).map(|p| p.0).filter(|p| *p > 0)?;
        Some(match self.pending.get(asset).map(|p| p.0).filter(|p| *p > 0) {
            Some(h) if collateral => c.min(h),
            Some(h) => c.max(h),
            None => c,
        })
    }

    /// Rules v2: withdrawals from `asset`'s pool wait while its price, or
    /// the price of collateral borrowed against it, is held — lenders must
    /// not leave ahead of a loss the breaker is still confirming.
    pub fn withdraw_paused(&self, asset: &Address) -> bool {
        self.v2()
            && (self.pending.contains_key(asset)
                || self.positions.values().any(|p| &p.borrow_asset == asset && self.pending.contains_key(&p.collateral_asset))
                || self.lots.values().any(|l| &l.borrow_asset == asset && self.pending.contains_key(&l.collateral_asset)))
    }

    /// Rules v2: deposits into `asset`'s pool wait while a lot may still pay
    /// back a loss already written off, which belongs to the lenders who
    /// carried it.
    pub fn supply_paused(&self, asset: &Address) -> bool {
        self.v2() && self.lots.values().any(|l| &l.borrow_asset == asset && l.written_off.0 > 0)
    }

    /// Collateral of `collateral` held for loans of `borrowed`, in positions
    /// and lots.
    pub fn collateral_backing(&self, collateral: &Address, borrowed: &Address) -> i128 {
        let p: i128 = self.positions.values().filter(|p| &p.collateral_asset == collateral && &p.borrow_asset == borrowed).map(|p| p.collateral.0).sum();
        let l: i128 = self.lots.values().filter(|l| &l.collateral_asset == collateral && &l.borrow_asset == borrowed).map(|l| l.collateral.0).sum();
        p + l
    }

    /// Collateral of `asset` held across every position and lot.
    pub fn collateral_held(&self, asset: &Address) -> i128 {
        let p: i128 = self.positions.values().filter(|p| &p.collateral_asset == asset).map(|p| p.collateral.0).sum();
        let l: i128 = self.lots.values().filter(|l| &l.collateral_asset == asset).map(|l| l.collateral.0).sum();
        p + l
    }

    /// What lending holds of `asset` for somebody: pool cash, collateral in
    /// positions and lots, auction proceeds not yet settled, bids in escrow.
    pub fn holdings(&self, asset: &Address) -> Option<i128> {
        let mut t: i128 = 0;
        if let Some(l) = self.listed.get(asset) {
            t = t.checked_add(l.pool.cash)?;
        }
        for p in self.positions.values() {
            if &p.collateral_asset == asset {
                t = t.checked_add(p.collateral.0)?;
            }
        }
        for l in self.lots.values() {
            if &l.collateral_asset == asset {
                t = t.checked_add(l.collateral.0)?;
            }
            if &l.borrow_asset == asset {
                t = t.checked_add(l.proceeds.0)?;
            }
        }
        for b in &self.bids {
            if &self.lots.get(&b.lot)?.borrow_asset == asset {
                t = t.checked_add(b.pay.0)?;
            }
        }
        if let Some(i) = self.insurance.get(asset) {
            t = t.checked_add(i.0)?;
        }
        Some(t)
    }

    pub fn is_receipt(&self, asset: &Address) -> bool {
        self.listed.keys().any(|u| &lend_asset(u) == asset)
    }

    /// A lot's price this epoch: borrowed units per collateral unit.
    pub fn lot_price(lot: &Lot, epoch: u64) -> Option<i128> {
        let steps = epoch.saturating_sub(lot.opened) as i128;
        let bps = (AUCTION_START_BPS - AUCTION_STEP_BPS.checked_mul(steps)?).max(AUCTION_FLOOR_BPS);
        zyn_lend::math::mul_div(lot.rate.0, bps, zyn_lend::BPS).ok()
    }

    /// Rules v2: a lot's price at ledger time `now`, a step a minute.
    pub fn lot_price_at(lot: &Lot, now: u64) -> Option<i128> {
        let steps = (now.saturating_sub(lot.opened_at) / AUCTION_STEP_SECS).min(1_000) as i128;
        let bps = (AUCTION_START_BPS - AUCTION_STEP_BPS * steps).max(AUCTION_FLOOR_BPS);
        zyn_lend::math::mul_div(lot.rate.0, bps, zyn_lend::BPS).ok()
    }

    /// A lot's price now, under whichever rules are in force.
    pub fn auction_price(&self, lot: &Lot, epoch: u64) -> Option<i128> {
        if self.v2() {
            Lend::lot_price_at(lot, self.time)
        } else {
            Lend::lot_price(lot, epoch)
        }
    }
}

// ------------------------------------------------------------------ state root

fn position_leaf(id: &Hash, p: &Position) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.lend-position.v1")
        .bytes(id)
        .bytes(&p.owner)
        .bytes(&p.collateral_asset)
        .fixed(p.collateral)
        .bytes(&p.borrow_asset)
        .fixed(p.debt_shares);
    e.leaf()
}

fn bid_leaf(b: &Bid) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.lend-bid.v1").bytes(&b.id).bytes(&b.owner).bytes(&b.lot).fixed(b.pay);
    e.leaf()
}

fn put_pool(e: &mut Encoder, p: &Pool) {
    e.i128(p.cash).i128(p.debt_shares).i128(p.borrow_index).i128(p.supply_shares).i128(p.reserves).u64(p.last);
}

fn put_risk(e: &mut Encoder, r: &Risk) {
    e.u32(r.max_ltv_bps).u32(r.liquidation_bps).u32(r.penalty_bps);
}

/// The lending part of the market leaf. Pools, prices and auction lots in the
/// clear — a market nobody can price is not one — but a lot without its
/// owner. Positions and bids as roots of blinded leaves.
pub(crate) fn section(e: &mut Encoder, l: &Lend) {
    e.bytes(b"objectvm.lend.v1").u32(l.listed.len() as u32);
    for (a, x) in &l.listed {
        e.bytes(a);
        put_risk(e, &x.risk);
        put_pool(e, &x.pool);
    }
    e.u32(l.prices.len() as u32);
    for (a, p) in &l.prices {
        e.bytes(a).fixed(*p);
    }
    e.u64(l.time).u32(l.lots.len() as u32);
    for (id, lot) in &l.lots {
        e.bytes(&lot_source(id))
            .bytes(&lot.collateral_asset)
            .fixed(lot.collateral)
            .bytes(&lot.borrow_asset)
            .fixed(lot.debt_shares)
            .fixed(lot.rate)
            .u64(lot.opened)
            .fixed(lot.proceeds);
    }
    let positions: Vec<Hash> = l.positions.iter().map(|(i, p)| position_leaf(i, p)).collect();
    let bids: Vec<Hash> = l.bids.iter().map(bid_leaf).collect();
    e.u32(positions.len() as u32).bytes(&merkle_root(&positions)).u32(bids.len() as u32).bytes(&merkle_root(&bids));
    // Joined only in guarded mode, so every root sealed before it keeps.
    if l.guarded {
        e.bytes(b"objectvm.lend.v2");
        put_guarded(e, l, true);
    }
    // And rules v2's, only once they are on (§160).
    if l.v2() {
        e.bytes(b"objectvm.lend.v3");
        put_v2(e, l, true);
    }
}

/// Rules v2's state, named as `put_guarded` names lots.
fn put_v2(e: &mut Encoder, l: &Lend, public: bool) {
    e.u32(l.rules).u32(l.priced_at.len() as u32);
    for (a, t) in &l.priced_at {
        e.bytes(a).u64(*t);
    }
    e.u32(l.held_posts.len() as u32);
    for (a, n) in &l.held_posts {
        e.bytes(a).u32(*n);
    }
    e.u32(l.lots.len() as u32);
    for (i, lot) in &l.lots {
        e.bytes(&if public { lot_source(i) } else { *i }).u64(lot.opened_at).fixed(lot.tab);
    }
}

/// Guarded mode's state. In the root, a lot is named by its public id, as
/// above; in the saved state, by its position.
fn put_guarded(e: &mut Encoder, l: &Lend, public: bool) {
    e.u64(l.priced_epoch).u32(l.pending.len() as u32);
    for (a, p) in &l.pending {
        e.bytes(a).fixed(*p);
    }
    e.u32(l.limits.len() as u32);
    for (a, x) in &l.limits {
        e.bytes(a).fixed(x.collateral_cap).fixed(x.borrow_cap).fixed(x.min_borrow_usd);
    }
    e.u32(l.insurance.len() as u32);
    for (a, v) in &l.insurance {
        e.bytes(a).fixed(*v);
    }
    e.u32(l.lots.len() as u32);
    for (i, lot) in &l.lots {
        e.bytes(&if public { lot_source(i) } else { *i }).fixed(lot.written_off).fixed(lot.insured);
        match lot.backstop {
            Some(o) => e.bool(true).bytes(&o),
            None => e.bool(false),
        };
    }
}

/// `v10`: with rules v2's state (MARKET10); without it, as MARKET09 saved.
pub(crate) fn encode(e: &mut Encoder, l: &Lend, v10: bool) {
    e.u32(l.listed.len() as u32);
    for (a, x) in &l.listed {
        e.bytes(a);
        put_risk(e, &x.risk);
        put_pool(e, &x.pool);
    }
    e.u32(l.prices.len() as u32);
    for (a, p) in &l.prices {
        e.bytes(a).fixed(*p);
    }
    e.u64(l.time).u32(l.positions.len() as u32);
    for (i, p) in &l.positions {
        e.bytes(i).bytes(&p.owner).bytes(&p.collateral_asset).fixed(p.collateral).bytes(&p.borrow_asset).fixed(p.debt_shares);
    }
    e.u32(l.lots.len() as u32);
    for (i, lot) in &l.lots {
        e.bytes(i)
            .bytes(&lot.owner)
            .bytes(&lot.collateral_asset)
            .fixed(lot.collateral)
            .bytes(&lot.borrow_asset)
            .fixed(lot.debt_shares)
            .fixed(lot.rate)
            .u64(lot.opened)
            .fixed(lot.proceeds);
    }
    e.u32(l.bids.len() as u32);
    for b in &l.bids {
        e.bytes(&b.id).bytes(&b.owner).bytes(&b.lot).fixed(b.pay);
    }
    e.bool(l.guarded);
    put_guarded(e, l, false);
    if v10 {
        put_v2(e, l, false);
    }
}

fn get_pool(d: &mut Decoder) -> Option<Pool> {
    Some(Pool {
        cash: d.i128().ok()?,
        debt_shares: d.i128().ok()?,
        borrow_index: d.i128().ok()?,
        supply_shares: d.i128().ok()?,
        reserves: d.i128().ok()?,
        last: d.u64().ok()?,
    })
}

fn get_risk(d: &mut Decoder) -> Option<Risk> {
    Some(Risk { max_ltv_bps: d.u32().ok()?, liquidation_bps: d.u32().ok()?, penalty_bps: d.u32().ok()? })
}

pub(crate) fn decode(d: &mut Decoder, v9: bool, v10: bool) -> Option<Lend> {
    let mut l = Lend::default();
    for _ in 0..d.u32().ok()? {
        let a = d.hash().ok()?;
        let risk = get_risk(d)?;
        l.listed.insert(a, Listed { risk, pool: get_pool(d)? });
    }
    for _ in 0..d.u32().ok()? {
        l.prices.insert(d.hash().ok()?, d.fixed().ok()?);
    }
    l.time = d.u64().ok()?;
    for _ in 0..d.u32().ok()? {
        let i = d.hash().ok()?;
        l.positions.insert(
            i,
            Position {
                owner: d.account().ok()?,
                collateral_asset: d.hash().ok()?,
                collateral: d.fixed().ok()?,
                borrow_asset: d.hash().ok()?,
                debt_shares: d.fixed().ok()?,
            },
        );
    }
    for _ in 0..d.u32().ok()? {
        let i = d.hash().ok()?;
        l.lots.insert(
            i,
            Lot {
                owner: d.account().ok()?,
                collateral_asset: d.hash().ok()?,
                collateral: d.fixed().ok()?,
                borrow_asset: d.hash().ok()?,
                debt_shares: d.fixed().ok()?,
                rate: d.fixed().ok()?,
                opened: d.u64().ok()?,
                proceeds: d.fixed().ok()?,
                opened_at: 0,
                tab: Fixed::ZERO,
                written_off: Fixed::ZERO,
                insured: Fixed::ZERO,
                backstop: None,
            },
        );
    }
    for _ in 0..d.u32().ok()? {
        l.bids.push(Bid { id: d.hash().ok()?, owner: d.account().ok()?, lot: d.hash().ok()?, pay: d.fixed().ok()? });
    }
    // MARKET08 predates guarded mode and reads as not guarded.
    if v9 {
        l.guarded = d.u8().ok()? == 1;
        l.priced_epoch = d.u64().ok()?;
        for _ in 0..d.u32().ok()? {
            l.pending.insert(d.hash().ok()?, d.fixed().ok()?);
        }
        for _ in 0..d.u32().ok()? {
            let a = d.hash().ok()?;
            l.limits.insert(a, Limits { collateral_cap: d.fixed().ok()?, borrow_cap: d.fixed().ok()?, min_borrow_usd: d.fixed().ok()? });
        }
        for _ in 0..d.u32().ok()? {
            l.insurance.insert(d.hash().ok()?, d.fixed().ok()?);
        }
        for _ in 0..d.u32().ok()? {
            let i = d.hash().ok()?;
            let (w, ins) = (d.fixed().ok()?, d.fixed().ok()?);
            let backstop = if d.u8().ok()? == 1 { Some(d.hash().ok()?) } else { None };
            let lot = l.lots.get_mut(&i)?;
            lot.written_off = w;
            lot.insured = ins;
            lot.backstop = backstop;
        }
    }
    // MARKET09 predates rules v2 and reads as rules 0.
    if v10 {
        l.rules = d.u32().ok()?;
        for _ in 0..d.u32().ok()? {
            l.priced_at.insert(d.hash().ok()?, d.u64().ok()?);
        }
        for _ in 0..d.u32().ok()? {
            l.held_posts.insert(d.hash().ok()?, d.u32().ok()?);
        }
        for _ in 0..d.u32().ok()? {
            let i = d.hash().ok()?;
            let (at, tab) = (d.u64().ok()?, d.fixed().ok()?);
            let lot = l.lots.get_mut(&i)?;
            lot.opened_at = at;
            lot.tab = tab;
        }
    }
    Some(l)
}

// ------------------------------------------------------------------ intents

fn math<T>(r: Result<T, zyn_lend::MathError>) -> Result<T, &'static str> {
    r.map_err(|_| "a lending sum did not fit")
}

fn pool_err<T>(r: Result<T, zyn_lend::PoolError>) -> Result<T, &'static str> {
    r.map_err(|e| match e {
        zyn_lend::PoolError::Liquidity => "the pool does not have that much free",
        zyn_lend::PoolError::Amount => "an amount the pool cannot take",
        zyn_lend::PoolError::Shares => "more shares than there are",
        zyn_lend::PoolError::Clock => "the lending clock cannot run backwards",
        zyn_lend::PoolError::Math(_) => "a lending sum did not fit",
        zyn_lend::PoolError::Empty => "the pool has nothing left to share",
    })
}

impl ObjectVm {
    fn lend_price(&self, asset: &Address) -> Result<i128, &'static str> {
        self.market.lend.prices.get(asset).map(|p| p.0).filter(|p| *p > 0).ok_or("no price posted for that asset")
    }

    /// Units of `b` per unit of `a` in Zyn's pool for the pair, WAD.
    fn pool_ratio(&self, a: &Address, b: &Address) -> Result<i128, &'static str> {
        let p = self.market.pools.get(&pool_id(a, b)).ok_or("Zyn has no pool for this pair yet")?;
        let (ra, rb) = if &p.a == a { (p.reserve_a.0, p.reserve_b.0) } else { (p.reserve_b.0, p.reserve_a.0) };
        if ra <= 0 || rb <= 0 {
            return Err("that pool is empty");
        }
        math(zyn_lend::math::mul_div(rb, WAD, ra))
    }

    /// Collateral value and debt value of a position at posted prices.
    fn position_values(&self, p: &Position) -> Result<(i128, i128), &'static str> {
        self.position_values_at(p, self.lend_price(&p.collateral_asset)?, self.lend_price(&p.borrow_asset)?)
    }

    fn position_values_at(&self, p: &Position, pc: i128, pb: i128) -> Result<(i128, i128), &'static str> {
        let pool = &self.market.lend.listed.get(&p.borrow_asset).ok_or("that asset is not lent here")?.pool;
        let owed = math(pool.owed(p.debt_shares.0))?;
        let cv = math(rules::value(p.collateral.0, pc))?;
        let dv = math(rules::debt_value(owed, pb))?;
        Ok((cv, dv))
    }

    /// Rules v2's depth limit for collateral `c` backing loans of `b`:
    /// `(c's reserve in Zyn's pool for the pair, the limit, room left)`.
    /// `None` when there is no such pool, which already closes the pair.
    pub fn depth_room(&self, c: &Address, b: &Address) -> Option<(i128, i128, i128)> {
        let p = self.market.pools.get(&pool_id(c, b))?;
        let reserve = if &p.a == c { p.reserve_a.0 } else { p.reserve_b.0 };
        let limit = zyn_lend::math::mul_div(reserve.max(0), DEPTH_LIMIT_BPS, zyn_lend::BPS).ok()?;
        Some((reserve, limit, limit - self.market.lend.collateral_backing(c, b)))
    }

    fn check_depth(&self, c: &Address, b: &Address, more: i128) -> Result<(), &'static str> {
        let (_, _, room) = self.depth_room(c, b).ok_or("Zyn has no pool for this pair yet")?;
        if more > room {
            return Err("this pair is at its depth limit: collateral may be at most a quarter of Zyn's pool");
        }
        Ok(())
    }

    /// Rules v2: the smallest bid a lot takes now — 1% of what it owes or
    /// the borrowed pool's smallest loan, whichever is less, but never more
    /// than what the lot still needs to sell.
    pub fn lot_min_bid(&self, lot: &Lot) -> Option<i128> {
        let l = &self.market.lend;
        let owed = l.listed.get(&lot.borrow_asset)?.pool.owed(lot.debt_shares.0).ok()?;
        let base = owed.checked_add(lot.written_off.0)?.checked_add(lot.insured.0)?;
        let mut min = zyn_lend::math::mul_div(base, MIN_BID_BPS, zyn_lend::BPS).ok()?;
        let pb = l.prices.get(&lot.borrow_asset).map(|p| p.0).filter(|p| *p > 0);
        if let (Some(m), Some(pb)) = (l.limits.get(&lot.borrow_asset), pb) {
            min = min.min(zyn_lend::math::mul_div(m.min_borrow_usd.0, WAD, pb).ok()?);
        }
        let price = Lend::lot_price_at(lot, l.time)?;
        let needs = self.lot_needs(lot, price)?;
        let cost = zyn_lend::math::wmul_up(needs, price).ok()?;
        Some(min.min(cost).max(1))
    }

    /// Bring every pool's interest to ledger time `at`, at `curve`. All or
    /// nothing: a refused intent must leave every pool as it was.
    fn accrue_all(&mut self, at: u64, curve: &Curve) -> Result<(), &'static str> {
        let mut listed = self.market.lend.listed.clone();
        for l in listed.values_mut() {
            pool_err(l.pool.accrue(at, curve))?;
        }
        self.market.lend.listed = listed;
        Ok(())
    }

    fn own_position(&self, who: &AccountId, position: &Hash) -> Result<Position, &'static str> {
        let p = *self.market.lend.positions.get(position).ok_or("no such position")?;
        if &p.owner != who {
            return Err("not the signer's position");
        }
        Ok(p)
    }

    pub(crate) fn apply_lend_list(&mut self, asset: &Address, risk: Risk) -> Result<Receipt, &'static str> {
        if !risk.is_sound() {
            return Err("those risk numbers do not leave room between the maximum and the line");
        }
        if self.market.lend.v2() && !risk.is_sound_v2() {
            return Err("under rules v2 a liquidated debt and its penalty must fit under the auction floor");
        }
        if !self.issued.contains_key(asset) || self.market.is_derived(asset) || self.market.lend.is_receipt(asset) {
            return Err("only an asset that exists and was not made here can be lent");
        }
        let time = self.market.lend.time;
        match self.market.lend.listed.get_mut(asset) {
            // Listing again changes the risk, never the pool.
            Some(l) => l.risk = risk,
            None => {
                self.market.lend.listed.insert(*asset, Listed { risk, pool: Pool::new(time) });
            }
        }
        Ok(Receipt::Lent { id: lend_asset(asset) })
    }

    /// Set an asset's caps and minimum loan. The first one turns guarded mode
    /// on for good.
    pub(crate) fn apply_lend_limits(&mut self, asset: &Address, limits: Limits) -> Result<Receipt, &'static str> {
        if !self.market.lend.listed.contains_key(asset) {
            return Err("limits for an asset that is not lent here");
        }
        if limits.collateral_cap.0 < 0 || limits.borrow_cap.0 < 0 || limits.min_borrow_usd.0 < 0 {
            return Err("a limit below nothing");
        }
        if !self.market.lend.guarded {
            self.market.lend.guarded = true;
            // Guarded mode counts price age from here.
            self.market.lend.priced_epoch = self.epoch;
        }
        self.market.lend.limits.insert(*asset, limits);
        Ok(Receipt::Lent { id: lend_asset(asset) })
    }

    /// Switch rules v2 on, for good (§160). Guarded mode must be on, and every
    /// listed collateral must already fit v2's risk test.
    pub(crate) fn apply_lend_rules(&mut self, version: u32) -> Result<Receipt, &'static str> {
        let l = &self.market.lend;
        if version != RULES_V2 {
            return Err("no such lending rules");
        }
        if !l.guarded {
            return Err("rules v2 come after guarded mode's limits");
        }
        if l.v2() {
            return Err("those lending rules are already in force");
        }
        if l.listed.values().any(|x| !x.risk.is_sound_v2()) {
            return Err("a listed collateral's line and penalty do not fit under the auction floor; list it again first");
        }
        // Interest up to now on the old curve; v2's from here.
        let time = l.time;
        self.accrue_all(time, &CURVE)?;
        let l = &mut self.market.lend;
        let pools: BTreeMap<Address, Pool> = l.listed.iter().map(|(a, x)| (*a, x.pool)).collect();
        // Under the old rules the clock moved only with prices, so every
        // posted price is as old as the clock.
        let priced: Vec<Address> = l.prices.keys().copied().collect();
        for a in priced {
            l.priced_at.insert(a, time);
        }
        for lot in l.lots.values_mut() {
            lot.opened_at = time;
            lot.tab = Fixed(pools.get(&lot.borrow_asset).and_then(|p| p.owed(lot.debt_shares.0).ok()).unwrap_or(0));
        }
        l.rules = RULES_V2;
        Ok(Receipt::Lent { id: [0u8; 32] })
    }

    /// Rules v2: the operator's clock, posted on its own. Interest accrues to
    /// it; no price gets any younger.
    pub(crate) fn apply_lend_time(&mut self, at: u64) -> Result<Receipt, &'static str> {
        if !self.market.lend.v2() {
            return Err("the lending clock is posted under rules v2 only");
        }
        if at < self.market.lend.time {
            return Err("the lending clock cannot run backwards");
        }
        self.accrue_all(at, &CURVE_V2)?;
        self.market.lend.time = at;
        Ok(Receipt::Lent { id: [0u8; 32] })
    }

    /// Rules v2: add to a pool's insurance. Anyone may; it buys no shares
    /// and nothing is paid back.
    pub(crate) fn apply_lend_insure(
        &mut self,
        donor: &AccountId,
        asset: &Address,
        amount: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        if !self.market.lend.v2() {
            return Err("insurance takes deposits under rules v2 only");
        }
        if amount.0 <= 0 {
            return Err("an amount must be above nothing");
        }
        if !self.market.lend.listed.contains_key(asset) {
            return Err("that asset is not lent here");
        }
        let have = self.market.lend.insurance.get(asset).map(|x| x.0).unwrap_or(0);
        let total = have.checked_add(amount.0).ok_or("insurance overflow")?;
        let f = self.plan_funding(donor, inputs, change, &[(*asset, amount)])?;
        self.apply_funding(donor, inputs, f);
        self.market.lend.insurance.insert(*asset, Fixed(total));
        Ok(Receipt::Lent { id: lend_asset(asset) })
    }

    pub(crate) fn apply_lend_prices(&mut self, at: u64, prices: &[(Address, Fixed)]) -> Result<Receipt, &'static str> {
        if at < self.market.lend.time {
            return Err("prices older than the last ones");
        }
        for (a, p) in prices {
            if !self.market.lend.listed.contains_key(a) {
                return Err("a price for an asset that is not lent here");
            }
            if p.0 <= 0 {
                return Err("a price must be above nothing");
            }
        }
        // Interest up to the moment these prices describe, at the old rates.
        let curve = *self.market.lend.curve();
        for l in self.market.lend.listed.values_mut() {
            pool_err(l.pool.accrue(at, &curve))?;
        }
        self.market.lend.time = at;
        if self.market.lend.v2() {
            self.post_prices_v2(at, prices);
            return Ok(Receipt::Lent { id: [0u8; 32] });
        }
        if !self.market.lend.guarded {
            for (a, p) in prices {
                self.market.lend.prices.insert(*a, *p);
            }
            return Ok(Receipt::Lent { id: [0u8; 32] });
        }
        // The circuit breaker: a jump is held until the next post confirms it.
        let within = |a: i128, b: i128, bps: i128| (a - b).abs().checked_mul(zyn_lend::BPS).map(|x| x <= b.checked_mul(bps).unwrap_or(i128::MAX)).unwrap_or(false);
        for (a, p) in prices {
            let l = &mut self.market.lend;
            let accept = match (l.prices.get(a), l.pending.get(a)) {
                (None, _) => true,
                (Some(c), _) if within(p.0, c.0, PRICE_JUMP_BPS) => true,
                (_, Some(h)) if within(p.0, h.0, PRICE_CONFIRM_BPS) => true,
                _ => false,
            };
            if accept {
                l.prices.insert(*a, *p);
                l.pending.remove(a);
            } else {
                l.pending.insert(*a, *p);
            }
        }
        self.market.lend.priced_epoch = self.epoch;
        Ok(Receipt::Lent { id: [0u8; 32] })
    }

    /// Rules v2's circuit breaker: a jump is held until a post lands within
    /// 2% of it, or until three posts in a row land on the same side of the
    /// confirmed price — a market that keeps falling is confirmed, not held.
    fn post_prices_v2(&mut self, at: u64, prices: &[(Address, Fixed)]) {
        let within = |a: i128, b: i128, bps: i128| (a - b).abs().checked_mul(zyn_lend::BPS).map(|x| x <= b.checked_mul(bps).unwrap_or(i128::MAX)).unwrap_or(false);
        let l = &mut self.market.lend;
        for (a, p) in prices {
            l.priced_at.insert(*a, at);
            let held = l.held_posts.get(a).copied().unwrap_or(0);
            let (accept, posts) = match (l.prices.get(a).map(|x| x.0), l.pending.get(a).map(|x| x.0)) {
                (None, _) => (true, 0),
                (Some(c), _) if within(p.0, c, PRICE_JUMP_BPS) => (true, 0),
                (Some(_), Some(h)) if within(p.0, h, PRICE_CONFIRM_BPS) => (true, 0),
                (Some(c), Some(h)) if (p.0 > c) == (h > c) => (held + 1 >= PRICE_CONFIRM_POSTS, held + 1),
                _ => (false, 1),
            };
            if accept {
                l.prices.insert(*a, *p);
                l.pending.remove(a);
                l.held_posts.remove(a);
            } else {
                l.pending.insert(*a, *p);
                l.held_posts.insert(*a, posts);
            }
        }
        l.priced_epoch = self.epoch;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_lend_supply(
        &mut self,
        lender: &AccountId,
        asset: &Address,
        amount: Fixed,
        nonce: &Hash,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let src = lend_source(lender, nonce);
        self.fresh_source(&src)?;
        if self.market.lend.supply_paused(asset) {
            return Err("deposits into this pool wait while an auction recovers a booked loss");
        }
        let mut pool = self.market.lend.listed.get(asset).ok_or("that asset is not lent here")?.pool;
        let shares = pool_err(pool.supply(amount.0))?;
        let f = self.plan_funding(lender, inputs, change, &[(*asset, amount)])?;
        self.apply_funding(lender, inputs, f);
        self.market.lend.listed.get_mut(asset).expect("listed").pool = pool;
        let receipt = lend_asset(asset);
        self.issue(receipt, Fixed(shares))?;
        self.market.pay(&src, 0, *lender, receipt, Fixed(shares));
        Ok(Receipt::Lent { id: src })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_lend_withdraw(
        &mut self,
        lender: &AccountId,
        asset: &Address,
        shares: Fixed,
        nonce: &Hash,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let src = lend_source(lender, nonce);
        self.fresh_source(&src)?;
        if self.market.lend.withdraw_paused(asset) {
            return Err("withdrawals from this pool wait for a held price to be confirmed");
        }
        let mut pool = self.market.lend.listed.get(asset).ok_or("that asset is not lent here")?.pool;
        let amount = pool_err(pool.withdraw(shares.0))?;
        let receipt = lend_asset(asset);
        let f = self.plan_funding(lender, inputs, change, &[(receipt, shares)])?;
        self.apply_funding(lender, inputs, f);
        self.burn(receipt, shares)?;
        self.market.lend.listed.get_mut(asset).expect("listed").pool = pool;
        self.market.pay(&src, 0, *lender, *asset, Fixed(amount));
        Ok(Receipt::Lent { id: src })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_lend_open(
        &mut self,
        borrower: &AccountId,
        collateral_asset: &Address,
        collateral: Fixed,
        borrow_asset: &Address,
        amount: Fixed,
        nonce: &Hash,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        if collateral_asset == borrow_asset {
            return Err("collateral and the borrowed asset must differ");
        }
        if collateral.0 <= 0 || amount.0 <= 0 {
            return Err("an amount must be above nothing");
        }
        let src = lend_source(borrower, nonce);
        self.fresh_source(&src)?;
        if !self.market.lend.listed.contains_key(collateral_asset) {
            return Err("that collateral is not accepted here");
        }
        let risk = self.market.lend.listed[collateral_asset].risk;
        if !self.market.lend.fresh(collateral_asset, self.epoch) || !self.market.lend.fresh(borrow_asset, self.epoch) {
            return Err("prices are stale or still moving; borrowing waits for the next confirmed price");
        }
        let (pc, pb) = (self.lend_price(collateral_asset)?, self.lend_price(borrow_asset)?);
        // Borrowing pauses in a pair whose pool disagrees with the posted price.
        if !math(rules::prices_agree(pc, pb, self.pool_ratio(collateral_asset, borrow_asset)?))? {
            return Err("borrowing in this pair is paused: Zyn's pool and the posted price disagree");
        }
        let mut pool = self.market.lend.listed.get(borrow_asset).ok_or("that asset is not lent here")?.pool;
        let shares = pool_err(pool.borrow(amount.0))?;
        let owed = math(pool.owed(shares))?;
        let cv = math(rules::value(collateral.0, pc))?;
        let dv = math(rules::debt_value(owed, pb))?;
        if !math(rules::within_limit(cv, dv, &risk))? {
            return Err("that is over the most this collateral may borrow");
        }
        if self.market.lend.guarded {
            let (lc, lb) = match (self.market.lend.limits.get(collateral_asset), self.market.lend.limits.get(borrow_asset)) {
                (Some(c), Some(b)) => (*c, *b),
                _ => return Err("borrowing in this pair is not open"),
            };
            if dv < lb.min_borrow_usd.0 {
                return Err("that loan is below the smallest this pool makes");
            }
            let held = self.market.lend.collateral_held(collateral_asset).checked_add(collateral.0).ok_or("collateral overflow")?;
            if held > lc.collateral_cap.0 {
                return Err("this collateral is at its cap");
            }
            if math(pool.debt())? > lb.borrow_cap.0 {
                return Err("this pool is at its borrowing cap");
            }
            if self.market.lend.v2() {
                self.check_depth(collateral_asset, borrow_asset, collateral.0)?;
            }
        }
        let f = self.plan_funding(borrower, inputs, change, &[(*collateral_asset, collateral)])?;
        self.apply_funding(borrower, inputs, f);
        self.market.lend.listed.get_mut(borrow_asset).expect("listed").pool = pool;
        self.market.lend.positions.insert(
            src,
            Position { owner: *borrower, collateral_asset: *collateral_asset, collateral, borrow_asset: *borrow_asset, debt_shares: Fixed(shares) },
        );
        self.market.pay(&src, 0, *borrower, *borrow_asset, amount);
        Ok(Receipt::Lent { id: src })
    }

    pub(crate) fn apply_lend_repay(
        &mut self,
        borrower: &AccountId,
        position: &Hash,
        amount: Fixed,
        nonce: &Hash,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let src = lend_source(borrower, nonce);
        self.fresh_source(&src)?;
        let mut p = self.own_position(borrower, position)?;
        let mut pool = self.market.lend.listed.get(&p.borrow_asset).ok_or("that asset is not lent here")?.pool;
        let (burned, used) = pool_err(pool.repay(amount.0, p.debt_shares.0))?;
        // Rules v2: a part payment may not leave a loan too small to be worth
        // liquidating.
        if self.market.lend.v2() && burned < p.debt_shares.0 {
            let left = math(pool.owed(p.debt_shares.0 - burned))?;
            let min = self.market.lend.limits.get(&p.borrow_asset).map(|m| m.min_borrow_usd.0).unwrap_or(0);
            if let Ok(pb) = self.lend_price(&p.borrow_asset) {
                if math(rules::debt_value(left, pb))? < min {
                    return Err("repay all of it, or leave at least the smallest loan");
                }
            }
        }
        let f = self.plan_funding(borrower, inputs, change, &[(p.borrow_asset, amount)])?;
        self.apply_funding(borrower, inputs, f);
        self.market.lend.listed.get_mut(&p.borrow_asset).expect("listed").pool = pool;
        p.debt_shares = Fixed(p.debt_shares.0 - burned);
        // Paying more than was owed: the rest comes back.
        self.market.pay(&src, 1, *borrower, p.borrow_asset, Fixed(amount.0 - used));
        self.store_position(position, p);
        Ok(Receipt::Lent { id: src })
    }

    pub(crate) fn apply_lend_add_collateral(
        &mut self,
        borrower: &AccountId,
        position: &Hash,
        amount: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let mut p = self.own_position(borrower, position)?;
        if amount.0 <= 0 {
            return Err("an amount must be above nothing");
        }
        // Rules v2: the collateral cap and the depth limit hold here as they
        // do when a position opens.
        if self.market.lend.v2() {
            if let Some(lc) = self.market.lend.limits.get(&p.collateral_asset) {
                let held = self.market.lend.collateral_held(&p.collateral_asset).checked_add(amount.0).ok_or("collateral overflow")?;
                if held > lc.collateral_cap.0 {
                    return Err("this collateral is at its cap");
                }
            }
            self.check_depth(&p.collateral_asset, &p.borrow_asset, amount.0)?;
        }
        let f = self.plan_funding(borrower, inputs, change, &[(p.collateral_asset, amount)])?;
        self.apply_funding(borrower, inputs, f);
        p.collateral = Fixed(p.collateral.0.checked_add(amount.0).ok_or("collateral overflow")?);
        self.market.lend.positions.insert(*position, p);
        Ok(Receipt::Lent { id: *position })
    }

    pub(crate) fn apply_lend_remove_collateral(
        &mut self,
        borrower: &AccountId,
        position: &Hash,
        amount: Fixed,
        nonce: &Hash,
    ) -> Result<Receipt, &'static str> {
        let src = lend_source(borrower, nonce);
        self.fresh_source(&src)?;
        let mut p = self.own_position(borrower, position)?;
        if amount.0 <= 0 || amount.0 > p.collateral.0 {
            return Err("more collateral than the position holds");
        }
        p.collateral = Fixed(p.collateral.0 - amount.0);
        if p.debt_shares.0 > 0 {
            if !self.market.lend.fresh(&p.collateral_asset, self.epoch) || !self.market.lend.fresh(&p.borrow_asset, self.epoch) {
                return Err("prices are stale or still moving; taking collateral out waits for the next confirmed price");
            }
            let risk = self.market.lend.listed.get(&p.collateral_asset).ok_or("that collateral is not accepted here")?.risk;
            let (cv, dv) = self.position_values(&p)?;
            if !math(rules::within_limit(cv, dv, &risk))? {
                return Err("that would leave the position over its maximum");
            }
            // Rules v2: as for borrowing, not while Zyn's pool disagrees with
            // the posted prices.
            if self.market.lend.v2() {
                let (pc, pb) = (self.lend_price(&p.collateral_asset)?, self.lend_price(&p.borrow_asset)?);
                if !math(rules::prices_agree(pc, pb, self.pool_ratio(&p.collateral_asset, &p.borrow_asset)?))? {
                    return Err("taking collateral out is paused: Zyn's pool and the posted price disagree");
                }
            }
        }
        self.market.pay(&src, 0, *borrower, p.collateral_asset, amount);
        self.store_position(position, p);
        Ok(Receipt::Lent { id: src })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_lend_bid(
        &mut self,
        bidder: &AccountId,
        lot: &Hash,
        pay: Fixed,
        nonce: &Hash,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let src = lend_source(bidder, nonce);
        self.fresh_source(&src)?;
        // Bidders name a lot by its public id (`lot_source`), never by the
        // position behind it.
        let (pos, l) = self
            .market
            .lend
            .lots
            .iter()
            .find(|(p, _)| &lot_source(p) == lot)
            .map(|(p, l)| (*p, *l))
            .ok_or("no such lot")?;
        if pay.0 <= 0 {
            return Err("a bid must pay something");
        }
        let full = self.market.lend.bids.iter().filter(|b| b.lot == pos).count() >= MAX_BIDS_PER_LOT;
        // Rules v2: no dust bids, and a full lot keeps its largest bids — a
        // new one pushes out the smallest, which is refunded.
        let mut evict = None;
        if self.market.lend.v2() {
            if pay.0 < self.lot_min_bid(&l).ok_or("that lot cannot be priced now")? {
                return Err("a bid below this lot's smallest");
            }
            if full {
                let smallest = self
                    .market
                    .lend
                    .bids
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| b.lot == pos)
                    .min_by(|(i, a), (j, b)| a.pay.0.cmp(&b.pay.0).then(j.cmp(i)))
                    .map(|(i, b)| (i, *b))
                    .ok_or("this lot's bids are full; outbid the smallest")?;
                if pay.0 <= smallest.1.pay.0 {
                    return Err("this lot's bids are full; outbid the smallest");
                }
                evict = Some(smallest);
            }
        } else if full {
            return Err("this lot has all the bids it can take this epoch; bid next epoch");
        }
        let f = self.plan_funding(bidder, inputs, change, &[(l.borrow_asset, pay)])?;
        self.apply_funding(bidder, inputs, f);
        if let Some((i, out)) = evict {
            self.market.lend.bids.remove(i);
            self.market.pay(&out.id, 1, out.owner, l.borrow_asset, out.pay);
        }
        self.market.lend.bids.push(Bid { id: src, owner: *bidder, lot: pos, pay });
        Ok(Receipt::Lent { id: src })
    }

    /// A position with nothing left in it is gone.
    fn store_position(&mut self, id: &Hash, p: Position) {
        if p.debt_shares.0 == 0 && p.collateral.0 == 0 {
            self.market.lend.positions.remove(id);
        } else {
            self.market.lend.positions.insert(*id, p);
        }
    }

    // -------------------------------------------------------------- the seal

    /// Lending's part of every seal: auctions clear, finished lots settle,
    /// positions past their line become lots, reserves go to the ZYN pot.
    pub(crate) fn settle_lend(&mut self) {
        if self.market.lend.is_empty() {
            return;
        }
        let v2 = self.market.lend.v2();
        if v2 {
            self.remark_lots();
            self.freeze_lots();
        }
        self.clear_bids();
        self.settle_lots();
        self.open_lots();
        if v2 {
            self.rebook_lots();
        }
        if self.market.lend.guarded {
            self.start_backstops();
        }
        let assets: Vec<Address> = self.market.lend.listed.keys().copied().collect();
        for a in assets {
            let pool = &mut self.market.lend.listed.get_mut(&a).expect("listed").pool;
            let take = pool.take_reserves();
            let debt = pool.debt().unwrap_or(0);
            if take == 0 {
                continue;
            }
            // Guarded: the reserve fills insurance before the ZYN pot.
            let to_insurance = if self.market.lend.guarded {
                let target = zyn_lend::math::bps(debt, INSURANCE_TARGET_BPS as u32).unwrap_or(0);
                let have = self.market.lend.insurance.get(&a).map(|x| x.0).unwrap_or(0);
                (target - have).clamp(0, take)
            } else {
                0
            };
            if to_insurance > 0 {
                let i = self.market.lend.insurance.entry(a).or_insert(Fixed::ZERO);
                i.0 += to_insurance;
            }
            let _ = self.market.to_pol(a, Fixed(take - to_insurance));
        }
    }

    /// Rules v2: every lot is priced from the current confirmed prices, as
    /// a Clipper is, not from the ones it opened at. A lot whose prices are
    /// stale keeps its rate, and takes no bids until they are live again.
    fn remark_lots(&mut self) {
        let l = &self.market.lend;
        let rates: Vec<(Hash, i128)> = l
            .lots
            .iter()
            .filter(|(_, lot)| lot.backstop.is_none() && l.live(&lot.collateral_asset) && l.live(&lot.borrow_asset))
            .filter_map(|(p, lot)| {
                let (pc, pb) = (l.prices.get(&lot.collateral_asset)?.0, l.prices.get(&lot.borrow_asset)?.0);
                Some((*p, zyn_lend::math::mul_div(pc, WAD, pb).ok()?))
            })
            .collect();
        for (p, rate) in rates {
            self.market.lend.lots.get_mut(&p).expect("lot").rate = Fixed(rate);
        }
    }

    /// Rules v2: a lot's debt stops growing when it opens. The interest the
    /// pool's index has added since is written off, with the reserve's cut
    /// of it, so neither lenders nor the ZYN pot count interest that a
    /// liquidated borrower will never pay.
    fn freeze_lots(&mut self) {
        let lots: Vec<(Hash, Lot)> = self.market.lend.lots.iter().map(|(p, l)| (*p, *l)).collect();
        for (pos, lot) in lots {
            let Some(listed) = self.market.lend.listed.get(&lot.borrow_asset).copied() else { continue };
            let mut pool = listed.pool;
            let Ok(keep) = zyn_lend::math::mul_div_up(lot.tab.0, WAD, pool.borrow_index) else { continue };
            if lot.debt_shares.0 <= keep || pool.forgive(lot.debt_shares.0 - keep, &CURVE_V2).is_err() {
                continue;
            }
            self.market.lend.listed.get_mut(&lot.borrow_asset).expect("listed").pool = pool;
            self.market.lend.lots.get_mut(&pos).expect("lot").debt_shares = Fixed(keep);
        }
    }

    /// Rules v2: each seal, what the lot can still be expected to fetch —
    /// its proceeds, and its collateral at the backstop's minimum of today's
    /// rate — is held against its debt, and any gap is booked now: insurance
    /// first, then written off. A price that keeps falling keeps booking.
    fn rebook_lots(&mut self) {
        let floor = AUCTION_FLOOR_BPS.min(BACKSTOP_MIN_BPS);
        let lots: Vec<(Hash, Lot)> = self.market.lend.lots.iter().filter(|(_, l)| l.backstop.is_none()).map(|(p, l)| (*p, *l)).collect();
        for (pos, mut lot) in lots {
            let Ok(rate) = zyn_lend::math::mul_div(lot.rate.0, floor, zyn_lend::BPS) else { continue };
            let Ok(sells) = zyn_lend::math::mul_div(lot.collateral.0, rate, WAD) else { continue };
            let Some(recovery) = sells.checked_add(lot.proceeds.0) else { continue };
            self.book_below(&mut lot, recovery);
            self.market.lend.lots.insert(pos, lot);
        }
    }

    /// A lot long past its floor goes to Zyn's pool batch, not below a
    /// minimum; `sweep_backstops` brings the result back after the batch.
    fn start_backstops(&mut self) {
        if self.market.lend.v2() {
            return self.start_backstops_v2();
        }
        let lots: Vec<(Hash, Lot)> = self.market.lend.lots.iter().map(|(p, l)| (*p, *l)).collect();
        for (pos, lot) in lots {
            if lot.backstop.is_some() || lot.collateral.0 <= 0 || self.epoch < lot.opened + BACKSTOP_AFTER_EPOCHS {
                continue;
            }
            let pid = pool_id(&lot.collateral_asset, &lot.borrow_asset);
            let Some(p) = self.market.pools.get(&pid).copied() else { continue };
            let Some(min_rate) = zyn_lend::math::mul_div(lot.rate.0, BACKSTOP_MIN_BPS, zyn_lend::BPS).ok() else { continue };
            let Some(min_out) = zyn_lend::math::mul_div(lot.collateral.0, min_rate, WAD).ok() else { continue };
            let oid = backstop_order(&pos, self.epoch);
            self.market.orders.push(crate::market::Order {
                id: oid,
                owner: BACKSTOP_OWNER,
                pool: pid,
                sell_a: p.a == lot.collateral_asset,
                amount_in: lot.collateral,
                min_out: Fixed(min_out),
            });
            let l = self.market.lend.lots.get_mut(&pos).expect("lot");
            l.collateral = Fixed::ZERO;
            l.backstop = Some(oid);
        }
    }

    /// Rules v2's backstop: 37 minutes after a lot opens (20 past its
    /// floor), only as much collateral as covers what it owes at the
    /// minimum rate — rounded up, never more than it has. The rest stays in
    /// the lot and goes back to the owner when it settles.
    fn start_backstops_v2(&mut self) {
        let l = &self.market.lend;
        let due: Vec<(Hash, Lot)> = l
            .lots
            .iter()
            .filter(|(_, lot)| lot.backstop.is_none() && lot.collateral.0 > 0 && l.time >= lot.opened_at.saturating_add(BACKSTOP_AFTER_SECS))
            .filter(|(_, lot)| l.live(&lot.collateral_asset) && l.live(&lot.borrow_asset))
            .map(|(p, lot)| (*p, *lot))
            .collect();
        for (pos, lot) in due {
            let pid = pool_id(&lot.collateral_asset, &lot.borrow_asset);
            let Some(p) = self.market.pools.get(&pid).copied() else { continue };
            let Ok(min_rate) = zyn_lend::math::mul_div(lot.rate.0, BACKSTOP_MIN_BPS, zyn_lend::BPS) else { continue };
            let Some(sell) = self.lot_needs(&lot, min_rate).filter(|x| *x > 0) else { continue };
            let Ok(min_out) = zyn_lend::math::mul_div(sell, min_rate, WAD) else { continue };
            let oid = backstop_order(&pos, self.epoch);
            self.market.orders.push(crate::market::Order {
                id: oid,
                owner: BACKSTOP_OWNER,
                pool: pid,
                sell_a: p.a == lot.collateral_asset,
                amount_in: Fixed(sell),
                min_out: Fixed(min_out),
            });
            let l = self.market.lend.lots.get_mut(&pos).expect("lot");
            l.collateral = Fixed(l.collateral.0 - sell);
            l.backstop = Some(oid);
        }
    }

    /// After the batch: a backstop's proceeds join the lot's, or its
    /// collateral comes back to wait for the next epoch. Under rules v2 a
    /// backstop the pool would not fill books all the lot's debt its
    /// proceeds have not covered, so it stops counting at face value; the
    /// lot keeps selling, and what it fetches later goes back to lenders.
    pub(crate) fn sweep_backstops(&mut self) {
        let v2 = self.market.lend.v2();
        let lots: Vec<(Hash, Lot)> = self.market.lend.lots.iter().filter(|(_, l)| l.backstop.is_some()).map(|(p, l)| (*p, *l)).collect();
        for (pos, lot) in lots {
            let oid = lot.backstop.expect("filtered");
            let filled = self.market.claims.remove(&claim_id(&oid, 0)).map(|c| c.amount.0).unwrap_or(0);
            let refunded = self.market.claims.remove(&claim_id(&oid, 1)).map(|c| c.amount.0).unwrap_or(0);
            let l = self.market.lend.lots.get_mut(&pos).expect("lot");
            l.proceeds = Fixed(l.proceeds.0 + filled);
            l.collateral = Fixed(l.collateral.0 + refunded);
            l.backstop = None;
            if v2 && filled == 0 {
                let mut lot = *l;
                let proceeds = lot.proceeds.0;
                self.book_below(&mut lot, proceeds);
                self.market.lend.lots.insert(pos, lot);
            }
        }
    }

    fn refund_bids(&mut self, bids: &[Bid], asset: Address) {
        for b in bids {
            self.market.pay(&b.id, 1, b.owner, asset, b.pay);
        }
    }

    /// Every bid on a lot pays the lot's one price this epoch.
    fn clear_bids(&mut self) {
        let bids = std::mem::take(&mut self.market.lend.bids);
        let lots: Vec<Hash> = self.market.lend.lots.keys().copied().collect();
        for pos in lots {
            let mine: Vec<Bid> = bids.iter().filter(|b| b.lot == pos).copied().collect();
            if mine.is_empty() {
                continue;
            }
            let mut lot = self.market.lend.lots[&pos];
            // Rules v2: not on a stale price — every bid is refunded.
            let l = &self.market.lend;
            let priced = !l.v2() || (l.live(&lot.collateral_asset) && l.live(&lot.borrow_asset));
            let price = if priced { l.auction_price(&lot, self.epoch) } else { None };
            // Sell only what covers the debt and its penalty.
            let (Some(price), Some(cap)) = (price, price.and_then(|p| self.lot_needs(&lot, p))) else {
                self.refund_bids(&mine, lot.borrow_asset);
                continue;
            };
            let total: i128 = mine.iter().map(|b| b.pay.0).sum();
            let want = zyn_lend::math::mul_div(total, WAD, price).unwrap_or(i128::MAX);
            let sold_total = want.min(cap);
            let mut sold = 0i128;
            let mut paid = 0i128;
            for b in &mine {
                // Pro rata by what each paid; everyone at the same price.
                let got = zyn_lend::math::mul_div(sold_total, b.pay.0, total).unwrap_or(0);
                let cost = zyn_lend::math::mul_div_up(got, price, WAD).unwrap_or(b.pay.0).min(b.pay.0);
                if got > 0 {
                    self.market.pay(&b.id, 0, b.owner, lot.collateral_asset, Fixed(got));
                }
                self.market.pay(&b.id, 1, b.owner, lot.borrow_asset, Fixed(b.pay.0 - if got > 0 { cost } else { 0 }));
                if got > 0 {
                    sold += got;
                    paid += cost;
                }
            }
            lot.collateral = Fixed(lot.collateral.0 - sold);
            lot.proceeds = Fixed(lot.proceeds.0 + paid);
            self.market.lend.lots.insert(pos, lot);
        }
        // Every bid named a lot that existed when it was placed, and lots only
        // close in `settle_lots`, after this: so every bid was on a lot above,
        // and every escrow was either filled or refunded.
    }

    /// How much collateral the lot still has to sell at `price` to cover its
    /// debt and penalty; never more than it holds.
    fn lot_needs(&self, lot: &Lot, price: i128) -> Option<i128> {
        let listed = self.market.lend.listed.get(&lot.borrow_asset)?;
        let risk = self.market.lend.listed.get(&lot.collateral_asset)?.risk;
        let owed = listed.pool.owed(lot.debt_shares.0).ok()?;
        let target = owed
            .checked_add(zyn_lend::math::bps(owed, risk.penalty_bps).ok()?)?
            .checked_add(lot.written_off.0)?
            .checked_add(lot.insured.0)?;
        let short = (target - lot.proceeds.0).max(0);
        let coll = zyn_lend::math::mul_div_up(short, WAD, price).ok()?;
        Some(coll.min(lot.collateral.0))
    }

    /// A lot is done when it has covered its debt and penalty, or sold
    /// everything. Proceeds repay the pool; the rest is the owner's.
    fn settle_lots(&mut self) {
        let lots: Vec<(Hash, Lot)> = self.market.lend.lots.iter().map(|(p, l)| (*p, *l)).collect();
        for (pos, lot) in lots {
            // A backstop still in the batch is settled after it comes back.
            if lot.backstop.is_some() {
                continue;
            }
            let Some(listed) = self.market.lend.listed.get(&lot.borrow_asset).copied() else { continue };
            let Some(risk) = self.market.lend.listed.get(&lot.collateral_asset).map(|l| l.risk) else { continue };
            let Ok(owed) = listed.pool.owed(lot.debt_shares.0) else { continue };
            let target = zyn_lend::math::bps(owed, risk.penalty_bps).map(|p| owed + p + lot.written_off.0 + lot.insured.0);
            let covered = target.map(|t| lot.proceeds.0 >= t).unwrap_or(false);
            if !(covered || lot.collateral.0 == 0) {
                continue;
            }
            let Ok(s) = rules::settle(owed, lot.proceeds.0, &risk) else { continue };
            let mut pool = listed.pool;
            let mut shares = lot.debt_shares.0;
            if s.repaid > 0 {
                if let Ok((burned, _)) = pool.repay(s.repaid, shares) {
                    shares -= burned;
                }
            }
            // What the proceeds could not cover is bad debt. Guarded: the
            // insurance pays first, the lenders only for what is left.
            if shares > 0 && self.market.lend.guarded {
                let have = self.market.lend.insurance.get(&lot.borrow_asset).map(|x| x.0).unwrap_or(0);
                let need = pool.owed(shares).unwrap_or(0);
                let pay = have.min(need);
                if pay > 0 {
                    if let Ok((burned, used)) = pool.repay(pay, shares) {
                        shares -= burned;
                        self.market.lend.insurance.get_mut(&lot.borrow_asset).expect("had insurance").0 -= used;
                    }
                }
            }
            if shares > 0 {
                let _ = pool.write_off(shares);
            }
            let _ = pool.credit(s.penalty);
            // Anything above debt and penalty recovers, in order, what was
            // written off (the lenders), then what insurance paid, then goes
            // to the owner.
            let mut left = s.to_owner;
            let to_lenders = left.min(lot.written_off.0);
            let _ = pool.credit(to_lenders);
            left -= to_lenders;
            let to_insurance = left.min(lot.insured.0);
            if to_insurance > 0 {
                self.market.lend.insurance.entry(lot.borrow_asset).or_insert(Fixed::ZERO).0 += to_insurance;
            }
            left -= to_insurance;
            self.market.lend.listed.get_mut(&lot.borrow_asset).expect("listed").pool = pool;
            let src = leftover_source(&pos);
            self.market.pay(&src, 0, lot.owner, lot.borrow_asset, Fixed(left));
            self.market.pay(&src, 1, lot.owner, lot.collateral_asset, lot.collateral);
            self.market.lend.lots.remove(&pos);
        }
    }

    /// Positions past their line become lots.
    fn open_lots(&mut self) {
        let guarded = self.market.lend.guarded;
        let v2 = self.market.lend.v2();
        let positions: Vec<(Hash, Position)> = self.market.lend.positions.iter().map(|(i, p)| (*i, *p)).collect();
        for (id, p) in positions {
            if p.debt_shares.0 == 0 {
                continue;
            }
            let l = &self.market.lend;
            // Rules v2: on live prices, the worse of confirmed and held.
            // Guarded: never on a stale price, or one the breaker is holding.
            if v2 {
                if !(l.live(&p.collateral_asset) && l.live(&p.borrow_asset)) {
                    continue;
                }
            } else if guarded && !(l.fresh(&p.collateral_asset, self.epoch) && l.fresh(&p.borrow_asset, self.epoch)) {
                continue;
            }
            let Some(risk) = self.market.lend.listed.get(&p.collateral_asset).map(|l| l.risk) else { continue };
            // No price, no verdict: a position is never liquidated blind.
            let values = if v2 {
                match (l.worse_price(&p.collateral_asset, true), l.worse_price(&p.borrow_asset, false)) {
                    (Some(pc), Some(pb)) => self.position_values_at(&p, pc, pb),
                    _ => continue,
                }
            } else {
                self.position_values(&p)
            };
            let Ok((cv, dv)) = values else { continue };
            if !matches!(rules::liquidatable(cv, dv, &risk), Ok(true)) {
                continue;
            }
            let (Ok(pc), Ok(pb)) = (self.lend_price(&p.collateral_asset), self.lend_price(&p.borrow_asset)) else { continue };
            let Ok(rate) = zyn_lend::math::mul_div(pc, WAD, pb) else { continue };
            self.market.lend.positions.remove(&id);
            let mut lot = Lot {
                owner: p.owner,
                collateral_asset: p.collateral_asset,
                collateral: p.collateral,
                borrow_asset: p.borrow_asset,
                debt_shares: p.debt_shares,
                rate: Fixed(rate),
                opened: self.epoch + 1,
                proceeds: Fixed::ZERO,
                opened_at: 0,
                tab: Fixed::ZERO,
                written_off: Fixed::ZERO,
                insured: Fixed::ZERO,
                backstop: None,
            };
            if v2 {
                // Its shortfall is booked with every other lot's, after this.
                lot.opened_at = self.market.lend.time;
                lot.tab = Fixed(self.market.lend.listed.get(&p.borrow_asset).and_then(|x| x.pool.owed(p.debt_shares.0).ok()).unwrap_or(0));
            } else if guarded {
                self.book_expected_shortfall(&mut lot);
            }
            self.market.lend.lots.insert(id, lot);
        }
    }

    /// What even the floor price would not cover is booked now — paid from
    /// insurance, then written off — so no lender can leave before a loss
    /// everyone else then carries. The auction recovers it if it does better.
    fn book_expected_shortfall(&mut self, lot: &mut Lot) {
        let Ok(floor_rate) = zyn_lend::math::mul_div(lot.rate.0, AUCTION_FLOOR_BPS, zyn_lend::BPS) else { return };
        let Ok(at_floor) = zyn_lend::math::mul_div(lot.collateral.0, floor_rate, WAD) else { return };
        self.book_below(lot, at_floor);
    }

    /// Book whatever of a lot's debt `at_floor` (what it can be counted on to
    /// fetch, in the borrowed asset) would not cover: from insurance first,
    /// then written off. Adds to what the lot has booked before; at a lot's
    /// opening, which is all guarded mode does, that is the same as setting
    /// it. Rules v2 keeps the lot's fixed debt in step.
    fn book_below(&mut self, lot: &mut Lot, at_floor: i128) {
        let Some(listed) = self.market.lend.listed.get(&lot.borrow_asset).copied() else { return };
        let mut pool = listed.pool;
        let Ok(owed) = pool.owed(lot.debt_shares.0) else { return };
        if at_floor >= owed {
            return;
        }
        let mut shares = lot.debt_shares.0;
        let have = self.market.lend.insurance.get(&lot.borrow_asset).map(|x| x.0).unwrap_or(0);
        let pay = have.min(owed - at_floor);
        if pay > 0 {
            if let Ok((burned, used)) = pool.repay(pay, shares) {
                shares -= burned;
                self.market.lend.insurance.get_mut(&lot.borrow_asset).expect("had insurance").0 -= used;
                lot.insured = Fixed(lot.insured.0 + used);
            }
        }
        let still = pool.owed(shares).unwrap_or(0) - at_floor;
        if still > 0 {
            let off = zyn_lend::math::mul_div(still, WAD, pool.borrow_index).unwrap_or(0).min(shares);
            if off > 0 && pool.write_off(off).is_ok() {
                shares -= off;
                lot.written_off = Fixed(lot.written_off.0 + zyn_lend::math::wmul(off, pool.borrow_index).unwrap_or(0));
            }
        }
        lot.debt_shares = Fixed(shares);
        if self.market.lend.v2() {
            lot.tab = Fixed(pool.owed(shares).unwrap_or(0));
        }
        self.market.lend.listed.get_mut(&lot.borrow_asset).expect("listed").pool = pool;
    }
}

pub(crate) fn source_in_use(l: &Lend, source: &Hash) -> bool {
    l.positions.contains_key(source) || l.lots.contains_key(source) || l.bids.iter().any(|b| &b.id == source)
}

/// For the claim sources a wallet derives: every claim a lend source can pay.
pub fn claim_ids(source: &Hash) -> [Hash; 2] {
    [claim_id(source, 0), claim_id(source, 1)]
}
