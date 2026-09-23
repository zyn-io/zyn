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
//!   ZYN_DA_MIRRORS     url[|token],… to PUT bundles to (none)
//!   ZYN_BRIDGE_SEED    hex, ≥32 bytes; turns deposits on (off)
//!   ZYN_BRIDGE_FROM_HEIGHT  where the replaced chain stopped; deposits
//!                      below it are not this chain's       (vault birth)
//!   ZYN_ZEC_ASSET      the asset bridged ZEC becomes    (XZEC)
//!   ZYN_EXIT_REVEALS   where exits say to pay           (/etc/zyn/zcash-exit-reveals)
//!   ZYN_SCAN_STEP      blocks per scan step             (25)
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
    /// `diversifier index -> account`: the deposit addresses that exist.
    ///
    /// Derived, not stored. `deposit_index` is a pure function of the account
    /// id, so every registered account already *has* a deposit address —
    /// this register is just the scanner being told which ones to recognise.
    addresses: std::sync::Arc<Mutex<std::collections::BTreeMap<[u8; 11], zyn_vm::spec::AccountId>>>,
    /// Enough to derive an address, and nothing that can spend.
    keys: zyn_custody::shielded::VaultKeys,
    network: zyn_custody::zebra::Network,
    /// Pays exits out of the vault, when there is somewhere to pay them.
    exits: Option<zynzapd::object_exit::ExitSettler>,
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

/// Pools and sales, public by design. Amounts are raw WAD integers as
/// strings: a wallet computes prices from them in `BigInt` and must never
/// round a reserve through a double.
fn market_json(vm: &ObjectVm, epoch_seconds: u64) -> String {
    use objectvm::market::{lp_asset, LaunchState};
    let pools: Vec<String> = vm
        .market
        .pools
        .iter()
        .map(|(id, p)| {
            format!(
                "{{\"id\":\"{}\",\"a\":\"{}\",\"b\":\"{}\",\"reserve_a\":\"{}\",\"reserve_b\":\"{}\",\"shares\":\"{}\",\"locked\":\"{}\",\"fee_bps\":{},\"lp\":\"{}\",\"pending\":{}}}",
                hex(id),
                hex(&p.a),
                hex(&p.b),
                p.reserve_a.0,
                p.reserve_b.0,
                p.shares.0,
                p.locked.0,
                p.fee_bps,
                hex(&lp_asset(id)),
                vm.market.orders.iter().filter(|o| &o.pool == id).count()
            )
        })
        .collect();
    let launches: Vec<String> = vm
        .market
        .launches
        .iter()
        .map(|(t, l)| {
            format!(
                "{{\"token\":\"{}\",\"quote\":\"{}\",\"total_supply\":\"{}\",\"sale_supply\":\"{}\",\"pool_supply\":\"{}\",\"soft_cap\":\"{}\",\"hard_cap\":\"{}\",\"pool_bps\":{},\"fee_bps\":{},\"closes_at_epoch\":{},\"meta\":\"{}\",\"raised\":\"{}\",\"state\":\"{}\"}}",
                hex(t),
                hex(&l.quote),
                l.total_supply.0,
                l.sale_supply.0,
                l.pool_supply.0,
                l.soft_cap.0,
                l.hard_cap.0,
                l.pool_bps,
                l.fee_bps,
                l.closes_at_epoch,
                hex(&l.meta),
                l.raised.0,
                match l.state {
                    LaunchState::Open => "open",
                    LaunchState::Graduated => "graduated",
                    LaunchState::Failed => "failed",
                }
            )
        })
        .collect();
    // What the chain charges and what it has kept — public, like every
    // keyless pot, so anyone can check the fee against the pot.
    let pot: Vec<String> = vm
        .market
        .pol
        .iter()
        .map(|(a, v)| format!("{{\"asset\":\"{}\",\"amount\":\"{}\"}}", hex(a), v.0))
        .collect();
    let collections: Vec<String> = vm
        .market
        .collections
        .iter()
        .map(|(a, c)| {
            format!(
                "{{\"asset\":\"{}\",\"creator\":\"{}\",\"quote\":\"{}\",\"fee_bps\":{},\"floor\":\"{}\",\"outstanding\":\"{}\"}}",
                hex(a),
                hex(&c.creator),
                hex(&c.quote),
                c.fee_bps,
                c.floor.0,
                vm.issued.get(a).map(|f| f.0).unwrap_or(0)
            )
        })
        .collect();
    // Bonding curves: everything the state root already makes public — lots
    // sold, what the curve holds, what it will graduate with, the next price.
    // The creator is not here, as with launches.
    let curves: Vec<String> = vm
        .market
        .curves
        .values()
        .map(|c| {
            let pool = objectvm::market::pool_id(&c.token, &c.quote);
            format!(
                "{{\"token\":\"{}\",\"quote\":\"{}\",\"meta\":\"{}\",\"created_at\":{},\"fee_bps\":{},\"sold\":{},\"lots\":{},\"graduate_at\":{},\"reserve\":\"{}\",\"graduation\":\"{}\",\"creator_fees\":\"{}\",\"bond\":\"{}\",\"price_per_token\":\"{}\",\"graduated\":{},\"pool\":\"{}\",\"pending\":{}}}",
                hex(&c.token),
                hex(&c.quote),
                hex(&c.meta),
                c.created_at,
                c.fee_bps,
                c.sold,
                objectvm::curve::LOTS,
                objectvm::curve::GRADUATE_AT,
                c.reserve.0,
                c.graduation.0,
                c.creator_fees.0,
                c.bond.0,
                c.price_per_token().0,
                c.graduated,
                if c.graduated { hex(&pool) } else { String::new() },
                vm.market.curve_orders.iter().filter(|o| o.token == c.token).count()
            )
        })
        .collect();
    let terms = vm.market.curve_terms.unwrap_or_default();
    format!(
        "{{\"epoch\":{},\"epoch_seconds\":{},\"fees\":{{\"bridge_bps\":{}}},\"curve_terms\":{{\"set\":{},\"fee_bps\":{},\"bond\":\"{}\"}},\"zyn_pot\":[{}],\"collections\":[{}],\"pools\":[{}],\"launches\":[{}],\"curves\":[{}]}}",
        vm.epoch,
        epoch_seconds,
        vm.market.bridge_bps(),
        vm.market.curve_terms.is_some(),
        terms.fee_bps,
        terms.bond.0,
        pot.join(","),
        collections.join(","),
        pools.join(","),
        launches.join(","),
        curves.join(",")
    )
}

/// The one-line JSON the status op answers with.
/// The recent anchors this node has published, oldest first.
///
/// Read from the published index rather than from memory: that file is what
/// `zyn-object-publish` and every replica already treat as the truth, so an
/// answer given here cannot drift from the one given there.
fn recent_anchors(da_dir: Option<&std::path::Path>, chain: u32, limit: usize) -> Vec<zynzapd::publish::IndexEntry> {
    let Some(dir) = da_dir else { return Vec::new() };
    let text = std::fs::read_to_string(dir.join(zynzapd::publish::index_rel(chain))).unwrap_or_default();
    let all = zynzapd::publish::parse_index(&text);
    let from = all.len().saturating_sub(limit);
    all[from..].to_vec()
}

fn status_json(
    vm: &ObjectVm,
    anchoring: bool,
    last: Option<&zynzapd::publish::IndexEntry>,
    sync: &SyncProgress,
) -> String {
    format!(
        concat!(
            "{{\"chain\":{},\"epoch\":{},\"seq\":{},\"accounts\":{},",
            "\"commitments\":{},\"live\":{},\"nullifiers\":{},\"reconciled\":{},",
            "\"offers\":{},\"exits\":{},",
            "\"anchored_epoch\":{},\"anchored_height\":{},\"anchor_txid\":\"{}\",",
            "\"issued\":{},\"deposits_credited\":{},\"owed_zatoshi\":{},",
            "\"synced_to\":{},\"sync_target\":{},\"sync_error\":\"{}\",",
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
        last.map(|a| a.epoch).unwrap_or(0),
        last.map(|a| a.height).unwrap_or(0),
        last.map(|a| a.txid.as_str()).unwrap_or(""),
        // What the ledger claims the vault owes it, per asset. A reader
        // comparing this with the vault's balance on Zcash is doing the same
        // check the bridge does every pass, without needing a key. Never
        // summed across assets: two different assets share no unit, and one
        // number covering both would be a number that means nothing.
        issued_json(vm),
        vm.deposits.len(),
        // What the ledger still owes and has not paid, in zatoshi. Counts
        // exits nobody has revealed a destination for, because an unrevealed
        // exit is still a debt — a vault sized only for the revealed ones is
        // a vault that runs out.
        zynzapd::object_exit::owed(vm, &zec_asset()),
        sync.synced_to,
        sync.target,
        // A JSON string, so a scanner error with a quote in it cannot break
        // the document it is being reported in.
        escape(&sync.error),
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

/// A string safe to drop inside the hand-rolled JSON above.
///
/// The scanner's errors carry txids and operator prose, and one unescaped
/// quote in a status document is a status document nothing can parse — which
/// looks exactly like the node being down.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// `{"<asset hex>": "<amount>"}` — what the ledger has issued, per asset.
fn issued_json(vm: &ObjectVm) -> String {
    let rows: Vec<String> = vm
        .issued
        .iter()
        .map(|(asset, amount)| format!("\"{}\":\"{}\"", hex(asset), amount))
        .collect();
    format!("{{{}}}", rows.join(","))
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
        }
        | Op::DepositWithFee {
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
        Op::MintItem { collection, serial, owner, nonce, enc, .. } => {
            let note = zyn_vm::object::Note {
                asset: objectvm::market::item_asset(collection, *serial),
                amount: objectvm::market::ITEM_UNIT,
                owner: *owner,
                nonce: *nonce,
            };
            if let Ok(c) = zyn_vm::object::commitment(&note) {
                add(&c, enc);
            }
        }
        other => {
            for n in other.retired() {
                pub_log.nullifiers.push(hex(&n));
            }
            for o in other.created() {
                if let Ok(c) = zyn_vm::object::commitment(&o.note) {
                    add(&c, &o.enc);
                }
            }
        }
    }
}

/// Signed frames already applied, so the same authorization cannot run twice.
///
/// A spend replays harmlessly — its inputs are gone — but not everything a
/// holder signs consumes something. A `PlaceOffer` replayed after its
/// `CancelOffer` re-lists an item its owner withdrew, and somebody can then
/// take it; a `Register` replayed after a key rotation points future payments
/// back at the old key. Keyed by a hash of the whole frame, which covers the
/// signature: Ed25519 is deterministic, so re-submitting the identical signed
/// intent produces the identical frame, while a fresh authorization with a new
/// expiry is a different frame and a deliberate act. Pruned at expiry, when
/// the envelope check would refuse the frame anyway.
static REPLAY: std::sync::OnceLock<Mutex<zyn::replay::ReplayIndex>> = std::sync::OnceLock::new();

type AddressRegister = std::sync::Arc<Mutex<std::collections::BTreeMap<[u8; 11], zyn_vm::spec::AccountId>>>;

/// Every deposit address this node has handed out, and where the list lives.
///
/// An address is a pure function of the account, and the scanner recognised
/// only those of accounts that had *registered*. Someone who asked for an
/// address and paid before registering sent ZEC the scanner could not place:
/// it fell through to the memo rules, had no memo, and stopped the vault's
/// scan until an operator attributed it by hand — which is what happened on
/// testnet. Remembering every address given out makes that payment a debt to
/// its account instead, credited when the account registers.
static ISSUED: std::sync::OnceLock<(AddressRegister, std::path::PathBuf)> = std::sync::OnceLock::new();

/// Zatoshi the vault holds for other people. Kept current by the anchoring
/// loop, read by the anchor builder, which refuses a fee that would dip into it.
static BACKING_FLOOR: std::sync::OnceLock<std::sync::Arc<std::sync::atomic::AtomicU64>> = std::sync::OnceLock::new();

/// The height the deposit bridge has judged up to, and the vault's note
/// stores. Anything the vault received above that height is a deposit nobody
/// has credited or refused yet, and until then it is not the operator's to
/// spend: an anchor that paid its fee from one left the vault short of the
/// deposit it was about to credit (23 Sep, testnet — the bridge was hours
/// behind, the fee reserve was empty, and two anchors spent from 0.003 ZEC
/// that arrived meanwhile). `u64::MAX` until a bridge starts: with no bridge
/// there are no deposits to protect.
static BRIDGE_JUDGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
static VAULT_NOTES: std::sync::OnceLock<zyn_custody::shielded::PoolStores> = std::sync::OnceLock::new();

/// Zatoshi in vault notes that arrived above `judged`.
fn unjudged_zat(notes: &zyn_custody::shielded::PoolStores, judged: u64) -> u64 {
    let mut total: u64 = 0;
    for pool in [orchard::ValuePool::Orchard, orchard::ValuePool::Ironwood] {
        if let Ok(store) = notes.of(pool).lock() {
            for h in store.held() {
                if h.height > judged {
                    total = total.saturating_add(h.value());
                }
            }
        }
    }
    total
}

/// Deposits the vault holds for accounts that have not registered yet.
static PENDING_ZAT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Exit destinations holders revealed over RPC, waiting for the settler to
/// pick them up, and the file they are also written to.
static NEW_REVEALS: Mutex<Vec<zynzapd::object_exit::Reveal>> = Mutex::new(Vec::new());
static REVEALS_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

fn issued_path(dir: &str, chain: u32) -> std::path::PathBuf {
    std::path::Path::new(dir).join(format!("chain-{chain}.deposit-accounts"))
}

/// Load the accounts that have been given addresses into the scanner's map.
fn load_issued(register: &AddressRegister, path: &std::path::Path) -> usize {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut map = register.lock().unwrap();
    let mut n = 0;
    for line in text.lines() {
        if let Some(account) = unhex(line.trim()).and_then(|b| <[u8; 32]>::try_from(b).ok()) {
            map.insert(*zyn_custody::shielded::deposit_index(&account).as_bytes(), account);
            n += 1;
        }
    }
    n
}

/// Remember that `account` was given an address, once.
fn remember_issued(account: &zyn_vm::spec::AccountId) {
    let Some((register, path)) = ISSUED.get() else {
        return;
    };
    let index = *zyn_custody::shielded::deposit_index(account).as_bytes();
    let mut map = register.lock().unwrap();
    if map.get(&index) == Some(account) {
        return;
    }
    map.insert(index, *account);
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", hex(account));
    }
}

/// The zatoshi the vault holds for others: issued ZEC.zy, exits owed, and
/// deposits waiting for their account to register.
fn backing_zat(vm: &ObjectVm, zec: &[u8; 32]) -> u64 {
    let issued = vm.issued.get(zec).map(|f| (f.0.max(0) / 10_000_000_000) as u64).unwrap_or(0);
    let unjudged = VAULT_NOTES
        .get()
        .map(|n| unjudged_zat(n, BRIDGE_JUDGED.load(std::sync::atomic::Ordering::SeqCst)))
        .unwrap_or(0);
    issued
        .saturating_add(zynzapd::object_exit::owed(vm, zec))
        .saturating_add(PENDING_ZAT.load(std::sync::atomic::Ordering::SeqCst))
        .saturating_add(unjudged)
}

fn consensus_network(n: zcash_protocol::consensus::NetworkType) -> zcash_protocol::consensus::Network {
    match n {
        zcash_protocol::consensus::NetworkType::Main => zcash_protocol::consensus::Network::MainNetwork,
        _ => zcash_protocol::consensus::Network::TestNetwork,
    }
}

/// `valid_until_epoch` from a signed frame: `ZYNAUTH1 ‖ kind ‖ chain u32 ‖
/// vm_id [32] ‖ valid_until u64 ‖ …`, big-endian. Only called after
/// `decode_authorized` has accepted the frame, so the layout is known good.
fn frame_expiry(frame: &[u8]) -> Option<u64> {
    let at = 8 + 1 + 4 + 32;
    Some(u64::from_be_bytes(frame.get(at..at + 8)?.try_into().ok()?))
}

fn frame_key(frame: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"objectvm.frame.v1");
    h.update(frame);
    h.finalize().into()
}

/// The operator's credential for `zyn_submit`, from `ZYN_SUBMIT_TOKEN`.
///
/// `zyn_submit` wraps whatever it is given as `Authorized::operator` and
/// applies it. Nothing checks a signature, because nothing *can*: the VM trusts
/// an operator intent by definition. Reachable from the public gateway, that
/// made every caller the operator — able to mint unbacked ZEC, credit a deposit
/// that never happened, mark an exit paid, or spend any note whose owner they
/// could guess. So it is closed unless the caller presents this token, and it
/// is closed when no token is configured at all: a console with no lock should
/// not open because somebody forgot to set one.
///
/// Holders' own intents need a different door — one that checks *their*
/// signature — and are refused here until that door exists.
static SUBMIT_TOKEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

fn submit_token() -> Option<&'static str> {
    SUBMIT_TOKEN
        .get_or_init(|| {
            std::env::var("ZYN_SUBMIT_TOKEN")
                .ok()
                .map(|t| t.trim().to_string())
                .filter(|t| t.len() >= 32)
        })
        .as_deref()
}

/// Equal-time comparison, so the check does not leak the token a byte at a
/// time to someone timing refusals.
fn token_matches(given: &str, want: &str) -> bool {
    let (a, b) = (given.as_bytes(), want.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn handle(
    body: &str,
    node: &Arc<Mutex<Node<ObjectVm>>>,
    journal: &Arc<Mutex<Journal>>,
    anchoring: bool,
    pub_log: &Arc<Mutex<Published>>,
    dir: &str,
    chain: u32,
    da_dir: Option<&std::path::Path>,
    sync: &Arc<Mutex<SyncProgress>>,
    vault: Option<&(
        zyn_custody::shielded::VaultKeys,
        zcash_protocol::consensus::NetworkType,
    )>,
) -> String {
    let method = field(body, "method").unwrap_or_default();
    match method.as_str() {
        "zyn_status" => {
            let n = node.lock().unwrap();
            let anchors = recent_anchors(da_dir, chain, 1);
            let sync = sync.lock().map(|g| SyncProgress {
                synced_to: g.synced_to,
                target: g.target,
                error: g.error.clone(),
            }).unwrap_or_default();
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}}",
                status_json(n.state(), anchoring, anchors.last(), &sync)
            )
        }
        // The epoch-to-Zcash mapping. This is the one thing worth checking
        // from away from a desk: whether roots are still landing.
        "zyn_anchors" => {
            let rows: Vec<String> = recent_anchors(da_dir, chain, 10)
                .iter()
                .map(|a| {
                    format!(
                        "{{\"epoch\":{},\"height\":{},\"txid\":\"{}\",\"root\":\"{}\"}}",
                        a.epoch,
                        a.height,
                        escape(&a.txid),
                        hex(&a.root)
                    )
                })
                .collect();
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":[{}]}}",
                rows.join(",")
            )
        }
        // Resting listings. A listing is already public — it is an offer to
        // the world — so serving it discloses nothing the marketplace does
        // not. The seller is not named: that stays with the operator.
        "zyn_offers" => {
            let n = node.lock().unwrap();
            let vm = n.state();
            let rows: Vec<String> = vm
                .offers
                .iter()
                .take(1000)
                .map(|(item, o)| {
                    let asset = vm.live.get(item).map(|x| x.asset).unwrap_or([0u8; 32]);
                    let info = vm.market.items.get(&asset);
                    let fee = vm
                        .market
                        .terms_for(&asset)
                        .and_then(|(_, t)| objectvm::market::fee_of(o.price_amount, t.fee_bps))
                        .unwrap_or(zyn_vm::Fixed::ZERO);
                    let listing = vm.market.listings.get(item);
                    format!(
                        "{{\"item\":\"{}\",\"asset\":\"{}\",\"collection\":\"{}\",\"serial\":{},\"price_asset\":\"{}\",\"price\":\"{}\",\"price_raw\":\"{}\",\"fee_raw\":\"{}\",\"pay_raw\":\"{}\",\"nullifier\":\"{}\",\"expires_at_epoch\":{}}}",
                        hex(item),
                        hex(&asset),
                        info.map(|i| hex(&i.collection)).unwrap_or_default(),
                        info.map(|i| i.serial as i64).unwrap_or(-1),
                        hex(&o.price_asset),
                        o.price_amount,
                        o.price_amount.0,
                        fee.0,
                        o.price_amount.0 + fee.0,
                        listing.map(|l| hex(&l.nullifier)).unwrap_or_default(),
                        o.expires_at_epoch
                    )
                })
                .collect();
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"total\":{},\"offers\":[{}]}}}}",
                vm.offers.len(),
                rows.join(",")
            )
        }
        // Which collection and serial an item asset is. Public: an item's
        // identity is in the collection's manifest already.
        "zyn_items" => {
            let Some(list) = field(body, "assets") else {
                return err("no assets");
            };
            let n = node.lock().unwrap();
            let vm = n.state();
            let mut rows = Vec::new();
            for a in list.split(',').filter(|s| !s.is_empty()).take(512) {
                let Some(asset) = unhex(a).and_then(|b| <[u8; 32]>::try_from(b).ok()) else {
                    return err("an asset is not 32 hex bytes");
                };
                if let Some(i) = vm.market.items.get(&asset) {
                    rows.push(format!(
                        "{{\"asset\":\"{}\",\"collection\":\"{}\",\"serial\":{}}}",
                        hex(&asset),
                        hex(&i.collection),
                        i.serial
                    ));
                }
            }
            format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":[{}]}}", rows.join(","))
        }
        // The primary sale's delivery, on the operator's token: mint one item
        // to a buyer, sealed to the viewing key they registered.
        "zyn_mintItem" => {
            match submit_token() {
                Some(want) if token_matches(&field(body, "token").unwrap_or_default(), want) => {}
                _ => return err("zyn_mintItem is the operator's and needs the operator's token"),
            }
            let (Some(collection), Some(owner)) = (
                field(body, "collection").and_then(|h| unhex(&h)).and_then(|b| <[u8; 32]>::try_from(b).ok()),
                field(body, "owner").and_then(|h| unhex(&h)).and_then(|b| <[u8; 32]>::try_from(b).ok()),
            ) else {
                return err("collection and owner are 32 hex bytes");
            };
            let Some(serial) = field(body, "serial").and_then(|s| s.parse::<u32>().ok()) else {
                return err("serial is a number, as a string");
            };
            let op = {
                use rand::RngCore;
                let (mut nonce, mut eph) = ([0u8; 32], [0u8; 32]);
                rand::rngs::OsRng.fill_bytes(&mut nonce);
                rand::rngs::OsRng.fill_bytes(&mut eph);
                let n = node.lock().unwrap();
                match objectvm::primary::mint_item(
                    n.state(),
                    collection,
                    serial,
                    owner,
                    nonce,
                    zyn_vm::note_enc::StaticSecret::from(eph),
                ) {
                    Ok(op) => op,
                    Err(e) => return err(e),
                }
            };
            let receipts = apply_operator(&op, node, journal, pub_log, dir, chain);
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"receipts\":\"{}\"}}}}",
                escape(&format!("{receipts:?}"))
            )
        }
        // The market's public face: every pool's reserves and every sale's
        // terms and raise. What a market cannot work without, and nothing
        // about who is in it (`objectvm::market`).
        "zyn_market" => {
            let n = node.lock().unwrap();
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}}",
                market_json(n.state(), n.policy().max_seconds_per_epoch)
            )
        }
        // Where to pay an account: its registered viewing key. Public by
        // design — it is in the state root already, and a payer who cannot
        // read it cannot address a note to anyone.
        "zyn_viewingKey" => {
            let Some(account) = field(body, "account")
                .and_then(|h| unhex(&h))
                .and_then(|b| <[u8; 32]>::try_from(b).ok())
            else {
                return err("account is not 32 hex bytes");
            };
            let n = node.lock().unwrap();
            match n.state().viewing.get(&account) {
                Some(v) => format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"viewing\":\"{}\"}}}}",
                    hex(v)
                ),
                None => err("that account has not registered where to be paid, so nothing can be sent to it yet"),
            }
        }
        // What the ledger owes, by claim id. Ids derive from a nonce only
        // the owner holds, so this answers the owner and nobody else — there
        // is deliberately no "claims for this account" to ask.
        "zyn_claims" => {
            let Some(list) = field(body, "ids") else {
                return err("no ids");
            };
            let n = node.lock().unwrap();
            let mut rows = Vec::new();
            for id in list.split(',').filter(|s| !s.is_empty()).take(256) {
                let Some(key) = unhex(id).and_then(|b| <[u8; 32]>::try_from(b).ok()) else {
                    return err("an id is not 32 hex bytes");
                };
                if let Some(c) = n.state().market.claims.get(&key) {
                    rows.push(format!(
                        "{{\"id\":\"{}\",\"asset\":\"{}\",\"amount\":\"{}\"}}",
                        hex(&key),
                        hex(&c.asset),
                        c.amount.0
                    ));
                }
            }
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":[{}]}}",
                rows.join(",")
            )
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
            match submit_token() {
                None => {
                    return err(
                        "zyn_submit is closed: it applies intents as the operator's, and this \
                         node has no ZYN_SUBMIT_TOKEN configured",
                    )
                }
                Some(want) => {
                    if !token_matches(&field(body, "token").unwrap_or_default(), want) {
                        return err(
                            "zyn_submit applies intents as the operator's and needs the \
                             operator's token",
                        );
                    }
                }
            }
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
            let receipts = format!(
                "{:?}",
                apply_operator(&op, node, journal, pub_log, dir, chain)
            );
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"receipts\":{:?}}}}}",
                receipts
            )
        }
        // Where to send Zcash so it becomes an object for this account.
        //
        // One address per account, so **no memo is needed**: the address is
        // the attribution. That matters because the things people actually
        // pay from -- faucets, exchange withdrawals, most wallets -- cannot
        // set one, and a deposit with no memo is not credited at all.
        //
        // Open rather than signature-gated. The address is derived from the
        // account id by a pure function, so anyone holding the id can already
        // compute it given the vault's public key; gating would protect
        // nothing and would stop a wallet showing a holder where to deposit.
        "zyn_depositAddress" => {
            let Some(keys) = vault.as_ref() else {
                return err("this node custodies no vault, so it has no deposit address");
            };
            let Some(hexed) = field(body, "account") else {
                return err("no account");
            };
            let Some(account) = unhex(&hexed).and_then(|b| <[u8; 32]>::try_from(b).ok()) else {
                return err("account is not 32 hex bytes");
            };
            // Said plainly, because a deposit to an unregistered account is
            // held rather than credited and a holder should know that before
            // they send rather than after.
            let registered = {
                let n = node.lock().unwrap();
                n.state().viewing.contains_key(&account)
            };
            remember_issued(&account);
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"address\":\"{}\",\"registered\":{},\"note\":\"{}\"}}}}",
                keys.0.deposit_address(&account, keys.1),
                registered,
                if registered {
                    "send Zcash here and it becomes an object for this account; no memo needed"
                } else {
                    "this account has not registered a viewing key, so a deposit here is held as owed and credited the moment it does"
                }
            )
        }
        // Which Zyn account a Zcash address belongs to, when it is one of the
        // deposit addresses this vault handed out. What lets a wallet pay a
        // Zyn account by the one address its owner shares for everything:
        // ZEC sent there from Zcash is a deposit, and ZEC.zy or an item sent
        // to it from Zyn is a private transfer to the same account.
        "zyn_resolveAddress" => {
            let Some(keys) = vault.as_ref() else {
                return err("this node custodies no vault, so it hands out no addresses");
            };
            let Some(address) = field(body, "address") else {
                return err("no address");
            };
            let account = match zynzapd::settle::parse_destination(address.trim(), consensus_network(keys.1)) {
                Some(zynzapd::settle::ZcashDestination::Shielded(a)) => keys
                    .0
                    .index_of_address(&a)
                    .and_then(|i| ISSUED.get().and_then(|(reg, _)| reg.lock().ok().and_then(|m| m.get(&i).copied()))),
                Some(_) => None,
                None => return err("not a transparent or unified address for this network"),
            };
            match account {
                Some(a) => format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"account\":\"{}\"}}}}", hex(&a)),
                None => "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"account\":null}}".to_string(),
            }
        }
        // The commitment a withdrawal names, for an address and a salt the
        // holder chose. A pure function, served here because parsing a
        // unified address takes the same Zcash libraries the vault pays with;
        // the address is not stored and says nothing the payout will not.
        "zyn_exitCommitment" | "zyn_exitReveal" => {
            let Some(keys) = vault.as_ref() else {
                return err("this node custodies no vault, so it pays no exits");
            };
            let (Some(address), Some(salt)) = (field(body, "address"), field(body, "salt")) else {
                return err("give an address and a salt");
            };
            let Some(salt) = unhex(&salt).and_then(|b| <[u8; 32]>::try_from(b).ok()) else {
                return err("the salt is not 32 hex bytes");
            };
            let Some(dest) = zynzapd::settle::parse_destination(address.trim(), consensus_network(keys.1)) else {
                return err("not a transparent or unified address for this network");
            };
            let commitment = zynzapd::settle::zcash_commitment(&dest, &salt);
            if method == "zyn_exitCommitment" {
                return format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"destination\":\"{}\"}}}}",
                    hex(&commitment)
                );
            }
            // A reveal is accepted only for an exit already waiting on it, so
            // this cannot be used to write arbitrary lines into the file the
            // vault pays from — and a reveal is proof, not instruction: it
            // must reproduce the commitment the holder already signed.
            let waiting = {
                let n = node.lock().unwrap();
                n.state().exits.iter().any(|e| e.paid_by.is_none() && e.destination == commitment)
            };
            if !waiting {
                return err("no unpaid exit names that address and salt");
            }
            let reveal = zynzapd::object_exit::Reveal { address: dest, salt };
            if let Ok(mut v) = NEW_REVEALS.lock() {
                if !v.contains(&reveal) {
                    v.push(reveal);
                }
            }
            if let Some(path) = REVEALS_PATH.get() {
                use std::io::Write as _;
                let known = std::fs::read_to_string(path).unwrap_or_default();
                let line = format!("{} {}", address.trim(), hex(&salt));
                if !known.lines().any(|l| l.trim() == line) {
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                        let _ = writeln!(f, "{line}");
                    }
                }
            }
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"filed\":true}}".to_string()
        }
        // The holders' door. A frame is a signed envelope around one intent,
        // exactly as the journal will record it: `decode_authorized` checks
        // the envelope is for this chain, this program and not expired, that
        // every signature verifies, and that together they cover every
        // account the intent names. A replica runs the same check on the same
        // bytes, so nothing here is taken on the sequencer's word.
        "zyn_submitSigned" => {
            let Some(hexed) = field(body, "frame") else {
                return err("no frame");
            };
            let Some(frame) = unhex(&hexed) else {
                return err("frame is not hex");
            };
            let authorized = {
                let n = node.lock().unwrap();
                match zyn_vm::verify::decode_authorized::<ObjectVm>(&frame, n.state()) {
                    Ok(a) => a,
                    Err(e) => return err(&format!("refused: {e:?}")),
                }
            };
            // An operator frame decodes too, for intents nobody signs. This
            // door is for signatures; the operator's is `zyn_submit`.
            if matches!(authorized.authority(), zyn_vm::verify::Authority::Operator) {
                return err("this door takes a holder's signature, not an operator frame");
            }
            let Some(expiry) = frame_expiry(&frame) else {
                return err("frame is truncated");
            };
            let epoch_now = node.lock().unwrap().state().epoch;
            if let Some(r) = REPLAY.get() {
                let mut r = r.lock().unwrap();
                r.prune(epoch_now);
                if !r.fresh(frame_key(&frame), expiry) {
                    return err("already applied: this exact authorization has been used");
                }
                let _ = r.save(std::path::Path::new(dir), chain);
            }
            let receipts = apply_authorized(authorized, node, journal, pub_log, dir, chain);
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"receipts\":\"{}\"}}}}",
                escape(&format!("{receipts:?}"))
            )
        }
        // Name what this chain serves rather than saying "unknown method".
        // A wallet built for the AMM chains reaches here the moment it asks
        // for `zyn_assets` or `zyn_account`, and a bare refusal reads as an
        // outage; this reads as what it is — a different ledger with a
        // different method table.
        other => err(&unknown_method(other)),
    }
}

/// Why a method this ledger does not serve was refused.
///
/// A wallet built for the AMM chains reaches here the moment it asks for
/// `zyn_assets` or `zyn_account`, and shows the holder whatever this says. A
/// bare "unknown method" reads as an outage and sends someone looking for a
/// fault that does not exist; naming the ledger and its method table says what
/// actually happened — they reached a different chain.
fn unknown_method(method: &str) -> String {
    let named = if method.is_empty() {
        "that method".to_string()
    } else {
        escape(method)
    };
    format!(
        "this chain is an object ledger and does not serve {named}. It serves \
         zyn_status, zyn_published, zyn_submitSigned, zyn_anchors, zyn_offers, zyn_market, zyn_claims, zyn_depositAddress, zyn_resolveAddress, zyn_viewingKey, zyn_items, zyn_exitCommitment and zyn_exitReveal"
    )
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

/// The deposit bridge's own state: where it has looked, and what it owes.
///
/// Opt-in and all-or-nothing, like anchoring. Without `ZYN_BRIDGE_SEED` there
/// is no bridge and the banner says objects are unbacked — which was true and
/// said so for as long as the bridge did not exist. Turning it on with a seed
/// that is not the one previous deposits used would rebuild *different*
/// objects for them, so the seed is as much a part of the chain's identity as
/// its id (see `object_ingress`).
struct Bridge {
    seed: Vec<u8>,
    zec: zyn_custody::watcher::AssetId,
    addresses: std::sync::Arc<
        Mutex<std::collections::BTreeMap<[u8; 11], zyn_vm::spec::AccountId>>,
    >,
    watcher: zyn_custody::watcher::Watcher,
    pending: zynzapd::object_ingress::pending::Pending,
}

fn bridge_cursor_path(dir: &str, chain: u32) -> std::path::PathBuf {
    std::path::Path::new(dir).join(format!("chain-{chain}.bridge"))
}

fn bridge_pending_path(dir: &str, chain: u32) -> std::path::PathBuf {
    std::path::Path::new(dir).join(format!("chain-{chain}.pending"))
}

/// Bridge passes run back to back before the tip is read again.
const BRIDGE_BURST: usize = 400;

/// Where the deposit bridge starts looking.
///
/// Three inputs, and the rule is that the highest wins:
///
/// - **the vault's birth** (`from_height`): nothing below it can be a deposit
///   to this vault. Starting at zero instead means millions of empty steps.
/// - **the floor** (`ZYN_BRIDGE_FROM_HEIGHT`): where the chain this one
///   *replaced* stopped scanning. A vault outlives the ledgers that custody it
///   — chain 11 handed this vault to chain 212, and the AMM mainnet will hand
///   its vault to the object ledger — and every deposit below the handover was
///   the old ledger's to credit. Scanning them again issues them twice: the
///   old chain's holders are owed on both, and each ledger's solvency check
///   passes on its own because each counts only what it issued.
/// - **a saved cursor**: where this bridge got to last time. Above the floor it
///   is simply progress; below it, it predates the floor being set and is
///   raised, because the floor is a statement about ownership and a cursor is
///   only a statement about work.
///
/// The value is the last height already handled, so the first height looked
/// at is one above it.
fn bridge_start(saved: Option<u64>, floor: Option<u64>, from_height: u64) -> u64 {
    let birth = from_height.saturating_sub(1);
    let floor = floor.map(|f| f.saturating_sub(1)).unwrap_or(0);
    saved.unwrap_or(birth).max(birth).max(floor)
}

/// Wire the bridge, if this deployment has a seed to derive from.
///
/// `next_index` comes from the ledger rather than from the cursor file: the
/// VM is the authority on what it has already credited, and a cursor that
/// disagreed with it would be the one thing that could double-credit.
fn build_bridge(
    dir: &str,
    chain: u32,
    confirmations: u64,
    vm: &ObjectVm,
    from_height: u64,
    addresses: std::sync::Arc<
        Mutex<std::collections::BTreeMap<[u8; 11], zyn_vm::spec::AccountId>>,
    >,
) -> Option<Bridge> {
    let seed_hex = std::env::var("ZYN_BRIDGE_SEED").ok().filter(|s| !s.is_empty())?;
    let Some(seed) = unhex(&seed_hex) else {
        eprintln!("zyn-object-node: ZYN_BRIDGE_SEED is not hex — the bridge stays off");
        return None;
    };
    if seed.len() < 32 {
        eprintln!(
            "zyn-object-node: ZYN_BRIDGE_SEED is {} bytes; 32 or more, or the derived \
             nonces are guessable — the bridge stays off",
            seed.len()
        );
        return None;
    }
    // The same asset id the AMM chains use for bridged ZEC, so one wallet
    // recognises it on either ledger. Overridable, because a chain that
    // custodies a different asset should say so rather than inherit a name.
    let zec = match std::env::var("ZYN_ZEC_ASSET").ok().filter(|s| !s.is_empty()) {
        None => swapvm::types::XZEC,
        Some(h) => match unhex(&h).and_then(|b| <[u8; 32]>::try_from(b).ok()) {
            Some(a) => a,
            None => {
                eprintln!("zyn-object-node: ZYN_ZEC_ASSET is not 32 hex bytes — the bridge stays off");
                return None;
            }
        },
    };

    let saved = std::fs::read_to_string(bridge_cursor_path(dir, chain))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok());
    let floor = std::env::var("ZYN_BRIDGE_FROM_HEIGHT")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok());
    let scanned_to = bridge_start(saved, floor, from_height);
    if let Some(f) = floor {
        eprintln!(
            "zyn-object-node: deposits below height {f} belong to the chain this one replaced \
             and are not credited here"
        );
    }
    let pending = std::fs::read(bridge_pending_path(dir, chain))
        .ok()
        .and_then(|b| zynzapd::object_ingress::pending::Pending::decode(&b))
        .unwrap_or_default();

    // A debt below the cursor is one the watcher will never report again, so
    // the cursor must not run ahead of what is still owed.
    let from = match pending.earliest_height() {
        Some(h) => scanned_to.min(h.saturating_sub(1)),
        None => scanned_to,
    };
    if from != scanned_to {
        eprintln!(
            "zyn-object-node: bridge cursor {scanned_to} is ahead of an unpaid debt at \
             {}; rewinding to {from}",
            pending.earliest_height().unwrap_or(0)
        );
    }

    // From here the anchor keeps its hands off anything the bridge has not
    // reached yet (see `BRIDGE_JUDGED`).
    BRIDGE_JUDGED.store(from.saturating_sub(1), std::sync::atomic::Ordering::SeqCst);
    eprintln!(
        "zyn-object-node: deposit bridge ON — scanning from {from}, {} debt(s) outstanding, \
         asset {}",
        pending.len(),
        hex(&zec)
    );
    // Every account already registered gets its deposit address recognised
    // before the first pass, not after it.
    let known = refresh_deposit_addresses(&addresses, vm);
    eprintln!("zyn-object-node: {known} deposit address(es) recognised");
    Some(Bridge {
        seed,
        zec,
        addresses,
        watcher: zyn_custody::watcher::Watcher::resume(
            confirmations,
            vm.deposits.len() as u64,
            from,
        ),
        pending,
    })
}

/// Teach the scanner every registered account's deposit address.
///
/// Derived from the ledger rather than remembered: `deposit_index` is a pure
/// function of the account id, so an account that has registered a viewing key
/// already has a deposit address whether anyone asked for it or not. Rebuilt
/// each pass because accounts register while the node runs, and an address the
/// scanner does not know is a deposit that falls through to the memo rules.
fn refresh_deposit_addresses(
    register: &Mutex<std::collections::BTreeMap<[u8; 11], zyn_vm::spec::AccountId>>,
    vm: &ObjectVm,
) -> usize {
    let Ok(mut map) = register.lock() else {
        return 0;
    };
    for account in vm.viewing.keys() {
        let index = *zyn_custody::shielded::deposit_index(account).as_bytes();
        map.insert(index, *account);
    }
    map.len()
}

/// What the vault holds, unspent, across every pool it uses.
///
/// Summed from the note stores rather than from a scan range: these are the
/// notes the vault still has, so anchors that spent from it are already
/// accounted for. A range sum is not a balance (see `object_bridge`).
fn vault_balance(notes: &zyn_custody::shielded::PoolStores) -> Result<zyn_vm::Fixed, String> {
    let mut held = Vec::new();
    for pool in [
        orchard::ValuePool::Orchard,
        orchard::ValuePool::Ironwood,
    ] {
        held.push(
            notes
                .of(pool)
                .lock()
                .map_err(|_| "note store lock poisoned".to_string())?
                .balance(),
        );
    }
    zatoshi_as_fixed(&held)
}

/// Zatoshi held, summed, in the ledger's own scale — the one every
/// attestation and every issued amount is in (`Fixed`, 1e18 per ZEC, so 1e10
/// per zatoshi). Comparing a raw zatoshi count against those read the vault
/// as ten billion times poorer than it was, and refused the first real
/// deposit as an insolvency (23 Sep, testnet).
fn zatoshi_as_fixed(zatoshi: &[u64]) -> Result<zyn_vm::Fixed, String> {
    const PER_ZAT: i128 = 10_000_000_000;
    let mut total: i128 = 0;
    for z in zatoshi {
        total = total
            .checked_add(*z as i128)
            .ok_or_else(|| "vault balance overflowed".to_string())?;
    }
    total
        .checked_mul(PER_ZAT)
        .map(zyn_vm::Fixed)
        .ok_or_else(|| "vault balance overflowed".to_string())
}

/// One pass of the deposit bridge: look at Zcash, credit what is backed.
///
/// Returns the number of deposits credited. An insolvent vault returns an
/// error and advances nothing — the cursor stays where it was, so the next
/// pass looks at the same range rather than stepping over the evidence.
#[allow(clippy::too_many_arguments)]
fn bridge_pass(
    bridge: &mut Bridge,
    scanner: &zyn_custody::shielded::Scanner,
    notes: &zyn_custody::shielded::PoolStores,
    zebra: &zyn_custody::zebra::Zebra,
    node: &Arc<Mutex<Node<ObjectVm>>>,
    journal: &Arc<Mutex<Journal>>,
    pub_log: &Arc<Mutex<Published>>,
    dir: &str,
    chain: u32,
) -> Result<usize, String> {
    // The height this pass starts at. Debts are recorded against it rather
    // than against where the pass ended: a rewind has to re-cover the whole
    // range a debt could have been in, and the watcher's actions do not carry
    // per-deposit heights.
    let from = bridge.watcher.scanned_to().saturating_add(1);
    let observed = scanner
        .observe(zebra, from)
        .map_err(|e| format!("scan failed: {e:?}"))?;
    // `poll` moves the watcher on — its height, the txids it has reported,
    // its credit index — before anything here decides whether to credit. A
    // pass refused below must put all of that back, or the deposit it refused
    // is marked seen and never offered again: a refusal meant to wait became
    // one that skipped (23 Sep, testnet, 0.003 TAZ).
    let before = bridge.watcher.clone();
    let mut actions = bridge
        .watcher
        .poll(&observed)
        .map_err(|e| format!("watcher refused the range: {e:?}"))?;

    // What the vault actually holds, unspent, across both pools. Not the
    // watcher's attestation: that sums one scan range (see `object_bridge`).
    // This figure accounts for the anchors that spent from the vault, which
    // is the whole reason a range sum cannot stand in for it.
    let vault = match vault_balance(notes) {
        Ok(v) => v,
        Err(e) => {
            bridge.watcher = before;
            return Err(e);
        }
    };

    // Retries go down the same road as live credits, deliberately: a retry
    // with its own path would be a path nothing tests.
    let decided = {
        let Ok(n) = node.lock() else {
            bridge.watcher = before;
            return Err("node lock poisoned".to_string());
        };
        let vm = n.state();
        refresh_deposit_addresses(&bridge.addresses, vm);
        actions.extend(bridge.pending.replayable(vm));
        if actions.is_empty() {
            // Nothing to credit, but the watcher still moved. Writing the
            // cursor down here is what stops a restart from rescanning every
            // empty block since the last deposit — which on a quiet chain is
            // all of them.
            let _ = std::fs::write(
                bridge_cursor_path(dir, chain),
                bridge.watcher.scanned_to().to_string(),
            );
            return Ok(0);
        }
        zynzapd::object_bridge::decide(vm, &bridge.seed, bridge.zec, &actions, vault)
    };

    let items = match decided {
        zynzapd::object_bridge::Pass::Insolvent {
            asset,
            observed,
            issued,
        } => {
            // No intent repairs this, and crediting on would deepen a hole
            // rather than fill it. The watcher goes back to where this pass
            // began, so the same range is judged again once the vault covers
            // it. Both figures in the message, always: which side is short
            // is the whole diagnosis.
            bridge.watcher = before;
            return Err(format!(
                "INSOLVENT: the check saw {observed:?} of asset {} against a vault holding \
                 {vault:?} and {issued:?} issued. Nothing was credited and the cursor did not move",
                hex(&asset.unwrap_or(bridge.zec))
            ));
        }
        zynzapd::object_bridge::Pass::Unverifiable { asset } => {
            bridge.watcher = before;
            return Err(format!(
                "a deposit arrived in asset {} whose backing this bridge cannot check. \
                 Nothing was credited",
                hex(&asset)
            ));
        }
        zynzapd::object_bridge::Pass::Apply(items) => items,
    };

    let mut credited = 0usize;
    for item in items {
        match item {
            zynzapd::object_bridge::Decided::Credit { op, txid } => {
                let receipts = apply_operator(&op, node, journal, pub_log, dir, chain);
                if receipts.iter().any(ObjectVm::rejected) {
                    eprintln!(
                        "zyn-object-node: the ledger refused the credit for {}: {receipts:?}",
                        hex(&txid)
                    );
                    continue;
                }
                // Settled against a receipt, never against having built the
                // intent: dropping a debt in that window is the loss this
                // whole list exists to prevent.
                bridge.pending.settle(&txid);
                credited += 1;
                eprintln!("zyn-object-node: credited deposit {}", hex(&txid));
            }
            zynzapd::object_bridge::Decided::Owed {
                account,
                amount,
                txid,
            } => {
                bridge.pending.owe(account, amount, txid, from);
                eprintln!(
                    "zyn-object-node: deposit {} is owed to an account that has not \
                     registered a viewing key — the ZEC stays in the vault and nothing \
                     expires it",
                    hex(&txid)
                );
            }
            zynzapd::object_bridge::Decided::AlreadyCredited { txid } => {
                // On the ledger already, so the debt (if any) is discharged.
                bridge.pending.settle(&txid);
            }
            zynzapd::object_bridge::Decided::Rejected { txid, why } => {
                eprintln!(
                    "zyn-object-node: refused deposit {}: {why}",
                    hex(&txid)
                );
            }
        }
    }

    // Persist the debts before the cursor. If only one of the two survives a
    // crash it must be the list of what is owed: a cursor that is behind
    // costs a rescan, and the VM refuses the duplicate credit anyway.
    let _ = std::fs::write(bridge_pending_path(dir, chain), bridge.pending.encode());
    PENDING_ZAT.store(
        bridge
            .pending
            .debts()
            .iter()
            .map(|d| (d.amount.0.max(0) / 10_000_000_000) as u64)
            .sum(),
        std::sync::atomic::Ordering::SeqCst,
    );
    let _ = std::fs::write(
        bridge_cursor_path(dir, chain),
        bridge.watcher.scanned_to().to_string(),
    );
    Ok(credited)
}

/// Apply one operator-authorised intent: journal it, apply it, publish it.
///
/// Every intent the node accepts goes through here, whichever door it came in
/// by — the RPC, or the deposit bridge. The bridge could have called
/// `submit_operator` directly and written its own journal record, and it would
/// have worked until the two copies drifted; the journal accounting here is
/// subtle enough (which epoch a record belongs to, the seal that must follow
/// it, the flush that must not be skipped) that a second implementation is a
/// second set of those bugs.
fn apply_operator(
    op: &Op,
    node: &Arc<Mutex<Node<ObjectVm>>>,
    journal: &Arc<Mutex<Journal>>,
    pub_log: &Arc<Mutex<Published>>,
    dir: &str,
    chain: u32,
) -> Vec<objectvm::Receipt> {
    apply_authorized(
        zyn_vm::verify::Authorized::operator(op.clone()),
        node,
        journal,
        pub_log,
        dir,
        chain,
    )
}

/// [`apply_operator`] for an intent whose authority has already been
/// established — the operator's, or a holder's verified signature.
///
/// The journal records the envelope exactly as it was authorized. That is what
/// lets a replica re-check a holder's signature rather than take the
/// sequencer's word for it: an operator envelope around a holder's intent is
/// one `decode_authorized` refuses outright.
fn apply_authorized(
    authorized: zyn_vm::verify::Authorized<Op>,
    node: &Arc<Mutex<Node<ObjectVm>>>,
    journal: &Arc<Mutex<Journal>>,
    pub_log: &Arc<Mutex<Published>>,
    dir: &str,
    chain: u32,
) -> Vec<objectvm::Receipt> {
        let op = authorized.intent().clone();
        let encoded = zyn_vm::verify::encode_authorized::<ObjectVm>(&authorized);
        let mut n = node.lock().unwrap();
        let step = n.submit(authorized, now_secs());
        
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
    step.receipts
}

fn serve(mut s: TcpStream, node: Arc<Mutex<Node<ObjectVm>>>, journal: Arc<Mutex<Journal>>, pub_log: Arc<Mutex<Published>>, dir: String, chain: u32, anchoring: bool, da_dir: Option<std::path::PathBuf>, sync: Arc<Mutex<SyncProgress>>, vault: Option<(zyn_custody::shielded::VaultKeys, zcash_protocol::consensus::NetworkType)>) {
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
                let out = with_request_id(
                    &body,
                    handle(
                        &body, &node, &journal, anchoring, &pub_log, &dir, chain,
                        da_dir.as_deref(), &sync, vault.as_ref(),
                    ),
                );
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

/// Answer with the id the request asked under.
///
/// JSON-RPC says a response carries its request's id, and the wallet checks
/// it — an answer to some other question is refused, which is the point. This
/// node answered every request as id 1, so the browser wallet refused every
/// answer an object ledger gave it. Only a number or a plain token string is
/// echoed; anything else keeps 1 rather than splicing caller bytes into JSON.
fn with_request_id(body: &str, out: String) -> String {
    let Some(id) = request_id(body) else {
        return out;
    };
    out.replacen("\"id\":1,", &format!("\"id\":{id},"), 1)
}

fn request_id(body: &str) -> Option<String> {
    let at = body.find("\"id\"")? + 4;
    let rest = body[at..].trim_start().strip_prefix(':')?.trim_start();
    if let Some(q) = rest.strip_prefix('"') {
        let end = q.find('"')?;
        let v = &q[..end];
        let plain = !v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        return plain.then(|| format!("\"{v}\""));
    }
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    (!digits.is_empty() && digits.len() <= 15).then_some(digits)
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

/// What the note-tree walker has learned, for anyone who asks.
///
/// The walker prints its progress and the settler prints its anchors, which
/// means the one question worth asking from a phone — "is the chain still
/// reaching Zcash, and if not how far behind is it" — could only be answered
/// by reading journald over ssh. The sync thread owns the writes; the RPC
/// reads. Anchors are not duplicated here: they are already on disk in the
/// published index, and a second copy is a second thing that can disagree.
#[derive(Default)]
struct SyncProgress {
    synced_to: u64,
    target: u64,
    /// Why the walk stopped, empty while it is walking. This is the field
    /// that turns "no anchors yet" from a mystery into a cause.
    error: String,
}

/// The asset id this chain custodies as bridged ZEC.
///
/// One definition, read by both the deposit bridge and the exit settler: if
/// they disagreed, money would enter as one asset and be owed as another.
fn zec_asset() -> [u8; 32] {
    match std::env::var("ZYN_ZEC_ASSET").ok().filter(|s| !s.is_empty()) {
        None => swapvm::types::XZEC,
        Some(h) => unhex(&h)
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .unwrap_or(swapvm::types::XZEC),
    }
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
        public.clone(),
        Some(custodians.clone()),
    )
    .ok()?
    .with_backing_floor(BACKING_FLOOR.get_or_init(Default::default).clone());
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
    let keys = zyn_custody::shielded::VaultKeys::from_full_viewing_key(fvk.clone());
    let addresses: std::sync::Arc<
        Mutex<std::collections::BTreeMap<[u8; 11], zyn_vm::spec::AccountId>>,
    > = std::sync::Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let given = load_issued(&addresses, &issued_path(dir, chain));
    eprintln!("zyn-object-node: {given} deposit address(es) handed out before, recognised whether or not their account has registered");
    let _ = ISSUED.set((addresses.clone(), issued_path(dir, chain)));
    // How many blocks one scan step covers before the trees are written down.
    //
    // Smaller is not slower: the work per block is the same, and the batch
    // size only decides how much is lost when the process stops. A 200-block
    // step on this hardware can run for the better part of an hour, and a
    // restart inside one throws all of it away -- which is how a catch-up can
    // run for hours and save nothing.
    let step: u64 = env("ZYN_SCAN_STEP", "25").parse().unwrap_or(25).max(1);
    let _ = VAULT_NOTES.set(notes.clone());
    let scanner = zyn_custody::shielded::Scanner::new(keys.clone(), from_height, step)
        .with_notes(notes.clone())
        .with_attributions(attributions.clone())
        // Without this the scanner has no idea an account's own deposit
        // address exists, so a payment to one falls through to the memo rules
        // and a depositor who cannot set a memo -- a faucet, most wallets --
        // has no way to be credited at all.
        .with_addresses(addresses.clone())
        // Feed the note trees only with blocks `confirmations` deep. The
        // deposit bridge's `observe` advances the trees as a side effect, and
        // without a lag it fed them to the tip -- into blocks a reorg can take
        // away, leaving a tree whose root the chain never had and whose
        // witnesses Zcash refuses. The AMM node has carried this since §32.11;
        // the object node's scanner was built without it, which showed as a
        // status reading "-12 blocks behind" the moment the bridge first ran.
        .with_tree_lag(confirmations);

    // Say where to pay this vault, at startup, in the log.
    //
    // The address is public — it is what a depositor is given — and having it
    // only in a config file meant the answer to "where do I send it" was a
    // key nobody should be reading. Index 0 is the vault's own address:
    // change, anchors and top-ups arrive here, and so does a deposit that
    // names its account in a `ZYN1:<hex>` memo.
    eprintln!(
        "zyn-object-node: vault address ({}) {}",
        std::env::var("ZYN_NETWORK").unwrap_or_else(|_| "testnet".into()),
        scanner.address(
            0,
            match network {
                Network::Mainnet => zcash_protocol::consensus::NetworkType::Main,
                _ => zcash_protocol::consensus::NetworkType::Test,
            },
        )
    );

    // Exits. Opt-in on a reveal file, because an exit cannot be paid until
    // its holder says where -- the chain carries a commitment, not an address.
    let exits = {
        let path = env("ZYN_EXIT_REVEALS", "/etc/zyn/zcash-exit-reveals");
        let _ = REVEALS_PATH.set(std::path::PathBuf::from(&path));
        let path = std::path::Path::new(&path);
        let network_c = match network {
            Network::Mainnet => zcash_protocol::consensus::Network::MainNetwork,
            _ => zcash_protocol::consensus::Network::TestNetwork,
        };
        let reveals = if path.exists() {
            match zynzapd::object_exit::load_reveals(path, network_c) {
                Ok(r) => r,
                Err(e) => {
                    // Refused rather than started with a partial list: paying
                    // some exits while silently ignoring a line the operator
                    // believes they filed is worse than paying none.
                    eprintln!("zyn-object-node: {e} — exit payments stay off");
                    return None;
                }
            }
        } else {
            Vec::new()
        };
        eprintln!(
            "zyn-object-node: exit payments ON — {} reveal(s) filed at {}",
            reveals.len(),
            path.display()
        );
        Some(zynzapd::object_exit::ExitSettler::new(
            Zebra::connect(&host, port, auth.clone(), network).ok()?,
            network_c,
            public.clone(),
            custodians.clone(),
            threshold,
            fvk.clone(),
            notes.clone(),
            reveals,
            zec_asset(),
            confirmations,
            std::path::Path::new(dir),
            chain,
        ))
    };

    Some(Anchoring {
        addresses,
        keys,
        network,
        exits,
        scanner,
        notes: notes.clone(),
        zebra: Zebra::connect(&host, port, auth, network).ok()?,
        confirmations,
        settler,
        da_dir: std::path::PathBuf::from(env("ZYN_DA_DIR", &format!("{dir}/da"))),
        da_key: da_key(),
        data_dir: std::path::PathBuf::from(dir),
    })
}

/// The key the published journals are sealed under. A key that is set but
/// unreadable stops the node: falling back to publishing in the clear would
/// hand every holder's intents to the public.
fn da_key() -> Option<[u8; 32]> {
    let v = std::env::var("ZYN_DA_KEY").ok().filter(|s| !s.trim().is_empty());
    match v {
        None => {
            eprintln!("zyn-object-node: no ZYN_DA_KEY — journals will be published UNSEALED");
            None
        }
        Some(v) => match zynzapd::dacrypt::key_from_hex(v.trim()) {
            Ok(k) => Some(k),
            Err(_) => {
                eprintln!("zyn-object-node: ZYN_DA_KEY is set but is not 64 hex characters; refusing to start");
                std::process::exit(2);
            }
        },
    }
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
    let max_io: u32 = std::env::var("ZYN_MAX_IO")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(zynzapd::object_verify::DEFAULT_MAX_IO);

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
    // The replay index outlives a restart, or a restart reopens every window
    // it was closing. A corrupt index stops the node rather than starting
    // empty: an empty one would accept every replay it had refused.
    match zyn::replay::ReplayIndex::load(std::path::Path::new(&dir), chain) {
        Ok(r) => {
            let _ = REPLAY.set(Mutex::new(r));
        }
        Err(e) => {
            eprintln!("zyn-object-node: {e}");
            std::process::exit(1);
        }
    }
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

    // Which Zcash this chain settles on, and whether it settles at all. These
    // are two facts and the banner said one: it read "TESTNET" whenever
    // anchoring was on, so a **mainnet** object ledger announced itself as a
    // testnet. A node that misnames the network it holds money on is the one
    // line of output nobody re-reads.
    eprintln!(
        "zyn-object-node: chain {} listening on {} — {} ({})",
        chain,
        listen,
        match env("ZYN_NETWORK", "testnet").as_str() {
            "mainnet" | "main" => "MAINNET",
            _ => "TESTNET",
        },
        if anchoring.is_some() {
            "anchoring"
        } else {
            "devnet, anchoring off"
        }
    );
    // Say which one it is, and never say the better one by default. A node
    // that claims to anchor and does not is worse than one that admits it.
    if anchoring.is_some() {
        eprintln!(
            "zyn-object-node: anchoring ON — roots are carried to Zcash and settle there"
        );
        // Anchoring a root and backing an object are different promises, and
        // the banner keeps saying them separately. This line was
        // unconditional for as long as no bridge existed; now it reports
        // which of the two is true rather than assuming the worse one.
        if std::env::var("ZYN_BRIDGE_SEED").ok().filter(|v| !v.is_empty()).is_some() {
            eprintln!(
                "zyn-object-node: deposit bridge configured — confirmed ZEC becomes an object, \
                 and every pass checks the vault still holds what the ledger issued"
            );
        } else {
            eprintln!(
                "zyn-object-node: objects are still unbacked: no deposit bridge credits real ZEC yet"
            );
        }
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
            // A sale closes at an epoch, so while one is open the clock has
            // to move epochs even when nobody trades.
            let sale_open = n
                .state()
                .market
                .launches
                .values()
                .any(|l| l.state == objectvm::market::LaunchState::Open);
            let sealed = n.seal_if_due(now_secs()).is_some()
                || (sale_open && n.seal_on_clock(now_secs()).is_some());
            if sealed {
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
    // Where the published index lives, kept for the RPC: the settler thread
    // takes ownership of the same path below, and a reader that has to wait
    // on the settler's lock to answer "how far along are we" is a reader that
    // blocks exactly when the settler is busy.
    let rpc_da_dir: Option<std::path::PathBuf> = anchoring.as_ref().map(|a| a.da_dir.clone());
    // Enough to derive a deposit address and nothing that can spend, kept for
    // the RPC because the scanner itself moves into the sync thread.
    let rpc_vault: Option<(
        zyn_custody::shielded::VaultKeys,
        zcash_protocol::consensus::NetworkType,
    )> = anchoring.as_ref().map(|a| {
        (
            a.keys.clone(),
            match a.network {
                zyn_custody::zebra::Network::Mainnet => {
                    zcash_protocol::consensus::NetworkType::Main
                }
                _ => zcash_protocol::consensus::NetworkType::Test,
            },
        )
    });
    let sync_progress = Arc::new(Mutex::new(SyncProgress::default()));
    let sync_writer = Arc::clone(&sync_progress);
    let anchoring = anchoring.map(|a| {
        // Split it: the scanner and its Zcash connection go to the sync
        // thread, the settler to the anchoring one. Neither type is `Clone`,
        // and neither should be — two scanners feeding one tree would walk
        // the same blocks into it twice.
        let (scanner, notes, zebra, confirmations) =
            (a.scanner, a.notes, a.zebra, a.confirmations);
        let addresses = a.addresses.clone();
        let dir2 = dir.clone();
        // The bridge shares this thread with the note walk on purpose. Both
        // want the scanner, and `observe` advances the note trees as a side
        // effect of looking for deposits — two threads holding it would walk
        // the same blocks into one tree twice.
        let bridge_node = Arc::clone(&node);
        let bridge_journal = Arc::clone(&journal);
        let bridge_pub = Arc::clone(&pub_log);
        let mut bridge = {
            let n = bridge_node.lock().unwrap();
            build_bridge(
                &dir2,
                chain,
                confirmations,
                n.state(),
                scanner.from_height(),
                addresses.clone(),
            )
        };
        if bridge.is_none() {
            eprintln!(
                "zyn-object-node: deposit bridge OFF — no ZYN_BRIDGE_SEED, so no Zcash \
                 deposit becomes an object and the ledger's objects stay unbacked"
            );
        }
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
            if let Ok(mut g) = sync_writer.lock() {
                g.target = target;
                g.synced_to = synced;
            }
            while synced < target {
                match scanner.sync_notes(&zebra, target) {
                    Ok(h) => {
                        synced = h;
                        save_note_trees(&dir2, chain, &notes);
                        if let Ok(mut g) = sync_writer.lock() {
                            g.synced_to = synced;
                            g.error.clear();
                        }
                        eprintln!("zyn-object-node: note trees synced to {synced} of {target}");
                    }
                    Err(zyn_custody::shielded::ScanRangeError::Unattributable {
                        txid,
                        height,
                        amount,
                    }) => {
                        let txid_hex: String =
                            txid.iter().map(|b| format!("{:02x}", b)).collect();
                        eprintln!(
                            "zyn-object-node: note tree catch-up stopped at {height}: {amount:?} \
                             in {txid_hex} carries no Zyn memo — add it to ZYN_ATTRIBUTIONS \
                             (the all-zero account means a vault top-up crediting nobody)"
                        );
                        if let Ok(mut g) = sync_writer.lock() {
                            g.error = format!(
                                "stopped at {height}: {txid_hex} carries no Zyn memo — add it to ZYN_ATTRIBUTIONS"
                            );
                        }
                        break;
                    }
                    Err(e) => {
                        eprintln!("zyn-object-node: note tree catch-up stopped: {e:?}");
                        if let Ok(mut g) = sync_writer.lock() {
                            g.error = format!("catch-up stopped: {e:?}");
                        }
                        break;
                    }
                }
            }
            // Deposits, once the trees are at the tip. Before that the vault's
            // own notes are still invisible, so the balance the watcher would
            // attest to is not the vault's balance -- and attesting to a
            // number that is too low is exactly what the solvency gate treats
            // as an emergency. Wait, rather than raise a false alarm.
            if let Some(b) = bridge.as_mut() {
                if synced >= target {
                    // Passes back to back while the bridge is behind, not one
                    // per sleep. A pass reports the end of its own range as the
                    // tip, so it keeps the last `confirmations` blocks back for
                    // the next pass and nets about half a step — one of those
                    // every thirty seconds turns an hour of catch-up into a
                    // day. The cap hands control back to refresh the tip.
                    for _ in 0..BRIDGE_BURST {
                        let before = b.watcher.scanned_to();
                        match bridge_pass(
                            b,
                            &scanner,
                            &notes,
                            &zebra,
                            &bridge_node,
                            &bridge_journal,
                            &bridge_pub,
                            &dir2,
                            chain,
                        ) {
                            Ok(0) => {}
                            Ok(n) => eprintln!("zyn-object-node: bridge credited {n} deposit(s)"),
                            Err(e) => {
                                eprintln!("zyn-object-node: BRIDGE PASS FAILED: {e}");
                                if let Ok(mut g) = sync_writer.lock() {
                                    g.error = e.chars().take(200).collect();
                                }
                                break;
                            }
                        }
                        let after = b.watcher.scanned_to();
                        BRIDGE_JUDGED.store(after, std::sync::atomic::Ordering::SeqCst);
                        // Caught up, or not moving: either way the next thing
                        // worth doing is reading the tip again.
                        if after >= target || after == before {
                            break;
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(30));
        });
        (a.settler, a.da_dir, a.da_key, a.data_dir, a.exits)
    });

    // Carry roots to Zcash, and publish what lets anyone check them.
    if let Some((mut settler, da_dir, da_key, data_dir, mut exits)) = anchoring {
        // Bundles written only to this box are bundles that die with it. The
        // retired chain pushed its DA to the mirror so a third party could
        // verify a root without asking us; the object ledger had no such push
        // at all, so its bundles would have existed in exactly one place.
        let mut publisher = zynzapd::publish::Publisher::new(zynzapd::publish::parse_mirrors(
            &std::env::var("ZYN_DA_MIRRORS").unwrap_or_default(),
        ));
        match publisher.mirrors().len() {
            0 => eprintln!(
                "zyn-object-node: DA bundles are written to {} only — set ZYN_DA_MIRRORS \
                 so a root outlives this box and someone else can check it",
                da_dir.display()
            ),
            n => eprintln!("zyn-object-node: DA bundles publish to {n} mirror(s)"),
        }
        let node = Arc::clone(&node);
        let journal = Arc::clone(&journal);
        // The settled-exit intents go through the same publish path every
        // other intent does, so this thread needs the published log too.
        let pub_log = Arc::clone(&pub_log);
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(20));
            if let (Some(floor), Ok(n)) = (BACKING_FLOOR.get(), node.lock()) {
                floor.store(backing_zat(n.state(), &zec_asset()), std::sync::atomic::Ordering::SeqCst);
            }
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
                        Ok(files) => {
                            eprintln!(
                                "zyn-object-node: DA bundle for epoch {} written ({} files)",
                                c.anchor.checkpoint.epoch,
                                files.len()
                            );
                            publisher.queue(files);
                        }
                        Err(e) => eprintln!(
                            "zyn-object-node: could not write the bundle for epoch {}: {e}",
                            c.anchor.checkpoint.epoch
                        ),
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("zyn-object-node: anchor pass failed: {e}"),
            }
            // Exits, after anchoring. Both spend from the same vault, and
            // running the anchor first keeps the chain's own settlement ahead
            // of payouts when notes are scarce -- a chain that cannot anchor
            // is one whose exits nobody can verify anyway.
            if let Some(ex) = exits.as_mut() {
                for r in NEW_REVEALS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default() {
                    ex.add_reveal(r);
                }
                let ops = {
                    let vm_ops = node
                        .lock()
                        .map_err(|_| "node lock poisoned".to_string())
                        .and_then(|n| ex.poll_once(n.state()));
                    match vm_ops {
                        Ok(ops) => ops,
                        Err(e) => {
                            eprintln!("zyn-object-node: exit pass failed: {e}");
                            Vec::new()
                        }
                    }
                };
                for op in &ops {
                    let receipts =
                        apply_operator(
                            op,
                            &node,
                            &journal,
                            &pub_log,
                            &data_dir.to_string_lossy(),
                            chain,
                        );
                    if receipts.iter().any(ObjectVm::rejected) {
                        // The vault paid and the ledger refused to record it.
                        // Loud, because the two now disagree about a debt that
                        // has actually been settled on Zcash.
                        eprintln!(
                            "zyn-object-node: SETTLEMENT REFUSED by the ledger: {receipts:?} — \
                             Zcash has paid this exit and the chain still says it is owed"
                        );
                    }
                }
                if !ops.is_empty() {
                    eprintln!("zyn-object-node: recorded {} settled exit(s)", ops.len());
                }
            }
            // Drained on every pass, not only after a confirmation: a mirror
            // that was down when a bundle was written must still receive it,
            // and the queue is what remembers that.
            if !publisher.mirrors().is_empty() && publisher.pending_len() > 0 {
                match publisher.poll_once() {
                    Ok(n) if n > 0 => eprintln!(
                        "zyn-object-node: pushed {n} DA file(s); {} still queued",
                        publisher.pending_len()
                    ),
                    Ok(_) => {}
                    Err(e) => eprintln!("zyn-object-node: DA mirror push failed: {e}"),
                }
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
        let da_dir = rpc_da_dir.clone();
        let sync = Arc::clone(&sync_progress);
        let vault = rpc_vault.clone();
        std::thread::spawn(move || {
            serve(s, node, journal, pub_log, dir, chain, anchors, da_dir, sync, vault)
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn deposits_the_bridge_has_not_reached_are_not_the_operators() {
        // An empty store has nothing unjudged, whatever the height.
        let notes = zyn_custody::shielded::PoolStores {
            orchard: std::sync::Arc::new(std::sync::Mutex::new(zyn_custody::notes::NoteStore::new())),
            ironwood: std::sync::Arc::new(std::sync::Mutex::new(zyn_custody::notes::NoteStore::new())),
        };
        assert_eq!(super::unjudged_zat(&notes, 0), 0);
        assert_eq!(super::unjudged_zat(&notes, u64::MAX), 0);
        // With no bridge, the judged height stays at MAX and nothing is held back.
        assert_eq!(super::BRIDGE_JUDGED.load(std::sync::atomic::Ordering::SeqCst), u64::MAX);
    }

    #[test]
    fn the_vault_balance_is_in_the_ledgers_scale() {
        // 0.003 ZEC held, as the bridge's attestation of a 0.003 deposit
        // states it: the two must compare equal, or every deposit reads as
        // more than the vault holds.
        let held = super::zatoshi_as_fixed(&[100_000, 200_000]).unwrap();
        assert_eq!(held, zyn_vm::Fixed::raw(3_000_000_000_000_000)); // 0.003 × 1e18
        assert_eq!(super::zatoshi_as_fixed(&[]).unwrap(), zyn_vm::Fixed::ZERO);
        assert!(super::zatoshi_as_fixed(&[u64::MAX, u64::MAX]).is_ok());
    }
    use super::*;

    /// The status document is hand-rolled JSON, and the one failure mode that
    /// matters is a document nothing can parse — which a reader cannot tell
    /// apart from the node being down. The scanner's errors are operator prose
    /// carrying txids, so they are the field most likely to contain a quote.
    #[test]
    fn a_scanner_error_with_a_quote_still_leaves_parseable_json() {
        let vm = ObjectVm::genesis(212, objectvm::Limits { max_io: 8 });
        let sync = SyncProgress {
            synced_to: 10,
            target: 20,
            error: "stopped: \"abc\" carries no memo\nadd it\tsomewhere".into(),
        };
        let out = status_json(&vm, true, None, &sync);
        let v: serde_json::Value =
            serde_json::from_str(&out).expect("the status must stay parseable");
        assert_eq!(
            v["sync_error"], "stopped: \"abc\" carries no memo\nadd it\tsomewhere",
            "and must round-trip the message rather than mangle it"
        );
        assert_eq!(v["synced_to"], 10);
        assert_eq!(v["sync_target"], 20);
    }

    /// A node that has never anchored must say so as zero rather than omit the
    /// field: the bot reads "no anchor yet" from it, and a missing key would
    /// read as the same zero for the wrong reason.
    #[test]
    fn a_chain_with_no_anchors_reports_zero_not_a_missing_field() {
        let vm = ObjectVm::genesis(212, objectvm::Limits { max_io: 8 });
        let out = status_json(&vm, true, None, &SyncProgress::default());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["anchored_epoch"], 0);
        assert_eq!(v["anchored_height"], 0);
        assert_eq!(v["anchor_txid"], "");
        assert!(v["anchoring"].as_str().unwrap().starts_with("on"));
    }

    /// The backing claim is the number that matters once real money is in:
    /// a reader compares it against the vault's balance on Zcash and is doing
    /// the same check the bridge does every pass, without needing a key. A
    /// chain that has issued nothing must still emit a valid empty object,
    /// not a bare `{}`-shaped hole that breaks the whole document.
    #[test]
    fn the_status_reports_what_the_ledger_has_issued_per_asset() {
        let vm = ObjectVm::genesis(212, objectvm::Limits { max_io: 8 });
        let out = status_json(&vm, true, None, &SyncProgress::default());
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON when nothing is issued");
        assert_eq!(v["issued"], serde_json::json!({}));
        assert_eq!(v["deposits_credited"], 0);
    }

    /// The debt figure. It counts exits nobody has revealed a destination
    /// for, because an unrevealed exit is still owed — a vault sized only for
    /// the revealed ones is a vault that runs out.
    #[test]
    fn the_status_reports_what_is_still_owed() {
        let mut vm = ObjectVm::genesis(212, objectvm::Limits { max_io: 8 });
        let v: serde_json::Value =
            serde_json::from_str(&status_json(&vm, true, None, &SyncProgress::default())).unwrap();
        assert_eq!(v["owed_zatoshi"], 0, "a chain owing nothing says zero");

        vm.exits.push(objectvm::PendingExit {
            asset: zec_asset(),
            amount: zynzapd::object_exit::from_zatoshi(5),
            destination: [0x01; 32],
            since_epoch: 1,
            paid_by: None,
        });
        vm.exits.push(objectvm::PendingExit {
            asset: zec_asset(),
            amount: zynzapd::object_exit::from_zatoshi(9),
            destination: [0x02; 32],
            since_epoch: 1,
            paid_by: Some([0xD1; 32]),
        });
        let v: serde_json::Value =
            serde_json::from_str(&status_json(&vm, true, None, &SyncProgress::default())).unwrap();
        assert_eq!(v["owed_zatoshi"], 5, "a paid exit is no longer owed");
    }

    /// Two assets are never added together: they share no unit, and one
    /// number covering both would mean nothing.
    #[test]
    fn issued_is_reported_per_asset_and_never_summed() {
        let mut vm = ObjectVm::genesis(212, objectvm::Limits { max_io: 8 });
        vm.issued.insert([0xAA; 32], zyn_vm::Fixed::whole(3));
        vm.issued.insert([0xBB; 32], zyn_vm::Fixed::whole(4));
        let v: serde_json::Value =
            serde_json::from_str(&status_json(&vm, true, None, &SyncProgress::default())).unwrap();
        let issued = v["issued"].as_object().expect("an object keyed by asset");
        assert_eq!(issued.len(), 2, "each asset stands alone: {issued:?}");
        assert!(issued.contains_key(&"aa".repeat(32)));
        assert!(issued.contains_key(&"bb".repeat(32)));
    }

    /// A vault outlives the ledgers that custody it. Every deposit below the
    /// handover was the old ledger's to credit, and scanning it again issues
    /// it twice — with each ledger's solvency check passing on its own.
    #[test]
    fn the_bridge_does_not_start_below_the_chain_it_replaced() {
        // Chain 11 stopped scanning at 4,380,197 and handed the vault on.
        assert_eq!(bridge_start(None, Some(4_380_198), 4_354_600), 4_380_197);
    }

    /// With no handover, the vault's birth — not zero, which is millions of
    /// empty steps the scanner would clamp away anyway.
    #[test]
    fn with_no_floor_the_bridge_starts_at_the_vaults_birth() {
        assert_eq!(bridge_start(None, None, 4_354_600), 4_354_599);
    }

    /// A saved cursor above the floor is progress and is kept.
    #[test]
    fn a_saved_cursor_above_the_floor_is_kept() {
        assert_eq!(bridge_start(Some(4_381_000), Some(4_380_198), 4_354_600), 4_381_000);
    }

    /// A saved cursor below the floor predates the floor being set. The floor
    /// is a statement about ownership and a cursor only about work, so the
    /// cursor is raised rather than trusted.
    #[test]
    fn a_saved_cursor_below_the_floor_is_raised_to_it() {
        assert_eq!(bridge_start(Some(4_360_000), Some(4_380_198), 4_354_600), 4_380_197);
    }

    /// A floor below the vault's birth means nothing: there were no deposits
    /// to this vault before it existed.
    #[test]
    fn a_floor_below_the_vaults_birth_is_the_birth() {
        assert_eq!(bridge_start(None, Some(100), 4_354_600), 4_354_599);
    }

    /// Equal-time, and length is not enough to match.
    #[test]
    fn the_operator_token_must_match_exactly() {
        let want = "k".repeat(40);
        assert!(token_matches(&want, &want));
        assert!(!token_matches(&"k".repeat(39), &want));
        assert!(!token_matches(&format!("{}x", "k".repeat(39)), &want));
        assert!(!token_matches("", &want));
    }

    /// What a wallet shows its holder when it reaches the wrong ledger.
    #[test]
    fn a_method_this_ledger_does_not_serve_says_which_ledger_this_is() {
        let msg = unknown_method("zyn_assets");
        assert!(msg.contains("object ledger"), "{msg}");
        assert!(msg.contains("zyn_assets"), "it must name what was asked: {msg}");
        assert!(msg.contains("zyn_status"), "and what it does serve: {msg}");
        // It goes into a JSON error field, so a method name with a quote in it
        // must not end the document early.
        let sneaky = unknown_method("a\"b");
        assert!(
            serde_json::from_str::<serde_json::Value>(&err(&sneaky)).is_ok(),
            "the refusal must stay parseable: {sneaky}"
        );
        // A body with no method at all still gets a sentence, not a blank.
        assert!(unknown_method("").contains("that method"));
    }

    /// And when one has landed, the status carries the mapping rather than
    /// making a reader go and find the index themselves.
    #[test]
    fn a_landed_anchor_is_reported_with_its_zcash_transaction() {
        let vm = ObjectVm::genesis(212, objectvm::Limits { max_io: 8 });
        let last = zynzapd::publish::IndexEntry {
            epoch: 7,
            root: [0xAB; 32],
            anchor_id: [0xCD; 32],
            txid: "deadbeef".into(),
            height: 4_380_000,
        };
        let out = status_json(&vm, true, Some(&last), &SyncProgress::default());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["anchored_epoch"], 7);
        assert_eq!(v["anchored_height"], 4_380_000u64);
        assert_eq!(v["anchor_txid"], "deadbeef");
    }
}

#[cfg(test)]
mod signed_door {
    //! The holders' door, driven through the real handler.
    //!
    //! Every refusal here is a way somebody could otherwise act on an account
    //! they do not control, and one test proves the door opens at all — without
    //! it the rest would pass for a door that is simply shut.
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use zyn_vm::auth::{signing_payload, Authorization};
    use zyn_vm::spec::MicrochainVm;
    use zyn_vm::verify::{authorize_intent, encode_authorized, Credential};

    const CHAIN: u32 = 212;

    struct Rig {
        node: Arc<Mutex<Node<ObjectVm>>>,
        journal: Arc<Mutex<Journal>>,
        pub_log: Arc<Mutex<Published>>,
        sync: Arc<Mutex<SyncProgress>>,
        dir: String,
    }

    fn rig(name: &str) -> Rig {
        let dir = std::env::temp_dir().join(format!("zyn-door-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _ = REPLAY.set(Mutex::new(zyn::replay::ReplayIndex::default()));
        let policy = EpochPolicy {
            intents_per_epoch: 64,
            epochs_per_anchor: u64::MAX,
            max_seconds_per_epoch: 3600,
            max_seconds_per_anchor: u64::MAX,
        };
        Rig {
            node: Arc::new(Mutex::new(Node::new(
                CHAIN,
                objectvm::Limits { max_io: 8 },
                policy,
                Economics::flat(1),
            ))),
            journal: Arc::new(Mutex::new(Journal::open(&dir, CHAIN).unwrap())),
            pub_log: Arc::new(Mutex::new(Published::default())),
            sync: Arc::new(Mutex::new(SyncProgress::default())),
            dir: dir.to_string_lossy().to_string(),
        }
    }

    impl Rig {
        fn submit(&self, frame: &[u8]) -> serde_json::Value {
            let body = format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"zyn_submitSigned\",\"frame\":\"{}\"}}",
                hex(frame)
            );
            let out = handle(
                &body, &self.node, &self.journal, false, &self.pub_log, &self.dir, CHAIN, None,
                &self.sync, None,
            );
            serde_json::from_str(&out).expect("the door answers JSON")
        }
    }

    /// A Register intent for `key`'s own account, signed by `signer`.
    fn register_frame(key: &SigningKey, signer: &SigningKey, expiry: u64) -> Vec<u8> {
        let account = zyn_vm::auth::account_of(
            zyn_vm::auth::Scheme::Ed25519,
            key.verifying_key().as_bytes(),
        );
        let intent = Op::Register {
            account,
            program: zyn_vm::program::register_viewing_key(),
            viewing: [0x42; 32],
        };
        let auth = Authorization {
            chain_id: CHAIN,
            vm_id: zyn_vm::zvm::vm_id::<ObjectVm>(),
            valid_until_epoch: expiry,
        };
        let sig = signer.sign(&signing_payload::<ObjectVm>(&auth, &intent));
        let cred = Credential::Ed25519 {
            key: signer.verifying_key().to_bytes(),
            signature: sig.to_bytes(),
        };
        // Built the long way round when the signer is not the account's key,
        // because `authorize_intent` rightly refuses to make one of those.
        match authorize_intent::<ObjectVm>(&[cred.clone()], &auth, intent.clone(), &ObjectVm::genesis(CHAIN, objectvm::Limits { max_io: 8 })) {
            Ok(a) => encode_authorized::<ObjectVm>(&a),
            Err(_) => {
                let mut e = zyn_vm::commit::Encoder::new();
                e.bytes(b"ZYNAUTH1").u8(1).u32(auth.chain_id).bytes(&auth.vm_id)
                    .u64(auth.valid_until_epoch).u8(1).u8(zyn_vm::auth::Scheme::Ed25519.tag())
                    .bytes(&signer.verifying_key().to_bytes()).bytes(&sig.to_bytes());
                let i = <ObjectVm as MicrochainVm>::encode_intent(&intent);
                e.u32(i.len() as u32).bytes(&i);
                e.finish().to_vec()
            }
        }
    }

    #[test]
    fn a_reply_carries_the_id_it_was_asked_under() {
        let out = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#.to_string();
        let uuid = r#"{"jsonrpc":"2.0","id":"0b5c9f3e-1d2a-4c55-9a51-7e1c2f0b9d11","method":"zyn_status"}"#;
        assert!(with_request_id(uuid, out.clone()).contains(r#""id":"0b5c9f3e-1d2a-4c55-9a51-7e1c2f0b9d11""#));
        assert!(with_request_id(r#"{"id": 42,"method":"x"}"#, out.clone()).contains(r#""id":42,"#));
        // Caller bytes are not spliced into JSON.
        assert!(with_request_id(r#"{"id":"a\",\"b","method":"x"}"#, out.clone()).contains(r#""id":1,"#));
        assert_eq!(with_request_id(r#"{"method":"x"}"#, out.clone()), out);
    }

    #[test]
    fn a_holders_own_signature_opens_it() {
        let r = rig("opens");
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let v = r.submit(&register_frame(&key, &key, 1_000));
        assert!(v.get("error").is_none(), "a valid signature must apply: {v}");
        assert!(v["result"]["receipts"].as_str().unwrap().contains("Registered"), "{v}");
        assert_eq!(r.node.lock().unwrap().state().viewing.len(), 1);
    }

    #[test]
    fn the_same_authorization_cannot_run_twice() {
        let r = rig("replay");
        let key = SigningKey::from_bytes(&[8u8; 32]);
        let frame = register_frame(&key, &key, 1_000);
        assert!(r.submit(&frame).get("error").is_none());
        let again = r.submit(&frame);
        assert!(
            again["error"]["message"].as_str().unwrap().contains("already applied"),
            "{again}"
        );
    }

    /// The attack the door exists to stop: a valid signature, from the wrong
    /// key, over an intent naming someone else's account.
    #[test]
    fn a_signature_from_someone_else_is_refused() {
        let r = rig("stranger");
        let victim = SigningKey::from_bytes(&[9u8; 32]);
        let thief = SigningKey::from_bytes(&[10u8; 32]);
        let v = r.submit(&register_frame(&victim, &thief, 1_000));
        let msg = v["error"]["message"].as_str().unwrap_or_default().to_string();
        // Refused on *authority* — the victim's account never signed — and not
        // because the frame was malformed, which would make this test pass for
        // nothing. (It did, until the credential's scheme tag was encoded right.)
        assert!(msg.contains("Authority("), "wrong reason: {v}");
        assert!(!msg.contains("Malformed") && !msg.contains("BadSignature"), "wrong reason: {v}");
        assert!(r.node.lock().unwrap().state().viewing.is_empty());
    }

    #[test]
    fn a_tampered_frame_is_refused() {
        let r = rig("tamper");
        let key = SigningKey::from_bytes(&[11u8; 32]);
        let mut frame = register_frame(&key, &key, 1_000);
        let last = frame.len() - 1;
        frame[last] ^= 0x01; // the viewing key the signature covered
        let v = r.submit(&frame);
        assert!(v["error"]["message"].as_str().unwrap_or_default().contains("BadSignature"), "wrong reason: {v}");
        assert!(r.node.lock().unwrap().state().viewing.is_empty());
    }

    /// An operator frame decodes for intents nobody signs — a Mint among them.
    /// This door takes signatures and nothing else.
    #[test]
    fn an_operator_frame_is_refused_here() {
        let r = rig("operator");
        let mint = encode_authorized::<ObjectVm>(&zyn_vm::verify::Authorized::operator(Op::Mint {
            asset: [0x0e; 32],
            amount: zyn_vm::Fixed::whole(1_000),
            owner: [0xAA; 32],
            nonce: [0x01; 32],
        }));
        let v = r.submit(&mint);
        assert!(v["error"]["message"].as_str().unwrap().contains("operator frame"), "{v}");
        assert!(r.node.lock().unwrap().state().issued.is_empty(), "nothing may be minted");
    }

    #[test]
    fn an_expired_authorization_is_refused() {
        let r = rig("expired");
        let key = SigningKey::from_bytes(&[12u8; 32]);
        // Sealed past epoch 0 first, so an envelope valid until 0 has expired.
        r.node.lock().unwrap().submit_operator(ObjectVm::seal_intent(), now_secs());
        let v = r.submit(&register_frame(&key, &key, 0));
        assert!(v["error"]["message"].as_str().unwrap_or_default().contains("Expired"), "wrong reason: {v}");
    }
}

#[cfg(test)]
mod market_door {
    //! The market through the node: signed intents in, public state and
    //! claims out, and every created object published for its owner to find.
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use objectvm::market;
    use zyn_vm::auth::{signing_payload, Authorization};
    use zyn_vm::spec::MicrochainVm;
    use zyn_vm::verify::{authorize_intent, encode_authorized, Credential};
    use zyn_vm::Fixed;

    const CHAIN: u32 = 212;
    const ZEC: [u8; 32] = [0x10; 32];
    const USD: [u8; 32] = [0xF0; 32];

    struct Rig {
        node: Arc<Mutex<Node<ObjectVm>>>,
        journal: Arc<Mutex<Journal>>,
        pub_log: Arc<Mutex<Published>>,
        sync: Arc<Mutex<SyncProgress>>,
        dir: String,
    }

    fn rig(name: &str) -> Rig {
        let dir = std::env::temp_dir().join(format!("zyn-market-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _ = REPLAY.set(Mutex::new(zyn::replay::ReplayIndex::default()));
        let policy = EpochPolicy {
            intents_per_epoch: 1_000,
            epochs_per_anchor: u64::MAX,
            max_seconds_per_epoch: 3600,
            max_seconds_per_anchor: u64::MAX,
        };
        Rig {
            node: Arc::new(Mutex::new(Node::new(
                CHAIN,
                objectvm::Limits { max_io: 8 },
                policy,
                Economics::flat(1),
            ))),
            journal: Arc::new(Mutex::new(Journal::open(&dir, CHAIN).unwrap())),
            pub_log: Arc::new(Mutex::new(Published::default())),
            sync: Arc::new(Mutex::new(SyncProgress::default())),
            dir: dir.to_string_lossy().to_string(),
        }
    }

    impl Rig {
        fn call(&self, body: &str) -> serde_json::Value {
            let out = handle(
                body, &self.node, &self.journal, false, &self.pub_log, &self.dir, CHAIN, None,
                &self.sync, None,
            );
            serde_json::from_str(&out).expect("JSON")
        }

        fn signed(&self, key: &SigningKey, intent: Op) -> serde_json::Value {
            let auth = Authorization {
                chain_id: CHAIN,
                vm_id: zyn_vm::zvm::vm_id::<ObjectVm>(),
                valid_until_epoch: 1_000,
            };
            let cred = Credential::Ed25519 {
                key: key.verifying_key().to_bytes(),
                signature: key.sign(&signing_payload::<ObjectVm>(&auth, &intent)).to_bytes(),
            };
            let a = {
                let n = self.node.lock().unwrap();
                authorize_intent::<ObjectVm>(&[cred], &auth, intent, n.state()).expect("authorizes")
            };
            self.call(&format!(
                "{{\"method\":\"zyn_submitSigned\",\"frame\":\"{}\"}}",
                hex(&encode_authorized::<ObjectVm>(&a))
            ))
        }

        fn operator(&self, op: Op) {
            let r = apply_operator(&op, &self.node, &self.journal, &self.pub_log, &self.dir, CHAIN);
            assert!(!r.iter().any(ObjectVm::rejected), "{r:?}");
        }

        fn live(&self, owner: &[u8; 32], asset: [u8; 32]) -> Vec<([u8; 32], zyn_vm::object::Note)> {
            let n = self.node.lock().unwrap();
            n.state()
                .live
                .iter()
                .filter(|(_, v)| &v.owner == owner && v.asset == asset)
                .map(|(c, v)| (*c, *v))
                .collect()
        }
    }

    fn account(key: &SigningKey) -> [u8; 32] {
        zyn_vm::auth::account_of(zyn_vm::auth::Scheme::Ed25519, key.verifying_key().as_bytes())
    }

    fn output(owner: [u8; 32], asset: [u8; 32], amount: Fixed, nonce: u8) -> objectvm::Output {
        use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
        objectvm::Output {
            note: zyn_vm::object::Note { asset, amount, owner, nonce: [nonce; 32] },
            enc: seal(&viewing_public(&StaticSecret::from([1; 32])), StaticSecret::from([nonce; 32]), b"x").unwrap(),
        }
    }

    #[test]
    fn a_pool_and_a_swap_through_the_door() {
        let r = rig("swap");
        let alice = SigningKey::from_bytes(&[21u8; 32]);
        let bob = SigningKey::from_bytes(&[22u8; 32]);
        let (a, b) = (account(&alice), account(&bob));
        for (owner, asset, n) in [(a, ZEC, 1u8), (a, USD, 2), (b, ZEC, 3)] {
            r.operator(Op::Mint { asset, amount: Fixed::whole(100), owner, nonce: [n; 32] });
        }

        let ins: Vec<_> = [r.live(&a, ZEC), r.live(&a, USD)]
            .concat()
            .into_iter()
            .map(|(c, _)| (c, zyn_vm::object::nullifier(&[1; 32], &c)))
            .collect();
        let v = r.signed(&alice, Op::CreatePool {
            provider: a,
            program: zyn_vm::program::add_liquidity(),
            asset_a: ZEC,
            asset_b: USD,
            amount_a: Fixed::whole(100),
            amount_b: Fixed::whole(100),
            fee_bps: 30,
            inputs: ins,
            change: vec![],
            nonce: [0x51; 32],
        });
        assert!(v["result"]["receipts"].as_str().unwrap().contains("Pooled"), "{v}");

        let pool = market::pool_id(&ZEC, &USD);
        let (c, _) = r.live(&b, ZEC)[0];
        let v = r.signed(&bob, Op::Swap {
            trader: b,
            program: zyn_vm::program::amm_swap(),
            pool,
            sell_a: true,
            amount_in: Fixed::whole(40),
            min_out: Fixed::ZERO,
            inputs: vec![(c, zyn_vm::object::nullifier(&[2; 32], &c))],
            change: vec![output(b, ZEC, Fixed::whole(60), 0x61)],
            nonce: [0x52; 32],
        });
        assert!(v["result"]["receipts"].as_str().unwrap().contains("Ordered"), "{v}");

        // Bob's change is published for him to find, and his input retired.
        {
            let p = r.pub_log.lock().unwrap();
            let change = zyn_vm::object::commitment(&output(b, ZEC, Fixed::whole(60), 0x61).note).unwrap();
            assert!(p.outputs.iter().any(|(cm, _, _)| cm == &hex(&change)));
            assert!(p.nullifiers.contains(&hex(&zyn_vm::object::nullifier(&[2; 32], &c))));
        }

        let m = r.call("{\"method\":\"zyn_market\"}");
        assert_eq!(m["result"]["pools"][0]["pending"], 1, "{m}");
        assert_eq!(m["result"]["pools"][0]["reserve_a"], Fixed::whole(100).0.to_string());

        r.operator(ObjectVm::seal_intent());
        let m = r.call("{\"method\":\"zyn_market\"}");
        assert_eq!(m["result"]["pools"][0]["pending"], 0);
        assert_eq!(m["result"]["pools"][0]["reserve_a"], Fixed::whole(140).0.to_string());

        // Bob, and only somebody holding his nonce, finds what he is owed.
        let proceeds = market::claim_id(&market::order_id(&b, &[0x52; 32]), 0);
        let v = r.call(&format!("{{\"method\":\"zyn_claims\",\"ids\":\"{}\"}}", hex(&proceeds)));
        let row = &v["result"][0];
        assert_eq!(row["asset"], hex(&USD));
        let owed: i128 = row["amount"].as_str().unwrap().parse().unwrap();
        assert!(owed > 0);

        let v = r.signed(&bob, Op::Claim {
            owner: b,
            program: zyn_vm::program::claim(),
            claims: vec![proceeds],
            outputs: vec![output(b, USD, Fixed(owed), 0x71)],
        });
        assert!(v["result"]["receipts"].as_str().unwrap().contains("Claimed"), "{v}");
        let v = r.call(&format!("{{\"method\":\"zyn_claims\",\"ids\":\"{}\"}}", hex(&proceeds)));
        assert_eq!(v["result"].as_array().unwrap().len(), 0, "redeemed once");
    }

    /// The NFT cycle through the node: mint, a signed listing that names no
    /// seller, a buyer who pays price plus fee from what `zyn_offers` says,
    /// and a seller who finds the proceeds by their own nonce.
    #[test]
    fn an_item_sells_through_the_door() {
        let r = rig("nft");
        let alice = SigningKey::from_bytes(&[31u8; 32]);
        let bob = SigningKey::from_bytes(&[32u8; 32]);
        let (a, b) = (account(&alice), account(&bob));
        const CAVE: [u8; 32] = [0xCA; 32];
        let view = |k: u8| zyn_vm::note_enc::viewing_public(&zyn_vm::note_enc::StaticSecret::from([k; 32]));
        r.operator(Op::Register { account: a, program: zyn_vm::program::register_viewing_key(), viewing: view(1) });
        r.operator(Op::Register { account: b, program: zyn_vm::program::register_viewing_key(), viewing: view(2) });
        r.operator(Op::Mint { asset: ZEC, amount: Fixed::whole(50), owner: b, nonce: [9; 32] });
        r.operator(Op::RegisterCollection {
            program: zyn_vm::program::register_collection(),
            asset: CAVE,
            creator: [0x7E; 32],
            quote: ZEC,
            fee_bps: 100,
            cap: 10,
        });
        let mint = {
            let n = r.node.lock().unwrap();
            objectvm::primary::mint_item(n.state(), CAVE, 4, a, [0x44; 32], zyn_vm::note_enc::StaticSecret::from([5; 32])).unwrap()
        };
        r.operator(mint);
        let asset = market::item_asset(&CAVE, 4);
        let (item, _) = r.live(&a, asset)[0];
        let nf = zyn_vm::object::nullifier(&[1; 32], &item);

        let v = r.signed(&alice, Op::List {
            seller: a,
            program: zyn_vm::program::place_offer(),
            item,
            item_nullifier: nf,
            price_asset: ZEC,
            price_amount: Fixed::whole(10),
            expires_at_epoch: 1_000,
            nonce: [0x61; 32],
        });
        assert!(v["result"]["receipts"].as_str().unwrap().contains("Listed"), "{v}");

        let offers = r.call("{\"method\":\"zyn_offers\"}");
        let o = &offers["result"]["offers"][0];
        assert_eq!(o["serial"], 4);
        assert_eq!(o["collection"], hex(&CAVE));
        assert_eq!(o["nullifier"], hex(&nf));
        assert!(!offers.to_string().contains(&hex(&a)), "a listing names no seller");
        let pay: i128 = o["pay_raw"].as_str().unwrap().parse().unwrap();
        assert_eq!(pay, Fixed(10_100_000_000_000_000_000).0);
        let items = r.call(&format!("{{\"method\":\"zyn_items\",\"assets\":\"{}\"}}", hex(&asset)));
        assert_eq!(items["result"][0]["serial"], 4);

        let (c, _) = r.live(&b, ZEC)[0];
        let v = r.signed(&bob, Op::TakeOffer {
            buyer: b,
            program: zyn_vm::program::take_offer(),
            item,
            item_nullifier: nf,
            payment: vec![(c, zyn_vm::object::nullifier(&[2; 32], &c))],
            outputs: vec![
                output(b, asset, market::ITEM_UNIT, 0x81),
                output(b, ZEC, Fixed(Fixed::whole(50).0 - pay), 0x82),
            ],
        });
        assert!(v["result"]["receipts"].as_str().unwrap().contains("Taken"), "{v}");
        assert_eq!(r.live(&b, asset).len(), 1, "Bob holds serial 4");

        let proceeds = market::claim_id(&market::listing_source(&a, &[0x61; 32]), 0);
        let v = r.call(&format!("{{\"method\":\"zyn_claims\",\"ids\":\"{}\"}}", hex(&proceeds)));
        assert_eq!(v["result"][0]["amount"], Fixed(9_900_000_000_000_000_000).0.to_string());
    }

    #[test]
    fn a_payer_can_look_up_where_to_pay_a_registered_account() {
        let r = rig("viewing");
        let v = r.call(&format!("{{\"method\":\"zyn_viewingKey\",\"account\":\"{}\"}}", hex(&[0x77; 32])));
        assert!(v["error"]["message"].as_str().unwrap().contains("not registered"), "{v}");
        r.operator(Op::Register { account: [0x77; 32], program: zyn_vm::program::register_viewing_key(), viewing: [0x42; 32] });
        let v = r.call(&format!("{{\"method\":\"zyn_viewingKey\",\"account\":\"{}\"}}", hex(&[0x77; 32])));
        assert_eq!(v["result"]["viewing"], hex(&[0x42; 32]));
    }

    /// Only the account an intent names can sign it — including a claim,
    /// which would otherwise let anyone who learned an id collect it.
    #[test]
    fn a_claim_needs_its_owners_signature() {
        let r = rig("claimsig");
        let bob = SigningKey::from_bytes(&[23u8; 32]);
        let eve = SigningKey::from_bytes(&[24u8; 32]);
        let intent = Op::Claim {
            owner: account(&bob),
            program: zyn_vm::program::claim(),
            claims: vec![[1; 32]],
            outputs: vec![output(account(&eve), USD, Fixed::whole(1), 0x72)],
        };
        let auth = Authorization {
            chain_id: CHAIN,
            vm_id: zyn_vm::zvm::vm_id::<ObjectVm>(),
            valid_until_epoch: 1_000,
        };
        let cred = Credential::Ed25519 {
            key: eve.verifying_key().to_bytes(),
            signature: eve.sign(&signing_payload::<ObjectVm>(&auth, &intent)).to_bytes(),
        };
        let n = r.node.lock().unwrap();
        let refused = authorize_intent::<ObjectVm>(&[cred], &auth, intent, n.state());
        assert!(format!("{refused:?}").contains("MissingSignature"), "{refused:?}");
    }
}
