//! Running the draw, once, for a whole collection.
//!
//! `cave_randomness::allocate` takes the **entire** ticket ledger and returns
//! one batch covering every ticket. It is not a per-buyer call, and treating it
//! as one is the mistake this module exists to make impossible.
//!
//! `sale::settle` asks for `allocate(n) -> Vec<u32>` per order, which is the
//! right shape for a sequential or instant-random launch and the wrong shape
//! for a concealed shuffle. Calling the collection-wide draw once per buyer
//! would re-run it against whatever ledger existed at that moment — so two
//! buyers settling either side of a new deposit would receive items from two
//! different allocations, both internally consistent and mutually contradictory
//! (§104.1). Nobody would notice until two people claimed the same serial.
//!
//! So the draw happens once, here, after the reveal. Orders then *look up* what
//! they were given rather than asking for it.
//!
//! # Once, and provably so
//!
//! A [`Run`] records the assignment root it produced and the ledger it ran
//! over. Running again with a different ledger is not an error to be retried —
//! it is a different sale, and [`Run::agrees_with`] is what a caller uses to
//! find out before anything is minted.

use std::collections::BTreeMap;

use cave_randomness::{
    allocate, AllocationBatch, AllocationPolicy, Hash, ManifestEntry, RevealEvidence,
    TicketEvidence,
};

/// One completed draw.
#[derive(Clone, Debug)]
pub struct Run {
    pub batch: AllocationBatch,
    /// How many tickets it covered. A later run over more tickets is a
    /// different allocation, not a continuation of this one.
    pub tickets: usize,
}

/// Why a draw could not be run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No tickets: an allocation over nobody is not an empty success, it is a
    /// sale that has not happened.
    NoTickets,
    /// A concealed shuffle needs its reveal before it can assign anything.
    NeedsReveal,
    /// The manifest does not cover the supply the policy claims.
    ManifestMismatch { entries: usize, supply: u32 },
    /// The engine refused. Carried verbatim: these are the interesting ones.
    Engine(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NoTickets => write!(f, "no tickets to allocate over"),
            Refused::NeedsReveal => {
                write!(f, "a concealed shuffle cannot assign items before its reveal")
            }
            Refused::ManifestMismatch { entries, supply } => write!(
                f,
                "the manifest has {entries} entries for a supply of {supply}"
            ),
            Refused::Engine(e) => write!(f, "{e}"),
        }
    }
}

/// Run the draw over the whole ledger.
///
/// `reveal` is required for a concealed shuffle and refused for the others, so
/// a caller cannot quietly allocate a concealed collection early by passing
/// `None` and getting sequential behaviour.
pub fn run(
    policy: &AllocationPolicy,
    manifest: &[ManifestEntry],
    tickets: &[TicketEvidence],
    reveal: Option<&RevealEvidence>,
) -> Result<Run, Refused> {
    run_resolved(policy, manifest, tickets, reveal, None)
}

/// The same draw, for a policy whose close was announced rather than committed.
///
/// `targets` carries what the announcement resolved to. A policy that announces
/// its close holds a close height of zero and no reveal targets, so without
/// these the allocator measures every ticket against zero and refuses the whole
/// sale — which is what Collection Z's real policy did until §113.
pub fn run_resolved(
    policy: &AllocationPolicy,
    manifest: &[ManifestEntry],
    tickets: &[TicketEvidence],
    reveal: Option<&RevealEvidence>,
    targets: Option<&cave_randomness::ResolvedClose>,
) -> Result<Run, Refused> {
    if tickets.is_empty() {
        return Err(Refused::NoTickets);
    }
    if manifest.len() != policy.supply as usize {
        return Err(Refused::ManifestMismatch {
            entries: manifest.len(),
            supply: policy.supply,
        });
    }
    if policy.identity == cave_randomness::IdentityPolicy::ConcealedShuffle && reveal.is_none() {
        return Err(Refused::NeedsReveal);
    }
    let batch = match targets {
        Some(t) => cave_randomness::allocate_announced(policy, manifest, tickets, reveal, t),
        None => allocate(policy, manifest, tickets, reveal),
    }
    .map_err(|e| Refused::Engine(e.to_string()))?;
    Ok(Run {
        tickets: tickets.len(),
        batch,
    })
}

impl Run {
    /// Which serials an account was allocated, ascending.
    ///
    /// An account with no ticket gets an empty list rather than an error: not
    /// being in a draw is an ordinary answer, and a caller that treats absence
    /// as failure will report a sale as broken to everyone who did not enter.
    pub fn serials_for(&self, account: &Hash) -> Vec<u32> {
        let mut out: Vec<u32> = self
            .batch
            .receipts
            .iter()
            .filter(|r| r.ticket.recipient == *account)
            .flat_map(|r| r.items.iter().map(|i| i.global_index))
            .collect();
        out.sort_unstable();
        out
    }

    /// Every account that received something, with its serials.
    pub fn by_account(&self) -> BTreeMap<Hash, Vec<u32>> {
        let mut out: BTreeMap<Hash, Vec<u32>> = BTreeMap::new();
        for r in &self.batch.receipts {
            let e = out.entry(r.ticket.recipient).or_default();
            e.extend(r.items.iter().map(|i| i.global_index));
        }
        for v in out.values_mut() {
            v.sort_unstable();
        }
        out
    }

    /// Whether another run assigned the same items to the same people.
    ///
    /// The assignment root covers the whole result, so this is one comparison
    /// rather than a walk — and it is the check to make before minting against
    /// a run you did not just produce.
    pub fn agrees_with(&self, other: &Run) -> bool {
        self.batch.assignment_root == other.batch.assignment_root
    }

    /// Items nobody was allocated. These roll into the open mint (§64.17).
    pub fn unallocated(&self) -> &[u32] {
        &self.batch.unused_indices
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cave_randomness::{
        AnnouncedClose, IdentityPolicy, QuantityPolicy, RandomnessPlan, Ticket, WeightedOutcome,
        QUICKNET_CHAIN_HASH,
    };
    use zyn_vm::derive::ADDRESS_SCOPE_V1;

    const COLLECTION: Hash = [2u8; 32];

    fn manifest(n: u32) -> Vec<ManifestEntry> {
        (0..n)
            .map(|i| ManifestEntry {
                global_index: i,
                item_id: zyn_vm::item_address(ADDRESS_SCOPE_V1, &COLLECTION, i),
                content_hash: {
                    let mut h = [0u8; 32];
                    h[..4].copy_from_slice(&i.to_be_bytes());
                    h[31] = 1;
                    h
                },
            })
            .collect()
    }

    /// Sequential, so the tests can run the draw without drand evidence. The
    /// concealed path is exercised by cave-randomness' own suite; what is
    /// under test here is the once-per-sale discipline around it.
    fn policy(supply: u32) -> AllocationPolicy {
        AllocationPolicy {
            collection_id: COLLECTION,
            supply,
            entry_price_zatoshi: 1_000_000,
            deposit_close_zcash_height: 3_500_000,
            quantity: QuantityPolicy::Fixed { items: 1 },
            identity: IdentityPolicy::Sequential,
            manifest_root: cave_randomness::manifest_root(COLLECTION, &manifest(supply)).unwrap(),
            randomness: RandomnessPlan {
                drand_chain_hash: QUICKNET_CHAIN_HASH,
                ticket_round_delay: 10,
                reveal_drand_round: None,
                reveal_zcash_height: None,
                announced_close: None,
            },
        }
    }

    fn ticket(p: &AllocationPolicy, txid: u8, who: u8, height: u32) -> TicketEvidence {
        let mut t = Ticket {
            ticket_id: [0u8; 32],
            deposit_txid: [txid; 32],
            deposit_output_index: 0,
            deposit_amount_zatoshi: p.entry_price_zatoshi,
            recipient: [who; 32],
            confirmed_height: height,
            tx_index: 0,
            issued_at_drand_round: 5_000_000,
            quantity_drand_round: None,
            identity_drand_round: None,
            requested_items: None,
        };
        t.ticket_id = cave_randomness::derive_ticket_id(p.digest().unwrap(), &t);
        TicketEvidence {
            ticket: t,
            quantity_beacon: None,
            identity_beacon: None,
        }
    }

    #[test]
    fn a_draw_assigns_every_ticket_and_reports_the_rest() {
        let p = policy(10);
        let ts = vec![ticket(&p, 1, 0xa1, 100), ticket(&p, 2, 0xb2, 101)];
        let r = run(&p, &manifest(10), &ts, None).unwrap();
        assert_eq!(r.tickets, 2);
        assert_eq!(r.serials_for(&[0xa1; 32]).len(), 1);
        assert_eq!(r.serials_for(&[0xb2; 32]).len(), 1);
        // Eight items nobody entered for; they roll into the open mint.
        assert_eq!(r.unallocated().len(), 8);
    }

    /// The property the whole module exists for: the same ledger draws the
    /// same result, and a *different* ledger is visibly a different draw
    /// rather than an extension of the first.
    #[test]
    fn the_same_ledger_draws_the_same_result_and_a_changed_one_does_not() {
        let p = policy(10);
        let ts = vec![ticket(&p, 1, 0xa1, 100), ticket(&p, 2, 0xb2, 101)];
        let first = run(&p, &manifest(10), &ts, None).unwrap();
        let again = run(&p, &manifest(10), &ts, None).unwrap();
        assert!(first.agrees_with(&again), "a rerun over one ledger is stable");

        let mut more = ts.clone();
        more.push(ticket(&p, 3, 0xc3, 102));
        let after = run(&p, &manifest(10), &more, None).unwrap();
        assert!(
            !first.agrees_with(&after),
            "a ledger that grew is a different allocation, and must be visible as one"
        );
    }

    #[test]
    fn an_account_that_did_not_enter_gets_an_empty_answer_not_an_error() {
        let p = policy(10);
        let ts = vec![ticket(&p, 1, 0xa1, 100)];
        let r = run(&p, &manifest(10), &ts, None).unwrap();
        assert!(r.serials_for(&[0xff; 32]).is_empty());
    }

    /// A concealed shuffle must not quietly fall back to sequential when the
    /// reveal is missing — that would assign items before the evidence that is
    /// supposed to decide them exists.
    #[test]
    fn a_concealed_collection_refuses_to_draw_before_its_reveal() {
        let mut p = policy(10);
        p.identity = IdentityPolicy::ConcealedShuffle;
        p.randomness.announced_close = Some(AnnouncedClose {
            reveal_offset_blocks: 100,
            reveal_offset_rounds: 2_400,
            min_notice_blocks: 1_152,
        });
        p.deposit_close_zcash_height = 0;
        let ts = vec![ticket(&p, 1, 0xa1, 100)];
        assert_eq!(run(&p, &manifest(10), &ts, None).unwrap_err(), Refused::NeedsReveal);
    }

    #[test]
    fn a_draw_over_nobody_and_a_short_manifest_are_separate_refusals() {
        let p = policy(10);
        assert_eq!(run(&p, &manifest(10), &[], None).unwrap_err(), Refused::NoTickets);
        let ts = vec![ticket(&p, 1, 0xa1, 100)];
        assert_eq!(
            run(&p, &manifest(9), &ts, None).unwrap_err(),
            Refused::ManifestMismatch { entries: 9, supply: 10 }
        );
    }

    /// Two tickets from one buyer collect into one list, because a buyer asks
    /// "what did I get", not "what did each of my payments get".
    #[test]
    fn one_buyer_with_two_tickets_sees_both_serials() {
        let p = policy(10);
        let ts = vec![ticket(&p, 1, 0xa1, 100), ticket(&p, 2, 0xa1, 101)];
        let r = run(&p, &manifest(10), &ts, None).unwrap();
        let mine = r.serials_for(&[0xa1; 32]);
        assert_eq!(mine.len(), 2);
        assert!(mine[0] < mine[1], "ascending, and no duplicates");
        assert_eq!(r.by_account()[&[0xa1; 32]], mine);
    }
}
