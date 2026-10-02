//! What the public and a position's owner may see of Zyn Lend.
//!
//! The node answers `zyn_lendMarkets` and `zyn_lendPositions` with these, and
//! the lending demo records its "what everyone sees" column with the same
//! functions, so the demo cannot show less than the node publishes.

use objectvm::ObjectVm;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Round `v` to the nearest `step` (both in ledger units).
/// The step that keeps `digits` significant figures of `v`.
pub fn sig_step(v: i128, digits: u32) -> i128 {
    let mut step: i128 = 1;
    while v / step >= 10i128.pow(digits) {
        step *= 10;
    }
    step
}

pub fn quantize(v: i128, step: i128) -> i128 {
    if step <= 1 {
        return v;
    }
    ((v + step / 2) / step) * step
}

/// A pool's public figures for the lending app — quantized, so one action
/// cannot be read off a total.
///
/// Cash and borrowed round to the same step, two significant figures of what
/// is supplied, and supplied is cash plus borrowed less the reserve on that
/// step, so the three always add up. Utilization is taken from the exact
/// figures and rounded *up* to the next percent — a loan never shows as 0%
/// — and the rates come from that rounded utilization, so they say no more
/// than it does. The share count is replaced by the share price, which shows
/// interest and nothing about amounts. No count of positions is published.
/// Insurance is the protocol's own money, so it keeps two figures of its own
/// size rather than the pool's.
///
/// Under rules v2 (DECISIONS §160) it also carries what the app needs to
/// act: per asset, how old its price is and whether the breaker holds it,
/// whether withdrawals or deposits wait, room under the collateral cap; per
/// pair, room under the depth limit; per lot, its price now, its smallest
/// bid and its timing; and the constants those come from (`auction`,
/// `price_max_age_secs`, `depth_limit_bps`, `curve`). Amounts are WAD
/// strings, times the ledger's clock (`time`) in seconds.
pub fn lend_markets_json(s: &ObjectVm) -> serde_json::Value {
    use objectvm::lend::{self as lend, lend_asset, lot_source};
    use objectvm::zyn_lend::math::{bps, mul_div, wmul};
    use objectvm::zyn_lend::WAD;
    let l = &s.market.lend;
    let v2 = l.v2();
    let curve = l.curve();
    let pools: Vec<serde_json::Value> = l
        .listed
        .iter()
        .map(|(a, x)| {
            let p = &x.pool;
            let value = p.value().unwrap_or(0);
            let debt = p.debt().unwrap_or(0);
            let step = sig_step(value, 2);
            let (bq, cq, rq) = (quantize(debt, step), quantize(p.cash, step), quantize(p.reserves, step));
            let sq = (cq + bq - rq).max(0);
            let util = round_up(p.utilization().unwrap_or(0), WAD / 100).min(WAD);
            let borrow_rate = curve.rate(util).unwrap_or(0);
            let gross = wmul(borrow_rate, util).unwrap_or(0);
            let supply_rate = gross - bps(gross, curve.reserve_bps).unwrap_or(0);
            // A share's price and the borrow index move with exact interest,
            // which gives exact utilization away; to six decimals they move
            // only as interest adds up.
            let share_price = if p.supply_shares > 0 { mul_div(value, WAD, p.supply_shares).unwrap_or(WAD) } else { WAD };
            let share_price = quantize(share_price, WAD / 1_000_000);
            let limits = l.limits.get(a);
            let insurance = l.insurance.get(a).map(|x| x.0).unwrap_or(0);
            let collateral_room = limits.map(|m| {
                let room = (m.collateral_cap.0 - l.collateral_held(a)).max(0);
                round_down(room, sig_step(m.collateral_cap.0, 2)).to_string()
            });
            serde_json::json!({
                "asset": hex(a),
                "receipt": hex(&lend_asset(a)),
                "cash": cq.to_string(),
                "borrowed": bq.to_string(),
                "supplied": sq.to_string(),
                "share_price": share_price.to_string(),
                "borrow_index": quantize(p.borrow_index, WAD / 1_000_000).to_string(),
                "utilization": util.to_string(),
                "borrow_rate": borrow_rate.to_string(),
                "supply_rate": supply_rate.to_string(),
                "max_ltv_bps": x.risk.max_ltv_bps,
                "liquidation_bps": x.risk.liquidation_bps,
                "penalty_bps": x.risk.penalty_bps,
                "collateral_cap": limits.map(|x| x.collateral_cap.0.to_string()),
                "borrow_cap": limits.map(|x| x.borrow_cap.0.to_string()),
                "min_borrow_usd": limits.map(|x| x.min_borrow_usd.0.to_string()),
                "collateral_room": collateral_room,
                "insurance": quantize(insurance, sig_step(insurance, 2)).to_string(),
                "price_at": if v2 { l.priced_at.get(a).copied() } else { None },
                "price_fresh": l.fresh(a, s.epoch),
                "price_held": l.pending.contains_key(a),
                "held_price": l.pending.get(a).map(|x| x.0.to_string()),
                "held_posts": if v2 { Some(l.held_posts.get(a).copied().unwrap_or(0)) } else { None },
                "withdraw_paused": l.withdraw_paused(a),
                "supply_paused": l.supply_paused(a),
            })
        })
        .collect();
    let prices: serde_json::Map<String, serde_json::Value> =
        l.prices.iter().map(|(a, p)| (hex(a), serde_json::Value::String(p.0.to_string()))).collect();
    // Every ordered pair with a Zyn pool: how much more collateral it takes.
    let mut pairs = Vec::new();
    if v2 {
        for c in l.listed.keys() {
            for b in l.listed.keys().filter(|b| *b != c) {
                let Some((reserve, limit, room)) = s.depth_room(c, b) else { continue };
                let agree = match (l.prices.get(c), l.prices.get(b), pool_ratio(s, c, b)) {
                    (Some(pc), Some(pb), Some(r)) => objectvm::zyn_lend::position::prices_agree(pc.0, pb.0, r).unwrap_or(false),
                    _ => false,
                };
                pairs.push(serde_json::json!({
                    "collateral_asset": hex(c),
                    "borrow_asset": hex(b),
                    "pool_reserve": reserve.to_string(),
                    "depth_limit": limit.to_string(),
                    "room": round_down(room.max(0), sig_step(limit, 2)).to_string(),
                    "prices_agree": agree,
                }));
            }
        }
    }
    let lots: Vec<serde_json::Value> = l
        .lots
        .iter()
        .map(|(pos, lot)| {
            let owed = l.listed.get(&lot.borrow_asset).and_then(|x| x.pool.owed(lot.debt_shares.0).ok()).unwrap_or(0);
            let bids: Vec<i128> = l.bids.iter().filter(|b| &b.lot == pos).map(|b| b.pay.0).collect();
            let full = v2 && bids.len() >= lend::MAX_BIDS_PER_LOT;
            serde_json::json!({
                "lot": hex(&lot_source(pos)),
                "collateral_asset": hex(&lot.collateral_asset),
                "collateral": lot.collateral.0.to_string(),
                "borrow_asset": hex(&lot.borrow_asset),
                "owed": owed.to_string(),
                "proceeds": lot.proceeds.0.to_string(),
                "price": l.auction_price(lot, s.epoch).unwrap_or(0).to_string(),
                "rate": lot.rate.0.to_string(),
                "opened": lot.opened,
                "opened_at": if v2 { Some(lot.opened_at) } else { None },
                "floor_at": if v2 { Some(lot.opened_at + lend::AUCTION_FLOOR_AFTER_SECS) } else { None },
                "backstop_at": if v2 { Some(lot.opened_at + lend::BACKSTOP_AFTER_SECS) } else { None },
                "min_bid": if v2 { s.lot_min_bid(lot).map(|m| m.to_string()) } else { None },
                "bids_full": full,
                "outbid_above": if full { bids.iter().min().map(|m| m.to_string()) } else { None },
            })
        })
        .collect();
    let prices_fresh = if v2 {
        l.listed.keys().all(|a| l.fresh(a, s.epoch))
    } else {
        !l.guarded || s.epoch <= l.priced_epoch + lend::MAX_PRICE_AGE_EPOCHS
    };
    serde_json::json!({
        "epoch": s.epoch,
        "time": l.time,
        "rules": if v2 { l.rules } else if l.guarded { 1 } else { 0 },
        "prices": prices,
        "prices_fresh": prices_fresh,
        "price_max_age_secs": if v2 { Some(lend::MAX_PRICE_AGE_SECS) } else { None },
        "guarded": l.guarded,
        "curve": {
            "base": curve.base.to_string(),
            "slope1": curve.slope1.to_string(),
            "kink": curve.kink.to_string(),
            "slope2": curve.slope2.to_string(),
            "reserve_bps": curve.reserve_bps,
        },
        "auction": if v2 {
            serde_json::json!({
                "start_bps": lend::AUCTION_START_BPS,
                "step_bps": lend::AUCTION_STEP_BPS,
                "step_secs": lend::AUCTION_STEP_SECS,
                "floor_bps": lend::AUCTION_FLOOR_BPS,
                "floor_after_secs": lend::AUCTION_FLOOR_AFTER_SECS,
                "backstop_after_secs": lend::BACKSTOP_AFTER_SECS,
                "backstop_min_bps": lend::BACKSTOP_MIN_BPS,
                "min_bid_bps": lend::MIN_BID_BPS,
                "max_bids": lend::MAX_BIDS_PER_LOT,
            })
        } else {
            serde_json::Value::Null
        },
        "depth_limit_bps": if v2 { Some(lend::DEPTH_LIMIT_BPS) } else { None },
        "pairs": if v2 { serde_json::Value::Array(pairs) } else { serde_json::Value::Null },
        "pools": pools,
        "lots": lots,
    })
}

/// Units of `b` per unit of `a` in Zyn's pool for the pair, WAD.
fn pool_ratio(s: &ObjectVm, a: &[u8; 32], b: &[u8; 32]) -> Option<i128> {
    let p = s.market.pools.get(&objectvm::market::pool_id(a, b))?;
    let (ra, rb) = if &p.a == a { (p.reserve_a.0, p.reserve_b.0) } else { (p.reserve_b.0, p.reserve_a.0) };
    if ra <= 0 || rb <= 0 {
        return None;
    }
    objectvm::zyn_lend::math::mul_div(rb, objectvm::zyn_lend::WAD, ra).ok()
}

/// Round `v` up to a multiple of `step`.
pub fn round_up(v: i128, step: i128) -> i128 {
    if step <= 1 {
        return v;
    }
    v.div_euclid(step) * step + if v.rem_euclid(step) > 0 { step } else { 0 }
}

/// Round `v` down to a multiple of `step`.
pub fn round_down(v: i128, step: i128) -> i128 {
    if step <= 1 {
        return v;
    }
    v.div_euclid(step) * step
}

/// One position, or the auction it became, for its owner.
pub fn lend_position_json(s: &ObjectVm, id: &[u8; 32]) -> Option<serde_json::Value> {
    use objectvm::lend::lot_source;
    let l = &s.market.lend;
    if let Some(p) = l.positions.get(id) {
        let pool = &l.listed.get(&p.borrow_asset)?.pool;
        let owed = pool.owed(p.debt_shares.0).ok()?;
        let risk = l.listed.get(&p.collateral_asset)?.risk;
        // Under rules v2, the worse of the confirmed and the held price —
        // what liquidation uses.
        let price = |a: &[u8; 32], collateral: bool| if l.v2() { l.worse_price(a, collateral) } else { l.prices.get(a).map(|x| x.0) };
        let health = match (price(&p.collateral_asset, true), price(&p.borrow_asset, false)) {
            (Some(pc), Some(pb)) => {
                let cv = objectvm::zyn_lend::position::value(p.collateral.0, pc).ok()?;
                let dv = objectvm::zyn_lend::position::debt_value(owed, pb).ok()?;
                objectvm::zyn_lend::position::health(cv, dv, &risk).ok()?.map(|h| h.to_string())
            }
            _ => None,
        };
        // Rules v2: a part payment must leave the pool's smallest loan.
        let repay_max_partial = if l.v2() {
            let min = l.limits.get(&p.borrow_asset).map(|m| m.min_borrow_usd.0).unwrap_or(0);
            let left = match l.prices.get(&p.borrow_asset).map(|x| x.0).filter(|x| *x > 0) {
                Some(pb) => objectvm::zyn_lend::math::mul_div_up(min, objectvm::zyn_lend::WAD, pb).unwrap_or(owed),
                None => 0,
            };
            Some((owed - left).max(0).to_string())
        } else {
            None
        };
        return Some(serde_json::json!({
            "id": hex(id),
            "state": "open",
            "collateral_asset": hex(&p.collateral_asset),
            "collateral": p.collateral.0.to_string(),
            "borrow_asset": hex(&p.borrow_asset),
            "owed": owed.to_string(),
            "health": health,
            "repay_max_partial": repay_max_partial,
        }));
    }
    let lot = l.lots.get(id)?;
    let owed = l.listed.get(&lot.borrow_asset).and_then(|x| x.pool.owed(lot.debt_shares.0).ok()).unwrap_or(0);
    Some(serde_json::json!({
        "id": hex(id),
        "state": "auction",
        "lot": hex(&lot_source(id)),
        "collateral_asset": hex(&lot.collateral_asset),
        "collateral": lot.collateral.0.to_string(),
        "borrow_asset": hex(&lot.borrow_asset),
        "proceeds": lot.proceeds.0.to_string(),
        "owed": owed.to_string(),
        "price": l.auction_price(lot, s.epoch).unwrap_or(0).to_string(),
        "opened_at": if l.v2() { Some(lot.opened_at) } else { None },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use objectvm::lend::{Limits, Listed, Lot, RULES_V2};
    use objectvm::zyn_lend::{params, Pool, WAD};
    use zyn_vm::spec::MicrochainVm;
    use zyn_vm::Fixed;

    const SOL: [u8; 32] = [0x50; 32];
    const ETH: [u8; 32] = [0xE0; 32];

    fn chain() -> ObjectVm {
        let mut s = ObjectVm::genesis(212, objectvm::Limits { max_io: 8 });
        let mut pool = Pool::new(0);
        pool.supply(14 * WAD).unwrap();
        // 0.1 SOL out of 14: less than half a step of the published figures.
        pool.borrow(WAD / 10).unwrap();
        let l = &mut s.market.lend;
        l.listed.insert(SOL, Listed { risk: params::SOL, pool });
        l.listed.insert(ETH, Listed { risk: params::v2::ETH, pool: Pool::new(0) });
        l.prices.insert(SOL, Fixed(150 * WAD));
        l.prices.insert(ETH, Fixed(2_700 * WAD));
        l.insurance.insert(SOL, Fixed(12_345_678_000_000_000));
        s
    }

    fn num(v: &serde_json::Value, k: &str) -> i128 {
        v[k].as_str().unwrap_or_else(|| panic!("{k}")).parse().unwrap()
    }

    /// §160: the three amounts add up, a small loan shows in utilization,
    /// and insurance keeps two figures of its own.
    #[test]
    fn the_figures_add_up_and_a_small_loan_is_not_zero_percent() {
        let v = lend_markets_json(&chain());
        let p = &v["pools"][0];
        assert_eq!(p["asset"], hex(&SOL));
        assert_eq!(num(p, "supplied"), num(p, "cash") + num(p, "borrowed"));
        assert_eq!(num(p, "borrowed"), 0, "the loan is under a step of the pool");
        assert_eq!(num(p, "utilization"), WAD / 100, "0.7% shows as 1%, not 0%");
        assert!(num(p, "borrow_rate") > 0 && num(p, "supply_rate") > 0);
        assert_eq!(num(p, "insurance"), 12_000_000_000_000_000);
        assert_eq!(v["rules"], 0);
        assert!(v["auction"].is_null() && v["pairs"].is_null());
    }

    /// §160: what the app reads under rules v2.
    #[test]
    fn rules_v2_publish_what_the_app_needs() {
        let mut s = chain();
        let l = &mut s.market.lend;
        l.guarded = true;
        l.rules = RULES_V2;
        l.time = 10_000;
        l.priced_at.insert(SOL, 9_900);
        l.priced_at.insert(ETH, 9_600);
        l.pending.insert(ETH, Fixed(2_000 * WAD));
        l.held_posts.insert(ETH, 2);
        l.limits.insert(SOL, Limits { collateral_cap: Fixed(100 * WAD), borrow_cap: Fixed(50 * WAD), min_borrow_usd: Fixed(WAD) });
        l.lots.insert(
            [7; 32],
            Lot {
                owner: [9; 32],
                collateral_asset: ETH,
                collateral: Fixed(WAD),
                borrow_asset: SOL,
                debt_shares: Fixed(WAD / 20),
                rate: Fixed(18 * WAD),
                opened: 5,
                proceeds: Fixed::ZERO,
                opened_at: 9_880,
                tab: Fixed(WAD / 20),
                written_off: Fixed::ZERO,
                insured: Fixed::ZERO,
                backstop: None,
            },
        );
        let v = lend_markets_json(&s);
        assert_eq!(v["rules"], 2);
        assert_eq!(v["price_max_age_secs"], 300);
        assert_eq!(v["auction"]["step_secs"], 60);
        assert_eq!(v["auction"]["backstop_after_secs"], 2_220);
        assert_eq!(v["depth_limit_bps"], 2_500);
        assert_eq!(v["curve"]["slope1"], (8 * WAD / 100).to_string());
        assert_eq!(v["prices_fresh"], false, "ETH is held, and 400 s old");
        let (sol, eth) = (&v["pools"][0], &v["pools"][1]);
        assert_eq!((eth["price_fresh"].as_bool(), eth["price_held"].as_bool(), eth["held_posts"].as_u64()), (Some(false), Some(true), Some(2)));
        assert_eq!(eth["held_price"], (2_000 * WAD).to_string());
        assert_eq!((sol["price_at"].as_u64(), sol["price_fresh"].as_bool()), (Some(9_900), Some(true)));
        assert_eq!(sol["withdraw_paused"], true, "ETH, collateral for SOL, is held");
        assert_eq!(num(sol, "collateral_room"), 100 * WAD);
        let lot = &v["lots"][0];
        // Two minutes in: 100% of the rate.
        assert_eq!(num(lot, "price"), 18 * WAD);
        assert_eq!((lot["floor_at"].as_u64(), lot["backstop_at"].as_u64()), (Some(9_880 + 1_020), Some(9_880 + 2_220)));
        // The smallest bid: 1% of 0.05 SOL beats $1 of SOL.
        assert_eq!(num(lot, "min_bid"), WAD / 2_000);
        assert_eq!(lot["bids_full"], false);
    }
}
