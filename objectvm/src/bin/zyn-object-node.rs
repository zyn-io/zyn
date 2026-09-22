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
use zyn::{Economics, Node};
use zyn_vm::read::Decoder;
use zyn_vm::spec::MicrochainVm;

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
fn status_json(vm: &ObjectVm) -> String {
    format!(
        concat!(
            "{{\"chain\":{},\"epoch\":{},\"seq\":{},\"accounts\":{},",
            "\"commitments\":{},\"nullifiers\":{},\"offers\":{},\"exits\":{},",
            "\"state_root\":\"{}\",\"anchoring\":\"none — devnet, roots reach no chain\"}}"
        ),
        vm.chain_id,
        vm.epoch,
        vm.seq,
        vm.accounts.len(),
        vm.commitments.len(),
        vm.nullifiers.len(),
        vm.offers.len(),
        vm.exits.len(),
        hex(&vm.state_root()),
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
    pub_log: &Arc<Mutex<Published>>,
    dir: &str,
    chain: u32,
) -> String {
    let method = field(body, "method").unwrap_or_default();
    match method.as_str() {
        "zyn_status" => {
            let n = node.lock().unwrap();
            format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}}", status_json(n.state()))
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

fn serve(mut s: TcpStream, node: Arc<Mutex<Node<ObjectVm>>>, pub_log: Arc<Mutex<Published>>, dir: String, chain: u32) {
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
                let out = handle(&body, &node, &pub_log, &dir, chain);
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

fn main() {
    let chain: u32 = env("ZYN_CHAIN_ID", "212").parse().unwrap_or(212);
    let listen = env("ZYN_LISTEN", "127.0.0.1:8399");
    let dir = env("ZYN_DATA_DIR", "./object-data");
    let epoch_secs: u64 = env("ZYN_EPOCH_SECS", "10").parse().unwrap_or(10);
    let max_io: u32 = env("ZYN_MAX_IO", "16").parse().unwrap_or(16);

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
        "zyn-object-node: chain {} listening on {} — DEVNET, objects are unbacked",
        chain, listen
    );
    eprintln!("zyn-object-node: anchoring OFF — roots are sealed and served, and reach no chain");
    eprintln!("zyn-object-node: state in {}", dir);

    // Seal on a timer, so a chain with no traffic still advances and a wallet
    // scanning it sees epochs go by rather than wondering if it is alive.
    {
        let node = Arc::clone(&node);
        let dir = dir.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let mut n = node.lock().unwrap();
            // Only fires for an epoch that had traffic and then went quiet:
            // `seal_if_due` refuses an empty epoch, and a submission that was
            // already due sealed itself above.
            if n.seal_if_due(now_secs()).is_some() {
                eprintln!(
                    "zyn-object-node: sealed epoch {} on the timer — root {}",
                    n.state().epoch,
                    hex(&n.state().state_root())
                );
                save(&dir, chain, n.state());
            }
        });
    }

    let Ok(l) = TcpListener::bind(&listen) else {
        eprintln!("zyn-object-node: cannot bind {}", listen);
        std::process::exit(1);
    };
    let mut _backlog: VecDeque<()> = VecDeque::new();
    for s in l.incoming().flatten() {
        let node = Arc::clone(&node);
        let pub_log = Arc::clone(&pub_log);
        let dir = dir.clone();
        std::thread::spawn(move || serve(s, node, pub_log, dir, chain));
    }
}
