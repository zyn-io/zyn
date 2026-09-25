//! `zyn-btc-bridge` — BTC.zy on the object ledger (DECISIONS §154).
//!
//! **In.** Bitcoin wallets cannot attach a memo, so every Zyn deposit code has
//! its own address: the vault's Taproot key tweaked by the code
//! (`zyn_custody::bitcoin::deposit_address`). Nap derives the address itself
//! and tells the node to watch the code (`zyn_depositWatch`); this bridge reads
//! the watched codes, finds coins paid to their addresses through an Esplora
//! API, and once a coin has enough confirmations posts it as
//! `DepositCommitted` — waiting on Zyn under the code until the wallet claims
//! it, as SOL.zy does.
//!
//! **Out.** Revealed exits are paid in one Taproot transaction from credited
//! deposit coins and the vault's change, signed input by input by the
//! custodians (FROST, 2 of 3). The network fee comes out of the first payout,
//! so the vault never pays out more than Zyn burned. The transaction is written
//! to `inflight.json` before it is broadcast; once it confirms, its exits are
//! settled with `SettleExit`.
//!
//! ```text
//!   ZYN_BTC_NODE          object node RPC                 (http://127.0.0.1:8499)
//!   ZYN_SUBMIT_TOKEN      the node's operator token
//!   ZYN_BTC_NETWORK       signet | testnet4 | bitcoin     (signet)
//!   ZYN_BTC_ESPLORA       Esplora API                     (https://mempool.space/signet/api)
//!   ZYN_BTC_PUBLIC        directory holding the vault's public.bin
//!   ZYN_BTC_CUSTODIANS    comma-separated host:port of the custodians
//!   ZYN_BTC_THRESHOLD     signatures required             (2)
//!   ZYN_BTC_CONFIRMATIONS a deposit counts after these    (2)
//!   ZYN_BTC_STATE         state directory                 (/var/lib/zyn/btc-bridge)
//!   ZYN_BTC_POLL          seconds between passes          (60)
//! ```
//!
//! `zyn-btc-bridge status | register | address <commitment> | check | once | run`

use std::collections::BTreeSet;
use std::time::Duration;

use bitcoin::{Address, Network, OutPoint, ScriptBuf};
use objectvm::bridge::asset_of;
use objectvm::{ObjectVm, Op};
use serde_json::{json, Value};
use zyn_custody::bitcoin::{self as btc, Coin, Esplora, Pay, PublicPackage};
use zyn_custody::custody_net::RemoteBitcoinQuorum;
use zyn_vm::commit::Encoder;
use zyn_vm::spec::MicrochainVm;
use zyn_vm::{program, Fixed};

/// One satoshi in `Fixed` (18 decimals).
const SAT: i128 = 10_000_000_000;

fn env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0).then(|| (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()).collect()).flatten()
}

/// The ledger's reference for a Bitcoin output: its outpoint, hashed.
fn out_ref(o: &OutPoint) -> [u8; 32] {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.bitcoin-out.v1").bytes(format!("{}:{}", o.txid, o.vout).as_bytes());
    e.leaf()
}

/// The ledger's reference for a payout transaction.
fn tx_ref(txid: &bitcoin::Txid) -> [u8; 32] {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.bitcoin-tx.v1").bytes(txid.to_string().as_bytes());
    e.leaf()
}

struct Bridge {
    node: String,
    chain: Esplora,
    network: Network,
    name: String,
    asset: [u8; 32],
    public: PublicPackage,
    custodians: Vec<String>,
    threshold: usize,
    confirmations: u64,
    state: std::path::PathBuf,
}

/// A payout sent and not yet confirmed. Written before it is broadcast.
struct InFlight {
    txid: String,
    raw: String,
    covers: Vec<u32>,
}

fn bridge() -> Result<Bridge, String> {
    let network = match env("ZYN_BTC_NETWORK", "signet").as_str() {
        "signet" => Network::Signet,
        "testnet4" => Network::Testnet4,
        "bitcoin" | "mainnet" => Network::Bitcoin,
        other => return Err(format!("ZYN_BTC_NETWORK {other} is not signet, testnet4 or bitcoin")),
    };
    let name = match network {
        Network::Bitcoin => "bitcoin",
        Network::Testnet4 => "bitcoin-test",
        _ => "bitcoin-signet",
    }
    .to_string();
    let public_dir = std::path::PathBuf::from(env("ZYN_BTC_PUBLIC", "/var/zyn/bitcoin-shares"));
    let public = zyn_custody::shares::load_public::<btc::Bitcoin>(&public_dir).map_err(|e| format!("{}: {e:?}", public_dir.display()))?;
    let state = std::path::PathBuf::from(env("ZYN_BTC_STATE", "/var/lib/zyn/btc-bridge"));
    std::fs::create_dir_all(&state).map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(Bridge {
        node: env("ZYN_BTC_NODE", "http://127.0.0.1:8499"),
        chain: Esplora::new(&env("ZYN_BTC_ESPLORA", "https://mempool.space/signet/api")),
        network,
        asset: asset_of(name.as_bytes(), &[]),
        name,
        public,
        custodians: env("ZYN_BTC_CUSTODIANS", "127.0.0.1:8110,127.0.0.1:8111,127.0.0.1:8112").split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        threshold: env("ZYN_BTC_THRESHOLD", "2").parse().map_err(|_| "ZYN_BTC_THRESHOLD is a number")?,
        confirmations: env("ZYN_BTC_CONFIRMATIONS", "2").parse().map_err(|_| "ZYN_BTC_CONFIRMATIONS is a number")?,
        state,
    })
}

impl Bridge {
    fn token(&self) -> Result<String, String> {
        std::env::var("ZYN_SUBMIT_TOKEN").map_err(|_| "set ZYN_SUBMIT_TOKEN (read it from the node's env file)".to_string())
    }

    fn submit(&self, op: &Op) -> Result<String, String> {
        let r = zynzapd::object_client::call(&self.node, "zyn_submit", json!({ "token": self.token()?, "intent": hex(&ObjectVm::encode_intent(op)) }))?;
        Ok(r.get("receipts").and_then(Value::as_str).unwrap_or("").to_string())
    }

    fn vault(&self) -> Result<Address, String> {
        btc::vault_address(&self.public, self.network).map_err(|e| e.to_string())
    }

    /// The deposit codes Nap asked to be watched.
    fn watched(&self) -> Result<Vec<[u8; 32]>, String> {
        let rows = zynzapd::object_client::call(&self.node, "zyn_depositWatched", json!({ "token": self.token()?, "network": self.name }))?;
        Ok(rows.as_array().into_iter().flatten().filter_map(|c| c.as_str().and_then(unhex).and_then(|b| <[u8; 32]>::try_from(b).ok())).collect())
    }

    fn credited_path(&self) -> std::path::PathBuf {
        self.state.join("credited")
    }

    /// Outpoints this bridge has already posted: the ledger would refuse them
    /// again anyway, but asking it every pass is waste.
    fn credited(&self) -> BTreeSet<String> {
        std::fs::read_to_string(self.credited_path()).unwrap_or_default().lines().map(str::to_string).collect()
    }

    fn remember(&self, o: &OutPoint) -> Result<(), String> {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.credited_path()).map_err(|e| e.to_string())?;
        writeln!(f, "{}:{}", o.txid, o.vout).map_err(|e| e.to_string())
    }

    /// Every watched address's coins, confirmed deep enough, posted once.
    /// Returns the coins the vault may spend: credited deposits, and
    /// confirmed change at its own address.
    fn deposits(&self) -> Result<Vec<Coin>, String> {
        let tip = self.chain.tip_height().map_err(|e| e.to_string())?;
        let deep = |h: Option<u64>| h.is_some_and(|h| tip + 1 >= h + self.confirmations);
        let mut credited = self.credited();
        let mut coins = Vec::new();
        for commitment in self.watched()? {
            let addr = btc::deposit_address(&self.public, &commitment, self.network).map_err(|e| e.to_string())?;
            for u in self.chain.utxos(&addr).map_err(|e| e.to_string())? {
                if !deep(u.confirmed_height) {
                    continue;
                }
                let outpoint = OutPoint { txid: u.txid, vout: u.vout };
                let key = format!("{}:{}", u.txid, u.vout);
                if !credited.contains(&key) {
                    let receipts = self.submit(&Op::DepositCommitted {
                        program: program::deposit(),
                        asset: self.asset,
                        gross: Fixed(u.value as i128 * SAT),
                        external_ref: out_ref(&outpoint),
                        commitment,
                    })?;
                    if receipts.contains("Refused") && !receipts.contains("already credited") {
                        return Err(format!("{key}: the ledger refused the deposit — {}", receipts.chars().take(160).collect::<String>()));
                    }
                    self.remember(&outpoint)?;
                    credited.insert(key.clone());
                    println!("in:  {key} — {} sat waiting for {}", u.value, hex(&commitment));
                }
                coins.push(Coin { outpoint, value: u.value, root: Some(btc::deposit_root(&commitment)), script: addr.script_pubkey() });
            }
        }
        let vault = self.vault()?;
        for u in self.chain.utxos(&vault).map_err(|e| e.to_string())? {
            if u.confirmed_height.is_some() {
                coins.push(Coin { outpoint: OutPoint { txid: u.txid, vout: u.vout }, value: u.value, root: None, script: vault.script_pubkey() });
            }
        }
        Ok(coins)
    }

    fn inflight_path(&self) -> std::path::PathBuf {
        self.state.join("inflight.json")
    }

    fn inflight(&self) -> Vec<InFlight> {
        // A file that does not parse is not "nothing in flight": that would
        // pay again. It stops the bridge until someone looks.
        match std::fs::read(self.inflight_path()) {
            Err(_) => Vec::new(),
            Ok(b) => {
                let v: Value = serde_json::from_slice(&b).unwrap_or_else(|_| panic!("{} is damaged — fix it before paying again", self.inflight_path().display()));
                v.as_array()
                    .into_iter()
                    .flatten()
                    .map(|x| {
                        (|| {
                            Some(InFlight {
                                txid: x["txid"].as_str()?.to_string(),
                                raw: x["raw"].as_str()?.to_string(),
                                covers: x["covers"].as_array()?.iter().map(|c| c.as_u64().map(|c| c as u32)).collect::<Option<_>>()?,
                            })
                        })()
                        .unwrap_or_else(|| panic!("{} is damaged", self.inflight_path().display()))
                    })
                    .collect()
            }
        }
    }

    fn save(&self, v: &[InFlight]) -> Result<(), String> {
        let all: Vec<Value> = v.iter().map(|f| json!({ "txid": f.txid, "raw": f.raw, "covers": f.covers })).collect();
        std::fs::write(self.inflight_path(), serde_json::to_vec_pretty(&all).unwrap()).map_err(|e| e.to_string())
    }

    /// Settle what has confirmed; rebroadcast what the network has forgotten.
    fn follow_up(&self) -> Result<bool, String> {
        let mut v = self.inflight();
        if v.is_empty() {
            return Ok(false);
        }
        let tip = self.chain.tip_height().map_err(|e| e.to_string())?;
        let mut keep = Vec::new();
        for f in v.drain(..) {
            let txid: bitcoin::Txid = f.txid.parse().map_err(|_| "a stored txid is not a txid")?;
            match self.chain.status(&txid).map_err(|e| e.to_string())? {
                Some(Some(h)) if tip + 1 >= h + self.confirmations => {
                    for index in &f.covers {
                        let r = self.submit(&Op::SettleExit { program: program::settle_exit(), index: *index, txid: tx_ref(&txid) })?;
                        if r.contains("Refused") && !r.contains("already settled") {
                            eprintln!("zyn-btc-bridge: SettleExit {index} refused — {r}");
                        }
                    }
                    println!("out: {} confirmed — {} exit(s) settled", f.txid, f.covers.len());
                }
                Some(_) => keep.push(f),
                None => {
                    let raw = unhex(&f.raw).ok_or("a stored transaction is not hex")?;
                    match bitcoin::consensus::deserialize::<bitcoin::Transaction>(&raw) {
                        Ok(tx) => {
                            if let Err(e) = self.chain.broadcast(&tx) {
                                eprintln!("zyn-btc-bridge: rebroadcast of {} refused: {e} — investigate before paying again", f.txid);
                            }
                        }
                        Err(_) => eprintln!("zyn-btc-bridge: stored transaction {} does not decode", f.txid),
                    }
                    keep.push(f);
                }
            }
        }
        let busy = !keep.is_empty();
        self.save(&keep)?;
        Ok(busy)
    }

    /// Pay the revealed exits in one transaction.
    fn pay(&self, coins: &[Coin]) -> Result<(), String> {
        let rows = zynzapd::object_client::call(&self.node, "zyn_bridgeExits", json!({ "token": self.token()?, "asset": hex(&self.asset) }))?;
        let mut pays = Vec::new();
        let mut covers = Vec::new();
        for r in rows.as_array().into_iter().flatten() {
            let (Some(index), Some(to)) = (r["index"].as_u64(), r["to"].as_str()) else { continue };
            let amount: i128 = r["amount"].as_str().and_then(|a| a.parse().ok()).unwrap_or(0);
            if amount <= 0 || amount % SAT != 0 {
                eprintln!("zyn-btc-bridge: exit {index} is not a whole number of satoshi; it waits");
                continue;
            }
            let Some(script) = unhex(to).map(ScriptBuf::from_bytes) else { continue };
            pays.push(Pay { script, value: (amount / SAT) as u64 });
            covers.push(index as u32);
        }
        if pays.is_empty() {
            return Ok(());
        }
        let rate = self.chain.fee_rate(6).map_err(|e| e.to_string())?;
        let change = self.vault()?.script_pubkey();
        // Build once to learn the fee, share it across the payouts, rebuild
        // from the same coins: what leaves the vault is what Zyn burned. An
        // exit too small to carry its share waits for a cheaper hour.
        let (mut pays, mut covers) = (pays, covers);
        let (tx, taken, fee) = loop {
            let (tx, taken) = btc::build(coins, &pays, &change, rate).map_err(|e| e.to_string())?;
            let fee = taken.iter().map(|c| c.value).sum::<u64>() - tx.output.iter().map(|o| o.value.to_sat()).sum::<u64>();
            let fee = fee.min(rate * 400 * pays.len() as u64);
            let share = fee.div_ceil(pays.len() as u64);
            let keep: Vec<bool> = pays.iter().map(|p| p.value >= share + btc::DUST).collect();
            if keep.iter().all(|k| *k) {
                break (tx, taken, share);
            }
            for (i, k) in keep.iter().enumerate().rev() {
                if !k {
                    eprintln!("zyn-btc-bridge: exit {} ({} sat) cannot carry its {share}-sat share of the fee; it waits", covers[i], pays[i].value);
                    pays.remove(i);
                    covers.remove(i);
                }
            }
            if pays.is_empty() {
                return Ok(());
            }
        };
        drop(tx);
        for p in pays.iter_mut() {
            p.value -= fee;
        }
        let (tx, taken) = btc::build(&taken, &pays, &change, rate).map_err(|e| e.to_string())?;
        let mut quorum = RemoteBitcoinQuorum::new(self.custodians.clone());
        let tx = btc::sign(&mut quorum, &self.public, self.threshold, tx, &taken).map_err(|e| e.to_string())?;
        let txid = btc::txid(&tx);
        let mut v = self.inflight();
        v.push(InFlight { txid: txid.to_string(), raw: btc::raw_hex(&tx), covers: covers.clone() });
        self.save(&v)?;
        match self.chain.broadcast(&tx) {
            Ok(id) => println!("out: {id} — {} exit(s) paid ({fee} sat of fee each), waiting for {} confirmation(s)", covers.len(), self.confirmations),
            Err(e) => eprintln!("zyn-btc-bridge: broadcast deferred: {e}"),
        }
        Ok(())
    }

    fn once(&self) -> Result<(), String> {
        let coins = self.deposits()?;
        if !self.follow_up()? {
            self.pay(&coins)?;
        }
        Ok(())
    }
}

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    let result = bridge().and_then(|b| match cmd.as_str() {
        "status" => {
            let bridged = zynzapd::object_client::call(&b.node, "zyn_bridged", json!({}))?;
            let issued = bridged.as_array().into_iter().flatten().find(|r| r["asset"].as_str() == Some(&hex(&b.asset))).map(|r| r["issued"].clone());
            let vault = b.vault()?;
            let held: u64 = b.chain.utxos(&vault).map_err(|e| e.to_string())?.iter().map(|u| u.value).sum();
            println!("network  {} ({})", b.name, b.network);
            println!("asset    {}", hex(&b.asset));
            println!("internal {}", btc::internal_key(&b.public).map_err(|e| e.to_string())?);
            println!("vault    {vault}  ({held} sat at the key path; deposits sit at their own addresses)");
            println!("issued   {}", issued.map_or("not registered — run `register`".into(), |i| format!("{i} (Fixed, 18 decimals)")));
            println!("watched  {} deposit code(s)", b.watched().map(|w| w.len()).unwrap_or(0));
            println!("in flight {}", b.inflight().len());
            Ok(())
        }
        "register" => {
            let r = b.submit(&Op::RegisterBridged { program: program::bridge_register(), network: b.name.as_bytes().to_vec(), origin: Vec::new(), decimals: 8 })?;
            println!("register BTC: {}", r.chars().take(120).collect::<String>());
            Ok(())
        }
        "address" => {
            let c = std::env::args().nth(2).and_then(|h| unhex(&h)).and_then(|b| <[u8; 32]>::try_from(b).ok()).ok_or("give a 64-hex deposit commitment")?;
            println!("{}", btc::deposit_address(&b.public, &c, b.network).map_err(|e| e.to_string())?);
            Ok(())
        }
        // The custodians sign a throwaway digest under a throwaway deposit
        // tweak, and the signature is checked against the tweaked key: the
        // quorum works before any coin depends on it.
        "check" => {
            let mut quorum = RemoteBitcoinQuorum::new(b.custodians.clone());
            btc::sign_digest(&mut quorum, &b.public, b.threshold, rand::random(), Some(btc::deposit_root(&rand::random()))).map_err(|e| e.to_string())?;
            println!("custodians sign: {}-of-{} ok", b.threshold, b.custodians.len());
            Ok(())
        }
        "once" => b.once(),
        "run" => {
            let every: u64 = env("ZYN_BTC_POLL", "60").parse().unwrap_or(60);
            loop {
                if let Err(e) = b.once() {
                    eprintln!("zyn-btc-bridge: {e}");
                }
                std::thread::sleep(Duration::from_secs(every));
            }
        }
        other => Err(format!("unknown command {other}: status | register | address <commitment> | check | once | run")),
    });
    if let Err(e) = result {
        eprintln!("zyn-btc-bridge {cmd}: {e}");
        std::process::exit(1);
    }
}
