//! `zyn-sol-bridge` — SOL in and out of the object ledger as SOL.zy
//! (DECISIONS §146).
//!
//! ```text
//!   zyn-sol-bridge status     the vault, its balance, what Zyn has issued
//!   zyn-sol-bridge register   RegisterBridged for this cluster's SOL (once)
//!   zyn-sol-bridge once       credit new deposits, pay revealed exits, stop
//!   zyn-sol-bridge run        the same, every ZYN_SOL_POLL seconds (and the relayer)
//!   zyn-sol-bridge relay      only the private channel's relayer (no vault needed)
//!   zyn-sol-bridge proposer-key <file>   a new keypair for Squads mode; prints its address
//! ```
//!
//! **Squads mode (§171), the only mode on mainnet:** with `ZYN_SOL_SQUADS`
//! set, the vault is that Squads v4 multisig's vault, and no key this holds
//! can move it alone. It proposes each payout batch — a vault transaction and
//! its proposal, from its own member key — after simulating the payouts as
//! the vault would make them. A key that may vote is accepted only while the
//! threshold is at least 3, so two owners are always needed besides it; it
//! approves its own proposal, and with Execute it executes once the owners
//! have approved. Otherwise the owners execute in the Squads app. It settles
//! the exits once the execution is final. A proposal the owners reject or
//! cancel is not proposed again until its record is deleted.
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
//!   ZYN_SOL_CLUSTER      devnet | testnet | mainnet                    (devnet)
//!   ZYN_SOL_RPC          Solana JSON-RPC               (https://api.devnet.solana.com)
//!   ZYN_SOL_SQUADS       the Squads multisig whose vault this is   (Squads mode)
//!   ZYN_SOL_PROPOSER     keypair file of its Initiate-only member  (Squads mode)
//!   ZYN_SOL_VAULT_INDEX  which of the multisig's vaults                     (0)
//!   ZYN_SOL_PUBLIC       dir holding the vault's public.bin    (custodians mode)
//!   ZYN_SOL_CUSTODIANS   host:port,… of the share holders      (custodians mode)
//!   ZYN_SOL_THRESHOLD    shares needed to sign                              (2)
//!   ZYN_SOL_NONCE        the vault's durable nonce account     (custodians mode)
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
use zynzapd::squads;
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
    custody: Custody,
    vault: String,
    vault_key: [u8; 32],
    state: std::path::PathBuf,
    agent: ureq::Agent,
    tokens: Vec<Token>,
    /// The pool program (base58), and each pool: its size and address.
    pool_program: Option<String>,
    pools: Vec<(u64, String)>,
    /// The pools' exit authority (testnet: one key, §158.4).
    pool_key: Option<String>,
}

/// Who signs for the vault.
enum Custody {
    /// The custodians' threshold key is the vault (testnet, §146).
    Custodians { public: SolanaPublicPackage, custodians: Vec<String>, threshold: u16, nonce: String },
    /// The vault is a Squads multisig's; this only proposes (§171).
    Squads { multisig: [u8; 32], vault_index: u8, proposer: sol_pool::Signer },
}

/// The most payouts one Squads proposal carries, so the owners can read it.
const SQUADS_BATCH: usize = 8;

/// Each cluster's genesis hash: what an endpoint must answer to be it.
fn genesis_of(cluster: Cluster) -> &'static str {
    match cluster {
        Cluster::Devnet => "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG",
        Cluster::Testnet => "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY",
        Cluster::MainnetBeta => "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d",
    }
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
        "mainnet" => (Cluster::MainnetBeta, b"solana"),
        other => return Err(format!("ZYN_SOL_CLUSTER is devnet, testnet or mainnet, not {other}")),
    };
    let rpc_url = env("ZYN_SOL_RPC", "https://api.devnet.solana.com");
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build();
    let squads_ms = env("ZYN_SOL_SQUADS", "");
    let custody = if !squads_ms.is_empty() {
        let multisig = zyn_custody::solana::pubkey(&squads_ms).ok_or_else(|| format!("ZYN_SOL_SQUADS {squads_ms} is not a Solana address"))?;
        let keyfile = env("ZYN_SOL_PROPOSER", "");
        if keyfile.is_empty() {
            return Err("Squads mode needs ZYN_SOL_PROPOSER, the keypair file of the multisig's Initiate-only member".into());
        }
        let vault_index: u8 = env("ZYN_SOL_VAULT_INDEX", "0").parse().map_err(|_| "ZYN_SOL_VAULT_INDEX must be 0-255".to_string())?;
        Custody::Squads { multisig, vault_index, proposer: sol_pool::Signer::load(&keyfile)? }
    } else {
        // The custodians' key never runs on mainnet: there the vault answers
        // to the team's multisig (§171).
        if cluster == Cluster::MainnetBeta {
            return Err("mainnet SOL runs only in Squads mode: set ZYN_SOL_SQUADS and ZYN_SOL_PROPOSER".into());
        }
        let dir = env("ZYN_SOL_PUBLIC", "");
        if dir.is_empty() {
            return Err("set ZYN_SOL_PUBLIC to the directory with the vault's public.bin".into());
        }
        let public: SolanaPublicPackage = zyn_custody::shares::load_public::<zyn_custody::ceremony::Solana>(std::path::Path::new(&dir)).map_err(|e| format!("{dir}/public.bin: {e}"))?;
        let nonce = env("ZYN_SOL_NONCE", "");
        if nonce.is_empty() {
            return Err("set ZYN_SOL_NONCE to the vault's durable nonce account".into());
        }
        Custody::Custodians {
            public,
            custodians: env("ZYN_SOL_CUSTODIANS", "").split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
            threshold: env("ZYN_SOL_THRESHOLD", "2").parse().map_err(|_| "ZYN_SOL_THRESHOLD must be a number".to_string())?,
            nonce,
        }
    };
    // The custody client refuses mainnet outright, as it should for the
    // custodians' key; Squads mode only reads and proposes, so it checks the
    // endpoint's cluster itself.
    let rpc = match &custody {
        Custody::Custodians { .. } => Rpc::connect(&rpc_url, cluster).map_err(|e| format!("Solana: {e:?}"))?,
        Custody::Squads { .. } => {
            let found = solana_call(&agent, &rpc_url, "getGenesisHash", json!([]))?;
            if found.as_str() != Some(genesis_of(cluster)) {
                return Err(format!("{rpc_url} is not Solana {} (genesis {found})", env("ZYN_SOL_CLUSTER", "devnet")));
            }
            Rpc::connect_unchecked(&rpc_url, cluster)
        }
    };
    let vault_key = match &custody {
        Custody::Custodians { public, .. } => custody::vault_address_of(public),
        Custody::Squads { multisig, vault_index, .. } => squads::vault_pda(multisig, *vault_index),
    };
    let vault = base58_encode(&vault_key);
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
        custody,
        vault,
        vault_key,
        state,
        agent,
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
                    let Custody::Custodians { nonce, .. } = &self.custody else { return Err("a custodians' payment in flight in Squads mode".into()) };
                    let now = hex(&self.rpc.nonce_value(nonce).map_err(|e| format!("{e:?}"))?);
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

    /// The revealed exits owed, as payouts, and the exits they cover — or,
    /// when a private exit's pool cannot back it yet, the pool and the size
    /// to fund it with first (§158.4).
    fn owed(&self) -> Result<(Vec<zyn_bridge::solana::SolPayout>, Vec<u32>, Option<([u8; 32], u64)>), String> {
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
        let funding = self.private_exits(&private)?;
        Ok((payouts, covers, funding))
    }

    /// Custodians' mode: pay the revealed exits that fit one transaction.
    fn pay(&self) -> Result<(), String> {
        let Custody::Custodians { public, custodians, threshold, nonce: nonce_account } = &self.custody else { return Err("not in custodians' mode".into()) };
        let (mut payouts, mut covers, funding) = self.owed()?;
        // Private exits are inserted as notes with the pools' key. One that
        // its pool cannot back yet is funded first: one size from the vault
        // into the pool, alone in its payment (it settles no exit), with the
        // network fee on top so the pool receives the size exactly.
        if let Some((pool, lamports)) = funding {
            payouts = vec![zyn_bridge::solana::SolPayout::native(pool, lamports + NETWORK_FEE)];
            covers = Vec::new();
            println!("out: funding a pool with {lamports} lamports from the vault for a private exit");
        }
        if payouts.is_empty() {
            return Ok(());
        }
        let accounts = zyn_bridge::solana::VaultAccounts {
            vault: self.vault_key,
            nonce_account: zyn_custody::solana::pubkey(nonce_account).ok_or("ZYN_SOL_NONCE is not a Solana address")?,
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
        let mut quorum = RemoteSolanaQuorum::new(custodians.clone());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let nonce = self.rpc.nonce_value(nonce_account).map_err(|e| format!("{e:?}"))?;
        let payment = custody::prepare_with(&self.rpc, &mut quorum, *threshold, public, nonce_account, &group, now).map_err(|e| format!("signing: {e:?}"))?;
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

    // ------------------------------------------------------- Squads mode

    fn account(&self, address: &str, commitment: &str) -> Result<Option<Vec<u8>>, String> {
        let r = self.solana("getAccountInfo", json!([address, { "encoding": "base64", "commitment": commitment }]))?;
        if r["value"].is_null() {
            return Ok(None);
        }
        r["value"]["data"][0].as_str().and_then(base64).map(Some).ok_or_else(|| format!("{address}: account data is not base64"))
    }

    fn multisig(&self) -> Result<squads::Multisig, String> {
        let Custody::Squads { multisig, .. } = &self.custody else { return Err("not in Squads mode".into()) };
        let raw = self.account(&base58_encode(multisig), "confirmed")?.ok_or("the Squads multisig account does not exist on this cluster")?;
        squads::Multisig::decode(&raw).ok_or_else(|| "the ZYN_SOL_SQUADS account is not a Squads v4 multisig".into())
    }

    fn proposal_path(&self) -> std::path::PathBuf {
        self.state.join("proposal.json")
    }

    /// The finalized transaction that executed a vault transaction, if any.
    fn executed_by(&self, transaction: &str) -> Result<Option<String>, String> {
        let rows = self.solana("getSignaturesForAddress", json!([transaction, { "commitment": "finalized", "limit": 50 }]))?;
        for r in rows.as_array().into_iter().flatten() {
            let Some(sig) = r["signature"].as_str() else { continue };
            if !r["err"].is_null() {
                continue;
            }
            let tx = self.solana("getTransaction", json!([sig, { "encoding": "json", "maxSupportedTransactionVersion": 0, "commitment": "finalized" }]))?;
            let ran = tx["meta"]["logMessages"].as_array().into_iter().flatten().filter_map(Value::as_str).any(|l| l.contains("Instruction: VaultTransactionExecute"));
            if ran && tx["meta"]["err"].is_null() {
                return Ok(Some(sig.to_string()));
            }
        }
        Ok(None)
    }

    /// Record a final payment's exits. False when the ledger refused one: the
    /// record is then kept, and nothing more is proposed until it is settled.
    fn settle(&self, covers: &Value, sig: &str) -> Result<bool, String> {
        let mut ok = true;
        for index in covers.as_array().into_iter().flatten().filter_map(Value::as_u64) {
            let r = self.submit(&Op::SettleExit { program: program::settle_exit(), index: index as u32, txid: tx_ref(sig) })?;
            if r.contains("Refused") && !r.contains("already settled") {
                eprintln!("zyn-sol-bridge: SETTLEMENT REFUSED — exit {index} was paid by {sig} and the ledger will not record it: {r}. Nothing more is proposed until it is.");
                ok = false;
            }
        }
        Ok(ok)
    }

    /// Run the payouts as the vault would, without signatures: owners are
    /// only asked to approve a batch the vault can pay.
    fn simulate_as_vault(&self, payer: &[u8; 32], ixs: &[sol_pool::Ix]) -> Result<(), String> {
        let msg = sol_pool::message(payer, ixs, &[0u8; 32]);
        let signers = msg[0] as usize;
        let mut tx = vec![signers as u8];
        tx.extend(std::iter::repeat(0u8).take(64 * signers));
        tx.extend_from_slice(&msg);
        let r = self.solana("simulateTransaction", json!([base64_encode(&tx), { "encoding": "base64", "sigVerify": false, "replaceRecentBlockhash": true, "commitment": "confirmed" }]))?;
        if r["value"]["err"].is_null() {
            return Ok(());
        }
        let logs: Vec<&str> = r["value"]["logs"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        Err(format!("the payouts would fail if the multisig executed them — not proposing: {} {}", r["value"]["err"], logs.join(" | ")))
    }

    /// The proposer's permissions, refused unless it may initiate and, if it
    /// may vote, two owners are still needed besides it.
    fn proposer_role(&self, ms: &squads::Multisig) -> Result<u8, String> {
        let Custody::Squads { proposer, .. } = &self.custody else { return Err("not in Squads mode".into()) };
        let me = base58_encode(&proposer.pubkey());
        match ms.permissions(&proposer.pubkey()) {
            None => Err(format!("the proposer {me} is not a member of the multisig yet")),
            Some(p) if p & squads::INITIATE == 0 => Err(format!("the proposer {me} may not initiate in the multisig")),
            Some(p) if p & squads::VOTE != 0 && ms.threshold < 3 => Err(format!("the proposer {me} may vote, so the threshold must be at least 3 (two owners besides it); it is {}", ms.threshold)),
            Some(p) => Ok(p),
        }
    }

    /// Execute an approved batch with the proposer's key. Left to the owners
    /// if it does not fit one transaction.
    fn execute(&self, index: u64, transaction: &str) -> Result<(), String> {
        let Custody::Squads { multisig, proposer, .. } = &self.custody else { return Err("not in Squads mode".into()) };
        let raw = self.account(transaction, "confirmed")?.ok_or("the approved batch's transaction account is gone")?;
        let keys = squads::vault_transaction_keys(&raw).ok_or("the approved batch does not decode as this bridge's")?;
        let bh = blockhash(&self.agent, &self.rpc_url)?;
        let (sig, tx) = proposer.transaction(&[sol_pool::compute_limit(400_000), squads::vault_transaction_execute(multisig, index, &proposer.pubkey(), &keys)], &bh);
        if tx.len() > zyn_bridge::solana::MAX_TRANSACTION_BYTES {
            eprintln!("zyn-sol-bridge: batch {index} is too large for this bridge to execute; an owner executes it in Squads");
            return Ok(());
        }
        match send_raw(&self.agent, &self.rpc_url, &tx) {
            Ok(_) => println!("out: batch {index} approved — executed by the bridge ({sig}), waiting for finality"),
            Err(e) => eprintln!("zyn-sol-bridge: executing batch {index}: {e}"),
        }
        Ok(())
    }

    /// Squads mode: propose a payout batch to the multisig, or follow the one
    /// proposed until its owners execute (or reject) it.
    fn pay_squads(&self) -> Result<(), String> {
        let Custody::Squads { multisig, vault_index, proposer } = &self.custody else { return Err("not in Squads mode".into()) };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Ok(raw) = std::fs::read(self.proposal_path()) {
            // A damaged record is not "nothing proposed": that would propose again.
            let p: Value = serde_json::from_slice(&raw).unwrap_or_else(|_| panic!("{} is damaged — fix it before proposing again", self.proposal_path().display()));
            let index = p["index"].as_u64().ok_or("proposal.json has no index")?;
            let sig = p["sig"].as_str().unwrap_or("").to_string();
            let transaction = base58_encode(&squads::transaction_pda(multisig, index));
            let proposal = self.account(&base58_encode(&squads::proposal_pda(multisig, index)), "confirmed")?;
            let Some((_, status)) = proposal.as_deref().and_then(squads::decode_proposal) else {
                // Not on chain: the proposal is made in the same transaction
                // as the batch, so neither landed — or both did and the
                // accounts were closed after execution.
                match self.rpc.signature_status(&sig).map_err(|e| format!("{e:?}"))? {
                    TxStatus::Failed => {
                        eprintln!("zyn-sol-bridge: proposing batch {index} failed on Solana ({sig}); proposing again");
                        std::fs::remove_file(self.proposal_path()).map_err(|e| e.to_string())?;
                    }
                    // Unseen after its blockhash expired, it can no longer land.
                    TxStatus::Unknown if now > p["at"].as_u64().unwrap_or(0) + 180 => {
                        eprintln!("zyn-sol-bridge: batch {index} ({sig}) never landed; proposing again");
                        std::fs::remove_file(self.proposal_path()).map_err(|e| e.to_string())?;
                    }
                    TxStatus::Finalized => match self.executed_by(&transaction)? {
                        Some(paid) => {
                            if self.settle(&p["covers"], &paid)? {
                                println!("out: {paid} executed by the multisig and final — {} exit(s) settled", p["covers"].as_array().map_or(0, Vec::len));
                                std::fs::remove_file(self.proposal_path()).map_err(|e| e.to_string())?;
                            }
                        }
                        None => eprintln!("zyn-sol-bridge: batch {index}'s proposal account is gone and no execution of it is on chain — investigate before proposing again"),
                    },
                    _ => {} // landing
                }
                return Ok(());
            };
            match status {
                squads::Status::Executed => {
                    let Some(paid) = self.executed_by(&transaction)? else { return Ok(()) }; // executed; waiting for finality
                    if self.settle(&p["covers"], &paid)? {
                        println!("out: {paid} executed by the multisig and final — {} exit(s) settled", p["covers"].as_array().map_or(0, Vec::len));
                        std::fs::remove_file(self.proposal_path()).map_err(|e| e.to_string())?;
                    }
                }
                squads::Status::Rejected | squads::Status::Cancelled => {
                    eprintln!("zyn-sol-bridge: the owners {} batch {index}; its exits wait. Delete {} to propose them again.", if status == squads::Status::Rejected { "rejected" } else { "cancelled" }, self.proposal_path().display());
                }
                squads::Status::Approved => {
                    // Once the owners' votes are in, the bridge executes if
                    // it may — at most once a minute, so a slow landing is
                    // not sent twice over.
                    let ms = self.multisig()?;
                    if self.proposer_role(&ms).map(|p| p & squads::EXECUTE != 0).unwrap_or(false) && now > p["executed_at"].as_u64().unwrap_or(0) + 60 {
                        let mut p = p.clone();
                        p["executed_at"] = json!(now);
                        std::fs::write(self.proposal_path(), serde_json::to_vec_pretty(&p).unwrap()).map_err(|e| e.to_string())?;
                        self.execute(index, &transaction)?;
                    }
                }
                _ => {
                    if index <= self.multisig()?.stale_transaction_index {
                        eprintln!("zyn-sol-bridge: batch {index} went stale (the multisig's settings changed); its exits wait. Delete {} to propose them again.", self.proposal_path().display());
                    }
                }
            }
            return Ok(());
        }

        let (mut payouts, mut covers, funding) = self.owed()?;
        // A pool that cannot back a private exit is funded first, alone. The
        // executing owner pays the network fee, so the pool gets the size.
        if let Some((pool, lamports)) = funding {
            payouts = vec![zyn_bridge::solana::SolPayout::native(pool, lamports)];
            covers = Vec::new();
        }
        if payouts.is_empty() {
            return Ok(());
        }
        let ms = self.multisig()?;
        let me = proposer.pubkey();
        let role = self.proposer_role(&ms)?;
        let index = ms.transaction_index + 1;
        payouts.truncate(SQUADS_BATCH);
        covers.truncate(payouts.len());
        let bh = blockhash(&self.agent, &self.rpc_url)?;
        // The batch and its proposal go in one transaction; fewer payouts
        // until it fits.
        let (ixs, sig, tx) = loop {
            let ixs = squads::payout_ixs(&self.vault_key, &payouts)?;
            let message = squads::transaction_message(&self.vault_key, &ixs)?;
            let memo = format!("Zyn bridge: pay out {} withdrawal(s)", payouts.len());
            let create = squads::vault_transaction_create(multisig, index, &me, *vault_index, &message, &memo);
            let propose = squads::proposal_create(multisig, index, &me);
            let mut ixs_out = vec![create, propose];
            // A voting proposer counts its own vote at once: the owners'
            // two are what remain.
            if role & squads::VOTE != 0 {
                ixs_out.push(squads::proposal_approve(multisig, index, &me));
            }
            let (sig, tx) = proposer.transaction(&ixs_out, &bh);
            if tx.len() <= zyn_bridge::solana::MAX_TRANSACTION_BYTES {
                break (ixs, sig, tx);
            }
            if payouts.len() == 1 {
                return Err("one payout does not fit a proposal".into());
            }
            payouts.pop();
            covers.truncate(payouts.len());
        };
        self.simulate_as_vault(&me, &ixs)?;
        let record = json!({ "index": index, "covers": covers, "sig": sig, "at": now, "transaction": base58_encode(&squads::transaction_pda(multisig, index)) });
        std::fs::write(self.proposal_path(), serde_json::to_vec_pretty(&record).unwrap()).map_err(|e| e.to_string())?;
        match send_raw(&self.agent, &self.rpc_url, &tx) {
            Ok(_) => println!("out: {} withdrawal(s) proposed to the multisig as transaction {index} ({sig}) — waiting for its owners", covers.len()),
            // Refused at preflight: nothing went out, so nothing is recorded.
            Err(e) if e.starts_with("Solana refused") => {
                std::fs::remove_file(self.proposal_path()).map_err(|e| e.to_string())?;
                return Err(format!("proposing batch {index}: {e}"));
            }
            Err(e) => eprintln!("zyn-sol-bridge: proposal {sig} may not have gone out ({e}); following it"),
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
        match self.custody {
            Custody::Squads { .. } => self.pay_squads(),
            Custody::Custodians { .. } => {
                if !self.follow_up()? {
                    self.pay()?;
                }
                Ok(())
            }
        }
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
    if cmd == "proposer-key" {
        if let Err(e) = proposer_key(std::env::args().nth(2)) {
            eprintln!("zyn-sol-bridge proposer-key: {e}");
            std::process::exit(1);
        }
        return;
    }
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
            match &b.custody {
                Custody::Custodians { .. } => println!("in flight {}", b.inflight().len()),
                Custody::Squads { multisig, vault_index, proposer } => {
                    let ms = b.multisig()?;
                    println!("custody  Squads multisig {} (vault {vault_index}), {} of {} owners, time lock {}s", base58_encode(multisig), ms.threshold, ms.voters(), ms.time_lock);
                    for (k, perms) in &ms.members {
                        let can: Vec<&str> = [(squads::INITIATE, "initiate"), (squads::VOTE, "vote"), (squads::EXECUTE, "execute")].iter().filter(|(bit, _)| perms & bit != 0).map(|(_, n)| *n).collect();
                        println!("         member {} — {}", base58_encode(k), can.join(", "));
                    }
                    let me = proposer.pubkey();
                    let bal = b.solana("getBalance", json!([base58_encode(&me), { "commitment": "confirmed" }]))?;
                    let role = match (b.proposer_role(&ms), ms.permissions(&me)) {
                        (Err(e), _) => format!("NOT READY — {e}"),
                        (Ok(p), _) if p & squads::VOTE != 0 && p & squads::EXECUTE != 0 => "ready — proposes, approves its own, executes once two owners approve".to_string(),
                        (Ok(p), _) if p & squads::VOTE != 0 => "ready — proposes and approves its own; owners approve and execute".to_string(),
                        (Ok(_), _) => "ready — proposes; owners approve and execute".to_string(),
                    };
                    println!("proposer {} ({} lamports for proposal rent): {role}", base58_encode(&me), bal["value"]);
                    println!("next     transaction {}", ms.transaction_index + 1);
                    match std::fs::read_to_string(b.proposal_path()) {
                        Ok(p) => println!("proposed {}", p.split_whitespace().collect::<Vec<_>>().join(" ")),
                        Err(_) => println!("proposed nothing waiting"),
                    }
                }
            }
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
        other => Err(format!("unknown command {other}: status | register | once | run | relay | proposer-key <file>")),
    });
    if let Err(e) = result {
        eprintln!("zyn-sol-bridge {cmd}: {e}");
        std::process::exit(1);
    }
}

/// A new keypair file (Solana CLI format, owner-only) for the Squads
/// proposer. Refuses to overwrite: a key already added to the multisig must
/// not be replaced by accident.
fn proposer_key(path: Option<String>) -> Result<(), String> {
    use rand::RngCore as _;
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let path = path.ok_or("proposer-key <file>")?;
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let key = ed25519_dalek::SigningKey::from_bytes(&secret);
    let mut bytes = secret.to_vec();
    bytes.extend_from_slice(&key.verifying_key().to_bytes());
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(|e| format!("{path}: {e}"))?;
    f.write_all(serde_json::to_string(&bytes).unwrap().as_bytes()).map_err(|e| format!("{path}: {e}"))?;
    println!("{}", base58_encode(&key.verifying_key().to_bytes()));
    Ok(())
}
