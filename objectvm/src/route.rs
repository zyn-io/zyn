//! Atomic routes (DECISIONS §167): one order through up to four venues —
//! a coin's curve, pools, another coin's curve — settled in one seal, all or
//! nothing.
//!
//! # One price per venue per epoch
//!
//! A route's first hop joins its venue's batch when it is placed; its amount
//! is known. Every later hop joins the next venue's batch with what the hop
//! before it produced, so a venue must clear after every venue that feeds it.
//! The seal clears venues in that order — curves, then pools, as before,
//! except that a venue fed by a route waits for its feeder — and each venue
//! still clears once, at one price. A route that would need a venue to clear
//! both before and after another (a cycle with the routes already in the
//! batch) is refused when placed: try the next epoch.
//!
//! # All or nothing
//!
//! A route names one minimum, on what it finally delivers. If it would get
//! less, the batch is cleared again without it, on a copy of the ledger,
//! until every route left meets its minimum; only then does the seal clear
//! for real. A route that drops out is refunded whole, in what it put in. No
//! route is ever left half done holding a coin it did not ask for.
//!
//! # Where what it is owed waits
//!
//! Under the route's id: what it delivered (k = 0), a whole refund (k = 1),
//! and what a last curve could not spend before graduation (k = 2).

use std::collections::{BTreeMap, BTreeSet};

use zyn_vm::commit::{merkle_root, Encoder, Hash};
use zyn_vm::derive::Address;
use zyn_vm::read::Decoder;
use zyn_vm::spec::AccountId;
use zyn_vm::Fixed;

use crate::curve::{CurveOrder, LOT};
use crate::market::{claim_id, Order};
use crate::ObjectVm;

/// Hops in one route: a coin's curve, two pools, another coin's curve.
pub const MAX_HOPS: usize = 4;
/// Routes in one epoch's batch.
pub const MAX_ROUTES: usize = 64;

/// One step of a route.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Hop {
    /// Through a pool, selling its `a` for its `b` or the reverse.
    Pool { pool: Hash, sell_a: bool },
    /// Sell a coin to its curve. First hops only: whole lots, known up front.
    CurveSell { token: Address },
    /// Buy a coin from its curve. Last hops only.
    CurveBuy { token: Address },
}

/// Where a hop clears.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Venue {
    Curve(Address),
    Pool(Hash),
}

impl Hop {
    pub fn venue(&self) -> Venue {
        match self {
            Hop::Pool { pool, .. } => Venue::Pool(*pool),
            Hop::CurveSell { token } | Hop::CurveBuy { token } => Venue::Curve(*token),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Route {
    pub id: Hash,
    pub owner: AccountId,
    pub hops: Vec<Hop>,
    /// What it puts in, of the first hop's input.
    pub amount_in: Fixed,
    /// What it must deliver at least, of the last hop's output.
    pub min_out: Fixed,
}

/// A route's id, and so where what it is owed is claimable.
pub fn route_id(trader: &AccountId, nonce: &Hash) -> Hash {
    crate::market::id(b"objectvm.route.v1", &[trader, nonce])
}

/// The order a route's hop places in its venue's batch.
pub fn leg_id(route: &Hash, hop: usize) -> Hash {
    crate::market::id(b"objectvm.route-leg.v1", &[route, &[hop as u8]])
}

/// The venues each route makes wait for another: hop i feeds hop i+1.
fn edges<'a>(routes: impl Iterator<Item = &'a Route>) -> BTreeSet<(Venue, Venue)> {
    routes.flat_map(|r| r.hops.windows(2).map(|w| (w[0].venue(), w[1].venue()))).collect()
}

/// Whether the venues these edges order can all clear once each.
pub(crate) fn acyclic(edges: &BTreeSet<(Venue, Venue)>) -> bool {
    let mut indeg: BTreeMap<Venue, usize> = BTreeMap::new();
    for (a, b) in edges {
        indeg.entry(*a).or_insert(0);
        *indeg.entry(*b).or_insert(0) += 1;
    }
    let mut ready: Vec<Venue> = indeg.iter().filter(|(_, d)| **d == 0).map(|(v, _)| *v).collect();
    let mut seen = 0;
    while let Some(v) = ready.pop() {
        seen += 1;
        for (_, b) in edges.iter().filter(|(a, _)| *a == v) {
            let d = indeg.get_mut(b).expect("counted");
            *d -= 1;
            if *d == 0 {
                ready.push(*b);
            }
        }
    }
    seen == indeg.len()
}

/// Every venue, in the order the seal clears them: curves by token, then
/// pools by id — as before routes — except that a venue fed by a route
/// waits until its feeder has cleared.
fn schedule(curves: impl Iterator<Item = Address>, pools: impl Iterator<Item = Hash>, edges: &BTreeSet<(Venue, Venue)>) -> Vec<Venue> {
    let all: Vec<Venue> = curves.map(Venue::Curve).chain(pools.map(Venue::Pool)).collect();
    let mut indeg: BTreeMap<Venue, usize> = all.iter().map(|v| (*v, 0)).collect();
    for (_, b) in edges {
        if let Some(d) = indeg.get_mut(b) {
            *d += 1;
        }
    }
    // `Venue` orders curves before pools, each by its key: the old order.
    let mut ready: BTreeSet<Venue> = indeg.iter().filter(|(_, d)| **d == 0).map(|(v, _)| *v).collect();
    let mut out = Vec::with_capacity(all.len());
    while let Some(v) = ready.pop_first() {
        out.push(v);
        for (_, b) in edges.iter().filter(|(a, _)| *a == v) {
            if let Some(d) = indeg.get_mut(b) {
                *d -= 1;
                if *d == 0 {
                    ready.insert(*b);
                }
            }
        }
    }
    out
}

pub(crate) fn route_leaf(r: &Route) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.route-leaf.v1").bytes(&r.id).bytes(&r.owner);
    put_hops(&mut e, &r.hops);
    e.fixed(r.amount_in).fixed(r.min_out);
    e.leaf()
}

/// The routes section of the market's leaf, joined only while routes wait.
pub(crate) fn section(e: &mut Encoder, routes: &[Route]) {
    let leaves: Vec<Hash> = routes.iter().map(route_leaf).collect();
    e.bytes(b"objectvm.routes.v1").u32(leaves.len() as u32).bytes(&merkle_root(&leaves));
}

pub(crate) fn put_hops(e: &mut Encoder, hops: &[Hop]) {
    e.u8(hops.len() as u8);
    for h in hops {
        match h {
            Hop::Pool { pool, sell_a } => {
                e.u8(0).bytes(pool).bool(*sell_a);
            }
            Hop::CurveSell { token } => {
                e.u8(1).bytes(token);
            }
            Hop::CurveBuy { token } => {
                e.u8(2).bytes(token);
            }
        }
    }
}

pub(crate) fn read_hops(d: &mut Decoder) -> Option<Vec<Hop>> {
    let n = d.u8().ok()? as usize;
    if n == 0 || n > MAX_HOPS {
        return None;
    }
    let mut hops = Vec::with_capacity(n);
    for _ in 0..n {
        hops.push(match d.u8().ok()? {
            0 => Hop::Pool {
                pool: d.hash().ok()?,
                sell_a: match d.u8().ok()? {
                    0 => false,
                    1 => true,
                    _ => return None,
                },
            },
            1 => Hop::CurveSell { token: d.hash().ok()? },
            2 => Hop::CurveBuy { token: d.hash().ok()? },
            _ => return None,
        });
    }
    Some(hops)
}

pub(crate) fn encode(e: &mut Encoder, routes: &[Route]) {
    e.u32(routes.len() as u32);
    for r in routes {
        e.bytes(&r.id).bytes(&r.owner);
        put_hops(e, &r.hops);
        e.fixed(r.amount_in).fixed(r.min_out);
    }
}

pub(crate) fn decode(d: &mut Decoder) -> Option<Vec<Route>> {
    let mut routes = Vec::new();
    for _ in 0..d.u32().ok()? {
        routes.push(Route {
            id: d.hash().ok()?,
            owner: d.account().ok()?,
            hops: read_hops(d)?,
            amount_in: d.fixed().ok()?,
            min_out: d.fixed().ok()?,
        });
    }
    Some(routes)
}

impl ObjectVm {
    /// What a hop takes in and gives out, as the ledger stands now.
    fn hop_assets(&self, hop: &Hop) -> Result<(Address, Address), &'static str> {
        match hop {
            Hop::Pool { pool, sell_a } => {
                let p = self.market.pools.get(pool).ok_or("no such pool")?;
                Ok(if *sell_a { (p.a, p.b) } else { (p.b, p.a) })
            }
            Hop::CurveSell { token } | Hop::CurveBuy { token } => {
                let c = self.market.curves.get(token).ok_or("no such curve")?;
                if c.graduated {
                    return Err("that coin has graduated; route through its pool");
                }
                Ok(if matches!(hop, Hop::CurveSell { .. }) { (c.token, c.quote) } else { (c.quote, c.token) })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_route(
        &mut self,
        trader: &AccountId,
        hops: &[Hop],
        amount_in: Fixed,
        min_out: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[crate::Output],
        nonce: &Hash,
    ) -> Result<crate::Receipt, &'static str> {
        if hops.is_empty() || hops.len() > MAX_HOPS {
            return Err("a route is one to four hops");
        }
        if amount_in.0 <= 0 || min_out.0 < 0 {
            return Err("a route puts something in and asks for at least nothing");
        }
        let venues: BTreeSet<Venue> = hops.iter().map(Hop::venue).collect();
        if venues.len() != hops.len() {
            return Err("a route passes each venue once");
        }
        // What flows: each hop's output is the next one's input.
        let mut flow = None;
        for (i, h) in hops.iter().enumerate() {
            match h {
                Hop::CurveSell { .. } if i != 0 => return Err("a curve sale can only start a route"),
                Hop::CurveBuy { .. } if i != hops.len() - 1 => return Err("a curve purchase can only end a route"),
                _ => {}
            }
            let (input, output) = self.hop_assets(h)?;
            if flow.is_some_and(|f| f != input) {
                return Err("each hop must take what the hop before it gives");
            }
            flow = Some(output);
        }
        let start = self.hop_assets(&hops[0])?.0;
        match hops[0] {
            Hop::CurveSell { .. } if amount_in.0 % LOT.0 != 0 => return Err("a sale to a curve is a whole number of lots (12,500 tokens each)"),
            Hop::CurveBuy { token } if amount_in < self.market.curves[&token].min_buy() => return Err("a buy is at least 0.001"),
            _ => {}
        }
        if self.market.routes.len() >= MAX_ROUTES {
            return Err("this epoch's routes are full; try next epoch");
        }
        // Every venue keeps its batch limit, counting the legs routes will add.
        for v in &venues {
            let legs = self.market.routes.iter().flat_map(|r| r.hops.iter()).filter(|h| h.venue() == *v).count();
            let (direct, cap) = match v {
                Venue::Pool(p) => (self.market.orders.iter().filter(|o| &o.pool == p).count(), crate::market::MAX_ORDERS_PER_POOL),
                Venue::Curve(t) => (self.market.curve_orders.iter().filter(|o| &o.token == t).count(), crate::curve::MAX_ORDERS_PER_CURVE),
            };
            if direct + legs >= cap {
                return Err("a venue on this route has a full batch; try next epoch");
            }
        }
        let id = route_id(trader, nonce);
        self.fresh_source(&id)?;
        let route = Route { id, owner: *trader, hops: hops.to_vec(), amount_in, min_out };
        let all = edges(self.market.routes.iter().chain(std::iter::once(&route)));
        if !acyclic(&all) {
            return Err("a route the other way through these venues is in this epoch's batch; try next epoch");
        }
        let f = self.plan_funding(trader, inputs, change, &[(start, amount_in)])?;
        self.apply_funding(trader, inputs, f);
        self.place_leg(&route, 0, amount_in);
        self.market.routes.push(route);
        Ok(crate::Receipt::Ordered { order: id })
    }

    /// Hop `i` of a route joins its venue's batch with `amount`.
    fn place_leg(&mut self, r: &Route, i: usize, amount: Fixed) {
        let id = leg_id(&r.id, i);
        match r.hops[i] {
            Hop::Pool { pool, sell_a } => self.market.orders.push(Order { id, owner: r.owner, pool, sell_a, amount_in: amount, min_out: Fixed::ZERO }),
            Hop::CurveSell { token } => {
                self.market.curve_orders.push(CurveOrder { id, owner: r.owner, token, buy: false, amount_in: amount, min_out: Fixed::ZERO })
            }
            Hop::CurveBuy { token } => {
                self.market.curve_orders.push(CurveOrder { id, owner: r.owner, token, buy: true, amount_in: amount, min_out: Fixed::ZERO })
            }
        }
    }

    /// The seal with routes waiting: drop every route that would miss its
    /// minimum (refunded whole), then clear the rest for real.
    pub(crate) fn settle_routed(&mut self) {
        let mut routes = std::mem::take(&mut self.market.routes);
        loop {
            let mut sim = self.clone();
            let got = sim.clear_venues(&routes);
            let short: Vec<Hash> = routes.iter().filter(|r| got.get(&r.id).is_none_or(|g| *g < r.min_out)).map(|r| r.id).collect();
            if short.is_empty() {
                break;
            }
            for id in short {
                let r = routes.iter().position(|r| r.id == id).expect("listed");
                let r = routes.remove(r);
                // Out of its first venue's batch, and back to its owner whole.
                let leg = leg_id(&r.id, 0);
                self.market.orders.retain(|o| o.id != leg);
                self.market.curve_orders.retain(|o| o.id != leg);
                let start = self.hop_assets_settled(&r.hops[0]).0;
                self.market.pay(&r.id, 1, r.owner, start, r.amount_in);
            }
        }
        self.clear_venues(&routes);
    }

    /// Clear every venue once, in route order, carrying each route's output
    /// from hop to hop; what each route finally delivered, by route.
    fn clear_venues(&mut self, routes: &[Route]) -> BTreeMap<Hash, Fixed> {
        let order = schedule(self.market.curves.keys().copied(), self.market.pools.keys().copied(), &edges(routes.iter()));
        let mut delivered = BTreeMap::new();
        let mut lend_done = false;
        for v in order {
            match v {
                Venue::Curve(token) => {
                    let batch: Vec<CurveOrder> = self.market.curve_orders.iter().filter(|o| o.token == token).copied().collect();
                    self.market.curve_orders.retain(|o| o.token != token);
                    if !batch.is_empty() {
                        self.settle_curve(token, batch);
                    }
                }
                Venue::Pool(pid) => {
                    if !lend_done {
                        // Lending before the pools clear, as without routes.
                        self.settle_lend();
                        lend_done = true;
                    }
                    let batch: Vec<Order> = self.market.orders.iter().filter(|o| o.pool == pid).copied().collect();
                    self.market.orders.retain(|o| o.pool != pid);
                    if !batch.is_empty() {
                        self.settle_pool(pid, &batch);
                    }
                }
            }
            // Every route with a hop here moves on, or delivers.
            for r in routes {
                let Some(i) = r.hops.iter().position(|h| h.venue() == v) else { continue };
                let leg = leg_id(&r.id, i);
                let out = self.market.claims.remove(&claim_id(&leg, 0)).map(|c| c.amount).unwrap_or(Fixed::ZERO);
                let back = self.market.claims.remove(&claim_id(&leg, 1));
                let last = i + 1 == r.hops.len();
                if last && back.is_some() && matches!(r.hops[i], Hop::Pool { .. }) {
                    // A last pool that refunded delivers nothing the route asked
                    // for: it drops out on the copy and is refunded whole, never
                    // left holding the coin before it (audit, 6 Oct 2026).
                    if let Some(b) = back {
                        self.market.pay(&r.id, 1, r.owner, b.asset, b.amount);
                    }
                } else if last {
                    let got = self.hop_assets_settled(&r.hops[i]).1;
                    self.market.pay(&r.id, 0, r.owner, got, out);
                    if let Some(b) = back {
                        // What a last curve could not spend before graduating.
                        self.market.pay(&r.id, 2, r.owner, b.asset, b.amount);
                    }
                    delivered.insert(r.id, out);
                } else if out.0 > 0 && back.is_none() {
                    self.place_leg(r, i + 1, out);
                } else if let Some(b) = back {
                    // A middle hop refunded: the route delivers nothing here
                    // and drops out on the copy; on the real ledger, never.
                    self.market.pay(&r.id, 1, r.owner, b.asset, b.amount);
                }
            }
        }
        if !lend_done {
            self.settle_lend();
        }
        delivered
    }

    /// A hop's assets once its venue may have changed this seal: a curve
    /// that graduated still names its token and quote.
    fn hop_assets_settled(&self, hop: &Hop) -> (Address, Address) {
        match hop {
            Hop::Pool { pool, sell_a } => {
                let p = &self.market.pools[pool];
                if *sell_a { (p.a, p.b) } else { (p.b, p.a) }
            }
            Hop::CurveSell { token } => (self.market.curves[token].token, self.market.curves[token].quote),
            Hop::CurveBuy { token } => (self.market.curves[token].quote, self.market.curves[token].token),
        }
    }
}
