//! `zyn-inspect` — what an AMM chain holds, read from its own files.
//!
//! ```text
//!   zyn-inspect <data-dir> <chain-id>
//! ```
//!
//! The RPC answers "what does *my* account hold" behind a signature, which is
//! the right shape for a wallet and the wrong one for an operator deciding
//! whether a chain can be retired. A chain holds more than ZEC backing:
//! collections, items, listings — things a `backing == 0.0` check does not
//! count. This reads the saved state and the journal directly, holds no key,
//! and changes nothing.

use std::collections::{BTreeMap, BTreeSet};

use swapvm::state::SwapState;
use swapvm::tx::Intent;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().ok_or("usage: zyn-inspect <data-dir> <chain-id>")?;
    let chain: u32 = args
        .next()
        .ok_or("usage: zyn-inspect <data-dir> <chain-id>")?
        .parse()
        .map_err(|_| "chain id must be a number")?;

    let store = zyn::store::FileStore::new(&dir).map_err(|e| format!("{e:?}"))?;
    let saved = store
        .load(chain)
        .map_err(|e| format!("{e:?}"))?
        .ok_or("no saved state for that chain in that directory")?;
    let s = SwapState::decode_state(&saved.blob).map_err(|e| format!("the saved state does not decode: {e:?}"))?;
    println!("chain {} epoch {} seq {}", s.chain_id, s.epoch, s.seq);
    println!("accounts {}  tokens {}  collections {}  offers {}", s.accounts.len(), s.tokens.len(), s.collections.len(), s.offers.len());

    // Which epoch minted each collection's items, from the journal.
    let mut minted_in: BTreeMap<[u8; 32], (u64, u64, u32)> = BTreeMap::new();
    let jdir = std::path::Path::new(&dir).join(format!("journal/chain-{chain}"));
    if let Ok(entries) = std::fs::read_dir(&jdir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(epoch) = name
                .strip_prefix("epoch-")
                .and_then(|r| r.strip_suffix(".intents"))
                .and_then(|n| n.parse::<u64>().ok())
            else {
                continue;
            };
            let Ok(raw) = std::fs::read(e.path()) else { continue };
            let Ok(f) = zyn::journal::read_epoch(&raw) else { continue };
            for (_, bytes) in &f.records {
                let Ok(op) = zyn_vm::verify::decode_committed_intent::<SwapState>(bytes) else {
                    continue;
                };
                let coll = match op {
                    Intent::MintCollectionItem { collection, .. } => collection,
                    Intent::MintCollectionItemLegacy { collection, .. } => collection,
                    _ => continue,
                };
                let r = minted_in.entry(coll).or_insert((u64::MAX, 0, 0));
                r.0 = r.0.min(epoch);
                r.1 = r.1.max(epoch);
                r.2 += 1;
            }
        }
    }

    for (cid, c) in &s.collections {
        let items: BTreeSet<[u8; 32]> = s
            .tokens
            .iter()
            .filter(|(_, t)| t.collection == Some(*cid))
            .map(|(a, _)| *a)
            .collect();
        let mut holders: BTreeMap<[u8; 32], u32> = BTreeMap::new();
        for (acct, a) in &s.accounts {
            for (asset, amount) in &a.balances {
                if items.contains(asset) && amount.0 > 0 {
                    *holders.entry(*acct).or_default() += 1;
                }
            }
        }
        let listed = s.offers.values().filter(|o| items.contains(&o.offer_asset)).count();
        println!();
        println!(
            "collection {} {}  cap {} minted {} outstanding {}  items-in-state {}",
            String::from_utf8_lossy(c.symbol.as_bytes()).trim_end_matches('\0'),
            hex(cid),
            c.cap,
            c.minted,
            c.outstanding,
            items.len()
        );
        println!("  creator {}", hex(&c.creator));
        match minted_in.get(cid) {
            Some((lo, hi, n)) => println!("  minted by {n} journal record(s) in epochs {lo}..={hi}"),
            None => println!("  no mint records found in the journal"),
        }
        for (acct, n) in &holders {
            println!("  holder {}  {n} item(s)", hex(acct));
        }
        if listed > 0 {
            println!("  listed in offers: {listed}");
        }
    }
    Ok(())
}
