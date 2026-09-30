//! Attendance certificates: tap an event's NFC tag, get its certificate in
//! your wallet (DECISIONS §150, `zynzapd::attend`).
//!
//! An event is a directory entry: `<name>.json` is its public manifest,
//! `<name>.keys.json` its tag keys (`{"meta":"<32 hex>","file":"<32 hex>"}`,
//! mode 0600 — whoever holds them can mint taps). Tags are NTAG 424 DNA in SUN
//! mode, their NDEF URL
//!
//!   https://nap.cavebro.win/tap?e=<collection>&picc=<picc>&cmac=<cmac>
//!
//! with the chip mirroring `picc` (encrypted UID and counter) and `cmac` on
//! every read. Nap opens that link and, once the wallet is open, posts the
//! tap here with the holder's account; the certificate is minted to it.
//!
//! Environment:
//!   ZYN_ATTEND_EVENTS   directory of events               (./events)
//!   ZYN_ATTEND_BOOK     claim book                        (./attend-book.json)
//!   ZYN_ATTEND_LISTEN   host:port                         (127.0.0.1:8123)
//!   ZYN_ATTEND_NODE     object node RPC                   (http://127.0.0.1:8398)
//!   ZYN_SUBMIT_TOKEN    the node's operator token         (register, serve)
//!
//! Usage:
//!   zyn-attend event <name>                 the event's collection and manifest hash
//!   zyn-attend keys <name>                  fresh tag keys into <name>.keys.json
//!   zyn-attend register <name> <creator>    register the event's collection (operator)
//!   zyn-attend tap <name> <uid> <counter>   the URL a tag would write (rehearsal)
//!   zyn-attend serve                        accept claims

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Mutex;

use objectvm::{ObjectVm, Op};
use zyn_vm::spec::MicrochainVm;
use serde_json::{json, Value};
use zynzapd::attend::{hex, Book, Manifest, Refusal};
use zynzapd::tap::{self, Tap};

fn env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("zyn-attend: {msg}");
    std::process::exit(1)
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    (s.len() % 2 == 0).then(|| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect())?
}

struct Event {
    manifest: Manifest,
    meta: [u8; 16],
    file: [u8; 16],
}

fn events_dir() -> PathBuf {
    PathBuf::from(env("ZYN_ATTEND_EVENTS", "events"))
}

fn manifest(name: &str) -> Manifest {
    let path = events_dir().join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| die(format!("{}: {e}", path.display())));
    Manifest::parse(&text).unwrap_or_else(|e| die(format!("{}: {e}", path.display())))
}

fn keys(name: &str) -> ([u8; 16], [u8; 16]) {
    let path = events_dir().join(format!("{name}.keys.json"));
    let v: Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| die(format!("{} is missing or not JSON", path.display())));
    let key = |k: &str| v.get(k).and_then(Value::as_str).and_then(unhex).and_then(|b| <[u8; 16]>::try_from(b).ok()).unwrap_or_else(|| die(format!("{}: {k} must be 32 hex", path.display())));
    (key("meta"), key("file"))
}

/// Every event in the directory that has both a manifest and keys.
fn load_events() -> Vec<Event> {
    let dir = events_dir();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| die(format!("{}: {e}", dir.display()))).flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".json").filter(|s| !s.ends_with(".keys")) else { continue };
        if !dir.join(format!("{stem}.keys.json")).exists() {
            eprintln!("zyn-attend: {stem} has no keys; not serving it");
            continue;
        }
        let (meta, file) = keys(stem);
        out.push(Event { manifest: manifest(stem), meta, file });
    }
    out
}

fn call(method: &str, fields: Value) -> Result<Value, String> {
    zynzapd::object_client::call(&env("ZYN_ATTEND_NODE", "http://127.0.0.1:8398"), method, fields)
}

fn token() -> String {
    std::env::var("ZYN_SUBMIT_TOKEN").unwrap_or_else(|_| die("set ZYN_SUBMIT_TOKEN (read it from the node's env file)"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, what: &str| args.get(i).cloned().unwrap_or_else(|| die(format!("missing {what}")));
    match args.first().map(String::as_str) {
        Some("event") => {
            let m = manifest(&arg(1, "event name"));
            println!("collection {}\nmanifest   {}\n{}", hex(&m.collection()), hex(&m.hash()), m.canonical());
        }
        Some("keys") => {
            let name = arg(1, "event name");
            let path = events_dir().join(format!("{name}.keys.json"));
            if path.exists() {
                die(format!("{} exists; not overwriting tag keys", path.display()));
            }
            let mut k = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut k);
            let body = json!({ "meta": hex(&k[..16]), "file": hex(&k[16..]) }).to_string();
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode_private().open(&path).unwrap_or_else(|e| die(e));
            f.write_all(body.as_bytes()).unwrap_or_else(|e| die(e));
            println!("tag keys written to {} (0600). Program them into the tags as the SDM meta-read and file-read keys; never print them.", path.display());
        }
        Some("register") => {
            let m = manifest(&arg(1, "event name"));
            let creator = unhex(&arg(2, "creator account (64 hex)")).and_then(|b| <[u8; 32]>::try_from(b).ok()).unwrap_or_else(|| die("creator must be 64 hex"));
            let op = Op::RegisterCollection {
                program: zyn_vm::program::register_collection(),
                asset: m.collection(),
                creator,
                // Certificates are given, not sold; a collection still names
                // the asset its resales would be priced in.
                quote: swapvm::types::XZEC,
                fee_bps: 0,
                cap: m.cap,
            };
            let r = call("zyn_submit", json!({ "token": token(), "intent": hex(&ObjectVm::encode_intent(&op)) })).unwrap_or_else(|e| die(e));
            println!("register {}: {}", m.name, r.get("receipts").and_then(Value::as_str).unwrap_or("").chars().take(160).collect::<String>());
        }
        Some("tap") => {
            let name = arg(1, "event name");
            let m = manifest(&name);
            let (meta, file) = keys(&name);
            let uid = unhex(&arg(2, "uid (14 hex)")).and_then(|b| <[u8; 7]>::try_from(b).ok()).unwrap_or_else(|| die("uid is 7 bytes, 14 hex"));
            let counter: u32 = arg(3, "counter").parse().unwrap_or_else(|_| die("counter is a number"));
            let (picc, cmac) = tap::emulate(&meta, &file, &Tap { uid, counter }, [0; 5]);
            println!("https://nap.cavebro.win/tap?e={}&picc={picc}&cmac={cmac}", hex(&m.collection()));
        }
        Some("serve") => serve(),
        _ => die("usage: zyn-attend event|keys|register|tap|serve — see the header of src/bin/zyn-attend.rs"),
    }
}

trait PrivateMode {
    fn mode_private(&mut self) -> &mut Self;
}
impl PrivateMode for std::fs::OpenOptions {
    fn mode_private(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.mode(0o600)
    }
}

struct Serving {
    events: Vec<Event>,
    book: Mutex<Book>,
    book_path: PathBuf,
    token: String,
}

fn serve() {
    let book_path = PathBuf::from(env("ZYN_ATTEND_BOOK", "attend-book.json"));
    let book = Book::load(&book_path).unwrap_or_else(|e| die(e));
    let events = load_events();
    if events.is_empty() {
        die("no events with keys to serve");
    }
    for e in &events {
        eprintln!("serving {} ({}) — {}", e.manifest.name, e.manifest.date, hex(&e.manifest.collection()));
    }
    let s = Serving { events, book: Mutex::new(book), book_path, token: token() };
    let listen = env("ZYN_ATTEND_LISTEN", "127.0.0.1:8123");
    let listener = TcpListener::bind(&listen).unwrap_or_else(|e| die(format!("{listen}: {e}")));
    eprintln!("zyn-attend listening on {listen}");
    for stream in listener.incoming().flatten() {
        if let Err(e) = handle(&s, stream) {
            eprintln!("zyn-attend: {e}");
        }
    }
}

fn handle(s: &Serving, mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let (head_end, length) = loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 16 * 1024 {
            return reply(&mut stream, 413, &json!({ "error": "too large" }));
        }
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
            let len = head.lines().find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
            break (i + 4, len.min(8 * 1024));
        }
    };
    while buf.len() < head_end + length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let line = head.lines().next().unwrap_or("");
    if line.starts_with("GET /events") {
        let list: Vec<Value> = s.events.iter().map(|e| json!({ "collection": hex(&e.manifest.collection()), "manifest": serde_json::from_str::<Value>(&e.manifest.canonical()).unwrap_or(Value::Null) })).collect();
        return reply(&mut stream, 200, &json!({ "events": list }));
    }
    if !line.starts_with("POST /attend") {
        return reply(&mut stream, 404, &json!({ "error": "POST /attend, GET /events" }));
    }
    let body: Value = serde_json::from_slice(&buf[head_end..(head_end + length).min(buf.len())]).unwrap_or(Value::Null);
    let (status, answer) = attend(s, &body);
    reply(&mut stream, status, &answer)
}

fn attend(s: &Serving, body: &Value) -> (u16, Value) {
    let field = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    let (event, account) = (field("event"), field("account"));
    if account.len() != 64 || unhex(&account).is_none() {
        return (400, json!({ "error": "account is the Zyn account, 64 hex" }));
    }
    let Some(ev) = s.events.iter().find(|e| hex(&e.manifest.collection()) == event) else {
        return (404, json!({ "error": refusal(Refusal::UnknownEvent) }));
    };
    let tap = match tap::verify(&ev.meta, &ev.file, &field("picc"), &field("cmac"), &[]) {
        Ok(t) => t,
        Err(_) => return (403, json!({ "error": "That tap did not come from this event's tag." })),
    };
    // Decide and write it down before minting: a crash between the two leaves
    // a claim the same account can retry into the same serial.
    let granted = {
        let mut book = s.book.lock().unwrap_or_else(|p| p.into_inner());
        let before = book.clone();
        match book.claim(&ev.manifest, &tap, &account) {
            Ok(g) => {
                if let Err(e) = book.save(&s.book_path) {
                    *book = before;
                    return (503, json!({ "error": format!("cannot record the claim: {e}") }));
                }
                g
            }
            Err(r) => return (409, json!({ "error": refusal(r) })),
        }
    };
    let collection = hex(&ev.manifest.collection());
    match call("zyn_mintItem", json!({ "token": s.token, "collection": collection, "owner": account, "serial": granted.serial.to_string() })) {
        Ok(r) => {
            let receipts = r.get("receipts").and_then(Value::as_str).unwrap_or("");
            if receipts.contains("Refused") && !receipts.contains("already minted") {
                let why = if receipts.contains("has not registered") { "Open your wallet once on this network, then tap again." } else { "The chain did not mint it; tap again in a moment." };
                return (502, json!({ "error": why }));
            }
            (200, json!({ "collection": collection, "serial": granted.serial, "fresh": granted.fresh, "event": ev.manifest.name }))
        }
        Err(_) => (502, json!({ "error": "The chain is not answering; tap again in a moment." })),
    }
}

fn refusal(r: Refusal) -> &'static str {
    match r {
        Refusal::TagTaken => "This badge's certificate already went to another wallet.",
        Refusal::TapUsed => "That tap was already used. Tap the tag again.",
        Refusal::SoldOut => "Every certificate for this event has been given out.",
        Refusal::UnknownEvent => "Nap does not know this event.",
    }
}

fn reply(stream: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
    let text = body.to_string();
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        502 => "Bad Gateway",
        _ => "Service Unavailable",
    };
    write!(stream, "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncache-control: no-store\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}", text.len())
}

