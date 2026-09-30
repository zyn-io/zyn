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
/// cannot be read off a total. Amounts round to 1% of the pool's size (three
/// figures of what is supplied); utilization and the rates come from the
/// rounded figures, not the exact ones, so they cannot be used to undo the
/// rounding. The share count is replaced by the share price, which shows
/// interest and nothing about amounts. No count of positions is published.
pub fn lend_markets_json(s: &ObjectVm) -> serde_json::Value {
    use objectvm::lend::{lend_asset, lot_source, Lend};
    use objectvm::zyn_lend::{CURVE, WAD};
    let l = &s.market.lend;
    let pools: Vec<serde_json::Value> = l
        .listed
        .iter()
        .map(|(a, x)| {
            let p = &x.pool;
            let supplied = p.value().unwrap_or(0);
            // Two significant figures of the pool's size, and borrowed and
            // cash to the same step: one supply or loan moves the published
            // figures only when it is a real share of the pool.
            let step = sig_step(supplied, 2);
            let (sq, bq, cq) = (quantize(supplied, step), quantize(p.debt().unwrap_or(0), step), quantize(p.cash, step));
            let util = if sq > 0 { objectvm::zyn_lend::math::mul_div(bq, WAD, sq).unwrap_or(0).min(WAD) } else { 0 };
            // Utilization to the nearest percent, then the curve.
            let util = quantize(util, WAD / 100);
            let borrow_rate = CURVE.rate(util).unwrap_or(0);
            let gross = objectvm::zyn_lend::math::wmul(borrow_rate, util).unwrap_or(0);
            let supply_rate = gross - objectvm::zyn_lend::math::bps(gross, CURVE.reserve_bps).unwrap_or(0);
            // A share's price and the borrow index move with exact interest,
            // which gives exact utilization away; to six decimals they move
            // only as interest adds up.
            let share_price = if p.supply_shares > 0 { objectvm::zyn_lend::math::mul_div(supplied, WAD, p.supply_shares).unwrap_or(WAD) } else { WAD };
            let share_price = quantize(share_price, WAD / 1_000_000);
            let limits = l.limits.get(a);
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
                "insurance": l.insurance.get(a).map(|x| quantize(x.0, step).to_string()).unwrap_or_else(|| "0".into()),
                "price_held": l.pending.contains_key(a),
            })
        })
        .collect();
    let prices: serde_json::Map<String, serde_json::Value> =
        l.prices.iter().map(|(a, p)| (hex(a), serde_json::Value::String(p.0.to_string()))).collect();
    let lots: Vec<serde_json::Value> = l
        .lots
        .iter()
        .map(|(pos, lot)| {
            let owed = l.listed.get(&lot.borrow_asset).and_then(|x| x.pool.owed(lot.debt_shares.0).ok()).unwrap_or(0);
            serde_json::json!({
                "lot": hex(&lot_source(pos)),
                "collateral_asset": hex(&lot.collateral_asset),
                "collateral": lot.collateral.0.to_string(),
                "borrow_asset": hex(&lot.borrow_asset),
                "owed": owed.to_string(),
                "proceeds": lot.proceeds.0.to_string(),
                "price": Lend::lot_price(lot, s.epoch).unwrap_or(0).to_string(),
                "opened": lot.opened,
            })
        })
        .collect();
    serde_json::json!({
        "epoch": s.epoch,
        "time": l.time,
        "prices": prices,
        "prices_fresh": !l.guarded || s.epoch <= l.priced_epoch + objectvm::lend::MAX_PRICE_AGE_EPOCHS,
        "guarded": l.guarded,
        "pools": pools,
        "lots": lots,
    })
}

/// One position, or the auction it became, for its owner.
pub fn lend_position_json(s: &ObjectVm, id: &[u8; 32]) -> Option<serde_json::Value> {
    use objectvm::lend::lot_source;
    let l = &s.market.lend;
    if let Some(p) = l.positions.get(id) {
        let pool = &l.listed.get(&p.borrow_asset)?.pool;
        let owed = pool.owed(p.debt_shares.0).ok()?;
        let risk = l.listed.get(&p.collateral_asset)?.risk;
        let price = |a: &[u8; 32]| l.prices.get(a).map(|x| x.0);
        let health = match (price(&p.collateral_asset), price(&p.borrow_asset)) {
            (Some(pc), Some(pb)) => {
                let cv = objectvm::zyn_lend::position::value(p.collateral.0, pc).ok()?;
                let dv = objectvm::zyn_lend::position::debt_value(owed, pb).ok()?;
                objectvm::zyn_lend::position::health(cv, dv, &risk).ok()?.map(|h| h.to_string())
            }
            _ => None,
        };
        return Some(serde_json::json!({
            "id": hex(id),
            "state": "open",
            "collateral_asset": hex(&p.collateral_asset),
            "collateral": p.collateral.0.to_string(),
            "borrow_asset": hex(&p.borrow_asset),
            "owed": owed.to_string(),
            "health": health,
        }));
    }
    let lot = l.lots.get(id)?;
    Some(serde_json::json!({
        "id": hex(id),
        "state": "auction",
        "lot": hex(&lot_source(id)),
        "collateral_asset": hex(&lot.collateral_asset),
        "collateral": lot.collateral.0.to_string(),
        "borrow_asset": hex(&lot.borrow_asset),
        "proceeds": lot.proceeds.0.to_string(),
    }))
}
