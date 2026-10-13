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

pub const SYMBOL: &str = "CAVEZ";
const SUPPLY: u32 = 4_444;

/// The manifest root over all 4,444, from `zyn-cave-manifest` (§86.3; moved
/// 23 Sep 2026, before any deposit, when every item gained its Rarity traits
/// and the Moles and Owls were re-cut square from their canvases — the 19 Sep terms named
/// `02b01b16…`, §142). Pinned
/// rather than recomputed: the terms must commit to a root someone checked,
/// not to whatever a rebuild happens to produce today.
const MANIFEST_ROOT: &str = "0c56593e1b16bd513a4c88baf31d42602acd89db85aef2b0dcbc4e4bce108426";

/// §64.17, frozen: entry / claim / open mint, in zatoshi.
const ENTRY: u64 = 1_000_000; // 0.01 ZEC
const CLAIM: u64 = 500_000; // 0.005 ZEC
const OPEN_MINT: u64 = 1_330_000; // 0.0133 ZEC
/// 10% to the treasury; the remaining 90% funds the collection and is the floor.
const TREASURY_BPS: u16 = 1_000;
/// Six confirmations before a deposit is a ticket.
pub const CONFIRMATIONS: u32 = 6;
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

/// The published policy with a testnet clock (DECISIONS §144): the same odds,
/// shuffle, manifest and prices, but minutes of notice and reveal instead of a
/// day and two hours, so a rehearsal of the whole sale fits in an afternoon.
/// A different digest from the published one — it is never sold under.
pub fn rehearsal_policy() -> AllocationPolicy {
    let mut p = policy();
    p.randomness.announced_close = Some(AnnouncedClose {
        reveal_offset_blocks: 2,
        reveal_offset_rounds: 60, // three minutes of 3 s rounds
        min_notice_blocks: 2,
    });
    p
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


/// The re-issued terms (§175): v1's sale, plus what was decided after it —
/// the whitelist gate and rising price (§168), the treasury share going to
/// the collection's keyless DAO treasury under its strict rules (§172, §173),
/// and the burn rule and its caps (§145, §174). The draw's policy, and so its
/// digest, is unchanged; v1's terms digest is named as the one superseded.
///
/// Its own version and domain rather than a v1 field change: a v1 reader must
/// not parse these bytes as v1 terms with a different treasury.
pub const TERMS_V2_VERSION: &str = "cave-sale-terms/v2";
const TERMS_V2_DOMAIN: &[u8] = b"cave.sale.terms.v2";

/// v1's published terms digest, which v2 replaces before any ticket was sold.
const SUPERSEDES: &str = "f2c3242060baf60e0c193b41dc726b6677d85c158a62ba200c948d48a177a59e";

/// Where the treasury share goes. 1: the collection's own DAO treasury,
/// which no key can spend; only a passed proposal pays out of it (§172.6).
pub const TREASURY_DAO: u8 = 1;

/// §168.2: a listed account's first 10 tickets cost the ticket price; each one
/// after costs one more ticket price than the last (11th 0.02, 12th 0.03, …).
pub const GATE_BASE: u32 = 10;
/// §173.11: the DAO's mainnet terms, fixed at its first proposal.
pub const DAO_BACKERS: u32 = 10;
pub const DAO_WINDOW_BLOCKS: u32 = 8_064;
pub const DAO_QUORUM_BPS: u32 = 1_000;
/// §174: a burn ticket costs one ticket price in ZEC, at the median of the
/// last hour's pool readings (at least 10).
pub const BURN_MEDIAN_SECONDS: u32 = 3_600;
pub const BURN_MIN_READINGS: u32 = 10;
/// §145.6: at most 20 tickets from one burn; and the user, 8 Oct 2026: "Cap
/// at 20" — at most 20 burn tickets in the whole sale (`SetVoucherCap`).
pub const BURN_PER_BURN: u32 = 20;
pub const BURN_CAP: u32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermsV2 {
    pub v1: SaleTerms,
    pub supersedes: [u8; 32],
    pub treasury_kind: u8,
    pub gate_base: u32,
    /// Added per ticket past the base: the n-th ticket past it costs
    /// entry + n × step.
    pub gate_step_zatoshi: u64,
    pub dao_backers: u32,
    pub dao_window_blocks: u32,
    pub dao_quorum_bps: u32,
    pub burn_ticket_zatoshi: u64,
    pub burn_median_seconds: u32,
    pub burn_min_readings: u32,
    pub burn_per_burn: u32,
    pub burn_cap: u32,
}

impl TermsV2 {
    /// v1's fields in v1's order, except the treasury account, which v2
    /// replaces with `treasury_kind`; then what v2 adds. Integers big-endian,
    /// the version length-prefixed, as in v1.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let t = &self.v1;
        let mut out = Vec::new();
        out.extend_from_slice(&(TERMS_V2_VERSION.len() as u32).to_be_bytes());
        out.extend_from_slice(TERMS_V2_VERSION.as_bytes());
        out.extend_from_slice(&self.supersedes);
        out.extend_from_slice(&t.policy_digest);
        out.extend_from_slice(&t.collection_id);
        out.extend_from_slice(&t.manifest_root);
        out.push(self.treasury_kind);
        out.push(t.network);
        out.extend_from_slice(&t.entry_price_zatoshi.to_be_bytes());
        out.extend_from_slice(&t.claim_price_zatoshi.to_be_bytes());
        out.extend_from_slice(&t.open_mint_price_zatoshi.to_be_bytes());
        out.extend_from_slice(&t.min_confirmations.to_be_bytes());
        out.extend_from_slice(&t.claim_window_blocks.to_be_bytes());
        out.extend_from_slice(&(t.treasury_bps as u32).to_be_bytes());
        out.extend_from_slice(&self.gate_base.to_be_bytes());
        out.extend_from_slice(&self.gate_step_zatoshi.to_be_bytes());
        out.extend_from_slice(&self.dao_backers.to_be_bytes());
        out.extend_from_slice(&self.dao_window_blocks.to_be_bytes());
        out.extend_from_slice(&self.dao_quorum_bps.to_be_bytes());
        out.extend_from_slice(&self.burn_ticket_zatoshi.to_be_bytes());
        out.extend_from_slice(&self.burn_median_seconds.to_be_bytes());
        out.extend_from_slice(&self.burn_min_readings.to_be_bytes());
        out.extend_from_slice(&self.burn_per_burn.to_be_bytes());
        out.extend_from_slice(&self.burn_cap.to_be_bytes());
        out
    }

    /// SHA-256 over the length-prefixed domain and payload, as v1.
    pub fn digest(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let payload = self.canonical_bytes();
        let mut h = Sha256::new();
        h.update((TERMS_V2_DOMAIN.len() as u32).to_be_bytes());
        h.update(TERMS_V2_DOMAIN);
        h.update((payload.len() as u32).to_be_bytes());
        h.update(&payload);
        h.finalize().into()
    }

    /// The price of a listed account's `n`-th ticket (1-based).
    pub fn ticket_price(&self, n: u32) -> u64 {
        let past = n.saturating_sub(self.gate_base) as u64;
        self.v1.entry_price_zatoshi + past * self.gate_step_zatoshi
    }
}

pub fn terms_v2(p: &AllocationPolicy) -> TermsV2 {
    TermsV2 {
        v1: terms(p),
        supersedes: unhex32(SUPERSEDES),
        treasury_kind: TREASURY_DAO,
        gate_base: GATE_BASE,
        gate_step_zatoshi: ENTRY,
        dao_backers: DAO_BACKERS,
        dao_window_blocks: DAO_WINDOW_BLOCKS,
        dao_quorum_bps: DAO_QUORUM_BPS,
        burn_ticket_zatoshi: ENTRY,
        burn_median_seconds: BURN_MEDIAN_SECONDS,
        burn_min_readings: BURN_MIN_READINGS,
        burn_per_burn: BURN_PER_BURN,
        burn_cap: BURN_CAP,
    }
}

/// Collection Z's terms, v3 (DECISIONS §182): v2's, except that burns no
/// longer become free, capped tickets — burning ZCAVE on Solana or REGI on
/// Arc gives credit in the sale worth what was burned at its USD price, at
/// the lower median of the last hour (at least ten readings), and credit
/// pays tickets, claims and the open mint before any ZEC.zy. The team pays
/// ZEC into the sale for that credit first, as an ordinary sale payment, so
/// token-paid items back the floor like any others. The draw's policy, and
/// so its digest, is unchanged; v2's terms digest is named as superseded.
pub const TERMS_V3_VERSION: &str = "cave-sale-terms/v3";
const TERMS_V3_DOMAIN: &[u8] = b"cave.sale.terms.v3";

/// v2's published terms digest (§175), which v3 replaces before any ticket.
const SUPERSEDES_V2: &str = "9267d58e849caf1bff4a622451185c609e476ac0d026e3c95761bee73cae3d3c";

/// What credit pays: tickets (1), claims (2) and the open mint (4).
pub const CREDIT_PAYS: u8 = 0b111;
/// USD (1): a burn is worth its token's USD price over ZEC's.
pub const CREDIT_PRICED_IN_USD: u8 = 1;
/// §182.6: the lower median of the last hour's readings, at least ten.
pub const CREDIT_MEDIAN_SECONDS: u32 = 3_600;
pub const CREDIT_MIN_READINGS: u32 = 10;
/// The tokens burned for credit, as the watchers read them: the chain and the
/// token in its own written form.
pub const CREDIT_TOKENS: [(&str, &str); 2] = [
    ("solana", "GQepeKGNpNatDRMPivFYVsJmfWUsJhtzFhBthCtLpump"),
    ("eip155:5042", "0x93d5b8c53ee763c2c4522bf0d958ce51af4360ae"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermsV3 {
    /// v2, whose burn-ticket fields v3 drops: no burn tickets are sold.
    pub v2: TermsV2,
    pub supersedes: [u8; 32],
    pub credit_pays: u8,
    pub credit_priced: u8,
    pub credit_median_seconds: u32,
    pub credit_min_readings: u32,
    pub credit_tokens: Vec<(String, String)>,
}

impl TermsV3 {
    /// v2's fields in v2's order up to its DAO terms, then the credit rule
    /// and its tokens (each string length-prefixed). Integers big-endian.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let v2 = &self.v2;
        let t = &v2.v1;
        let mut out = Vec::new();
        let mut text = |out: &mut Vec<u8>, s: &str| {
            out.extend_from_slice(&(s.len() as u32).to_be_bytes());
            out.extend_from_slice(s.as_bytes());
        };
        text(&mut out, TERMS_V3_VERSION);
        out.extend_from_slice(&self.supersedes);
        out.extend_from_slice(&t.policy_digest);
        out.extend_from_slice(&t.collection_id);
        out.extend_from_slice(&t.manifest_root);
        out.push(v2.treasury_kind);
        out.push(t.network);
        out.extend_from_slice(&t.entry_price_zatoshi.to_be_bytes());
        out.extend_from_slice(&t.claim_price_zatoshi.to_be_bytes());
        out.extend_from_slice(&t.open_mint_price_zatoshi.to_be_bytes());
        out.extend_from_slice(&t.min_confirmations.to_be_bytes());
        out.extend_from_slice(&t.claim_window_blocks.to_be_bytes());
        out.extend_from_slice(&(t.treasury_bps as u32).to_be_bytes());
        out.extend_from_slice(&v2.gate_base.to_be_bytes());
        out.extend_from_slice(&v2.gate_step_zatoshi.to_be_bytes());
        out.extend_from_slice(&v2.dao_backers.to_be_bytes());
        out.extend_from_slice(&v2.dao_window_blocks.to_be_bytes());
        out.extend_from_slice(&v2.dao_quorum_bps.to_be_bytes());
        out.push(self.credit_pays);
        out.push(self.credit_priced);
        out.extend_from_slice(&self.credit_median_seconds.to_be_bytes());
        out.extend_from_slice(&self.credit_min_readings.to_be_bytes());
        out.extend_from_slice(&(self.credit_tokens.len() as u32).to_be_bytes());
        for (chain, token) in &self.credit_tokens {
            text(&mut out, chain);
            text(&mut out, token);
        }
        out
    }

    /// SHA-256 over the length-prefixed domain and payload, as v1 and v2.
    pub fn digest(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let payload = self.canonical_bytes();
        let mut h = Sha256::new();
        h.update((TERMS_V3_DOMAIN.len() as u32).to_be_bytes());
        h.update(TERMS_V3_DOMAIN);
        h.update((payload.len() as u32).to_be_bytes());
        h.update(&payload);
        h.finalize().into()
    }
}

pub fn terms_v3(p: &AllocationPolicy) -> TermsV3 {
    TermsV3 {
        v2: terms_v2(p),
        supersedes: unhex32(SUPERSEDES_V2),
        credit_pays: CREDIT_PAYS,
        credit_priced: CREDIT_PRICED_IN_USD,
        credit_median_seconds: CREDIT_MEDIAN_SECONDS,
        credit_min_readings: CREDIT_MIN_READINGS,
        credit_tokens: CREDIT_TOKENS.iter().map(|(c, t)| (c.to_string(), t.to_string())).collect(),
    }
}

/// Collection Z's terms, v4 (DECISIONS §187): no tickets and no lottery; the
/// collection goes straight to its open mint. The user, 13 Oct 2026: "0.05
/// zec is price. whitelist applicants we accept all of them and allow them to
/// mint in 0.01 zec. every wallet max 20 mints". The sale's list is every
/// whitelist applicant who links a Zyn account; they pay the listed price.
/// No account mints more than the cap, listed or not. The order the items
/// come in is the policy's concealed shuffle over no tickets, under the
/// announced close's reveal, so nobody chooses it and anyone can replay it.
/// The treasury split, the DAO's terms and the credit rule are v3's, except
/// that credit pays only the open mint, the one thing left to pay for.
pub const TERMS_V4_VERSION: &str = "cave-sale-terms/v4";
const TERMS_V4_DOMAIN: &[u8] = b"cave.sale.terms.v4";

/// v3's published terms digest (§182), which v4 replaces before any sale.
const SUPERSEDES_V3: &str = "cd4cdc6ab086ed51aa318093d34972f9567bcd8d3b855d19bfdbfc857b44e315";

/// §187: the open price, the price for accounts on the list, and the cap.
pub const OPEN_V4: u64 = 5_000_000; // 0.05 ZEC
pub const LISTED_V4: u64 = 1_000_000; // 0.01 ZEC
pub const CAP_V4: u32 = 20;
/// Who is on the list: 1, every whitelist applicant who links a Zyn account.
pub const LIST_APPLICANTS: u8 = 1;
/// How the order is fixed: 1, the policy's concealed shuffle over no tickets
/// under the announced close's reveal (drand round and Zcash block).
pub const ORDER_REVEALED: u8 = 1;
/// What credit pays in v4: the open mint (4).
pub const CREDIT_PAYS_V4: u8 = 0b100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermsV4 {
    pub v3: TermsV3,
    pub supersedes: [u8; 32],
    pub open_price_zatoshi: u64,
    pub listed_price_zatoshi: u64,
    pub cap_per_account: u32,
    pub list_rule: u8,
    pub order_rule: u8,
}

impl TermsV4 {
    /// The version, what it replaces, the collection, the open mint's prices,
    /// cap, list and order, then v3's treasury, DAO and credit terms (credit
    /// paying the open mint only). Integers big-endian, strings
    /// length-prefixed, as before.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let v3 = &self.v3;
        let v2 = &v3.v2;
        let t = &v2.v1;
        let mut out = Vec::new();
        let text = |out: &mut Vec<u8>, s: &str| {
            out.extend_from_slice(&(s.len() as u32).to_be_bytes());
            out.extend_from_slice(s.as_bytes());
        };
        text(&mut out, TERMS_V4_VERSION);
        out.extend_from_slice(&self.supersedes);
        out.extend_from_slice(&t.policy_digest);
        out.extend_from_slice(&t.collection_id);
        out.extend_from_slice(&t.manifest_root);
        out.push(v2.treasury_kind);
        out.push(t.network);
        out.extend_from_slice(&self.open_price_zatoshi.to_be_bytes());
        out.extend_from_slice(&self.listed_price_zatoshi.to_be_bytes());
        out.extend_from_slice(&self.cap_per_account.to_be_bytes());
        out.push(self.list_rule);
        out.push(self.order_rule);
        out.extend_from_slice(&(t.treasury_bps as u32).to_be_bytes());
        out.extend_from_slice(&v2.dao_backers.to_be_bytes());
        out.extend_from_slice(&v2.dao_window_blocks.to_be_bytes());
        out.extend_from_slice(&v2.dao_quorum_bps.to_be_bytes());
        out.push(v3.credit_pays);
        out.push(v3.credit_priced);
        out.extend_from_slice(&v3.credit_median_seconds.to_be_bytes());
        out.extend_from_slice(&v3.credit_min_readings.to_be_bytes());
        out.extend_from_slice(&(v3.credit_tokens.len() as u32).to_be_bytes());
        for (chain, token) in &v3.credit_tokens {
            text(&mut out, chain);
            text(&mut out, token);
        }
        out
    }

    /// SHA-256 over the length-prefixed domain and payload, as before.
    pub fn digest(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let payload = self.canonical_bytes();
        let mut h = Sha256::new();
        h.update((TERMS_V4_DOMAIN.len() as u32).to_be_bytes());
        h.update(TERMS_V4_DOMAIN);
        h.update((payload.len() as u32).to_be_bytes());
        h.update(&payload);
        h.finalize().into()
    }
}

pub fn terms_v4(p: &AllocationPolicy) -> TermsV4 {
    let mut v3 = terms_v3(p);
    v3.credit_pays = CREDIT_PAYS_V4;
    TermsV4 {
        v3,
        supersedes: unhex32(SUPERSEDES_V3),
        open_price_zatoshi: OPEN_V4,
        listed_price_zatoshi: LISTED_V4,
        cap_per_account: CAP_V4,
        list_rule: LIST_APPLICANTS,
        order_rule: ORDER_REVEALED,
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
        const V3_DIGEST: &str = "cd4cdc6ab086ed51aa318093d34972f9567bcd8d3b855d19bfdbfc857b44e315";
        let p = policy();
        p.validate().expect("valid");
        assert_eq!(
            hex(&p.digest().unwrap()),
            "a572a74778052829a768adb56f84404353ee5ad188b274b36b5fa58c68500b13",
            "the policy digest is published; moving it silently is the failure"
        );
        assert_eq!(
            hex(&terms(&p).digest()),
            "f2c3242060baf60e0c193b41dc726b6677d85c158a62ba200c948d48a177a59e"
        );
        let v2 = terms_v2(&p);
        assert_eq!(hex(&v2.supersedes), hex(&terms(&p).digest()), "v2 names the v1 it replaces");
        assert_eq!(
            hex(&v2.digest()),
            "9267d58e849caf1bff4a622451185c609e476ac0d026e3c95761bee73cae3d3c",
            "the re-issued terms (§175)"
        );
        let v3 = terms_v3(&p);
        assert_eq!(hex(&v3.supersedes), hex(&v2.digest()), "v3 names the v2 it replaces");
        assert_eq!(hex(&v3.digest()), V3_DIGEST, "the credit terms (§182)");
        let v4 = terms_v4(&p);
        assert_eq!(hex(&v4.supersedes), V3_DIGEST, "v4 names the v3 it replaces");
        assert_eq!((v4.open_price_zatoshi, v4.listed_price_zatoshi, v4.cap_per_account), (5_000_000, 1_000_000, 20), "§187");
        assert_eq!(v4.v3.credit_pays, 0b100, "credit pays the open mint only");
        assert_eq!(hex(&v4.digest()), "ec3b0c2ffc7ae98423479d71d788ed47f975549e66626dc6f9df1b221d5004e8", "the open-mint terms (§187)");
    }

    /// §168.2's prices, from the terms rather than from the ledger's code.
    #[test]
    fn the_v2_prices_rise_past_ten() {
        let v2 = terms_v2(&policy());
        assert_eq!(v2.ticket_price(1), 1_000_000);
        assert_eq!(v2.ticket_price(10), 1_000_000);
        assert_eq!(v2.ticket_price(11), 2_000_000);
        assert_eq!(v2.ticket_price(12), 3_000_000);
        assert_eq!((v2.burn_per_burn, v2.burn_cap), (20, 20));
        assert_eq!((v2.dao_backers, v2.dao_window_blocks, v2.dao_quorum_bps), (10, 8_064, 1_000));
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
            "0c56593e1b16bd513a4c88baf31d42602acd89db85aef2b0dcbc4e4bce108426",
            "the manifest the draw reads must be the one the terms committed to"
        );
        assert_eq!(root, policy().manifest_root);
    }
}
