//! A node that runs the object ledger.
//!
//! `zynzapd` is wired to swapvm across thirteen files, and porting it is not
//! what the object ledger needs to become real — it needs a chain it runs on.
//! This is that: a sequencer, a place to send intents, and state that survives
//! a restart. No bridge, no Solana, no feeds, no launchpad, and deliberately
//! no anchoring — chains 11 and 26460 keep all of that and keep running while
//! this one is free to break (DECISIONS §133).
//!
//! ```text
//!   ZYN_CHAIN_ID       the chain                       (212)
//!   ZYN_LISTEN         JSON-RPC, read and write        (127.0.0.1:8399)
//!   ZYN_DATA_DIR       where state is kept             (./object-data)
//!   ZYN_EPOCH_SECS     seal at least this often        (10)
//!   ZYN_MAX_IO         objects per spend               (16)
//! ```
//!
//! # What it does not do, and why that is on purpose
//!
//! It does not anchor. An anchor is a Zcash transaction signed by a threshold
//! of custodians, and giving a development chain the ability to spend from a
//! vault is the wrong trade entirely. Roots here are sealed and served and go
//! no further, which the status output says in as many words so that nobody
//! reads a devnet root as a settled one.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use objectvm::{Limits, ObjectVm, Op};
use zyn::epoch::EpochPolicy;
use zyn::journal::Journal;
use zyn::{Economics, Node};
use zyn_vm::read::Decoder;
use zyn_vm::spec::MicrochainVm;
use zynzapd::anchor::Chain as _;

/// Everything the chain needs to reach Zcash. Absent means this is a devnet
/// and says so — a root that reaches no chain is a claim, not a settlement.
struct Anchoring {
    /// Walks the vault's note trees forward. Without it the trees stay at the
    /// height they were seeded and the vault's notes -- which exist on chain
    /// -- are invisible, so every anchor fails with "waiting for a spendable
    /// Ironwood note" while the money is sitting right there.
    scanner: zyn_custody::shielded::Scanner,
    notes: zyn_custody::shielded::PoolStores,
    zebra: zyn_custody::zebra::Zebra,
    confirmations: u64,
    settler: zynzapd::anchor::AnchorSettler<zyn_custody::zebra::Zebra, zynzapd::anchor::ZcashBuilder>,
    da_dir: std::path::PathBuf,
    da_key: Option<[u8; 32]>,
    data_dir: std::path::PathBuf,
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn state_path(dir: &str, chain: u32) -> std::path::PathBuf {
    std::path::Path::new(dir).join(format!("chain-{}.object-state", chain))
}

/// Save the whole state under a temporary name and rename it into place.
///
/// A half-written state file is worse than none: the node would come back
/// believing a prefix of itself. Rename is atomic on the filesystems this
/// runs on, so a crash leaves either the old state or the new one.
fn save(dir: &str, chain: u32, vm: &ObjectVm) {
    let path = state_path(dir, chain);
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, vm.encode())
        .and_then(|_| std::fs::rename(&tmp, &path))
        .is_err()
    {
        eprintln!("zyn-object-node: could not save state");
    }
}

fn published_path(dir: &str, chain: u32) -> std::path::PathBuf {
    std::path::Path::new(dir).join(format!("chain-{}.published", chain))
}

/// One record per line: `o commitment epk ciphertext` or `n nullifier`. A
/// text format on purpose — this is the file a wallet author debugs against,
/// and being able to read it with `cat` is worth more than the bytes it costs.
fn save_published(dir: &str, chain: u32, p: &Published) {
    let mut out = String::new();
    for (c, e, x) in &p.outputs {
        out.push_str(&format!("o {} {} {}\n", c, e, x));
    }
    for n in &p.nullifiers {
        out.push_str(&format!("n {}\n", n));
    }
    let path = published_path(dir, chain);
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, out)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .is_err()
    {
        eprintln!("zyn-object-node: could not save the published log");
    }
}

fn load_published(dir: &str, chain: u32) -> Published {
    let mut p = Published::default();
    let Ok(text) = std::fs::read_to_string(published_path(dir, chain)) else {
        return p;
    };
    for line in text.lines() {
        let mut f = line.split_whitespace();
        match (f.next(), f.next(), f.next(), f.next()) {
            (Some("o"), Some(c), Some(e), Some(x)) => {
                p.outputs.push((c.into(), e.into(), x.into()))
            }
            (Some("n"), Some(n), None, None) => p.nullifiers.push(n.into()),
            _ => {} // a damaged line loses one record, not the file
        }
    }
    p
}

fn load(dir: &str, chain: u32) -> Option<ObjectVm> {
    let bytes = std::fs::read(state_path(dir, chain)).ok()?;
    let vm = ObjectVm::decode(&bytes)?;
    // A state file from another chain is not a state file for this one, and
    // continuing from it would produce a chain whose history never happened.
    (vm.chain_id == chain).then_some(vm)
}

/// The one-line JSON the status op answers with.
fn status_json(vm: &ObjectVm, anchoring: bool) -> String {
    format!(
        concat!(
            "{{\"chain\":{},\"epoch\":{},\"seq\":{},\"accounts\":{},",
            "\"commitments\":{},\"live\":{},\"nullifiers\":{},\"reconciled\":{},",
            "\"offers\":{},\"exits\":{},",
            "\"state_root\":\"{}\",\"anchoring\":\"{}\"}}"
        ),
        vm.chain_id,
        vm.epoch,
        vm.seq,
        vm.accounts.len(),
        vm.commitments.len(),
        // `live` and `reconciled` are here so the double-spend check can be
        // run by anyone reading this endpoint: every object ever created is
        // either still held or retired exactly once, and the counts say so
        // without needing a key (A120).
        vm.live.len(),
        vm.nullifiers.len(),
        vm.reconciled().is_ok(),
        vm.offers.len(),
        vm.exits.len(),
        hex(&vm.state_root()),
        // The status is what a reader believes without asking anyone. It said
        // "devnet" while the node was carrying roots to Zcash, which is the
        // wrong lie to tell in either direction.
        if anchoring {
            "on — roots are carried to Zcash and settle there"
        } else {
            "none — devnet, roots reach no chain"
        },
    )
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok())
        .collect()
}

/// Everything published since genesis, so a wallet can scan without being
/// given anything privileged.
///
/// A real deployment serves this from the data-availability bundles. This
/// node keeps its own copy and **persists it**, because a wallet that cannot
/// rebuild after the node restarts is not a wallet — the whole point is that
/// discovery needs nothing but published data and a key, and published data
/// that evaporates is not published.
#[derive(Default)]
struct Published {
    outputs: Vec<(String, String, String)>, // commitment, epk, ciphertext
    nullifiers: Vec<String>,
}

fn record(pub_log: &mut Published, op: &Op) {
    let mut add = |c: &[u8; 32], enc: &zyn_vm::note_enc::EncryptedNote| {
        pub_log
            .outputs
            .push((hex(c), hex(&enc.epk), hex(&enc.ciphertext)));
    };
    match op {
        Op::Deposit {
            asset,
            amount,
            owner,
            nonce,
            enc,
            ..
        } => {
            let note = zyn_vm::object::Note {
                asset: *asset,
                amount: *amount,
                owner: *owner,
                nonce: *nonce,
            };
            if let Ok(c) = zyn_vm::object::commitment(&note) {
                add(&c, enc);
            }
        }
        Op::Spend { inputs, outputs, .. } | Op::Withdraw { inputs, outputs, .. } => {
            for (_, n) in inputs {
                pub_log.nullifiers.push(hex(n));
            }
            for o in outputs {
                if let Ok(c) = zyn_vm::object::commitment(&o.note) {
                    add(&c, &o.enc);
                }
            }
        }
        Op::TakeOffer {
            item_nullifier,
            payment,
            outputs,
            ..
        } => {
            pub_log.nullifiers.push(hex(item_nullifier));
            for (_, n) in payment {
                pub_log.nullifiers.push(hex(n));
            }
            for o in outputs {
                if let Ok(c) = zyn_vm::object::commitment(&o.note) {
                    add(&c, &o.enc);
                }
            }
        }
        _ => {}
    }
}

fn handle(
    body: &str,
    node: &Arc<Mutex<Node<ObjectVm>>>,
    journal: &Arc<Mutex<Journal>>,
    anchoring: bool,
    pub_log: &Arc<Mutex<Published>>,
    dir: &str,
    chain: u32,
) -> String {
    let method = field(body, "method").unwrap_or_default();
    match method.as_str() {
        "zyn_status" => {
            let n = node.lock().unwrap();
            format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}}", status_json(n.state(), anchoring))
        }
        // Everything a wallet needs to rebuild itself, and nothing else.
        "zyn_published" => {
            let p = pub_log.lock().unwrap();
            let outs: Vec<String> = p
                .outputs
                .iter()
                .map(|(c, e, x)| {
                    format!(
                        "{{\"commitment\":\"{}\",\"epk\":\"{}\",\"ciphertext\":\"{}\"}}",
                        c, e, x
                    )
                })
                .collect();
            let nulls: Vec<String> = p.nullifiers.iter().map(|n| format!("\"{}\"", n)).collect();
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"outputs\":[{}],\"nullifiers\":[{}]}}}}",
                outs.join(","),
                nulls.join(",")
            )
        }
        // An intent as `encode_intent` produced it. Hex because this is a
        // devnet console, not a wire protocol.
        "zyn_submit" => {
            let Some(hexed) = field(body, "intent") else {
                return err("no intent");
            };
            let Some(bytes) = unhex(&hexed) else {
                return err("intent is not hex");
            };
            let mut d = Decoder::new(&bytes);
            let Some(op) = ObjectVm::decode_intent(&mut d) else {
                return err("intent did not decode");
            };
            let mut n = node.lock().unwrap();
            let step = n.submit_operator(op.clone(), now_secs());
            let receipts = format!("{:?}", step.receipts);

            // Write the intent down before anything derived from it is served.
            // A chain nobody can replay is a chain nobody can check, and the
            // published root is only meaningful next to the intents that
            // produced it. The seal is recorded too: an epoch whose last
            // record is not a seal is one a verifier refuses, and refusing
            // *here* costs one pass where publishing it halts every replica
            // that reads the bundle.
            {
                // The epoch this intent belongs to, read *before* a seal moves
                // the state on. Taking it afterwards files the intent under
                // the epoch that follows it, and the one it actually closed
                // ends up empty.
                let epoch_for_record = if step.sealed.is_some() {
                    n.state().epoch.saturating_sub(1)
                } else {
                    n.state().epoch
                };
                let mut j = journal.lock().unwrap();
                // A v2 journal holds *authorized* intents, not raw ones: a
                // replica decodes the envelope and re-checks the authority
                // before applying anything, so writing a bare intent produces
                // a file that reads as unsealed and a bundle no verifier will
                // take. These arrive over the operator's own RPC, so
                // `Authorized::operator` is the honest envelope — it says who
                // vouched for the intent rather than implying a signature
                // nobody made.
                let encoded = zyn_vm::verify::encode_authorized::<ObjectVm>(
                    &zyn_vm::verify::Authorized::operator(op.clone()),
                );
                if let Err(err) = j.record(epoch_for_record, step.seq, &encoded) {
                    eprintln!("zyn-object-node: could not journal seq {}: {}", step.seq, err);
                }
                if step.sealed.is_some() {
                    let seal = zyn_vm::verify::encode_authorized::<ObjectVm>(
                        &zyn_vm::verify::Authorized::operator(ObjectVm::seal_intent()),
                    );
                    let _ = j.record(epoch_for_record, step.seq.saturating_add(1), &seal);
                }
                // Always. The writer buffers, so an intent recorded and not
                // flushed is one a verifier fetching this epoch will not find
                // — and the node had exactly that bug, leaving a file holding
                // nothing but its header.
                let _ = j.sync();
            }
            // `submit` seals itself when the policy is due, so most seals
            // happen here rather than on the timer. Log from both, or a chain
            // that is working looks like one that is asleep — which cost most
            // of 21 Sep to learn on testnet (A115).
            if step.sealed.is_some() {
                eprintln!(
                    "zyn-object-node: sealed epoch {} — root {}",
                    n.state().epoch,
                    hex(&n.state().state_root())
                );
            }
            if !step.receipts.iter().any(ObjectVm::rejected) {
                let mut p = pub_log.lock().unwrap();
                record(&mut p, &op);
                save_published(dir, chain, &p);
            }
            save(dir, chain, n.state());
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"receipts\":{:?}}}}}",
                receipts
            )
        }
        _ => err("unknown method"),
    }
}

fn err(why: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{{\"code\":-32600,\"message\":\"{}\"}}}}",
        why
    )
}

/// Pull one string field out of a JSON body without a parser.
fn field(body: &str, name: &str) -> Option<String> {
    let key = format!("\"{}\"", name);
    let at = body.find(&key)? + key.len();
    let rest = &body[at..];
    let colon = rest.find(':')? + 1;
    let rest = rest[colon..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// The most a request may be. An intent is a few hundred bytes; this is
/// generous and still small enough that a thousand of them cannot matter.
const MAX_REQUEST: usize = 256 * 1024;

/// How long one connection may take. Without it a client that opens a socket
/// and says nothing holds a thread forever, and enough of them hold all of
/// them. nginx caps the public route, but this process also listens on
/// loopback and should not depend on something in front of it.
const CONN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

fn serve(mut s: TcpStream, node: Arc<Mutex<Node<ObjectVm>>>, journal: Arc<Mutex<Journal>>, pub_log: Arc<Mutex<Published>>, dir: String, chain: u32, anchoring: bool) {
    let _ = s.set_read_timeout(Some(CONN_TIMEOUT));
    let _ = s.set_write_timeout(Some(CONN_TIMEOUT));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let Ok(n) = s.read(&mut chunk) else { return };
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        // Read no more than could ever be a request. This grew without a
        // ceiling, so a declared Content-Length of a terabyte — or headers
        // that simply never end — grew it until the box ran out of memory,
        // and the box runs the mainnet sequencer too.
        if buf.len() > MAX_REQUEST {
            let _ = s.write_all(
                b"HTTP/1.1 413 Payload Too Large\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            );
            return;
        }
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
            let want = head
                .split("content-length:")
                .nth(1)
                .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).find(|x| !x.is_empty()))
                .and_then(|d| d.parse::<usize>().ok())
                .unwrap_or(0);
            if want > MAX_REQUEST {
                let _ = s.write_all(
                    b"HTTP/1.1 413 Payload Too Large\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                );
                return;
            }
            if buf.len() >= pos + 4 + want {
                let body = String::from_utf8_lossy(&buf[pos + 4..pos + 4 + want]).to_string();
                let out = handle(&body, &node, &journal, anchoring, &pub_log, &dir, chain);
                let _ = s.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        out.len(),
                        out
                    )
                    .as_bytes(),
                );
                return;
            }
        }
        if buf.len() > 1 << 20 {
            return; // a devnet console does not accept a megabyte
        }
    }
}

fn find(h: &[u8], needle: &[u8]) -> Option<usize> {
    h.windows(needle.len()).position(|w| w == needle)
}

/// `<txid> <account-hex>` per line, `#` comments allowed.
///
/// A shielded deposit with no Zyn memo cannot be credited to anyone, and the
/// scanner refuses to guess. This is where an operator says, out of band, who
/// one belongs to.
fn load_attributions(
    path: &std::path::Path,
) -> Result<std::collections::BTreeMap<[u8; 32], [u8; 32]>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    let mut out = std::collections::BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 2 {
            return Err(format!(
                "{} line {}: expected `<txid> <account-hex>`",
                path.display(),
                n + 1
            ));
        }
        let parse = |h: &str| -> Option<[u8; 32]> {
            if h.len() != 64 {
                return None;
            }
            (0..32)
                .map(|i| u8::from_str_radix(h.get(i * 2..i * 2 + 2)?, 16).ok())
                .collect::<Option<Vec<u8>>>()?
                .try_into()
                .ok()
        };
        let (txid, account) = (parse(f[0]), parse(f[1]));
        match (txid, account) {
            (Some(t), Some(a)) => {
                out.insert(t, a);
            }
            _ => {
                return Err(format!(
                    "{} line {}: both fields must be 32 bytes of hex",
                    path.display(),
                    n + 1
                ))
            }
        }
    }
    Ok(out)
}

/// Wire the settler, if this deployment has a vault to anchor from.
///
/// Every piece is required: a Zcash node to watch, the vault's viewing key to
/// find its notes, and a custodian quorum to authorise the spend. Missing any
/// one of them is a devnet, not a misconfiguration, so this returns `None`
/// rather than failing — and the caller says which it got.
fn build_anchoring(chain: u32, dir: &str) -> Option<Anchoring> {
    use zyn_custody::zebra::{Network, Zebra};

    let host = std::env::var("ZYN_ZEBRA_HOST").ok()?;
    let port: u16 = std::env::var("ZYN_ZEBRA_PORT").ok()?.parse().ok()?;
    let fvk_hex = std::env::var("ZYN_VAULT_FVK").ok()?;
    let custodians: Vec<String> = std::env::var("ZYN_CUSTODIANS")
        .ok()?
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if custodians.is_empty() {
        return None;
    }
    let threshold: u16 = env("ZYN_CUSTODY_THRESHOLD", "2").parse().ok()?;
    let network = match env("ZYN_NETWORK", "testnet").as_str() {
        "mainnet" => Network::Mainnet,
        _ => Network::Testnet,
    };
    let from_height: u64 = env("ZYN_FROM_HEIGHT", "0").parse().unwrap_or(0);
    let confirmations: u64 = env("ZYN_CONFIRMATIONS", "6").parse().unwrap_or(6);

    let bytes = (0..fvk_hex.len() / 2)
        .map(|i| u8::from_str_radix(fvk_hex.get(i * 2..i * 2 + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let fvk = orchard::keys::FullViewingKey::from_bytes(&bytes.try_into().ok()?)?;
    let public = zyn_custody::shares::load_public::<zyn_custody::ceremony::Zcash>(
        std::path::Path::new(&env("ZYN_VAULT_PUBLIC", "/var/lib/zyn/vault-public")),
    )
    .ok()?;
    // Zebra's RPC credentials, if it wants any.
    let user = std::env::var("ZYN_ZEBRA_USER").ok();
    let pass = std::env::var("ZYN_ZEBRA_PASSWORD").ok();
    let auth = match (&user, &pass) {
        (Some(u), Some(p)) => Some((u.as_str(), p.as_str())),
        _ => None,
    };
    // The same trees the account ledger uses, opened by the same code.
    let zebra = Zebra::connect(&host, port, auth.clone(), network).ok()?;
    let notes =
        zynzapd::boot::open_note_trees(std::path::Path::new(dir), chain, &zebra, from_height)
            .map_err(|e| eprintln!("zyn-object-node: {e}"))
            .ok()?;

    // Which account a memo-less shielded deposit belongs to.
    //
    // Without these the tree walk stops dead at the first such deposit —
    // `Unattributable` — and every anchor then fails for want of a note that
    // is sitting on chain a few blocks further on. The vault's own faucet
    // funding is exactly that kind of deposit, so this is not an edge case
    // here, it is the first block that matters.
    let attributions = std::env::var("ZYN_ATTRIBUTIONS")
        .ok()
        .map(|p| load_attributions(std::path::Path::new(&p)))
        .transpose()
        .map_err(|e: String| eprintln!("zyn-object-node: {e}"))
        .ok()
        .flatten()
        .unwrap_or_default();

    let builder = zynzapd::anchor::ZcashBuilder::new(
        // The builder speaks `zcash_protocol`'s network, the RPC client ours.
        match network {
            Network::Mainnet => zcash_protocol::consensus::Network::MainNetwork,
            _ => zcash_protocol::consensus::Network::TestNetwork,
        },
        Vec::new(), // no local shares: this box signs through custodians
        threshold,
        fvk.clone(),
        notes.clone(),
        zebra,
        from_height,
        confirmations,
        attributions.clone(),
        public,
        Some(custodians),
    )
    .ok()?;
    let settler = zynzapd::anchor::AnchorSettler::new(
        Zebra::connect(&host, port, auth, network).ok()?,
        builder,
        confirmations,
        std::path::Path::new(dir),
        chain,
    )
    .ok()?;

    eprintln!(
        "zyn-object-node: {} shielded deposit(s) attributed out of band",
        attributions.len()
    );
    let scanner = zyn_custody::shielded::Scanner::new(
        zyn_custody::shielded::VaultKeys::from_full_viewing_key(fvk.clone()),
        from_height,
        200,
    )
    .with_notes(notes.clone())
    .with_attributions(attributions.clone());

    Some(Anchoring {
        scanner,
        notes: notes.clone(),
        zebra: Zebra::connect(&host, port, auth, network).ok()?,
        confirmations,
        settler,
        da_dir: std::path::PathBuf::from(env("ZYN_DA_DIR", &format!("{dir}/da"))),
        da_key: std::env::var("ZYN_DA_KEY")
            .ok()
            .and_then(|k| zynzapd::dacrypt::key_from_hex(&k).ok()),
        data_dir: std::path::PathBuf::from(dir),
    })
}

/// Write both note trees where the next boot will look for them.
fn save_note_trees(dir: &str, chain: u32, notes: &zyn_custody::shielded::PoolStores) {
    for (pool, store) in [
        (orchard::ValuePool::Orchard, &notes.orchard),
        (orchard::ValuePool::Ironwood, &notes.ironwood),
    ] {
        let path = zynzapd::boot::note_tree_path(std::path::Path::new(dir), chain, pool);
        if let Ok(s) = store.lock() {
            if let Err(e) = std::fs::write(&path, s.encode()) {
                eprintln!("zyn-object-node: cannot save the {pool:?} note tree: {e}");
            }
        }
    }
}

fn main() {
    let chain: u32 = env("ZYN_CHAIN_ID", "212").parse().unwrap_or(212);
    let listen = env("ZYN_LISTEN", "127.0.0.1:8399");
    let dir = env("ZYN_DATA_DIR", "./object-data");
    let epoch_secs: u64 = env("ZYN_EPOCH_SECS", "10").parse().unwrap_or(10);
    let max_io: u32 = env("ZYN_MAX_IO", "16").parse().unwrap_or(16);

    // Anchoring is opt-in and all-or-nothing. Without a vault to pay from and
    // a Zcash node to watch, a root reaches nothing and the banner below says
    // so — being loudly a devnet is better than being quietly one.
    let anchoring = build_anchoring(chain, &dir);

    if std::fs::create_dir_all(&dir).is_err() {
        eprintln!("zyn-object-node: cannot use {}", dir);
        std::process::exit(1);
    }

    let policy = EpochPolicy {
        intents_per_epoch: 64,
        epochs_per_anchor: u64::MAX, // never: this chain does not anchor
        max_seconds_per_epoch: epoch_secs,
        max_seconds_per_anchor: 0,
    };

    let vm = match load(&dir, chain) {
        Some(v) => {
            eprintln!(
                "zyn-object-node: resumed chain {} at epoch {} seq {} with {} object(s)",
                chain,
                v.epoch,
                v.seq,
                v.live.len()
            );
            Node::resume(v, policy, Economics::flat(1), now_secs())
        }
        None => {
            eprintln!("zyn-object-node: new chain {}", chain);
            Node::new(chain, Limits { max_io }, policy, Economics::flat(1))
        }
    };
    let node = Arc::new(Mutex::new(vm.with_manual_anchoring()));
    // The journal is what makes this chain checkable by anyone else: a root
    // is only meaningful next to the intents that produced it.
    let journal = match Journal::open(&dir, chain) {
        Ok(j) => Arc::new(Mutex::new(j)),
        Err(e) => {
            eprintln!("zyn-object-node: cannot open the journal in {}: {}", dir, e);
            std::process::exit(1);
        }
    };
    eprintln!("zyn-object-node: journalling intents to {}/journal/chain-{}", dir, chain);
    let pub_log = Arc::new(Mutex::new(load_published(&dir, chain)));
    {
        let p = pub_log.lock().unwrap();
        if !p.outputs.is_empty() {
            eprintln!(
                "zyn-object-node: {} published output(s) and {} nullifier(s) restored",
                p.outputs.len(),
                p.nullifiers.len()
            );
        }
    }

    eprintln!(
        "zyn-object-node: chain {} listening on {} — {}",
        chain,
        listen,
        if anchoring.is_some() { "TESTNET" } else { "DEVNET" }
    );
    // Say which one it is, and never say the better one by default. A node
    // that claims to anchor and does not is worse than one that admits it.
    if anchoring.is_some() {
        eprintln!(
            "zyn-object-node: anchoring ON — roots are carried to Zcash and settle there"
        );
        eprintln!(
            "zyn-object-node: objects are still unbacked: no deposit bridge credits real ZEC yet"
        );
    } else {
        eprintln!(
            "zyn-object-node: anchoring OFF — roots are sealed and served, and reach no chain"
        );
    }
    eprintln!("zyn-object-node: state in {}", dir);

    // Seal on a timer, so a chain with no traffic still advances and a wallet
    // scanning it sees epochs go by rather than wondering if it is alive.
    {
        let node = Arc::clone(&node);
        let journal = Arc::clone(&journal);
        let dir = dir.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let mut n = node.lock().unwrap();
            // Only fires for an epoch that had traffic and then went quiet:
            // `seal_if_due` refuses an empty epoch, and a submission that was
            // already due sealed itself above.
            if n.seal_if_due(now_secs()).is_some() {
                // A sealed epoch is a root worth carrying. Proposing is free
                // and idempotent — `propose_anchor` refuses while one is in
                // flight — and the settler below is what turns a proposal
                // into something Zcash has seen.
                n.propose_anchor(now_secs());
                // The seal belongs in the journal wherever it happens. Sealing
                // here and recording it only on the submit path left the epoch
                // closed in memory and open on disk, which every verifier
                // reads as an epoch that was never finished.
                let closed = n.state().epoch.saturating_sub(1);
                if let Ok(mut j) = journal.lock() {
                    let seal = zyn_vm::verify::encode_authorized::<ObjectVm>(
                        &zyn_vm::verify::Authorized::operator(ObjectVm::seal_intent()),
                    );
                    if let Err(e) = j.record(closed, n.state().seq(), &seal) {
                        eprintln!("zyn-object-node: could not journal the timer seal: {e}");
                    }
                    let _ = j.sync();
                }
                eprintln!(
                    "zyn-object-node: sealed epoch {} on the timer — root {}",
                    n.state().epoch,
                    hex(&n.state().state_root())
                );
                save(&dir, chain, n.state());
            }
        });
    }

    // Walk the vault's note trees forward, before and during anchoring.
    //
    // Seeding a tree only fixes where it starts. Until it is fed the blocks
    // since, the vault's notes are on chain and invisible, and every anchor
    // fails with "waiting for a spendable Ironwood note" while the money sits
    // right there. Each batch is persisted: catch-up can take hours, and a
    // restart that discarded it would begin again from the same stale
    // position (A115).
    // Remembered before `anchoring` is consumed below, so the status can say
    // which mode this node is in.
    let anchors = anchoring.is_some();
    let anchoring = anchoring.map(|a| {
        // Split it: the scanner and its Zcash connection go to the sync
        // thread, the settler to the anchoring one. Neither type is `Clone`,
        // and neither should be — two scanners feeding one tree would walk
        // the same blocks into it twice.
        let (scanner, notes, zebra, confirmations) =
            (a.scanner, a.notes, a.zebra, a.confirmations);
        let dir2 = dir.clone();
        std::thread::spawn(move || loop {
            let tip = match zebra.tip() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("zyn-object-node: cannot read the Zcash tip: {e}");
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    continue;
                }
            };
            let target = tip.saturating_sub(confirmations);
            let mut synced = notes.synced_to().unwrap_or(0);
            while synced < target {
                match scanner.sync_notes(&zebra, target) {
                    Ok(h) => {
                        synced = h;
                        save_note_trees(&dir2, chain, &notes);
                        eprintln!("zyn-object-node: note trees synced to {synced} of {target}");
                    }
                    Err(zyn_custody::shielded::ScanRangeError::Unattributable {
                        txid,
                        height,
                        amount,
                    }) => {
                        eprintln!(
                            "zyn-object-node: note tree catch-up stopped at {height}: {amount:?} \
                             in {} carries no Zyn memo — add it to ZYN_ATTRIBUTIONS \
                             (the all-zero account means a vault top-up crediting nobody)",
                            txid.iter().map(|b| format!("{:02x}", b)).collect::<String>()
                        );
                        break;
                    }
                    Err(e) => {
                        eprintln!("zyn-object-node: note tree catch-up stopped: {e:?}");
                        break;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(30));
        });
        (a.settler, a.da_dir, a.da_key, a.data_dir)
    });

    // Carry roots to Zcash, and publish what lets anyone check them.
    if let Some((mut settler, da_dir, da_key, data_dir)) = anchoring {
        let node = Arc::clone(&node);
        let journal = Arc::clone(&journal);
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(20));
            match settler.poll_once(&node, now_secs()) {
                Ok(Some(c)) => {
                    eprintln!(
                        "zyn-object-node: anchor for epoch {} CONFIRMED on Zcash in {} at height {} — root {} is now something anyone can check",
                        c.anchor.checkpoint.epoch,
                        c.txid,
                        c.height,
                        hex(&c.anchor.checkpoint.state_root),
                    );
                    // Flush first: the bundle is read back off disk, and an
                    // epoch still buffered would publish without its seal.
                    if let Ok(mut j) = journal.lock() {
                        let _ = j.sync();
                    }
                    let built = node
                        .lock()
                        .map_err(|_| "node lock poisoned".to_string())
                        .and_then(|n| {
                            zynzapd::publish::bundle_for::<ObjectVm>(
                                &n,
                                &data_dir,
                                chain,
                                &c.anchor,
                                &c.txid,
                                c.height,
                            )
                        })
                        .and_then(|b| {
                            zynzapd::publish::write_local_sealed(
                                &da_dir,
                                chain,
                                &b,
                                da_key.as_ref(),
                            )
                        });
                    match built {
                        Ok(files) => eprintln!(
                            "zyn-object-node: DA bundle for epoch {} written ({} files)",
                            c.anchor.checkpoint.epoch,
                            files.len()
                        ),
                        Err(e) => eprintln!(
                            "zyn-object-node: could not write the bundle for epoch {}: {e}",
                            c.anchor.checkpoint.epoch
                        ),
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("zyn-object-node: anchor pass failed: {e}"),
            }
        });
    }

    let Ok(l) = TcpListener::bind(&listen) else {
        eprintln!("zyn-object-node: cannot bind {}", listen);
        std::process::exit(1);
    };
    let mut _backlog: VecDeque<()> = VecDeque::new();
    for s in l.incoming().flatten() {
        let journal = Arc::clone(&journal);
        let node = Arc::clone(&node);
        let pub_log = Arc::clone(&pub_log);
        let dir = dir.clone();
        std::thread::spawn(move || serve(s, node, journal, pub_log, dir, chain, anchors));
    }
}
