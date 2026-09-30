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

use zyn_lend::{position as rules, Pool, Risk, CURVE, WAD};
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

/// The receipt asset for lending `underlying`: lZEC, lETH, lSOL.
pub fn lend_asset(underlying: &Address) -> Address {
    id(b"objectvm.lend.v1", &[underlying])
}

/// Every lend intent's claims and records derive from this: only the owner,
/// who knows the nonce, can find them.
pub fn lend_source(owner: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.lend-src.v1", &[owner, nonce])
}

/// Where a liquidation's leftovers are claimable: from the position, which
/// only its owner knew.
pub fn lot_source(position: &Hash) -> Hash {
    id(b"objectvm.lend-lot.v1", &[position])
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
}

impl Lend {
    pub fn is_empty(&self) -> bool {
        self.listed.is_empty() && self.positions.is_empty() && self.lots.is_empty() && self.bids.is_empty()
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
}

pub(crate) fn encode(e: &mut Encoder, l: &Lend) {
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

pub(crate) fn decode(d: &mut Decoder) -> Option<Lend> {
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
            },
        );
    }
    for _ in 0..d.u32().ok()? {
        l.bids.push(Bid { id: d.hash().ok()?, owner: d.account().ok()?, lot: d.hash().ok()?, pay: d.fixed().ok()? });
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
        let pool = &self.market.lend.listed.get(&p.borrow_asset).ok_or("that asset is not lent here")?.pool;
        let owed = math(pool.owed(p.debt_shares.0))?;
        let cv = math(rules::value(p.collateral.0, self.lend_price(&p.collateral_asset)?))?;
        let dv = math(rules::debt_value(owed, self.lend_price(&p.borrow_asset)?))?;
        Ok((cv, dv))
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
        for l in self.market.lend.listed.values_mut() {
            pool_err(l.pool.accrue(at, &CURVE))?;
        }
        self.market.lend.time = at;
        for (a, p) in prices {
            self.market.lend.prices.insert(*a, *p);
        }
        Ok(Receipt::Lent { id: [0u8; 32] })
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
            let risk = self.market.lend.listed.get(&p.collateral_asset).ok_or("that collateral is not accepted here")?.risk;
            let (cv, dv) = self.position_values(&p)?;
            if !math(rules::within_limit(cv, dv, &risk))? {
                return Err("that would leave the position over its maximum");
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
        if self.market.lend.bids.iter().filter(|b| b.lot == pos).count() >= MAX_BIDS_PER_LOT {
            return Err("this lot has all the bids it can take this epoch; bid next epoch");
        }
        let f = self.plan_funding(bidder, inputs, change, &[(l.borrow_asset, pay)])?;
        self.apply_funding(bidder, inputs, f);
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
        self.clear_bids();
        self.settle_lots();
        self.open_lots();
        let assets: Vec<Address> = self.market.lend.listed.keys().copied().collect();
        for a in assets {
            let take = self.market.lend.listed.get_mut(&a).expect("listed").pool.take_reserves();
            let _ = self.market.to_pol(a, Fixed(take));
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
            // Sell only what covers the debt and its penalty.
            let (Some(price), Some(cap)) = (
                Lend::lot_price(&lot, self.epoch),
                Lend::lot_price(&lot, self.epoch).and_then(|p| self.lot_needs(&lot, p)),
            ) else {
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
        let target = owed.checked_add(zyn_lend::math::bps(owed, risk.penalty_bps).ok()?)?;
        let short = (target - lot.proceeds.0).max(0);
        let coll = zyn_lend::math::mul_div_up(short, WAD, price).ok()?;
        Some(coll.min(lot.collateral.0))
    }

    /// A lot is done when it has covered its debt and penalty, or sold
    /// everything. Proceeds repay the pool; the rest is the owner's.
    fn settle_lots(&mut self) {
        let lots: Vec<(Hash, Lot)> = self.market.lend.lots.iter().map(|(p, l)| (*p, *l)).collect();
        for (pos, lot) in lots {
            let Some(listed) = self.market.lend.listed.get(&lot.borrow_asset).copied() else { continue };
            let Some(risk) = self.market.lend.listed.get(&lot.collateral_asset).map(|l| l.risk) else { continue };
            let Ok(owed) = listed.pool.owed(lot.debt_shares.0) else { continue };
            let covered = zyn_lend::math::bps(owed, risk.penalty_bps).map(|p| lot.proceeds.0 >= owed + p).unwrap_or(false);
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
            // What the proceeds could not cover is bad debt: the lenders'.
            if shares > 0 {
                let _ = pool.write_off(shares);
            }
            let _ = pool.credit(s.penalty);
            self.market.lend.listed.get_mut(&lot.borrow_asset).expect("listed").pool = pool;
            let src = lot_source(&pos);
            self.market.pay(&src, 0, lot.owner, lot.borrow_asset, Fixed(s.to_owner));
            self.market.pay(&src, 1, lot.owner, lot.collateral_asset, lot.collateral);
            self.market.lend.lots.remove(&pos);
        }
    }

    /// Positions past their line become lots.
    fn open_lots(&mut self) {
        let positions: Vec<(Hash, Position)> = self.market.lend.positions.iter().map(|(i, p)| (*i, *p)).collect();
        for (id, p) in positions {
            if p.debt_shares.0 == 0 {
                continue;
            }
            let Some(risk) = self.market.lend.listed.get(&p.collateral_asset).map(|l| l.risk) else { continue };
            // No price, no verdict: a position is never liquidated blind.
            let Ok((cv, dv)) = self.position_values(&p) else { continue };
            if !matches!(rules::liquidatable(cv, dv, &risk), Ok(true)) {
                continue;
            }
            let (Ok(pc), Ok(pb)) = (self.lend_price(&p.collateral_asset), self.lend_price(&p.borrow_asset)) else { continue };
            let Ok(rate) = zyn_lend::math::mul_div(pc, WAD, pb) else { continue };
            self.market.lend.positions.remove(&id);
            self.market.lend.lots.insert(
                id,
                Lot {
                    owner: p.owner,
                    collateral_asset: p.collateral_asset,
                    collateral: p.collateral,
                    borrow_asset: p.borrow_asset,
                    debt_shares: p.debt_shares,
                    rate: Fixed(rate),
                    opened: self.epoch + 1,
                    proceeds: Fixed::ZERO,
                },
            );
        }
    }
}

pub(crate) fn source_in_use(l: &Lend, source: &Hash) -> bool {
    l.positions.contains_key(source) || l.lots.contains_key(source) || l.bids.iter().any(|b| &b.id == source)
}

/// For the claim sources a wallet derives: every claim a lend source can pay.
pub fn claim_ids(source: &Hash) -> [Hash; 2] {
    [claim_id(source, 0), claim_id(source, 1)]
}
