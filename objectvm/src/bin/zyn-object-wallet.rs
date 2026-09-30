//! A wallet for the object ledger, and a demonstration of what one has to do.
//!
//! On an account ledger a wallet asks the node what an account holds. Against
//! this chain there is nobody to ask: the node serves commitments and
//! ciphertexts, and only the holder's key can tell which are theirs. So the
//! wallet downloads what anyone could download and reconstructs itself
//! (DECISIONS §133.5).
//!
//! ```text
//!   zyn-object-wallet balance   what I hold, rebuilt from scratch
//!   zyn-object-wallet notes     the notes behind that number
//!   zyn-object-wallet address   the viewing key to be paid at
//! ```
//!
//! Nothing here is privileged. Point it at any node serving `zyn_published`
//! and it produces the same answer, which is the property that makes the
//! ledger private rather than merely unlogged: the node cannot tell which
//! outputs this wallet opened, because it never asked about any of them.

use std::io::{Read, Write};
use std::net::TcpStream;

use zyn_vm::note_enc::{viewing_public, StaticSecret};
use zyn_vm::scan::{PublishedOutput, Scanner};
use zyn_vm::spec::AccountId;

fn env(n: &str, d: &str) -> String {
    std::env::var(n).unwrap_or_else(|_| d.to_string())
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

fn hash32(s: &str) -> Option<[u8; 32]> {
    unhex(s)?.try_into().ok()
}

/// One POST, no dependencies. A wallet that needed a JSON stack to check a
/// balance would be a wallet nobody runs on a phone.
fn rpc(host: &str, body: &str) -> Result<String, String> {
    let mut s = TcpStream::connect(host).map_err(|e| e.to_string())?;
    let req = format!(
        "POST / HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        host,
        body.len(),
        body
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = String::new();
    s.read_to_string(&mut out).map_err(|e| e.to_string())?;
    let at = out.find("\r\n\r\n").ok_or("no body")?;
    Ok(out[at + 4..].to_string())
}

/// Pull every `"key":"value"` for one key, in order. Enough for this shape and
/// small enough to read; a real wallet uses the node's own client.
fn all_values(body: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{}\":\"", key);
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find(&needle) {
        rest = &rest[at + needle.len()..];
        match rest.find('"') {
            Some(end) => {
                out.push(rest[..end].to_string());
                rest = &rest[end..];
            }
            None => break,
        }
    }
    out
}

/// The nullifier list is a bare array of strings, so it needs its own read.
fn nullifiers_of(body: &str) -> Vec<String> {
    let Some(at) = body.find("\"nullifiers\":[") else {
        return Vec::new();
    };
    let rest = &body[at + "\"nullifiers\":[".len()..];
    let end = rest.find(']').unwrap_or(0);
    rest[..end]
        .split(',')
        .filter_map(|s| {
            let t = s.trim().trim_matches('"');
            (!t.is_empty()).then(|| t.to_string())
        })
        .collect()
}

fn main() {
    let host = env("ZYN_NODE", "127.0.0.1:8399");
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "balance".into());

    // A wallet's two keys. Separate on purpose: the viewing key finds what
    // arrived, the nullifier key recognises what left, and a watch-only
    // holder can be given the first alone (§scan).
    let viewing = StaticSecret::from(
        hash32(&env("ZYN_VIEWING_KEY", &"aa".repeat(32))).unwrap_or([0xAA; 32]),
    );
    let nullifier_key = hash32(&env("ZYN_NULLIFIER_KEY", &"a9".repeat(32))).unwrap_or([0xA9; 32]);
    let owner: AccountId = hash32(&env("ZYN_ACCOUNT", &"aa".repeat(32))).unwrap_or([0xAA; 32]);
    let asset = hash32(&env("ZYN_ASSET", &"7a".repeat(32))).unwrap_or([0x7a; 32]);

    if cmd == "address" {
        println!("viewing key: {}", hex(&viewing_public(&viewing)));
        println!("account:     {}", hex(&owner));
        return;
    }

    let body = match rpc(
        &host,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"zyn_published\"}",
    ) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("zyn-object-wallet: cannot reach {}: {}", host, e);
            std::process::exit(1);
        }
    };

    let commitments = all_values(&body, "commitment");
    let epks = all_values(&body, "epk");
    let ciphertexts = all_values(&body, "ciphertext");
    if commitments.len() != epks.len() || epks.len() != ciphertexts.len() {
        eprintln!("zyn-object-wallet: the node's output list is inconsistent");
        std::process::exit(1);
    }

    let outputs: Vec<PublishedOutput> = commitments
        .iter()
        .zip(epks.iter())
        .zip(ciphertexts.iter())
        .filter_map(|((c, e), x)| {
            Some(PublishedOutput {
                commitment: hash32(c)?,
                enc: zyn_vm::note_enc::EncryptedNote {
                    epk: hash32(e)?,
                    ciphertext: unhex(x)?,
                },
            })
        })
        .collect();
    let spent: Vec<[u8; 32]> = nullifiers_of(&body).iter().filter_map(|s| hash32(s)).collect();

    let wallet = Scanner::new(viewing, nullifier_key).scan(&owner, &outputs, 0, &spent);

    match cmd.as_str() {
        "notes" => {
            println!("scanned {} output(s), {} nullifier(s)", outputs.len(), spent.len());
            for o in &wallet.live {
                println!(
                    "  live  position {:<6} {:>20}  {}",
                    o.position,
                    o.note.amount.to_string(),
                    hex(&o.commitment)[..16].to_string()
                );
            }
            for o in &wallet.spent {
                println!(
                    "  spent position {:<6} {:>20}  {}",
                    o.position,
                    o.note.amount.to_string(),
                    hex(&o.commitment)[..16].to_string()
                );
            }
        }
        _ => {
            println!(
                "{} — from {} output(s), none of which the node knew were ours",
                zyn_vm::fixed::Fixed(wallet.balance(&asset)),
                outputs.len()
            );
        }
    }
}
