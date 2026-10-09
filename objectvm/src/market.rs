//! The private market: pools, batch swaps, and token launches.
//!
//! Everything here is tier 2, the same line the rest of the ledger holds
//! (§133.4): private from the public, visible to the operator. What the public
//! sees is what a market cannot work without — each pool's reserves and fee,
//! each launch's terms and how much it has raised. What it does not see is who
//! traded, who provided liquidity, who committed to a sale, or how much any one
//! of them moved. Those records are committed only as hashes blinded by a
//! nonce the user chose, so a stranger holding the state root cannot even test
//! a guess.
//!
//! # Batches, not a queue
//!
//! Swaps do not execute when they arrive. They are escrowed for the epoch and
//! cleared together at the seal, every order in a pool at **one price**:
//!
//! ```text
//!     p  =  (y + 2·b′) / (x + 2·a′)          (B per A)
//! ```
//!
//! where `x, y` are the reserves and `a′, b′` the fee-adjusted amounts sold into
//! the pool on each side. This is the function-maximising AMM of Canidio &
//! Fritsch ("Arbitrageurs' profits, LVR, and sandwich attacks", 2023), the rule
//! CoW AMM runs: the price at which the pool's post-trade marginal price equals
//! the price it traded at. Two things follow and both are the point:
//!
//! * **Nothing to sandwich.** Order within the epoch does not matter, so the
//!   sequencer — the one party who sees the orders — gains nothing by
//!   reordering them, and neither does anybody it might leak them to.
//! * **The pool cannot be drained.** `x′·y′ ≥ x·y` for every batch (the gap is
//!   `(x·b′ − y·a′)²`, see `clearing_never_loses_the_pool_value`), and each
//!   reserve stays above half of what it was.
//!
//! An order whose `min_out` the batch price does not meet is refunded whole
//! and the price recomputed without it, until every order left is satisfied.
//!
//! # Claims
//!
//! A batch settles at the seal, after the trader has signed and left, and the
//! VM cannot encrypt a note for anyone — it holds no randomness and no reason
//! to. So what the ledger owes is recorded as a **claim**: owner, asset,
//! amount. The owner redeems it with an ordinary signed intent carrying a note
//! they encrypted for themselves. Liquidity, launches and refunds all pay out
//! the same way, so there is one redemption path to get right rather than five.
//!
//! A claim's id is derived from the nonce in the intent that created it, which
//! only its owner knows. That is how a wallet finds what it is owed without an
//! "everything this account holds" query that would hand the same answer to
//! anyone who asked.
//!
//! # Launches
//!
//! A creator names a token, a sale size and a raise. The whole supply is issued
//! into escrow. Buyers commit the quote asset privately until the close, and at
//! the close everybody pays **the same price**: if the raise is over its hard
//! cap each commitment is taken pro rata and the rest refunded. Under the soft
//! cap everything is refunded and the token is burned. On success part of the
//! raise and a reserve of tokens seed a pool whose liquidity is locked for
//! good — nobody holds its shares, so nobody can pull it.

use std::collections::BTreeMap;

use zyn_vm::commit::{merkle_root, Encoder, Hash};
use zyn_vm::derive::Address;
use zyn_vm::object::{commitment, Note, ObjectError};
use zyn_vm::read::Decoder;
use zyn_vm::spec::AccountId;
use zyn_vm::Fixed;

use crate::{ObjectVm, Output, Receipt};

/// Fees are in basis points of what is sold into a pool, and are kept by it.
pub const BPS: i128 = 10_000;
/// A pool may charge at most 10%.
pub const MAX_FEE_BPS: u32 = 1_000;
/// Shares locked in every pool at creation and owned by nobody, so a pool can
/// never be emptied back to zero and its share price never reset (the
/// Uniswap v2 argument, at 1e-15 of a share).
pub const MINIMUM_LIQUIDITY: Fixed = Fixed(1_000);
/// Orders one pool will take in one epoch. Clearing refunds and recomputes,
/// so its cost grows with the batch; this bounds it.
pub const MAX_ORDERS_PER_POOL: usize = 256;

pub(crate) fn id(domain: &[u8], parts: &[&[u8]]) -> Hash {
    let mut e = Encoder::new();
    e.bytes(domain);
    for p in parts {
        e.bytes(p);
    }
    e.leaf()
}

/// A pool is named by its two assets, in byte order, so there is one per pair.
pub fn pool_id(a: &Address, b: &Address) -> Hash {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    id(b"objectvm.pool.v1", &[a, b])
}

/// The asset a pool's shares are denominated in.
pub fn lp_asset(pool: &Hash) -> Address {
    id(b"objectvm.lp.v1", &[pool])
}

/// The token a launch issues. Derived from its creator and nonce, so nobody
/// can launch into an asset that already exists.
pub fn token_of(creator: &AccountId, nonce: &Hash) -> Address {
    id(b"objectvm.token.v1", &[creator, nonce])
}

/// Where a launch creator's claims derive from. Not the token id, which is
/// public: a creator's claims would then be findable, and with them who the
/// creator is, by trying every registered account.
pub fn launch_source(creator: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.launch.v1", &[creator, nonce])
}

/// An order's id: the trader's nonce, bound to the trader.
pub fn order_id(trader: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.order.v1", &[trader, nonce])
}

/// A commitment to a sale.
pub fn commit_id(buyer: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.commit.v1", &[buyer, nonce])
}

/// The id of a liquidity action, which its claims derive from.
pub fn liquidity_id(provider: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.liquidity.v1", &[provider, nonce])
}

/// The `k`th claim a source produces. A wallet that knows the source's nonce
/// computes these without asking anyone which claims are its own.
///
/// | source            | 0            | 1              | 2        |
/// |-------------------|--------------|----------------|----------|
/// | order             | proceeds     | refund         |          |
/// | create/add liq.   | shares       | refund A       | refund B |
/// | remove liquidity  | A            | B              |          |
/// | commit            | tokens       | quote refund   |          |
/// | launch (creator)  | tokens       | quote proceeds |          |
///
/// A launch's source is [`launch_source`].
pub fn claim_id(source: &Hash, k: u8) -> Hash {
    id(b"objectvm.claim.v1", &[source, &[k]])
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pool {
    /// The lower asset in byte order.
    pub a: Address,
    pub b: Address,
    pub reserve_a: Fixed,
    pub reserve_b: Fixed,
    /// Every share in existence, including the locked ones.
    pub shares: Fixed,
    /// Shares nobody holds: the minimum at creation, or all of a launch's.
    pub locked: Fixed,
    pub fee_bps: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Order {
    pub id: Hash,
    pub owner: AccountId,
    pub pool: Hash,
    /// Selling A for B, or B for A.
    pub sell_a: bool,
    pub amount_in: Fixed,
    pub min_out: Fixed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Claim {
    pub owner: AccountId,
    pub asset: Address,
    pub amount: Fixed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LaunchState {
    Open,
    Graduated,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Launch {
    pub creator: AccountId,
    /// Where the creator's claims derive from: [`launch_source`].
    pub source: Hash,
    pub token: Address,
    pub quote: Address,
    pub total_supply: Fixed,
    pub sale_supply: Fixed,
    pub pool_supply: Fixed,
    pub soft_cap: Fixed,
    pub hard_cap: Fixed,
    /// Share of the raise that seeds the pool.
    pub pool_bps: u32,
    pub fee_bps: u32,
    /// The last epoch commitments are taken in. The sale settles at its seal.
    pub closes_at_epoch: u64,
    /// `H(name ‖ symbol ‖ image …)`, published by the creator off chain.
    pub meta: Hash,
    pub raised: Fixed,
    pub state: LaunchState,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Commitment {
    pub launch: Address,
    pub owner: AccountId,
    pub amount: Fixed,
}

/// What the chain charges, set by the operator with `SetFees`. Absent until
/// then, so a chain that never set it charges nothing and replays unchanged.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FeeSchedule {
    /// Charged on a deposit (taken from it) and on a withdrawal (on top).
    pub bridge_bps: u32,
}

/// The ceiling on the bridge fee: 2%.
pub const MAX_BRIDGE_BPS: u32 = 200;
/// The ceiling on a collection's resale fee, per side: 10%.
pub const MAX_COLLECTION_FEE_BPS: u32 = 1_000;

/// A collection's resale terms (DECISIONS §69.9, carried to the object
/// ledger). Each side of a sale pays `fee_bps` of the price in `quote` — the
/// buyer on top, the seller out of the proceeds, so the listed price is the
/// price — and the total splits 50% to the ZYN liquidity pot, 40% to this
/// collection's redemption floor and 10% to its creator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CollectionTerms {
    pub creator: AccountId,
    pub quote: Address,
    pub fee_bps: u32,
    /// Held for the collection's holders: any of them may burn items for
    /// `floor × items / outstanding`.
    pub floor: Fixed,
    /// The most items that will ever exist. Fixed at registration: minting
    /// past what holders were promised dilutes their share of the floor.
    pub cap: u32,
    pub minted: u32,
    /// Minted and not redeemed — what the floor is shared between.
    pub outstanding: u32,
}

/// One item is one unit of its own asset.
pub const ITEM_UNIT: Fixed = Fixed(zyn_vm::fixed::WAD);

/// An item's own asset: `item_address(collection, serial)`, the same address
/// the collection's manifest commits to (§86.2). Its own asset rather than a
/// unit of the collection's, so conservation carries *which* item moved
/// through every transfer and sale, not merely that one did.
pub fn item_asset(collection: &Address, serial: u32) -> Address {
    zyn_vm::item_address(zyn_vm::derive::ADDRESS_SCOPE_V1, collection, serial)
}

/// Which collection an item belongs to, and its serial.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ItemInfo {
    pub collection: Address,
    pub serial: u32,
}

/// What a listing adds to an offer: where the seller is paid, and the item's
/// nullifier, published so a buyer can retire it (§132.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Listing {
    pub source: Hash,
    pub nullifier: Hash,
}

/// Where a seller's proceeds are claimable: from a nonce only the seller
/// holds, so the listing names no seller and a stranger cannot find it.
pub fn listing_source(seller: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.listing.v1", &[seller, nonce])
}

/// The fee split of one resale: `(to ZYN pot, to floor, to creator)`, shares
/// rounded down and the remainder to the ZYN pot (§69.27's rule).
pub fn resale_split(taken: Fixed) -> (Fixed, Fixed, Fixed) {
    let to_floor = Fixed(taken.0 * 40 / 100);
    let to_creator = Fixed(taken.0 * 10 / 100);
    (Fixed(taken.0 - to_floor.0 - to_creator.0), to_floor, to_creator)
}

/// A fee of `bps` on `amount`, rounded down.
pub fn fee_of(amount: Fixed, bps: u32) -> Option<Fixed> {
    zyn_vm::fixed::mul_div(amount.0, bps as i128, BPS).map(Fixed)
}

/// Where the creator's share of one sale is claimable: derived from the item,
/// which the listing made public, so the creator can find it without asking.
pub fn sale_fee_source(item: &Hash) -> Hash {
    id(b"objectvm.sale-fee.v1", &[item])
}

/// Where a redemption's proceeds are claimable: derived from the first item
/// burned, which only its holder knew.
pub fn redemption_source(first_item: &Hash) -> Hash {
    id(b"objectvm.redeem.v1", &[first_item])
}

/// Everything the market holds between an intent and its settlement.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Market {
    pub pools: BTreeMap<Hash, Pool>,
    /// This epoch's orders, in arrival order. Arrival order changes nothing
    /// about the price; it is kept only so clearing is deterministic.
    pub orders: Vec<Order>,
    pub claims: BTreeMap<Hash, Claim>,
    /// Keyed by token.
    pub launches: BTreeMap<Address, Launch>,
    pub commits: BTreeMap<Hash, Commitment>,
    pub schedule: Option<FeeSchedule>,
    /// The ZYN liquidity pot: fees the protocol keeps, per asset. Keyless —
    /// no intent spends from it — and public, like every keyless pot (§71.23).
    pub pol: BTreeMap<Address, Fixed>,
    /// Collections with resale terms, keyed by the collection's id.
    pub collections: BTreeMap<Address, CollectionTerms>,
    /// Every item minted, by its asset.
    pub items: BTreeMap<Address, ItemInfo>,
    /// Listings made with `List`, by the listed item's commitment.
    pub listings: BTreeMap<Hash, Listing>,
    /// Bonding-curve tokens, by token (`crate::curve`).
    pub curves: BTreeMap<Address, crate::curve::Curve>,
    /// This epoch's curve orders.
    pub curve_orders: Vec<crate::curve::CurveOrder>,
    /// What curves charge; absent until the operator sets it.
    pub curve_terms: Option<crate::curve::CurveTerms>,
    /// The assets coins launch in and how curves in each are sized (§166);
    /// empty until the operator names one.
    pub curve_quotes: BTreeMap<Address, crate::curve::QuoteTerms>,
    /// This epoch's routes (§167), cleared at its seal.
    pub routes: Vec<crate::route::Route>,
    /// Primary sales, by collection (`crate::sale`).
    pub sales: BTreeMap<Address, crate::sale::Sale>,
    /// Their tickets, by an id only each ticket's buyer can derive.
    pub tickets: BTreeMap<Hash, crate::sale::Ticket>,
    /// Burns already turned into tickets (§145), by the burn's id, so none is
    /// posted twice.
    pub burns: BTreeMap<Hash, Address>,
    /// Gated sales' lists and per-account counts (§168), by collection.
    pub sale_gates: BTreeMap<Address, crate::sale::Gate>,
    /// Collections' DAOs (§172), by collection.
    pub daos: BTreeMap<Address, crate::dao::Dao>,
    /// The Zcash clock DAOs' strict windows run on (§173): height and hash.
    pub dao_clock: Option<(u32, Hash)>,
    /// Assets bridged from other chains beside ZEC, by address (§146).
    pub bridged: BTreeMap<Address, crate::bridge::Origin>,
    /// Committed deposits waiting for their depositor, by `deposit_key`.
    pub waiting: BTreeMap<Hash, crate::bridge::Waiting>,
    /// Zyn Lend: pools, prices, positions and liquidation auctions.
    pub lend: crate::lend::Lend,
}

impl Market {
    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
            && self.orders.is_empty()
            && self.claims.is_empty()
            && self.launches.is_empty()
            && self.commits.is_empty()
            && self.schedule.is_none()
            && self.pol.is_empty()
            && self.collections.is_empty()
            && self.items.is_empty()
            && self.listings.is_empty()
            && self.curves.is_empty()
            && self.curve_orders.is_empty()
            && self.curve_terms.is_none()
            && self.curve_quotes.is_empty()
            && self.routes.is_empty()
            && self.sale_gates.is_empty()
            && self.daos.is_empty()
            && self.dao_clock.is_none()
            && self.sales.is_empty()
            && self.tickets.is_empty()
            && self.burns.is_empty()
            && self.bridged.is_empty()
            && self.waiting.is_empty()
            && self.lend.is_empty()
    }

    /// The terms an item trades under, if it is a registered collection's.
    pub fn terms_for(&self, asset: &Address) -> Option<(Address, CollectionTerms)> {
        let i = self.items.get(asset)?;
        Some((i.collection, *self.collections.get(&i.collection)?))
    }

    pub fn bridge_bps(&self) -> u32 {
        self.schedule.map(|s| s.bridge_bps).unwrap_or(0)
    }

    pub(crate) fn to_pol(&mut self, asset: Address, amount: Fixed) -> Option<()> {
        if amount.0 > 0 {
            let p = self.pol.entry(asset).or_insert(Fixed::ZERO);
            p.0 = p.0.checked_add(amount.0)?;
        }
        Some(())
    }

    /// How much of `asset` the market is holding for somebody.
    ///
    /// Conservation is "live objects plus this equals issued". Leave any of it
    /// out and the check fails on the first trade; count any of it twice and
    /// it passes while value leaks.
    pub fn holdings(&self, asset: &Address) -> Option<i128> {
        let mut t: i128 = 0;
        for (pid, p) in &self.pools {
            if &p.a == asset {
                t = t.checked_add(p.reserve_a.0)?;
            }
            if &p.b == asset {
                t = t.checked_add(p.reserve_b.0)?;
            }
            if &lp_asset(pid) == asset {
                t = t.checked_add(p.locked.0)?;
            }
        }
        for o in &self.orders {
            let p = self.pools.get(&o.pool)?;
            let sold = if o.sell_a { p.a } else { p.b };
            if &sold == asset {
                t = t.checked_add(o.amount_in.0)?;
            }
        }
        for c in self.claims.values() {
            if &c.asset == asset {
                t = t.checked_add(c.amount.0)?;
            }
        }
        // Committed deposits nobody has claimed yet are issued and owed (§146).
        for w in self.waiting.values() {
            if &w.asset == asset {
                t = t.checked_add(w.amount.0)?;
            }
        }
        for l in self.launches.values() {
            if l.state == LaunchState::Open && &l.token == asset {
                t = t.checked_add(l.total_supply.0)?;
            }
        }
        for c in self.commits.values() {
            let l = self.launches.get(&c.launch)?;
            if &l.quote == asset {
                t = t.checked_add(c.amount.0)?;
            }
        }
        if let Some(p) = self.pol.get(asset) {
            t = t.checked_add(p.0)?;
        }
        for c in self.collections.values() {
            if &c.quote == asset {
                t = t.checked_add(c.floor.0)?;
            }
        }
        for c in self.curves.values() {
            if &c.quote == asset {
                t = t.checked_add(c.quote_held()?)?;
            }
        }
        for o in &self.curve_orders {
            let c = self.curves.get(&o.token)?;
            let escrowed = if o.buy { c.quote } else { c.token };
            if &escrowed == asset {
                t = t.checked_add(o.amount_in.0)?;
            }
        }
        t = t.checked_add(self.lend.holdings(asset)?)?;
        t = t.checked_add(crate::dao::holdings(&self.daos, asset)?)?;
        Some(t)
    }

    /// Assets the market made rather than received: pool shares, launched
    /// tokens and collection items. They exist only here, so none of them can leave for Zcash.
    pub fn is_derived(&self, asset: &Address) -> bool {
        self.launches.contains_key(asset)
            || self.curves.contains_key(asset)
            || self.items.contains_key(asset)
            || self.pools.keys().any(|p| &lp_asset(p) == asset)
            || self.lend.is_receipt(asset)
    }

    /// Owe `amount` under a claim id. An existing claim for the same owner and
    /// asset grows rather than being replaced: two wallets of one holder can
    /// reuse a nonce, and an overwrite would silently lose the first debt.
    pub(crate) fn pay(&mut self, source: &Hash, k: u8, owner: AccountId, asset: Address, amount: Fixed) {
        if amount.0 <= 0 {
            return;
        }
        let id = claim_id(source, k);
        match self.claims.get_mut(&id) {
            Some(c) if c.owner == owner && c.asset == asset => c.amount = Fixed(c.amount.0.saturating_add(amount.0)),
            Some(_) => {
                // A different owner or asset under one id cannot be derived by
                // honest wallets; keep the first debt and record nothing new
                // rather than overwrite it. Loud in tests: dropping a payment
                // here breaks conservation, as a duplicate route id did.
                debug_assert!(false, "a second claim under one id in another asset or owner was dropped");
            }
            None => {
                self.claims.insert(id, Claim { owner, asset, amount });
            }
        }
    }

    /// The market section of the state root.
    ///
    /// Pools and launches in the clear, because a market nobody can price is
    /// not one. Orders, claims and commitments as a Merkle root of blinded
    /// leaves: their ids carry the user's nonce, so the leaf cannot be
    /// recomputed from a guess at who and how much.
    pub fn leaf(&self) -> Hash {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.market.v1").u32(self.pools.len() as u32);
        for (pid, p) in &self.pools {
            e.bytes(pid)
                .bytes(&p.a)
                .bytes(&p.b)
                .fixed(p.reserve_a)
                .fixed(p.reserve_b)
                .fixed(p.shares)
                .fixed(p.locked)
                .u32(p.fee_bps);
        }
        e.u32(self.launches.len() as u32);
        for (t, l) in &self.launches {
            e.bytes(t)
                .bytes(&l.quote)
                .fixed(l.total_supply)
                .fixed(l.sale_supply)
                .fixed(l.pool_supply)
                .fixed(l.soft_cap)
                .fixed(l.hard_cap)
                .u32(l.pool_bps)
                .u32(l.fee_bps)
                .u64(l.closes_at_epoch)
                .bytes(&l.meta)
                .fixed(l.raised)
                .u8(l.state as u8);
        }
        let orders: Vec<Hash> = self.orders.iter().map(order_leaf).collect();
        let claims: Vec<Hash> = self.claims.iter().map(|(i, c)| claim_leaf(i, c)).collect();
        let commits: Vec<Hash> = self.commits.iter().map(|(i, c)| commit_leaf(i, c)).collect();
        // Each launch's creator is private too: committed with its source.
        let creators: Vec<Hash> = self
            .launches
            .iter()
            .map(|(t, l)| id(b"objectvm.creator.v1", &[t, &l.creator, &l.source]))
            .collect();
        e.bytes(&merkle_root(&orders))
            .bytes(&merkle_root(&claims))
            .bytes(&merkle_root(&commits))
            .bytes(&merkle_root(&creators));
        // Fees, the pot and collection terms: public, because what the chain
        // charges and what it keeps are things anyone should be able to check.
        e.u32(self.bridge_bps()).u32(self.pol.len() as u32);
        for (a, v) in &self.pol {
            e.bytes(a).fixed(*v);
        }
        e.u32(self.collections.len() as u32);
        for (a, c) in &self.collections {
            e.bytes(a)
                .bytes(&c.creator)
                .bytes(&c.quote)
                .u32(c.fee_bps)
                .fixed(c.floor)
                .u32(c.cap)
                .u32(c.minted)
                .u32(c.outstanding);
        }
        let items: Vec<Hash> = self
            .items
            .iter()
            .map(|(a, i)| id(b"objectvm.item-leaf.v1", &[a, &i.collection, &i.serial.to_be_bytes()]))
            .collect();
        let listings: Vec<Hash> = self
            .listings
            .iter()
            .map(|(c, l)| id(b"objectvm.listing-leaf.v1", &[c, &l.source, &l.nullifier]))
            .collect();
        // Joined only once there are any, so a market that set fees before
        // items existed keeps the roots it already sealed (testnet 212).
        if !items.is_empty() || !listings.is_empty() {
            e.bytes(&merkle_root(&items)).bytes(&merkle_root(&listings));
        }
        // Likewise curves: joined once there are any, or terms for them.
        if !self.curves.is_empty() || !self.curve_orders.is_empty() || self.curve_terms.is_some() || !self.curve_quotes.is_empty() {
            crate::curve::section(&mut e, self.curve_terms, &self.curve_quotes, &self.curves, &self.curve_orders);
        }
        // And sales: joined once one exists, so every root sealed before keeps.
        if !self.sales.is_empty() || !self.tickets.is_empty() || !self.burns.is_empty() {
            crate::sale::section(&mut e, &self.sales, &self.tickets, &self.burns);
        }
        // And bridged assets beside ZEC (§146): joined once one is registered.
        if !self.bridged.is_empty() || !self.waiting.is_empty() {
            let b: Vec<Hash> = self.bridged.iter().map(|(a, o)| crate::bridge::origin_leaf(a, o)).collect();
            let w: Vec<Hash> = self.waiting.iter().map(|(k, w)| crate::bridge::waiting_leaf(k, w)).collect();
            e.bytes(b"objectvm.bridged.v1")
                .u32(b.len() as u32)
                .bytes(&zyn_vm::commit::merkle_root(&b))
                .u32(w.len() as u32)
                .bytes(&zyn_vm::commit::merkle_root(&w));
        }
        // And lending: joined once an asset is listed, so older roots keep.
        if !self.lend.is_empty() {
            crate::lend::section(&mut e, &self.lend);
        }
        // And routes, only while one waits for its seal (§167).
        if !self.routes.is_empty() {
            crate::route::section(&mut e, &self.routes);
        }
        // And sale gates, once a sale is gated (§168).
        if !self.sale_gates.is_empty() {
            crate::sale::gates_section(&mut e, &self.sale_gates);
        }
        // And DAOs, once a collection has one (§172).
        if !self.daos.is_empty() {
            crate::dao::section(&mut e, &self.daos);
        }
        // And the Zcash clock, once started (§173).
        if let Some(c) = &self.dao_clock {
            crate::dao::clock_section(&mut e, c);
        }
        e.leaf()
    }

    pub fn encode(&self, e: &mut Encoder) {
        // MARKET10 only once lending rules v2 are on: until then it would
        // carry nothing MARKET09 does not, and a node rolled back to a binary
        // from before §160 can still read the state a newer one saved.
        // MARKET11 (§166) only once a quote asset is named or a curve is
        // sized for one; it carries MARKET10's lending state too.
        // MARKET15 (§173) once the Zcash clock has started: strict DAOs and
        // proposals carry their own terms. It carries MARKET14's tail too.
        // MARKET16 (§176) once a DAO has a Zyn track: its proposals carry the
        // team's vote. It carries MARKET15's tail too.
        let v16 = self.daos.values().any(|d| d.zyn.is_some());
        let v15 = v16 || self.dao_clock.is_some();
        let v14 = v15 || !self.daos.is_empty();
        let v13 = v14 || !self.sale_gates.is_empty();
        let v12 = v13 || !self.routes.is_empty();
        let v11 = v12 || !self.curve_quotes.is_empty() || self.curves.values().any(|c| c.unit != crate::curve::ZAT);
        let v10 = self.lend.v2() || v11;
        // MARKET12 (§167) only while a route waits: it carries MARKET11 too.
        // MARKET13 (§168) once a sale is gated: it carries MARKET12's tail too.
        // MARKET14 (§172) once a collection has a DAO: it carries MARKET13's too.
        e.bytes(if v16 { b"MARKET16" } else if v15 { b"MARKET15" } else if v14 { b"MARKET14" } else if v13 { b"MARKET13" } else if v12 { b"MARKET12" } else if v11 { b"MARKET11" } else if v10 { b"MARKET10" } else { b"MARKET09" }).u32(self.pools.len() as u32);
        for (pid, p) in &self.pools {
            e.bytes(pid)
                .bytes(&p.a)
                .bytes(&p.b)
                .fixed(p.reserve_a)
                .fixed(p.reserve_b)
                .fixed(p.shares)
                .fixed(p.locked)
                .u32(p.fee_bps);
        }
        e.u32(self.orders.len() as u32);
        for o in &self.orders {
            e.bytes(&o.id)
                .bytes(&o.owner)
                .bytes(&o.pool)
                .bool(o.sell_a)
                .fixed(o.amount_in)
                .fixed(o.min_out);
        }
        e.u32(self.claims.len() as u32);
        for (i, c) in &self.claims {
            e.bytes(i).bytes(&c.owner).bytes(&c.asset).fixed(c.amount);
        }
        e.u32(self.launches.len() as u32);
        for (t, l) in &self.launches {
            e.bytes(t)
                .bytes(&l.creator)
                .bytes(&l.source)
                .bytes(&l.quote)
                .fixed(l.total_supply)
                .fixed(l.sale_supply)
                .fixed(l.pool_supply)
                .fixed(l.soft_cap)
                .fixed(l.hard_cap)
                .u32(l.pool_bps)
                .u32(l.fee_bps)
                .u64(l.closes_at_epoch)
                .bytes(&l.meta)
                .fixed(l.raised)
                .u8(l.state as u8);
        }
        e.u32(self.commits.len() as u32);
        for (i, c) in &self.commits {
            e.bytes(i).bytes(&c.launch).bytes(&c.owner).fixed(c.amount);
        }
        e.bool(self.schedule.is_some()).u32(self.bridge_bps());
        e.u32(self.pol.len() as u32);
        for (a, v) in &self.pol {
            e.bytes(a).fixed(*v);
        }
        e.u32(self.collections.len() as u32);
        for (a, c) in &self.collections {
            e.bytes(a)
                .bytes(&c.creator)
                .bytes(&c.quote)
                .u32(c.fee_bps)
                .fixed(c.floor)
                .u32(c.cap)
                .u32(c.minted)
                .u32(c.outstanding);
        }
        e.u32(self.items.len() as u32);
        for (a, i) in &self.items {
            e.bytes(a).bytes(&i.collection).u32(i.serial);
        }
        e.u32(self.listings.len() as u32);
        for (c, l) in &self.listings {
            e.bytes(c).bytes(&l.source).bytes(&l.nullifier);
        }
        crate::curve::encode(e, self.curve_terms, &self.curves, &self.curve_orders);
        crate::sale::encode(e, &self.sales, &self.tickets, &self.burns);
        crate::bridge::encode(e, &self.bridged, &self.waiting);
        crate::lend::encode(e, &self.lend, v10);
        if v11 {
            crate::curve::encode_quotes(e, &self.curve_quotes, &self.curves);
        }
        if v12 {
            crate::route::encode(e, &self.routes);
        }
        if v13 {
            crate::sale::encode_gates(e, &self.sale_gates);
        }
        if v14 {
            crate::dao::encode(e, &self.daos, v15, v16);
        }
        if v15 {
            let (h, hash) = self.dao_clock.expect("v15 means started");
            e.u32(h).bytes(&hash);
        }
    }

    pub fn decode(d: &mut Decoder) -> Option<Market> {
        // v2 was saved by nodes that had set a fee schedule before items
        // existed (testnet 212, 23 Sep); it lacks the supply counters, items
        // and listings, and reads as a market that had none of them.
        // v3 predates curves and reads as a market with none.
        // v4 predates sales and reads as a market with none.
        // v5 predates burn tickets (§145): its sales have no cap and no burns.
        // v6 predates bridged assets beside ZEC (§146) and reads as none.
        // v7 predates lending (Zyn Lend) and reads as a market with none.
        // v8 predates guarded lending and reads as not guarded.
        // v9 predates lending rules v2 (§160) and reads as rules 0.
        // v10 predates quote assets (§166) and reads as none named.
        let tag = d.take_bytes(8).ok()?;
        let v16 = tag == b"MARKET16";
        let v15 = v16 || tag == b"MARKET15";
        let v14 = v15 || tag == b"MARKET14";
        let v13 = v14 || tag == b"MARKET13";
        let v12 = v13 || tag == b"MARKET12";
        let v11 = v12 || tag == b"MARKET11";
        let (v3, v4, v5, v6, v7, v8, v9, v10) = match tag {
            b"MARKET16" | b"MARKET15" | b"MARKET14" | b"MARKET13" | b"MARKET12" | b"MARKET11" | b"MARKET10" => (true, true, true, true, true, true, true, true),
            b"MARKET09" => (true, true, true, true, true, true, true, false),
            b"MARKET08" => (true, true, true, true, true, true, false, false),
            b"MARKET07" => (true, true, true, true, true, false, false, false),
            b"MARKET06" => (true, true, true, true, false, false, false, false),
            b"MARKET05" => (true, true, true, false, false, false, false, false),
            b"MARKET04" => (true, true, false, false, false, false, false, false),
            b"MARKET03" => (true, false, false, false, false, false, false, false),
            b"MARKET02" => (false, false, false, false, false, false, false, false),
            _ => return None,
        };
        let mut m = Market::default();
        for _ in 0..d.u32().ok()? {
            let pid = d.hash().ok()?;
            m.pools.insert(
                pid,
                Pool {
                    a: d.hash().ok()?,
                    b: d.hash().ok()?,
                    reserve_a: d.fixed().ok()?,
                    reserve_b: d.fixed().ok()?,
                    shares: d.fixed().ok()?,
                    locked: d.fixed().ok()?,
                    fee_bps: d.u32().ok()?,
                },
            );
        }
        for _ in 0..d.u32().ok()? {
            m.orders.push(Order {
                id: d.hash().ok()?,
                owner: d.account().ok()?,
                pool: d.hash().ok()?,
                sell_a: d.u8().ok()? == 1,
                amount_in: d.fixed().ok()?,
                min_out: d.fixed().ok()?,
            });
        }
        for _ in 0..d.u32().ok()? {
            let i = d.hash().ok()?;
            m.claims.insert(
                i,
                Claim {
                    owner: d.account().ok()?,
                    asset: d.hash().ok()?,
                    amount: d.fixed().ok()?,
                },
            );
        }
        for _ in 0..d.u32().ok()? {
            let t = d.hash().ok()?;
            let creator = d.account().ok()?;
            let source = d.hash().ok()?;
            let quote = d.hash().ok()?;
            let l = Launch {
                creator,
                source,
                token: t,
                quote,
                total_supply: d.fixed().ok()?,
                sale_supply: d.fixed().ok()?,
                pool_supply: d.fixed().ok()?,
                soft_cap: d.fixed().ok()?,
                hard_cap: d.fixed().ok()?,
                pool_bps: d.u32().ok()?,
                fee_bps: d.u32().ok()?,
                closes_at_epoch: d.u64().ok()?,
                meta: d.hash().ok()?,
                raised: d.fixed().ok()?,
                state: match d.u8().ok()? {
                    0 => LaunchState::Open,
                    1 => LaunchState::Graduated,
                    2 => LaunchState::Failed,
                    _ => return None,
                },
            };
            m.launches.insert(t, l);
        }
        for _ in 0..d.u32().ok()? {
            let i = d.hash().ok()?;
            m.commits.insert(
                i,
                Commitment {
                    launch: d.hash().ok()?,
                    owner: d.account().ok()?,
                    amount: d.fixed().ok()?,
                },
            );
        }
        let has = d.u8().ok()? == 1;
        let bridge_bps = d.u32().ok()?;
        m.schedule = has.then_some(FeeSchedule { bridge_bps });
        for _ in 0..d.u32().ok()? {
            m.pol.insert(d.hash().ok()?, d.fixed().ok()?);
        }
        for _ in 0..d.u32().ok()? {
            let a = d.hash().ok()?;
            m.collections.insert(
                a,
                CollectionTerms {
                    creator: d.account().ok()?,
                    quote: d.hash().ok()?,
                    fee_bps: d.u32().ok()?,
                    floor: d.fixed().ok()?,
                    cap: if v3 { d.u32().ok()? } else { 0 },
                    minted: if v3 { d.u32().ok()? } else { 0 },
                    outstanding: if v3 { d.u32().ok()? } else { 0 },
                },
            );
        }
        if v3 {
            for _ in 0..d.u32().ok()? {
                let a = d.hash().ok()?;
                m.items.insert(a, ItemInfo { collection: d.hash().ok()?, serial: d.u32().ok()? });
            }
            for _ in 0..d.u32().ok()? {
                let c = d.hash().ok()?;
                m.listings.insert(c, Listing { source: d.hash().ok()?, nullifier: d.hash().ok()? });
            }
        }
        if v4 {
            let (terms, curves, orders) = crate::curve::decode(d)?;
            m.curve_terms = terms;
            m.curves = curves;
            m.curve_orders = orders;
        }
        if v5 {
            let (sales, tickets, burns) = crate::sale::decode(d, v6)?;
            m.sales = sales;
            m.tickets = tickets;
            m.burns = burns;
        }
        if v7 {
            let (bridged, waiting) = crate::bridge::decode(d)?;
            m.bridged = bridged;
            m.waiting = waiting;
        }
        if v8 {
            m.lend = crate::lend::decode(d, v9, v10)?;
        }
        if v11 {
            m.curve_quotes = crate::curve::decode_quotes(d, &mut m.curves)?;
        }
        if v12 {
            m.routes = crate::route::decode(d)?;
        }
        if v13 {
            m.sale_gates = crate::sale::decode_gates(d)?;
        }
        if v14 {
            m.daos = crate::dao::decode(d, v15, v16)?;
        }
        if v15 {
            m.dao_clock = Some((d.u32().ok()?, d.hash().ok()?));
        }
        Some(m)
    }
}

fn order_leaf(o: &Order) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.order-leaf.v1")
        .bytes(&o.id)
        .bytes(&o.owner)
        .bytes(&o.pool)
        .bool(o.sell_a)
        .fixed(o.amount_in)
        .fixed(o.min_out);
    e.leaf()
}

fn claim_leaf(i: &Hash, c: &Claim) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.claim-leaf.v1")
        .bytes(i)
        .bytes(&c.owner)
        .bytes(&c.asset)
        .fixed(c.amount);
    e.leaf()
}

fn commit_leaf(i: &Hash, c: &Commitment) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.commit-leaf.v1")
        .bytes(i)
        .bytes(&c.launch)
        .bytes(&c.owner)
        .fixed(c.amount);
    e.leaf()
}

/// `floor(√(a·b))` in WAD, exactly reproducible: the product is taken to
/// WAD², rooted as an integer, and is therefore WAD again.
pub fn geometric_mean(a: Fixed, b: Fixed) -> Option<Fixed> {
    if a.0 <= 0 || b.0 <= 0 {
        return None;
    }
    let (hi, lo) = mul_u128(a.0 as u128, b.0 as u128);
    let r = isqrt_u256(hi, lo);
    (r <= i128::MAX as u128).then_some(Fixed(r as i128))
}

fn mul_u128(a: u128, b: u128) -> (u128, u128) {
    let (a1, a0) = (a >> 64, a & u64::MAX as u128);
    let (b1, b0) = (b >> 64, b & u64::MAX as u128);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 64) + (p01 & u64::MAX as u128) + (p10 & u64::MAX as u128);
    let lo = (p00 & u64::MAX as u128) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    (hi, lo)
}

/// Integer square root of a 256-bit value, by bisection on the result — slow
/// and obviously right, which is the trade a state root wants.
fn isqrt_u256(hi: u128, lo: u128) -> u128 {
    let le = |r: u128| -> bool {
        let (h, l) = mul_u128(r, r);
        h < hi || (h == hi && l <= lo)
    };
    let (mut lo_r, mut hi_r) = (0u128, u128::MAX);
    while lo_r < hi_r {
        let mid = lo_r + (hi_r - lo_r) / 2 + 1;
        if le(mid) {
            lo_r = mid;
        } else {
            hi_r = mid - 1;
        }
    }
    lo_r
}

/// What remains of `amount` once the pool's fee is taken.
fn net_of_fee(amount: Fixed, fee_bps: u32) -> Option<Fixed> {
    zyn_vm::fixed::mul_div(amount.0, BPS - fee_bps as i128, BPS).map(Fixed)
}

/// The outcome of clearing one pool's batch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Clearing {
    /// `(order index, amount out)` for every filled order.
    pub filled: Vec<(usize, Fixed)>,
    /// Orders refunded whole.
    pub refunded: Vec<usize>,
    pub reserve_a: Fixed,
    pub reserve_b: Fixed,
}

/// Clear one pool's orders at the FM-AMM batch price.
///
/// Pure, so it can be tested against the algebra without a ledger around it.
/// Outputs are floored — the pool keeps every rounding — and an order whose
/// output is below its minimum, or rounds to nothing, is refunded and the
/// price recomputed without it. Each pass removes at least one order, so this
/// ends. Any arithmetic overflow refunds the whole batch: a batch that cannot
/// be priced is not priced wrong.
pub fn clear(pool: &Pool, orders: &[Order]) -> Clearing {
    let refund_all = || Clearing {
        filled: Vec::new(),
        refunded: (0..orders.len()).collect(),
        reserve_a: pool.reserve_a,
        reserve_b: pool.reserve_b,
    };
    let mut nets: Vec<Fixed> = Vec::with_capacity(orders.len());
    for o in orders {
        let Some(net) = net_of_fee(o.amount_in, pool.fee_bps) else {
            return refund_all();
        };
        nets.push(net);
    }
    // Every active order's output at the price they make together, or None
    // if any misses its minimum or the arithmetic overflows.
    let outs_at = |active: &[bool]| -> Option<Vec<Option<Fixed>>> {
        let mut in_a: i128 = 0; // net of fee
        let mut in_b: i128 = 0;
        for (i, o) in orders.iter().enumerate() {
            if active[i] {
                let side = if o.sell_a { &mut in_a } else { &mut in_b };
                *side = side.checked_add(nets[i].0)?;
            }
        }
        let num = in_b.checked_mul(2)?.checked_add(pool.reserve_b.0)?;
        let den = in_a.checked_mul(2)?.checked_add(pool.reserve_a.0)?;
        let mut outs: Vec<Option<Fixed>> = vec![None; orders.len()];
        for (i, o) in orders.iter().enumerate() {
            if !active[i] {
                continue;
            }
            // A seller of A receives B at p = num/den; a seller of B receives
            // A at 1/p. Floored either way.
            let v = if o.sell_a { zyn_vm::fixed::mul_div(nets[i].0, num, den)? } else { zyn_vm::fixed::mul_div(nets[i].0, den, num)? };
            if v <= 0 || v < o.min_out.0 {
                return None;
            }
            outs[i] = Some(Fixed(v));
        }
        Some(outs)
    };
    let keys = orders.iter().map(|o| (asking_vs(o.min_out, o.amount_in, pool, o.sell_a), o.id)).collect();
    let active = admit_batch(orders.len(), keys, |a| outs_at(a).is_some());
    let Some(outs) = outs_at(&active) else {
        return refund_all();
    };
    let mut ra = pool.reserve_a.0;
    let mut rb = pool.reserve_b.0;
    let mut filled = Vec::new();
    let mut refunded = Vec::new();
    for (i, o) in orders.iter().enumerate() {
        match outs[i] {
            Some(out) => {
                let (sold, bought) = if o.sell_a { (&mut ra, &mut rb) } else { (&mut rb, &mut ra) };
                match (sold.checked_add(o.amount_in.0), bought.checked_sub(out.0)) {
                    (Some(s), Some(b)) => {
                        *sold = s;
                        *bought = b;
                    }
                    _ => return refund_all(),
                }
                filled.push((i, out));
            }
            None => refunded.push(i),
        }
    }
    // Proven positive in exact arithmetic and floors only raise it; this
    // is the belt to that brace.
    if ra <= 0 || rb <= 0 {
        return refund_all();
    }
    Clearing {
        filled,
        refunded,
        reserve_a: Fixed(ra),
        reserve_b: Fixed(rb),
    }
}

/// The batch as it cleared before `ObjectVm::fair_batches`: every order
/// that missed its minimum dropped together, per pass. Kept so the history
/// sealed under it replays exactly.
pub fn clear_legacy(pool: &Pool, orders: &[Order]) -> Clearing {
    let refund_all = || Clearing {
        filled: Vec::new(),
        refunded: (0..orders.len()).collect(),
        reserve_a: pool.reserve_a,
        reserve_b: pool.reserve_b,
    };
    let mut active: Vec<bool> = alloc_active(orders.len());
    loop {
        let mut in_a: i128 = 0; // net of fee
        let mut in_b: i128 = 0;
        let mut nets: Vec<Fixed> = Vec::with_capacity(orders.len());
        for (i, o) in orders.iter().enumerate() {
            let Some(net) = net_of_fee(o.amount_in, pool.fee_bps) else {
                return refund_all();
            };
            nets.push(net);
            if !active[i] {
                continue;
            }
            let side = if o.sell_a { &mut in_a } else { &mut in_b };
            *side = match side.checked_add(net.0) {
                Some(v) => v,
                None => return refund_all(),
            };
        }
        let Some(num) = in_b
            .checked_mul(2)
            .and_then(|v| v.checked_add(pool.reserve_b.0))
        else {
            return refund_all();
        };
        let Some(den) = in_a
            .checked_mul(2)
            .and_then(|v| v.checked_add(pool.reserve_a.0))
        else {
            return refund_all();
        };
        let mut outs: Vec<Option<Fixed>> = vec![None; orders.len()];
        let mut dropped = false;
        for (i, o) in orders.iter().enumerate() {
            if !active[i] {
                continue;
            }
            // A seller of A receives B at p = num/den; a seller of B receives
            // A at 1/p. Floored either way.
            let out = if o.sell_a {
                zyn_vm::fixed::mul_div(nets[i].0, num, den)
            } else {
                zyn_vm::fixed::mul_div(nets[i].0, den, num)
            };
            match out {
                Some(v) if v > 0 && v >= o.min_out.0 => outs[i] = Some(Fixed(v)),
                Some(_) => {
                    active[i] = false;
                    dropped = true;
                }
                None => return refund_all(),
            }
        }
        if dropped {
            continue;
        }
        let mut ra = pool.reserve_a.0;
        let mut rb = pool.reserve_b.0;
        let mut filled = Vec::new();
        let mut refunded = Vec::new();
        for (i, o) in orders.iter().enumerate() {
            match outs[i] {
                Some(out) => {
                    let (sold, bought) = if o.sell_a { (&mut ra, &mut rb) } else { (&mut rb, &mut ra) };
                    match (sold.checked_add(o.amount_in.0), bought.checked_sub(out.0)) {
                        (Some(s), Some(b)) => {
                            *sold = s;
                            *bought = b;
                        }
                        _ => return refund_all(),
                    }
                    filled.push((i, out));
                }
                None => refunded.push(i),
            }
        }
        // Proven positive in exact arithmetic and floors only raise it; this
        // is the belt to that brace.
        if ra <= 0 || rb <= 0 {
            return refund_all();
        }
        return Clearing {
            filled,
            refunded,
            reserve_a: Fixed(ra),
            reserve_b: Fixed(rb),
        };
    }
}

fn alloc_active(n: usize) -> Vec<bool> {
    vec![true; n]
}

/// What an order asks for each unit it puts in (WAD): the lower, the more
/// willing. `None` when it cannot be said; such an order comes last.
pub(crate) fn asking(min_out: Fixed, amount_in: Fixed) -> Option<i128> {
    if amount_in.0 <= 0 || min_out.0 < 0 {
        return None;
    }
    zyn_vm::fixed::mul_div(min_out.0, zyn_vm::fixed::WAD, amount_in.0)
}

/// The same, against a pool's own price, so both sides of a batch compare:
/// 1.0 asks exactly the pool's price, 0.95 accepts 5% less.
pub(crate) fn asking_vs(min_out: Fixed, amount_in: Fixed, pool: &Pool, sell_a: bool) -> Option<i128> {
    let a = asking(min_out, amount_in)?;
    let (have, get) = if sell_a { (pool.reserve_a.0, pool.reserve_b.0) } else { (pool.reserve_b.0, pool.reserve_a.0) };
    if get <= 0 {
        return None;
    }
    zyn_vm::fixed::mul_div(a, have, get)
}

/// Passes over the orders a batch turned away, for those that fit once
/// orders the other way are in.
const ADMIT_PASSES: usize = 3;

/// Which orders a batch keeps. If every order meets its minimum together,
/// all of them, as always. Otherwise they are admitted one at a time, most
/// willing first (the least asked; ties by id), and one is kept only if,
/// with it, every order kept still meets its minimum (`fits`). An order
/// that can never be met is turned away on its own: it cannot take honest
/// orders out of the batch with it (audit, 9 Oct 2026 — dropping every
/// unmet order together let a decoy with an impossible floor push others
/// past theirs, leave with them, and hand its own real order the cheaper
/// price, for nothing). A further pass retries what was turned away, for an
/// order that fits only once one the other way is in.
pub(crate) fn admit_batch(n: usize, keys: Vec<(Option<i128>, Hash)>, mut fits: impl FnMut(&[bool]) -> bool) -> Vec<bool> {
    let all = vec![true; n];
    if n == 0 || fits(&all) {
        return all;
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&x, &y| {
        let by = match (keys[x].0, keys[y].0) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        by.then(keys[x].1.cmp(&keys[y].1))
    });
    let mut active = vec![false; n];
    for _ in 0..ADMIT_PASSES {
        let mut added = false;
        for &i in &order {
            if active[i] {
                continue;
            }
            active[i] = true;
            if fits(&active) {
                added = true;
            } else {
                active[i] = false;
            }
        }
        if !added {
            break;
        }
    }
    active
}

/// When a batch is full: the least willing order in it, if a new order
/// asking `new` is more willing (lower), to make room — it goes back to its
/// owner whole. Otherwise none, and the new order is refused. Orders that
/// can fill (a route's legs ask nothing) are never the ones displaced, so a
/// batch filled with floors that can never be met no longer shuts a venue
/// for the epoch (audit, 9 Oct 2026).
pub(crate) fn displace(asked: impl Iterator<Item = (usize, Option<i128>, Hash)>, new: Option<i128>) -> Option<usize> {
    let new = new?;
    let worst = asked.max_by(|x, y| {
        let by = match (x.1, y.1) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        by.then(x.2.cmp(&y.2))
    })?;
    match worst.1 {
        Some(w) if w <= new => None,
        _ => Some(worst.0),
    }
}

/// Consume `inputs` owned by `who` and create `change`, leaving exactly
/// `owed` behind per asset. Checks everything and mutates nothing; the caller
/// applies the returned plan once every other check has passed too.
pub(crate) struct Funding {
    pub created: Vec<(Hash, Note)>,
}

impl ObjectVm {
    pub(crate) fn plan_funding(
        &self,
        who: &AccountId,
        inputs: &[(Hash, Hash)],
        change: &[Output],
        owed: &[(Address, Fixed)],
    ) -> Result<Funding, &'static str> {
        if inputs.is_empty() {
            return Err("nothing funds it");
        }
        if inputs.len() as u32 > self.limits.max_io || change.len() as u32 > self.limits.max_io {
            return Err("too many objects in one intent");
        }
        let mut delta: BTreeMap<Address, i128> = BTreeMap::new();
        let mut seen: Vec<Hash> = Vec::with_capacity(inputs.len());
        for (c, n) in inputs {
            if seen.contains(c) {
                return Err("the same object twice in one intent");
            }
            seen.push(*c);
            let note = self.live.get(c).ok_or("no such live object")?;
            if &note.owner != who {
                return Err("not the signer's object");
            }
            if self.offers.contains_key(c) {
                return Err("that object is listed");
            }
            if self.nullifiers.contains(n) {
                return Err("already spent");
            }
            let e = delta.entry(note.asset).or_insert(0);
            *e = e.checked_add(note.amount.0).ok_or("input overflow")?;
        }
        let mut created = Vec::with_capacity(change.len());
        for out in change {
            if out.enc.ciphertext.is_empty() {
                return Err("an output needs an encrypted note");
            }
            let c = match commitment(&out.note) {
                Ok(c) => c,
                Err(ObjectError::ZeroNonce) => return Err("an output needs a nonce"),
                Err(ObjectError::NotPositive) => return Err("an output must be worth something"),
            };
            if self.live.contains_key(&c) || created.iter().any(|(x, _)| x == &c) {
                return Err("output already exists");
            }
            let e = delta.entry(out.note.asset).or_insert(0);
            *e = e.checked_sub(out.note.amount.0).ok_or("output overflow")?;
            created.push((c, out.note));
        }
        for (asset, amount) in owed {
            if amount.0 <= 0 {
                return Err("an amount must be above nothing");
            }
            let e = delta.entry(*asset).or_insert(0);
            *e = e.checked_sub(amount.0).ok_or("overflow")?;
        }
        if delta.values().any(|d| *d != 0) {
            return Err("value not conserved");
        }
        Ok(Funding { created })
    }

    pub(crate) fn apply_funding(&mut self, who: &AccountId, inputs: &[(Hash, Hash)], f: Funding) {
        for (c, n) in inputs {
            self.live.remove(c);
            self.nullifiers.insert(*n);
        }
        for (c, note) in f.created {
            self.commitments.append(c);
            self.live.insert(c, note);
        }
        *self.accounts.entry(*who).or_insert(0) += 1;
    }

    /// A claim id not already in use, or a refusal.
    pub(crate) fn fresh_source(&self, source: &Hash) -> Result<(), &'static str> {
        if (0..3).any(|k| self.market.claims.contains_key(&claim_id(source, k)))
            || self.market.orders.iter().any(|o| &o.id == source)
            || self.market.curve_orders.iter().any(|o| &o.id == source)
            || self.market.curves.values().any(|c| &c.source == source)
            || self.market.commits.contains_key(source)
            // A route waiting for its seal (§167): two routes under one id
            // would share their legs and one would vanish (audit, 6 Oct 2026).
            || self.market.routes.iter().any(|r| &r.id == source)
            || crate::lend::source_in_use(&self.market.lend, source)
        {
            return Err("that nonce was already used");
        }
        Ok(())
    }

    pub(crate) fn issue(&mut self, asset: Address, amount: Fixed) -> Result<(), &'static str> {
        let s = self.issued.entry(asset).or_insert(Fixed::ZERO);
        s.0 = s.0.checked_add(amount.0).ok_or("supply overflow")?;
        Ok(())
    }

    pub(crate) fn burn(&mut self, asset: Address, amount: Fixed) -> Result<(), &'static str> {
        let s = self.issued.entry(asset).or_insert(Fixed::ZERO);
        let v = s.0.checked_sub(amount.0).filter(|v| *v >= 0).ok_or("burn exceeds supply")?;
        s.0 = v;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_create_pool(
        &mut self,
        provider: &AccountId,
        asset_a: &Address,
        asset_b: &Address,
        amount_a: Fixed,
        amount_b: Fixed,
        fee_bps: u32,
        inputs: &[(Hash, Hash)],
        change: &[Output],
        nonce: &Hash,
    ) -> Result<Receipt, &'static str> {
        if asset_a >= asset_b {
            return Err("a pool names its assets in byte order");
        }
        if fee_bps > MAX_FEE_BPS {
            return Err("the fee is above the ceiling");
        }
        let is_share = |x: &Address| self.market.pools.keys().any(|p| &lp_asset(p) == x);
        if is_share(asset_a) || is_share(asset_b) {
            return Err("pool shares cannot be pooled");
        }
        if self.market.launches.get(asset_a).is_some_and(|l| l.state != LaunchState::Graduated)
            || self.market.launches.get(asset_b).is_some_and(|l| l.state != LaunchState::Graduated)
        {
            return Err("that token has not launched yet");
        }
        // A curve token's pool is the one it graduates into. Opening it early
        // would take the pool graduation needs, and the curve could then
        // never graduate.
        if self.market.curves.get(asset_a).is_some_and(|c| !c.graduated)
            || self.market.curves.get(asset_b).is_some_and(|c| !c.graduated)
        {
            return Err("that token is still on its curve");
        }
        let pid = pool_id(asset_a, asset_b);
        if self.market.pools.contains_key(&pid) {
            return Err("that pool exists");
        }
        let source = liquidity_id(provider, nonce);
        self.fresh_source(&source)?;
        let shares = geometric_mean(amount_a, amount_b).ok_or("amounts must be above nothing")?;
        if shares <= MINIMUM_LIQUIDITY {
            return Err("too little liquidity to open a pool");
        }
        let f = self.plan_funding(provider, inputs, change, &[(*asset_a, amount_a), (*asset_b, amount_b)])?;
        self.issue(lp_asset(&pid), shares)?;
        self.apply_funding(provider, inputs, f);
        self.market.pools.insert(
            pid,
            Pool {
                a: *asset_a,
                b: *asset_b,
                reserve_a: amount_a,
                reserve_b: amount_b,
                shares,
                locked: MINIMUM_LIQUIDITY,
                fee_bps,
            },
        );
        let mine = Fixed(shares.0 - MINIMUM_LIQUIDITY.0);
        self.market.pay(&source, 0, *provider, lp_asset(&pid), mine);
        Ok(Receipt::Pooled { pool: pid, shares: mine, source })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_add_liquidity(
        &mut self,
        provider: &AccountId,
        pool: &Hash,
        amount_a: Fixed,
        amount_b: Fixed,
        min_shares: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[Output],
        nonce: &Hash,
    ) -> Result<Receipt, &'static str> {
        let p = *self.market.pools.get(pool).ok_or("no such pool")?;
        let source = liquidity_id(provider, nonce);
        self.fresh_source(&source)?;
        if amount_a.0 <= 0 || amount_b.0 <= 0 {
            return Err("amounts must be above nothing");
        }
        let sa = amount_a.mul_div(p.shares, p.reserve_a).ok_or("overflow")?;
        let sb = amount_b.mul_div(p.shares, p.reserve_b).ok_or("overflow")?;
        let shares = sa.min(sb);
        if shares.0 <= 0 {
            return Err("too little to earn a share");
        }
        if shares < min_shares {
            return Err("fewer shares than the minimum asked");
        }
        // What the shares cost, rounded up: the pool is never shortchanged.
        let used_a = shares.mul_div_ceil(p.reserve_a, p.shares).ok_or("overflow")?;
        let used_b = shares.mul_div_ceil(p.reserve_b, p.shares).ok_or("overflow")?;
        if used_a > amount_a || used_b > amount_b {
            return Err("rounding exceeded the amounts offered");
        }
        let f = self.plan_funding(provider, inputs, change, &[(p.a, amount_a), (p.b, amount_b)])?;
        let new = Pool {
            reserve_a: Fixed(p.reserve_a.0.checked_add(used_a.0).ok_or("overflow")?),
            reserve_b: Fixed(p.reserve_b.0.checked_add(used_b.0).ok_or("overflow")?),
            shares: Fixed(p.shares.0.checked_add(shares.0).ok_or("overflow")?),
            ..p
        };
        self.issue(lp_asset(pool), shares)?;
        self.apply_funding(provider, inputs, f);
        self.market.pools.insert(*pool, new);
        self.market.pay(&source, 0, *provider, lp_asset(pool), shares);
        self.market.pay(&source, 1, *provider, p.a, Fixed(amount_a.0 - used_a.0));
        self.market.pay(&source, 2, *provider, p.b, Fixed(amount_b.0 - used_b.0));
        Ok(Receipt::Pooled { pool: *pool, shares, source })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_remove_liquidity(
        &mut self,
        provider: &AccountId,
        pool: &Hash,
        shares: Fixed,
        min_a: Fixed,
        min_b: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[Output],
        nonce: &Hash,
    ) -> Result<Receipt, &'static str> {
        let p = *self.market.pools.get(pool).ok_or("no such pool")?;
        let source = liquidity_id(provider, nonce);
        self.fresh_source(&source)?;
        if shares.0 <= 0 {
            return Err("an amount must be above nothing");
        }
        // Unreachable for anyone not holding locked shares, which is nobody.
        if p.shares.0 - shares.0 < p.locked.0 {
            return Err("the locked shares stay");
        }
        let out_a = shares.mul_div(p.reserve_a, p.shares).ok_or("overflow")?;
        let out_b = shares.mul_div(p.reserve_b, p.shares).ok_or("overflow")?;
        if out_a < min_a || out_b < min_b {
            return Err("less out than the minimum asked");
        }
        let f = self.plan_funding(provider, inputs, change, &[(lp_asset(pool), shares)])?;
        let new = Pool {
            reserve_a: Fixed(p.reserve_a.0 - out_a.0),
            reserve_b: Fixed(p.reserve_b.0 - out_b.0),
            shares: Fixed(p.shares.0 - shares.0),
            ..p
        };
        if new.reserve_a.0 <= 0 || new.reserve_b.0 <= 0 {
            return Err("a pool cannot be emptied");
        }
        self.check_depth_after(&p.a, &p.b, new.reserve_a.0, new.reserve_b.0)?;
        self.burn(lp_asset(pool), shares)?;
        self.apply_funding(provider, inputs, f);
        self.market.pools.insert(*pool, new);
        self.market.pay(&source, 0, *provider, p.a, out_a);
        self.market.pay(&source, 1, *provider, p.b, out_b);
        Ok(Receipt::Unpooled { pool: *pool, source })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_swap(
        &mut self,
        trader: &AccountId,
        pool: &Hash,
        sell_a: bool,
        amount_in: Fixed,
        min_out: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[Output],
        nonce: &Hash,
    ) -> Result<Receipt, &'static str> {
        let p = *self.market.pools.get(pool).ok_or("no such pool")?;
        if min_out.0 < 0 {
            return Err("a minimum below nothing");
        }
        let oid = order_id(trader, nonce);
        self.fresh_source(&oid)?;
        let sold = if sell_a { p.a } else { p.b };
        let mut out_of_batch = None;
        if self.market.orders.iter().filter(|o| &o.pool == pool).count() >= MAX_ORDERS_PER_POOL {
            if !self.fair_batches() {
                return Err("this pool's batch is full; try next epoch");
            }
            let asked = self.market.orders.iter().enumerate().filter(|(_, o)| &o.pool == pool).map(|(i, o)| (i, asking_vs(o.min_out, o.amount_in, &p, o.sell_a), o.id));
            out_of_batch = Some(displace(asked, asking_vs(min_out, amount_in, &p, sell_a)).ok_or("this pool's batch is full; try next epoch")?);
        }
        let f = self.plan_funding(trader, inputs, change, &[(sold, amount_in)])?;
        self.apply_funding(trader, inputs, f);
        if let Some(i) = out_of_batch {
            let o = self.market.orders.remove(i);
            let back = if o.sell_a { p.a } else { p.b };
            self.market.pay(&o.id, 1, o.owner, back, o.amount_in);
        }
        self.market.orders.push(Order {
            id: oid,
            owner: *trader,
            pool: *pool,
            sell_a,
            amount_in,
            min_out,
        });
        Ok(Receipt::Ordered { order: oid })
    }

    pub(crate) fn apply_claim(
        &mut self,
        owner: &AccountId,
        claims: &[Hash],
        outputs: &[Output],
    ) -> Result<Receipt, &'static str> {
        if claims.is_empty() || outputs.is_empty() {
            return Err("a claim redeems something into something");
        }
        if claims.len() as u32 > self.limits.max_io || outputs.len() as u32 > self.limits.max_io {
            return Err("too many objects in one claim");
        }
        let mut delta: BTreeMap<Address, i128> = BTreeMap::new();
        let mut seen: Vec<Hash> = Vec::new();
        for i in claims {
            if seen.contains(i) {
                return Err("the same claim twice");
            }
            seen.push(*i);
            let c = self.market.claims.get(i).ok_or("no such claim")?;
            if &c.owner != owner {
                return Err("not the signer's claim");
            }
            let e = delta.entry(c.asset).or_insert(0);
            *e = e.checked_add(c.amount.0).ok_or("overflow")?;
        }
        let mut created: Vec<(Hash, Note)> = Vec::new();
        for out in outputs {
            if out.enc.ciphertext.is_empty() {
                return Err("an output needs an encrypted note");
            }
            let c = commitment(&out.note).map_err(|_| "an output needs a nonce and a value")?;
            if self.live.contains_key(&c) || created.iter().any(|(x, _)| x == &c) {
                return Err("output already exists");
            }
            let e = delta.entry(out.note.asset).or_insert(0);
            *e = e.checked_sub(out.note.amount.0).ok_or("overflow")?;
            created.push((c, out.note));
        }
        if delta.values().any(|d| *d != 0) {
            return Err("value not conserved");
        }
        for i in claims {
            self.market.claims.remove(i);
        }
        let mut made = Vec::new();
        for (c, note) in created {
            self.commitments.append(c);
            self.live.insert(c, note);
            made.push(c);
        }
        *self.accounts.entry(*owner).or_insert(0) += 1;
        Ok(Receipt::Claimed { created: made })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_create_launch(
        &mut self,
        creator: &AccountId,
        nonce: &Hash,
        quote: &Address,
        total_supply: Fixed,
        sale_supply: Fixed,
        pool_supply: Fixed,
        soft_cap: Fixed,
        hard_cap: Fixed,
        pool_bps: u32,
        fee_bps: u32,
        closes_at_epoch: u64,
        meta: &Hash,
    ) -> Result<Receipt, &'static str> {
        let token = token_of(creator, nonce);
        // A curve's token is not issued until its first lot sells, so the
        // issued check alone let a curve's creator open a launch for the same
        // token and mint it outside the curve (audit, 6 Oct 2026).
        if self.market.launches.contains_key(&token) || self.issued.contains_key(&token) || self.market.curves.contains_key(&token) {
            return Err("that token exists");
        }
        let source = launch_source(creator, nonce);
        self.fresh_source(&source)?;
        if total_supply.0 <= 0 || sale_supply.0 <= 0 || pool_supply.0 < 0 {
            return Err("supplies must be above nothing");
        }
        if sale_supply.0.checked_add(pool_supply.0).map_or(true, |v| v > total_supply.0) {
            return Err("the sale and the pool exceed the supply");
        }
        if hard_cap.0 <= 0 || soft_cap.0 < 0 || soft_cap > hard_cap {
            return Err("caps must satisfy 0 ≤ soft ≤ hard, hard above nothing");
        }
        if pool_bps as i128 > BPS || fee_bps > MAX_FEE_BPS {
            return Err("a share above the whole");
        }
        if (pool_bps > 0) != (pool_supply.0 > 0) {
            return Err("a pool needs both tokens and part of the raise");
        }
        if closes_at_epoch < self.epoch {
            return Err("that sale would already be closed");
        }
        if !self.issued.contains_key(quote) || self.market.is_derived(quote) {
            return Err("a sale is priced in an asset people can deposit");
        }
        self.issue(token, total_supply)?;
        self.market.launches.insert(
            token,
            Launch {
                creator: *creator,
                source,
                token,
                quote: *quote,
                total_supply,
                sale_supply,
                pool_supply,
                soft_cap,
                hard_cap,
                pool_bps,
                fee_bps,
                closes_at_epoch,
                meta: *meta,
                raised: Fixed::ZERO,
                state: LaunchState::Open,
            },
        );
        *self.accounts.entry(*creator).or_insert(0) += 1;
        Ok(Receipt::Launched { token })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_commit(
        &mut self,
        buyer: &AccountId,
        launch: &Address,
        amount: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[Output],
        nonce: &Hash,
    ) -> Result<Receipt, &'static str> {
        let l = *self.market.launches.get(launch).ok_or("no such sale")?;
        if l.state != LaunchState::Open || self.epoch > l.closes_at_epoch {
            return Err("that sale is closed");
        }
        let cid = commit_id(buyer, nonce);
        self.fresh_source(&cid)?;
        let f = self.plan_funding(buyer, inputs, change, &[(l.quote, amount)])?;
        let raised = Fixed(l.raised.0.checked_add(amount.0).ok_or("overflow")?);
        self.apply_funding(buyer, inputs, f);
        self.market.commits.insert(
            cid,
            Commitment {
                launch: *launch,
                owner: *buyer,
                amount,
            },
        );
        self.market.launches.insert(*launch, Launch { raised, ..l });
        Ok(Receipt::Committed { commit: cid })
    }

    /// Burn items for their share of the collection's redemption floor:
    /// `floor × burned / outstanding`, rounded down, as a claim.
    pub(crate) fn apply_redeem(
        &mut self,
        holder: &AccountId,
        collection: &Address,
        inputs: &[(Hash, Hash)],
    ) -> Result<Receipt, &'static str> {
        let terms = *self.market.collections.get(collection).ok_or("that is not a collection with a floor")?;
        if inputs.is_empty() || inputs.len() as u32 > self.limits.max_io {
            return Err("redeem between one and max_io items");
        }
        let mut burned: Vec<(Hash, Hash, Address)> = Vec::new();
        for (c, n) in inputs {
            if burned.iter().any(|(x, _, _)| x == c) {
                return Err("the same item twice");
            }
            let note = self.live.get(c).ok_or("no such live object")?;
            if &note.owner != holder {
                return Err("not the holder's item");
            }
            if self.market.items.get(&note.asset).map(|i| &i.collection) != Some(collection) {
                return Err("not an item of that collection");
            }
            if note.amount != ITEM_UNIT {
                return Err("an item is one whole unit");
            }
            if self.offers.contains_key(c) {
                return Err("that object is listed");
            }
            if self.nullifiers.contains(n) {
                return Err("already spent");
            }
            burned.push((*c, *n, note.asset));
        }
        let k = burned.len() as u32;
        if terms.outstanding == 0 || k > terms.outstanding {
            return Err("the collection's supply does not cover that");
        }
        let payout = zyn_vm::fixed::mul_div(terms.floor.0, k as i128, terms.outstanding as i128).ok_or("overflow")?;
        for (c, n, asset) in &burned {
            self.burn(*asset, ITEM_UNIT)?;
            self.live.remove(c);
            self.nullifiers.insert(*n);
        }
        *self.accounts.entry(*holder).or_insert(0) += 1;
        self.market.collections.insert(
            *collection,
            CollectionTerms { floor: Fixed(terms.floor.0 - payout), outstanding: terms.outstanding - k, ..terms },
        );
        let source = redemption_source(&inputs[0].0);
        self.market.pay(&source, 0, *holder, terms.quote, Fixed(payout));
        Ok(Receipt::Redeemed { paid: Fixed(payout), source })
    }

    /// Mint one item of a registered collection to `owner`, as a note they
    /// can find: the primary sale's delivery.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_mint_item(
        &mut self,
        collection: &Address,
        serial: u32,
        owner: &AccountId,
        nonce: &Hash,
        enc: &zyn_vm::note_enc::EncryptedNote,
    ) -> Result<Receipt, &'static str> {
        let terms = *self.market.collections.get(collection).ok_or("register the collection first")?;
        if serial >= terms.cap {
            return Err("that serial is beyond the collection's cap");
        }
        let asset = item_asset(collection, serial);
        if self.market.items.contains_key(&asset) || self.issued.contains_key(&asset) {
            return Err("that item was already minted");
        }
        if enc.ciphertext.is_empty() {
            return Err("an item needs an encrypted note, or its owner cannot find it");
        }
        let note = Note { asset, amount: ITEM_UNIT, owner: *owner, nonce: *nonce };
        let c = commitment(&note).map_err(|_| "an item needs a nonce")?;
        if self.live.contains_key(&c) {
            return Err("that object already exists");
        }
        self.issue(asset, ITEM_UNIT)?;
        let position = self.commitments.append(c) as u64;
        self.live.insert(c, note);
        self.accounts.entry(*owner).or_insert(0);
        self.market.items.insert(asset, ItemInfo { collection: *collection, serial });
        self.market.collections.insert(
            *collection,
            CollectionTerms { minted: terms.minted + 1, outstanding: terms.outstanding + 1, ..terms },
        );
        Ok(Receipt::Minted { commitment: c, position })
    }

    /// Everything that happens at a seal, before the root is taken: every
    /// pool's batch clears, and every sale whose last epoch this is settles.
    pub(crate) fn settle_market(&mut self) {
        if self.market.is_empty() {
            return;
        }
        if !self.market.routes.is_empty() {
            // Routes waiting (§167): every venue clears once, a route's later
            // hops after its earlier ones, all or nothing per route.
            self.settle_routed();
        } else {
            // Curves first: a curve that graduates this epoch opens its pool
            // here, and its pool's first swaps are next epoch's.
            self.settle_curves();
            // Lending before the pools clear: its auctions and liquidations are
            // priced from the posted prices, not from this batch.
            self.settle_lend();
            let orders = std::mem::take(&mut self.market.orders);
            let pools: Vec<Hash> = self.market.pools.keys().copied().collect();
            for pid in pools {
                let batch: Vec<Order> = orders.iter().filter(|o| o.pool == pid).copied().collect();
                if batch.is_empty() {
                    continue;
                }
                self.settle_pool(pid, &batch);
            }
        }
        // Lending's backstop orders were in these batches; their results go
        // back to their lots before the root is taken.
        self.sweep_backstops();
        let due: Vec<Address> = self
            .market
            .launches
            .iter()
            .filter(|(_, l)| l.state == LaunchState::Open && l.closes_at_epoch <= self.epoch)
            .map(|(t, _)| *t)
            .collect();
        for t in due {
            self.close_launch(&t);
        }
        // DAOs last (§172): backing lapses and votes close on the epoch's end.
        if !self.market.daos.is_empty() {
            self.settle_daos();
        }
    }

    /// One pool's batch, at one price.
    pub(crate) fn settle_pool(&mut self, pid: Hash, batch: &[Order]) {
        let p = self.market.pools[&pid];
        let c = if self.fair_batches() { clear(&p, batch) } else { clear_legacy(&p, batch) };
        for (i, out) in &c.filled {
            let o = &batch[*i];
            let got = if o.sell_a { p.b } else { p.a };
            self.market.pay(&o.id, 0, o.owner, got, *out);
        }
        for i in &c.refunded {
            let o = &batch[*i];
            let sold = if o.sell_a { p.a } else { p.b };
            self.market.pay(&o.id, 1, o.owner, sold, o.amount_in);
        }
        self.market.pools.insert(
            pid,
            Pool {
                reserve_a: c.reserve_a,
                reserve_b: c.reserve_b,
                ..p
            },
        );
    }

    fn close_launch(&mut self, token: &Address) {
        let l = self.market.launches[token];
        let commits: Vec<(Hash, Commitment)> = self
            .market
            .commits
            .iter()
            .filter(|(_, c)| &c.launch == token)
            .map(|(i, c)| (*i, *c))
            .collect();
        for (i, _) in &commits {
            self.market.commits.remove(i);
        }
        let fail = |vm: &mut ObjectVm| {
            for (i, c) in &commits {
                vm.market.pay(i, 1, c.owner, l.quote, c.amount);
            }
            // Burned whole: a sale that did not happen issued nothing.
            let _ = vm.burn(l.token, l.total_supply);
            vm.market.launches.insert(l.token, Launch { state: LaunchState::Failed, ..l });
        };
        if l.raised.0 <= 0 || l.raised < l.soft_cap {
            fail(self);
            return;
        }
        // One price for everyone: each commitment is taken in the proportion
        // the hard cap allows and buys tokens in the same proportion.
        let accepted = l.raised.min(l.hard_cap);
        let mut taken: i128 = 0;
        let mut sold: i128 = 0;
        let mut plan = Vec::with_capacity(commits.len());
        for (i, c) in &commits {
            let (Some(take), Some(tokens)) = (
                zyn_vm::fixed::mul_div(c.amount.0, accepted.0, l.raised.0),
                zyn_vm::fixed::mul_div(c.amount.0, l.sale_supply.0, l.raised.0),
            ) else {
                fail(self);
                return;
            };
            taken += take;
            sold += tokens;
            plan.push((*i, c.owner, tokens, c.amount.0 - take));
        }
        let to_pool_quote = zyn_vm::fixed::mul_div(taken, l.pool_bps as i128, BPS).unwrap_or(0);
        let pid = pool_id(&l.token, &l.quote);
        let seeds_pool = to_pool_quote > 0 && l.pool_supply.0 > 0 && !self.market.pools.contains_key(&pid);
        let shares = if seeds_pool {
            let (ra, rb) = if l.token < l.quote {
                (l.pool_supply, Fixed(to_pool_quote))
            } else {
                (Fixed(to_pool_quote), l.pool_supply)
            };
            geometric_mean(ra, rb).filter(|s| *s > MINIMUM_LIQUIDITY)
        } else {
            None
        };
        for (i, owner, tokens, refund) in plan {
            self.market.pay(&i, 0, owner, l.token, Fixed(tokens));
            self.market.pay(&i, 1, owner, l.quote, Fixed(refund));
        }
        // The creator: their allocation, the token dust of rounding, and the
        // raise less the pool's part. Without a pool, the pool's part too.
        let mut creator_tokens = l.total_supply.0 - sold;
        let mut creator_quote = taken;
        if let Some(shares) = shares {
            creator_tokens -= l.pool_supply.0;
            creator_quote -= to_pool_quote;
            let (a, b, ra, rb) = if l.token < l.quote {
                (l.token, l.quote, l.pool_supply, Fixed(to_pool_quote))
            } else {
                (l.quote, l.token, Fixed(to_pool_quote), l.pool_supply)
            };
            let _ = self.issue(lp_asset(&pid), shares);
            self.market.pools.insert(
                pid,
                Pool {
                    a,
                    b,
                    reserve_a: ra,
                    reserve_b: rb,
                    shares,
                    // Every share, not just the minimum: nobody holds any of
                    // them, so nobody can pull the liquidity out from under
                    // the people who just bought.
                    locked: shares,
                    fee_bps: l.fee_bps,
                },
            );
        }
        self.market.pay(&l.source, 0, l.creator, l.token, Fixed(creator_tokens));
        self.market.pay(&l.source, 1, l.creator, l.quote, Fixed(creator_quote));
        self.market.launches.insert(l.token, Launch { state: LaunchState::Graduated, ..l });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(x: i64, y: i64, fee: u32) -> Pool {
        Pool {
            a: [1; 32],
            b: [2; 32],
            reserve_a: Fixed::whole(x),
            reserve_b: Fixed::whole(y),
            shares: Fixed::whole(1),
            locked: MINIMUM_LIQUIDITY,
            fee_bps: fee,
        }
    }

    fn order(n: u8, sell_a: bool, amount: i64, min_out: Fixed) -> Order {
        Order {
            id: [n; 32],
            owner: [n; 32],
            pool: [0; 32],
            sell_a,
            amount_in: Fixed::whole(amount),
            min_out,
        }
    }

    fn k(a: Fixed, b: Fixed) -> (u128, u128) {
        mul_u128(a.0 as u128, b.0 as u128)
    }

    /// One seller of 100 A into 1000/1000 at no fee: p = 1000/(1000+200),
    /// so 100 A buys 83.33… B — against 90.9 B at the constant-product
    /// marginal path. The difference is what an arbitrageur would have taken.
    #[test]
    fn a_single_sale_is_priced_at_the_fm_amm_price() {
        let c = clear(&pool(1000, 1000, 0), &[order(1, true, 100, Fixed::ZERO)]);
        assert_eq!(c.refunded, Vec::<usize>::new());
        let out = c.filled[0].1;
        assert_eq!(out, Fixed(83_333_333_333_333_333_333));
        assert_eq!(c.reserve_a, Fixed::whole(1100));
        assert_eq!(c.reserve_b.0, Fixed::whole(1000).0 - out.0);
    }

    /// Opposite orders net against each other and trade at one price: equal
    /// and opposite sales clear at the pool price with the pool untouched.
    #[test]
    fn opposite_orders_cross_at_one_price() {
        let c = clear(
            &pool(1000, 1000, 0),
            &[order(1, true, 50, Fixed::ZERO), order(2, false, 50, Fixed::ZERO)],
        );
        assert_eq!(c.filled.len(), 2);
        assert_eq!(c.filled[0].1, Fixed::whole(50));
        assert_eq!(c.filled[1].1, Fixed::whole(50));
        assert_eq!((c.reserve_a, c.reserve_b), (Fixed::whole(1000), Fixed::whole(1000)));
    }

    /// Order within a batch changes nothing. That is the whole defence
    /// against the one party who sees the orders.
    #[test]
    fn order_within_the_batch_does_not_matter() {
        let p = pool(5000, 7000, 30);
        let o = [
            order(1, true, 120, Fixed::ZERO),
            order(2, false, 80, Fixed::ZERO),
            order(3, true, 7, Fixed::ZERO),
        ];
        let forward = clear(&p, &o);
        let rev: Vec<Order> = o.iter().rev().copied().collect();
        let backward = clear(&p, &rev);
        let by_id = |c: &Clearing, os: &[Order]| {
            let mut v: Vec<(Hash, Fixed)> = c.filled.iter().map(|(i, f)| (os[*i].id, *f)).collect();
            v.sort();
            v
        };
        assert_eq!(by_id(&forward, &o), by_id(&backward, &rev));
        assert_eq!((forward.reserve_a, forward.reserve_b), (backward.reserve_a, backward.reserve_b));
    }

    /// `x′·y′ ≥ x·y` for every batch, fee or not, one side or both.
    #[test]
    fn clearing_never_loses_the_pool_value() {
        for (x, y, fee) in [(1000, 1000, 0), (10, 90_000, 30), (777, 3, 100)] {
            for (sa, sb) in [(1, 0), (0, 1), (500, 3), (3, 500), (100_000, 1), (40, 40)] {
                let p = pool(x, y, fee);
                let mut os = Vec::new();
                if sa > 0 {
                    os.push(order(1, true, sa, Fixed::ZERO));
                }
                if sb > 0 {
                    os.push(order(2, false, sb, Fixed::ZERO));
                }
                let c = clear(&p, &os);
                assert!(k(c.reserve_a, c.reserve_b) >= k(p.reserve_a, p.reserve_b), "{x} {y} {fee} {sa} {sb}");
                assert!(c.reserve_a.0 * 2 > p.reserve_a.0 && c.reserve_b.0 * 2 > p.reserve_b.0);
            }
        }
    }

    /// An order the price does not satisfy is refunded, and the others clear
    /// at the price recomputed without it.
    #[test]
    fn an_unmet_minimum_is_refunded_and_the_rest_reprice() {
        let p = pool(1000, 1000, 0);
        let greedy = order(1, true, 100, Fixed::whole(99));
        let fine = order(2, true, 100, Fixed::ZERO);
        let c = clear(&p, &[greedy, fine]);
        assert_eq!(c.refunded, vec![0]);
        let alone = clear(&p, &[fine]);
        assert_eq!(c.filled[0].1, alone.filled[0].1);
    }

    #[test]
    fn the_geometric_mean_is_exact_on_squares() {
        assert_eq!(geometric_mean(Fixed::whole(4), Fixed::whole(9)), Some(Fixed::whole(6)));
        assert_eq!(geometric_mean(Fixed::whole(1), Fixed::whole(1)), Some(Fixed::whole(1)));
        assert_eq!(geometric_mean(Fixed::ZERO, Fixed::whole(1)), None);
        let big = Fixed::whole(1_000_000_000_000);
        assert_eq!(geometric_mean(big, big), Some(big));
    }
}
