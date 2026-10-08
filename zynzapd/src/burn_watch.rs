//! Burn tickets from Solana (DECISIONS §145): what counts as a burn, and how
//! many tickets it is worth.
//!
//! A holder of the promotional token burns it on Solana in a transaction
//! that also carries a memo `zyn:<commitment>` — the commitment Nap made from
//! their Zyn account and a secret. The operator's watcher reads finalized
//! burns of that one mint and posts each as a `PostVoucher`: the burn's id,
//! the commitment, and `burned / per_ticket` tickets (at most 20). The chain
//! records a burn once, so a watcher that runs twice, or two watchers, post
//! nothing twice.
//!
//! Everything here is public on Solana, so anyone can recompute it: the burn
//! id is `H("objectvm.solana-burn.v1", signature)`, and the tickets a burn is
//! worth follow from its amount and the published rate.

use serde_json::Value;
use zyn_vm::commit::{Encoder, Hash};

/// The SPL memo program, v2 and v1.
const MEMO_PROGRAMS: [&str; 2] = ["MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr", "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo"];
/// Tickets one burn can be worth — one `PostVoucher`.
pub const MAX_TICKETS_PER_BURN: u64 = 20;

/// One burn the watcher can post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Burn {
    /// Base units of the mint burned in this transaction.
    pub amount: u64,
    /// From the memo: whose tickets these are, without saying.
    pub commitment: Hash,
}

/// The id a Solana burn is recorded under on Zyn: from its signature alone,
/// so anyone holding the signature can find it.
pub fn burn_id(signature: &str) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.solana-burn.v1").bytes(signature.as_bytes());
    e.leaf()
}

/// Tickets `amount` is worth at `per_ticket` base units each, capped at what
/// one post can carry. Anything past a whole ticket is not refunded — Nap
/// only offers whole-ticket burns.
pub fn tickets_for(amount: u64, per_ticket: u64) -> u32 {
    if per_ticket == 0 {
        return 0;
    }
    (amount / per_ticket).min(MAX_TICKETS_PER_BURN) as u32
}

/// The commitment in a memo of the form `zyn:<64 hex>`. A memo program may
/// prefix nothing or whitespace; anything else is not ours.
pub fn memo_commitment(memo: &str) -> Option<Hash> {
    let rest = memo.trim().strip_prefix("zyn:")?.trim();
    if rest.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(rest.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn instructions(tx: &Value) -> Vec<&Value> {
    let mut all: Vec<&Value> = tx
        .pointer("/transaction/message/instructions")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    if let Some(inner) = tx.pointer("/meta/innerInstructions").and_then(Value::as_array) {
        for set in inner {
            if let Some(ix) = set.get("instructions").and_then(Value::as_array) {
                all.extend(ix.iter());
            }
        }
    }
    all
}

/// The burn in a finalized, successful `jsonParsed` transaction: every burn
/// of `mint` in it (outer or inner, SPL Token or Token-2022), summed, and the
/// one `zyn:` memo it carries. `None` for anything else — a failed
/// transaction, another mint, no memo, two different memos.
pub fn decode_burn(tx: &Value, mint: &str) -> Option<Burn> {
    if !tx.pointer("/meta/err").is_none_or(Value::is_null) {
        return None;
    }
    let mut amount: u64 = 0;
    let mut commitment: Option<Hash> = None;
    for ix in instructions(tx) {
        let program = ix.get("programId").and_then(Value::as_str).unwrap_or("");
        if MEMO_PROGRAMS.contains(&program) {
            if let Some(c) = ix.get("parsed").and_then(Value::as_str).and_then(memo_commitment) {
                if commitment.is_some_and(|x| x != c) {
                    return None;
                }
                commitment = Some(c);
            }
            continue;
        }
        let kind = ix.get("program").and_then(Value::as_str).unwrap_or("");
        if kind != "spl-token" && kind != "spl-token-2022" {
            continue;
        }
        let Some(parsed) = ix.get("parsed") else { continue };
        let ty = parsed.get("type").and_then(Value::as_str).unwrap_or("");
        let info = parsed.get("info");
        if (ty != "burn" && ty != "burnChecked") || info.and_then(|i| i.get("mint")).and_then(Value::as_str) != Some(mint) {
            continue;
        }
        let raw = info
            .and_then(|i| i.get("amount").or_else(|| i.pointer("/tokenAmount/amount")))
            .and_then(Value::as_str)
            .and_then(|a| a.parse::<u64>().ok())?;
        amount = amount.checked_add(raw)?;
    }
    if amount == 0 {
        return None;
    }
    Some(Burn { amount, commitment: commitment? })
}

// ---------------------------------------------------------------------------
// The rate (the user, 8 Oct 2026): a burn ticket costs one ticket price in ZEC,
// in the token at its price against bridged ZEC on Solana. The price is read
// from the token's pool — its two reserves — and a burn is priced at the
// median of the last hour's readings, so a moment's pump of the pool buys no
// extra tickets.

/// Token base units one ticket costs when the pool holds `token_reserve` base
/// units of the token and `zec_reserve` base units of bridged ZEC (with
/// `zec_decimals` places): the ticket's price (`ticket_wad`, ZEC in WAD) at
/// the pool's price, rounded up. None for an empty pool or a price so high a
/// ticket costs less than one base unit.
pub fn per_ticket_at(ticket_wad: u128, token_reserve: u128, zec_reserve: u128, zec_decimals: u32) -> Option<u64> {
    if token_reserve == 0 || zec_reserve == 0 || zec_decimals > 18 {
        return None;
    }
    let zec_wad = zec_reserve.checked_mul(10u128.checked_pow(18 - zec_decimals)?)?;
    let v = ticket_wad.checked_mul(token_reserve)?.div_ceil(zec_wad);
    u64::try_from(v).ok().filter(|v| *v > 0)
}

/// Readings a rate needs before any burn is priced: five minutes' worth at
/// the watcher's 30-second poll.
pub const MIN_READINGS: usize = 10;

/// The rate a burn is priced at: the median of the readings (the upper one of
/// an even count, so the rate never favours the burner).
pub fn median_rate(readings: &[u64]) -> Option<u64> {
    if readings.len() < MIN_READINGS {
        return None;
    }
    let mut v = readings.to_vec();
    v.sort_unstable();
    Some(v[v.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MINT: &str = "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU";
    const C: &str = "5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e";

    fn tx(ixs: Value, err: Value) -> Value {
        json!({ "meta": { "err": err, "innerInstructions": [] }, "transaction": { "message": { "instructions": ixs } } })
    }
    fn burn(ty: &str, mint: &str, amount: &str) -> Value {
        let info = if ty == "burn" {
            json!({ "mint": mint, "amount": amount })
        } else {
            json!({ "mint": mint, "tokenAmount": { "amount": amount, "decimals": 6 } })
        };
        json!({ "program": "spl-token", "programId": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", "parsed": { "type": ty, "info": info } })
    }
    fn memo(text: &str) -> Value {
        json!({ "program": "spl-memo", "programId": MEMO_PROGRAMS[0], "parsed": text })
    }

    #[test]
    fn a_burn_with_a_zyn_memo_is_a_burn_and_anything_else_is_not() {
        let good = tx(json!([burn("burnChecked", MINT, "3000000"), memo(&format!("zyn:{C}"))]), Value::Null);
        assert_eq!(decode_burn(&good, MINT), Some(Burn { amount: 3_000_000, commitment: [0x5e; 32] }));
        let two = tx(json!([burn("burn", MINT, "1000000"), burn("burnChecked", MINT, "500000"), memo(&format!(" zyn:{C} "))]), Value::Null);
        assert_eq!(decode_burn(&two, MINT).unwrap().amount, 1_500_000, "every burn of the mint in one transaction counts");
        assert_eq!(decode_burn(&tx(json!([burn("burnChecked", MINT, "3000000")]), Value::Null), MINT), None, "no memo, no ticket");
        assert_eq!(decode_burn(&tx(json!([burn("burnChecked", "Other111", "3000000"), memo(&format!("zyn:{C}"))]), Value::Null), MINT), None);
        assert_eq!(decode_burn(&tx(json!([burn("burnChecked", MINT, "3000000"), memo(&format!("zyn:{C}"))]), json!({"InstructionError": [0, "x"]})), MINT), None);
        let d = "6e".repeat(32);
        assert_eq!(decode_burn(&tx(json!([burn("burnChecked", MINT, "1"), memo(&format!("zyn:{C}")), memo(&format!("zyn:{d}"))]), Value::Null), MINT), None, "two memos name two burners");
        assert_eq!(decode_burn(&tx(json!([json!({"program":"spl-token","parsed":{"type":"transferChecked","info":{"mint":MINT,"tokenAmount":{"amount":"9"}}}}), memo(&format!("zyn:{C}"))]), Value::Null), MINT), None);
    }

    #[test]
    fn tickets_are_whole_and_capped() {
        assert_eq!(tickets_for(2_999_999, 1_000_000), 2);
        assert_eq!(tickets_for(999, 1_000_000), 0);
        assert_eq!(tickets_for(u64::MAX, 1), 20);
        assert_eq!(tickets_for(5, 0), 0);
        assert_eq!(memo_commitment("zyn:zz"), None);
        assert_ne!(burn_id("a"), burn_id("b"));
    }

    #[test]
    fn a_ticket_costs_its_zec_price_at_the_pool() {
        // 0.01 ZEC a ticket; the pool holds 1,000,000 tokens (6 places) and
        // 10 ZEC (8 places): a token is 0.00001 ZEC, a ticket 1,000 tokens.
        let ticket = 10_000_000_000_000_000u128;
        assert_eq!(per_ticket_at(ticket, 1_000_000 * 1_000_000, 10 * 100_000_000, 8), Some(1_000 * 1_000_000));
        // The price doubles: half the tokens. Rounded up, never down.
        assert_eq!(per_ticket_at(ticket, 1_000_000 * 1_000_000, 20 * 100_000_000, 8), Some(500 * 1_000_000));
        assert_eq!(per_ticket_at(ticket, 3, 7 * 100_000_000, 8), Some(1));
        assert_eq!(per_ticket_at(ticket, 0, 1, 8), None);
        assert_eq!(per_ticket_at(ticket, 1, 0, 8), None);
    }

    #[test]
    fn a_burn_is_priced_at_the_hours_median() {
        assert_eq!(median_rate(&[5; 9]), None, "too few readings: no burn is priced");
        let mut r = vec![1_000; 9];
        r.push(1); // a pumped moment: one ticket for one base unit
        assert_eq!(median_rate(&r), Some(1_000), "one spike does not move the median");
        assert_eq!(median_rate(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]), Some(6), "the upper middle of an even count");
    }
}
