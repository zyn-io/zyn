//! Running the draw against the chain.
//!
//! §104-§106 built the pieces and `tests/sale_pipeline.rs` proved they compose.
//! What did not exist was the thing that runs them for real (A98). This is
//! that, and it is a module rather than a body inside the `zyn-sale` binary for
//! one reason: the service has to serve the same draw an operator ran. The
//! alternative was a file written by the command and trusted by the service,
//! which is a serialisation format plus a way for the two to disagree about
//! who owns what.
//!
//! It can be a shared function instead of a shared file because the draw is
//! deterministic — §106.2 is the test that says the result is identical
//! whichever order the scanner reported payments in. Two operators running this
//! over the same close reach the same assignment, and `Run::agrees_with` is how
//! they check without trusting each other.
//!
//! Every source is injected. A draw that can only be exercised against mainnet
//! is a draw whose first real run is also its first run.

use cave_randomness::{
    AllocationPolicy, CloseAnnouncement, IdentityPolicy, ManifestEntry, PublicationProof,
    ResolvedClose, QUICKNET_CHAIN_HASH,
};

use crate::allocation::{self, Refused};
use crate::reveal::{self, Sighting};
use crate::tickets::NotATicket;

pub type Hash = [u8; 32];

/// What the published announcement fixed, and what the operator must not be
/// free to choose at draw time.
///
/// These are inputs rather than settings: every one of them was committed to
/// before the deposits arrived (§100, §101). A draw that took them from the
/// operator's shell at reveal time would be a draw the operator could retry
/// until they liked the outcome.
#[derive(Clone, Debug)]
pub struct Plan {
    /// Where to begin scanning. Below the first possible entry is merely slow;
    /// above it silently loses the buyers who paid early.
    pub scan_from: u64,
    pub close_height: u32,
    pub reveal_zcash_height: u32,
    pub reveal_drand_round: u64,
    /// The round current when the sale was announced, which every ticket id
    /// binds. Not the reveal round.
    pub issued_at_drand_round: u64,
    pub min_confirmations: u32,
    pub terms_digest: Hash,
}

impl Plan {
    /// Derive a plan from the published announcement — the operator path.
    ///
    /// The reveal height and round are **not** operator inputs. §101 closed the
    /// backdating hole by deriving both from an announcement whose digest is on
    /// Zcash, and reading them from the environment here would reopen it: an
    /// operator who could type a reveal round could try rounds until one paid.
    /// `resolve` also refuses an announcement that gave less notice than the
    /// policy promised, or that belongs to another policy.
    ///
    /// Constructing a `Plan` field-by-field stays possible for tests. This is
    /// the constructor an operator gets.
    pub fn from_announcement(
        policy: &AllocationPolicy,
        announcement: &CloseAnnouncement,
        proof: &PublicationProof,
        min_confirmations: u32,
        scan_from: u64,
        terms_digest: Hash,
    ) -> Result<Plan, String> {
        let resolved = announcement
            .resolve(policy, proof)
            .map_err(|e| format!("the close announcement does not resolve: {e}"))?;
        Ok(Plan {
            scan_from,
            close_height: resolved.deposit_close_zcash_height,
            reveal_zcash_height: resolved.reveal_zcash_height,
            reveal_drand_round: resolved.reveal_drand_round,
            issued_at_drand_round: announcement.announced_at_drand_round,
            min_confirmations,
            terms_digest,
        })
    }
}

/// The chain, as the draw needs to see it.
///
/// A trait rather than a `Zebra` so the whole sequence is testable without a
/// node — the bug this shape prevents is the one §105.4 already caught once,
/// where every stubbed test passed against an endpoint that did not exist.
pub trait Chain {
    fn tip(&self) -> Result<u64, String>;
    /// Cave payments in `from..=to`, plus how far the scan actually reached.
    /// The scanner caps its own range, so a caller must loop.
    fn cave_payments(&self, from: u64, to: u64) -> Result<(u64, Vec<Sighting>), String>;
    fn block_hash(&self, height: u64) -> Result<Hash, String>;
}

/// One completed draw and everything a human has to answer for.
#[derive(Debug)]
pub struct Outcome {
    pub run: allocation::Run,
    /// Payments that did not become tickets. Carried out of here rather than
    /// logged and dropped: each one is somebody whose money arrived.
    pub refused: Vec<(Sighting, NotATicket)>,
    pub tip: u64,
    pub scanned_to: u64,
    pub seen: usize,
}

/// Scan, admit, reveal, draw.
pub fn execute(
    plan: &Plan,
    policy: &AllocationPolicy,
    manifest: &[ManifestEntry],
    chain: &dyn Chain,
    get: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<Outcome, String> {
    let tip = chain.tip()?;

    // Drawing before the close is buried means a reorg can still change who
    // paid, and the draw happens once (§104.1). Refuse rather than warn.
    let buried = plan.close_height as u64 + plan.min_confirmations as u64;
    if tip < buried {
        return Err(format!(
            "tip {tip} has not buried the close at {} by {} confirmation(s) — \
             wait for {buried} rather than draw over a ledger that can still change",
            plan.close_height, plan.min_confirmations
        ));
    }
    if tip < plan.reveal_zcash_height as u64 {
        return Err(format!(
            "the reveal block {} does not exist yet (tip {tip})",
            plan.reveal_zcash_height
        ));
    }

    let seen = scan(chain, plan.scan_from, plan.close_height as u64)?;
    let scanned_to = plan.close_height as u64;

    let tip_u32 = u32::try_from(tip).map_err(|_| format!("tip {tip} does not fit a u32 height"))?;
    let (ledger, refused) = reveal::ledger_from(
        policy,
        plan.terms_digest,
        plan.close_height,
        plan.min_confirmations,
        tip_u32,
        &seen,
        plan.issued_at_drand_round,
    );

    // Only a concealed shuffle has a reveal. Fetching a beacon for a policy
    // that does not use one would be a network call whose failure stopped a
    // sale that never needed it.
    let evidence = if policy.identity == IdentityPolicy::ConcealedShuffle {
        let beacon = reveal::beacon(plan.reveal_drand_round, QUICKNET_CHAIN_HASH, get)?;
        let block = chain.block_hash(plan.reveal_zcash_height as u64)?;
        Some(reveal::reveal(beacon, plan.reveal_zcash_height, block)?)
    } else {
        None
    };

    let tickets = reveal::evidence_for(ledger.tickets());
    // An announced close means the numbers the allocator measures against were
    // derived from the announcement, not committed in the policy (§113). The
    // plan already holds them, resolved and publication-checked.
    let targets = policy.randomness.announced_close.map(|_| ResolvedClose {
        deposit_close_zcash_height: plan.close_height,
        reveal_zcash_height: plan.reveal_zcash_height,
        reveal_drand_round: plan.reveal_drand_round,
    });
    let run = allocation::run_resolved(policy, manifest, &tickets, evidence.as_ref(), targets.as_ref())
        .map_err(|e| match e {
        // The one refusal worth expanding: it means real money arrived and
        // none of it was admissible, which is a configuration answer rather
        // than a sale that nobody entered.
        Refused::NoTickets if !refused.is_empty() => format!(
            "no tickets: all {} payment(s) seen were refused — {}",
            refused.len(),
            refused
                .iter()
                .map(|(_, why)| why.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        ),
        other => other.to_string(),
    })?;

    Ok(Outcome {
        run,
        refused,
        tip,
        scanned_to,
        seen: seen.len(),
    })
}

/// Walk the range in whatever steps the scanner will give.
fn scan(chain: &dyn Chain, from: u64, to: u64) -> Result<Vec<Sighting>, String> {
    let mut found = Vec::new();
    let mut at = from;
    while at <= to {
        let (reached, mut batch) = chain.cave_payments(at, to)?;
        found.append(&mut batch);
        // A scanner that will not advance is misconfigured — typically its own
        // `from_height` sits above this range. Spinning here would look like a
        // slow scan forever.
        if reached < at {
            return Err(format!(
                "the scanner reached {reached} when asked for {at}..={to} — \
                 check its start height against the sale's"
            ));
        }
        at = reached + 1;
    }
    Ok(found)
}

impl Outcome {
    /// What an operator reads before publishing anything.
    pub fn report(&self) -> String {
        let batch = &self.run.batch;
        let mut out = String::new();
        out.push_str(&format!(
            "scanned to {}, tip {}, {} payment(s) seen\n",
            self.scanned_to, self.tip, self.seen
        ));
        out.push_str(&format!(
            "{} ticket(s) drawn, {} item(s) unallocated\n",
            self.run.tickets,
            self.run.unallocated().len()
        ));
        out.push_str(&format!("policy digest     {}\n", hex(&batch.policy_digest)));
        out.push_str(&format!("manifest root     {}\n", hex(&batch.manifest_root)));
        out.push_str(&format!(
            "ticket ledger     {}\n",
            hex(&batch.ticket_ledger_root)
        ));
        out.push_str(&format!(
            "assignment root   {}\n",
            hex(&batch.assignment_root)
        ));
        if !self.refused.is_empty() {
            out.push_str(&format!(
                "\n{} payment(s) refused — each one is owed an answer:\n",
                self.refused.len()
            ));
            for (s, why) in &self.refused {
                out.push_str(&format!(
                    "  {} output {} at height {}: {}\n",
                    hex(&s.txid),
                    s.output_index,
                    s.height,
                    why
                ));
            }
        }
        out
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cave_randomness::{QuantityPolicy, RandomnessPlan};
    use std::cell::RefCell;
    use zyn_custody::memo::{CavePaymentMemo, CavePaymentPurpose};
    use zyn_vm::derive::ADDRESS_SCOPE_V1;

    const COLLECTION: Hash = [0x02; 32];
    pub(super) const TERMS: Hash = [0x7c; 32];
    const ENTRY: u64 = 1_000_000;
    const CLOSE: u32 = 3_500_000;
    const SUPPLY: u32 = 20;

    pub(super) fn manifest() -> Vec<ManifestEntry> {
        (0..SUPPLY)
            .map(|i| ManifestEntry {
                global_index: i,
                item_id: zyn_vm::item_address(ADDRESS_SCOPE_V1, &COLLECTION, i),
                content_hash: {
                    let mut h = [0u8; 32];
                    h[..4].copy_from_slice(&i.to_be_bytes());
                    h[31] = 0xc0;
                    h
                },
            })
            .collect()
    }

    pub(super) fn policy(identity: IdentityPolicy) -> AllocationPolicy {
        AllocationPolicy {
            collection_id: COLLECTION,
            supply: SUPPLY,
            entry_price_zatoshi: ENTRY,
            deposit_close_zcash_height: CLOSE,
            quantity: QuantityPolicy::Fixed { items: 1 },
            identity,
            manifest_root: cave_randomness::manifest_root(COLLECTION, &manifest()).unwrap(),
            randomness: RandomnessPlan {
                drand_chain_hash: QUICKNET_CHAIN_HASH,
                ticket_round_delay: 10,
                reveal_drand_round: None,
                reveal_zcash_height: None,
                announced_close: None,
            },
        }
    }

    fn plan() -> Plan {
        Plan {
            scan_from: 3_499_000,
            close_height: CLOSE,
            reveal_zcash_height: CLOSE + 100,
            reveal_drand_round: 5_002_400,
            issued_at_drand_round: 5_000_000,
            min_confirmations: 6,
            terms_digest: TERMS,
        }
    }

    fn paid(txid: u8, who: u8, height: u64, amount: u64) -> Sighting {
        Sighting {
            txid: [txid; 32],
            height,
            tx_index: 0,
            output_index: 0,
            amount_zatoshi: amount,
            payment: CavePaymentMemo {
                purpose: CavePaymentPurpose::Entry,
                reference: TERMS,
                recipient: [who; 32],
            },
        }
    }

    /// A chain that hands back a fixed set, in fixed-size steps, so the scan
    /// loop is exercised rather than assumed.
    struct Fake {
        tip: u64,
        step: u64,
        seen: Vec<Sighting>,
        calls: RefCell<u32>,
    }

    impl Fake {
        fn new(tip: u64, seen: Vec<Sighting>) -> Fake {
            Fake { tip, step: 400, seen, calls: RefCell::new(0) }
        }
    }

    impl Chain for Fake {
        fn tip(&self) -> Result<u64, String> {
            Ok(self.tip)
        }
        fn cave_payments(&self, from: u64, to: u64) -> Result<(u64, Vec<Sighting>), String> {
            *self.calls.borrow_mut() += 1;
            let end = to.min(from + self.step - 1);
            let batch = self
                .seen
                .iter()
                .filter(|s| s.height >= from && s.height <= end)
                .copied()
                .collect();
            Ok((end, batch))
        }
        fn block_hash(&self, _height: u64) -> Result<Hash, String> {
            Ok([0x5b; 32])
        }
    }

    /// The transport must not be reached for a policy with no reveal.
    fn no_network(_: &str) -> Result<Vec<u8>, String> {
        panic!("a sequential draw fetched a beacon it does not use");
    }

    #[test]
    fn a_scan_walks_the_whole_range_in_the_steps_the_scanner_allows() {
        let seen = vec![
            paid(1, 0xa1, 3_499_050, ENTRY),
            // Far enough away that one 400-block step cannot cover both.
            paid(2, 0xb2, 3_499_900, ENTRY),
        ];
        let chain = Fake::new(3_500_110, seen);
        let out = execute(&plan(), &policy(IdentityPolicy::Sequential), &manifest(), &chain, &no_network)
            .expect("the draw runs");
        assert_eq!(out.seen, 2, "both payments found across separate steps");
        assert!(*chain.calls.borrow() > 1, "the range needed more than one call");
        assert_eq!(out.run.tickets, 2);
    }

    #[test]
    fn the_draw_is_the_same_whichever_order_the_scanner_reported() {
        let a = vec![paid(1, 0xa1, 3_499_050, ENTRY), paid(2, 0xb2, 3_499_060, ENTRY)];
        let b = vec![a[1], a[0]];
        let p = policy(IdentityPolicy::Sequential);
        let one = execute(&plan(), &p, &manifest(), &Fake::new(3_500_110, a), &no_network).unwrap();
        let two = execute(&plan(), &p, &manifest(), &Fake::new(3_500_110, b), &no_network).unwrap();
        assert!(
            one.run.agrees_with(&two.run),
            "two operators over one close must reach one assignment"
        );
    }

    #[test]
    fn it_refuses_to_draw_before_the_close_is_buried() {
        // Past the close, but only by 2 of the 6 confirmations required.
        let chain = Fake::new(CLOSE as u64 + 2, vec![paid(1, 0xa1, 3_499_050, ENTRY)]);
        let e = execute(&plan(), &policy(IdentityPolicy::Sequential), &manifest(), &chain, &no_network)
            .expect_err("a reorg could still change who paid");
        assert!(e.contains("has not buried the close"), "{e}");
    }

    #[test]
    fn it_refuses_when_the_reveal_block_does_not_exist_yet() {
        let mut p = plan();
        p.reveal_zcash_height = 3_600_000;
        let chain = Fake::new(3_500_110, vec![paid(1, 0xa1, 3_499_050, ENTRY)]);
        let e = execute(&p, &policy(IdentityPolicy::Sequential), &manifest(), &chain, &no_network)
            .expect_err("the reveal block is in the future");
        assert!(e.contains("does not exist yet"), "{e}");
    }

    #[test]
    fn a_refused_payment_is_named_with_its_reason_and_costs_nobody_else() {
        let seen = vec![
            paid(1, 0xa1, 3_499_050, ENTRY),
            paid(2, 0xb2, 3_499_060, ENTRY - 1), // underpaid by a zatoshi
        ];
        let out = execute(&plan(), &policy(IdentityPolicy::Sequential), &manifest(), &Fake::new(3_500_110, seen), &no_network)
            .expect("one good payment is still a sale");
        assert_eq!(out.run.tickets, 1, "the underpayment did not become a ticket");
        assert_eq!(out.refused.len(), 1);
        assert!(out.report().contains("owed an answer"), "the report names them");
        assert!(out.report().contains("refundable by hand"), "and says why");
        assert_eq!(out.run.serials_for(&[0xa1; 32]).len(), 1, "the payer who was fine is unaffected");
    }

    #[test]
    fn every_payment_refused_reports_why_rather_than_an_empty_sale() {
        let seen = vec![paid(1, 0xa1, 3_499_050, ENTRY - 1)];
        let e = execute(&plan(), &policy(IdentityPolicy::Sequential), &manifest(), &Fake::new(3_500_110, seen), &no_network)
            .expect_err("nothing admissible");
        assert!(e.contains("all 1 payment(s) seen were refused"), "{e}");
        assert!(e.contains("refundable by hand"), "{e}");
    }

    #[test]
    fn a_scanner_that_will_not_advance_is_an_error_rather_than_a_spin() {
        struct Stuck;
        impl Chain for Stuck {
            fn tip(&self) -> Result<u64, String> {
                Ok(3_500_110)
            }
            fn cave_payments(&self, from: u64, _to: u64) -> Result<(u64, Vec<Sighting>), String> {
                Ok((from - 1, Vec::new()))
            }
            fn block_hash(&self, _: u64) -> Result<Hash, String> {
                Ok([0x5b; 32])
            }
        }
        let e = execute(&plan(), &policy(IdentityPolicy::Sequential), &manifest(), &Stuck, &no_network)
            .expect_err("a scan that cannot progress must say so");
        assert!(e.contains("check its start height"), "{e}");
    }

    #[test]
    fn a_concealed_shuffle_reaches_drand_for_the_announced_round() {
        let asked = RefCell::new(String::new());
        let get = |url: &str| -> Result<Vec<u8>, String> {
            *asked.borrow_mut() = url.to_string();
            Err("beacon unreachable".into())
        };
        let chain = Fake::new(3_500_110, vec![paid(1, 0xa1, 3_499_050, ENTRY)]);
        let e = execute(&plan(), &policy(IdentityPolicy::ConcealedShuffle), &manifest(), &chain, &get)
            .expect_err("no beacon, no draw");
        assert!(e.contains("beacon unreachable"), "{e}");
        assert!(
            asked.borrow().ends_with("/public/5002400"),
            "it must ask for the announced round, got {}",
            asked.borrow()
        );
    }
}

#[cfg(test)]
mod announced {
    use super::tests::{manifest, policy, TERMS};
    use super::*;
    use cave_randomness::AnnouncedClose;

    fn announced_policy() -> AllocationPolicy {
        let mut p = policy(IdentityPolicy::Sequential);
        p.deposit_close_zcash_height = 0;
        p.randomness.announced_close = Some(AnnouncedClose {
            reveal_offset_blocks: 100,
            reveal_offset_rounds: 2_400,
            min_notice_blocks: 1_152,
        });
        p
    }

    fn announcement(p: &AllocationPolicy, at: u32, close: u32) -> CloseAnnouncement {
        CloseAnnouncement {
            policy_digest: p.digest().unwrap(),
            announced_at_zcash_height: at,
            announced_at_drand_round: 5_000_000,
            deposit_close_zcash_height: close,
        }
    }

    #[test]
    fn the_reveal_is_derived_from_the_announcement_not_chosen() {
        let p = announced_policy();
        let a = announcement(&p, 3_490_000, 3_500_000);
        let proof = PublicationProof { txid: [0x11; 32], height: 3_490_000 };
        let plan = Plan::from_announcement(&p, &a, &proof, 6, 3_489_000, TERMS).unwrap();
        assert_eq!(plan.close_height, 3_500_000);
        assert_eq!(plan.reveal_zcash_height, 3_500_100, "close + the committed 100");
        assert_eq!(plan.reveal_drand_round, 5_002_400, "announced round + the committed 2400");
        assert_eq!(plan.issued_at_drand_round, 5_000_000, "tickets bind the announcement's round");
    }

    #[test]
    fn an_announcement_the_chain_does_not_carry_at_that_height_is_refused() {
        let p = announced_policy();
        let a = announcement(&p, 3_490_000, 3_500_000);
        // The memo confirmed somewhere else: backdating is exactly this shape.
        let proof = PublicationProof { txid: [0x11; 32], height: 3_495_000 };
        let e = Plan::from_announcement(&p, &a, &proof, 6, 3_489_000, TERMS).unwrap_err();
        assert!(e.contains("does not resolve"), "{e}");
    }

    #[test]
    fn a_close_with_less_notice_than_promised_is_refused() {
        let p = announced_policy();
        // 1,000 blocks of notice where the policy committed to 1,152.
        let a = announcement(&p, 3_499_000, 3_500_000);
        let proof = PublicationProof { txid: [0x11; 32], height: 3_499_000 };
        let e = Plan::from_announcement(&p, &a, &proof, 6, 3_489_000, TERMS).unwrap_err();
        assert!(e.contains("does not resolve"), "{e}");
    }

    #[test]
    fn an_announcement_for_another_policy_cannot_be_lifted_onto_this_one() {
        let p = announced_policy();
        let mut other = announced_policy();
        other.supply = 21;
        let a = announcement(&other, 3_490_000, 3_500_000);
        let proof = PublicationProof { txid: [0x11; 32], height: 3_490_000 };
        let e = Plan::from_announcement(&p, &a, &proof, 6, 3_489_000, TERMS).unwrap_err();
        assert!(e.contains("does not resolve"), "{e}");
    }
}
