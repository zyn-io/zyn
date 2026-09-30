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

use objectvm::{Limits, ObjectVm, Receipt};
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

/// The I/O limit an object ledger runs with unless its operator set
/// `ZYN_MAX_IO`. It is part of the state root, so a verifier that assumes
/// another one disagrees with an honest chain.
pub const DEFAULT_MAX_IO: u32 = 16;

/// Replay one sealed epoch's journal into `vm`, and return the root its seal
/// committed — the checkpoint root, taken before the seal advances the epoch,
/// which is the root an anchor carries.
///
/// Refuses an epoch whose last record is not a seal before replaying anything:
/// an epoch the operator could still be adding to commits to nothing. Seals
/// are replayed with everything else — a seal advances the epoch and is part
/// of what the next root commits to. Every record is applied at the sequence
/// number the journal gives it, and a gap is refused: the VM numbers its
/// objects by it, so a missing intent is a different history.
pub fn replay_epoch(raw: &[u8], vm: &mut ObjectVm, seq: &mut u64) -> Result<[u8; 32], String> {
    let f = read_epoch(raw).map_err(|e| format!("the journal does not parse: {e:?}"))?;
    match f.records.last() {
        None => return Err("the epoch is empty".into()),
        Some((_, bytes)) => match zyn_vm::verify::decode_committed_intent::<ObjectVm>(bytes) {
            Ok(op) if <ObjectVm as MicrochainVm>::is_seal(&op) => {}
            Ok(_) => return Err("the last record is not a seal, so the epoch was never closed".into()),
            Err(e) => return Err(format!("the last record is not an authorized intent: {e:?}")),
        },
    }
    let mut committed = None;
    for (at, bytes) in &f.records {
        if *at != *seq + 1 {
            return Err(format!("the journal jumps from seq {} to {at}", *seq));
        }
        let op = zyn_vm::verify::decode_committed_intent::<ObjectVm>(bytes)
            .map_err(|e| format!("a record is not an authorized intent: {e:?}"))?;
        *seq = *at;
        for r in vm.apply(*at, &op) {
            if let Receipt::Sealed(cp) = r {
                committed = Some(cp.state_root);
            }
        }
    }
    committed.ok_or_else(|| "the seal was refused, so the epoch never closed".into())
}

/// Replay every epoch one anchor covers, in order, and check the root it
/// reaches against the one the anchor committed.
///
/// An anchor is daily and a seal is every minute or so, so one anchor covers
/// many epochs. Only the last epoch's root is anchored; the others are checked
/// for being sealed and are what gets the state to that root.
pub fn verify_anchor(
    epochs: &[Vec<u8>],
    expected: &[u8; 32],
    vm: &mut ObjectVm,
    seq: &mut u64,
) -> Verdict {
    if epochs.is_empty() {
        return Verdict::Disagrees {
            why: "an anchor that covers no epochs".into(),
        };
    }
    let mut got = [0u8; 32];
    for (i, raw) in epochs.iter().enumerate() {
        match replay_epoch(raw, vm, seq) {
            Ok(root) => got = root,
            Err(why) => {
                return Verdict::Disagrees {
                    why: format!("covered epoch #{i}: {why}"),
                }
            }
        }
    }
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

/// [`verify_anchor`] for an anchor covering a single epoch.
pub fn verify_epoch(
    raw: &[u8],
    expected: &[u8; 32],
    vm: &mut ObjectVm,
    seq: &mut u64,
) -> Verdict {
    verify_anchor(&[raw.to_vec()], expected, vm, seq)
}

/// Open a journal file fetched from a mirror.
///
/// Sealed files are opened with the DA key under the label derived from their
/// path, so a file moved to another epoch's directory does not open. A sealed
/// file with no key is an honest "cannot check" rather than a disagreement:
/// the chain may be fine and this verifier simply is not one of those allowed
/// to read it.
pub fn open_journal(
    bytes: Vec<u8>,
    rel: &str,
    chain: u32,
    key: Option<&[u8; 32]>,
) -> Result<Vec<u8>, String> {
    if !crate::dacrypt::is_sealed(&bytes) {
        return Ok(bytes);
    }
    let Some(k) = key else {
        return Err("the journal is sealed and this verifier holds no DA key".into());
    };
    crate::dacrypt::open(k, chain, &crate::dacrypt::label_for(rel), &bytes)
}

/// A fresh verifier for `chain`.
///
/// Replay starts at genesis every time, deliberately: a replica that trusted
/// its own previous state would be trusting the conclusion it is supposed to
/// be re-deriving.
pub fn fresh(chain: u32, max_io: u32) -> ObjectVm {
    ObjectVm::genesis(chain, Limits { max_io })
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
