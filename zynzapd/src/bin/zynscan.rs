//! `zynscan` — the object ledger's epochs and transactions, readable by anyone.
//!
//! Every intent the sequencer accepts is journaled before it is applied, one
//! file per epoch. This replays those files through `ObjectVm` — the rules a
//! replica uses, nothing interpreted by hand — and serves what the public may
//! see of the result over a small read-only HTTP API:
//!
//! * **Epochs**: number, sealed state root, when it sealed, how many
//!   transactions, and whether a Zcash transaction carries it yet.
//! * **Transactions**: sequence, epoch, the kind of operation, accepted or
//!   refused, and how many notes it created and spent.
//! * **Market state**: the node's own public `zyn_market` and `zyn_status`.
//!
//! What it does not serve is the reason it exists at tier 2 (DECISIONS
//! §130.12, §133.4): "private from the public; we can see them". Accounts,
//! amounts, owners and destinations of holders' operations never leave this
//! process. Operator operations (fees, collections, curve terms, item mints'
//! collection and serial) are public by nature and are shown whole; for a
//! holder's operation the public learns that it happened, what kind it was,
//! and which public object it touched (a pool, a curve token) — nothing that
//! says who, or how much.
//!
//! ```text
//!   ZYN_CHAIN_ID        the chain                       (required)
//!   ZYN_SCAN_DATA       the node's data dir (journal/)  (required)
//!   ZYN_SCAN_NODE       the node's JSON-RPC             (http://127.0.0.1:8499)
//!   ZYN_SCAN_LISTEN     HTTP, read-only                 (127.0.0.1:8090)
//!   ZYN_SCAN_REFRESH    seconds between reads           (5)
//!   ZYN_SCAN_NETWORK    label shown on the page         (testnet)
//!   ZYN_MAX_IO          the chain's I/O limit           (16, as the node)
//! ```
//!
//! Nothing here can write: it reads journal files and a public RPC, holds no
//! key and has no submit path. The chain is read on a timer, not per request,
//! so attention on the page never becomes load on the sequencer.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use objectvm::{ObjectVm, Op, Receipt};
use serde_json::{json, Value};
use zyn_vm::spec::MicrochainVm;

fn env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// What the public may see of one intent.

struct Tx {
    seq: u64,
    epoch: u64,
    kind: &'static str,
    operator: bool,
    ok: bool,
    refusal: Option<String>,
    created: usize,
    spent: usize,
    detail: Value,
}

impl Tx {
    fn json(&self) -> Value {
        json!({
            "seq": self.seq, "epoch": self.epoch, "kind": self.kind, "operator": self.operator,
            "ok": self.ok, "refusal": self.refusal, "created": self.created, "spent": self.spent,
            "detail": self.detail,
        })
    }
}

/// The kind of an operation, whether only the operator may send it, and the
/// fields of it that are public. For a holder's operation that is at most a
/// public object id; never an account, an amount or a destination.
fn describe(op: &Op, receipts: &[Receipt]) -> (&'static str, bool, Value) {
    let from_receipt = |f: &dyn Fn(&Receipt) -> Option<Value>| receipts.iter().find_map(f).unwrap_or(Value::Null);
    match op {
        Op::Mint { .. } => ("Mint (devnet)", true, Value::Null),
        Op::Spend { .. } => ("Transfer", false, Value::Null),
        Op::SettleExit { index, txid, .. } => ("Exit paid", true, json!({ "index": index, "zcash_txid": hex(txid) })),
        Op::Register { .. } => ("Register", false, Value::Null),
        Op::Withdraw { .. } => ("Withdraw to Zcash", false, Value::Null),
        Op::Deposit { .. } => ("Deposit", true, Value::Null),
        Op::DepositWithFee { .. } => ("Deposit", true, Value::Null),
        Op::PlaceOffer { .. } => ("Offer placed", false, Value::Null),
        Op::CancelOffer { .. } => ("Offer cancelled", false, Value::Null),
        Op::TakeOffer { .. } => ("Offer taken", false, Value::Null),
        Op::CreatePool { .. } => (
            "Pool created",
            false,
            from_receipt(&|r| match r {
                Receipt::Pooled { pool, .. } => Some(json!({ "pool": hex(pool) })),
                _ => None,
            }),
        ),
        Op::AddLiquidity { pool, .. } => ("Liquidity added", false, json!({ "pool": hex(pool) })),
        Op::RemoveLiquidity { pool, .. } => ("Liquidity removed", false, json!({ "pool": hex(pool) })),
        Op::Swap { pool, .. } => ("Swap", false, json!({ "pool": hex(pool) })),
        Op::Claim { .. } => ("Claim", false, Value::Null),
        Op::CreateLaunch { .. } => (
            "Launch created",
            false,
            from_receipt(&|r| match r {
                Receipt::Launched { token } => Some(json!({ "token": hex(token) })),
                _ => None,
            }),
        ),
        Op::Commit { .. } => ("Launch commit", false, Value::Null),
        Op::SetFees { bridge_bps, .. } => ("Fees set", true, json!({ "bridge_bps": bridge_bps })),
        Op::RegisterCollection { asset, creator, quote, fee_bps, cap, .. } => (
            "Collection registered",
            true,
            json!({ "collection": hex(asset), "creator": hex(creator), "quote": hex(quote), "fee_bps": fee_bps, "cap": cap }),
        ),
        Op::List { .. } => ("Item listed", false, Value::Null),
        Op::MintItem { collection, serial, .. } => ("Item minted", true, json!({ "collection": hex(collection), "serial": serial })),
        Op::Redeem { asset, .. } => ("Item redeemed", false, json!({ "collection_asset": hex(asset) })),
        Op::CreateCurve { .. } => (
            "Coin created",
            false,
            from_receipt(&|r| match r {
                Receipt::CurveCreated { token } => Some(json!({ "token": hex(token) })),
                _ => None,
            }),
        ),
        Op::CurveTrade { token, .. } => ("Curve trade", false, json!({ "token": hex(token) })),
        Op::SetCurveTerms { fee_bps, bond, .. } => ("Curve terms set", true, json!({ "fee_bps": fee_bps, "bond": bond.0.to_string() })),
        Op::SetSale { collection, ticket_price, claim_price, open_price, treasury_bps, .. } => (
            "Sale opened",
            true,
            json!({ "collection": hex(collection), "ticket_price": ticket_price.0.to_string(), "claim_price": claim_price.0.to_string(), "open_price": open_price.0.to_string(), "treasury_bps": treasury_bps }),
        ),
        // A ticket's id is its buyer's to derive; the count and the sale are public.
        Op::BuyTickets { collection, count, .. } => ("Tickets bought", false, json!({ "collection": hex(collection), "tickets": count })),
        Op::CloseSale { collection, zcash_height, zcash_hash, .. } => (
            "Sale closed",
            true,
            json!({ "collection": hex(collection), "zcash_height": zcash_height, "zcash_block": hex(zcash_hash) }),
        ),
        Op::PostDraw { collection, root, .. } => ("Draw posted", true, json!({ "collection": hex(collection), "root": hex(root) })),
        // Minted items are public (collection and serial); who claimed them is not.
        Op::ClaimItems { collection, serials, .. } => ("Items claimed", false, json!({ "collection": hex(collection), "serials": serials })),
        Op::PostOpenMint { collection, root, count, .. } => ("Open mint posted", true, json!({ "collection": hex(collection), "root": hex(root), "items": count })),
        Op::MintOpen { collection, serial, .. } => ("Open mint", false, json!({ "collection": hex(collection), "serial": serial })),
        Op::SetVoucherCap { collection, cap, .. } => ("Burn tickets capped", true, json!({ "collection": hex(collection), "cap": cap })),
        // A burn is public on its own chain already; which Zyn account it
        // belongs to is not shown (the commitment hides it).
        Op::PostVoucher { collection, burn, count, .. } => ("Burn posted", true, json!({ "collection": hex(collection), "burn": hex(burn), "tickets": count })),
        Op::ClaimVoucherItems { collection, serials, .. } => ("Items claimed", false, json!({ "collection": hex(collection), "serials": serials })),
        Op::RegisterBridged { network, origin, decimals, .. } => (
            "Bridged asset registered",
            true,
            json!({ "network": String::from_utf8_lossy(network), "origin": hex(origin), "decimals": decimals }),
        ),
        // Like a ZEC deposit: which asset arrived, not how much or for whom.
        Op::DepositCommitted { asset, .. } => ("Bridge deposit", true, json!({ "asset": hex(asset) })),
        Op::ClaimDeposit { asset, .. } => ("Bridge deposit claimed", false, json!({ "asset": hex(asset) })),
        Op::Seal => ("Seal", true, Value::Null),
    }
}

// ---------------------------------------------------------------------------
// The index: replayed incrementally from the journal.

struct Epoch {
    n: u64,
    root: Option<[u8; 32]>,
    sealed_at: Option<u64>,
    seqs: Vec<u64>,
}

struct Index {
    chain: u32,
    max_io: u32,
    data: PathBuf,
    vm: ObjectVm,
    epochs: BTreeMap<u64, Epoch>,
    txs: BTreeMap<u64, Tx>,
    /// The next epoch file to read, and how many of its records are in.
    next_epoch: u64,
    applied_in_open: usize,
    anchors: BTreeMap<u64, (u64, String)>,
    market: Value,
    status: Value,
    error: Option<String>,
    at: u64,
}

impl Index {
    fn new(chain: u32, max_io: u32, data: PathBuf) -> Index {
        Index {
            chain,
            max_io,
            data,
            vm: zynzapd::object_verify::fresh(chain, max_io),
            epochs: BTreeMap::new(),
            txs: BTreeMap::new(),
            next_epoch: 0,
            applied_in_open: 0,
            anchors: BTreeMap::new(),
            market: Value::Null,
            status: Value::Null,
            error: None,
            at: 0,
        }
    }

    fn reset(&mut self) {
        let fresh = Index::new(self.chain, self.max_io, self.data.clone());
        self.vm = fresh.vm;
        self.epochs.clear();
        self.txs.clear();
        self.next_epoch = 0;
        self.applied_in_open = 0;
    }

    /// Read whatever the journal holds past where this index is. Stops at the
    /// open epoch; picks up its new records next time.
    fn ingest(&mut self) -> Result<(), String> {
        loop {
            let path = zyn::journal::epoch_path(&self.data, self.chain, self.next_epoch);
            let Ok(bytes) = std::fs::read(&path) else { return Ok(()) };
            let file = zyn::journal::read_epoch(&bytes).map_err(|e| format!("epoch {}: {e:?}", self.next_epoch))?;
            if file.version != 2 {
                return Err(format!("epoch {} is a version {} journal; zynscan reads version 2", file.epoch, file.version));
            }
            if file.chain_id != self.chain || file.epoch != self.next_epoch {
                return Err(format!("{} is not chain {} epoch {}", path.display(), self.chain, self.next_epoch));
            }
            let epoch_n = self.next_epoch;
            self.epochs.entry(epoch_n).or_insert(Epoch { n: epoch_n, root: None, sealed_at: None, seqs: Vec::new() });
            let mut sealed = false;
            for (seq, record) in file.records.iter().skip(self.applied_in_open) {
                let expected = self.vm.seq() + 1;
                if *seq != expected {
                    return Err(format!("sequence gap in epoch {epoch_n}: expected {expected}, got {seq}"));
                }
                let authorized = zyn_vm::verify::decode_authorized::<ObjectVm>(record, &self.vm)
                    .map_err(|e| format!("seq {seq} does not verify: {e:?}"))?;
                let op = authorized.into_intent();
                let receipts = self.vm.apply_committed(*seq, &op, record);
                let (kind, operator, detail) = describe(&op, &receipts);
                let refusal = receipts.iter().find_map(|r| match r {
                    Receipt::Refused(why) => Some(why.to_string()),
                    _ => None,
                });
                self.txs.insert(*seq, Tx {
                    seq: *seq,
                    epoch: epoch_n,
                    kind,
                    operator,
                    ok: refusal.is_none(),
                    refusal,
                    created: op.created().len(),
                    spent: op.retired().len(),
                    detail,
                });
                self.applied_in_open += 1;
                let e = self.epochs.get_mut(&epoch_n).expect("inserted above");
                e.seqs.push(*seq);
                if let Some(cp) = ObjectVm::sealed(&receipts) {
                    e.root = Some(cp.state_root);
                    e.sealed_at = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());
                    sealed = true;
                }
            }
            if !sealed {
                return Ok(());
            }
            self.next_epoch += 1;
            self.applied_in_open = 0;
        }
    }

    fn refresh(&mut self, node: &str) {
        let mut errors = Vec::new();
        if let Err(e) = self.ingest() {
            // A journal that no longer replays from where we were (a rewound
            // node, a restored backup) is rebuilt from genesis rather than
            // patched: the index must be what the files say, all of it.
            self.reset();
            if let Err(e2) = self.ingest() {
                errors.push(format!("journal: {e2}"));
            } else {
                eprintln!("zynscan: rebuilt from genesis after: {e}");
            }
        }
        match zynzapd::object_client::call(node, "zyn_status", json!({})) {
            Ok(v) => self.status = v,
            Err(e) => errors.push(format!("status: {e}")),
        }
        match zynzapd::object_client::call(node, "zyn_market", json!({})) {
            Ok(v) => self.market = v,
            Err(e) => errors.push(format!("market: {e}")),
        }
        if let Ok(Value::Array(rows)) = zynzapd::object_client::call(node, "zyn_anchors", json!({})) {
            for r in rows {
                if let (Some(e), Some(h), Some(t)) = (r["epoch"].as_u64(), r["height"].as_u64(), r["txid"].as_str()) {
                    if h > 0 {
                        self.anchors.insert(e, (h, t.to_string()));
                    }
                }
            }
        }
        self.error = if errors.is_empty() { None } else { Some(errors.join("; ")) };
        self.at = now();
    }

    /// The first anchor at or after `epoch`: an anchor covers every epoch
    /// since the one before it.
    fn anchor_for(&self, epoch: u64) -> Value {
        match self.anchors.range(epoch..).next() {
            Some((e, (h, t))) => json!({ "epoch": e, "zcash_height": h, "zcash_txid": t }),
            None => Value::Null,
        }
    }

    fn epoch_json(&self, e: &Epoch, with_txs: bool) -> Value {
        let txs: Vec<&Tx> = e.seqs.iter().filter_map(|s| self.txs.get(s)).filter(|t| t.kind != "Seal").collect();
        let mut v = json!({
            "epoch": e.n,
            "sealed": e.root.is_some(),
            "root": e.root.map(|r| hex(&r)),
            "sealed_at": e.sealed_at,
            "txs": txs.len(),
            "refused": txs.iter().filter(|t| !t.ok).count(),
            "anchor": self.anchor_for(e.n),
        });
        if with_txs {
            v["transactions"] = Value::Array(txs.iter().map(|t| t.json()).collect());
        }
        v
    }
}

// ---------------------------------------------------------------------------
// HTTP.

fn query(path: &str) -> (String, BTreeMap<String, String>) {
    let (p, q) = path.split_once('?').unwrap_or((path, ""));
    let params = q
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    (p.trim_end_matches('/').to_string(), params)
}

fn route(ix: &Index, network: &str, path: &str) -> (u16, Value) {
    let (p, q) = query(path);
    let num = |k: &str| q.get(k).and_then(|v| v.parse::<u64>().ok());
    let limit = num("limit").unwrap_or(25).clamp(1, 100) as usize;
    let user_txs = || ix.txs.values().filter(|t| t.kind != "Seal");
    match p.as_str() {
        "/api/summary" | "/api" => {
            let last_sealed = ix.epochs.values().rev().find(|e| e.root.is_some());
            (200, json!({
                "chain": ix.chain,
                "network": network,
                "epoch": ix.status.get("epoch").cloned().unwrap_or(Value::Null),
                "seq": ix.vm.seq(),
                "sealed_epochs": ix.epochs.values().filter(|e| e.root.is_some()).count(),
                "transactions": user_txs().count(),
                "refused": user_txs().filter(|t| !t.ok).count(),
                "last_root": last_sealed.and_then(|e| e.root).map(|r| hex(&r)),
                "last_sealed_epoch": last_sealed.map(|e| e.n),
                "last_sealed_at": last_sealed.and_then(|e| e.sealed_at),
                "anchored_through": ix.anchors.keys().next_back(),
                // The index's own replay against the node's word for it: equal
                // roots at the same sequence mean the journal this page reads
                // reproduces exactly the state the sequencer holds.
                "replayed_root": hex(&ix.vm.state_root()),
                "node_root": ix.status.get("state_root").cloned().unwrap_or(Value::Null),
                "agrees": match (ix.status.get("seq").and_then(Value::as_u64), ix.status.get("state_root").and_then(Value::as_str)) {
                    (Some(seq), Some(root)) if seq == ix.vm.seq() => Value::Bool(root == hex(&ix.vm.state_root())),
                    _ => Value::Null,
                },
                "kinds": user_txs().fold(BTreeMap::<&str, u64>::new(), |mut m, t| { *m.entry(t.kind).or_default() += 1; m }),
                "updated_at": ix.at,
                "error": ix.error,
                "privacy": "tier 2: accounts, amounts and owners of holders' operations are not published",
            }))
        }
        "/api/epochs" => {
            let before = num("before").unwrap_or(u64::MAX);
            let rows: Vec<Value> = ix.epochs.range(..before).rev().take(limit).map(|(_, e)| ix.epoch_json(e, false)).collect();
            (200, Value::Array(rows))
        }
        "/api/txs" => {
            let before = num("before").unwrap_or(u64::MAX);
            let rows: Vec<Value> = ix.txs.range(..before).rev().filter(|(_, t)| t.kind != "Seal").take(limit).map(|(_, t)| t.json()).collect();
            (200, Value::Array(rows))
        }
        "/api/market" => (200, json!({ "market": ix.market, "status": ix.status })),
        "/health" => (200, json!({ "ok": ix.error.is_none(), "updated_at": ix.at })),
        _ => {
            if let Some(n) = p.strip_prefix("/api/epoch/").and_then(|s| s.parse::<u64>().ok()) {
                return match ix.epochs.get(&n) {
                    Some(e) => (200, ix.epoch_json(e, true)),
                    None => (404, json!({ "error": "no such epoch yet" })),
                };
            }
            if let Some(n) = p.strip_prefix("/api/tx/").and_then(|s| s.parse::<u64>().ok()) {
                return match ix.txs.get(&n) {
                    Some(t) => {
                        let mut v = t.json();
                        v["anchor"] = ix.anchor_for(t.epoch);
                        (200, v)
                    }
                    None => (404, json!({ "error": "no such transaction yet" })),
                };
            }
            (404, json!({ "error": "not found" }))
        }
    }
}

fn serve(mut s: TcpStream, ix: &Arc<Mutex<Index>>, network: &str) {
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let mut buf = [0u8; 4096];
    let n = s.read(&mut buf).unwrap_or(0);
    let head = String::from_utf8_lossy(&buf[..n]);
    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    let (code, body) = if method == "OPTIONS" {
        (204, Value::Null)
    } else if method != "GET" {
        (405, json!({ "error": "read-only" }))
    } else {
        let ix = ix.lock().unwrap();
        route(&ix, network, path)
    };
    let text = if body.is_null() { String::new() } else { body.to_string() };
    let reason = match code { 200 => "OK", 204 => "No Content", 404 => "Not Found", _ => "Method Not Allowed" };
    let _ = write!(
        s,
        "HTTP/1.1 {code} {reason}\r\ncontent-type: application/json\r\naccess-control-allow-origin: *\r\naccess-control-allow-methods: GET, OPTIONS\r\ncache-control: public, max-age=3\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    );
}

fn main() {
    let chain: u32 = match std::env::var("ZYN_CHAIN_ID").ok().and_then(|v| v.parse().ok()) {
        Some(c) => c,
        None => {
            eprintln!("zynscan: ZYN_CHAIN_ID is required — it names the chain to serve (26460 mainnet, 212 testnet)");
            std::process::exit(1)
        }
    };
    let data = match std::env::var("ZYN_SCAN_DATA").ok().filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            eprintln!("zynscan: ZYN_SCAN_DATA is required — the node's data directory, which holds journal/");
            std::process::exit(1)
        }
    };
    let node = env("ZYN_SCAN_NODE", "http://127.0.0.1:8499");
    let listen = env("ZYN_SCAN_LISTEN", "127.0.0.1:8090");
    let network = env("ZYN_SCAN_NETWORK", "testnet");
    let refresh: u64 = env("ZYN_SCAN_REFRESH", "5").parse().unwrap_or(5).max(1);
    let max_io: u32 = env("ZYN_MAX_IO", "").parse().unwrap_or(zynzapd::object_verify::DEFAULT_MAX_IO);

    let ix = Arc::new(Mutex::new(Index::new(chain, max_io, data)));
    ix.lock().unwrap().refresh(&node);
    {
        let i = ix.lock().unwrap();
        eprintln!(
            "zynscan: chain {chain} ({network}) — {} epochs, {} records replayed; serving {listen}{}",
            i.epochs.len(),
            i.txs.len(),
            i.error.as_ref().map(|e| format!(" (warning: {e})")).unwrap_or_default()
        );
    }
    {
        let ix = ix.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(refresh));
            ix.lock().unwrap().refresh(&node);
        });
    }
    let listener = TcpListener::bind(&listen).unwrap_or_else(|e| {
        eprintln!("zynscan: cannot listen on {listen}: {e}");
        std::process::exit(1)
    });
    for s in listener.incoming().flatten() {
        let ix = ix.clone();
        let network = network.clone();
        std::thread::spawn(move || serve(s, &ix, &network));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A holder's operation says what kind it was and nothing about who or
    /// how much: the whole privacy line of this binary, pinned.
    #[test]
    fn a_holders_operation_publishes_no_account_or_amount() {
        let op = Op::CurveTrade {
            trader: [0xA1; 32],
            program: zyn_vm::program::curve_trade(),
            token: [0x77; 32],
            buy: true,
            amount_in: zyn_vm::Fixed::whole(3),
            min_out: zyn_vm::Fixed::ZERO,
            inputs: vec![],
            change: vec![],
            nonce: [9; 32],
        };
        let (kind, operator, detail) = describe(&op, &[]);
        assert_eq!(kind, "Curve trade");
        assert!(!operator);
        let text = detail.to_string();
        assert!(!text.contains(&hex(&[0xA1; 32])), "the trader must not appear: {text}");
        assert!(!text.contains("3000000000000000000"), "the amount must not appear: {text}");
        assert!(text.contains(&hex(&[0x77; 32])), "the public token does");
    }

    #[test]
    fn a_journal_replays_into_epochs_and_transactions() {
        let dir = std::env::temp_dir().join(format!("zynscan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut j = zyn::journal::Journal::open(&dir, 212).unwrap();
        let mut vm = zynzapd::object_verify::fresh(212, 16);
        let ops = [
            Op::SetFees { program: zyn_vm::program::configure_fees(), bridge_bps: 50 },
            Op::Seal,
            Op::SetCurveTerms { program: zyn_vm::program::configure_curves(), fee_bps: 100, bond: zyn_vm::Fixed::ZERO },
        ];
        let mut epoch = 0;
        for op in ops {
            let enc = zyn_vm::verify::encode_authorized::<ObjectVm>(&zyn_vm::verify::Authorized::operator(op.clone()));
            let seq = vm.seq() + 1;
            j.record(epoch, seq, &enc).unwrap();
            let r = vm.apply_committed(seq, &op, &enc);
            assert!(!ObjectVm::rejected(&r[0]), "{r:?}");
            if ObjectVm::sealed(&r).is_some() {
                epoch += 1;
            }
        }
        j.sync().unwrap();
        let mut ix = Index::new(212, 16, dir.clone());
        ix.ingest().unwrap();
        assert_eq!(ix.epochs.len(), 2, "one sealed epoch and the open one");
        assert!(ix.epochs[&0].root.is_some() && ix.epochs[&1].root.is_none());
        let (_, v) = route(&ix, "testnet", "/api/epoch/0");
        assert_eq!(v["txs"], 1);
        assert_eq!(v["transactions"][0]["kind"], "Fees set");
        assert_eq!(v["transactions"][0]["detail"]["bridge_bps"], 50);
        let (_, t) = route(&ix, "testnet", "/api/txs");
        assert_eq!(t.as_array().unwrap().len(), 2, "seals are not listed as transactions");
        // Picking up new records later, without replaying what it has.
        let op = Op::SetFees { program: zyn_vm::program::configure_fees(), bridge_bps: 60 };
        let enc = zyn_vm::verify::encode_authorized::<ObjectVm>(&zyn_vm::verify::Authorized::operator(op));
        j.record(1, vm.seq() + 1, &enc).unwrap();
        j.sync().unwrap();
        ix.ingest().unwrap();
        assert_eq!(route(&ix, "testnet", "/api/txs").1.as_array().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
