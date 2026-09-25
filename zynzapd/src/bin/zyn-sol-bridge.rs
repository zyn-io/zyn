//! `zyn-sol-bridge` — SOL in and out of the object ledger as SOL.zy
//! (DECISIONS §146).
//!
//! ```text
//!   zyn-sol-bridge status     the vault, its balance, what Zyn has issued
//!   zyn-sol-bridge register   RegisterBridged for this cluster's SOL (once)
//!   zyn-sol-bridge once       credit new deposits, pay revealed exits, stop
//!   zyn-sol-bridge run        the same, every ZYN_SOL_POLL seconds
//! ```
//!
//! **In:** a transfer into the vault carrying `zyn:<commitment>` becomes a
//! `DepositCommitted`, recorded once by the transaction's signature; the
//! depositor's Nap claims it. **Out:** a `Withdraw` of SOL.zy commits to
//! `keccak(address ‖ salt)`; once its holder reveals the address
//! (`zyn_bridgeReveal`), this pays it from the vault — signed by the vault's
//! FROST custodians at their threshold, against a durable nonce so a payment
//! can be rebroadcast and never repeated — and records `SettleExit` once
//! Solana has finalized it. One payment in flight at a time.
//!
//! **Tokens** (`ZYN_SOL_MINTS`): an SPL token carried the same way — each is
//! registered under §66.13's address for `solana-devnet` and its mint,
//! deposited by a memo'd transfer into the vault's own token account, and
//! paid out by a token transfer that opens the recipient's account if it
//! has none. The network fee of a token-only payment comes from the vault's
//! SOL buffer.
//!
//! ```text
//!   ZYN_SOL_NODE         the object ledger's JSON-RPC     (http://127.0.0.1:8499)
//!   ZYN_SUBMIT_TOKEN     the operator door's token (read from the node's env)
//!   ZYN_SOL_CLUSTER      devnet | testnet                              (devnet)
//!   ZYN_SOL_RPC          Solana JSON-RPC               (https://api.devnet.solana.com)
//!   ZYN_SOL_PUBLIC       dir holding the vault's public.bin            (required)
//!   ZYN_SOL_CUSTODIANS   host:port,… of the share holders              (required)
//!   ZYN_SOL_THRESHOLD    shares needed to sign                              (2)
//!   ZYN_SOL_NONCE        the vault's durable nonce account             (required)
//!   ZYN_SOL_MINTS        SPL tokens it also carries, `mint:decimals,…`  (none)
//!   ZYN_SOL_STATE        dir for the deposit cursors and payments in flight
//!   ZYN_SOL_POLL         seconds between passes in `run`                   (20)
//! ```

use std::time::Duration;

use objectvm::bridge::asset_of;
use objectvm::{ObjectVm, Op};
use serde_json::{json, Value};
use zyn_custody::custody_net::{RemoteSolanaQuorum, SolanaPublicPackage};
use zyn_custody::solana::custody;
use zyn_custody::solana::{base58_encode, Cluster, Rpc, TxStatus};
use zyn_vm::spec::MicrochainVm;
use zyn_vm::{program, Fixed};
use zynzapd::sol_bridge::{decode_deposit, decode_token_deposit, fixed_of, fixed_of_units, lamports_of, tx_ref, units_of, NETWORK_FEE};

fn env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0).then(|| (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()).collect()).flatten()
}

struct Bridge {
    node: String,
    rpc: Rpc,
    rpc_url: String,
    network: &'static [u8],
    asset: [u8; 32],
    public: SolanaPublicPackage,
    vault: String,
    custodians: Vec<String>,
    threshold: u16,
    nonce: String,
    state: std::path::PathBuf,
    agent: ureq::Agent,
    tokens: Vec<Token>,
}

/// An SPL token this bridge carries.
struct Token {
    mint: String,
    mint_key: [u8; 32],
    decimals: u8,
    asset: [u8; 32],
    /// The vault's own token account for this mint, where deposits land.
    ata: String,
}

/// A settlement sent and not yet final. Written before it is broadcast.
#[derive(Clone)]
struct InFlight {
    signature: String,
    nonce: String,
    transaction: String,
    covers: Vec<u32>,
}

impl InFlight {
    fn json(&self) -> Value {
        json!({ "signature": self.signature, "nonce": self.nonce, "transaction": self.transaction, "covers": self.covers })
    }

    fn from(v: &Value) -> Option<InFlight> {
        Some(InFlight {
            signature: v["signature"].as_str()?.to_string(),
            nonce: v["nonce"].as_str()?.to_string(),
            transaction: v["transaction"].as_str()?.to_string(),
            covers: v["covers"].as_array()?.iter().map(|x| x.as_u64().map(|x| x as u32)).collect::<Option<_>>()?,
        })
    }
}

fn bridge() -> Result<Bridge, String> {
    let (cluster, network): (Cluster, &'static [u8]) = match env("ZYN_SOL_CLUSTER", "devnet").as_str() {
        "devnet" => (Cluster::Devnet, b"solana-devnet"),
        "testnet" => (Cluster::Testnet, b"solana-devnet"),
        other => return Err(format!("ZYN_SOL_CLUSTER is devnet or testnet here, not {other} — mainnet SOL waits for federated custody (§36.7)")),
    };
    let rpc_url = env("ZYN_SOL_RPC", "https://api.devnet.solana.com");
    let rpc = Rpc::connect(&rpc_url, cluster).map_err(|e| format!("Solana: {e:?}"))?;
    let dir = env("ZYN_SOL_PUBLIC", "");
    if dir.is_empty() {
        return Err("set ZYN_SOL_PUBLIC to the directory with the vault's public.bin".into());
    }
    let public: SolanaPublicPackage = zyn_custody::shares::load_public::<zyn_custody::ceremony::Solana>(std::path::Path::new(&dir)).map_err(|e| format!("{dir}/public.bin: {e}"))?;
    let vault = base58_encode(&custody::vault_address_of(&public));
    let custodians: Vec<String> = env("ZYN_SOL_CUSTODIANS", "").split(',').filter(|s| !s.is_empty()).map(String::from).collect();
    let nonce = env("ZYN_SOL_NONCE", "");
    if nonce.is_empty() {
        return Err("set ZYN_SOL_NONCE to the vault's durable nonce account".into());
    }
    let vault_key = custody::vault_address_of(&public);
    let mut tokens = Vec::new();
    for entry in env("ZYN_SOL_MINTS", "").split(',').filter(|s| !s.is_empty()) {
        let (mint, decimals) = entry.split_once(':').ok_or_else(|| format!("ZYN_SOL_MINTS entry {entry} is not mint:decimals"))?;
        let mint_key = zyn_custody::solana::pubkey(mint).ok_or_else(|| format!("{mint} is not a Solana address"))?;
        let decimals: u8 = decimals.parse().map_err(|_| format!("{entry}: decimals must be a number"))?;
        tokens.push(Token {
            mint: mint.to_string(),
            mint_key,
            decimals,
            asset: asset_of(network, &mint_key),
            ata: base58_encode(&zyn_bridge::solana::associated_token_address(&vault_key, &mint_key)),
        });
    }
    let state = std::path::PathBuf::from(env("ZYN_SOL_STATE", "/var/lib/zyn/sol-bridge"));
    std::fs::create_dir_all(&state).map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(Bridge {
        node: env("ZYN_SOL_NODE", "http://127.0.0.1:8499"),
        rpc,
        rpc_url,
        network,
        asset: asset_of(network, b""),
        public,
        vault,
        custodians,
        threshold: env("ZYN_SOL_THRESHOLD", "2").parse().map_err(|_| "ZYN_SOL_THRESHOLD must be a number".to_string())?,
        nonce,
        state,
        agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
        tokens,
    })
}

impl Bridge {
    fn solana(&self, method: &str, params: Value) -> Result<Value, String> {
        let r = self
            .agent
            .post(&self.rpc_url)
            .send_json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
            .map_err(|e| format!("Solana: {e}"))?;
        let v: Value = r.into_json().map_err(|_| "Solana answered something that is not JSON".to_string())?;
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            return Err(format!("Solana refused {method}: {e}"));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    fn token(&self) -> Result<String, String> {
        std::env::var("ZYN_SUBMIT_TOKEN").map_err(|_| "set ZYN_SUBMIT_TOKEN (read it from the node's env file)".to_string())
    }

    fn submit(&self, op: &Op) -> Result<String, String> {
        let r = zynzapd::object_client::call(&self.node, "zyn_submit", json!({ "token": self.token()?, "intent": hex(&ObjectVm::encode_intent(op)) }))?;
        Ok(r.get("receipts").and_then(Value::as_str).unwrap_or("").to_string())
    }

    /// New deposits since each cursor: SOL into the vault, and each token
    /// into the vault's token account. Each is posted once; the chain refuses
    /// a reference it has seen, so re-reading history is harmless.
    fn deposits(&self) -> Result<(), String> {
        self.deposits_at(&self.vault, "deposit-cursor".into(), None)?;
        for (i, t) in self.tokens.iter().enumerate() {
            self.deposits_at(&t.ata, format!("deposit-cursor-{}", t.ata), Some(i))?;
        }
        Ok(())
    }

    fn deposits_at(&self, watched: &str, cursor_name: String, token: Option<usize>) -> Result<(), String> {
        let cursor = self.state.join(cursor_name);
        let until = std::fs::read_to_string(&cursor).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let mut opts = json!({ "commitment": "finalized", "limit": 1000 });
        if let Some(u) = &until {
            opts["until"] = json!(u);
        }
        let rows = self.solana("getSignaturesForAddress", json!([watched, opts]))?;
        let rows = rows.as_array().cloned().unwrap_or_default();
        let newest = rows.first().and_then(|r| r["signature"].as_str()).map(String::from);
        for r in rows.iter().rev() {
            let Some(sig) = r["signature"].as_str() else { continue };
            if !r["err"].is_null() || !r["memo"].as_str().unwrap_or("").contains("zyn:") {
                continue;
            }
            let tx = self.solana("getTransaction", json!([sig, { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0, "commitment": "finalized" }]))?;
            let (asset, gross, commitment, what) = match token {
                None => {
                    let Some((lamports, c)) = decode_deposit(&tx, &self.vault) else { continue };
                    (self.asset, fixed_of(lamports), c, format!("{lamports} lamports"))
                }
                Some(i) => {
                    let t = &self.tokens[i];
                    let Some((units, c)) = decode_token_deposit(&tx, &t.ata, &t.mint) else { continue };
                    let Some(gross) = fixed_of_units(units, t.decimals) else { continue };
                    (t.asset, gross, c, format!("{units} units of {}", t.mint))
                }
            };
            let receipts = self.submit(&Op::DepositCommitted { program: program::deposit(), asset, gross, external_ref: tx_ref(sig), commitment })?;
            if receipts.contains("already credited") {
                continue;
            }
            if receipts.contains("Refused") {
                return Err(format!("{sig}: the ledger refused the deposit — {}", receipts.chars().take(160).collect::<String>()));
            }
            println!("in:  {sig} — {what} waiting for {}", hex(&commitment));
        }
        if let Some(n) = newest {
            std::fs::write(&cursor, n).map_err(|e| format!("{}: {e}", cursor.display()))?;
        }
        Ok(())
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
                v.as_array().into_iter().flatten().map(|x| InFlight::from(x).unwrap_or_else(|| panic!("{} is damaged", self.inflight_path().display()))).collect()
            }
        }
    }

    fn save(&self, v: &[InFlight]) -> Result<(), String> {
        let all: Vec<Value> = v.iter().map(InFlight::json).collect();
        std::fs::write(self.inflight_path(), serde_json::to_vec_pretty(&all).unwrap()).map_err(|e| e.to_string())
    }

    /// Settle what Solana has finalized; rebroadcast what it has not seen.
    fn follow_up(&self) -> Result<bool, String> {
        let mut v = self.inflight();
        if v.is_empty() {
            return Ok(false);
        }
        let mut keep = Vec::new();
        for f in v.drain(..) {
            match self.rpc.signature_status(&f.signature).map_err(|e| format!("{e:?}"))? {
                TxStatus::Finalized => {
                    for index in &f.covers {
                        let r = self.submit(&Op::SettleExit { program: program::settle_exit(), index: *index, txid: tx_ref(&f.signature) })?;
                        if r.contains("Refused") && !r.contains("already settled") {
                            eprintln!("zyn-sol-bridge: SettleExit {index} refused — {r}");
                        }
                    }
                    println!("out: {} final — {} exit(s) settled", f.signature, f.covers.len());
                }
                TxStatus::Failed => eprintln!("zyn-sol-bridge: {} failed on Solana; its exits stay unpaid", f.signature),
                TxStatus::Confirmed => keep.push(f),
                TxStatus::Unknown => {
                    let now = hex(&self.rpc.nonce_value(&self.nonce).map_err(|e| format!("{e:?}"))?);
                    if now != f.nonce {
                        eprintln!("zyn-sol-bridge: NONCE MOVED under {} without it landing — dropped; investigate before paying again", f.signature);
                    } else {
                        let tx = unhex(&f.transaction).ok_or("a stored transaction is not hex")?;
                        let _ = self.rpc.send_transaction(&tx);
                        keep.push(f);
                    }
                }
            }
        }
        let busy = !keep.is_empty();
        self.save(&keep)?;
        Ok(busy)
    }

    /// Pay the revealed exits that fit one transaction.
    fn pay(&self) -> Result<(), String> {
        // SOL first, then each token: one transaction can carry both.
        let mut payouts = Vec::new();
        let mut covers = Vec::new();
        let kinds: Vec<([u8; 32], Option<&Token>)> = std::iter::once((self.asset, None)).chain(self.tokens.iter().map(|t| (t.asset, Some(t)))).collect();
        for (asset, token) in kinds {
            let rows = zynzapd::object_client::call(&self.node, "zyn_bridgeExits", json!({ "token": self.token()?, "asset": hex(&asset) }))?;
            for r in rows.as_array().into_iter().flatten() {
                let (Some(index), Some(to)) = (r["index"].as_u64(), r["to"].as_str()) else { continue };
                let amount = Fixed(r["amount"].as_str().and_then(|a| a.parse().ok()).unwrap_or(0));
                let Some(to) = unhex(to).and_then(|b| <[u8; 32]>::try_from(b).ok()) else { continue };
                let payout = match token {
                    None => lamports_of(amount).map(|l| zyn_bridge::solana::SolPayout::native(to, l)),
                    Some(t) => units_of(amount, t.decimals).map(|u| zyn_bridge::solana::SolPayout::token(to, t.mint_key, u)),
                };
                let Some(payout) = payout else {
                    eprintln!("zyn-sol-bridge: exit {index} is not a whole number of the asset's units; it waits");
                    continue;
                };
                payouts.push(payout);
                covers.push(index as u32);
            }
        }
        if payouts.is_empty() {
            return Ok(());
        }
        let accounts = zyn_bridge::solana::VaultAccounts {
            vault: custody::vault_address_of(&self.public),
            nonce_account: zyn_custody::solana::pubkey(&self.nonce).ok_or("ZYN_SOL_NONCE is not a Solana address")?,
        };
        let mut group = zyn_bridge::solana::chunk(accounts, &payouts).map_err(|e| format!("{e:?}"))?.remove(0);
        covers.truncate(group.len());
        // A SOL exit pays the network fee, so lamports leaving the vault equal
        // what Zyn burned. A token-only payment's fee comes from the vault's
        // SOL buffer, which a plain top-up (no memo) keeps up.
        if let Some(first) = group.iter_mut().find(|p| p.token.is_none()) {
            if first.lamports <= NETWORK_FEE {
                return Err(format!("an exit cannot cover the {NETWORK_FEE}-lamport network fee"));
            }
            first.lamports -= NETWORK_FEE;
        }
        let mut quorum = RemoteSolanaQuorum::new(self.custodians.clone());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let nonce = self.rpc.nonce_value(&self.nonce).map_err(|e| format!("{e:?}"))?;
        let payment = custody::prepare_with(&self.rpc, &mut quorum, self.threshold, &self.public, &self.nonce, &group, now).map_err(|e| format!("signing: {e:?}"))?;
        let entry = InFlight { signature: payment.id(), nonce: hex(&nonce), transaction: hex(&payment.transaction), covers: covers.clone() };
        let mut v = self.inflight();
        v.push(entry);
        self.save(&v)?;
        match custody::broadcast(&self.rpc, &payment) {
            Ok(sig) => println!("out: {sig} — {} exit(s) paid, waiting for finality", covers.len()),
            Err(e) => eprintln!("zyn-sol-bridge: broadcast deferred: {e:?}"),
        }
        Ok(())
    }

    fn once(&self) -> Result<(), String> {
        self.deposits()?;
        if !self.follow_up()? {
            self.pay()?;
        }
        Ok(())
    }
}

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    let result = bridge().and_then(|b| match cmd.as_str() {
        "status" => {
            let bal = b.solana("getBalance", json!([b.vault, { "commitment": "finalized" }]))?;
            let bridged = zynzapd::object_client::call(&b.node, "zyn_bridged", json!({}))?;
            let issued = bridged.as_array().into_iter().flatten().find(|r| r["asset"].as_str() == Some(&hex(&b.asset))).map(|r| r["issued"].clone());
            println!("network  {}", String::from_utf8_lossy(b.network));
            println!("asset    {}", hex(&b.asset));
            println!("vault    {}  ({} lamports)", b.vault, bal["value"]);
            println!("issued   {}", issued.map_or("not registered — run `register`".into(), |i| format!("{i} (Fixed, 18 decimals)")));
            for t in &b.tokens {
                println!("token    {} ({} decimals) → {} · vault account {}", t.mint, t.decimals, hex(&t.asset), t.ata);
            }
            println!("in flight {}", b.inflight().len());
            Ok(())
        }
        "register" => {
            let r = b.submit(&Op::RegisterBridged { program: program::bridge_register(), network: b.network.to_vec(), origin: Vec::new(), decimals: 9 })?;
            println!("register SOL: {}", r.chars().take(120).collect::<String>());
            for t in &b.tokens {
                let r = b.submit(&Op::RegisterBridged { program: program::bridge_register(), network: b.network.to_vec(), origin: t.mint_key.to_vec(), decimals: t.decimals })?;
                println!("register {}: {}", t.mint, r.chars().take(120).collect::<String>());
            }
            Ok(())
        }
        "once" => b.once(),
        "run" => {
            let every: u64 = env("ZYN_SOL_POLL", "20").parse().unwrap_or(20);
            loop {
                if let Err(e) = b.once() {
                    eprintln!("zyn-sol-bridge: {e}");
                }
                std::thread::sleep(Duration::from_secs(every));
            }
        }
        other => Err(format!("unknown command {other}: status | register | once | run")),
    });
    if let Err(e) = result {
        eprintln!("zyn-sol-bridge {cmd}: {e}");
        std::process::exit(1);
    }
}
