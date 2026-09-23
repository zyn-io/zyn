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

fn id(domain: &[u8], parts: &[&[u8]]) -> Hash {
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
}

impl Market {
    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
            && self.orders.is_empty()
            && self.claims.is_empty()
            && self.launches.is_empty()
            && self.commits.is_empty()
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
        Some(t)
    }

    /// Assets the market made rather than received: pool shares and launched
    /// tokens. They exist only here, so none of them can leave for Zcash.
    pub fn is_derived(&self, asset: &Address) -> bool {
        self.launches.contains_key(asset) || self.pools.keys().any(|p| &lp_asset(p) == asset)
    }

    fn pay(&mut self, source: &Hash, k: u8, owner: AccountId, asset: Address, amount: Fixed) {
        if amount.0 > 0 {
            self.claims.insert(claim_id(source, k), Claim { owner, asset, amount });
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
        e.leaf()
    }

    pub fn encode(&self, e: &mut Encoder) {
        e.bytes(b"MARKET01").u32(self.pools.len() as u32);
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
    }

    pub fn decode(d: &mut Decoder) -> Option<Market> {
        if d.take_bytes(8).ok()? != b"MARKET01" {
            return None;
        }
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
    fn fresh_source(&self, source: &Hash) -> Result<(), &'static str> {
        if (0..3).any(|k| self.market.claims.contains_key(&claim_id(source, k)))
            || self.market.orders.iter().any(|o| &o.id == source)
            || self.market.commits.contains_key(source)
        {
            return Err("that nonce was already used");
        }
        Ok(())
    }

    fn issue(&mut self, asset: Address, amount: Fixed) -> Result<(), &'static str> {
        let s = self.issued.entry(asset).or_insert(Fixed::ZERO);
        s.0 = s.0.checked_add(amount.0).ok_or("supply overflow")?;
        Ok(())
    }

    fn burn(&mut self, asset: Address, amount: Fixed) -> Result<(), &'static str> {
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
        if self.market.orders.iter().filter(|o| &o.pool == pool).count() >= MAX_ORDERS_PER_POOL {
            return Err("this pool's batch is full; try next epoch");
        }
        let sold = if sell_a { p.a } else { p.b };
        let f = self.plan_funding(trader, inputs, change, &[(sold, amount_in)])?;
        self.apply_funding(trader, inputs, f);
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
        if self.market.launches.contains_key(&token) || self.issued.contains_key(&token) {
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

    /// Everything that happens at a seal, before the root is taken: every
    /// pool's batch clears, and every sale whose last epoch this is settles.
    pub(crate) fn settle_market(&mut self) {
        if self.market.is_empty() {
            return;
        }
        let orders = std::mem::take(&mut self.market.orders);
        let pools: Vec<Hash> = self.market.pools.keys().copied().collect();
        for pid in pools {
            let batch: Vec<Order> = orders.iter().filter(|o| o.pool == pid).copied().collect();
            if batch.is_empty() {
                continue;
            }
            let p = self.market.pools[&pid];
            let c = clear(&p, &batch);
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
