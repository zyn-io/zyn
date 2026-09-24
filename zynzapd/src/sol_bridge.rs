//! SOL.zy's bridge (DECISIONS §146): what counts as a deposit into the Solana
//! vault, and the units on either side.
//!
//! A deposit is a finalized, successful transaction that moves native SOL into
//! the vault with one `zyn:<commitment>` memo — the commitment the depositor's
//! Nap made from their Zyn account and a secret. It names no Zyn account; the
//! ledger keeps it waiting under the commitment until that account claims it.
//! A transfer into the vault without the memo is a top-up and credits nobody.
//!
//! Lamports have 9 decimals and Zyn amounts 18, so one lamport is `10^9` in
//! `Fixed` — exact both ways, and an exit that is not a whole number of
//! lamports is refused rather than rounded.

use serde_json::Value;
use zyn_vm::commit::{Encoder, Hash};
use zyn_vm::Fixed;

use crate::burn_watch::memo_commitment;

/// The system program, whose `transfer` moves native SOL.
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
const MEMO_PROGRAMS: [&str; 2] = ["MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr", "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo"];
/// One lamport in `Fixed`'s units.
pub const LAMPORT: i128 = 1_000_000_000;
/// The network fee one settlement transaction costs, taken from its first
/// payout so the vault never pays out more than Zyn burned.
pub const NETWORK_FEE: u64 = 5_000;

/// The ledger's reference for a Solana transaction: from its signature, so
/// anyone holding the signature can find it.
pub fn tx_ref(signature: &str) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.solana-tx.v1").bytes(signature.as_bytes());
    e.leaf()
}

pub fn fixed_of(lamports: u64) -> Fixed {
    Fixed(lamports as i128 * LAMPORT)
}

/// Whole lamports, or `None` for an amount Solana cannot pay exactly.
pub fn lamports_of(amount: Fixed) -> Option<u64> {
    (amount.0 > 0 && amount.0 % LAMPORT == 0).then(|| u64::try_from(amount.0 / LAMPORT).ok()).flatten()
}

/// A deposit into `vault` in a `jsonParsed` transaction: every system transfer
/// to it, summed, and its one `zyn:` memo.
pub fn decode_deposit(tx: &Value, vault: &str) -> Option<(u64, Hash)> {
    if !tx.pointer("/meta/err").is_none_or(Value::is_null) {
        return None;
    }
    let mut ixs: Vec<&Value> = tx.pointer("/transaction/message/instructions").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    for set in tx.pointer("/meta/innerInstructions").and_then(Value::as_array).into_iter().flatten() {
        ixs.extend(set.get("instructions").and_then(Value::as_array).into_iter().flatten());
    }
    let mut lamports: u64 = 0;
    let mut commitment: Option<Hash> = None;
    for ix in ixs {
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
        if program != SYSTEM_PROGRAM {
            continue;
        }
        let Some(p) = ix.get("parsed") else { continue };
        if p.get("type").and_then(Value::as_str) != Some("transfer") || p.pointer("/info/destination").and_then(Value::as_str) != Some(vault) {
            continue;
        }
        lamports = lamports.checked_add(p.pointer("/info/lamports").and_then(Value::as_u64)?)?;
    }
    (lamports > 0).then_some(())?;
    Some((lamports, commitment?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const VAULT: &str = "7KacSYXuVhKSf8dpXcLY3qFZQYiZg2m6vKh4m1HxKZmA";

    fn tx(ixs: Value) -> Value {
        json!({ "meta": { "err": null, "innerInstructions": [] }, "transaction": { "message": { "instructions": ixs } } })
    }
    fn transfer(to: &str, lamports: u64) -> Value {
        json!({ "program": "system", "programId": SYSTEM_PROGRAM, "parsed": { "type": "transfer", "info": { "source": "x", "destination": to, "lamports": lamports } } })
    }
    fn memo(t: &str) -> Value {
        json!({ "program": "spl-memo", "programId": MEMO_PROGRAMS[0], "parsed": t })
    }

    #[test]
    fn a_memoed_transfer_to_the_vault_is_a_deposit_and_nothing_else_is() {
        let c = format!("zyn:{}", "ab".repeat(32));
        assert_eq!(decode_deposit(&tx(json!([transfer(VAULT, 250_000_000), memo(&c)])), VAULT), Some((250_000_000, [0xab; 32])));
        assert_eq!(decode_deposit(&tx(json!([transfer(VAULT, 1), transfer(VAULT, 2), memo(&c)])), VAULT).unwrap().0, 3);
        assert_eq!(decode_deposit(&tx(json!([transfer(VAULT, 5)])), VAULT), None, "a top-up credits nobody");
        assert_eq!(decode_deposit(&tx(json!([transfer("elsewhere", 5), memo(&c)])), VAULT), None);
        let mut failed = tx(json!([transfer(VAULT, 5), memo(&c)]));
        failed["meta"]["err"] = json!({"InstructionError": [0, "x"]});
        assert_eq!(decode_deposit(&failed, VAULT), None);
    }

    #[test]
    fn lamports_are_exact_both_ways() {
        assert_eq!(fixed_of(1), Fixed(LAMPORT));
        assert_eq!(lamports_of(fixed_of(123_456_789)), Some(123_456_789));
        assert_eq!(lamports_of(Fixed(LAMPORT + 1)), None, "not rounded");
        assert_eq!(lamports_of(Fixed(0)), None);
        assert_eq!(fixed_of(1_000_000_000), Fixed::whole(1), "one SOL is one SOL.zy");
    }
}
