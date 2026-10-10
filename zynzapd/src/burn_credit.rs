//! Burns become sale credit (DECISIONS §182): what a burn is worth, and how
//! the price behind it is read.
//!
//! A holder burns ZCAVE on Solana (`burnChecked` with a `zyn:<commitment>`
//! memo, as §145) or sends REGI to `0x…dEaD` on Arc with the commitment
//! appended to the transfer's calldata. The chain's own transaction is the
//! proof; the operator's watchers read it and post `PostCredit`: the burn's
//! id, the commitment, and what was burned valued in ZEC — the token's USD
//! price over ZEC's — out of the sale's credit budget.
//!
//! Prices are read every pass and a burn is valued at the **lower median of
//! the last hour's readings** (at least ten), so a moment's pump of a thin
//! pool buys nothing extra, and an even count never favours the burner.
//!
//! Everything here is public on its chain, so anyone can recompute it: the
//! burn ids, the amounts, and the published price sources.

use serde_json::Value;
use zyn_vm::commit::{Encoder, Hash};

/// The window a burn's price is taken over.
pub const WINDOW_SECS: u64 = 3_600;
/// Readings a price needs before any burn is valued: five minutes' worth at
/// the watchers' 30-second poll.
pub const MIN_READINGS: usize = 10;
/// WAD: the chain's 18 decimal places.
pub const WAD: u128 = 1_000_000_000_000_000_000;

/// The burn address on EVM chains. A transfer there is a burn: nobody holds
/// its key.
pub const EVM_DEAD: &str = "000000000000000000000000000000000000dead";
/// `transfer(address,uint256)`.
pub const TRANSFER_SELECTOR: &str = "a9059cbb";
/// `Transfer(address,address,uint256)`.
pub const TRANSFER_TOPIC: &str = "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

/// The id an EVM burn is recorded under on Zyn: from the chain, the
/// transaction and the log, so anyone holding the transaction can find it.
pub fn evm_burn_id(chain_id: u64, tx_hash: &[u8; 32], log_index: u32) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.evm-burn.v1").u64(chain_id).bytes(tx_hash).u32(log_index);
    e.leaf()
}

/// What `amount` base units of a token with `decimals` places are worth in
/// ZEC (the chain's WAD), at `zec_per_token` (WAD per whole token), rounded
/// down. None on overflow or for nothing.
pub fn credit_for(amount: u128, decimals: u32, zec_per_token: u128) -> Option<i128> {
    if decimals > 30 {
        return None;
    }
    // amount × price / 10^decimals, split so 10^30 × 10^18 never overflows.
    let scale = 10u128.checked_pow(decimals)?;
    let whole = amount / scale;
    let part = amount % scale;
    let v = whole.checked_mul(zec_per_token)?.checked_add(part.checked_mul(zec_per_token)? / scale)?;
    i128::try_from(v).ok().filter(|v| *v > 0)
}

/// A price as WAD, rounded down; None for nothing usable.
pub fn wad(x: f64) -> Option<u128> {
    if !x.is_finite() || x <= 0.0 || x > 1e20 {
        return None;
    }
    let v = (x * 1e18).floor();
    (v >= 1.0).then_some(v as u128)
}

/// The price a burn is valued at: the lower median of the readings.
pub fn lower_median(readings: &[u128]) -> Option<u128> {
    if readings.len() < MIN_READINGS {
        return None;
    }
    let mut v = readings.to_vec();
    v.sort_unstable();
    Some(v[(v.len() - 1) / 2])
}

/// Record `price` (WAD of ZEC per whole token) at `now` in `path`, keep the
/// last hour, and return the median a burn is valued at now — None until
/// there are enough readings.
pub fn record(path: &std::path::Path, now: u64, price: u128) -> Result<Option<u128>, String> {
    let mut readings: Vec<(u64, String)> = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    readings.retain(|(t, _)| now.saturating_sub(*t) <= WINDOW_SECS);
    readings.push((now, price.to_string()));
    std::fs::write(path, serde_json::to_vec(&readings).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))?;
    let prices: Vec<u128> = readings.iter().filter_map(|(_, p)| p.parse().ok()).collect();
    Ok(lower_median(&prices))
}

// ---------------------------------------------------------------------------
// Prices.

/// A pump.fun bonding curve's price, in ZEC (WAD) per whole token, from its
/// account data: the virtual reserves, which are what its trades price at
/// (the real reserves are not — a new curve holds almost no quote). Also
/// whether the curve has completed, after which it no longer trades.
pub fn pump_curve_price(data: &[u8], token_decimals: u32, quote_decimals: u32) -> Option<(u128, bool)> {
    if data.len() < 49 || token_decimals > 30 || quote_decimals > 18 {
        return None;
    }
    let word = |i: usize| u64::from_le_bytes(data[8 + i * 8..16 + i * 8].try_into().unwrap()) as u128;
    let (virtual_token, virtual_quote) = (word(0), word(1));
    if virtual_token == 0 || virtual_quote == 0 {
        return None;
    }
    // quote/10^q ÷ token/10^t, in WAD.
    let num = virtual_quote.checked_mul(10u128.checked_pow(18 - quote_decimals)?)?.checked_mul(10u128.checked_pow(token_decimals)?)?;
    Some((num / virtual_token, data[48] != 0))
}

/// A Uniswap v4 pool's price of `token` in its quote, from `slot0`'s
/// `sqrtPriceX96`: whole quote per whole token. `token_is_0` when the token
/// sorts first in the pool (`currency0`).
pub fn v4_price(sqrt_price_x96: &[u8; 32], token_is_0: bool, token_decimals: i32, quote_decimals: i32) -> Option<f64> {
    // The low 160 bits.
    let mut x = 0f64;
    for b in &sqrt_price_x96[12..] {
        x = x * 256.0 + *b as f64;
    }
    let r = x / 2f64.powi(96);
    let raw = r * r; // currency1 base units per currency0 base unit
    if !raw.is_finite() || raw <= 0.0 {
        return None;
    }
    let p = if token_is_0 { raw * 10f64.powi(token_decimals - quote_decimals) } else { 10f64.powi(token_decimals - quote_decimals) / raw };
    (p.is_finite() && p > 0.0).then_some(p)
}

// ---------------------------------------------------------------------------
// EVM burns.

/// One EVM burn the watcher can post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvmBurn {
    pub tx_hash: [u8; 32],
    pub log_index: u32,
    pub amount: u128,
    pub commitment: Hash,
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()).collect()
}

/// A burn from one `Transfer` log and its transaction: the log is the token's
/// own, to `0x…dEaD`; the transaction calls the token's `transfer` to
/// `0x…dEaD` for the same amount, with exactly one 32-byte word appended —
/// the commitment. Anything else (a router, a batch, no commitment) is not a
/// burn for credit.
pub fn decode_evm_burn(log: &Value, tx: &Value, token: &str) -> Option<EvmBurn> {
    let lower = |v: &Value| v.as_str().map(|s| s.to_ascii_lowercase());
    let token = token.to_ascii_lowercase();
    if lower(&log["address"])? != token || lower(&tx["to"])? != token {
        return None;
    }
    if log["removed"].as_bool() == Some(true) {
        return None;
    }
    let topics = log["topics"].as_array()?;
    if topics.len() != 3 || lower(&topics[0])?.trim_start_matches("0x") != TRANSFER_TOPIC {
        return None;
    }
    if !lower(&topics[2])?.ends_with(EVM_DEAD) {
        return None;
    }
    let amount_log = unhex(log["data"].as_str()?)?;
    let input = unhex(tx["input"].as_str()?)?;
    if input.len() != 4 + 32 + 32 + 32 || hex(&input[..4]) != TRANSFER_SELECTOR {
        return None;
    }
    if hex(&input[4 + 12..4 + 32]) != EVM_DEAD || input[4..4 + 12].iter().any(|b| *b != 0) {
        return None;
    }
    if amount_log.len() != 32 || input[36..68] != amount_log[..] {
        return None;
    }
    if amount_log[..16].iter().any(|b| *b != 0) {
        return None;
    }
    let amount = u128::from_be_bytes(amount_log[16..32].try_into().ok()?);
    let commitment: Hash = input[68..100].try_into().ok()?;
    if amount == 0 || commitment == [0; 32] {
        return None;
    }
    let tx_hash: [u8; 32] = unhex(tx["hash"].as_str()?)?.try_into().ok()?;
    if lower(&log["transactionHash"])? != lower(&tx["hash"])? {
        return None;
    }
    let log_index = u32::try_from(u64::from_str_radix(log["logIndex"].as_str()?.trim_start_matches("0x"), 16).ok()?).ok()?;
    Some(EvmBurn { tx_hash, log_index, amount, commitment })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REGI: &str = "0x93d5b8c53ee763c2c4522bf0d958ce51af4360ae";
    const TX: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const C: &str = "5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e";

    fn word(n: u128) -> String {
        format!("{n:064x}")
    }

    fn pair(amount: u128, input_tail: &str, to: &str) -> (Value, Value) {
        let log = json!({
            "address": REGI, "removed": false, "transactionHash": TX, "logIndex": "0x3",
            "topics": [format!("0x{TRANSFER_TOPIC}"), format!("0x{}", "0".repeat(24) + "abababababababababababababababababababab"), format!("0x{}{}", "0".repeat(24), to)],
            "data": format!("0x{}", word(amount)),
        });
        let tx = json!({ "hash": TX, "to": REGI, "input": format!("0x{TRANSFER_SELECTOR}{}{}{input_tail}", "0".repeat(24) + to, word(amount)) });
        (log, tx)
    }

    #[test]
    fn a_transfer_to_dead_with_a_commitment_is_a_burn() {
        let (log, tx) = pair(250_000 * 10u128.pow(18), C, EVM_DEAD);
        let b = decode_evm_burn(&log, &tx, REGI).unwrap();
        assert_eq!((b.amount, b.log_index, hex(&b.commitment)), (250_000 * 10u128.pow(18), 3, C.to_string()));
        // Case does not matter for addresses.
        assert!(decode_evm_burn(&log, &tx, &REGI.to_ascii_uppercase().replace("0X", "0x")).is_some());
    }

    #[test]
    fn anything_else_is_not() {
        // No commitment, or more than one word after the call.
        assert!(decode_evm_burn(&pair(5, "", EVM_DEAD).0, &pair(5, "", EVM_DEAD).1, REGI).is_none());
        let (log, tx) = pair(5, &format!("{C}{C}"), EVM_DEAD);
        assert!(decode_evm_burn(&log, &tx, REGI).is_none());
        // To anyone but 0x…dEaD.
        let other = "abababababababababababababababababababab";
        let (log, tx) = pair(5, C, other);
        assert!(decode_evm_burn(&log, &tx, REGI).is_none());
        // Another token's log, or a call through something else.
        let (mut log, tx) = pair(5, C, EVM_DEAD);
        log["address"] = json!("0x6440f144b7e50d6a8439336510312d2f54beb01d");
        assert!(decode_evm_burn(&log, &tx, REGI).is_none());
        let (log, mut tx) = pair(5, C, EVM_DEAD);
        tx["to"] = json!("0x6440f144b7e50d6a8439336510312d2f54beb01d");
        assert!(decode_evm_burn(&log, &tx, REGI).is_none());
        // A log for a different amount than the call names.
        let (mut log, tx) = pair(5, C, EVM_DEAD);
        log["data"] = json!(format!("0x{}", word(6)));
        assert!(decode_evm_burn(&log, &tx, REGI).is_none());
        // A zero commitment.
        let (log, tx) = pair(5, &"0".repeat(64), EVM_DEAD);
        assert!(decode_evm_burn(&log, &tx, REGI).is_none());
        // A log a reorg removed.
        let (mut log, tx) = pair(5, C, EVM_DEAD);
        log["removed"] = json!(true);
        assert!(decode_evm_burn(&log, &tx, REGI).is_none());
    }

    #[test]
    fn credit_is_what_was_burned_at_the_price_rounded_down() {
        // 3,470,000 ZCAVE (6 places) at 2.878e-9 ZEC each ≈ 0.009987 ZEC.
        let price = wad(2.878e-9).unwrap();
        let c = credit_for(3_470_000 * 10u128.pow(6), 6, price).unwrap();
        assert_eq!(c, 3_470_000 * price as i128);
        // 18 places, and a fraction of a whole token.
        assert_eq!(credit_for(15 * 10u128.pow(17), 18, 2 * WAD).unwrap(), 3 * WAD as i128);
        assert_eq!(credit_for(0, 18, WAD), None);
        assert_eq!(credit_for(1, 18, 1), None, "less than one unit of credit is nothing");
    }

    #[test]
    fn the_lower_median_of_an_hour_never_favours_the_burner() {
        assert_eq!(lower_median(&[5; 9]), None, "too few readings");
        let mut r = vec![10u128; 5];
        r.extend([20u128; 5]);
        assert_eq!(lower_median(&r), Some(10), "even count: the lower one");
        r.push(1_000_000);
        assert_eq!(lower_median(&r), Some(20), "one pump moves nothing");
    }

    /// The ZCAVE curve as Solana served it on 10 Oct 2026: 1,007,685,028.6
    /// virtual tokens against 2.9010928 virtual ZEC — 2.878e-9 ZEC each, as
    /// DexScreener showed.
    #[test]
    fn a_pump_curve_prices_at_its_virtual_reserves() {
        let mut data = vec![0u8; 166];
        data[..8].copy_from_slice(&hex_bytes("17b7f83760d8ac60"));
        data[8..16].copy_from_slice(&1_007_685_028_645_276u64.to_le_bytes());
        data[16..24].copy_from_slice(&290_109_280u64.to_le_bytes());
        data[24..32].copy_from_slice(&727_785_028_645_276u64.to_le_bytes());
        data[32..40].copy_from_slice(&17_659_415u64.to_le_bytes());
        let (p, complete) = pump_curve_price(&data, 6, 8).unwrap();
        assert!(!complete);
        assert_eq!(p, 2_878_967_849, "2.879e-9 ZEC in WAD");
        data[48] = 1;
        assert!(pump_curve_price(&data, 6, 8).unwrap().1, "a completed curve says so");
    }

    /// REGI/USDC on Arc as the PoolManager answered on 10 Oct 2026 (USDC
    /// sorts first): about $0.0000168 a REGI, as DexScreener showed.
    #[test]
    fn a_v4_pool_prices_its_token() {
        let sqrt: u128 = 19_355_183_266_636_175_460_765_404_804_463_425_430;
        let mut w = [0u8; 32];
        w[16..].copy_from_slice(&sqrt.to_be_bytes());
        let usd = v4_price(&w, false, 18, 6).unwrap();
        assert!((usd - 0.000016756).abs() < 1e-9, "{usd}");
    }

    fn hex_bytes(s: &str) -> Vec<u8> {
        unhex(s).unwrap()
    }
}
