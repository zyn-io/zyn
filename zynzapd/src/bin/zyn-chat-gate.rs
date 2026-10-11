//! `zyn-chat-gate` — the door of ZCaveDAO's members' chat.
//!
//! A member pass (DECISIONS §172.3) is the hash of a token a holder's Nap
//! made, tied on the ledger to one of their items; it dies when the item is
//! sold, moved or redeemed. The chat is an ejabberd server with no sign-up and
//! no federation. This gate is the only way in:
//!
//! * `POST /auth {"token": <64 hex>}`: the pass is the token's hash; if the
//!   ledger says it is live, the holder becomes a member of every room and
//!   gets a login — a JID derived from the pass (`m-<12 hex>`, the same on
//!   every device, naming no account or item) and a signed token ejabberd
//!   accepts as its password.
//! * Every `ZYN_CHAT_SWEEP` seconds it asks the ledger about every pass it
//!   let in, and takes each dead one out of every room and off the server.
//!
//! It keeps one thing: which pass became which JID, so it can take a member
//! out when their item moves. The Cave team, which runs it, could therefore
//! link a chat name to a pass, and the operator can link a pass to an item;
//! the chat's door says so.
//!
//! ```text
//!   ZYN_CHAT_LISTEN      HTTP for /auth (behind nginx)     (127.0.0.1:8597)
//!   ZYN_CHAT_NODE        the object ledger's JSON-RPC       (http://127.0.0.1:8599)
//!   ZYN_CHAT_COLLECTION  whose passes open the door          (Collection Z)
//!   ZYN_CHAT_DOMAIN      the chat's XMPP host                (chat.cavebro.win)
//!   ZYN_CHAT_MUC         its rooms' service              (rooms.chat.cavebro.win)
//!   ZYN_CHAT_ROOMS       room:Title,…  (general:General,diggers:Diggers,lookouts:Lookouts,lanterns:Lanterns)
//!   ZYN_CHAT_API         ejabberd's admin API         (http://127.0.0.1:5281/api)
//!   ZYN_CHAT_KEY         the JWK ejabberd checks logins with      (required)
//!   ZYN_CHAT_STATE       pass → JID              (/var/lib/zyn/chat/gate.json)
//!   ZYN_CHAT_ORIGINS     sites allowed to call /auth          (https://cavebro.win)
//!   ZYN_CHAT_SWEEP       seconds between ledger checks                 (300)
//!   ZYN_CHAT_DAYS        how long a login lasts, in days                (30)
//! ```

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let v: Option<Vec<u8>> = (0..32).map(|i| u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()).collect();
    v?.try_into().ok()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Logins: HS256 tokens, as ejabberd's `auth_method: jwt` checks them.

const B64URL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64url(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().fold(0u32, |a, b| (a << 8) | *b as u32) << (8 * (3 - chunk.len()));
        for i in 0..=chunk.len() {
            out.push(B64URL[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        acc = (acc << 6) | B64URL.iter().position(|x| *x == c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let pad = |b: u8| k.iter().map(|x| x ^ b).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(msg).finalize();
    Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize().into()
}

fn jwt(key: &[u8], jid: &str, exp: u64) -> String {
    let head = b64url(br#"{"alg":"HS256","typ":"JWT"}"#);
    let body = b64url(json!({ "jid": jid, "exp": exp }).to_string().as_bytes());
    let signed = format!("{head}.{body}");
    let sig = b64url(&hmac_sha256(key, signed.as_bytes()));
    format!("{signed}.{sig}")
}

/// A member's chat name, from their pass alone: the same on every device and
/// after every login, and nothing about the account or the item.
fn local_of(pass: &[u8; 32]) -> String {
    let h = Sha256::new().chain_update(b"cave.chat.jid.v1").chain_update(pass).finalize();
    format!("m-{}", &hex(&h)[..12])
}

// ---------------------------------------------------------------------------

struct Gate {
    node: String,
    collection: String,
    domain: String,
    muc: String,
    rooms: Vec<(String, String)>,
    api: String,
    key: Vec<u8>,
    state: std::path::PathBuf,
    origins: Vec<String>,
    days: u64,
    agent: ureq::Agent,
    /// Pass (hex) → JID local part, of everyone let in and not yet taken out.
    members: Mutex<BTreeMap<String, String>>,
    /// Requests per IP this hour.
    tries: Mutex<BTreeMap<(String, u64), u32>>,
}

impl Gate {
    /// One admin call. ejabberd answers a refused command (a room that
    /// already exists, say) with a 4xx and a reason: that is an answer too.
    fn ejabberd(&self, command: &str, args: Value) -> Result<Value, String> {
        let r = match self.agent.post(&format!("{}/{command}", self.api)).send_json(args) {
            Ok(r) | Err(ureq::Error::Status(_, r)) => r,
            Err(e) => return Err(format!("ejabberd {command}: {e}")),
        };
        r.into_json().map_err(|e| format!("ejabberd {command}: {e}"))
    }

    fn live(&self, pass: &[u8; 32]) -> Result<bool, String> {
        let r = zynzapd::object_client::call(&self.node, "zyn_memberPass", json!({ "collection": self.collection, "pass": hex(pass) }))?;
        r.get("member").and_then(Value::as_bool).ok_or_else(|| "the ledger answered no membership".to_string())
    }

    fn affiliate(&self, local: &str, affiliation: &str) -> Result<(), String> {
        for (room, _) in &self.rooms {
            let r = self.ejabberd(
                "set_room_affiliation",
                json!({ "room": room, "service": self.muc, "user": local, "host": self.domain, "affiliation": affiliation }),
            )?;
            if r != json!(0) {
                return Err(format!("ejabberd set_room_affiliation {room}: {r}"));
            }
        }
        Ok(())
    }

    fn save(&self, members: &BTreeMap<String, String>) {
        let tmp = self.state.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(members).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.state);
        }
    }

    /// The rooms, made once; every member of the server is a member of each.
    fn rooms(&self) -> Result<(), String> {
        for (room, title) in &self.rooms {
            let r = self.ejabberd(
                "create_room_with_opts",
                json!({ "room": room, "service": self.muc, "host": self.domain, "options": [
                    { "name": "title", "value": title },
                    { "name": "members_only", "value": "true" },
                    { "name": "anonymous", "value": "false" },
                    { "name": "public", "value": "false" },
                    { "name": "persistent", "value": "true" },
                    { "name": "mam", "value": "true" },
                ] }),
            )?;
            // 0 when made; an error string when it already exists.
            if r != json!(0) {
                let _ = self.ejabberd("change_room_option", json!({ "name": room, "service": self.muc, "option": "title", "value": title }));
            }
        }
        let members = self.members.lock().unwrap().clone();
        for local in members.values() {
            self.affiliate(local, "member")?;
        }
        Ok(())
    }

    fn auth(&self, body: &str, ip: &str) -> (u16, Value) {
        let hour = now() / 3600;
        {
            let mut tries = self.tries.lock().unwrap();
            tries.retain(|(_, h), _| *h == hour);
            let n = tries.entry((ip.to_string(), hour)).or_insert(0);
            *n += 1;
            if *n > 30 {
                return (429, json!({ "error": "too many tries from here — try again later" }));
            }
        }
        let token = serde_json::from_str::<Value>(body).ok().and_then(|v| v.get("token").and_then(Value::as_str).and_then(unhex32));
        let Some(token) = token else {
            return (400, json!({ "error": "send the member pass code Nap gave you" }));
        };
        let pass = objectvm::dao::pass_of(&token);
        match self.live(&pass) {
            Ok(true) => {}
            Ok(false) => return (403, json!({ "error": "that pass is not live: get one on /dao with a Collection Z item in your Nap" })),
            Err(e) => {
                eprintln!("zyn-chat-gate: {e}");
                return (503, json!({ "error": "the ledger is not answering — try again shortly" }));
            }
        }
        let local = local_of(&pass);
        if let Err(e) = self.affiliate(&local, "member") {
            eprintln!("zyn-chat-gate: {e}");
            return (503, json!({ "error": "the chat is not answering — try again shortly" }));
        }
        {
            let mut members = self.members.lock().unwrap();
            if members.insert(hex(&pass), local.clone()).is_none() {
                self.save(&members);
            }
        }
        let jid = format!("{local}@{}", self.domain);
        let rooms: Vec<String> = self.rooms.iter().map(|(r, _)| format!("{r}@{}", self.muc)).collect();
        (200, json!({ "jid": jid, "password": jwt(&self.key, &jid, now() + self.days * 86_400), "rooms": rooms }))
    }

    /// Take out everyone whose pass died: out of every room, off the server.
    fn sweep(&self) {
        let members = self.members.lock().unwrap().clone();
        let mut gone = Vec::new();
        for (pass, local) in &members {
            let Some(p) = unhex32(pass) else { continue };
            match self.live(&p) {
                Ok(true) => {}
                Ok(false) => {
                    if self.affiliate(local, "none").is_ok() {
                        let _ = self.ejabberd("kick_session", json!({ "user": local, "host": self.domain, "resource": "", "reason": "membership ended" }));
                        gone.push(pass.clone());
                    }
                }
                // The ledger not answering is not a reason to take anyone out.
                Err(e) => {
                    eprintln!("zyn-chat-gate: sweep: {e}");
                    return;
                }
            }
        }
        if !gone.is_empty() {
            let mut members = self.members.lock().unwrap();
            for p in &gone {
                members.remove(p);
            }
            self.save(&members);
            println!("zyn-chat-gate: {} member(s) out: their item moved", gone.len());
        }
    }

    fn serve(&self, mut s: TcpStream) {
        let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
        let mut buf = vec![0u8; 8192];
        let mut got = 0;
        // Headers, then as much body as Content-Length says (a /auth body is small).
        loop {
            let Ok(n) = s.read(&mut buf[got..]) else { return };
            if n == 0 {
                break;
            }
            got += n;
            let text = String::from_utf8_lossy(&buf[..got]).to_string();
            if let Some(end) = text.find("\r\n\r\n") {
                let len = text[..end]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                if got >= end + 4 + len || got == buf.len() {
                    break;
                }
            }
            if got == buf.len() {
                break;
            }
        }
        let text = String::from_utf8_lossy(&buf[..got]).to_string();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let mut lines = head.lines();
        let first = lines.next().unwrap_or("");
        let header = |name: &str| head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()));
        let origin = header("origin").filter(|o| self.origins.iter().any(|x| x == o));
        let ip = header("x-real-ip").unwrap_or_else(|| s.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default());
        let (code, v) = if first.starts_with("OPTIONS ") {
            (204, Value::Null)
        } else if first.starts_with("POST /auth ") {
            if origin.is_none() {
                (403, json!({ "error": "this door opens only from the Cave's own site" }))
            } else {
                self.auth(body, &ip)
            }
        } else if first.starts_with("GET /health ") {
            (200, json!({ "ok": true, "members": self.members.lock().unwrap().len() }))
        } else {
            (404, json!({ "error": "not found" }))
        };
        let reason = match code {
            200 => "OK",
            204 => "No Content",
            400 => "Bad Request",
            403 => "Forbidden",
            404 => "Not Found",
            429 => "Too Many Requests",
            _ => "Service Unavailable",
        };
        let body = if code == 204 { String::new() } else { v.to_string() };
        let cors = origin.map_or(String::new(), |o| {
            format!("Access-Control-Allow-Origin: {o}\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: content-type\r\nVary: Origin\r\n")
        });
        let _ = write!(
            s,
            "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nCache-Control: no-store\r\n{cors}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

fn main() {
    let key_file = env("ZYN_CHAT_KEY", "");
    let key = std::fs::read(&key_file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.get("k").and_then(Value::as_str).and_then(b64url_decode))
        .filter(|k| k.len() >= 32)
        .unwrap_or_else(|| {
            eprintln!("zyn-chat-gate: set ZYN_CHAT_KEY to the JWK ejabberd checks logins with");
            std::process::exit(2);
        });
    let collection = std::env::var("ZYN_CHAT_COLLECTION").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| hex(&zynzapd::cave_z::collection_id()));
    let rooms: Vec<(String, String)> = env("ZYN_CHAT_ROOMS", "general:General,diggers:Diggers,lookouts:Lookouts,lanterns:Lanterns")
        .split(',')
        .filter_map(|r| r.split_once(':').map(|(a, b)| (a.trim().to_string(), b.trim().to_string())))
        .collect();
    let state: std::path::PathBuf = env("ZYN_CHAT_STATE", "/var/lib/zyn/chat/gate.json").into();
    let members: BTreeMap<String, String> = std::fs::read(&state).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let gate = Arc::new(Gate {
        node: env("ZYN_CHAT_NODE", "http://127.0.0.1:8599"),
        collection,
        domain: env("ZYN_CHAT_DOMAIN", "chat.cavebro.win"),
        muc: env("ZYN_CHAT_MUC", "rooms.chat.cavebro.win"),
        rooms,
        api: env("ZYN_CHAT_API", "http://127.0.0.1:5281/api"),
        key,
        state,
        origins: env("ZYN_CHAT_ORIGINS", "https://cavebro.win").split(',').map(|o| o.trim().to_string()).collect(),
        days: env("ZYN_CHAT_DAYS", "30").parse().unwrap_or(30),
        agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build(),
        members: Mutex::new(members),
        tries: Mutex::new(BTreeMap::new()),
    });
    // The rooms first: retried until ejabberd answers.
    loop {
        match gate.rooms() {
            Ok(()) => break,
            Err(e) => {
                eprintln!("zyn-chat-gate: rooms: {e}; again in 10 s");
                std::thread::sleep(Duration::from_secs(10));
            }
        }
    }
    let every: u64 = env("ZYN_CHAT_SWEEP", "300").parse().unwrap_or(300);
    let sweeper = gate.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(every));
        sweeper.sweep();
    });
    let listen = env("ZYN_CHAT_LISTEN", "127.0.0.1:8597");
    let l = TcpListener::bind(&listen).unwrap_or_else(|e| {
        eprintln!("zyn-chat-gate: {listen}: {e}");
        std::process::exit(1);
    });
    println!("zyn-chat-gate: {} member(s); rooms {}; listening on {listen}", gate.members.lock().unwrap().len(), gate.rooms.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>().join(", "));
    for s in l.incoming().flatten() {
        let g = gate.clone();
        std::thread::spawn(move || g.serve(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4231's first HMAC-SHA256 vector: the login signature is standard.
    #[test]
    fn hmac_is_rfc_4231() {
        assert_eq!(hex(&hmac_sha256(&[0x0b; 20], b"Hi There")), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    }

    #[test]
    fn base64url_round_trips_without_padding() {
        for n in 0..40u8 {
            let data: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            let e = b64url(&data);
            assert!(!e.contains('=') && !e.contains('+') && !e.contains('/'));
            assert_eq!(b64url_decode(&e).unwrap(), data);
        }
        assert_eq!(b64url(b"Ma"), "TWE");
    }

    #[test]
    fn a_login_is_a_jwt_ejabberd_reads() {
        let t = jwt(&[7; 32], "m-0123456789ab@chat.cavebro.win", 2_000_000_000);
        let parts: Vec<&str> = t.split('.').collect();
        assert_eq!(parts.len(), 3);
        let body: Value = serde_json::from_slice(&b64url_decode(parts[1]).unwrap()).unwrap();
        assert_eq!(body, json!({ "jid": "m-0123456789ab@chat.cavebro.win", "exp": 2_000_000_000u64 }));
        assert_eq!(parts[2], b64url(&hmac_sha256(&[7; 32], format!("{}.{}", parts[0], parts[1]).as_bytes())));
    }

    #[test]
    fn a_chat_name_comes_from_the_pass_alone() {
        let a = local_of(&[1; 32]);
        assert_eq!(a, local_of(&[1; 32]));
        assert_ne!(a, local_of(&[2; 32]));
        assert!(a.starts_with("m-") && a.len() == 14);
    }
}
