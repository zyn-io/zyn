//! `zyn-evm-pool` — the private channel's service on one EVM chain
//! (DECISIONS §158): ETH.zy through the Sepolia pools, mzBTC.zy through the
//! Mezo pools. One process per chain.
//!
//! ```text
//!   zyn-evm-pool status     the pools, their balances and live notes
//!   zyn-evm-pool register   RegisterBridged for this chain's coin (once)
//!   zyn-evm-pool once       credit spends into Zyn, insert private exits, stop
//!   zyn-evm-pool run        the same every ZYN_EVM_POOL_POLL seconds, and the relayer
//! ```
//!
//! **In:** `spendToZyn` in a pool emits `ToZyn(nullifier hash, code)`; each
//! final one becomes a `DepositCommitted` of the pool's size under `code`,
//! recorded once by chain, pool and nullifier hash. The coin stays in the
//! pool: it now backs the bridged asset.
//!
//! **Out:** a withdrawal whose revealed destination is a pool note
//! (`"zyn-pool" ‖ size ‖ commitment`, 48 bytes) is inserted into that size's
//! pool with `insertExit` — the pool itself refuses one it could not pay. A
//! pool short of free backing is topped up from a sibling pool's with
//! `sweep`, which can only move what no live note is owed; failing that, the
//! exit waits and this says how much the pool needs. Once the insert is
//! final, `SettleExit` records it. Before inserting, the pool's own `Exit`
//! logs are checked, so a lost state file never inserts a note twice.
//!
//! **Relayer:** `zyn_poolInfo`, `zyn_poolLeaves` and `zyn_poolRelay` on
//! ZYN_EVM_POOL_RELAY, which the node forwards by `chain`. It submits spends
//! so no wallet of the holder's pays for one: estimated first, so a spend
//! the pool would refuse costs nothing and says why.
//!
//! ```text
//!   ZYN_EVM_POOL_NODE      the object ledger's JSON-RPC         (http://127.0.0.1:8499)
//!   ZYN_SUBMIT_TOKEN       the operator door's token (read from the node's env)
//!   ZYN_EVM_POOL_RPC       the chain's JSON-RPC                              (required)
//!   ZYN_EVM_POOL_CHAIN_ID  the chain                                        (required)
//!   ZYN_EVM_POOL_POOLS     size:address,… (size in wei)                     (required)
//!   ZYN_EVM_POOL_FROM      the block the pools were deployed at              (required)
//!   ZYN_EVM_POOL_KEY       file with the hex key: exit authority and relayer (required)
//!   ZYN_EVM_POOL_FEE       the relayer's fee on a spend to an address, wei       (0)
//!   ZYN_EVM_POOL_FINALITY  block tag spends and inserts must reach       (finalized)
//!   ZYN_EVM_POOL_SPAN      blocks per eth_getLogs                             (5000)
//!   ZYN_EVM_POOL_STATE     dir for cursors, notes and transactions in flight
//!   ZYN_EVM_POOL_POLL      seconds between passes in `run`                      (20)
//!   ZYN_EVM_POOL_RELAY     where the relayer listens                (127.0.0.1:8496)
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use k256::ecdsa::SigningKey;
use objectvm::bridge::asset_of;
use objectvm::{ObjectVm, Op};
use serde_json::{json, Value};
use zyn_vm::spec::MicrochainVm;
use zyn_vm::{program, Fixed};
use zynzapd::eth_bridge::{self as eth, Address20, Tx};
use zynzapd::evm_pool::{self as pool, Event};
use zynzapd::sol_bridge::as_pool_destination;
use zynzapd::sol_pool::Leaves;

fn env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim().trim_start_matches("0x");
    (s.len() % 2 == 0).then(|| (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()).collect()).flatten()
}

fn qty(v: &Value) -> Option<u128> {
    u128::from_str_radix(v.as_str()?.trim_start_matches("0x"), 16).ok()
}

fn hex32(v: &Value) -> Option<[u8; 32]> {
    unhex(v.as_str()?).and_then(|b| b.try_into().ok())
}

fn addr(s: &str) -> Option<Address20> {
    unhex(s).and_then(|b| b.try_into().ok())
}

fn size_of(v: &Value) -> Option<u128> {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64().map(u128::from))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The chain, its pools and the one key that inserts exits and relays
/// spends. Shared by the pass and the relayer, which take turns sending
/// (one account, one nonce sequence).
struct Chain {
    node: String,
    rpc: String,
    chain_id: u64,
    network: Vec<u8>,
    asset: [u8; 32],
    pools: Vec<(u128, Address20)>,
    from: u64,
    span: u64,
    key: SigningKey,
    me: Address20,
    fee: u128,
    finality: String,
    state: std::path::PathBuf,
    agent: ureq::Agent,
    sending: Mutex<()>,
}

fn chain() -> Result<Chain, String> {
    let rpc = env("ZYN_EVM_POOL_RPC", "");
    if rpc.is_empty() {
        return Err("set ZYN_EVM_POOL_RPC to the chain's JSON-RPC".into());
    }
    let chain_id: u64 = env("ZYN_EVM_POOL_CHAIN_ID", "").parse().map_err(|_| "set ZYN_EVM_POOL_CHAIN_ID".to_string())?;
    if chain_id == 1 {
        return Err("the private channel is testnet only until its keys come from a ceremony (§158.8)".into());
    }
    let mut pools = Vec::new();
    for p in env("ZYN_EVM_POOL_POOLS", "").split(',').filter(|s| !s.is_empty()) {
        let (size, a) = p.split_once(':').ok_or_else(|| format!("ZYN_EVM_POOL_POOLS entry {p} is not size:address"))?;
        let size: u128 = size.parse().map_err(|_| format!("{size} is not a size in wei"))?;
        if size > u64::MAX as u128 {
            return Err(format!("a {size}-wei pool cannot be named in a 48-byte destination"));
        }
        pools.push((size, addr(a).ok_or_else(|| format!("{a} is not a 0x address"))?));
    }
    if pools.is_empty() {
        return Err("set ZYN_EVM_POOL_POOLS to size:address,…".into());
    }
    let keyfile = env("ZYN_EVM_POOL_KEY", "");
    let raw = std::fs::read_to_string(&keyfile).map_err(|e| format!("ZYN_EVM_POOL_KEY {keyfile}: {e}"))?;
    let key = unhex(&raw).filter(|b| b.len() == 32).and_then(|b| SigningKey::from_slice(&b).ok()).ok_or("ZYN_EVM_POOL_KEY is not a 32-byte hex key")?;
    let network = format!("eip155:{chain_id}").into_bytes();
    let state = std::path::PathBuf::from(env("ZYN_EVM_POOL_STATE", &format!("/var/lib/zyn/evm-pool-{chain_id}")));
    std::fs::create_dir_all(&state).map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(Chain {
        node: env("ZYN_EVM_POOL_NODE", "http://127.0.0.1:8499"),
        rpc,
        chain_id,
        asset: asset_of(&network, b""),
        network,
        pools,
        from: env("ZYN_EVM_POOL_FROM", "").parse().map_err(|_| "set ZYN_EVM_POOL_FROM to the pools' deploy block".to_string())?,
        span: env("ZYN_EVM_POOL_SPAN", "5000").parse().unwrap_or(5000).max(1),
        me: eth::address_of(&key),
        key,
        fee: env("ZYN_EVM_POOL_FEE", "0").parse().map_err(|_| "ZYN_EVM_POOL_FEE must be wei".to_string())?,
        finality: env("ZYN_EVM_POOL_FINALITY", "finalized"),
        state,
        agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
        sending: Mutex::new(()),
    })
}

impl Chain {
    fn eth(&self, method: &str, params: Value) -> Result<Value, String> {
        let r = self.agent.post(&self.rpc).send_json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })).map_err(|e| format!("{}: {e}", self.name()))?;
        let v: Value = r.into_json().map_err(|_| "the chain answered something that is not JSON".to_string())?;
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            let why = e["message"].as_str().map_or_else(|| e.to_string(), String::from);
            return Err(format!("{} refused {method}: {why}{}", self.name(), e["data"].as_str().map(|d| format!(" ({d})")).unwrap_or_default()));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    fn name(&self) -> String {
        String::from_utf8_lossy(&self.network).into_owned()
    }

    fn block(&self, tag: &str) -> Result<u64, String> {
        let b = self.eth("eth_getBlockByNumber", json!([tag, false]))?;
        qty(&b["number"]).map(|n| n as u64).ok_or_else(|| format!("no {tag} block"))
    }

    fn balance(&self, a: &Address20) -> Result<u128, String> {
        qty(&self.eth("eth_getBalance", json!([format!("0x{}", hex(a)), "latest"]))?).ok_or_else(|| "no balance".to_string())
    }

    fn live(&self, pool: &Address20) -> Result<u128, String> {
        let r = self.eth("eth_call", json!([{ "to": format!("0x{}", hex(pool)), "data": format!("0x{}", hex(&pool::live_call())) }, "latest"]))?;
        let b = unhex(r.as_str().unwrap_or("")).filter(|b| b.len() == 32).ok_or("live() answered oddly")?;
        Ok(u128::from_be_bytes(b[16..].try_into().unwrap()))
    }

    /// A pool's yes-or-no view.
    fn ask(&self, pool: &Address20, data: &[u8]) -> Result<bool, String> {
        let r = self.eth("eth_call", json!([{ "to": format!("0x{}", hex(pool)), "data": format!("0x{}", hex(data)) }, "latest"]))?;
        let b = unhex(r.as_str().unwrap_or("")).filter(|b| b.len() == 32).ok_or("the pool answered oddly")?;
        Ok(b[31] == 1)
    }

    /// What a pool holds beyond what its live notes are owed.
    fn free(&self, size: u128, pool: &Address20) -> Result<u128, String> {
        Ok(self.balance(pool)?.saturating_sub(size * self.live(pool)?))
    }

    /// One pool's logs over `[from, to]`, optionally only `topics`.
    fn logs(&self, pool: &Address20, from: u64, to: u64, topics: Value) -> Result<Vec<(Event, String, u64)>, String> {
        let mut out = Vec::new();
        let mut at = from;
        while at <= to {
            let end = (at + self.span - 1).min(to);
            let rows = self.eth("eth_getLogs", json!([{ "address": format!("0x{}", hex(pool)), "fromBlock": format!("0x{at:x}"), "toBlock": format!("0x{end:x}"), "topics": topics }]))?;
            for l in rows.as_array().into_iter().flatten() {
                if l["removed"].as_bool() == Some(true) {
                    continue;
                }
                let topics: Vec<[u8; 32]> = l["topics"].as_array().into_iter().flatten().filter_map(|t| hex32(t)).collect();
                let data = unhex(l["data"].as_str().unwrap_or("")).unwrap_or_default();
                if let Some(e) = pool::decode(&topics, &data) {
                    out.push((e, l["transactionHash"].as_str().unwrap_or("").to_string(), qty(&l["blockNumber"]).unwrap_or(0) as u64));
                }
            }
            at = end + 1;
        }
        Ok(out)
    }

    fn token(&self) -> Result<String, String> {
        std::env::var("ZYN_SUBMIT_TOKEN").map_err(|_| "set ZYN_SUBMIT_TOKEN (read it from the node's env file)".to_string())
    }

    fn submit(&self, op: &Op) -> Result<String, String> {
        let r = zynzapd::object_client::call(&self.node, "zyn_submit", json!({ "token": self.token()?, "intent": hex(&ObjectVm::encode_intent(op)) }))?;
        Ok(r.get("receipts").and_then(Value::as_str).unwrap_or("").to_string())
    }

    /// Estimate, sign and send one call from the service key; the estimate
    /// is the preflight: a call the contract would refuse is never sent.
    fn send(&self, to: &Address20, data: Vec<u8>) -> Result<String, String> {
        let _turn = self.sending.lock().unwrap_or_else(|p| p.into_inner());
        let from = format!("0x{}", hex(&self.me));
        let gas = qty(&self.eth("eth_estimateGas", json!([{ "from": from, "to": format!("0x{}", hex(to)), "data": format!("0x{}", hex(&data)) }]))?).ok_or("no gas estimate")?;
        let price = qty(&self.eth("eth_gasPrice", json!([]))?).ok_or("no gas price")?;
        let nonce = qty(&self.eth("eth_getTransactionCount", json!([from, "pending"]))?).ok_or("no account nonce")?;
        let raw = Tx { nonce: nonce as u64, gas_price: (price * 12 / 10).max(1), gas: (gas * 13 / 10) as u64, to: *to, value: 0, data, chain_id: self.chain_id }.signed(&self.key)?;
        let hash = format!("0x{}", hex(&zyn_vm::eip712::keccak(&[&raw])));
        self.eth("eth_sendRawTransaction", json!([format!("0x{}", hex(&raw))]))?;
        Ok(hash)
    }

    fn pool(&self, size: Option<u128>) -> Result<(u128, Address20), String> {
        let size = size.ok_or("give the pool's size in wei")?;
        self.pools.iter().find(|(s, _)| *s == size).copied().ok_or_else(|| format!("no {size}-wei pool on {}", self.name()))
    }

    // ------------------------------------------------------------ in

    /// Spends into Zyn, once each is final.
    fn spends(&self) -> Result<(), String> {
        let head = self.block(&self.finality)?;
        for (size, p) in &self.pools {
            let cursor = self.state.join(format!("spend-cursor-{}", hex(p)));
            let from: u64 = std::fs::read_to_string(&cursor).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(self.from);
            if from > head {
                continue;
            }
            for (e, tx, _) in self.logs(p, from, head, json!([format!("0x{}", hex(&pool::to_zyn_topic()))]))? {
                let Event::ToZyn { nullifier_hash, code } = e else { continue };
                let r = self.submit(&Op::DepositCommitted {
                    program: program::deposit(),
                    asset: self.asset,
                    gross: Fixed(*size as i128),
                    external_ref: pool::spend_ref(self.chain_id, p, &nullifier_hash),
                    commitment: code,
                })?;
                if r.contains("already credited") {
                    continue;
                }
                if r.contains("Refused") {
                    return Err(format!("{tx}: the ledger refused a private deposit — {}", r.chars().take(160).collect::<String>()));
                }
                println!("in:  {tx} — private {size} wei waiting for {}", hex(&code));
            }
            std::fs::write(&cursor, (head + 1).to_string()).map_err(|e| format!("{}: {e}", cursor.display()))?;
        }
        Ok(())
    }

    // ------------------------------------------------------------ out

    fn sent_path(&self) -> std::path::PathBuf {
        self.state.join("exits-sent.json")
    }

    /// The insert of this note already on chain, if any: `(tx, block)`.
    fn inserted(&self, p: &Address20, commitment: &[u8; 32]) -> Result<Option<(String, u64)>, String> {
        let head = self.block("latest")?;
        let hit = self.logs(p, self.from, head, json!([format!("0x{}", hex(&pool::exit_topic())), format!("0x{}", hex(commitment))]))?;
        Ok(hit.into_iter().map(|(_, tx, b)| (tx, b)).next())
    }

    fn exits(&self) -> Result<(), String> {
        let rows = zynzapd::object_client::call(&self.node, "zyn_bridgeExits", json!({ "token": self.token()?, "asset": hex(&self.asset) }))?;
        let mut sent: serde_json::Map<String, Value> = std::fs::read(self.sent_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let final_block = self.block(&self.finality)?;
        let mut swept = false;
        // One insert per pool in flight: a second, estimated against a state
        // without the first, could revert for want of backing.
        let mut busy: Vec<Address20> = Vec::new();
        for r in rows.as_array().into_iter().flatten() {
            let (Some(index), Some(to)) = (r["index"].as_u64(), r["to"].as_str()) else { continue };
            // Only pool notes: 20-byte addresses are the transparent bridge's.
            let Some((size, commitment)) = unhex(to).and_then(|b| as_pool_destination(&b)) else { continue };
            let size = size as u128;
            let Some(&(_, p)) = self.pools.iter().find(|(s, _)| *s == size) else {
                eprintln!("zyn-evm-pool: private exit {index} names a {size}-wei pool {} does not have; it waits", self.name());
                continue;
            };
            if r["amount"].as_str().and_then(|a| a.parse::<u128>().ok()) != Some(size) {
                eprintln!("zyn-evm-pool: private exit {index} is not exactly one {size}-wei note; it waits");
                continue;
            }
            let key = index.to_string();
            if let Some((tx, block)) = self.inserted(&p, &commitment)? {
                if block > final_block {
                    continue;
                }
                let txid: [u8; 32] = unhex(&tx).and_then(|b| b.try_into().ok()).ok_or("a transaction hash is not 32 bytes")?;
                let r = self.submit(&Op::SettleExit { program: program::settle_exit(), index: index as u32, txid })?;
                if r.contains("Refused") && !r.contains("already settled") {
                    eprintln!("zyn-evm-pool: SettleExit {index} refused — {r}");
                } else {
                    println!("out: private exit {index} final — note in the {size}-wei pool ({tx})");
                }
                sent.remove(&key);
                continue;
            }
            // Sent and not yet in: wait while it is pending; try again once
            // it reverted or went missing.
            if let Some(entry) = sent.get(&key) {
                let tx = entry["tx"].as_str().unwrap_or("").to_string();
                let receipt = self.eth("eth_getTransactionReceipt", json!([tx]))?;
                if receipt.is_null() {
                    let known = !self.eth("eth_getTransactionByHash", json!([tx]))?.is_null();
                    if known || now() < entry["at"].as_u64().unwrap_or(0) + 600 {
                        busy.push(p);
                        continue;
                    }
                } else if qty(&receipt["status"]) == Some(1) {
                    busy.push(p);
                    continue; // in a block; its Exit log shows on the next pass
                } else {
                    eprintln!("zyn-evm-pool: inserting private exit {index} reverted ({tx}); it is tried again");
                }
                sent.remove(&key);
            }
            if busy.contains(&p) {
                continue;
            }
            // The pool would refuse an insert it could not pay; ask it first
            // (not every chain says why a call reverts).
            if self.free(size, &p)? < size {
                if !swept {
                    swept = true;
                    self.rebalance(size, &p)?;
                }
                continue;
            }
            match self.send(&p, pool::insert_exit_call(&commitment)) {
                Ok(tx) => {
                    println!("out: private exit {index} — inserting its note into the {size}-wei pool ({tx})");
                    sent.insert(key, json!({ "tx": tx, "at": now() }));
                    busy.push(p);
                }
                Err(e) => eprintln!("zyn-evm-pool: inserting private exit {index}: {e}"),
            }
            std::fs::write(self.sent_path(), Value::Object(sent.clone()).to_string()).map_err(|e| e.to_string())?;
        }
        std::fs::write(self.sent_path(), Value::Object(sent).to_string()).map_err(|e| e.to_string())
    }

    /// Move one size of free backing into `to` from a sibling pool that has
    /// it; else say what `to` needs.
    fn rebalance(&self, size: u128, to: &Address20) -> Result<(), String> {
        // One sweep at a time: the last lands before another is weighed.
        let last = self.state.join("last-sweep");
        if let Some(tx) = std::fs::read_to_string(&last).ok().filter(|t| !t.trim().is_empty()) {
            if self.eth("eth_getTransactionReceipt", json!([tx.trim()]))?.is_null() && !self.eth("eth_getTransactionByHash", json!([tx.trim()]))?.is_null() {
                return Ok(());
            }
        }
        for (s, p) in &self.pools {
            if p == to {
                continue;
            }
            if self.free(*s, p)? >= size {
                let tx = self.send(p, pool::sweep_call(to, size))?;
                std::fs::write(&last, &tx).map_err(|e| e.to_string())?;
                println!("out: moving {size} wei of free backing from the {s}-wei pool into the {size}-wei pool ({tx})");
                return Ok(());
            }
        }
        eprintln!("zyn-evm-pool: the {size}-wei pool 0x{} needs {size} wei more to back a private exit; no sibling pool has it free — send it from the vault", hex(to));
        Ok(())
    }

    fn once(&self) -> Result<(), String> {
        self.spends()?;
        self.exits()
    }

    // ------------------------------------------------------------ relayer

    /// A pool's notes in order and its spent nullifier hashes, from its logs.
    fn leaves(&self, p: &Address20) -> Result<Leaves, String> {
        let path = self.state.join(format!("pool-leaves-{}.json", hex(p)));
        let mut l = Leaves::load(&path);
        let from = l.newest.as_deref().and_then(|n| n.parse().ok()).unwrap_or(self.from);
        let head = self.block("latest")?;
        if from > head {
            return Ok(l);
        }
        for (e, _, _) in self.logs(p, from, head, Value::Null)? {
            match e {
                Event::Note { commitment, index, .. } => l.place(index as usize, hex(&commitment))?,
                Event::ToZyn { nullifier_hash, .. } | Event::ToAddress { nullifier_hash } => {
                    let nh = hex(&nullifier_hash);
                    if !l.spent.contains(&nh) {
                        l.spent.push(nh);
                    }
                }
            }
        }
        l.newest = Some((head + 1).to_string());
        l.save(&path)?;
        Ok(l)
    }

    fn handle(&self, body: &Value) -> Result<Value, String> {
        match body["method"].as_str().unwrap_or("") {
            "zyn_poolInfo" => Ok(json!({
                "chain": self.name(),
                "relayer": eth::checksummed(&self.me),
                "fee": self.fee.to_string(),
                "pools": self.pools.iter().map(|(s, a)| json!({ "size": s.to_string(), "address": eth::checksummed(a) })).collect::<Vec<_>>(),
            })),
            "zyn_poolLeaves" => {
                let (size, p) = self.pool(size_of(&body["size"]))?;
                let l = self.leaves(&p)?;
                Ok(json!({ "size": size.to_string(), "leaves": l.leaves, "spent": l.spent }))
            }
            "zyn_poolRelay" => {
                let (_, p) = self.pool(size_of(&body["size"]))?;
                let proof: [u8; 256] = body["proof"].as_str().and_then(unhex).and_then(|b| b.try_into().ok()).ok_or("proof: 256 bytes of hex")?;
                let root = hex32(&body["root"]).ok_or("root: 32 bytes of hex")?;
                let nh = hex32(&body["nullifier_hash"]).ok_or("nullifier_hash: 32 bytes of hex")?;
                let data = match body["kind"].as_str() {
                    Some("to_zyn") => pool::spend_to_zyn_call(&proof, &root, &nh, &hex32(&body["code"]).ok_or("code: 32 bytes of hex")?),
                    Some("to_address") => {
                        let recipient = body["recipient"].as_str().and_then(addr).ok_or("recipient: a 0x address")?;
                        let fee = size_of(&body["fee"]).ok_or("fee: wei")?;
                        if fee < self.fee {
                            return Err(format!("the relayer's fee is {} wei", self.fee));
                        }
                        pool::spend_to_address_call(&proof, &root, &nh, &recipient, &self.me, fee)
                    }
                    _ => return Err("kind: to_zyn or to_address".into()),
                };
                // Not every chain says why a call reverts: ask the pool the
                // two things a holder can fix before sending.
                if self.ask(&p, &pool::spent_call(&nh))? {
                    return Err("the pool refused it: that note is already spent".into());
                }
                if !self.ask(&p, &pool::known_root_call(&root))? {
                    return Err("the pool refused it: the pool does not know that root; refresh the notes and try again".into());
                }
                let tx = self.send(&p, data).map_err(|e| format!("the pool refused it: {}", pool::refusal(&e).map_or_else(|| e.chars().take(300).collect::<String>(), String::from)))?;
                println!("relay: {tx}");
                Ok(json!({ "signature": tx }))
            }
            other => Err(format!("no pool method {other}")),
        }
    }

    fn serve(self: Arc<Self>, at: &str) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = match std::net::TcpListener::bind(at) {
            Ok(l) => l,
            Err(e) => return eprintln!("zyn-evm-pool: the relayer cannot listen on {at}: {e}"),
        };
        println!("relay: {} listening on {at} as {}", self.name(), eth::checksummed(&self.me));
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

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    let result = chain().and_then(|c| match cmd.as_str() {
        "status" => {
            println!("network  {}", c.name());
            println!("asset    {}", hex(&c.asset));
            println!("service  {} ({} wei for gas)", eth::checksummed(&c.me), c.balance(&c.me)?);
            for (s, p) in &c.pools {
                println!("pool     {s} wei · {} · {} wei held, {} live note(s)", eth::checksummed(p), c.balance(p)?, c.live(p)?);
            }
            let bridged = zynzapd::object_client::call(&c.node, "zyn_bridged", json!({}))?;
            let issued = bridged.as_array().into_iter().flatten().find(|r| r["asset"].as_str() == Some(&hex(&c.asset))).map(|r| r["issued"].as_str().unwrap_or("?").to_string());
            println!("issued   {}", issued.map_or("not registered — run `register`".into(), |i| format!("{i} wei")));
            Ok(())
        }
        "register" => {
            let r = c.submit(&Op::RegisterBridged { program: program::bridge_register(), network: c.network.clone(), origin: Vec::new(), decimals: 18 })?;
            println!("register: {}", r.chars().take(160).collect::<String>());
            Ok(())
        }
        "once" => c.once(),
        "relay" => {
            let at = env("ZYN_EVM_POOL_RELAY", "127.0.0.1:8496");
            Arc::new(c).serve(&at);
            Ok(())
        }
        "run" => {
            let c = Arc::new(c);
            let relay = c.clone();
            let at = env("ZYN_EVM_POOL_RELAY", "127.0.0.1:8496");
            std::thread::spawn(move || relay.serve(&at));
            let every: u64 = env("ZYN_EVM_POOL_POLL", "20").parse().unwrap_or(20);
            loop {
                if let Err(e) = c.once() {
                    eprintln!("zyn-evm-pool: {e}");
                }
                std::thread::sleep(Duration::from_secs(every));
            }
        }
        other => Err(format!("unknown command {other}: status | register | once | relay | run")),
    });
    if let Err(e) = result {
        eprintln!("zyn-evm-pool {cmd}: {e}");
        std::process::exit(1);
    }
}
