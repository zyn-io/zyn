//! `zyn-sol-bridge` — SOL in and out of the object ledger as SOL.zy
//! (DECISIONS §146).
//!
//! ```text
//!   zyn-sol-bridge status     the vault, its balance, what Zyn has issued
//!   zyn-sol-bridge register   RegisterBridged for this cluster's SOL (once)
//!   zyn-sol-bridge once       credit new deposits, pay revealed exits, stop
//!   zyn-sol-bridge run        the same, every ZYN_SOL_POLL seconds (and the relayer)
//!   zyn-sol-bridge relay      only the private channel's relayer (no vault needed)
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
//!   ZYN_POOL_PROGRAM     the private channel's pool program            (none: off)
//!   ZYN_POOL_SIZES       its pools, in lamports    (100000000,1000000000,10000000000)
//!   ZYN_POOL_KEY         keypair file: the pools' exit authority and the relayer
//!   ZYN_POOL_RELAY       where the relayer listens                (127.0.0.1:8497)
//!   ZYN_POOL_FEE         the relayer's fee for a spend to an address (1000000)
//! ```
//!
//! **Private channel** (§158): a spend into Zyn from one of the pools — a
//! zero-knowledge proof of *one of* its notes, bound to a deposit code —
//! becomes a `DepositCommitted` of the pool's size, referenced by its
//! nullifier hash so it is credited once. The SOL stays in the pool. A
//! private exit names a note instead of an address (`sol_bridge::
//! pool_destination`); this inserts it into its pool — first moving one size
//! from the vault into the pool when the pool's own reserve cannot back it —
//! and settles the exit once the insert is final. The relayer (a thread of
//! `run`, reached through the node's `zyn_pool*` methods) serves the pools'
//! notes and submits spends, paying their fees, so no wallet of the holder's
//! appears beside a private spend.

use std::time::Duration;

use objectvm::bridge::asset_of;
use objectvm::{ObjectVm, Op};
use serde_json::{json, Value};
use zyn_custody::custody_net::{RemoteSolanaQuorum, SolanaPublicPackage};
use zyn_custody::solana::custody;
use zyn_custody::solana::{base58_encode, Cluster, Rpc, TxStatus};
use zyn_vm::spec::MicrochainVm;
use zyn_vm::{program, Fixed};
use zynzapd::sol_pool;
use zynzapd::sol_bridge::{as_pool_destination, base64, base64_encode, decode_deposit, decode_token_deposit, fixed_of, fixed_of_units, lamports_of, pool_address, pool_events, tx_ref, units_of, NETWORK_FEE};

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
    /// The pool program (base58), and each pool: its size and address.
    pool_program: Option<String>,
    pools: Vec<(u64, String)>,
    /// The pools' exit authority (testnet: one key, §158.4).
    pool_key: Option<String>,
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
    let pool_program = Some(env("ZYN_POOL_PROGRAM", "")).filter(|p| !p.is_empty());
    let mut pools = Vec::new();
    if let Some(p) = &pool_program {
        let key = zyn_custody::solana::pubkey(p).ok_or_else(|| format!("ZYN_POOL_PROGRAM {p} is not a Solana address"))?;
        for size in env("ZYN_POOL_SIZES", "100000000,1000000000,10000000000").split(',').filter(|s| !s.is_empty()) {
            let lamports: u64 = size.parse().map_err(|_| format!("ZYN_POOL_SIZES entry {size} is not lamports"))?;
            pools.push((lamports, base58_encode(&pool_address(&key, lamports))));
        }
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
        pool_program,
        pools,
        pool_key: Some(env("ZYN_POOL_KEY", "")).filter(|k| !k.is_empty()),
    })
}

fn solana_call(agent: &ureq::Agent, url: &str, method: &str, params: Value) -> Result<Value, String> {
    let r = agent.post(url).send_json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })).map_err(|e| format!("Solana: {e}"))?;
    let v: Value = r.into_json().map_err(|_| "Solana answered something that is not JSON".to_string())?;
    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
        return Err(format!("Solana refused {method}: {e}"));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

fn blockhash(agent: &ureq::Agent, url: &str) -> Result<[u8; 32], String> {
    let r = solana_call(agent, url, "getLatestBlockhash", json!([{ "commitment": "finalized" }]))?;
    let h = r["value"]["blockhash"].as_str().ok_or("Solana gave no blockhash")?;
    zyn_custody::solana::pubkey(h).ok_or_else(|| "Solana's blockhash is not base58".into())
}

fn send_raw(agent: &ureq::Agent, url: &str, tx: &[u8]) -> Result<String, String> {
    let b64 = base64_encode(tx);
    let r = solana_call(agent, url, "sendTransaction", json!([b64, { "encoding": "base64", "preflightCommitment": "confirmed" }]))?;
    r.as_str().map(String::from).ok_or_else(|| "Solana did not return a signature".into())
}


impl Bridge {
    fn solana(&self, method: &str, params: Value) -> Result<Value, String> {
        solana_call(&self.agent, &self.rpc_url, method, params)
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
        self.pool_spends()
    }

    /// Spends into Zyn from the private pools since each pool's cursor.
    fn pool_spends(&self) -> Result<(), String> {
        let Some(program) = &self.pool_program else { return Ok(()) };
        for (lamports, pool) in &self.pools {
            let cursor = self.state.join(format!("pool-cursor-{pool}"));
            let until = std::fs::read_to_string(&cursor).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            let mut opts = json!({ "commitment": "finalized", "limit": 1000 });
            if let Some(u) = &until {
                opts["until"] = json!(u);
            }
            let rows = self.solana("getSignaturesForAddress", json!([pool, opts]))?;
            let rows = rows.as_array().cloned().unwrap_or_default();
            let newest = rows.first().and_then(|r| r["signature"].as_str()).map(String::from);
            for r in rows.iter().rev() {
                let Some(sig) = r["signature"].as_str() else { continue };
                if !r["err"].is_null() {
                    continue;
                }
                let tx = self.solana("getTransaction", json!([sig, { "encoding": "json", "maxSupportedTransactionVersion": 0, "commitment": "finalized" }]))?;
                for ev in pool_events(&tx, program) {
                    if ev.len() != 3 || ev[0] != b"to_zyn" {
                        continue;
                    }
                    let (Ok(nh), Ok(code)) = (<[u8; 32]>::try_from(ev[1].as_slice()), <[u8; 32]>::try_from(ev[2].as_slice())) else { continue };
                    let external_ref = zyn_vm::eip712::keccak(&[b"zyn-pool.to-zyn.v1", &nh[..]]);
                    let receipts = self.submit(&Op::DepositCommitted { program: program::deposit(), asset: self.asset, gross: fixed_of(*lamports), external_ref, commitment: code })?;
                    if receipts.contains("already credited") {
                        continue;
                    }
                    if receipts.contains("Refused") {
                        return Err(format!("{sig}: the ledger refused a private deposit — {}", receipts.chars().take(160).collect::<String>()));
                    }
                    println!("in:  {sig} — private {lamports} lamports waiting for {}", hex(&code));
                }
            }
            if let Some(n) = newest {
                std::fs::write(&cursor, n).map_err(|e| format!("{}: {e}", cursor.display()))?;
            }
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
                    // Kept until the ledger has recorded every exit it paid:
                    // forgotten while one still reads as owed, it is paid again.
                    let mut refused = false;
                    for index in &f.covers {
                        let r = self.submit(&Op::SettleExit { program: program::settle_exit(), index: *index, txid: tx_ref(&f.signature) })?;
                        if r.contains("Refused") && !r.contains("already settled") {
                            eprintln!("zyn-sol-bridge: SETTLEMENT REFUSED — exit {index} was paid by {} and the ledger will not record it: {r}. Nothing more is paid until it is.", f.signature);
                            refused = true;
                        }
                    }
                    if refused {
                        keep.push(f);
                    } else {
                        println!("out: {} final — {} exit(s) settled", f.signature, f.covers.len());
                    }
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
        let mut private = Vec::new();
        let kinds: Vec<([u8; 32], Option<&Token>)> = std::iter::once((self.asset, None)).chain(self.tokens.iter().map(|t| (t.asset, Some(t)))).collect();
        for (asset, token) in kinds {
            let rows = zynzapd::object_client::call(&self.node, "zyn_bridgeExits", json!({ "token": self.token()?, "asset": hex(&asset) }))?;
            for r in rows.as_array().into_iter().flatten() {
                let (Some(index), Some(to)) = (r["index"].as_u64(), r["to"].as_str()) else { continue };
                let amount = Fixed(r["amount"].as_str().and_then(|a| a.parse().ok()).unwrap_or(0));
                let Some(to) = unhex(to) else { continue };
                // A private exit names a note, not an address (§158.4).
                if token.is_none() {
                    if let Some((size, commitment)) = as_pool_destination(&to) {
                        private.push((index as u32, amount, size, commitment));
                        continue;
                    }
                }
                let Ok(to) = <[u8; 32]>::try_from(to) else { continue };
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
        // Private exits are inserted as notes with the pools' key. One that
        // its pool cannot back yet is funded first: one size from the vault
        // into the pool, alone in its payment (it settles no exit), with the
        // network fee on top so the pool receives the size exactly.
        if let Some((pool, lamports)) = self.private_exits(&private)? {
            payouts = vec![zyn_bridge::solana::SolPayout::native(pool, lamports + NETWORK_FEE)];
            covers = Vec::new();
            println!("out: funding a pool with {lamports} lamports from the vault for a private exit");
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

    /// Insert each private exit's note once its pool can back it; settle it
    /// once the insert is final. Returns a pool that needs funding first.
    fn private_exits(&self, list: &[(u32, Fixed, u64, [u8; 32])]) -> Result<Option<([u8; 32], u64)>, String> {
        if list.is_empty() {
            return Ok(None);
        }
        let (Some(program), Some(keyfile)) = (&self.pool_program, &self.pool_key) else {
            eprintln!("zyn-sol-bridge: {} private exit(s) wait: set ZYN_POOL_PROGRAM and ZYN_POOL_KEY", list.len());
            return Ok(None);
        };
        let signer = sol_pool::Signer::load(keyfile)?;
        let program_key = zyn_custody::solana::pubkey(program).ok_or("ZYN_POOL_PROGRAM is not a Solana address")?;
        let path = self.state.join("pool-exits.json");
        let mut sent: serde_json::Map<String, Value> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let rent = self.solana("getMinimumBalanceForRentExemption", json!([sol_pool::POOL_STATE_LEN]))?.as_u64().ok_or("no rent figure")?;
        let mut funding = None;
        for (index, amount, size, commitment) in list {
            let key = index.to_string();
            if let Some(entry) = sent.get(&key).cloned() {
                let sig = entry["sig"].as_str().unwrap_or("").to_string();
                match self.rpc.signature_status(&sig).map_err(|e| format!("{e:?}"))? {
                    TxStatus::Finalized => {
                        let r = self.submit(&Op::SettleExit { program: program::settle_exit(), index: *index, txid: tx_ref(&sig) })?;
                        if r.contains("Refused") && !r.contains("already settled") {
                            // Kept, so the note is not inserted a second time.
                            eprintln!("zyn-sol-bridge: SETTLEMENT REFUSED — private exit {index} is in the pool ({sig}) and the ledger will not record it: {r}");
                        } else {
                            println!("out: private exit {index} final — note in the pool ({sig})");
                            sent.remove(&key);
                        }
                    }
                    TxStatus::Failed => {
                        eprintln!("zyn-sol-bridge: inserting private exit {index} failed on Solana; it is tried again");
                        sent.remove(&key);
                    }
                    // A transaction unseen after its blockhash has expired
                    // can no longer land: only then is it tried again.
                    TxStatus::Unknown if now > entry["at"].as_u64().unwrap_or(0) + 180 => {
                        sent.remove(&key);
                    }
                    _ => {}
                }
                continue;
            }
            let Some((_, pool)) = self.pools.iter().find(|(l, _)| l == size) else {
                eprintln!("zyn-sol-bridge: private exit {index} names a {size}-lamport pool this bridge does not have; it waits");
                continue;
            };
            if *amount != fixed_of(*size) {
                eprintln!("zyn-sol-bridge: private exit {index} is not exactly one {size}-lamport note; it waits");
                continue;
            }
            let pool_key = zyn_custody::solana::pubkey(pool).ok_or("a pool address is not base58")?;
            let acct = self.solana("getAccountInfo", json!([pool, { "encoding": "base64", "commitment": "confirmed" }]))?;
            let lamports = acct["value"]["lamports"].as_u64().ok_or("the pool account is missing")?;
            let data = acct["value"]["data"][0].as_str().and_then(base64).ok_or("the pool account has no data")?;
            let live = sol_pool::live_notes(&data).ok_or("the pool account is too short")?;
            let owed = rent as u128 + *size as u128 * live as u128;
            if (lamports as u128) < owed + *size as u128 {
                if funding.is_none() {
                    funding = Some((pool_key, *size));
                }
                continue;
            }
            let bh = blockhash(&self.agent, &self.rpc_url)?;
            let (sig, tx) = signer.transaction(&[sol_pool::insert_exit(&program_key, &signer.pubkey(), &pool_key, commitment)], &bh);
            sent.insert(key.clone(), json!({ "sig": sig, "at": now }));
            std::fs::write(&path, Value::Object(sent.clone()).to_string()).map_err(|e| e.to_string())?;
            match send_raw(&self.agent, &self.rpc_url, &tx) {
                Ok(_) => println!("out: private exit {index} — inserting its note into the {size}-lamport pool ({sig})"),
                Err(e) => {
                    eprintln!("zyn-sol-bridge: inserting private exit {index}: {e}");
                    sent.remove(&key);
                }
            }
        }
        std::fs::write(&path, Value::Object(sent).to_string()).map_err(|e| e.to_string())?;
        Ok(funding)
    }

    fn once(&self) -> Result<(), String> {
        self.deposits()?;
        if !self.follow_up()? {
            self.pay()?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The relayer (§158.6): its own thread and Solana client, local only. The
// node forwards `zyn_poolInfo`, `zyn_poolLeaves` and `zyn_poolRelay` here.

struct Relay {
    agent: ureq::Agent,
    rpc_url: String,
    program: String,
    program_key: [u8; 32],
    pools: Vec<(u64, String)>,
    signer: sol_pool::Signer,
    fee: u64,
    state: std::path::PathBuf,
}

fn size_of(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn hex32(v: &Value) -> Option<[u8; 32]> {
    unhex(v.as_str()?).and_then(|b| b.try_into().ok())
}

impl Relay {
    fn solana(&self, method: &str, params: Value) -> Result<Value, String> {
        solana_call(&self.agent, &self.rpc_url, method, params)
    }

    fn pool(&self, size: Option<u64>) -> Result<(u64, String), String> {
        let size = size.ok_or("give the pool's size in lamports")?;
        self.pools.iter().find(|(l, _)| *l == size).cloned().ok_or_else(|| format!("no {size}-lamport pool"))
    }

    /// A pool's notes in order, brought up to date from its logs.
    /// A pool's notes, brought up to date; when the chain's RPC refuses
    /// (public endpoints rate-limit), the last copy saved — a pool's notes
    /// only ever grow, so an older copy is a true prefix of the current one.
    fn leaves(&self, pool: &str) -> Result<sol_pool::Leaves, String> {
        self.refresh(pool).or_else(|e| {
            let path = self.state.join(format!("pool-leaves-v2-{pool}.json"));
            if path.exists() {
                eprintln!("relay: answering from the saved notes ({e})");
                Ok(sol_pool::Leaves::load(&path))
            } else {
                Err(e)
            }
        })
    }

    fn refresh(&self, pool: &str) -> Result<sol_pool::Leaves, String> {
        // v2: with spent nullifier hashes (a v1 file is rebuilt from the start).
        let path = self.state.join(format!("pool-leaves-v2-{pool}.json"));
        let mut l = sol_pool::Leaves::load(&path);
        let mut rows: Vec<Value> = Vec::new();
        let mut before: Option<String> = None;
        loop {
            let mut opts = json!({ "commitment": "confirmed", "limit": 1000 });
            if let Some(n) = &l.newest {
                opts["until"] = json!(n);
            }
            if let Some(b) = &before {
                opts["before"] = json!(b);
            }
            let page = self.solana("getSignaturesForAddress", json!([pool, opts]))?;
            let page = page.as_array().cloned().unwrap_or_default();
            let full = page.len() == 1000;
            before = page.last().and_then(|r| r["signature"].as_str()).map(String::from);
            rows.extend(page);
            if !full {
                break;
            }
        }
        let newest = rows.first().and_then(|r| r["signature"].as_str()).map(String::from);
        for r in rows.iter().rev() {
            let Some(sig) = r["signature"].as_str() else { continue };
            if !r["err"].is_null() {
                continue;
            }
            let tx = self.solana("getTransaction", json!([sig, { "encoding": "json", "maxSupportedTransactionVersion": 0, "commitment": "confirmed" }]))?;
            for ev in pool_events(&tx, &self.program) {
                if ev.len() == 4 && (ev[0] == b"deposit" || ev[0] == b"exit") {
                    let index = u32::from_le_bytes(ev[2].as_slice().try_into().map_err(|_| "a malformed pool event")?) as usize;
                    l.place(index, hex(&ev[1]))?;
                } else if ev.len() >= 2 && (ev[0] == b"to_zyn" || ev[0] == b"to_address") {
                    let nh = hex(&ev[1]);
                    if !l.spent.contains(&nh) {
                        l.spent.push(nh);
                    }
                }
            }
        }
        if newest.is_some() {
            l.newest = newest;
        }
        // Saved even when empty, so a refused refresh has a copy to answer from.
        l.save(&path)?;
        Ok(l)
    }

    fn handle(&self, body: &Value) -> Result<Value, String> {
        match body["method"].as_str().unwrap_or("") {
            "zyn_poolInfo" => Ok(json!({
                "program": self.program,
                "relayer": base58_encode(&self.signer.pubkey()),
                "fee": self.fee,
                "pools": self.pools.iter().map(|(l, a)| json!({ "size": l, "address": a })).collect::<Vec<_>>(),
            })),
            "zyn_poolLeaves" => {
                let (size, pool) = self.pool(size_of(&body["size"]))?;
                let l = self.leaves(&pool)?;
                Ok(json!({ "size": size, "leaves": l.leaves, "spent": l.spent }))
            }
            "zyn_poolRelay" => {
                let (_, pool) = self.pool(size_of(&body["size"]))?;
                let pool_key = zyn_custody::solana::pubkey(&pool).ok_or("bad pool address")?;
                let proof: [u8; 256] = body["proof"].as_str().and_then(unhex).and_then(|b| b.try_into().ok()).ok_or("proof: 256 bytes of hex")?;
                let root = hex32(&body["root"]).ok_or("root: 32 bytes of hex")?;
                let nh = hex32(&body["nullifier_hash"]).ok_or("nullifier_hash: 32 bytes of hex")?;
                let to = match body["kind"].as_str() {
                    Some("to_zyn") => sol_pool::SpendTo::Zyn { code: hex32(&body["code"]).ok_or("code: 32 bytes of hex")? },
                    Some("to_address") => {
                        let recipient = body["recipient"].as_str().and_then(zyn_custody::solana::pubkey).ok_or("recipient: a Solana address")?;
                        let fee = size_of(&body["fee"]).ok_or("fee: lamports")?;
                        if fee < self.fee {
                            return Err(format!("the relayer's fee is {} lamports", self.fee));
                        }
                        sol_pool::SpendTo::Address { recipient, fee }
                    }
                    _ => return Err("kind: to_zyn or to_address".into()),
                };
                let ix = sol_pool::spend(&self.program_key, &self.signer.pubkey(), &pool_key, &proof, &root, &nh, &to);
                let bh = blockhash(&self.agent, &self.rpc_url)?;
                let (sig, tx) = self.signer.transaction(&[sol_pool::compute_limit(400_000), ix], &bh);
                // Preflight runs the program first: a spend it refuses costs
                // the relayer nothing and says why.
                send_raw(&self.agent, &self.rpc_url, &tx).map_err(|e| format!("the pool refused it: {}", e.chars().take(300).collect::<String>()))?;
                println!("relay: {sig}");
                Ok(json!({ "signature": sig }))
            }
            other => Err(format!("no pool method {other}")),
        }
    }

    fn serve(self, addr: &str) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = match std::net::TcpListener::bind(addr) {
            Ok(l) => l,
            Err(e) => return eprintln!("zyn-sol-bridge: the relayer cannot listen on {addr}: {e}"),
        };
        println!("relay: listening on {addr} as {}", base58_encode(&self.signer.pubkey()));
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let mut reader = BufReader::new(&stream);
            let mut len = 0usize;
            let mut line = String::new();
            let mut ok = reader.read_line(&mut line).is_ok();
            while ok {
                line.clear();
                if reader.read_line(&mut line).map(|n| n == 0).unwrap_or(true) || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len.min(64 * 1024)];
            ok &= reader.read_exact(&mut body).is_ok();
            let (status, text) = match serde_json::from_slice::<Value>(&body) {
                Ok(v) if ok => match self.handle(&v) {
                    Ok(r) => ("200 OK", r.to_string()),
                    Err(e) => ("400 Bad Request", e),
                },
                _ => ("400 Bad Request", "a JSON request".into()),
            };
            let mut s = &stream;
            let _ = write!(s, "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}", text.len());
        }
    }
}

/// The relayer from the environment alone: the pools and their key, no vault.
fn relay_from_env() -> Result<Relay, String> {
    let program = env("ZYN_POOL_PROGRAM", "");
    let program_key = zyn_custody::solana::pubkey(&program).ok_or("set ZYN_POOL_PROGRAM to the pool program")?;
    let mut pools = Vec::new();
    for size in env("ZYN_POOL_SIZES", "100000000,1000000000,10000000000").split(',').filter(|s| !s.is_empty()) {
        let lamports: u64 = size.parse().map_err(|_| format!("ZYN_POOL_SIZES entry {size} is not lamports"))?;
        pools.push((lamports, base58_encode(&pool_address(&program_key, lamports))));
    }
    let state = std::path::PathBuf::from(env("ZYN_SOL_STATE", "/var/lib/zyn/sol-bridge"));
    std::fs::create_dir_all(&state).map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(Relay {
        agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
        rpc_url: env("ZYN_SOL_RPC", "https://api.devnet.solana.com"),
        program,
        program_key,
        pools,
        signer: sol_pool::Signer::load(&env("ZYN_POOL_KEY", ""))?,
        fee: env("ZYN_POOL_FEE", "1000000").parse().map_err(|_| "ZYN_POOL_FEE must be lamports".to_string())?,
        state,
    })
}

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    if cmd == "relay" {
        match relay_from_env() {
            Ok(r) => r.serve(&env("ZYN_POOL_RELAY", "127.0.0.1:8497")),
            Err(e) => {
                eprintln!("zyn-sol-bridge relay: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
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
            if let Some(p) = &b.pool_program {
                println!("pools    program {p}");
                for (lamports, pool) in &b.pools {
                    let bal = b.solana("getBalance", json!([pool, { "commitment": "finalized" }]))?;
                    println!("         {} SOL · {pool} ({} lamports)", *lamports as f64 / 1e9, bal["value"]);
                }
            }
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
            if let (Some(program), Some(keyfile)) = (b.pool_program.clone(), b.pool_key.clone()) {
                let relay = Relay {
                    agent: b.agent.clone(),
                    rpc_url: b.rpc_url.clone(),
                    program_key: zyn_custody::solana::pubkey(&program).ok_or("ZYN_POOL_PROGRAM is not a Solana address")?,
                    program,
                    pools: b.pools.clone(),
                    signer: sol_pool::Signer::load(&keyfile)?,
                    fee: env("ZYN_POOL_FEE", "1000000").parse().map_err(|_| "ZYN_POOL_FEE must be lamports".to_string())?,
                    state: b.state.clone(),
                };
                let addr = env("ZYN_POOL_RELAY", "127.0.0.1:8497");
                std::thread::spawn(move || relay.serve(&addr));
            }
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
