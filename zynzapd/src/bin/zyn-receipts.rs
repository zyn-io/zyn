//! `zyn-receipts` — the Zyn app's service: deposit receipts, and the keyring
//! that lets several wallets open one Zyn account.
//!
//! **Receipts**: a record, as an NFT on the chain a deposit came from, that
//! it was made. One per deposit, minted only after the deposit is final on
//! that chain. See `zynzapd::receipt`.
//!
//! **Keyring**: a Zyn account's key, sealed under a key only a linked wallet
//! can make (from its signature), stored under an id only that wallet can
//! make (from the same signature). Nobody can look a wallet up, and the
//! service holds ciphertext only. The first write names the account's
//! public key and is signed by the account; replacing or removing an entry
//! takes that same account's signature.
//!
//! ```text
//!   zyn-receipts serve      answer zyn_receiptInfo / zyn_receiptMint (the node forwards them)
//!   zyn-receipts status     the keys, the contract, what has been minted
//! ```
//!
//! **Transparent**: the deposit transaction must pay the bridge's vault; the
//! receipt goes to the address that sent it and shows the amount.
//! **Private**: the transaction must put a note into one of the pools; the
//! depositor's wallet must have signed `receipt::private_request_text` for
//! this deposit, this receiving address and this hash; the receipt goes to
//! that address and shows only the hash.
//!
//! ```text
//!   ZYN_RECEIPTS_LISTEN          where to answer                     (127.0.0.1:8494)
//!   ZYN_RECEIPTS_STATE           what has been minted     (/var/lib/zyn/receipts)
//!   ZYN_RECEIPTS_URI             base for the Solana metadata files
//!                                (https://lend.zyn-network.com/receipt)
//!   ZYN_RECEIPTS_SOL_RPC         Solana JSON-RPC            (https://api.devnet.solana.com)
//!   ZYN_RECEIPTS_SOL_KEY         keypair file: the receipts' authority and payer
//!   ZYN_RECEIPTS_SOL_VAULT       the SOL bridge's vault
//!   ZYN_RECEIPTS_SOL_POOLS       the private channel's pool program
//!   ZYN_RECEIPTS_SOL_NAME        the chain as a receipt names it          (Solana devnet)
//!   ZYN_RECEIPTS_EVM_RPC         Ethereum JSON-RPC
//!   ZYN_RECEIPTS_EVM_CHAIN_ID    the chain                                     (11155111)
//!   ZYN_RECEIPTS_EVM_CONTRACT    ZynReceipt
//!   ZYN_RECEIPTS_EVM_KEY         file with the minter's hex key
//!   ZYN_RECEIPTS_EVM_VAULT       the ETH bridge's vault
//!   ZYN_RECEIPTS_EVM_POOLS       the private channel's pools, comma-separated
//!   ZYN_RECEIPTS_EVM_NAME        the chain as a receipt names it                (Sepolia)
//! ```

use std::sync::Mutex;
use std::time::Duration;

use k256::ecdsa::SigningKey;
use serde_json::{json, Value};
use zynzapd::eth_bridge::{self as eth, Address20, Tx};
use zynzapd::evm_pool::{self, Event};
use zynzapd::receipt::{self, amount_text, private_request_text};
use zynzapd::sol_bridge::{decode_deposit, pool_events};
use zynzapd::sol_pool::{self, Signer};

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

struct Sol {
    rpc: String,
    signer: Signer,
    vault: String,
    pools: String,
    name: String,
}

struct Evm {
    rpc: String,
    chain_id: u64,
    contract: Address20,
    key: SigningKey,
    vault: Address20,
    pools: Vec<Address20>,
    name: String,
}

struct Service {
    agent: ureq::Agent,
    state: std::path::PathBuf,
    uri: String,
    sol: Option<Sol>,
    evm: Option<Evm>,
    /// One mint at a time: the record of what is minted, and the EVM nonce.
    lock: Mutex<()>,
}

fn rpc(agent: &ureq::Agent, url: &str, method: &str, params: Value) -> Result<Value, String> {
    let r = agent.post(url).send_json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })).map_err(|e| format!("{method}: {e}"))?;
    let v: Value = r.into_json().map_err(|_| "the chain answered something that is not JSON".to_string())?;
    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
        return Err(format!("{method}: {}", e["message"].as_str().map_or_else(|| e.to_string(), String::from)));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

fn service() -> Result<Service, String> {
    let sol = match env("ZYN_RECEIPTS_SOL_KEY", "") {
        k if k.is_empty() => None,
        k => Some(Sol {
            rpc: env("ZYN_RECEIPTS_SOL_RPC", "https://api.devnet.solana.com"),
            signer: Signer::load(&k)?,
            vault: env("ZYN_RECEIPTS_SOL_VAULT", ""),
            pools: env("ZYN_RECEIPTS_SOL_POOLS", ""),
            name: env("ZYN_RECEIPTS_SOL_NAME", "Solana devnet"),
        }),
    };
    let evm = match env("ZYN_RECEIPTS_EVM_KEY", "") {
        k if k.is_empty() => None,
        k => {
            let raw = std::fs::read_to_string(&k).map_err(|e| format!("{k}: {e}"))?;
            let key = unhex(&raw).filter(|b| b.len() == 32).and_then(|b| SigningKey::from_slice(&b).ok()).ok_or("ZYN_RECEIPTS_EVM_KEY is not a 32-byte hex key")?;
            let addr = |s: &str| unhex(s).and_then(|b| <Address20>::try_from(b).ok());
            Some(Evm {
                rpc: env("ZYN_RECEIPTS_EVM_RPC", "https://ethereum-sepolia-rpc.publicnode.com"),
                chain_id: env("ZYN_RECEIPTS_EVM_CHAIN_ID", "11155111").parse().map_err(|_| "ZYN_RECEIPTS_EVM_CHAIN_ID")?,
                contract: addr(&env("ZYN_RECEIPTS_EVM_CONTRACT", "")).ok_or("set ZYN_RECEIPTS_EVM_CONTRACT")?,
                key,
                vault: addr(&env("ZYN_RECEIPTS_EVM_VAULT", "")).ok_or("set ZYN_RECEIPTS_EVM_VAULT")?,
                pools: env("ZYN_RECEIPTS_EVM_POOLS", "").split(',').filter(|s| !s.is_empty()).map(|s| addr(s).ok_or_else(|| format!("{s} is not a 0x address"))).collect::<Result<_, _>>()?,
                name: env("ZYN_RECEIPTS_EVM_NAME", "Sepolia"),
            })
        }
    };
    if sol.is_none() && evm.is_none() {
        return Err("set ZYN_RECEIPTS_SOL_KEY and/or ZYN_RECEIPTS_EVM_KEY".into());
    }
    let state = std::path::PathBuf::from(env("ZYN_RECEIPTS_STATE", "/var/lib/zyn/receipts"));
    std::fs::create_dir_all(&state).map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(Service {
        agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
        state,
        uri: env("ZYN_RECEIPTS_URI", "https://lend.zyn-network.com/receipt"),
        sol,
        evm,
        lock: Mutex::new(()),
    })
}

impl Service {
    fn keyring_path(&self) -> std::path::PathBuf {
        self.state.join("keyring.json")
    }

    fn keyring(&self) -> serde_json::Map<String, Value> {
        std::fs::read(self.keyring_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save_keyring(&self, ring: &serde_json::Map<String, Value>) -> Result<(), String> {
        let tmp = self.keyring_path().with_extension("tmp");
        std::fs::write(&tmp, Value::Object(ring.clone()).to_string()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.keyring_path()).map_err(|e| e.to_string())
    }

    fn ledger_path(&self) -> std::path::PathBuf {
        self.state.join("minted.json")
    }

    fn minted(&self) -> serde_json::Map<String, Value> {
        std::fs::read(self.ledger_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn record(&self, key: &str, v: &Value) -> Result<(), String> {
        let mut m = self.minted();
        m.insert(key.to_string(), v.clone());
        let tmp = self.ledger_path().with_extension("tmp");
        std::fs::write(&tmp, Value::Object(m).to_string()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.ledger_path()).map_err(|e| e.to_string())
    }

    fn handle(&self, body: &Value) -> Result<Value, String> {
        match body["method"].as_str().unwrap_or("") {
            "zyn_receiptInfo" => Ok(json!({
                "sol": self.sol.as_ref().map(|s| json!({ "authority": zyn_custody::solana::base58_encode(&s.signer.pubkey()), "chain": s.name })),
                "evm": self.evm.as_ref().map(|e| json!({ "contract": eth::checksummed(&e.contract), "chain_id": e.chain_id, "chain": e.name })),
            })),
            "zyn_receiptMint" => {
                let chain = body["chain"].as_str().unwrap_or("");
                let kind = body["kind"].as_str().unwrap_or("");
                let tx = body["tx"].as_str().unwrap_or("").trim().to_string();
                if !matches!(kind, "transparent" | "private") {
                    return Err("kind: transparent or private".into());
                }
                let key = format!("{chain}:{kind}:{}", tx.to_ascii_lowercase());
                let _turn = self.lock.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(done) = self.minted().get(&key) {
                    return Ok(done.clone());
                }
                let out = match chain {
                    "sol" => self.mint_sol(kind, &tx, body)?,
                    "evm" => self.mint_evm(kind, &tx, body)?,
                    _ => return Err("chain: sol or evm".into()),
                };
                self.record(&key, &out)?;
                println!("receipt: {key} → {out}");
                Ok(out)
            }
            "zyn_keyringGet" => {
                let id = keyring_id(body)?;
                Ok(self.keyring().get(&id).map(|e| json!({ "blob": e["blob"] })).unwrap_or(Value::Null))
            }
            "zyn_keyringPut" | "zyn_keyringDelete" => {
                let id = keyring_id(body)?;
                let account: [u8; 32] = body["account"].as_str().and_then(unhex).and_then(|b| b.try_into().ok()).ok_or("account: the account's 32-byte public key, hex")?;
                let sig: [u8; 64] = body["signature"].as_str().and_then(unhex).and_then(|b| b.try_into().ok()).ok_or("signature: 64 bytes of hex")?;
                let put = body["method"] == "zyn_keyringPut";
                let blob = if put { body["blob"].as_str().and_then(unhex).filter(|b| (40..=512).contains(&b.len())).ok_or("blob: 40 to 512 bytes of hex")? } else { Vec::new() };
                let what = if put { keyring_put_digest(&id, &blob) } else { keyring_delete_digest(&id) };
                let vk = ed25519_dalek::VerifyingKey::from_bytes(&account).map_err(|_| "account: not an ed25519 key")?;
                vk.verify_strict(&what, &ed25519_dalek::Signature::from_bytes(&sig)).map_err(|_| "the account did not sign this")?;
                let _turn = self.lock.lock().unwrap_or_else(|p| p.into_inner());
                let mut ring = self.keyring();
                if let Some(e) = ring.get(&id) {
                    if e["account"].as_str() != Some(&hex(&account)) {
                        return Err("that wallet is linked to another account; unlink it there first".into());
                    }
                }
                if put {
                    ring.insert(id, json!({ "account": hex(&account), "blob": hex(&blob) }));
                } else {
                    ring.remove(&id);
                }
                self.save_keyring(&ring)?;
                Ok(json!({ "ok": true }))
            }
            other => Err(format!("no method {other}")),
        }
    }

    /// The hash and receiving address a private request names, checked
    /// against the depositor's signature by `verify`.
    fn private_parts(body: &Value) -> Result<([u8; 32], String, Vec<u8>), String> {
        let hash: [u8; 32] = body["hash"].as_str().and_then(unhex).and_then(|b| b.try_into().ok()).ok_or("hash: 32 bytes of hex")?;
        let to = body["to"].as_str().unwrap_or("").trim().to_string();
        let sig = body["signature"].as_str().and_then(unhex).ok_or("signature: hex")?;
        Ok((hash, to, sig))
    }

    fn mint_sol(&self, kind: &str, sig: &str, body: &Value) -> Result<Value, String> {
        let s = self.sol.as_ref().ok_or("no Solana receipts here")?;
        let tx = rpc(&self.agent, &s.rpc, "getTransaction", json!([sig, { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0, "commitment": "finalized" }]))?;
        if tx.is_null() {
            return Err("that transaction is not final on Solana yet; try again in a minute".into());
        }
        let first = &tx["transaction"]["message"]["accountKeys"][0];
        let depositor = first["pubkey"].as_str().or(first.as_str()).ok_or("the transaction names no payer")?.to_string();
        let (owner, name, attrs): (String, String, Vec<(String, String)>) = if kind == "transparent" {
            let (lamports, _) = decode_deposit(&tx, &s.vault).ok_or("that transaction is not a deposit into the SOL bridge")?;
            let amount = amount_text(lamports as u128, 9);
            (depositor, format!("Zyn deposit: {amount} SOL"), vec![("Channel".into(), "Transparent".into()), ("Asset".into(), "SOL".into()), ("Amount".into(), amount), ("Chain".into(), s.name.clone())])
        } else {
            if !pool_events(&tx, &s.pools).iter().any(|e| e.first().map(|n| n.as_slice()) == Some(&b"deposit"[..])) {
                return Err("that transaction puts no note into a pool".into());
            }
            let (hash, to, signature) = Self::private_parts(body)?;
            zyn_custody::solana::pubkey(&to).ok_or("to: a Solana address")?;
            let text = private_request_text("sol", sig, &to, &hash);
            let pk = zyn_custody::solana::pubkey(&depositor).ok_or("the payer is not a Solana address")?;
            let vk = ed25519_dalek::VerifyingKey::from_bytes(&pk).map_err(|_| "the payer is not an ed25519 key")?;
            let sg = ed25519_dalek::Signature::from_slice(&signature).map_err(|_| "signature: 64 bytes")?;
            vk.verify_strict(text.as_bytes(), &sg).map_err(|_| "the depositing wallet did not sign this request")?;
            (to, "Zyn private deposit".into(), vec![("Channel".into(), "Private".into()), ("Hash".into(), format!("0x{}", hex(&hash))), ("Chain".into(), s.name.clone())])
        };
        let owner_key = zyn_custody::solana::pubkey(&owner).ok_or("the owner is not a Solana address")?;
        // The receipt's address follows from this service's key and the
        // deposit, so a second mint for the same deposit fails on-chain
        // (the account exists) even if the record of the first is lost.
        let asset = ed25519_dalek::SigningKey::from_bytes(&s.signer.derive(format!("receipt:{kind}:{sig}").as_bytes()));
        let asset_pk = asset.verifying_key().to_bytes();
        let uri = format!("{}/{}.json", self.uri, kind);
        let a: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let ix = receipt::core_create(&asset_pk, &s.signer.pubkey(), &owner_key, &name, &uri, &a);
        let bh = rpc(&self.agent, &s.rpc, "getLatestBlockhash", json!([{ "commitment": "finalized" }]))?;
        let blockhash: [u8; 32] = bh["value"]["blockhash"].as_str().and_then(zyn_custody::solana::pubkey).ok_or("no blockhash")?;
        let msg = sol_pool::message(&s.signer.pubkey(), &[sol_pool::compute_limit(200_000), ix], &blockhash);
        use ed25519_dalek::Signer as _;
        let payer_sig = s.signer.sign(&msg);
        let asset_sig = asset.sign(&msg).to_bytes();
        let raw = receipt::signed(&msg, &[payer_sig, asset_sig]);
        let b64 = zynzapd::sol_bridge::base64_encode(&raw);
        let sent = rpc(&self.agent, &s.rpc, "sendTransaction", json!([b64, { "encoding": "base64", "preflightCommitment": "confirmed" }]))?;
        Ok(json!({ "chain": "sol", "kind": kind, "receipt": zyn_custody::solana::base58_encode(&asset_pk), "tx": sent, "owner": owner }))
    }

    fn mint_evm(&self, kind: &str, txh: &str, body: &Value) -> Result<Value, String> {
        let e = self.evm.as_ref().ok_or("no Ethereum receipts here")?;
        let deposit_tx: [u8; 32] = unhex(txh).and_then(|b| b.try_into().ok()).ok_or("tx: a 32-byte transaction hash")?;
        let rcpt = rpc(&self.agent, &e.rpc, "eth_getTransactionReceipt", json!([txh]))?;
        if rcpt.is_null() || qty(&rcpt["status"]) != Some(1) {
            return Err("that transaction has not succeeded on this chain".into());
        }
        let fin = rpc(&self.agent, &e.rpc, "eth_getBlockByNumber", json!(["finalized", false]))?;
        if qty(&rcpt["blockNumber"]).unwrap_or(u128::MAX) > qty(&fin["number"]).unwrap_or(0) {
            return Err("that transaction is not final yet; try again in a few minutes".into());
        }
        let from: Address20 = rcpt["from"].as_str().and_then(unhex).and_then(|b| b.try_into().ok()).ok_or("the receipt names no sender")?;
        let logs: Vec<(Address20, Vec<[u8; 32]>, Vec<u8>)> = rcpt["logs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| {
                let at: Address20 = unhex(l["address"].as_str()?)?.try_into().ok()?;
                let topics = l["topics"].as_array()?.iter().filter_map(|t| unhex(t.as_str()?)?.try_into().ok()).collect();
                Some((at, topics, unhex(l["data"].as_str().unwrap_or("0x"))?))
            })
            .collect();
        let data = if kind == "transparent" {
            let wad = logs
                .iter()
                .filter(|(at, _, _)| *at == e.vault)
                .find_map(|(_, t, d)| eth::decode_deposited(t, d).map(|(_, _, wad, _)| wad))
                .ok_or("that transaction is not a deposit into the ETH bridge")?;
            receipt::mint_transparent_call(&from, &deposit_tx, "ETH", &e.name, wad, 18)
        } else {
            let in_pool = logs.iter().any(|(at, t, d)| e.pools.contains(at) && matches!(evm_pool::decode(t, d), Some(Event::Note { exit: false, .. })));
            if !in_pool {
                return Err("that transaction puts no note into a pool".into());
            }
            let (hash, to, signature) = Self::private_parts(body)?;
            let to_addr: Address20 = unhex(&to).and_then(|b| b.try_into().ok()).ok_or("to: a 0x address")?;
            let text = private_request_text("evm", txh, &to, &hash);
            if recover_personal(text.as_bytes(), &signature)? != from {
                return Err("the depositing wallet did not sign this request".into());
            }
            receipt::mint_private_call(&to_addr, &hash, &e.name)
        };
        let me = format!("0x{}", hex(&eth::address_of(&e.key)));
        let to = format!("0x{}", hex(&e.contract));
        let gas = qty(&rpc(&self.agent, &e.rpc, "eth_estimateGas", json!([{ "from": me, "to": to, "data": format!("0x{}", hex(&data)) }]))?).ok_or("no gas estimate")?;
        let price = qty(&rpc(&self.agent, &e.rpc, "eth_gasPrice", json!([]))?).ok_or("no gas price")?;
        let nonce = qty(&rpc(&self.agent, &e.rpc, "eth_getTransactionCount", json!([me, "pending"]))?).ok_or("no nonce")?;
        let raw = Tx { nonce: nonce as u64, gas_price: price * 12 / 10, gas: (gas * 13 / 10) as u64, to: e.contract, value: 0, data, chain_id: e.chain_id }.signed(&e.key)?;
        let sent = rpc(&self.agent, &e.rpc, "eth_sendRawTransaction", json!([format!("0x{}", hex(&raw))]))?;
        Ok(json!({ "chain": "evm", "kind": kind, "contract": eth::checksummed(&e.contract), "tx": sent }))
    }
}

fn keyring_id(body: &Value) -> Result<String, String> {
    // `key`, not `id`: a JSON-RPC request's `id` is the request's own.
    body["key"].as_str().filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).map(|s| s.to_ascii_lowercase()).ok_or_else(|| "key: 32 bytes of hex".to_string())
}

/// What an account signs to store `blob` under `id` (the Zyn app signs the
/// same bytes: `zyn-keyring.put.v1` ‖ id ‖ blob, hashed).
fn keyring_put_digest(id: &str, blob: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(b"zyn-keyring.put.v1");
    h.update(unhex(id).unwrap_or_default());
    h.update(blob);
    h.finalize().into()
}

fn keyring_delete_digest(id: &str) -> [u8; 32] {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(b"zyn-keyring.delete.v1");
    h.update(unhex(id).unwrap_or_default());
    h.finalize().into()
}

/// The address that signed `msg` with `personal_sign` (EIP-191).
fn recover_personal(msg: &[u8], sig: &[u8]) -> Result<Address20, String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    if sig.len() != 65 {
        return Err("signature: 65 bytes".into());
    }
    let prefix = format!("\x19Ethereum Signed Message:\n{}", msg.len());
    let digest = zyn_vm::eip712::keccak(&[prefix.as_bytes(), msg]);
    let s = Signature::from_slice(&sig[..64]).map_err(|_| "a malformed signature")?;
    let v = if sig[64] >= 27 { sig[64] - 27 } else { sig[64] };
    let id = RecoveryId::from_byte(v).ok_or("a malformed recovery id")?;
    let vk = VerifyingKey::recover_from_prehash(&digest, &s, id).map_err(|_| "the signature does not recover")?;
    let point = vk.to_encoded_point(false);
    let h = zyn_vm::eip712::keccak(&[&point.as_bytes()[1..]]);
    Ok(h[12..].try_into().unwrap())
}

fn serve(svc: Service, at: &str) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = match std::net::TcpListener::bind(at) {
        Ok(l) => l,
        Err(e) => return eprintln!("zyn-receipts: cannot listen on {at}: {e}"),
    };
    println!("receipts: listening on {at}");
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
            Ok(v) if ok => match svc.handle(&v) {
                Ok(r) => ("200 OK", r.to_string()),
                Err(e) => ("400 Bad Request", e),
            },
            _ => ("400 Bad Request", "a JSON request".into()),
        };
        let mut s = &stream;
        let _ = write!(s, "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}", text.len());
    }
}

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    let result = service().and_then(|svc| match cmd.as_str() {
        "serve" => {
            serve(svc, &env("ZYN_RECEIPTS_LISTEN", "127.0.0.1:8494"));
            Ok(())
        }
        "status" => {
            println!("{}", svc.handle(&json!({ "method": "zyn_receiptInfo" }))?);
            println!("minted {}", svc.minted().len());
            Ok(())
        }
        other => Err(format!("unknown command {other}: serve | status")),
    });
    if let Err(e) = result {
        eprintln!("zyn-receipts {cmd}: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(dir: &str) -> Service {
        let state = std::env::temp_dir().join(format!("zyn-keyring-{dir}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        std::fs::create_dir_all(&state).unwrap();
        Service { agent: ureq::AgentBuilder::new().build(), state, uri: String::new(), sol: None, evm: None, lock: Mutex::new(()) }
    }

    /// The keyring stores ciphertext under an id, signed by the account; only
    /// that account replaces or removes it; anyone with the id reads it.
    #[test]
    fn a_link_is_written_and_removed_only_by_its_account() {
        use ed25519_dalek::Signer as _;
        let s = svc("link");
        let a = ed25519_dalek::SigningKey::from_bytes(&[1; 32]);
        let b = ed25519_dalek::SigningKey::from_bytes(&[2; 32]);
        let id = "ab".repeat(32);
        let blob = vec![7u8; 72];
        let put = |k: &ed25519_dalek::SigningKey, blob: &[u8]| json!({ "method": "zyn_keyringPut", "key": id, "blob": hex(blob), "account": hex(&k.verifying_key().to_bytes()), "signature": hex(&k.sign(&keyring_put_digest(&id, blob)).to_bytes()) });
        assert!(s.handle(&json!({ "method": "zyn_keyringGet", "key": id })).unwrap().is_null());
        // Signed by someone else than the account it names: refused.
        let mut forged = put(&a, &blob);
        forged["account"] = json!(hex(&b.verifying_key().to_bytes()));
        assert!(s.handle(&forged).is_err());
        s.handle(&put(&a, &blob)).unwrap();
        assert_eq!(s.handle(&json!({ "method": "zyn_keyringGet", "key": id })).unwrap()["blob"], json!(hex(&blob)));
        // Another account cannot take it over.
        assert!(s.handle(&put(&b, &[9u8; 72])).unwrap_err().contains("another account"));
        // Its own account removes it.
        let del = json!({ "method": "zyn_keyringDelete", "key": id, "account": hex(&a.verifying_key().to_bytes()), "signature": hex(&a.sign(&keyring_delete_digest(&id)).to_bytes()) });
        s.handle(&del).unwrap();
        assert!(s.handle(&json!({ "method": "zyn_keyringGet", "key": id })).unwrap().is_null());
    }

    /// A personal_sign signature recovers to the address that made it.
    #[test]
    fn a_personal_signature_recovers_its_signer() {
        let key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let msg = b"Zyn private receipt";
        let prefix = format!("\x19Ethereum Signed Message:\n{}", msg.len());
        let digest = zyn_vm::eip712::keccak(&[prefix.as_bytes(), msg]);
        let sig = eth::sign(&key, &digest).unwrap();
        assert_eq!(recover_personal(msg, &sig).unwrap(), eth::address_of(&key));
        assert_ne!(recover_personal(b"another message", &sig).unwrap(), eth::address_of(&key));
    }
}
