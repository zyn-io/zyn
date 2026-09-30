//! Attendance certificates (DECISIONS §150): an event is a collection on the
//! object ledger, and a certificate is one of its items, minted into the
//! attendee's wallet when they tap the event's NFC tag.
//!
//! - **The event is fixed before anyone attends.** Its manifest (name, date,
//!   place, issuer, artwork, cap, mode) is canonicalised and hashed, and the
//!   collection's address is derived from that hash, so the words and picture
//!   a certificate points at cannot be swapped afterwards.
//! - **A tap is checked** (`tap.rs`): a genuine NTAG 424 DNA read, under the
//!   event's own keys, with a counter nobody has used.
//! - **The book decides who gets which serial.** `Badge` events give one
//!   certificate per tag (a tag per attendee: badges, wristbands); `Door`
//!   events give one per account to anyone who taps the shared tag. Either
//!   way a claim binds once and retrying it is safe.
//!
//! The mint itself is the operator's (`zyn_mintItem`), like the Cave sale's
//! delivery: this service is trusted to hand certificates out, and says so.
//! A mint signed by the event's own issuer is the step that opens this to
//! any organiser (§150.4).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use zyn_vm::commit::Encoder;

use crate::tap::Tap;

/// How many certificates an event gives out, and to whom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// One per tag: the first account to claim a tag's certificate keeps it.
    Badge,
    /// One per account, from a shared tag; each tap is good once.
    Door,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Badge => "badge",
            Mode::Door => "door",
        }
    }
}

/// What an event is. Everything here is public and fixed at registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    /// ISO date, `2026-10-15`.
    pub date: String,
    pub place: String,
    /// Who hands the certificates out, as they want to be named.
    pub issuer: String,
    /// The artwork's content hash (64 hex) on the media gateway, or empty.
    pub art: String,
    pub cap: u32,
    pub mode: Mode,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).map(str::to_string);
        let name = s("name").filter(|x| !x.is_empty() && x.chars().count() <= 64).ok_or("name: 1–64 characters")?;
        let date = s("date").filter(|x| is_date(x)).ok_or("date: YYYY-MM-DD")?;
        let place = s("place").filter(|x| x.chars().count() <= 64).ok_or("place: up to 64 characters")?;
        let issuer = s("issuer").filter(|x| !x.is_empty() && x.chars().count() <= 64).ok_or("issuer: 1–64 characters")?;
        let art = s("art").unwrap_or_default().to_lowercase();
        if !(art.is_empty() || (art.len() == 64 && art.bytes().all(|b| b.is_ascii_hexdigit()))) {
            return Err("art: a 64-hex content hash, or empty".into());
        }
        let cap = v.get("cap").and_then(Value::as_u64).filter(|c| (1..=1_000_000).contains(c)).ok_or("cap: 1–1,000,000")? as u32;
        let mode = match s("mode").as_deref() {
            Some("badge") => Mode::Badge,
            Some("door") => Mode::Door,
            _ => return Err("mode: \"badge\" (one per tag) or \"door\" (one per account)".into()),
        };
        Ok(Manifest { name, date, place, issuer, art, cap, mode })
    }

    /// The manifest as it is hashed: fixed key order, no whitespace.
    pub fn canonical(&self) -> String {
        json!({
            "v": 1,
            "name": self.name,
            "date": self.date,
            "place": self.place,
            "issuer": self.issuer,
            "art": self.art,
            "cap": self.cap,
            "mode": self.mode.name(),
        })
        .to_string()
    }

    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.canonical().as_bytes()).into()
    }

    /// The event's collection address: bound to the manifest's hash.
    pub fn collection(&self) -> [u8; 32] {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.attend.event.v1").bytes(&self.hash());
        e.leaf()
    }
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Why a claim did not get a certificate.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// This tag's certificate went to another account.
    TagTaken,
    /// This tap was already used (a copied or replayed link).
    TapUsed,
    /// Every certificate has been handed out.
    SoldOut,
    /// Not an event this service hands out.
    UnknownEvent,
}

/// A claim's outcome: which serial the account holds, and whether it is new.
#[derive(Debug, PartialEq, Eq)]
pub struct Granted {
    pub serial: u32,
    pub fresh: bool,
}

/// One event's claims.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    pub next_serial: u32,
    /// Badge: tag UID → (account, serial).
    pub tags: BTreeMap<String, (String, u32)>,
    /// Door: every (UID, counter) already used.
    pub taps: BTreeMap<String, Vec<u32>>,
    /// Account → serial (both modes).
    pub accounts: BTreeMap<String, u32>,
}

impl Ledger {
    /// Decide a claim. Nothing is minted here; the caller mints `serial` to
    /// `account` after saving, and a retry reaches the same serial.
    pub fn claim(&mut self, manifest: &Manifest, tap: &Tap, account: &str) -> Result<Granted, Refusal> {
        let uid = hex(&tap.uid);
        match manifest.mode {
            Mode::Badge => {
                if let Some((owner, serial)) = self.tags.get(&uid) {
                    return if owner == account { Ok(Granted { serial: *serial, fresh: false }) } else { Err(Refusal::TagTaken) };
                }
                if let Some(serial) = self.accounts.get(account) {
                    // One per account too: a second badge in the same hands is
                    // not a second attendance.
                    return Ok(Granted { serial: *serial, fresh: false });
                }
                let serial = self.take(manifest)?;
                self.tags.insert(uid, (account.to_string(), serial));
                self.accounts.insert(account.to_string(), serial);
                Ok(Granted { serial, fresh: true })
            }
            Mode::Door => {
                if let Some(serial) = self.accounts.get(account) {
                    return Ok(Granted { serial: *serial, fresh: false });
                }
                let used = self.taps.entry(uid).or_default();
                if used.contains(&tap.counter) {
                    return Err(Refusal::TapUsed);
                }
                let serial = if self.next_serial < manifest.cap { self.next_serial } else { return Err(Refusal::SoldOut) };
                self.next_serial += 1;
                used.push(tap.counter);
                self.accounts.insert(account.to_string(), serial);
                Ok(Granted { serial, fresh: true })
            }
        }
    }

    fn take(&mut self, manifest: &Manifest) -> Result<u32, Refusal> {
        if self.next_serial >= manifest.cap {
            return Err(Refusal::SoldOut);
        }
        self.next_serial += 1;
        Ok(self.next_serial - 1)
    }

    fn to_json(&self) -> Value {
        json!({
            "next_serial": self.next_serial,
            "tags": self.tags.iter().map(|(k, (a, s))| (k.clone(), json!([a, s]))).collect::<serde_json::Map<_, _>>(),
            "taps": self.taps.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
            "accounts": self.accounts.iter().map(|(k, s)| (k.clone(), json!(s))).collect::<serde_json::Map<_, _>>(),
        })
    }

    fn from_json(v: &Value) -> Result<Ledger, String> {
        let bad = || "the claim book is damaged".to_string();
        let obj = |k: &str| v.get(k).and_then(Value::as_object).ok_or_else(bad);
        let mut l = Ledger { next_serial: v.get("next_serial").and_then(Value::as_u64).ok_or_else(bad)? as u32, ..Default::default() };
        for (k, t) in obj("tags")? {
            let a = t.get(0).and_then(Value::as_str).ok_or_else(bad)?;
            let s = t.get(1).and_then(Value::as_u64).ok_or_else(bad)? as u32;
            l.tags.insert(k.clone(), (a.to_string(), s));
        }
        for (k, t) in obj("taps")? {
            let list = t.as_array().ok_or_else(bad)?.iter().map(|c| c.as_u64().map(|c| c as u32).ok_or_else(bad)).collect::<Result<_, _>>()?;
            l.taps.insert(k.clone(), list);
        }
        for (k, s) in obj("accounts")? {
            l.accounts.insert(k.clone(), s.as_u64().ok_or_else(bad)? as u32);
        }
        Ok(l)
    }
}

/// Every event's claims, kept in one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Book {
    pub events: BTreeMap<String, Ledger>,
}

impl Book {
    pub fn load(path: &Path) -> Result<Book, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Book::default()),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let v: Value = serde_json::from_str(&text).map_err(|_| "the claim book is damaged".to_string())?;
        let mut book = Book::default();
        for (k, l) in v.get("events").and_then(Value::as_object).ok_or("the claim book is damaged")? {
            book.events.insert(k.clone(), Ledger::from_json(l)?);
        }
        Ok(book)
    }

    /// Written whole to a temporary file and renamed: a crash leaves the old
    /// book or the new one, never half of each.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let v = json!({ "v": 1, "events": self.events.iter().map(|(k, l)| (k.clone(), l.to_json())).collect::<serde_json::Map<_, _>>() });
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, v.to_string()).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
    }

    pub fn claim(&mut self, manifest: &Manifest, tap: &Tap, account: &str) -> Result<Granted, Refusal> {
        self.events.entry(hex(&manifest.collection())).or_default().claim(manifest, tap, account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(mode: &str, cap: u32) -> Manifest {
        Manifest::parse(&format!(r#"{{"name":"Zcon Denver","date":"2026-10-15","place":"Denver","issuer":"Cave","art":"","cap":{cap},"mode":"{mode}"}}"#)).unwrap()
    }
    const TAG_A: Tap = Tap { uid: [4, 1, 2, 3, 4, 5, 6], counter: 7 };
    const TAG_B: Tap = Tap { uid: [4, 9, 9, 9, 9, 9, 9], counter: 1 };

    #[test]
    fn the_collection_is_bound_to_every_word_of_the_manifest() {
        let a = manifest("badge", 100);
        let mut b = a.clone();
        b.place = "Boulder".into();
        assert_ne!(a.collection(), b.collection());
        assert_eq!(a.collection(), manifest("badge", 100).collection(), "the same event is the same collection");
        assert_eq!(a.canonical(), manifest("badge", 100).canonical(), "canonical form is deterministic");
        assert!(Manifest::parse(r#"{"name":"x","date":"15/10/2026","place":"","issuer":"c","cap":1,"mode":"badge"}"#).is_err());
        assert!(Manifest::parse(r#"{"name":"x","date":"2026-10-15","place":"","issuer":"c","cap":1,"mode":"airdrop"}"#).is_err());
    }

    #[test]
    fn a_badge_gives_one_certificate_to_its_first_claimant() {
        let m = manifest("badge", 100);
        let mut book = Book::default();
        assert_eq!(book.claim(&m, &TAG_A, "alice"), Ok(Granted { serial: 0, fresh: true }));
        assert_eq!(book.claim(&m, &Tap { counter: 8, ..TAG_A }, "alice"), Ok(Granted { serial: 0, fresh: false }), "retry is safe");
        assert_eq!(book.claim(&m, &TAG_A, "bob"), Err(Refusal::TagTaken), "the badge is Alice's");
        assert_eq!(book.claim(&m, &TAG_B, "bob"), Ok(Granted { serial: 1, fresh: true }));
        assert_eq!(book.claim(&m, &Tap { uid: [4, 7, 7, 7, 7, 7, 7], counter: 1 }, "bob"), Ok(Granted { serial: 1, fresh: false }), "one per account too");
    }

    #[test]
    fn a_door_tag_serves_everyone_once_and_each_tap_once() {
        let m = manifest("door", 2);
        let mut book = Book::default();
        assert_eq!(book.claim(&m, &TAG_A, "alice"), Ok(Granted { serial: 0, fresh: true }));
        assert_eq!(book.claim(&m, &TAG_A, "bob"), Err(Refusal::TapUsed), "Alice's link, replayed");
        assert_eq!(book.claim(&m, &Tap { counter: 8, ..TAG_A }, "bob"), Ok(Granted { serial: 1, fresh: true }));
        assert_eq!(book.claim(&m, &Tap { counter: 9, ..TAG_A }, "alice"), Ok(Granted { serial: 0, fresh: false }));
        assert_eq!(book.claim(&m, &Tap { counter: 10, ..TAG_A }, "carol"), Err(Refusal::SoldOut));
    }

    #[test]
    fn the_book_survives_a_restart() {
        let m = manifest("door", 10);
        let mut book = Book::default();
        book.claim(&m, &TAG_A, "alice").unwrap();
        book.claim(&manifest("badge", 10), &TAG_B, "bob").unwrap();
        let dir = std::env::temp_dir().join(format!("attend-book-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.json");
        book.save(&path).unwrap();
        assert_eq!(Book::load(&path).unwrap(), book);
        assert_eq!(Book::load(&dir.join("absent.json")).unwrap(), Book::default());
        std::fs::remove_dir_all(dir).ok();
    }
}
