//! Gathering the three things a draw needs.
//!
//! `allocation::run` is a pure function over a ticket ledger and some evidence.
//! Neither existed as data: the tickets were Cave payments the scanner had
//! seen, and the evidence was a drand round and a Zcash block that nobody had
//! fetched. This is the fetching (§105).
//!
//! # Why the evidence is fetched and not trusted
//!
//! Both halves are verified where they land. The drand beacon is checked
//! in-process against the pinned Quicknet public key — `cave-randomness` will
//! not take a signature it has not verified, and neither does this. The Zcash
//! block hash is asked of a node rather than derived from a header, because
//! re-deriving it here would be a second implementation of consensus that could
//! disagree with the first.
//!
//! What this module cannot establish is that the node it asked is honest about
//! which block is canonical at that height. That is the caller's obligation and
//! the crate says so; here it means asking a node you run.

use cave_randomness::{
    verify_quicknet_beacon, AllocationPolicy, DrandBeacon, Hash, RevealEvidence, Ticket,
    TicketEvidence,
};
use zyn_custody::memo::{CavePaymentMemo, CavePaymentPurpose};

use crate::tickets::{Deposit, Ledger, NotATicket};

/// A Cave payment as the scanner saw it, with its position in the chain.
///
/// Mirrors `zyn_custody::shielded::CavePaymentSighting` rather than importing
/// it, so this module can be tested without a scanner and so a change to the
/// scanner's shape is a compile error here rather than a silent mismatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sighting {
    pub txid: Hash,
    pub height: u64,
    pub tx_index: u32,
    pub output_index: u32,
    pub amount_zatoshi: u64,
    pub payment: CavePaymentMemo,
}

/// Build the ticket ledger from what the scanner saw.
///
/// Returns the ledger and everything it refused, because a refusal is a person
/// whose money arrived and who is owed an answer — dropping them silently is
/// how a sale acquires a support queue nobody can reconstruct.
pub fn ledger_from(
    policy: &AllocationPolicy,
    terms_digest: Hash,
    close: u32,
    min_confirmations: u32,
    tip: u32,
    sightings: &[Sighting],
    issued_at_drand_round: u64,
) -> (Ledger, Vec<(Sighting, NotATicket)>) {
    let mut ledger = Ledger::new();
    let mut refused = Vec::new();
    for s in sightings {
        // Only entries. A claim payment is for an allocation that already
        // exists and is not a ticket in the draw.
        if s.payment.purpose != CavePaymentPurpose::Entry {
            continue;
        }
        // And only entries for *this* sale. A payment naming another terms
        // digest is someone paying into a different promise.
        if s.payment.reference != terms_digest {
            continue;
        }
        let d = Deposit {
            txid: s.txid,
            output_index: s.output_index,
            amount_zatoshi: s.amount_zatoshi,
            confirmed_height: s.height as u32,
            tx_index: s.tx_index,
            recipient: s.payment.recipient,
            issued_at_drand_round,
        };
        match ledger.admit(policy, close, min_confirmations, tip, &d) {
            Ok(_) => {}
            Err(why) => refused.push((*s, why)),
        }
    }
    (ledger, refused)
}

/// Tickets with no per-ticket beacons — the concealed case, where quantity and
/// identity both come from the collection-wide reveal.
pub fn evidence_for(tickets: &[Ticket]) -> Vec<TicketEvidence> {
    tickets
        .iter()
        .map(|t| TicketEvidence {
            ticket: t.clone(),
            quantity_beacon: None,
            identity_beacon: None,
        })
        .collect()
}

/// Fetch a drand round and verify it before returning it.
///
/// `get` is the transport, so this is testable without a network and so the
/// caller chooses the endpoint. The verification is not optional: an
/// unverified beacon is a number someone sent you.
pub fn beacon(
    round: u64,
    chain_hash: Hash,
    get: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<DrandBeacon, String> {
    // The classic endpoint, `/<chain-hash>/public/<round>`. The v2 path shape
    // was a guess and returned 500 for every round; the stubs in this module
    // passed anyway, which is exactly why there is a test against the live
    // service (§105.4).
    let url = format!("https://api.drand.sh/{}/public/{}", hex(&chain_hash), round);
    let body = get(&url)?;
    let v: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("drand reply is not JSON: {e}"))?;
    let sig = v
        .get("signature")
        .and_then(serde_json::Value::as_str)
        .ok_or("drand reply carries no signature")?;
    let got = v
        .get("round")
        .and_then(serde_json::Value::as_u64)
        .ok_or("drand reply carries no round")?;
    // Asked for one round and handed another is not a retryable hiccup: the
    // round is what the reveal committed to.
    if got != round {
        return Err(format!("asked drand for round {round} and got {got}"));
    }
    let beacon = DrandBeacon {
        chain_hash,
        round,
        signature: unhex(sig).ok_or("drand signature is not hex")?,
    };
    verify_quicknet_beacon(&beacon).map_err(|e| format!("drand beacon does not verify: {e}"))?;
    Ok(beacon)
}

/// The complete reveal: a verified beacon and the block at the reveal height.
pub fn reveal(
    beacon: DrandBeacon,
    zcash_height: u32,
    zcash_block_hash: Hash,
) -> Result<RevealEvidence, String> {
    if zcash_block_hash == [0u8; 32] {
        return Err("the reveal block hash is zero".into());
    }
    Ok(RevealEvidence {
        beacon,
        zcash_height,
        zcash_block_hash,
    })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cave_randomness::{
        IdentityPolicy, QuantityPolicy, RandomnessPlan, QUICKNET_CHAIN_HASH,
    };

    const TERMS: Hash = [0x7c; 32];

    fn policy() -> AllocationPolicy {
        AllocationPolicy {
            collection_id: [2; 32],
            supply: 4_444,
            entry_price_zatoshi: 1_000_000,
            deposit_close_zcash_height: 3_500_000,
            quantity: QuantityPolicy::Fixed { items: 1 },
            identity: IdentityPolicy::Sequential,
            manifest_root: [3; 32],
            randomness: RandomnessPlan {
                drand_chain_hash: QUICKNET_CHAIN_HASH,
                ticket_round_delay: 10,
                reveal_drand_round: None,
                reveal_zcash_height: None,
                announced_close: None,
            },
        }
    }

    fn sighting(txid: u8, who: u8, purpose: CavePaymentPurpose, reference: Hash) -> Sighting {
        Sighting {
            txid: [txid; 32],
            height: 3_499_000,
            tx_index: 0,
            output_index: 0,
            amount_zatoshi: 1_000_000,
            payment: CavePaymentMemo {
                purpose,
                reference,
                recipient: [who; 32],
            },
        }
    }

    #[test]
    fn only_entries_for_this_sale_become_tickets() {
        let p = policy();
        let s = [
            sighting(1, 0xa1, CavePaymentPurpose::Entry, TERMS),
            // A claim: for an allocation that already exists, not a ticket.
            sighting(2, 0xb2, CavePaymentPurpose::Claim, TERMS),
            // An entry into some other sale's promise.
            sighting(3, 0xc3, CavePaymentPurpose::Entry, [0x99; 32]),
        ];
        let (l, refused) = ledger_from(&p, TERMS, 3_500_000, 6, 3_500_010, &s, 5_000_000);
        assert_eq!(l.len(), 1, "one ticket, from the one entry that named this sale");
        assert!(refused.is_empty(), "the other two are not refusals, they are not ours");
        assert_eq!(l.tickets()[0].recipient, [0xa1; 32]);
    }

    /// A payment that arrived and cannot be a ticket is reported, not dropped.
    /// Someone's money is in the vault and they are owed an answer.
    #[test]
    fn a_payment_that_cannot_be_a_ticket_comes_back_with_its_reason() {
        let p = policy();
        let mut short = sighting(1, 0xa1, CavePaymentPurpose::Entry, TERMS);
        short.amount_zatoshi -= 1;
        let mut late = sighting(2, 0xb2, CavePaymentPurpose::Entry, TERMS);
        late.height = 3_500_001;
        let (l, refused) = ledger_from(&p, TERMS, 3_500_000, 6, 3_500_010, &[short, late], 5_000_000);
        assert!(l.is_empty());
        assert_eq!(refused.len(), 2);
        assert!(matches!(refused[0].1, NotATicket::WrongAmount { .. }));
        assert!(matches!(refused[1].1, NotATicket::AfterClose { .. }));
    }

    #[test]
    fn a_drand_reply_for_the_wrong_round_is_refused() {
        let get = |_: &str| -> Result<Vec<u8>, String> {
            Ok(br#"{"round":999,"signature":"aabb"}"#.to_vec())
        };
        let e = beacon(1000, QUICKNET_CHAIN_HASH, &get).unwrap_err();
        assert!(e.contains("asked drand for round 1000 and got 999"), "{e}");
    }

    /// The signature must verify against the pinned key. A well-formed reply
    /// with a wrong signature is the case that matters: it looks fine.
    #[test]
    fn a_beacon_that_does_not_verify_is_refused() {
        let get = |_: &str| -> Result<Vec<u8>, String> {
            Ok(br#"{"round":1000,"signature":"00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"}"#.to_vec())
        };
        assert!(beacon(1000, QUICKNET_CHAIN_HASH, &get).is_err());
    }

    #[test]
    fn a_transport_failure_and_malformed_json_are_both_errors() {
        let boom = |_: &str| -> Result<Vec<u8>, String> { Err("no route to host".into()) };
        assert!(beacon(1, QUICKNET_CHAIN_HASH, &boom).is_err());
        let junk = |_: &str| -> Result<Vec<u8>, String> { Ok(b"<html>".to_vec()) };
        assert!(beacon(1, QUICKNET_CHAIN_HASH, &junk).is_err());
    }

    #[test]
    fn a_zero_block_hash_is_not_a_reveal() {
        let b = DrandBeacon {
            chain_hash: QUICKNET_CHAIN_HASH,
            round: 1,
            signature: vec![0; 48],
        };
        assert!(reveal(b, 3_500_100, [0u8; 32]).is_err());
    }
}
