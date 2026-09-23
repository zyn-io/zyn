//! The Cave checkout: an invitation in, an item out.
//!
//! This is the piece that was missing while every part around it worked. The
//! whitelist registry, the order book, the allocator and the treasury signing
//! path all existed and never met, so there was no place where money changed
//! hands (§99).
//!
//! # What a buyer does
//!
//! ```text
//!   POST /quote   {code, account}  -> price, and the address to pay it at
//!   (buyer sends ZEC to that address; the vault scanner credits it)
//!   POST /settle  {code, read}     -> serials, minted to the same account
//!   GET  /order?code=…             -> where the order got to
//! ```
//!
//! # Paid to the account that will hold the item
//!
//! The address quoted is the deposit address of the buyer's **collection
//! receiving account** (§97.5), not of their identity. Three things follow, and
//! the third is the one that made this simple:
//!
//!   * no memo — a per-account address attributes itself, which is what makes
//!     this payable from an exchange (§71.39, A79);
//!   * the journal names an account with no other history, so the mint does not
//!     write the buyer's identity into it;
//!   * the money and the item end up in the same account, so nothing has to be
//!     moved afterwards — and a move is exactly what would have written the
//!     link back in.
//!
//! # Why the buyer proves their own payment
//!
//! Balances are private, so this service cannot look up whether a buyer paid,
//! and giving it the power to would mean giving it a read over every account.
//! Instead `/settle` takes the buyer's own **signed read** — the same payload
//! their wallet already produces to see its balance — and this service relays
//! it. The service holds no key of the buyer's and learns nothing it was not
//! handed (§74.23).
//!
//! # What this is not
//!
//! Trusted by construction, like the claim service. The creator key can mint
//! any item to anyone regardless of what this does. What it guarantees is the
//! ordering in `sale.rs`: a paid order is remembered, retrying is safe, and an
//! allocation nobody minted is given back.
//!
//! Environment:
//!   ZYN_SALE_LISTEN      host:port to serve on
//!   ZYN_SALE_BOOK        path to the order book
//!   ZYN_SALE_REGISTRY    path to the whitelist registry
//!   ZYN_SALE_NODE        node RPC                        (127.0.0.1:8099)
//!   ZYN_SALE_CHAIN       chain id                        (26460)
//!   (the collection id, the entry price and the symbol are NOT settings:
//!    they come from the published terms, and an environment variable that
//!    disagrees stops the service rather than overriding them — §113)
//!   ZYN_SALE_CUSTODIANS  comma-separated host:port
//!   ZYN_SALE_THRESHOLD   signatures required             (2)
//!   ZYN_SALE_TREASURY    directory holding public.bin
//!
//! # `zyn-sale draw`
//!
//! Runs the collection-wide draw once, after the close (A98, §104-§106). It
//! serves nothing and mints nothing: it scans the vault for entry payments up
//! to the announced close, admits the ones that qualify, fetches and verifies
//! the reveal beacon, and prints the assignment with its four roots.
//!
//! The close and reveal are read from the **published announcement**, never
//! from the environment — see `draw::Plan::from_announcement`.
//!
//!   ZYN_SALE_ANNOUNCEMENT  JSON: the close announcement and its publication
//!   ZYN_SALE_SCAN_FROM     first Zcash height to scan
//!   ZYN_VAULT_FVK          the vault full viewing key, 96 hex (read-only)
//!   ZYN_ZEBRA_HOST/PORT    the Zcash node            (127.0.0.1:8232)
//!   ZYN_ZEBRA_USER/PASS    RPC credentials, if any
//!   ZYN_BRIDGE_MAX_BLOCKS  blocks per scan step      (200)

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use swapvm::fixed::Fixed;
use swapvm::tx::Intent;
use zynzapd::client::{fixed_of, hex, Node};
use zynzapd::allocation as crate_allocation;
use zynzapd::sale::{self, Book, Refused, State};
use zynzapd::whitelist::Registry;

/// `content.json` as the manifest builder writes it: `{"<serial>": {"content":
/// "<64 hex>", …}}`. Indices are the committed serials, 0-based, exactly as
/// `zyn-cave-manifest` derived the root over them.
fn load_manifest(path: &str) -> Result<std::collections::BTreeMap<u32, [u8; 32]>, String> {
    if path.trim().is_empty() {
        return Err("set ZYN_SALE_MANIFEST to the collection's content.json".into());
    }
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    let doc: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("{}: {}", path, e))?;
    let map = doc.as_object().ok_or("content.json is not an object")?;
    let mut out = std::collections::BTreeMap::new();
    for (k, v) in map {
        let serial: u32 = k
            .parse()
            .map_err(|_| format!("manifest key {} is not a serial", k))?;
        let content = v
            .get("content")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("serial {} has no content", serial))?;
        out.insert(
            serial,
            unhex32(content).ok_or_else(|| format!("serial {}: content is not 32 bytes", serial))?,
        );
    }
    if out.is_empty() {
        return Err("the manifest is empty".into());
    }
    Ok(out)
}

fn env(k: &str, default: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| default.to_string())
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

struct Service {
    node: Node,
    book: Mutex<Book>,
    registry: Mutex<Registry>,
    book_path: std::path::PathBuf,
    registry_path: std::path::PathBuf,
    chain: u32,
    collection: [u8; 32],
    price: Fixed,
    /// The published terms. The split is read from here so the pool's share is
    /// the one buyers were promised rather than one this service decided.
    terms: cave_randomness::SaleTerms,
    /// Serial to its committed content hash, from the same `content.json` the
    /// manifest root was computed over (§86). A mint names the content, so the
    /// service cannot invent one: a serial missing here is refused rather than
    /// minted with a zero commitment.
    manifest: std::collections::BTreeMap<u32, [u8; 32]>,
    symbol_prefix: String,
    custodians: Vec<String>,
    threshold: usize,
    public: zyn_custody::treasury::PublicKeyPackage<zyn_custody::treasury::ZynAccount>,
    treasury: [u8; 32],
    request: std::sync::atomic::AtomicU64,
    /// The completed draw, once it has run. `None` until then, which is why
    /// `/settle` can record a payment before the reveal and only mints after.
    allocation: Mutex<Option<crate_allocation::Run>>,
}

fn main() {
    // `draw` is a different job from serving: it runs once, touches no book and
    // signs nothing. Dispatching here rather than in a second binary keeps the
    // manifest loader, the hex helpers and the price in one place.
    if std::env::args().nth(1).as_deref() == Some("draw") {
        std::process::exit(draw_command());
    }
    // `reconcile` needs the same configuration the service runs with — the
    // treasury, the custodians, the terms — so it is a mode of this binary
    // rather than a tool that would have to be told all of it again.
    let reconciling = std::env::args().nth(1).as_deref() == Some("reconcile");

    let listen = env("ZYN_SALE_LISTEN", "127.0.0.1:8096");
    let book_path = std::path::PathBuf::from(env("ZYN_SALE_BOOK", "./sale.book"));
    let registry_path = std::path::PathBuf::from(env("ZYN_SALE_REGISTRY", "./whitelist.registry"));
    let chain: u32 = env("ZYN_SALE_CHAIN", "26460").parse().expect("chain id");
    // Derived from the published terms, not read from the environment.
    //
    // §64.16: no price, recipient, split or close rule may live only in a
    // service environment variable. It was not a style preference — admission
    // requires the deposit to equal `policy.entry_price_zatoshi` *exactly*, so
    // a service quoting anything else takes everyone's money and then refuses
    // every one of them at the draw, after the close, when it cannot be undone.
    let sale_policy = zynzapd::cave_z::policy();
    let sale_terms = zynzapd::cave_z::terms(&sale_policy);
    let collection = sale_policy.collection_id;
    if let Ok(given) = std::env::var("ZYN_SALE_COLLECTION") {
        if unhex32(&given) != Some(collection) {
            eprintln!(
                "zyn-sale: ZYN_SALE_COLLECTION is {} but the published terms commit {}",
                given,
                hex(&collection)
            );
            std::process::exit(2);
        }
    }
    // §64.17 froze this at 0.01 ZEC for entry. It is read from the environment
    // rather than hardcoded so a testnet run can differ, and printed at start
    // so a wrong one is visible before a buyer meets it rather than after.
    // One zatoshi in Fixed's 1e18 scale, so the quoted price and the admitted
    // price are the same number in two units rather than two numbers.
    let price = Fixed(sale_terms.entry_price_zatoshi as i128 * 10_000_000_000);
    if let Ok(given) = std::env::var("ZYN_SALE_PRICE") {
        match fixed_of(&given) {
            Ok(p) if p == price => {}
            Ok(p) => {
                eprintln!(
                    "zyn-sale: ZYN_SALE_PRICE is {} but the published terms commit {} ({} zatoshi) \
                     — a buyer paying the first is refused at the draw",
                    p, price, sale_terms.entry_price_zatoshi
                );
                std::process::exit(2);
            }
            Err(e) => {
                eprintln!("zyn-sale: ZYN_SALE_PRICE is not a decimal amount: {}", e);
                std::process::exit(2);
            }
        }
    }
    let custodians: Vec<String> = env("ZYN_SALE_CUSTODIANS", "")
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .collect();
    let threshold: usize = env("ZYN_SALE_THRESHOLD", "2").parse().expect("threshold");
    if custodians.len() < threshold {
        eprintln!(
            "zyn-sale: {} custodian(s) named but threshold is {} — a mint is signed when the \
             buyer settles, so they must be up while this serves",
            custodians.len(),
            threshold
        );
        std::process::exit(2);
    }
    let treasury_dir = env("ZYN_SALE_TREASURY", "");
    let public = zyn_custody::shares::load_public::<zyn_custody::treasury::ZynAccount>(
        std::path::Path::new(&treasury_dir),
    )
    .unwrap_or_else(|e| {
        eprintln!("zyn-sale: cannot read {}/public.bin: {}", treasury_dir, e);
        std::process::exit(1);
    });
    let treasury = zyn_custody::treasury::account_of_group(&public).expect("treasury account");

    let manifest_path = env("ZYN_SALE_MANIFEST", "");
    let manifest = load_manifest(&manifest_path).unwrap_or_else(|e| {
        eprintln!("zyn-sale: {}", e);
        std::process::exit(1);
    });
    let symbol_prefix = zynzapd::cave_z::SYMBOL.to_string();

    let book = Book::load(&book_path).unwrap_or_else(|e| {
        eprintln!("zyn-sale: {}", e);
        std::process::exit(1);
    });
    let registry = Registry::load(&registry_path).unwrap_or_else(|e| {
        eprintln!("zyn-sale: {}", e);
        std::process::exit(1);
    });
    let (orders, taken) = book.totals();
    if !registry.open {
        eprintln!("zyn-sale: the whitelist phase is CLOSED — quotes will be refused until it opens");
    }

    let node = Node::new(&env("ZYN_SALE_NODE", "127.0.0.1:8099"), chain);

    let s = Arc::new(Service {
        node,
        book: Mutex::new(book),
        registry: Mutex::new(registry),
        book_path,
        registry_path,
        chain,
        collection,
        price,
        terms: sale_terms.clone(),
        manifest,
        symbol_prefix,
        custodians,
        threshold,
        public,
        treasury,
        request: std::sync::atomic::AtomicU64::new(1),
        allocation: Mutex::new(None),
    });

    let listener = TcpListener::bind(&listen).unwrap_or_else(|e| {
        eprintln!("zyn-sale: cannot bind {}: {}", listen, e);
        std::process::exit(1);
    });
    eprintln!(
        "zyn-sale: collection {} at {} ZEC.zy each; {} item(s) in the manifest; {} order(s) so far, {} paid",
        hex(&s.collection),
        s.price,
        s.manifest.len(),
        orders,
        taken
    );
    if reconciling {
        std::process::exit(reconcile_command(&s));
    }
    eprintln!("zyn-sale: POST http://{}/quote  {{\"code\":\"…\",\"account\":\"64 hex\"}}", listen);
    eprintln!("zyn-sale: POST http://{}/settle {{\"code\":\"…\",\"read\":\"hex signed read\"}}", listen);

    for stream in listener.incoming().flatten() {
        let s = Arc::clone(&s);
        std::thread::spawn(move || {
            let _ = handle(&s, stream);
        });
    }
}

fn handle(s: &Service, mut stream: TcpStream) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(15)));
    let mut r = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    // A body cap, because this is reachable from a browser and a checkout has
    // no reason to send more than a code, an account and a signed read.
    if len > 8192 {
        return reply(&mut stream, 413, r#"{"error":"body too large"}"#);
    }
    let mut body = vec![0u8; len];
    if len > 0 {
        r.read_exact(&mut body)?;
    }

    if method == "OPTIONS" {
        return preflight(&mut stream);
    }
    match (method.as_str(), path.split('?').next().unwrap_or("")) {
        ("POST", "/quote") => quote(s, &mut stream, &body),
        ("POST", "/settle") => settle(s, &mut stream, &body),
        ("GET", "/order") => order(s, &mut stream, &path),
        _ => reply(
            &mut stream,
            404,
            r#"{"error":"POST /quote, POST /settle, GET /order?code="}"#,
        ),
    }
}

fn field(body: &[u8], key: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get(key)?.as_str().map(str::to_string)
}

fn quote(s: &Service, stream: &mut TcpStream, body: &[u8]) -> std::io::Result<()> {
    let (Some(code), Some(account_hex)) = (field(body, "code"), field(body, "account")) else {
        return reply(stream, 400, r#"{"error":"code and account are required"}"#);
    };
    let Some(account) = unhex32(&account_hex) else {
        return reply(stream, 400, r#"{"error":"account must be 64 hex characters"}"#);
    };
    let quantity = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("quantity").and_then(serde_json::Value::as_u64))
        .unwrap_or(1) as u32;

    let result = {
        let mut registry = s.registry.lock().map_err(|_| std::io::Error::other("lock poisoned"))?;
        let mut book = s.book.lock().map_err(|_| std::io::Error::other("lock poisoned"))?;
        let q = sale::quote(&mut registry, &mut book, &code, account, quantity, s.price);
        if q.is_ok() {
            // Persisted before the buyer is told a price, so a crash between
            // the two cannot hand out an allocation the book has forgotten.
            let _ = registry.save(&s.registry_path);
            let _ = book.save(&s.book_path);
        }
        q
    };

    match result {
        Ok(q) => {
            // The address to pay: the deposit address of the account that will
            // hold the item. Asked of the node, which is the only party that
            // can issue one.
            let address = match s.node.deposit_address(account) {
                Ok(a) => a,
                Err(e) => {
                    return reply(
                        stream,
                        502,
                        &format!(
                            r#"{{"error":"reserved, but no deposit address yet: {}"}}"#,
                            e.replace('"', "'")
                        ),
                    )
                }
            };
            reply(
                stream,
                200,
                &format!(
                    r#"{{"reserved":true,"quantity":{},"unitPrice":"{}","total":"{}","payTo":"{}","memo":false,"treasury":"{}"}}"#,
                    q.quantity, q.unit_price, q.total, address, hex(&s.treasury)
                ),
            )
        }
        Err(e) => refused(stream, e),
    }
}

fn settle(s: &Service, stream: &mut TcpStream, body: &[u8]) -> std::io::Result<()> {
    let (Some(code), Some(read_hex)) = (field(body, "code"), field(body, "read")) else {
        return reply(
            stream,
            400,
            r#"{"error":"code and read (a signed read from the paying account) are required"}"#,
        );
    };
    // The buyer's signed payment. This service holds no key of theirs, so the
    // only way the entry can be paid is that they signed it and this relays it.
    let payment = match field(body, "payment").map(|p| unhex(&p)) {
        Some(Some(bytes)) => Some(bytes),
        Some(None) => return reply(stream, 400, r#"{"error":"payment must be hex"}"#),
        None => None,
    };
    let Some(read) = unhex(&read_hex) else {
        return reply(stream, 400, r#"{"error":"read must be hex"}"#);
    };

    let account = {
        let book = s.book.lock().map_err(|_| std::io::Error::other("lock poisoned"))?;
        match book.get(&code) {
            Some(o) => o.account,
            None => return reply(stream, 404, r#"{"error":"no such order — quote first"}"#),
        }
    };

    // The buyer's own signed read, relayed. This service never holds a key of
    // theirs and cannot read any account that has not handed it one.
    let record = match s.node.account_raw(&read) {
        Ok(Some(r)) => r,
        Ok(None) => return reply(stream, 402, r#"{"error":"that account holds nothing yet"}"#),
        Err(e) => {
            return reply(
                stream,
                400,
                &format!(r#"{{"error":"{}"}}"#, e.replace('"', "'")),
            )
        }
    };
    let held = record
        .spendable
        .iter()
        .find(|(a, _)| *a == swapvm::types::XZEC)
        .map(|(_, v)| *v)
        .unwrap_or(Fixed::ZERO);

    let pay = || -> Result<Fixed, String> {
        // The money has to leave the buyer, because the terms say where it
        // goes: 90% to the collection's pool, which is what a redeemed item is
        // paid out of, and 10% to the treasury. An earlier version of this
        // recognised the balance and moved nothing, which meant a buyer kept
        // their ZEC.zy *and* got an item, and the redemption floor every buyer
        // is trusting was funded by nobody (§113.8).
        //
        // This service holds no key of the buyer's. It relays the frame their
        // wallet signed — `submit_raw` takes the bytes untouched, because the
        // signature commits to exactly those bytes.
        if held < s.price {
            return Err(format!(
                "this account holds {} and the entry is {} — the deposit may not be credited yet",
                held, s.price
            ));
        }
        let frame = payment.as_ref().ok_or("no signed payment accompanied this settle")?;
        s.node.submit_raw(frame).map_err(|e| format!("the payment was refused: {e}"))?;
        Ok(s.price)
    };

    // The collection's share, as a separate step by a different signer. Split
    // by the published terms rather than a percentage computed here, and
    // rounding goes to the pool because the floor is what buyers are trusting.
    //
    // `settle` calls this only after the buyer's payment is written down, so a
    // failure here is recoverable by `reconcile` rather than money that moved
    // with nothing recording it.
    let fund_pool = || -> Result<(), String> {
        let (pool_zat, _treasury_zat) = s.terms.split(s.terms.entry_price_zatoshi);
        let pool = Fixed(pool_zat as i128 * 10_000_000_000);
        let epoch = s.node.status().map_err(|e| e.to_string())?.epoch;
        let rid = s.request.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let frame = zynzapd::treasury::frame_submission_across(
            &s.custodians,
            &s.public,
            s.threshold,
            s.chain,
            epoch,
            &Intent::FundCollection {
                from: s.treasury,
                collection: s.collection,
                amount: pool,
            },
            rid,
        )?;
        s.node
            .submit_raw(&frame)
            .map_err(|e| format!("the entry was paid but the pool was not funded: {e}"))?;
        Ok(())
    };
    // A lookup, never a draw. The collection-wide allocation runs once, after
    // the reveal, over the whole ticket ledger (§104.1); settling an order
    // reads what that run gave this account. Calling the draw here instead —
    // once per buyer, against whatever ledger existed at that moment — is how
    // two people end up holding results from two different allocations.
    let allocate = |want: u32| -> Result<Vec<u32>, String> {
        let run = s.allocation.lock().map_err(|_| "allocation lock poisoned")?;
        let Some(run) = run.as_ref() else {
            return Err("the draw has not run yet — it happens once, after the reveal".into());
        };
        let mine = run.serials_for(&account);
        if mine.len() < want as usize {
            // Fewer items than the order reserved. Refusing beats handing over
            // a short set, because the reservation is what the buyer paid
            // against and a partial fill is a support problem, not a sale.
            return Err(format!(
                "the draw gave this account {} item(s), not the {} reserved",
                mine.len(),
                want
            ));
        }
        Ok(mine.into_iter().take(want as usize).collect())
    };
    let mint = |serial: u32, to: [u8; 32]| -> Result<(), String> {
        // The content commitment comes from the manifest the root was computed
        // over. A serial this service cannot find is refused rather than minted
        // against a zero commitment, which would be an item committing to
        // nothing and could never be corrected.
        let content = *s
            .manifest
            .get(&serial)
            .ok_or_else(|| format!("serial {} is not in the manifest", serial))?;
        let symbol = swapvm::state::symbol(
            format!("{}{:04}", s.symbol_prefix, serial + 1).as_bytes(),
        );
        let epoch = s.node.status().map_err(|e| e.to_string())?.epoch;
        let rid = s.request.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let frame = zynzapd::treasury::frame_submission_across(
            &s.custodians,
            &s.public,
            s.threshold,
            s.chain,
            epoch,
            &Intent::MintCollectionItem {
                creator: s.treasury,
                collection: s.collection,
                serial,
                to,
                symbol,
                content,
            },
            rid,
        )?;
        match s.node.submit_raw(&frame) {
            Ok(_) => Ok(()),
            // A serial already minted is this attempt finishing what an earlier
            // one started, which for a retry is success.
            Err(e) if e.contains("(code 3)") => Ok(()),
            Err(e) => Err(e),
        }
    };

    let outcome = {
        let mut book = s.book.lock().map_err(|_| std::io::Error::other("lock poisoned"))?;
        let r = sale::settle(&mut book, &code, account, s.price, &pay, &fund_pool, &allocate, &mint);
        let _ = book.save(&s.book_path);
        r
    };

    match outcome {
        Ok(sold) => {
            eprintln!(
                "zyn-sale: {} -> {} serial(s) {:?}",
                hex(&account),
                sold.serials.len(),
                sold.serials
            );
            let serials = sold
                .serials
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(",");
            reply(
                stream,
                200,
                &format!(r#"{{"minted":true,"serials":[{}]}}"#, serials),
            )
        }
        Err(e) => refused(stream, e),
    }
}

fn order(s: &Service, stream: &mut TcpStream, path: &str) -> std::io::Result<()> {
    let code = path
        .split_once("code=")
        .map(|(_, c)| c.split('&').next().unwrap_or("").to_string())
        .unwrap_or_default();
    if code.trim().is_empty() {
        return reply(stream, 400, r#"{"error":"GET /order?code=…"}"#);
    }
    let book = s.book.lock().map_err(|_| std::io::Error::other("lock poisoned"))?;
    match book.get(&code) {
        None => reply(stream, 404, r#"{"error":"no such order"}"#),
        Some(o) => {
            let state = match o.state {
                State::Reserved => "reserved",
                State::Paid => "paid",
                State::Minted => "minted",
            };
            let serials = o
                .serials
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(",");
            reply(
                stream,
                200,
                &format!(
                    r#"{{"state":"{}","quantity":{},"paid":"{}","serials":[{}]}}"#,
                    state, o.quantity, o.paid, serials
                ),
            )
        }
    }
}

/// A refusal the buyer can act on. Each maps to the thing they would do next,
/// which is why they are not collapsed into one error.
fn refused(stream: &mut TcpStream, e: Refused) -> std::io::Result<()> {
    let (status, message): (u16, &str) = match e {
        Refused::UnknownCode => (404, "that invitation is not one of ours"),
        Refused::BoundElsewhere => (409, "that invitation is already in use by another account"),
        Refused::Closed => (403, "the whitelist phase is not open"),
        Refused::Exhausted { .. } => (409, "that invitation has no allocation left"),
        Refused::WrongAmount { .. } => (402, "the payment has not arrived in full yet"),
        Refused::PaymentRefused => (502, "the chain refused the payment"),
        // The buyer is not out of pocket for nothing: their payment is
        // recorded and `zyn-sale reconcile` finishes the collection's share.
        Refused::PoolNotFunded => (
            503,
            "your payment landed; the collection's share has not, and an operator will finish it",
        ),
        Refused::NotMinting => (409, "this collection is not minting"),
    };
    reply(
        stream,
        status,
        &format!(r#"{{"error":"{}"}}"#, message.replace('"', "'")),
    )
}

fn reply(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {} \r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
        status,
        body.len(),
        body
    )?;
    stream.flush()
}

fn preflight(stream: &mut TcpStream) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 204 \r\nAccess-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Headers: content-type\r\n\
         Access-Control-Allow-Methods: POST, GET, OPTIONS\r\nConnection: close\r\n\r\n"
    )?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The service must read exactly what the manifest builder wrote, because
    /// a mint names the content commitment and there is no second source for
    /// it. This reads the real Collection Z manifest when it is present.
    #[test]
    fn it_reads_the_collection_z_manifest() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../examples/zyn-pfp-concepts/out/content.json"
        );
        if !std::path::Path::new(path).exists() {
            eprintln!("skipping: run build_collection_z_manifests.py first");
            return;
        }
        let m = load_manifest(path).expect("the builder's own output must load");
        assert_eq!(m.len(), 4444, "every serial, none invented");
        // Serials are 0-based and contiguous: the same indices the manifest
        // root was derived over (§86.1).
        assert!(m.contains_key(&0) && m.contains_key(&4443));
        assert_eq!(*m.keys().next().unwrap(), 0);
        assert_eq!(*m.keys().next_back().unwrap(), 4443);
        assert!(m.values().all(|c| *c != [0u8; 32]), "no zero commitments");
        // Item 0's content, pinned: this is the value the mint will name, and
        // it matches what zyn-cave-manifest hashed into the root.
        assert_eq!(
            hex(&m[&0]),
            "fdd5ddd8363c800e9f84bff82d8d209d4bfa93f6df82db2a6d5273c0c7595b3b"
        );
    }

    #[test]
    fn a_missing_or_empty_manifest_is_refused_rather_than_defaulted() {
        assert!(load_manifest("").is_err(), "no path is an error, not an empty map");
        assert!(load_manifest("/nonexistent/content.json").is_err());
    }

    /// A serial whose content is not 32 bytes must fail the load, not mint an
    /// item committing to something truncated.
    #[test]
    fn a_short_content_hash_fails_the_load() {
        let dir = std::env::temp_dir().join(format!("zyn-sale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.json");
        std::fs::write(&p, r#"{"0":{"content":"abcd"}}"#).unwrap();
        assert!(load_manifest(p.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// One zatoshi in `Fixed`'s 1e18 scale, the same constant the scanner uses.
const ZAT: i128 = 10_000_000_000;

/// The chain, through a real node and a read-only vault key.
struct ZebraChain {
    zebra: zyn_custody::zebra::Zebra,
    scanner: zyn_custody::shielded::Scanner,
}

impl zynzapd::draw::Chain for ZebraChain {
    fn tip(&self) -> Result<u64, String> {
        self.zebra.block_count().map_err(|e| e.to_string())
    }

    fn cave_payments(
        &self,
        from: u64,
        to: u64,
    ) -> Result<(u64, Vec<zynzapd::reveal::Sighting>), String> {
        let (reached, found) = self
            .scanner
            .cave_payments(&self.zebra, from, to)
            // `Unattributable` is fatal by design: shielded funds arrived that
            // nobody can be credited for, and drawing past it would allocate a
            // sale while somebody's money sat unaccounted.
            .map_err(|e| format!("scan {from}..={to}: {e:?}"))?;
        let mut out = Vec::with_capacity(found.len());
        for s in found {
            let zatoshi = u64::try_from(s.amount.0 / ZAT)
                .map_err(|_| format!("payment in {} is not a whole zatoshi amount", hex(&s.txid)))?;
            out.push(zynzapd::reveal::Sighting {
                txid: s.txid,
                height: s.height,
                tx_index: s.tx_index,
                output_index: s.output_index,
                amount_zatoshi: zatoshi,
                payment: s.payment,
            });
        }
        Ok((reached, out))
    }

    fn block_hash(&self, height: u64) -> Result<[u8; 32], String> {
        self.zebra.block_hash(height).map_err(|e| e.to_string())
    }
}

/// The announcement as the operator published it, plus where it landed.
///
/// A file rather than six environment variables, because these travel together
/// and a verifier is handed the same object.
fn load_announcement(
    path: &str,
) -> Result<(cave_randomness::CloseAnnouncement, cave_randomness::PublicationProof), String> {
    if path.trim().is_empty() {
        return Err("set ZYN_SALE_ANNOUNCEMENT to the published close announcement".into());
    }
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    let d: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("{}: {}", path, e))?;
    let n = |k: &str| -> Result<u64, String> {
        d.get(k)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("{}: {} is missing or not a number", path, k))
    };
    let txid = d
        .get("publication_txid")
        .and_then(serde_json::Value::as_str)
        .and_then(unhex32)
        .ok_or_else(|| format!("{}: publication_txid must be 64 hex", path))?;
    let announcement = cave_randomness::CloseAnnouncement {
        // Bound to the policy this binary holds, not to a digest in the file:
        // a file that could name its own policy could name a different sale.
        policy_digest: zynzapd::cave_z::policy()
            .digest()
            .map_err(|e| format!("the pinned policy does not validate: {e}"))?,
        announced_at_zcash_height: n("announced_at_zcash_height")? as u32,
        announced_at_drand_round: n("announced_at_drand_round")?,
        deposit_close_zcash_height: n("deposit_close_zcash_height")? as u32,
    };
    let proof = cave_randomness::PublicationProof {
        txid,
        height: n("publication_height")? as u32,
    };
    Ok((announcement, proof))
}

/// `zyn-sale draw` — run the collection-wide draw and report it.
fn draw_command() -> i32 {
    match run_draw() {
        Ok(report) => {
            print!("{}", report);
            0
        }
        Err(e) => {
            eprintln!("zyn-sale draw: {}", e);
            1
        }
    }
}

fn run_draw() -> Result<String, String> {
    let policy = zynzapd::cave_z::policy();
    let terms_digest = zynzapd::cave_z::terms(&policy).digest();

    let content = load_manifest(&env("ZYN_SALE_MANIFEST", ""))?;
    let manifest = zynzapd::cave_z::manifest_from(&content);
    // The committed root is what buyers were promised. A manifest that rebuilds
    // a different one is a different collection, and finding that out here is
    // far cheaper than finding it out from a buyer (§106.4).
    let root = cave_randomness::manifest_root(policy.collection_id, &manifest)
        .map_err(|e| format!("the manifest does not form a root: {e}"))?;
    if root != policy.manifest_root {
        return Err(format!(
            "the manifest rebuilds {} but the policy commits {} — this is not that collection",
            hex(&root),
            hex(&policy.manifest_root)
        ));
    }

    let (announcement, proof) = load_announcement(&env("ZYN_SALE_ANNOUNCEMENT", ""))?;
    let scan_from: u64 = env("ZYN_SALE_SCAN_FROM", "0")
        .parse()
        .map_err(|_| "ZYN_SALE_SCAN_FROM must be a height".to_string())?;
    if scan_from == 0 {
        return Err("set ZYN_SALE_SCAN_FROM — scanning from 0 is a day of work, and \
                    scanning from too high silently loses whoever paid early"
            .into());
    }
    let plan = zynzapd::draw::Plan::from_announcement(
        &policy,
        &announcement,
        &proof,
        zynzapd::cave_z::CONFIRMATIONS,
        scan_from,
        terms_digest,
    )?;

    let fvk_hex = env("ZYN_VAULT_FVK", "");
    let bytes = unhex(&fvk_hex).ok_or("ZYN_VAULT_FVK must be 96 bytes of hex")?;
    let bytes: [u8; 96] = bytes
        .try_into()
        .map_err(|_| "ZYN_VAULT_FVK must be 96 bytes of hex".to_string())?;
    let fvk = orchard::keys::FullViewingKey::from_bytes(&bytes)
        .ok_or("ZYN_VAULT_FVK is not a valid Orchard full viewing key")?;
    let max_blocks: u64 = env("ZYN_BRIDGE_MAX_BLOCKS", "200").parse().unwrap_or(200);
    let chain = ZebraChain {
        zebra: zyn_custody::zebra::Zebra::connect(
            &env("ZYN_ZEBRA_HOST", "127.0.0.1"),
            env("ZYN_ZEBRA_PORT", "8232")
                .parse()
                .map_err(|_| "ZYN_ZEBRA_PORT must be a port".to_string())?,
            match (
                std::env::var("ZYN_ZEBRA_USER").ok(),
                std::env::var("ZYN_ZEBRA_PASS").ok(),
            ) {
                (Some(u), Some(p)) => Some((Box::leak(u.into_boxed_str()) as &str, Box::leak(p.into_boxed_str()) as &str)),
                _ => None,
            },
            zyn_custody::zebra::Network::Mainnet,
        )
        .map_err(|e| format!("cannot reach the Zcash node: {e}"))?,
        scanner: zyn_custody::shielded::Scanner::new(
            zyn_custody::shielded::VaultKeys::from_full_viewing_key(fvk),
            scan_from,
            max_blocks,
        ),
    };

    let get = |url: &str| -> Result<Vec<u8>, String> {
        let r = ureq::get(url)
            .timeout(std::time::Duration::from_secs(20))
            .call()
            .map_err(|e| format!("{url}: {e}"))?;
        let mut body = Vec::new();
        r.into_reader()
            .read_to_end(&mut body)
            .map_err(|e| format!("{url}: {e}"))?;
        Ok(body)
    };

    let outcome = zynzapd::draw::execute(&plan, &policy, &manifest, &chain, &get)?;

    let mut report = String::new();
    report.push_str(&format!(
        "close {} (reveal block {}, drand round {})\n",
        plan.close_height, plan.reveal_zcash_height, plan.reveal_drand_round
    ));
    report.push_str(&format!("terms digest      {}\n", hex(&terms_digest)));
    report.push_str(&outcome.report());
    report.push_str("\nassignment:\n");
    for (account, serials) in outcome.run.by_account() {
        report.push_str(&format!(
            "  {} -> {}\n",
            hex(&account),
            serials
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    Ok(report)
}

/// `zyn-sale reconcile` — find entries whose collection share never landed, and
/// finish them.
///
/// The failure it repairs is silent: a buyer paid, the treasury holds their
/// money, and the pool that backs every holder's redemption is short by one
/// entry's 90%. Nothing surfaces that except this.
fn reconcile_command(s: &Service) -> i32 {
    let mut book = match s.book.lock() {
        Ok(b) => b,
        Err(_) => {
            eprintln!("zyn-sale reconcile: the order book is locked");
            return 1;
        }
    };
    let owed = sale::unfunded(&book);
    if owed.is_empty() {
        println!("zyn-sale reconcile: every paid entry has funded the pool");
        return 0;
    }
    println!("zyn-sale reconcile: {} entry(s) paid with the pool's share unfunded", owed.len());

    let (pool_zat, _) = s.terms.split(s.terms.entry_price_zatoshi);
    let pool = Fixed(pool_zat as i128 * 10_000_000_000);
    let mut fixed = 0usize;
    for order in &owed {
        let fund = || -> Result<(), String> {
            let epoch = s.node.status().map_err(|e| e.to_string())?.epoch;
            let rid = s.request.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let frame = zynzapd::treasury::frame_submission_across(
                &s.custodians,
                &s.public,
                s.threshold,
                s.chain,
                epoch,
                &Intent::FundCollection {
                    from: s.treasury,
                    collection: s.collection,
                    amount: pool,
                },
                rid,
            )?;
            s.node.submit_raw(&frame).map_err(|e| e.to_string())?;
            Ok(())
        };
        match sale::reconcile(&mut book, order, &fund) {
            // Stops at the first failure rather than carrying on: if the
            // treasury cannot fund one share it will not fund the next, and a
            // long list of identical errors buries the reason.
            Err(e) => {
                eprintln!("  {}: {}", hex(&order.code_hash), e);
                eprintln!("zyn-sale reconcile: stopped after {} of {}", fixed, owed.len());
                return 1;
            }
            Ok(true) => {
                fixed += 1;
                println!("  {} funded {}", hex(&order.code_hash), pool);
            }
            Ok(false) => println!("  {} was already funded", hex(&order.code_hash)),
        }
    }
    println!("zyn-sale reconcile: {} of {} finished", fixed, owed.len());
    0
}
