//! Checking an object ledger's published root without trusting the operator.
//!
//! The sequencer says a root. Given the intents it published, this rebuilds
//! that root and says whether the two agree. It is the argument behind
//! "settled in batches on Zcash": a claim anyone can re-derive from data the
//! operator does not control, rather than a number they are asked to believe.
//!
//! Kept in the library rather than inside the replica binary so it can be
//! tested against a journal built here — and so a wallet, a store, or anyone
//! else can run the same check without running a replica.

use objectvm::{Limits, ObjectVm};
use zyn::journal::read_epoch;
use zyn_vm::spec::MicrochainVm;

/// What verifying one epoch concluded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Replayed to the root the chain published, and the epoch was sealed.
    Verified { root: [u8; 32] },
    /// The chain says one thing and its own published intents say another.
    ///
    /// Not a retry. A replica that re-fetched and moved on would be a replica
    /// that verifies nothing.
    Disagrees { why: String },
}

/// Replay one epoch's journal into `vm` and check it against `expected`.
///
/// `seq` carries across epochs because the VM refuses a gap: a replica
/// verifying epoch 7 has already applied everything before it.
///
/// Two checks, in this order, because the second is meaningless without the
/// first: an epoch whose last record is not a seal is one the operator could
/// still be adding to, so a root over it commits to nothing.
pub fn verify_epoch(
    raw: &[u8],
    expected: &[u8; 32],
    vm: &mut ObjectVm,
    seq: &mut u64,
) -> Verdict {
    let f = match read_epoch(raw) {
        Ok(f) => f,
        Err(e) => {
            return Verdict::Disagrees {
                why: format!("the journal does not parse: {e:?}"),
            }
        }
    };

    match f.records.last() {
        None => {
            return Verdict::Disagrees {
                why: "the epoch is empty".into(),
            }
        }
        Some((_, bytes)) => match zyn_vm::verify::decode_committed_intent::<ObjectVm>(bytes) {
            Ok(op) if <ObjectVm as MicrochainVm>::is_seal(&op) => {}
            Ok(_) => {
                return Verdict::Disagrees {
                    why: "the last record is not a seal, so the epoch was never closed".into(),
                }
            }
            Err(e) => {
                return Verdict::Disagrees {
                    why: format!("the last record is not an authorized intent: {e:?}"),
                }
            }
        },
    }

    // Seals included. A seal advances the epoch and is part of what the root
    // commits to, so a replay that skips them lands somewhere else and reads
    // as corruption when nothing is wrong.
    for (_, bytes) in &f.records {
        let op = match zyn_vm::verify::decode_committed_intent::<ObjectVm>(bytes) {
            Ok(op) => op,
            Err(e) => {
                return Verdict::Disagrees {
                    why: format!("a record is not an authorized intent: {e:?}"),
                }
            }
        };
        *seq += 1;
        vm.apply(*seq, &op);
    }

    let got = vm.state_root();
    if got != *expected {
        return Verdict::Disagrees {
            why: format!(
                "replaying the published intents gives root {} and the chain published {}",
                hex(&got),
                hex(expected)
            ),
        };
    }
    Verdict::Verified { root: got }
}

/// A fresh verifier for `chain`.
///
/// Replay starts at genesis every time, deliberately: a replica that trusted
/// its own previous state would be trusting the conclusion it is supposed to
/// be re-deriving.
pub fn fresh(chain: u32) -> ObjectVm {
    ObjectVm::genesis(chain, Limits { max_io: 8 })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

// ---------------------------------------------------------------------------
// The third claim: Zcash carries it.
// ---------------------------------------------------------------------------

/// What checking an anchor against Zcash concluded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OnChain {
    /// A Zcash transaction at this height carries exactly this anchor.
    Carried { height: u64 },
    /// The transaction exists and does not say what the chain claims, or no
    /// transaction carrying this anchor was found in the range searched.
    Missing { why: String },
}

/// Is the anchor the index names really on Zcash?
///
/// This is what makes the replay claim expensive to lie about. Replaying
/// intents proves the operator applied the rules they published; it says
/// nothing about *when*, and nothing stops them republishing a different
/// history tomorrow. An anchor buried in Zcash does: rewriting it means
/// rewriting Zcash.
///
/// Needs the vault's **viewing** key, which finds notes and cannot spend one.
/// A replica holding it can read the vault's own anchors and nothing else of
/// consequence — the same posture the AMM's replica runs in.
///
/// Searched around the height the index claims rather than at it exactly: the
/// index records the height the settler saw a confirmation at, and a reorg can
/// move a transaction by a block or two without making it a different anchor.
pub fn anchor_on_zcash(
    scanner: &zyn_custody::shielded::Scanner,
    zebra: &zyn_custody::zebra::Zebra,
    chain: u32,
    entry: &crate::publish::IndexEntry,
    slack: u64,
) -> OnChain {
    let from = entry.height.saturating_sub(slack);
    let to = entry.height.saturating_add(slack);
    let (_, sightings) = match scanner.anchors(zebra, from, to) {
        Ok(v) => v,
        Err(e) => {
            return OnChain::Missing {
                why: format!("cannot read Zcash {from}..={to}: {e:?}"),
            }
        }
    };
    for s in &sightings {
        let Some((seen_chain, seen_epoch, seen_id)) = zyn::anchor::Anchor::parse_memo(&s.memo)
        else {
            continue;
        };
        if seen_chain != chain || seen_epoch != entry.epoch {
            continue;
        }
        // The anchor id is a hash over the whole anchor, and the anchor
        // commits to the state root — so matching it ties the root this
        // replica just recomputed to a transaction in a Zcash block.
        if seen_id == entry.anchor_id {
            return OnChain::Carried { height: s.height };
        }
        return OnChain::Missing {
            why: format!(
                "Zcash carries anchor {} for epoch {} and the chain published {}",
                hex(&seen_id),
                entry.epoch,
                hex(&entry.anchor_id)
            ),
        };
    }
    OnChain::Missing {
        why: format!(
            "no anchor for epoch {} found on Zcash in {from}..={to}",
            entry.epoch
        ),
    }
}
