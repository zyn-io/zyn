//! `zyn-eth-bridge` — ETH and the vault's tokens in and out of the object
//! ledger as ETH.zy, BOLD.zy, … (DECISIONS §146, §162).
//!
//! ```text
//!   zyn-eth-bridge status     the vault, each asset's balance, what Zyn has issued
//!   zyn-eth-bridge register   RegisterBridged for every carried asset not yet registered
//!   zyn-eth-bridge register-vault-asset <id> <token>
//!                             the vault's registerAsset(id, token), sent by the gas payer
//!                             (the vault's admin on Sepolia)
//!   zyn-eth-bridge safe-register-asset <id> <token>
//!                             Safe mode: checks and prints the Safe transaction that
//!                             registers it (to the vault, value 0, registerAsset calldata)
//!                             for the owners to propose in Safe{Wallet}; needs no key
//!   zyn-eth-bridge once       credit new deposits, pay revealed exits, stop
//!   zyn-eth-bridge run        the same, every ZYN_ETH_POLL seconds
//! ```
//!
//! **One process carries every vault asset** (`ZYN_ETH_ASSETS`), because
//! withdrawals share the vault's one nonce. **In:** `depositNative` or an
//! ERC-20 `deposit` emits `Deposited`; each finalized one becomes a
//! `DepositCommitted` of that vault asset's Zyn asset, recorded once by the
//! vault's own per-asset index — and only once the vault's record of the id
//! names the token this bridge was told it is. **Out:** a `Withdraw` commits
//! to `keccak(address ‖ salt)`; once revealed, this signs the vault's
//! withdrawal digest — one batch over every carried asset, amounts in WAD as
//! the vault scales them — with `threshold` of the `ZynSigners` owners' keys,
//! after checking the contract computes the same digest; sends it, and
//! records `SettleExit` with the transaction hash once it is final. One
//! payment in flight at a time.
//!
//! **Safe mode (§155), the only mode on mainnet:** when the vault's signer is
//! a Safe (`ZYN_ETH_SAFE`), this holds no signing keys and pays no gas. It
//! proposes each payout batch — `withdrawAsSigner`, simulated first as the
//! Safe — into the Safe's queue with a *proposer* key that can queue but not
//! sign. The Safe's owners approve and execute it; this settles the exits
//! once it is final. A proposal the owners reject is not proposed again.
//! Caps (§155, per asset since §163): `ZYN_ETH_CAPS` caps what this bridge
//! issues of each vault asset, in the token's base units; deposits beyond an
//! asset's cap are not credited, and are listed (with their sender) for the
//! Safe to refund. A capped bridge caps every asset it carries.
//!
//! ```text
//!   ZYN_ETH_NODE        the object ledger's JSON-RPC       (http://127.0.0.1:8499)
//!   ZYN_SUBMIT_TOKEN    the operator door's token (read from the node's env)
//!   ZYN_ETH_RPC         Ethereum JSON-RPC                                (required)
//!   ZYN_ETH_CHAIN_ID    the chain                                   (11155111, Sepolia)
//!   ZYN_ETH_VAULT       the ZynVault address                             (required)
//!   ZYN_ETH_ASSETS      the vault assets carried: id=native|id=0x<token>,…   (1=native)
//!   ZYN_ETH_ASSET_ID    the native id when ZYN_ETH_ASSETS is unset (older env)  (1)
//!   ZYN_ETH_KEYS        files, each one owner's hex key, comma-separated (required)
//!   ZYN_ETH_THRESHOLD   owners needed                                          (2)
//!   ZYN_ETH_PAYER       file with the hex key that pays gas                (required)
//!   ZYN_ETH_FINALITY    block tag deposits and payments must reach  (finalized)
//!   ZYN_ETH_STATE       dir for the block cursor and payments in flight
//!   ZYN_ETH_POLL        seconds between passes in `run`                        (20)
//!   ZYN_ETH_SAFE        the Safe that is the vault's signer (Safe mode)
//!   ZYN_ETH_PROPOSER    file with the proposer's hex key            (Safe mode)
//!   ZYN_ETH_SAFE_API    Safe transaction service  (api.safe.global/tx-service/…)
//!   ZYN_ETH_SAFE_API_KEY  its API key, sent as a bearer token         (none)
//!   ZYN_ETH_CAPS        per vault asset, the most issued, in base units:
//!                       1=<wei>,2=<token units>                       (no caps)
//!   ZYN_ETH_CAP_WEI     the native asset's cap (older env; may sit beside
//!                       ZYN_ETH_CAPS if they agree)
//! ```

use std::time::Duration;

use k256::ecdsa::SigningKey;
use objectvm::{ObjectVm, Op};
use serde_json::{json, Value};
use zyn_vm::spec::MicrochainVm;
use zyn_vm::{program, Fixed};
use zynzapd::eth_bridge::{self as eth, Carried, Exit, Tx, VaultAsset};

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
    network: Vec<u8>,
    assets: Vec<Carried>,
    keys: Vec<SigningKey>,
    threshold: usize,
    /// The gas payer; `None` in Safe mode (and for keyless commands).
    payer: Option<SigningKey>,
    finality: String,
    state: std::path::PathBuf,
    agent: ureq::Agent,
    safe: Option<eth::Address20>,
    proposer: Option<SigningKey>,
    safe_api: String,
    safe_api_key: Option<String>,
    /// Per vault id, the most issued, in the token's base units (§163).
    caps: std::collections::BTreeMap<u32, u128>,
}

/// The bridge as configured; `keys` false loads no key at all, for commands
/// that only read and print (`safe-register-asset`).
fn bridge(keys: bool) -> Result<Bridge, String> {
    let rpc = env("ZYN_ETH_RPC", "");
    if rpc.is_empty() {
        return Err("set ZYN_ETH_RPC to an Ethereum JSON-RPC endpoint".into());
    }
    let chain_id: u64 = env("ZYN_ETH_CHAIN_ID", "11155111").parse().map_err(|_| "ZYN_ETH_CHAIN_ID must be a number".to_string())?;
    let safe: Option<eth::Address20> = match env("ZYN_ETH_SAFE", "") {
        s if s.is_empty() => None,
        s => Some(unhex(&s).and_then(|b| b.try_into().ok()).ok_or("ZYN_ETH_SAFE is not a 0x address")?),
    };
    if chain_id == 1 && safe.is_none() {
        return Err("ETH mainnet runs only with a Safe as the vault's signer (§155): set ZYN_ETH_SAFE".into());
    }
    let vault: eth::Address20 = unhex(&env("ZYN_ETH_VAULT", "")).and_then(|b| b.try_into().ok()).ok_or("set ZYN_ETH_VAULT to the vault's 0x address")?;
    // Safe mode holds no signing keys and pays no gas; the Safe's owners do both.
    let (keys, payer, proposer) = if !keys {
        (Vec::new(), None, None)
    } else if safe.is_some() {
        let p = key_from(&env("ZYN_ETH_PROPOSER", "")).map_err(|e| format!("ZYN_ETH_PROPOSER: {e}"))?;
        (Vec::new(), None, Some(p))
    } else {
        let keys = env("ZYN_ETH_KEYS", "").split(',').filter(|s| !s.is_empty()).map(key_from).collect::<Result<Vec<_>, _>>()?;
        (keys, Some(key_from(&env("ZYN_ETH_PAYER", ""))?), None)
    };
    let safe_api = env("ZYN_ETH_SAFE_API", match chain_id {
        1 => "https://api.safe.global/tx-service/eth",
        11155111 => "https://api.safe.global/tx-service/sep",
        _ => "",
    });
    let network = format!("eip155:{chain_id}").into_bytes();
    let spec = match env("ZYN_ETH_ASSETS", "") {
        s if s.is_empty() => format!("{}=native", env("ZYN_ETH_ASSET_ID", "1")),
        s => s,
    };
    let assets = eth::parse_assets(&spec, &network)?;
    let caps = eth::parse_caps(&env("ZYN_ETH_CAPS", ""), &env("ZYN_ETH_CAP_WEI", ""), &assets)?;
    let state = std::path::PathBuf::from(env("ZYN_ETH_STATE", "/var/lib/zyn/eth-bridge"));
    std::fs::create_dir_all(&state).map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(Bridge {
        node: env("ZYN_ETH_NODE", "http://127.0.0.1:8499"),
        rpc,
        chain_id,
        vault,
        network,
        assets,
        keys,
        threshold: env("ZYN_ETH_THRESHOLD", "2").parse().map_err(|_| "ZYN_ETH_THRESHOLD must be a number".to_string())?,
        payer,
        finality: env("ZYN_ETH_FINALITY", "finalized"),
        state,
        agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
        safe,
        proposer,
        safe_api,
        safe_api_key: Some(env("ZYN_ETH_SAFE_API_KEY", "")).filter(|k| !k.is_empty()),
        caps,
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
        self.call_to(&self.vault, data)
    }

    fn call_to(&self, to: &eth::Address20, data: &[u8]) -> Result<Vec<u8>, String> {
        let r = self.eth("eth_call", json!([{ "to": format!("0x{}", hex(to)), "data": format!("0x{}", hex(data)) }, "latest"]))?;
        unhex(r.as_str().unwrap_or("")).ok_or_else(|| "eth_call answered something that is not hex".to_string())
    }

    /// What the vault says of a carried asset's id — refused outright when
    /// it names another token than this bridge was told: crediting or paying
    /// under the wrong id would issue or pay the wrong asset.
    fn vault_asset(&self, c: &Carried) -> Result<VaultAsset, String> {
        let v = eth::decode_vault_asset(&self.call(&eth::assets_call(c.id))?).ok_or_else(|| format!("assets({}) answered oddly", c.id))?;
        if v.registered && v.token != c.token {
            return Err(format!(
                "the vault's asset {} is {} but ZYN_ETH_ASSETS says {} — REFUSING to carry it",
                c.id,
                v.token.map_or("native".into(), |t| format!("0x{}", hex(&t))),
                c.token.map_or("native".into(), |t| format!("0x{}", hex(&t)))
            ));
        }
        Ok(v)
    }

    /// The token's own decimals, or 18 for the chain's coin.
    fn token_decimals(&self, c: &Carried) -> Result<u8, String> {
        let Some(t) = c.token else { return Ok(18) };
        let r = self.call_to(&t, &eth::decimals_call())?;
        (r.len() == 32 && r[..31].iter().all(|b| *b == 0)).then(|| r[31]).ok_or_else(|| format!("0x{}.decimals() answered oddly", hex(&t)))
    }

    /// What the vault holds of a carried asset, in its own units.
    fn held(&self, c: &Carried) -> Result<u128, String> {
        match c.token {
            None => qty(&self.eth("eth_getBalance", json!([format!("0x{}", hex(&self.vault)), "latest"]))?).ok_or_else(|| "no balance".to_string()),
            Some(t) => {
                let r = self.call_to(&t, &eth::balance_of_call(&self.vault))?;
                (r.len() == 32 && r[..16].iter().all(|b| *b == 0)).then(|| u128::from_be_bytes(r[16..].try_into().unwrap())).ok_or_else(|| "balanceOf answered oddly".to_string())
            }
        }
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
        // Per capped asset, what is issued (WAD), read once and kept current
        // through this pass so two deposits in one pass cannot both slip under
        // its cap; and its cap in WAD, by the decimals the vault scales it by.
        let mut issued: std::collections::BTreeMap<u32, u128> = std::collections::BTreeMap::new();
        // Each carried asset's id checked against the vault once per pass.
        let mut checked: std::collections::BTreeMap<u32, VaultAsset> = std::collections::BTreeMap::new();
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
                let Some(carried) = eth::carried(&self.assets, asset) else {
                    eprintln!("zyn-eth-bridge: deposit {index} of vault asset {asset} — not carried here (ZYN_ETH_ASSETS), NOT credited");
                    continue;
                };
                if !checked.contains_key(&asset) {
                    checked.insert(asset, self.vault_asset(carried)?);
                }
                if let Some(units) = self.caps.get(&asset) {
                    let decimals = checked[&asset].decimals;
                    let cap = eth::cap_wad(*units, decimals).ok_or_else(|| format!("vault asset {asset}: its cap does not fit in WAD"))?;
                    if !issued.contains_key(&asset) {
                        issued.insert(asset, self.issued(&carried.zyn)?);
                    }
                    if eth::past_cap(issued[&asset], wad, cap) {
                        self.over_cap(l, carried, decimals, index, wad, &commitment)?;
                        continue;
                    }
                }
                let r = self.submit(&Op::DepositCommitted {
                    program: program::deposit(),
                    asset: carried.zyn,
                    gross: Fixed(wad as i128),
                    external_ref: eth::deposit_ref(self.chain_id, &self.vault, asset, index),
                    commitment,
                })?;
                if r.contains("already credited") {
                    continue;
                }
                if let Some(i) = issued.get_mut(&asset) {
                    *i = i.saturating_add(wad);
                }
                if r.contains("Refused") {
                    return Err(format!("deposit {index}: the ledger refused it — {}", r.chars().take(160).collect::<String>()));
                }
                println!("in:  deposit {index} of vault asset {asset} — {wad} (WAD) waiting for {}", hex(&commitment));
            }
            from = to + 1;
            std::fs::write(&cursor, from.to_string()).map_err(|e| format!("{}: {e}", cursor.display()))?;
        }
        Ok(())
    }

    /// What Zyn has issued of a bridged asset, in WAD (Fixed has 18 decimals).
    fn issued(&self, zyn: &[u8; 32]) -> Result<u128, String> {
        let bridged = zynzapd::object_client::call(&self.node, "zyn_bridged", json!({}))?;
        Ok(bridged
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["asset"].as_str() == Some(&hex(zyn)))
            .and_then(|r| r["issued"].as_str().and_then(|i| i.parse::<u128>().ok()))
            .unwrap_or(0))
    }

    /// Every revealed, unpaid exit of every carried asset the vault has
    /// registered, oldest first, batched: what one withdrawal pays.
    fn next_batch(&self) -> Result<(Vec<eth::Payout>, Vec<u64>), String> {
        let mut exits = Vec::new();
        for c in &self.assets {
            let v = self.vault_asset(c)?;
            let rows = zynzapd::object_client::call(&self.node, "zyn_bridgeExits", json!({ "token": self.token()?, "asset": hex(&c.zyn) }))?;
            for r in rows.as_array().into_iter().flatten() {
                let (Some(index), Some(to)) = (r["index"].as_u64(), r["to"].as_str()) else { continue };
                let Some(amount) = r["amount"].as_str().and_then(|a| a.parse::<u128>().ok()).filter(|a| *a > 0) else { continue };
                let Some(to) = unhex(to).and_then(|b| <[u8; 20]>::try_from(b).ok()) else { continue };
                if !v.registered {
                    eprintln!("zyn-eth-bridge: exit {index} waits — the vault has not registered asset {} yet", c.id);
                    continue;
                }
                exits.push(Exit { index, to, vault_asset: c.id, decimals: v.decimals, amount });
            }
        }
        exits.sort_by_key(|e| e.index);
        let (payouts, covers, inexact) = eth::batch(&exits, PER_TX);
        for i in inexact {
            eprintln!("zyn-eth-bridge: exit {i} is not a whole number of the token's units — the vault would revert; NOT paid");
        }
        Ok((payouts, covers))
    }

    /// A deposit past its asset's cap: not credited, recorded once with its
    /// sender so the Safe can send it back. Deposit indexes count per vault
    /// asset, so a row is known by (asset, index); the pilot's rows, written
    /// before there was more than one asset, carry no asset and are ETH's.
    fn over_cap(&self, log: &Value, c: &Carried, decimals: u8, index: u64, wad: u128, commitment: &[u8; 32]) -> Result<(), String> {
        let path = self.state.join("over-cap.jsonl");
        let seen = std::fs::read_to_string(&path).unwrap_or_default();
        let native = self.assets.iter().find(|a| a.token.is_none()).map(|a| a.id);
        let row_asset = |v: &Value| v["asset"].as_u64().map(|a| a as u32).or(native);
        if seen.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).any(|v| v["index"].as_u64() == Some(index) && row_asset(&v) == Some(c.id)) {
            return Ok(());
        }
        let tx = log["transactionHash"].as_str().unwrap_or("").to_string();
        let from = self.eth("eth_getTransactionByHash", json!([tx]))?["from"].as_str().unwrap_or("").to_string();
        // What to send back, in the token's own units.
        let units = eth::token_units(wad, decimals).map_or_else(|| "inexact".to_string(), |u| u.to_string());
        let token = c.token.map_or("native".to_string(), |t| eth::checksummed(&t));
        let mut row = json!({ "asset": c.id, "token": token, "index": index, "wad": wad.to_string(), "units": units, "from": from, "tx": tx, "commitment": hex(commitment) });
        if c.token.is_none() {
            row["wei"] = json!(wad.to_string());
        }
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        writeln!(f, "{row}").map_err(|e| e.to_string())?;
        eprintln!("zyn-eth-bridge: deposit {index} of vault asset {} ({units} base units of {token} from {from}) is past its cap — NOT credited; listed in {} for the Safe to refund", c.id, path.display());
        Ok(())
    }

    fn safe_auth(&self, req: ureq::Request) -> ureq::Request {
        match &self.safe_api_key {
            Some(k) => req.set("Authorization", &format!("Bearer {k}")),
            None => req,
        }
    }

    fn safe_get(&self, path: &str) -> Result<Option<Value>, String> {
        match self.safe_auth(self.agent.get(&format!("{}{path}", self.safe_api))).call() {
            Ok(r) => r.into_json().map(Some).map_err(|_| "the Safe service answered something that is not JSON".to_string()),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(format!("Safe service: {e}")),
        }
    }

    fn safe_post(&self, path: &str, body: &Value) -> Result<(), String> {
        match self.safe_auth(self.agent.post(&format!("{}{path}", self.safe_api))).send_json(body.clone()) {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, r)) => Err(format!("Safe service refused the proposal ({code}): {}", r.into_string().unwrap_or_default().chars().take(300).collect::<String>())),
            Err(e) => Err(format!("Safe service: {e}")),
        }
    }

    fn safe_call(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        let safe = self.safe.ok_or("not in Safe mode")?;
        let r = self.eth("eth_call", json!([{ "to": format!("0x{}", hex(&safe)), "data": format!("0x{}", hex(data)) }, "latest"]))?;
        unhex(r.as_str().unwrap_or("")).ok_or_else(|| "eth_call answered something that is not hex".to_string())
    }

    fn safe_nonce(&self) -> Result<u64, String> {
        let r = self.safe_call(&eth::nonce_call())?;
        (r.len() == 32).then(|| u64::from_be_bytes(r[24..].try_into().unwrap())).ok_or_else(|| "the Safe's nonce() answered oddly".to_string())
    }

    fn proposal_path(&self) -> std::path::PathBuf {
        self.state.join("proposal.json")
    }

    /// Safe mode: propose a payout batch into the Safe's queue, or follow the
    /// one already proposed until the owners execute (or reject) it.
    fn pay_safe(&self) -> Result<(), String> {
        let safe = self.safe.ok_or("not in Safe mode")?;
        let safe_hex = eth::checksummed(&safe);
        if let Ok(raw) = std::fs::read(self.proposal_path()) {
            // A damaged record is not "nothing proposed": that would propose again.
            let p: Value = serde_json::from_slice(&raw).unwrap_or_else(|_| panic!("{} is damaged — fix it before proposing again", self.proposal_path().display()));
            let hash = p["safe_tx_hash"].as_str().unwrap_or("").to_string();
            let info = self.safe_get(&format!("/api/v1/multisig-transactions/{hash}/"))?;
            if self.vault_nonce()? > p["vault_nonce"].as_u64().unwrap_or(u64::MAX) {
                let executed = info.as_ref().filter(|i| i["isExecuted"].as_bool() == Some(true) && i["isSuccessful"].as_bool() == Some(true));
                let Some(tx) = executed.and_then(|i| i["transactionHash"].as_str()).map(str::to_string) else {
                    eprintln!("zyn-eth-bridge: THE VAULT NONCE MOVED but proposal {hash} was not what executed — investigate before paying again");
                    return Ok(());
                };
                let receipt = self.eth("eth_getTransactionReceipt", json!([tx]))?;
                if receipt.is_null() || qty(&receipt["blockNumber"]).unwrap_or(u128::MAX) as u64 > self.final_block()? {
                    return Ok(()); // executed; waiting for finality
                }
                let txid: [u8; 32] = unhex(&tx).and_then(|b| b.try_into().ok()).ok_or("a transaction hash is not 32 bytes")?;
                if !self.settle(&p["covers"], txid, &tx)? {
                    return Ok(()); // kept: nothing is proposed until the ledger records it
                }
                println!("out: {tx} executed by the Safe and final — {} exit(s) settled", p["covers"].as_array().map_or(0, Vec::len));
                std::fs::remove_file(self.proposal_path()).map_err(|e| e.to_string())?;
                return Ok(());
            }
            if info.is_none() {
                // Written before it was sent; send it now.
                self.safe_post(&format!("/api/v1/safes/{safe_hex}/multisig-transactions/"), &p["body"])?;
                println!("out: proposal {hash} sent to the Safe's queue");
            } else if self.safe_nonce()? > p["safe_nonce"].as_u64().unwrap_or(u64::MAX) {
                eprintln!("zyn-eth-bridge: the Safe used nonce {} for something else — proposal {hash} was rejected or replaced; its exits wait. Delete {} to propose them again.", p["safe_nonce"], self.proposal_path().display());
            }
            return Ok(());
        }

        let (payouts, covers) = self.next_batch()?;
        if payouts.is_empty() {
            return Ok(());
        }
        let nonce = self.vault_nonce()?;
        let epoch = zynzapd::object_client::call(&self.node, "zyn_status", json!({}))?["epoch"].as_u64().unwrap_or(0);
        let data = eth::withdraw_as_signer_call(nonce, epoch, &payouts);
        let vault_hex = eth::checksummed(&self.vault);
        // Simulate it as the Safe first: owners are only asked to approve a
        // payout the vault would accept (right nonce, enough funds, known asset).
        self.eth("eth_call", json!([{ "from": safe_hex, "to": vault_hex, "data": format!("0x{}", hex(&data)) }, "latest"]))
            .map_err(|e| format!("the payout would fail if the Safe executed it — not proposing: {e}"))?;
        // The Safe's next free nonce: past what is executed and what is queued.
        let onchain = self.safe_nonce()?;
        let queued = self
            .safe_get(&format!("/api/v1/safes/{safe_hex}/multisig-transactions/?executed=false&nonce__gte={onchain}&ordering=-nonce&limit=1"))?
            .and_then(|v| v["results"].get(0).and_then(|t| t["nonce"].as_u64().or_else(|| t["nonce"].as_str().and_then(|n| n.parse().ok()))));
        let safe_nonce = queued.map_or(onchain, |q| (q + 1).max(onchain));
        let h = self.safe_call(&eth::safe_tx_hash_call(&self.vault, 0, &data, safe_nonce))?;
        let safe_tx_hash: [u8; 32] = h.as_slice().try_into().map_err(|_| "getTransactionHash answered oddly")?;
        let proposer = self.proposer.as_ref().ok_or("no proposer key")?;
        let signature = eth::sign(proposer, &safe_tx_hash)?;
        let zero = eth::checksummed(&[0u8; 20]);
        let body = json!({
            "to": vault_hex, "value": "0", "data": format!("0x{}", hex(&data)), "operation": 0,
            "safeTxGas": "0", "baseGas": "0", "gasPrice": "0", "gasToken": zero, "refundReceiver": zero,
            "nonce": safe_nonce, "contractTransactionHash": format!("0x{}", hex(&safe_tx_hash)),
            "sender": eth::checksummed(&eth::address_of(proposer)), "signature": format!("0x{}", hex(&signature)),
            "origin": format!("Zyn bridge: pay out {} withdrawal(s)", payouts.len()),
        });
        let record = json!({ "safe_tx_hash": format!("0x{}", hex(&safe_tx_hash)), "safe_nonce": safe_nonce, "vault_nonce": nonce, "covers": covers, "body": body });
        std::fs::write(self.proposal_path(), serde_json::to_vec_pretty(&record).unwrap()).map_err(|e| e.to_string())?;
        self.safe_post(&format!("/api/v1/safes/{safe_hex}/multisig-transactions/"), &body)?;
        println!("out: {} withdrawal(s) proposed to the Safe (nonce {safe_nonce}, {}) — waiting for its owners", covers.len(), record["safe_tx_hash"].as_str().unwrap_or(""));
        Ok(())
    }

    /// Record a final payment's exits. False when the ledger refused one: the
    /// payment is then kept and retried, never forgotten — forgotten while an
    /// exit still reads as owed, it would be paid again.
    fn settle(&self, covers: &Value, txid: [u8; 32], tx: &str) -> Result<bool, String> {
        let mut ok = true;
        for index in covers.as_array().into_iter().flatten().filter_map(Value::as_u64) {
            let r = self.submit(&Op::SettleExit { program: program::settle_exit(), index: index as u32, txid })?;
            if r.contains("Refused") && !r.contains("already settled") {
                eprintln!("zyn-eth-bridge: SETTLEMENT REFUSED — exit {index} was paid by {tx} and the ledger will not record it: {r}. Nothing more is paid until it is.");
                ok = false;
            }
        }
        Ok(ok)
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
            if !self.settle(&f["covers"], txid, &tx)? {
                return Ok(true); // kept: nothing is paid until the ledger records it
            }
            println!("out: {tx} final — {} exit(s) settled", f["covers"].as_array().map_or(0, Vec::len));
        }
        std::fs::remove_file(self.inflight_path()).map_err(|e| e.to_string())?;
        Ok(false)
    }

    fn pay(&self) -> Result<(), String> {
        let (payouts, covers) = self.next_batch()?;
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
        let payer_key = self.payer.as_ref().ok_or("no gas payer key (ZYN_ETH_PAYER)")?;
        let payer = format!("0x{}", hex(&eth::address_of(payer_key)));
        let to = format!("0x{}", hex(&self.vault));
        let gas = qty(&self.eth("eth_estimateGas", json!([{ "from": payer, "to": to, "data": format!("0x{}", hex(&data)) }]))?).ok_or("no gas estimate")?;
        let price = qty(&self.eth("eth_gasPrice", json!([]))?).ok_or("no gas price")?;
        let tx_nonce = qty(&self.eth("eth_getTransactionCount", json!([payer, "pending"]))?).ok_or("no account nonce")?;
        let raw = Tx { nonce: tx_nonce as u64, gas_price: price * 12 / 10, gas: (gas * 13 / 10) as u64, to: self.vault, value: 0, data, chain_id: self.chain_id }.signed(payer_key)?;
        let tx_hash = format!("0x{}", hex(&zyn_vm::eip712::keccak(&[&raw])));
        let record = json!({ "tx": tx_hash, "raw": format!("0x{}", hex(&raw)), "vault_nonce": nonce, "covers": covers });
        std::fs::write(self.inflight_path(), serde_json::to_vec_pretty(&record).unwrap()).map_err(|e| e.to_string())?;
        match self.eth("eth_sendRawTransaction", json!([record["raw"]])) {
            Ok(_) => println!("out: {tx_hash} — {} exit(s) paid, waiting for finality", covers.len()),
            Err(e) => eprintln!("zyn-eth-bridge: broadcast deferred: {e}"),
        }
        Ok(())
    }

    /// `RegisterBridged` for every carried asset Zyn does not know yet: the
    /// origin is the raw token address (none for ETH), with the decimals the
    /// vault scales it by.
    fn register(&self) -> Result<(), String> {
        let bridged = zynzapd::object_client::call(&self.node, "zyn_bridged", json!({}))?;
        for c in &self.assets {
            if bridged.as_array().into_iter().flatten().any(|r| r["asset"].as_str() == Some(&hex(&c.zyn))) {
                println!("register: vault asset {} ({}) is registered already", c.id, hex(&c.zyn));
                continue;
            }
            let v = self.vault_asset(c)?;
            let decimals = if v.registered { v.decimals } else { self.token_decimals(c)? };
            let r = self.submit(&Op::RegisterBridged { program: program::bridge_register(), network: self.network.clone(), origin: c.origin(), decimals })?;
            println!("register: vault asset {} as {} — {}", c.id, hex(&c.zyn), r.chars().take(160).collect::<String>());
        }
        Ok(())
    }

    /// The vault's `registerAsset(id, token)`, sent and signed by the gas
    /// payer — which must be the vault's admin. Add-only on the contract.
    fn register_vault_asset(&self, id: u32, token: eth::Address20) -> Result<(), String> {
        if self.safe.is_some() {
            return Err("in Safe mode the vault's admin registers assets from Safe{Wallet}, not this bridge: `safe-register-asset <id> <token>` prints the transaction to propose".into());
        }
        let payer_key = self.payer.as_ref().ok_or("no gas payer key (ZYN_ETH_PAYER)")?;
        let c = Carried::new(&self.network, id, Some(token));
        let v = eth::decode_vault_asset(&self.call(&eth::assets_call(id))?).ok_or("assets() answered oddly")?;
        if v.registered {
            return if v.token == Some(token) {
                println!("vault: asset {id} is 0x{} already ({} decimals)", hex(&token), v.decimals);
                Ok(())
            } else {
                Err(format!("the vault's asset {id} is taken by {}", v.token.map_or("native ETH".into(), |t| format!("0x{}", hex(&t)))))
            };
        }
        let payer = eth::address_of(payer_key);
        let admin = self.call(&eth::admin_call())?;
        if admin.len() != 32 || admin[12..] != payer {
            return Err(format!("the vault's admin is 0x{}, not the gas payer 0x{} — only the admin may register", hex(admin.get(12..).unwrap_or(&[])), hex(&payer)));
        }
        let decimals = self.token_decimals(&c)?;
        let data = eth::register_asset_call(id, &token);
        let from = format!("0x{}", hex(&payer));
        let to = format!("0x{}", hex(&self.vault));
        let gas = qty(&self.eth("eth_estimateGas", json!([{ "from": from, "to": to, "data": format!("0x{}", hex(&data)) }]))?).ok_or("no gas estimate")?;
        let price = qty(&self.eth("eth_gasPrice", json!([]))?).ok_or("no gas price")?;
        let nonce = qty(&self.eth("eth_getTransactionCount", json!([from, "pending"]))?).ok_or("no account nonce")?;
        let raw = Tx { nonce: nonce as u64, gas_price: price * 12 / 10, gas: (gas * 13 / 10) as u64, to: self.vault, value: 0, data, chain_id: self.chain_id }.signed(payer_key)?;
        let tx = format!("0x{}", hex(&zyn_vm::eip712::keccak(&[&raw])));
        self.eth("eth_sendRawTransaction", json!([format!("0x{}", hex(&raw))]))?;
        println!("vault: registerAsset({id}, 0x{}) sent — {tx}", hex(&token));
        for _ in 0..60 {
            std::thread::sleep(Duration::from_secs(5));
            let receipt = self.eth("eth_getTransactionReceipt", json!([tx]))?;
            if receipt.is_null() {
                continue;
            }
            if qty(&receipt["status"]) != Some(1) {
                return Err(format!("{tx} reverted"));
            }
            println!("vault: asset {id} registered ({decimals} decimals) — Zyn asset {}", hex(&c.zyn));
            return Ok(());
        }
        Err(format!("{tx} not mined after five minutes; check it before sending again"))
    }

    /// Safe mode: the transaction the vault's admin — the Safe — executes to
    /// register a token as vault asset `id`, printed for its owners to
    /// propose in Safe{Wallet}. Nothing is signed or sent; the call is
    /// simulated from the Safe first, so what is printed is what would pass.
    fn safe_register_asset(&self, id: u32, token: eth::Address20) -> Result<(), String> {
        let safe = self.safe.ok_or("not in Safe mode: use `register-vault-asset`")?;
        let admin = self.call(&eth::admin_call())?;
        if admin.len() != 32 || admin[12..] != safe {
            return Err(format!("the vault's admin is 0x{}, not the Safe — the Safe cannot register assets", hex(admin.get(12..).unwrap_or(&[]))));
        }
        let v = eth::decode_vault_asset(&self.call(&eth::assets_call(id))?).ok_or("assets() answered oddly")?;
        if v.registered {
            return if v.token == Some(token) {
                println!("vault: asset {id} is {} already ({} decimals) — nothing to propose", eth::checksummed(&token), v.decimals);
                Ok(())
            } else {
                Err(format!("the vault's asset {id} is taken by {}", v.token.map_or("native ETH".into(), |t| eth::checksummed(&t))))
            };
        }
        let c = Carried::new(&self.network, id, Some(token));
        let decimals = self.token_decimals(&c)?;
        let data = eth::register_asset_call(id, &token);
        let (safe_hex, vault_hex) = (eth::checksummed(&safe), eth::checksummed(&self.vault));
        self.eth("eth_call", json!([{ "from": safe_hex, "to": vault_hex, "data": format!("0x{}", hex(&data)) }, "latest"]))
            .map_err(|e| format!("registerAsset would fail if the Safe executed it: {e}"))?;
        println!("Propose from the Safe {safe_hex} (Safe{{Wallet}} → New transaction → Transaction Builder, custom data):");
        println!("  to         {vault_hex}   (ZynVault)");
        println!("  value      0");
        println!("  data       0x{}", hex(&data));
        println!("  operation  0 (call)");
        println!("  = registerAsset({id}, {}) — token {decimals} decimals; simulated from the Safe: passes", eth::checksummed(&token));
        println!("  on Zyn it is {} ({})", hex(&c.zyn), String::from_utf8_lossy(&self.network));
        println!("Once two owners have executed it: add {id}={} to ZYN_ETH_ASSETS and its cap to ZYN_ETH_CAPS, then restart the bridge.", eth::checksummed(&token));
        Ok(())
    }

    fn once(&self) -> Result<(), String> {
        self.deposits()?;
        if self.safe.is_some() {
            return self.pay_safe();
        }
        if !self.follow_up()? {
            self.pay()?;
        }
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).cloned().unwrap_or_else(|| "status".into());
    let result = bridge(cmd != "safe-register-asset").and_then(|b| match cmd.as_str() {
        "status" => {
            println!("network  {}", String::from_utf8_lossy(&b.network));
            println!("vault    0x{}  (withdrawal nonce {})", hex(&b.vault), b.vault_nonce()?);
            let bridged = zynzapd::object_client::call(&b.node, "zyn_bridged", json!({}))?;
            for c in &b.assets {
                let issued = bridged.as_array().into_iter().flatten().find(|r| r["asset"].as_str() == Some(&hex(&c.zyn))).map(|r| r["issued"].as_str().unwrap_or("?").to_string());
                let v = b.vault_asset(c)?;
                let what = c.token.map_or("native".into(), |t| format!("0x{}", hex(&t)));
                println!("asset {}  {what} → {}", c.id, hex(&c.zyn));
                println!("  vault  {}", if v.registered { format!("registered, {} decimals, holds {}", v.decimals, b.held(c)?) } else { "NOT registered — run `register-vault-asset`".into() });
                println!("  issued {}", issued.map_or("not registered on Zyn — run `register`".into(), |i| format!("{i} (WAD)")));
                println!("  cap    {}", b.caps.get(&c.id).map_or("none".into(), |u| format!("{u} base units")));
            }
            if let Some(safe) = b.safe {
                let proposer = eth::checksummed(&eth::address_of(b.proposer.as_ref().unwrap()));
                let listed = b.safe_get(&format!("/api/v2/delegates/?safe={}&delegate={proposer}", eth::checksummed(&safe)))?.and_then(|v| v["count"].as_u64()).unwrap_or(0) > 0;
                println!("signer   Safe 0x{} (nonce {})", hex(&safe), b.safe_nonce()?);
                println!("proposer {proposer} — {}", if listed { "listed as a proposer on the Safe" } else { "NOT yet a proposer on the Safe: add it in Safe{Wallet} → Settings → Setup → Proposers" });
                println!("proposal {}", std::fs::read_to_string(b.proposal_path()).ok().and_then(|r| serde_json::from_str::<Value>(&r).ok()).map_or("none".into(), |p| format!("{} (Safe nonce {})", p["safe_tx_hash"].as_str().unwrap_or(""), p["safe_nonce"])));
            } else {
                println!("owners   {} key(s) here, {} needed", b.keys.len(), b.threshold);
                println!("payer    0x{}", b.payer.as_ref().map_or("none".into(), |p| hex(&eth::address_of(p))));
            }
            println!("in flight {}", b.inflight().is_some());
            Ok(())
        }
        "register" => b.register(),
        "register-vault-asset" => {
            let id: u32 = args.get(2).and_then(|a| a.parse().ok()).ok_or("usage: register-vault-asset <id> <0x token>")?;
            let token: eth::Address20 = args.get(3).and_then(|t| unhex(t)).and_then(|t| t.try_into().ok()).ok_or("usage: register-vault-asset <id> <0x token>")?;
            b.register_vault_asset(id, token)
        }
        "safe-register-asset" => {
            let id: u32 = args.get(2).and_then(|a| a.parse().ok()).ok_or("usage: safe-register-asset <id> <0x token>")?;
            let token: eth::Address20 = args.get(3).and_then(|t| unhex(t)).and_then(|t| t.try_into().ok()).ok_or("usage: safe-register-asset <id> <0x token>")?;
            b.safe_register_asset(id, token)
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
        other => Err(format!("unknown command {other}: status | register | register-vault-asset | safe-register-asset | once | run")),
    });
    if let Err(e) = result {
        eprintln!("zyn-eth-bridge {cmd}: {e}");
        std::process::exit(1);
    }
}
