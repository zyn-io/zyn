//! Price history for launchpad tokens: what a chart is drawn from.
//!
//! Derived, not consensus (§69.23): the node samples its own state after each
//! epoch and records a point whenever a curve's lots or reserve moved, or a
//! graduated token's pool did. Anyone replaying the journal can recompute the
//! same points, and a missing chart never blocks settlement.
//!
//! What a point says is public already — lots sold, what the curve holds,
//! the pool's reserves. Batches clear at one price, so there is no per-trade,
//! per-wallet record to leak: an epoch's net movement is the finest grain.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use objectvm::ObjectVm;
use serde_json::{json, Value};
use zyn_vm::fixed::WAD;

type Token = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Point {
    pub epoch: u64,
    /// Seconds since the Unix epoch, when the node saw it.
    pub time: u64,
    pub token: Token,
    /// On its curve, or trading in its pool.
    pub pool: bool,
    /// Lots sold on the curve (the whole curve once graduated).
    pub sold: u32,
    /// The quote asset per token: the next lot on a curve, spot in a pool.
    pub price: i128,
    /// The quote asset the token is set in: its curve's reserve, or its pool's side.
    pub zec: i128,
    /// What moved since the previous point: lots (a curve), quote (either).
    pub delta_lots: i64,
    pub delta_zec: i128,
}

impl Point {
    pub fn json(&self) -> Value {
        json!({
            "epoch": self.epoch, "time": self.time, "token": hex(&self.token), "venue": if self.pool { "pool" } else { "curve" },
            "sold": self.sold, "price": self.price.to_string(), "zec": self.zec.to_string(),
            "delta_lots": self.delta_lots, "delta_zec": self.delta_zec.to_string(),
        })
    }

    fn parse(v: &Value) -> Option<Point> {
        Some(Point {
            epoch: v.get("epoch")?.as_u64()?,
            time: v.get("time")?.as_u64()?,
            token: unhex32(v.get("token")?.as_str()?)?,
            pool: v.get("venue")?.as_str()? == "pool",
            sold: v.get("sold")?.as_u64()? as u32,
            price: v.get("price")?.as_str()?.parse().ok()?,
            zec: v.get("zec")?.as_str()?.parse().ok()?,
            delta_lots: v.get("delta_lots")?.as_i64()?,
            delta_zec: v.get("delta_zec")?.as_str()?.parse().ok()?,
        })
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex32(s: &str) -> Option<Token> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Where each token stands: `(pool, sold, price, zec)`.
fn snapshot(vm: &ObjectVm) -> BTreeMap<Token, (bool, u32, i128, i128)> {
    let mut out = BTreeMap::new();
    for c in vm.market.curves.values() {
        if !c.graduated {
            out.insert(c.token, (false, c.sold, c.price_per_token().0, c.reserve.0));
            continue;
        }
        let pid = objectvm::market::pool_id(&c.token, &c.quote);
        if let Some(p) = vm.market.pools.get(&pid) {
            let (zec, tok) = if p.a == c.quote { (p.reserve_a.0, p.reserve_b.0) } else { (p.reserve_b.0, p.reserve_a.0) };
            if tok > 0 {
                let price = zyn_vm::fixed::mul_div(zec, WAD, tok).unwrap_or(0);
                out.insert(c.token, (true, c.sold, price, zec));
            }
        }
    }
    out
}

pub struct History {
    path: Option<PathBuf>,
    points: Vec<Point>,
    last: BTreeMap<Token, (bool, u32, i128, i128)>,
    /// When the node first saw each token.
    created: BTreeMap<Token, u64>,
    seen_epoch: Option<u64>,
}

impl History {
    /// Read what was recorded before, if anything.
    pub fn load(path: Option<&Path>) -> History {
        let mut h = History { path: path.map(Path::to_path_buf), points: Vec::new(), last: BTreeMap::new(), created: BTreeMap::new(), seen_epoch: None };
        if let Some(text) = path.and_then(|p| std::fs::read_to_string(p).ok()) {
            for line in text.lines() {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                if let Some(t) = v.get("created").and_then(Value::as_str).and_then(unhex32) {
                    if let Some(time) = v.get("time").and_then(Value::as_u64) {
                        h.created.entry(t).or_insert(time);
                    }
                    continue;
                }
                if let Some(p) = Point::parse(&v) {
                    h.last.insert(p.token, (p.pool, p.sold, p.price, p.zec));
                    h.points.push(p);
                }
            }
        }
        h
    }

    fn append(&self, line: &Value) {
        if let Some(path) = &self.path {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    /// Record whatever moved since the last look. Cheap when nothing did:
    /// called every couple of seconds, it only works once per epoch.
    pub fn observe(&mut self, vm: &ObjectVm, now: u64) -> usize {
        if self.seen_epoch == Some(vm.epoch) {
            return 0;
        }
        self.seen_epoch = Some(vm.epoch);
        let mut added = 0;
        for (token, now_at) in snapshot(vm) {
            if !self.created.contains_key(&token) {
                self.created.insert(token, now);
                self.append(&json!({ "created": hex(&token), "time": now }));
            }
            let before = self.last.get(&token).copied();
            if before == Some(now_at) {
                continue;
            }
            let (pool, sold, price, zec) = now_at;
            let (_, b_sold, _, b_zec) = before.unwrap_or((pool, sold, price, zec));
            let p = Point {
                epoch: vm.epoch,
                time: now,
                token,
                pool,
                sold,
                price,
                zec,
                delta_lots: sold as i64 - b_sold as i64,
                delta_zec: zec - b_zec,
            };
            self.append(&p.json());
            self.last.insert(token, now_at);
            self.points.push(p);
            added += 1;
        }
        added
    }

    /// A token's points from `since` on, newest last, at most `limit`.
    pub fn for_token(&self, token: &Token, since: u64, limit: usize) -> Vec<&Point> {
        let all: Vec<&Point> = self.points.iter().filter(|p| &p.token == token && p.time >= since).collect();
        let skip = all.len().saturating_sub(limit);
        all.into_iter().skip(skip).collect()
    }

    /// For the gallery: when each token was first seen, when it last moved,
    /// its price an hour and a day ago, and what moved in the last day.
    pub fn stats(&self, now: u64) -> Value {
        let mut out = Vec::new();
        for (token, created) in &self.created {
            let mine: Vec<&Point> = self.points.iter().filter(|p| &p.token == token).collect();
            // A token younger than the window changed by what it did since
            // it was made: its first point stands in for the price back then.
            let price_at = |t: u64| mine.iter().rev().find(|p| p.time <= t).or(mine.first()).map(|p| p.price.to_string());
            let day = now.saturating_sub(86_400);
            let recent: Vec<&&Point> = mine.iter().filter(|p| p.time >= day && (p.delta_lots != 0 || p.delta_zec != 0)).collect();
            out.push(json!({
                "token": hex(token),
                "created_time": created,
                "last_trade_time": mine.iter().rev().find(|p| p.delta_lots != 0 || p.delta_zec != 0).map(|p| p.time),
                "price": mine.last().map(|p| p.price.to_string()),
                "price_1h": price_at(now.saturating_sub(3_600)),
                "price_24h": price_at(day),
                "volume_24h": recent.iter().map(|p| p.delta_zec.unsigned_abs()).sum::<u128>().to_string(),
                "moves_24h": recent.len(),
            }));
        }
        Value::Array(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objectvm::curve::{curve_order_id, LOT};
    use objectvm::market::token_of;
    use objectvm::{Limits, Op};
    use zyn_vm::object::Note;
    use zyn_vm::spec::MicrochainVm;
    use zyn_vm::{program, Fixed};

    const ZEC: [u8; 32] = [0x10; 32];
    const ALICE: [u8; 32] = [0xA1; 32];

    #[test]
    fn a_curve_records_its_creation_and_each_epoch_it_moved() {
        let dir = std::env::temp_dir().join(format!("zyn-history-{}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        let mut vm = ObjectVm::genesis(212, Limits { max_io: 16 });
        let mut seq = 0;
        let mut apply = |vm: &mut ObjectVm, op: Op| {
            seq += 1;
            let r = vm.apply(seq, &op);
            assert!(!ObjectVm::rejected(&r[0]), "{r:?}");
        };
        apply(&mut vm, Op::Mint { asset: ZEC, amount: Fixed::whole(5), owner: ALICE, nonce: [1; 32] });
        apply(&mut vm, Op::CreateCurve { creator: ALICE, program: program::curve_create(), nonce: [2; 32], quote: ZEC, meta: [0; 32], inputs: vec![], change: vec![] });
        apply(&mut vm, Op::Seal);
        let token = token_of(&ALICE, &[2; 32]);
        let mut h = History::load(Some(&dir));
        assert_eq!(h.observe(&vm, 1_000), 1, "a new token gets its first point");
        assert_eq!(h.observe(&vm, 1_001), 0, "nothing new in the same epoch");

        let note = *vm.live.iter().find(|(_, n)| n.owner == ALICE).unwrap().0;
        let enc = zyn_vm::note_enc::seal(&zyn_vm::note_enc::viewing_public(&zyn_vm::note_enc::StaticSecret::from([1; 32])), zyn_vm::note_enc::StaticSecret::from([2; 32]), b"n").unwrap();
        let change = objectvm::Output { note: Note { asset: ZEC, amount: Fixed::whole(4), owner: ALICE, nonce: [3; 32] }, enc };
        apply(&mut vm, Op::CurveTrade {
            trader: ALICE, program: program::curve_trade(), token, buy: true, amount_in: Fixed::whole(1), min_out: Fixed::ZERO,
            inputs: vec![(note, zyn_vm::object::nullifier(&[0xA1; 32], &note))], change: vec![change], nonce: [4; 32],
        });
        apply(&mut vm, Op::Seal);
        assert_eq!(h.observe(&vm, 1_060), 1);
        let pts = h.for_token(&token, 0, 100);
        assert_eq!(pts.len(), 2);
        assert!(pts[1].delta_lots > 0 && pts[1].delta_zec > 0 && pts[1].price > pts[0].price);
        let _ = (curve_order_id, LOT);

        // What was recorded survives a restart.
        let back = History::load(Some(&dir));
        assert_eq!(back.for_token(&token, 0, 100).len(), 2);
        let stats = back.stats(1_100);
        assert_eq!(stats[0]["created_time"], 1_000);
        assert_eq!(stats[0]["last_trade_time"], 1_060);
        // Younger than a day: the day's change is its change since it was made.
        assert_eq!(stats[0]["price_24h"], pts[0].price.to_string());
        let _ = std::fs::remove_file(&dir);
    }
}
