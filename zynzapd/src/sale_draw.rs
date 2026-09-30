//! Collection Z's draw, run over tickets bought on Zyn (DECISIONS §144).
//!
//! The lottery engine (`cave-randomness`) and the published terms are the
//! ones the sale was announced with. What changed is where a ticket comes
//! from: it was a confirmed Zcash deposit to the vault, and it is now a
//! `BuyTickets` on the object ledger. Each Zyn ticket stands in the engine's
//! ticket where a deposit stood:
//!
//! * `deposit_txid` and `recipient` are the Zyn ticket id — unique, derived
//!   only by the buyer, and what the posted draw names;
//! * `tx_index` is its place in the sale's order, so the engine orders
//!   tickets exactly as the chain recorded them;
//! * `confirmed_height` is the Zcash height the sale closed at. The chain
//!   refused tickets after the close, so every ticket it holds was bought
//!   before it — the engine's "not after the close" is true by construction;
//! * `issued_at_drand_round` is the announcement's round. Collection Z's
//!   tickets name no round of their own: quantity and identity both come
//!   from the one reveal after the close, so no buyer learns anything before
//!   every ticket is in.
//!
//! The engine's result becomes the root `PostDraw` commits: one leaf per
//! winning ticket, `draw_leaf(ticket, serials)`, in ticket order. Anyone
//! holding the ticket list (`zyn_tickets`), the announcement, the drand
//! beacon and the Zcash reveal block can run this again and get the same root.

use std::collections::BTreeSet;

use cave_randomness::{AllocationBatch, AllocationPolicy, ManifestEntry, ResolvedClose, RevealEvidence, Ticket, TicketEvidence};
use serde_json::{json, Value};
use zyn_vm::commit::{merkle_root, Hash};

/// One Zyn ticket as `zyn_tickets` lists it: its id and its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZynTicket {
    pub id: Hash,
    pub index: u32,
}

/// What the draw decided.
#[derive(Clone, Debug)]
pub struct Draw {
    pub batch: AllocationBatch,
    /// Every ticket that won something, in ticket order, with its serials.
    pub won: Vec<(Hash, Vec<u32>)>,
    /// The root `PostDraw` commits.
    pub root: Hash,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The engine's tickets for these Zyn tickets.
pub fn engine_tickets(
    policy_digest: Hash,
    entry_price_zatoshi: u64,
    close_zcash_height: u32,
    issued_at_drand_round: u64,
    tickets: &[ZynTicket],
) -> Vec<TicketEvidence> {
    tickets
        .iter()
        .map(|t| {
            let mut ticket = Ticket {
                ticket_id: [0; 32],
                deposit_txid: t.id,
                deposit_output_index: 0,
                deposit_amount_zatoshi: entry_price_zatoshi,
                recipient: t.id,
                confirmed_height: close_zcash_height,
                tx_index: t.index,
                issued_at_drand_round,
                quantity_drand_round: None,
                identity_drand_round: None,
                requested_items: None,
            };
            ticket.ticket_id = cave_randomness::derive_ticket_id(policy_digest, &ticket);
            TicketEvidence { ticket, quantity_beacon: None, identity_beacon: None }
        })
        .collect()
}

/// Run the draw. Tickets must be the sale's whole list, in its order.
pub fn run(
    policy: &AllocationPolicy,
    manifest: &[ManifestEntry],
    tickets: &[ZynTicket],
    close: &ResolvedClose,
    reveal: &RevealEvidence,
    issued_at_drand_round: u64,
) -> Result<Draw, String> {
    if tickets.iter().enumerate().any(|(i, t)| t.index as usize != i) {
        return Err("the ticket list is not the sale's whole list in order".into());
    }
    let digest = policy.digest().map_err(|e| format!("the policy does not validate: {e}"))?;
    let evidence = engine_tickets(digest, policy.entry_price_zatoshi, close.deposit_close_zcash_height, issued_at_drand_round, tickets);
    let run = crate::allocation::run_resolved(policy, manifest, &evidence, Some(reveal), Some(close)).map_err(|e| e.to_string())?;
    let mut won = Vec::new();
    for t in tickets {
        let r = run.batch.receipts.iter().find(|r| r.ticket.deposit_txid == t.id).ok_or("the engine dropped a ticket")?;
        let serials: Vec<u32> = r.items.iter().map(|i| i.global_index).collect();
        if !serials.is_empty() {
            won.push((t.id, serials));
        }
    }
    let leaves: Vec<Hash> = won.iter().map(|(t, s)| objectvm::sale::draw_leaf(t, s)).collect();
    Ok(Draw { root: merkle_root(&leaves), won, batch: run.batch })
}

/// The order the rest sell in once claims end: what the draw handed nobody,
/// in the engine's revealed order, then what winners left unclaimed, in draw
/// order. Both are fixed by the reveal, so nobody chooses what comes next.
pub fn open_order(draw: &Draw, claimed: &BTreeSet<Hash>) -> Vec<u32> {
    let mut order = draw.batch.unused_indices.clone();
    for (t, serials) in &draw.won {
        if !claimed.contains(t) {
            order.extend(serials);
        }
    }
    order
}

/// The root `PostOpenMint` commits over an order.
pub fn open_root(order: &[u32]) -> Hash {
    let leaves: Vec<Hash> = order.iter().enumerate().map(|(i, s)| objectvm::sale::open_leaf(i as u32, *s)).collect();
    merkle_root(&leaves)
}

/// Everything a winner needs to claim, and a stranger needs to re-run it.
pub fn draw_json(collection: &Hash, close: &ResolvedClose, reveal: &RevealEvidence, draw: &Draw) -> Value {
    json!({
        "schema": "zyn-sale-draw/v1",
        "collection": hex(collection),
        "policy_digest": hex(&draw.batch.policy_digest),
        "manifest_root": hex(&draw.batch.manifest_root),
        "assignment_root": hex(&draw.batch.assignment_root),
        "close_zcash_height": close.deposit_close_zcash_height,
        "reveal": {
            "zcash_height": reveal.zcash_height,
            "zcash_block_hash": hex(&reveal.zcash_block_hash),
            "drand_round": reveal.beacon.round,
            "drand_signature": hex(&reveal.beacon.signature),
        },
        "root": hex(&draw.root),
        "won": draw.won.iter().map(|(t, s)| json!({ "ticket": hex(t), "serials": s })).collect::<Vec<_>>(),
        "unused": draw.batch.unused_indices,
    })
}

pub fn open_json(collection: &Hash, order: &[u32]) -> Value {
    json!({ "schema": "zyn-sale-open/v1", "collection": hex(collection), "root": hex(&open_root(order)), "order": order })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cave_randomness::DrandBeacon;

    fn manifest(n: u32) -> Vec<ManifestEntry> {
        (0..n)
            .map(|i| ManifestEntry {
                global_index: i,
                item_id: zyn_vm::item_address(zyn_vm::derive::ADDRESS_SCOPE_V1, &crate::cave_z::collection_id(), i),
                content_hash: [i as u8 + 1; 32],
            })
            .collect()
    }

    /// A small policy with the real one's shape: weighted, concealed, announced.
    fn policy(supply: u32) -> AllocationPolicy {
        let mut p = crate::cave_z::policy();
        p.supply = supply;
        let m = manifest(supply);
        p.manifest_root = cave_randomness::manifest_root(p.collection_id, &m).unwrap();
        p
    }

    #[test]
    fn the_draw_is_a_function_of_the_tickets_and_the_reveal_and_its_root_proves_each_win() {
        let p = policy(20);
        let m = manifest(20);
        let tickets: Vec<ZynTicket> = (0..6).map(|i| ZynTicket { id: [i as u8 + 0x40; 32], index: i }).collect();
        // A real quicknet round (tests/terms_match_code.rs), so the engine's
        // beacon check is the real one.
        let close = ResolvedClose { deposit_close_zcash_height: 3_402_000, reveal_zcash_height: 3_402_100, reveal_drand_round: 5_002_400 };
        let sig = "83559dd072b76a12aaf0b3f902f24a01c731307878d424fd095986f8de75f2687d0a1489aaa72432d74366ae31591e1f";
        let signature = (0..sig.len() / 2).map(|i| u8::from_str_radix(&sig[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let reveal = RevealEvidence {
            beacon: DrandBeacon { chain_hash: cave_randomness::QUICKNET_CHAIN_HASH, round: 5_002_400, signature },
            zcash_height: 3_402_100,
            zcash_block_hash: [0x5b; 32],
        };
        let a = run(&p, &m, &tickets, &close, &reveal, 4_000).unwrap();
        let b = run(&p, &m, &tickets, &close, &reveal, 4_000).unwrap();
        assert_eq!(a.root, b.root, "the same inputs draw the same result");
        let total: usize = a.won.iter().map(|(_, s)| s.len()).sum::<usize>() + a.batch.unused_indices.len();
        assert_eq!(total, 20, "every serial is either won or left for the open mint");
        let leaves: Vec<Hash> = a.won.iter().map(|(t, s)| objectvm::sale::draw_leaf(t, s)).collect();
        for (i, (t, s)) in a.won.iter().enumerate() {
            assert!(zyn_vm::commit::verify_proof(objectvm::sale::draw_leaf(t, s), &zyn_vm::commit::merkle_proof(&leaves, i), a.root));
        }
        let order = open_order(&a, &BTreeSet::new());
        assert_eq!(order.len(), 20, "unclaimed wins go back into the open mint");
    }

    #[test]
    fn a_list_out_of_order_is_refused() {
        let p = policy(4);
        let tickets = vec![ZynTicket { id: [1; 32], index: 1 }];
        let close = ResolvedClose { deposit_close_zcash_height: 1, reveal_zcash_height: 2, reveal_drand_round: 3 };
        let reveal = RevealEvidence { beacon: DrandBeacon { chain_hash: [0; 32], round: 3, signature: vec![1] }, zcash_height: 2, zcash_block_hash: [9; 32] };
        assert!(run(&p, &manifest(4), &tickets, &close, &reveal, 0).unwrap_err().contains("in order"));
    }
}
