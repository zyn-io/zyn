//! Rewrite one anchor's journal files in its DA bundle, sealed and complete.
//!
//! For a bundle that went out before its chain had a `ZYN_DA_KEY`, or that
//! left out epochs because its sequencer restarted between a seal and the
//! anchor. It reads the journal the node keeps, seals each epoch under the
//! bundle's own path, and writes it into the DA directory. The anchor, the
//! snapshot and the certificate are not touched: they are commitments, and
//! the journal is what produces them — the replica still checks it does.
//!
//!   ZYN_DA_KEY=… zyn-da-reseal <anchor-epoch> <first-epoch> [<out-dir>…]
//!
//! Every `<out-dir>` (the node's DA directory, a mirror's root) gets the same
//! files. Nothing is printed but paths and sizes.

use std::path::PathBuf;

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return Err("usage: zyn-da-reseal <anchor-epoch> <first-epoch> <out-dir>…".into());
    }
    let anchor: u64 = args[0].parse().map_err(|_| "anchor epoch is a number")?;
    let first: u64 = args[1].parse().map_err(|_| "first epoch is a number")?;
    if first > anchor {
        return Err("first epoch is after the anchor".into());
    }
    let outs: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
    let chain: u32 = env("ZYN_CHAIN_ID", "212").parse().map_err(|_| "ZYN_CHAIN_ID")?;
    let data = PathBuf::from(env("ZYN_DATA_DIR", "/var/lib/zyn-object"));
    let key = zynzapd::dacrypt::key_from_hex(&std::env::var("ZYN_DA_KEY").map_err(|_| "no ZYN_DA_KEY")?)?;

    let dir = zynzapd::publish::rel_dir(chain, anchor);
    for n in first..=anchor {
        let plain = std::fs::read(zyn::journal::epoch_path(&data, chain, n))
            .map_err(|e| format!("epoch {n}: {e}"))?;
        zyn::journal::read_epoch(&plain).map_err(|e| format!("epoch {n} is malformed: {e:?}"))?;
        let rel = format!("{dir}/intents/epoch-{n}.intents");
        let sealed = zynzapd::dacrypt::seal(&key, chain, &zynzapd::dacrypt::label_for(&rel), &plain)?;
        for out in &outs {
            let path = out.join(&rel);
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            let tmp = path.with_extension("intents.tmp");
            std::fs::write(&tmp, &sealed).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            println!("{} ({} bytes sealed)", path.display(), sealed.len());
        }
    }
    Ok(())
}
