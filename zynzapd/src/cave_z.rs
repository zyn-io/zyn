//! Collection Z's sale policy and terms, as data.
//!
//! Every value is settled somewhere else and repeated here rather than chosen:
//! the draw on 9 Sep, prices in §64.17, the manifest root by building it, the
//! close rule in §100.8. It lives in the library rather than in the binary that
//! prints it because the draw needs the same policy (§106.3) — a policy
//! constructed twice is two policies, and their digests would differ silently.

use cave_randomness::{
    AllocationPolicy, AnnouncedClose, IdentityPolicy, ManifestEntry, QuantityPolicy,
    RandomnessPlan, SaleTerms, WeightedOutcome, QUICKNET_CHAIN_HASH,
};
use swapvm::types::TREASURY_26460;
use zyn_vm::derive::ADDRESS_SCOPE_V1;

use std::collections::BTreeMap;

const SYMBOL: &str = "CAVEZ";
const SUPPLY: u32 = 4_444;

/// The manifest root over all 4,444, from `zyn-cave-manifest` (§86.3). Pinned
/// rather than recomputed: the terms must commit to a root someone checked,
/// not to whatever a rebuild happens to produce today.
const MANIFEST_ROOT: &str = "02b01b16da58e0d71ae27099daba30a0a7936cdd852ee325853f679886dd8f19";

/// §64.17, frozen: entry / claim / open mint, in zatoshi.
const ENTRY: u64 = 1_000_000; // 0.01 ZEC
const CLAIM: u64 = 500_000; // 0.005 ZEC
const OPEN_MINT: u64 = 1_330_000; // 0.0133 ZEC
/// 10% to the treasury; the remaining 90% funds the collection and is the floor.
const TREASURY_BPS: u16 = 1_000;
/// Six confirmations before a deposit is a ticket.
const CONFIRMATIONS: u32 = 6;
/// A claim may be made for about a fortnight of mainnet blocks after the close.
const CLAIM_WINDOW: u32 = 16_128;

#[allow(dead_code)]
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn unhex32(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex");
    }
    out
}

pub fn policy() -> AllocationPolicy {
    AllocationPolicy {
        collection_id: zyn_vm::collection_address(ADDRESS_SCOPE_V1, &TREASURY_26460, SYMBOL.as_bytes()),
        supply: SUPPLY,
        entry_price_zatoshi: ENTRY,
        // Announced, not committed: how long to run is judged by watching
        // demand, and the reveal is derived from whatever is announced so the
        // timing stays a decision and the outcome does not (§100.2).
        deposit_close_zcash_height: 0,
        // Decided 9 Sep: one ticket draws 1, 2 or 3 items.
        quantity: QuantityPolicy::Weighted {
            outcomes: vec![
                WeightedOutcome { items: 1, weight: 80 },
                WeightedOutcome { items: 2, weight: 19 },
                WeightedOutcome { items: 3, weight: 1 },
            ],
        },
        identity: IdentityPolicy::ConcealedShuffle,
        manifest_root: unhex32(MANIFEST_ROOT),
        randomness: RandomnessPlan {
            drand_chain_hash: QUICKNET_CHAIN_HASH,
            // A weighted draw resolves at a round this far past the ticket's
            // own, so it cannot be known when the ticket is made.
            ticket_round_delay: 10,
            reveal_drand_round: None,
            reveal_zcash_height: None,
            announced_close: Some(AnnouncedClose {
                reveal_offset_blocks: 100,   // ~2 h past the close
                reveal_offset_rounds: 2_400, // ~2 h of 3 s drand rounds
                min_notice_blocks: 1_152,    // ~1 day of notice
            }),
        },
    }
}

pub fn terms(p: &AllocationPolicy) -> SaleTerms {
    SaleTerms {
        policy_digest: p.digest().expect("the policy must validate"),
        collection_id: p.collection_id,
        manifest_root: p.manifest_root,
        treasury: TREASURY_26460,
        network: 0, // Zcash mainnet
        entry_price_zatoshi: ENTRY,
        claim_price_zatoshi: CLAIM,
        open_mint_price_zatoshi: OPEN_MINT,
        min_confirmations: CONFIRMATIONS,
        claim_window_blocks: CLAIM_WINDOW,
        treasury_bps: TREASURY_BPS,
    }
}


/// The manifest as the allocator needs it, from the builder's `content.json`.
///
/// The item id is derived here rather than read, because it is a function of
/// the collection and the index and deriving it is what `manifest_root` will
/// check anyway (§86.4). A file that disagreed would fail there, loudly.
pub fn manifest_from(content: &BTreeMap<u32, [u8; 32]>) -> Vec<ManifestEntry> {
    let collection = collection_id();
    content
        .iter()
        .map(|(&global_index, &content_hash)| ManifestEntry {
            global_index,
            item_id: zyn_vm::item_address(ADDRESS_SCOPE_V1, &collection, global_index),
            content_hash,
        })
        .collect()
}

pub fn collection_id() -> [u8; 32] {
    zyn_vm::collection_address(ADDRESS_SCOPE_V1, &TREASURY_26460, SYMBOL.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The policy must be one object, not two that happen to agree. Both
    /// binaries build it from here, so this asserts the digest is a function of
    /// the recorded values and nothing else.
    #[test]
    fn the_digests_are_the_published_ones() {
        let p = policy();
        p.validate().expect("valid");
        assert_eq!(
            hex(&p.digest().unwrap()),
            "12da4f1b1052660d5496207eefa0231801dcad04d6a85b77db0134883500abaf",
            "the policy digest is published; moving it silently is the failure"
        );
        assert_eq!(
            hex(&terms(&p).digest()),
            "159b454efe0bfe7778f58924e6e2951d7e585e918a9a7d94bbab8e69ae016267"
        );
    }

    /// The manifest the allocator reads must produce the root the terms
    /// commit to, or the draw runs over a different collection than the one
    /// that was published.
    #[test]
    fn the_manifest_rebuilds_the_committed_root() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../examples/zyn-pfp-concepts/out/content.json"
        );
        if !std::path::Path::new(path).exists() {
            eprintln!("skipping: run build_collection_z_manifests.py first");
            return;
        }
        let raw = std::fs::read_to_string(path).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let mut content = BTreeMap::new();
        for (k, v) in doc.as_object().unwrap() {
            let mut h = [0u8; 32];
            let s = v.get("content").unwrap().as_str().unwrap();
            for (i, b) in h.iter_mut().enumerate() {
                *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
            }
            content.insert(k.parse::<u32>().unwrap(), h);
        }
        let entries = manifest_from(&content);
        assert_eq!(entries.len(), 4_444);
        let root = cave_randomness::manifest_root(collection_id(), &entries).unwrap();
        assert_eq!(
            hex(&root),
            "02b01b16da58e0d71ae27099daba30a0a7936cdd852ee325853f679886dd8f19",
            "the manifest the draw reads must be the one the terms committed to"
        );
        assert_eq!(root, policy().manifest_root);
    }
}
