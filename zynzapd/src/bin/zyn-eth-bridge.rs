//! `zyn-eth-bridge` — ETH in and out of the object ledger as ETH.zy
//! (DECISIONS §146).
//!
//! ```text
//!   zyn-eth-bridge status     the vault, its balance, what Zyn has issued
//!   zyn-eth-bridge register   RegisterBridged for this chain's ETH (once)
//!   zyn-eth-bridge once       credit new deposits, pay revealed exits, stop
//!   zyn-eth-bridge run        the same, every ZYN_ETH_POLL seconds
//! ```
//!
//! **In:** `ZynVault.depositNative(commitment, asset)` emits `Deposited`;
//! each finalized one becomes a `DepositCommitted`, recorded once by the
//! vault's own per-asset index. **Out:** a `Withdraw` of ETH.zy commits to
//! `keccak(address ‖ salt)`; once revealed, this signs the vault's withdrawal
//! digest with `threshold` of the `ZynSigners` owners' keys — after checking
//! the contract computes the same digest — sends it, and records `SettleExit`
//! with the transaction hash once it is final. One payment in flight at a time.
//!
//! ```text
//!   ZYN_ETH_NODE        the object ledger's JSON-RPC       (http://127.0.0.1:8499)
//!   ZYN_SUBMIT_TOKEN    the operator door's token (read from the node's env)
//!   ZYN_ETH_RPC         Ethereum JSON-RPC                                (required)
//!   ZYN_ETH_CHAIN_ID    the chain                                   (11155111, Sepolia)
//!   ZYN_ETH_VAULT       the ZynVault address                             (required)
//!   ZYN_ETH_ASSET_ID    the vault's id for native ETH                          (1)
//!   ZYN_ETH_KEYS        files, each one owner's hex key, comma-separated (required)
//!   ZYN_ETH_THRESHOLD   owners needed                                          (2)
//!   ZYN_ETH_PAYER       file with the hex key that pays gas                (required)
//!   ZYN_ETH_FINALITY    block tag deposits and payments must reach  (finalized)
//!   ZYN_ETH_STATE       dir for the block cursor and payments in flight
//!   ZYN_ETH_POLL        seconds between passes in `run`                        (20)
//! ```

use std::time::Duration;

use k256::ecdsa::SigningKey;
use objectvm::bridge::asset_of;
use objectvm::{ObjectVm, Op};
use serde_json::{json, Value};
use zyn_vm::spec::MicrochainVm;
use zyn_vm::{program, Fixed};
use zynzapd::eth_bridge::{self as eth, Payout, Tx};

/// Payouts per withdrawal transaction.
const PER_TX: usize = 8;

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

fn key_from(path: &str) -> Result<SigningKey, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let b = unhex(&raw).filter(|b| b.len() == 32).ok_or_else(|| format!("{path} is not a 32-byte hex key"))?;
    SigningKey::from_slice(&b).map_err(|e| format!("{path}: {e}"))
}

struct Bridge {
    node: String,
    rpc: String,
    chain_id: u64,
    vault: eth::Address20,
    vault_asset: u32,
    network: Vec<u8>,
    asset: [u8; 32],
    keys: Vec<SigningKey>,
    threshold: usize,
    payer: SigningKey,
    finality: String,
    state: std::path::PathBuf,
    agent: ureq::Agent,
}

fn bridge() -> Result<Bridge, String> {
    let rpc = env("ZYN_ETH_RPC", "");
    if rpc.is_empty() {
        return Err("set ZYN_ETH_RPC to an Ethereum JSON-RPC endpoint".into());
    }
    let chain_id: u64 = env("ZYN_ETH_CHAIN_ID", "11155111").parse().map_err(|_| "ZYN_ETH_CHAIN_ID must be a number".to_string())?;
    if chain_id == 1 {
        return Err("ETH mainnet waits for federated signers (§146); this bridge runs on testnets".into());
    }
    let vault: eth::Address20 = unhex(&env("ZYN_ETH_VAULT", "")).and_then(|b| b.try_into().ok()).ok_or("set ZYN_ETH_VAULT to the vault's 0x address")?;
    let keys = env("ZYN_ETH_KEYS", "").split(',').filter(|s| !s.is_empty()).map(key_from).collect::<Result<Vec<_>, _>>()?;
    let payer = key_from(&env("ZYN_ETH_PAYER", ""))?;
    let network = format!("eip155:{chain_id}").into_bytes();
    let state = std::path::PathBuf::from(env("ZYN_ETH_STATE", "/var/lib/zyn/eth-bridge"));
    std::fs::create_dir_all(&state).map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(Bridge {
        node: env("ZYN_ETH_NODE", "http://127.0.0.1:8499"),
        rpc,
        chain_id,
        vault,
        vault_asset: env("ZYN_ETH_ASSET_ID", "1").parse().map_err(|_| "ZYN_ETH_ASSET_ID must be a number".to_string())?,
        asset: asset_of(&network, b""),
        network,
        keys,
        threshold: env("ZYN_ETH_THRESHOLD", "2").parse().map_err(|_| "ZYN_ETH_THRESHOLD must be a number".to_string())?,
        payer,
        finality: env("ZYN_ETH_FINALITY", "finalized"),
        state,
        agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
    })
}

impl Bridge {
    fn eth(&self, method: &str, params: Value) -> Result<Value, String> {
        let r = self.agent.post(&self.rpc).send_json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })).map_err(|e| format!("Ethereum: {e}"))?;
        let v: Value = r.into_json().map_err(|_| "Ethereum answered something that is not JSON".to_string())?;
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            return Err(format!("Ethereum refused {method}: {e}"));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    fn call(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        let r = self.eth("eth_call", json!([{ "to": format!("0x{}", hex(&self.vault)), "data": format!("0x{}", hex(data)) }, "latest"]))?;
        unhex(r.as_str().unwrap_or("")).ok_or_else(|| "eth_call answered something that is not hex".to_string())
    }

    fn final_block(&self) -> Result<u64, String> {
        let b = self.eth("eth_getBlockByNumber", json!([self.finality, false]))?;
        qty(&b["number"]).map(|n| n as u64).ok_or_else(|| format!("no {} block", self.finality))
    }

    fn token(&self) -> Result<String, String> {
        std::env::var("ZYN_SUBMIT_TOKEN").map_err(|_| "set ZYN_SUBMIT_TOKEN (read it from the node's env file)".to_string())
    }

    fn submit(&self, op: &Op) -> Result<String, String> {
        let r = zynzapd::object_client::call(&self.node, "zyn_submit", json!({ "token": self.token()?, "intent": hex(&ObjectVm::encode_intent(op)) }))?;
        Ok(r.get("receipts").and_then(Value::as_str).unwrap_or("").to_string())
    }

    fn deposits(&self) -> Result<(), String> {
        let cursor = self.state.join("block-cursor");
        let head = self.final_block()?;
        let mut from: u64 = std::fs::read_to_string(&cursor).ok().and_then(|s| s.trim().parse().ok()).unwrap_or_else(|| head.saturating_sub(5_000));
        while from <= head {
            let to = (from + 4_999).min(head);
            let logs = self.eth(
                "eth_getLogs",
                json!([{ "address": format!("0x{}", hex(&self.vault)), "fromBlock": format!("0x{from:x}"), "toBlock": format!("0x{to:x}"), "topics": [format!("0x{}", hex(&eth::deposited_topic()))] }]),
            )?;
            for l in logs.as_array().into_iter().flatten() {
                let topics: Vec<[u8; 32]> = l["topics"].as_array().into_iter().flatten().filter_map(|t| unhex(t.as_str()?)?.try_into().ok()).collect();
                let data = unhex(l["data"].as_str().unwrap_or("")).unwrap_or_default();
                let Some((commitment, asset, wad, index)) = eth::decode_deposited(&topics, &data) else { continue };
                if asset != self.vault_asset {
                    continue;
                }
                let r = self.submit(&Op::DepositCommitted {
                    program: program::deposit(),
                    asset: self.asset,
                    gross: Fixed(wad as i128),
                    external_ref: eth::deposit_ref(self.chain_id, &self.vault, asset, index),
                    commitment,
                })?;
                if r.contains("already credited") {
                    continue;
                }
                if r.contains("Refused") {
                    return Err(format!("deposit {index}: the ledger refused it — {}", r.chars().take(160).collect::<String>()));
                }
                println!("in:  deposit {index} — {wad} wei waiting for {}", hex(&commitment));
            }
            from = to + 1;
            std::fs::write(&cursor, from.to_string()).map_err(|e| format!("{}: {e}", cursor.display()))?;
        }
        Ok(())
    }

    fn inflight_path(&self) -> std::path::PathBuf {
        self.state.join("inflight.json")
    }

    fn inflight(&self) -> Option<Value> {
        // A damaged record is not "nothing in flight": that would pay again.
        std::fs::read(self.inflight_path()).ok().map(|b| serde_json::from_slice(&b).unwrap_or_else(|_| panic!("{} is damaged — fix it before paying again", self.inflight_path().display())))
    }

    fn vault_nonce(&self) -> Result<u64, String> {
        let r = self.call(&eth::nonce_call())?;
        (r.len() == 32).then(|| u64::from_be_bytes(r[24..].try_into().unwrap())).ok_or_else(|| "nonce() answered oddly".to_string())
    }

    /// Settle a final payment; rebroadcast one Ethereum has not seen.
    fn follow_up(&self) -> Result<bool, String> {
        let Some(f) = self.inflight() else { return Ok(false) };
        let tx = f["tx"].as_str().unwrap_or("").to_string();
        let receipt = self.eth("eth_getTransactionReceipt", json!([tx]))?;
        if receipt.is_null() {
            if self.vault_nonce()? > f["vault_nonce"].as_u64().unwrap_or(u64::MAX) {
                eprintln!("zyn-eth-bridge: THE VAULT NONCE MOVED without {tx} — dropped; investigate before paying again");
                let _ = std::fs::remove_file(self.inflight_path());
                return Ok(false);
            }
            let _ = self.eth("eth_sendRawTransaction", json!([f["raw"]]));
            return Ok(true);
        }
        let block = qty(&receipt["blockNumber"]).unwrap_or(u128::MAX) as u64;
        if block > self.final_block()? {
            return Ok(true);
        }
        if qty(&receipt["status"]) != Some(1) {
            eprintln!("zyn-eth-bridge: {tx} reverted; its exits stay unpaid");
        } else {
            let txid: [u8; 32] = unhex(&tx).and_then(|b| b.try_into().ok()).ok_or("a stored hash is not 32 bytes")?;
            for index in f["covers"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
                let r = self.submit(&Op::SettleExit { program: program::settle_exit(), index: index as u32, txid })?;
                if r.contains("Refused") && !r.contains("already settled") {
                    eprintln!("zyn-eth-bridge: SettleExit {index} refused — {r}");
                }
            }
            println!("out: {tx} final — {} exit(s) settled", f["covers"].as_array().map_or(0, Vec::len));
        }
        std::fs::remove_file(self.inflight_path()).map_err(|e| e.to_string())?;
        Ok(false)
    }

    fn pay(&self) -> Result<(), String> {
        let rows = zynzapd::object_client::call(&self.node, "zyn_bridgeExits", json!({ "token": self.token()?, "asset": hex(&self.asset) }))?;
        let mut payouts = Vec::new();
        let mut covers = Vec::new();
        for r in rows.as_array().into_iter().flatten() {
            let (Some(index), Some(to)) = (r["index"].as_u64(), r["to"].as_str()) else { continue };
            let Some(amount) = r["amount"].as_str().and_then(|a| a.parse::<u128>().ok()).filter(|a| *a > 0) else { continue };
            let Some(to) = unhex(to).and_then(|b| <[u8; 20]>::try_from(b).ok()) else { continue };
            payouts.push(Payout { to, asset: self.vault_asset, amount });
            covers.push(index);
            if payouts.len() == PER_TX {
                break;
            }
        }
        if payouts.is_empty() {
            return Ok(());
        }
        let nonce = self.vault_nonce()?;
        let epoch = zynzapd::object_client::call(&self.node, "zyn_status", json!({}))?["epoch"].as_u64().unwrap_or(0);
        let digest = eth::withdrawal_digest(self.chain_id, &self.vault, nonce, epoch, &payouts);
        // The keys sign only what the contract itself would check.
        let theirs = self.call(&eth::digest_call(nonce, epoch, &payouts))?;
        if theirs != digest {
            return Err(format!("the vault computes digest {} where this bridge computes {} — REFUSING to sign", hex(&theirs), hex(&digest)));
        }
        let signature = eth::threshold_signature(&self.keys, self.threshold, &digest)?;
        let data = eth::withdraw_call(nonce, epoch, &payouts, &signature);
        let payer = format!("0x{}", hex(&eth::address_of(&self.payer)));
        let to = format!("0x{}", hex(&self.vault));
        let gas = qty(&self.eth("eth_estimateGas", json!([{ "from": payer, "to": to, "data": format!("0x{}", hex(&data)) }]))?).ok_or("no gas estimate")?;
        let price = qty(&self.eth("eth_gasPrice", json!([]))?).ok_or("no gas price")?;
        let tx_nonce = qty(&self.eth("eth_getTransactionCount", json!([payer, "pending"]))?).ok_or("no account nonce")?;
        let raw = Tx { nonce: tx_nonce as u64, gas_price: price * 12 / 10, gas: (gas * 13 / 10) as u64, to: self.vault, value: 0, data, chain_id: self.chain_id }.signed(&self.payer)?;
        let tx_hash = format!("0x{}", hex(&zyn_vm::eip712::keccak(&[&raw])));
        let record = json!({ "tx": tx_hash, "raw": format!("0x{}", hex(&raw)), "vault_nonce": nonce, "covers": covers });
        std::fs::write(self.inflight_path(), serde_json::to_vec_pretty(&record).unwrap()).map_err(|e| e.to_string())?;
        match self.eth("eth_sendRawTransaction", json!([record["raw"]])) {
            Ok(_) => println!("out: {tx_hash} — {} exit(s) paid, waiting for finality", covers.len()),
            Err(e) => eprintln!("zyn-eth-bridge: broadcast deferred: {e}"),
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
            let bal = qty(&b.eth("eth_getBalance", json!([format!("0x{}", hex(&b.vault)), "latest"]))?).unwrap_or(0);
            let bridged = zynzapd::object_client::call(&b.node, "zyn_bridged", json!({}))?;
            let issued = bridged.as_array().into_iter().flatten().find(|r| r["asset"].as_str() == Some(&hex(&b.asset))).map(|r| r["issued"].clone());
            println!("network  {}", String::from_utf8_lossy(&b.network));
            println!("asset    {}", hex(&b.asset));
            println!("vault    0x{}  ({bal} wei, withdrawal nonce {})", hex(&b.vault), b.vault_nonce()?);
            println!("owners   {} key(s) here, {} needed", b.keys.len(), b.threshold);
            println!("payer    0x{}", hex(&eth::address_of(&b.payer)));
            println!("issued   {}", issued.map_or("not registered — run `register`".into(), |i| format!("{i} wei")));
            println!("in flight {}", b.inflight().is_some());
            Ok(())
        }
        "register" => {
            let r = b.submit(&Op::RegisterBridged { program: program::bridge_register(), network: b.network.clone(), origin: Vec::new(), decimals: 18 })?;
            println!("register: {}", r.chars().take(160).collect::<String>());
            Ok(())
        }
        "once" => b.once(),
        "run" => {
            let every: u64 = env("ZYN_ETH_POLL", "20").parse().unwrap_or(20);
            loop {
                if let Err(e) = b.once() {
                    eprintln!("zyn-eth-bridge: {e}");
                }
                std::thread::sleep(Duration::from_secs(every));
            }
        }
        other => Err(format!("unknown command {other}: status | register | once | run")),
    });
    if let Err(e) = result {
        eprintln!("zyn-eth-bridge {cmd}: {e}");
        std::process::exit(1);
    }
}
