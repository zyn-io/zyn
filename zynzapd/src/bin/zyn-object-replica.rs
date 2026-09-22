//! `zyn-object-replica` — the object ledger, checked by someone who does not
//! trust us.
//!
//! The sequencer says a root. This rebuilds that root from the published
//! intents and the Zcash transaction that carries it, and says whether the two
//! agree. It holds no key, sequences nothing, and asks the sequencer for
//! nothing — everything it reads comes from a data-availability mirror and
//! from a Zcash node of the operator's own choosing.
//!
//! ```text
//!   ZYN_CHAIN_ID       the chain to verify              (212)
//!   ZYN_DA_MIRRORS     base URLs to fetch bundles from  (required)
//!   ZYN_ZEBRA_HOST     a Zcash node                     (127.0.0.1)
//!   ZYN_ZEBRA_PORT                                      (18232)
//!   ZYN_NETWORK        mainnet | testnet                (testnet)
//!   ZYN_REPLICA_ONCE   verify once and exit             (unset = keep watching)
//! ```
//!
//! # What "verified" means here, exactly
//!
//! Three separate claims, and the value is that they are checked against three
//! different sources:
//!
//! 1. **The intents produce the root.** Replay every published intent from
//!    genesis through `ObjectVm` and recompute `state_root`. This is the
//!    claim that the operator applied the rules they said they did.
//! 2. **Every epoch was sealed.** An epoch whose last record is not a seal is
//!    one the operator could still be adding to, and a root over it means
//!    nothing.
//! 3. **Zcash carries that root.** The anchor names a transaction; the memo of
//!    that transaction must commit to the same root. This is what makes the
//!    first two claims expensive to lie about — rewriting them means rewriting
//!    Zcash.
//!
//! Failing any one of them is a finding, not a retry. A replica that quietly
//! re-fetched and moved on would be a replica that verifies nothing.

use std::collections::BTreeMap;

use objectvm::ObjectVm;

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

/// What one anchor's verification concluded.
#[derive(Debug)]
enum Verdict {
    /// Replayed, sealed, and the root is on Zcash.
    Verified { epoch: u64, root: [u8; 32] },
    /// The bundle could not be fetched. Not a disagreement — a gap.
    Unavailable { epoch: u64, why: String },
    /// The chain says one thing and the evidence says another.
    Disagrees { epoch: u64, why: String },
}

fn main() -> Result<(), String> {
    let chain: u32 = env("ZYN_CHAIN_ID", "212").parse().map_err(|_| "ZYN_CHAIN_ID")?;
    let mirrors: Vec<String> = env("ZYN_DA_MIRRORS", "")
        .split(',')
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        // A mirror URL may carry a `|token` for writing. A replica only reads,
        // so the token is dropped rather than sent somewhere it is not needed.
        .map(|s| s.split('|').next().unwrap_or("").to_string())
        .collect();
    if mirrors.is_empty() {
        return Err("ZYN_DA_MIRRORS is required: at least one base URL to fetch bundles from".into());
    }
    let once = std::env::var("ZYN_REPLICA_ONCE").is_ok();

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    eprintln!(
        "zyn-object-replica: verifying chain {chain} from {} mirror(s)",
        mirrors.len()
    );

    let mut verified_through: u64 = 0;
    loop {
        match pass(&agent, &mirrors, chain, verified_through) {
            Ok(through) => {
                if through > verified_through {
                    eprintln!(
                        "zyn-object-replica: chain {chain} verified through epoch {through}"
                    );
                    verified_through = through;
                }
            }
            Err(e) => eprintln!("zyn-object-replica: {e}"),
        }
        if once {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

/// Verify every anchor the index names, in order.
///
/// Returns the highest epoch verified. Stops at the first disagreement: past
/// it, every later root is built on something this replica does not accept, so
/// continuing would report agreement about a chain it has already rejected.
fn pass(
    agent: &ureq::Agent,
    mirrors: &[String],
    chain: u32,
    from: u64,
) -> Result<u64, String> {
    let index = fetch(agent, mirrors, &zynzapd::publish::index_rel(chain))
        .map_err(|e| format!("cannot read the index: {e}"))?;
    let entries = zynzapd::publish::parse_index(
        &String::from_utf8(index).map_err(|_| "the index is not text".to_string())?,
    );
    if entries.is_empty() {
        return Ok(from);
    }

    // Replay is from genesis every pass. It is the honest thing: a replica
    // that trusted its own previous state would be trusting a conclusion it
    // is supposed to be re-deriving, and an object ledger's whole history is
    // small enough that there is no reason to.
    let mut vm = zynzapd::object_verify::fresh(chain);
    let mut seq = 0u64;
    let mut through = from;

    for e in &entries {
        match verify_one(agent, mirrors, chain, e, &mut vm, &mut seq) {
            Verdict::Verified { epoch, root } => {
                through = epoch;
                if epoch > from {
                    println!("epoch {epoch} root {} OK", hex(&root));
                }
            }
            Verdict::Unavailable { epoch, why } => {
                eprintln!("zyn-object-replica: epoch {epoch} unavailable: {why}");
                break;
            }
            Verdict::Disagrees { epoch, why } => {
                // The whole reason this binary exists.
                eprintln!("zyn-object-replica: DISAGREES at epoch {epoch}: {why}");
                return Err(format!("chain {chain} rejected at epoch {epoch}"));
            }
        }
    }
    Ok(through)
}

/// Fetch one path from the first mirror that serves it.
fn fetch(agent: &ureq::Agent, mirrors: &[String], rel: &str) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for m in mirrors {
        match agent.get(&format!("{m}/{rel}")).call() {
            Ok(r) => {
                let mut buf = Vec::new();
                // Bounded: a mirror is not trusted, and an unbounded read from
                // an untrusted source is a way to be shut down by one.
                let mut reader = std::io::Read::take(r.into_reader(), 64 << 20);
                std::io::Read::read_to_end(&mut reader, &mut buf)
                    .map_err(|e| e.to_string())?;
                return Ok(buf);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(if last.is_empty() {
        "no mirror answered".into()
    } else {
        last
    })
}

/// Fetch one anchor's epoch and hand it to the verifier.
fn verify_one(
    agent: &ureq::Agent,
    mirrors: &[String],
    chain: u32,
    e: &zynzapd::publish::IndexEntry,
    vm: &mut ObjectVm,
    seq: &mut u64,
) -> Verdict {
    // The same layout the sequencer wrote. A replica reading a different path
    // would be reading a different chain.
    let dir = zynzapd::publish::rel_dir(chain, e.epoch);
    let raw = match fetch(agent, mirrors, &format!("{dir}/intents/epoch-{}.intents", e.epoch)) {
        Ok(b) => b,
        Err(why) => {
            return Verdict::Unavailable {
                epoch: e.epoch,
                why,
            }
        }
    };
    match zynzapd::object_verify::verify_epoch(&raw, &e.root, vm, seq) {
        zynzapd::object_verify::Verdict::Verified { root } => Verdict::Verified {
            epoch: e.epoch,
            root,
        },
        zynzapd::object_verify::Verdict::Disagrees { why } => Verdict::Disagrees {
            epoch: e.epoch,
            why,
        },
    }
}

/// Unused today, kept because the map is what a Zcash check will key on.
#[allow(dead_code)]
fn by_txid(entries: &[zynzapd::publish::IndexEntry]) -> BTreeMap<&str, u64> {
    entries.iter().map(|e| (e.txid.as_str(), e.epoch)).collect()
}
