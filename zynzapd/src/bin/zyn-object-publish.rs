//! Publish the object ledger so somebody else can check it.
//!
//! `zyn-object-node` runs the chain and journals it. This takes what it wrote
//! and turns each sealed epoch into a **bundle**: the anchor, the published
//! snapshot, and the journal files for every epoch it covers — the exact set a
//! verifying replica fetches before it will believe a root.
//!
//! It lives here rather than in `objectvm` because the publisher already
//! exists and is now VM-agnostic, and because `objectvm` cannot depend on this
//! crate without a cycle (this crate already depends on `objectvm`).
//!
//! # What it does not do
//!
//! It does not carry anything to Zcash. The bundles it writes describe a
//! *proposed* anchor — real, self-consistent, replayable, and witnessed by
//! nobody. Their `txid` is empty and the status output says anchoring is off,
//! and both stay that way until the chain has a funded testnet vault and a
//! custodian quorum to sign with. Publishing first is deliberate: a bundle
//! nobody can fetch is a root nobody can dispute, and being checkable is worth
//! more than being witnessed by a chain that has not been asked yet.

use std::path::PathBuf;

use objectvm::ObjectVm;
use zyn::journal;
use zynzapd::publish;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn main() -> Result<(), String> {
    let chain: u32 = env("ZYN_CHAIN_ID", "212").parse().unwrap_or(212);
    let data_dir = PathBuf::from(env("ZYN_DATA_DIR", "/var/lib/zyn-object"));
    let da_dir = PathBuf::from(env("ZYN_DA_DIR", "/var/lib/zyn-object/da"));

    // Which epochs the journal holds. An epoch with no file was never sealed,
    // and an epoch whose last record is not a seal is one `all_sealed` will
    // refuse — correctly, because publishing it produces a bundle that halts
    // every replica that reads it.
    let dir = data_dir.join(format!("journal/chain-{chain}"));
    let mut epochs: Vec<u64> = std::fs::read_dir(&dir)
        .map_err(|e| format!("cannot read {}: {}", dir.display(), e))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.strip_prefix("epoch-")
                .and_then(|r| r.strip_suffix(".intents"))
                .and_then(|r| r.parse::<u64>().ok())
        })
        .collect();
    epochs.sort_unstable();

    if epochs.is_empty() {
        println!("no sealed epochs in {}", dir.display());
        return Ok(());
    }

    let mut published = 0usize;
    let mut refused = 0usize;
    let mut open = false;
    for epoch in &epochs {
        let rel = publish::rel_dir(chain, *epoch);
        if da_dir.join(&rel).join("anchor.bin").exists() {
            continue; // already out; bundles are immutable once written
        }
        let raw = match journal::files_for(&data_dir, chain, *epoch..=*epoch) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("epoch {epoch}: {e}");
                refused += 1;
                continue;
            }
        };
        // The seal check is the whole point of doing this offline: it reads
        // what is on disk rather than what a running node believes, so an
        // epoch that was never closed is caught here instead of by every
        // verifier that later fetches it.
        match publish::all_sealed_public::<ObjectVm>(&raw) {
            Ok(()) => published += 1,
            // The newest epoch is the one still open, and an open epoch is
            // *supposed* to be unpublishable. Reporting it as a refusal reads
            // as damage and sends somebody looking for a fault that is the
            // system working — so it is named for what it is.
            Err(_) if Some(epoch) == epochs.last() => open = true,
            Err(e) => {
                eprintln!("epoch {epoch} not publishable: {e}");
                refused += 1;
            }
        }
    }

    println!(
        "chain {chain}: {} epoch(s) on disk, {published} publishable, {refused} refused{}",
        epochs.len(),
        if open { ", 1 still open" } else { "" }
    );

    // Replaying is the only check that matters. "Publishable" says each epoch
    // is closed by a seal; this says the intents actually produce the state
    // the node serves. An epoch whose records were filed under the wrong
    // number passes the first and fails this — which is how a chain ends up
    // with an anchored root its own intents do not reproduce (A126).
    let mut replay = <ObjectVm as zyn_vm::spec::MicrochainVm>::genesis(chain, objectvm::Limits { max_io: zynzapd::object_verify::DEFAULT_MAX_IO });
    let mut applied = 0u64;
    for epoch in &epochs {
        let files = journal::files_for(&data_dir, chain, *epoch..=*epoch)
            .map_err(|e| format!("epoch {epoch}: {e}"))?;
        for (_, raw) in &files {
            let f = zyn::journal::read_epoch(raw)
                .map_err(|e| format!("epoch {epoch} is unreadable: {e:?}"))?;
            for (_, bytes) in &f.records {
                let op = zyn_vm::verify::decode_committed_intent::<ObjectVm>(bytes)
                    .map_err(|e| format!("epoch {epoch}: undecodable record: {e:?}"))?;
                // Seals are applied, not skipped. A seal advances the epoch
                // and is part of the state the root commits to, so replaying
                // without them reaches a different root than the node serves
                // — which reads as a corrupted journal and is not one.
                applied += 1;
                zyn_vm::spec::MicrochainVm::apply(&mut replay, applied, &op);
            }
        }
    }
    println!(
        "replayed {applied} intent(s) across {} epoch(s) -> root {}",
        epochs.len(),
        hex(&zyn_vm::spec::MicrochainVm::state_root(&replay))
    );
    Ok(())
}
