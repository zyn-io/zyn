//! `zyn-evm-burn-watch` — burns of a token on an EVM chain become Collection Z
//! sale credit on Zyn (DECISIONS §182). Built for REGI on Arc.
//!
//! ```text
//!   zyn-evm-burn-watch status   the price, the budget, where the cursor is
//!   zyn-evm-burn-watch once     post every finalized burn since the cursor, then stop
//!   zyn-evm-burn-watch run      the same, every ZYN_BURN_POLL seconds
//! ```
//!
//! A burn is a call of the token's own `transfer` to `0x…dEaD`, with the
//! burner's commitment appended to the calldata as one more 32-byte word —
//! the token ignores it, the watcher reads it from the transaction. The
//! transaction is the proof: anyone can fetch it and check the amount, the
//! commitment and the burn id, `H("objectvm.evm-burn.v1", chain, tx, log)`.
//! The chain records a burn once, so a second run posts nothing twice.
//!
//! A burn is valued in USD: the token's price from its Uniswap v4 pool
//! (read from the PoolManager's storage, `slot0`), over ZEC's from the
//! exchanges Zyn Lend reads, two of which must agree; at the lower median
//! of the last hour's readings.
//!
//! ```text
//!   ZYN_EVM_RPC          JSON-RPC       (https://rpc.quicknode.mainnet.arc.io)
//!   ZYN_EVM_CHAIN_ID     the chain's id, checked against the RPC      (5042)
//!   ZYN_EVM_TOKEN        the token burned                          (required)
//!   ZYN_EVM_DECIMALS     its decimal places                             (18)
//!   ZYN_EVM_PRICE        v4:<pool manager>:<pool id>:<quote>:<quote decimals>
//!                        — the token's pool against a USD stablecoin; or
//!                        usd:<price> for a test token with no market
//!   ZYN_EVM_FROM_BLOCK   where reading starts when there is no cursor (required then)
//!   ZYN_EVM_STEP         blocks per log query                       (10000)
//!   ZYN_EVM_CURSOR       the last block read      (/var/lib/zyn/evm-burn-cursor)
//!   ZYN_EVM_PRICES       the hour's readings  (/var/lib/zyn/evm-burn-prices.json)
//!   ZYN_BURN_NODE        the object ledger's JSON-RPC  (http://127.0.0.1:8599)
//!   ZYN_SUBMIT_TOKEN     the operator door's token (read from the node's env)
//!   ZYN_BURN_COLLECTION  the sale's collection                (Collection Z)
//!   ZYN_BURN_POLL        seconds between reads in `run`                  (30)
//! ```

use std::time::Duration;

use objectvm::{ObjectVm, Op};
use serde_json::{json, Value};
use zyn_vm::program;
use zyn_vm::spec::MicrochainVm;
use zynzapd::burn_credit::{self, EVM_DEAD, TRANSFER_TOPIC};
use zynzapd::feeds::{self, Fetcher};

fn env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()).collect()
}

fn address(s: &str) -> Result<String, String> {
    let a = s.trim().to_ascii_lowercase();
    let bare = a.strip_prefix("0x").unwrap_or(&a);
    if bare.len() != 40 || unhex(bare).is_none() {
        return Err(format!("{s} is not an address"));
    }
    Ok(format!("0x{bare}"))
}

/// Where the token's USD price comes from.
enum Price {
    /// A Uniswap v4 pool against a USD stablecoin, read from the PoolManager.
    V4 { manager: String, pool: [u8; 32], token_is_0: bool, quote_decimals: i32 },
    /// A fixed price, for a test token with no market.
    Usd(f64),
}

struct Watch {
    rpc: String,
    chain_id: u64,
    token: String,
    decimals: u32,
    price: Price,
    from_block: Option<u64>,
    step: u64,
    cursor: std::path::PathBuf,
    prices: std::path::PathBuf,
    node: String,
    collection: [u8; 32],
    agent: ureq::Agent,
}

fn watch() -> Result<Watch, String> {
    let token = address(&env("ZYN_EVM_TOKEN", "")).map_err(|_| "set ZYN_EVM_TOKEN to the token's address".to_string())?;
    let spec = env("ZYN_EVM_PRICE", "");
    let price = if let Some(p) = spec.strip_prefix("usd:") {
        Price::Usd(p.parse().ok().filter(|x: &f64| *x > 0.0).ok_or("usd:<price> needs a price above nothing")?)
    } else if let Some(rest) = spec.strip_prefix("v4:") {
        let parts: Vec<&str> = rest.split(':').collect();
        let [manager, pool, quote, quote_decimals] = parts[..] else {
            return Err("ZYN_EVM_PRICE is v4:<pool manager>:<pool id>:<quote>:<quote decimals>".into());
        };
        let pool: [u8; 32] = unhex(pool).and_then(|b| b.try_into().ok()).ok_or("the pool id is 32 bytes of hex")?;
        let quote = address(quote)?;
        Price::V4 {
            manager: address(manager)?,
            pool,
            // A v4 pool sorts its two currencies by address.
            token_is_0: token < quote,
            quote_decimals: quote_decimals.parse().map_err(|_| "the quote's decimals are a number")?,
        }
    } else {
        return Err("set ZYN_EVM_PRICE to v4:<pool manager>:<pool id>:<quote>:<quote decimals>, or usd:<price> for a test token".into());
    };
    let collection = match std::env::var("ZYN_BURN_COLLECTION").ok().filter(|v| !v.is_empty()) {
        None => zynzapd::cave_z::collection_id(),
        Some(h) => unhex(&h).and_then(|b| b.try_into().ok()).ok_or("ZYN_BURN_COLLECTION must be 64 hex characters")?,
    };
    Ok(Watch {
        rpc: env("ZYN_EVM_RPC", "https://rpc.quicknode.mainnet.arc.io"),
        chain_id: env("ZYN_EVM_CHAIN_ID", "5042").parse().map_err(|_| "ZYN_EVM_CHAIN_ID must be a number".to_string())?,
        token,
        decimals: env("ZYN_EVM_DECIMALS", "18").parse().map_err(|_| "ZYN_EVM_DECIMALS must be a number".to_string())?,
        price,
        from_block: std::env::var("ZYN_EVM_FROM_BLOCK").ok().and_then(|b| b.parse().ok()),
        step: env("ZYN_EVM_STEP", "10000").parse().map_err(|_| "ZYN_EVM_STEP must be a number".to_string())?,
        cursor: env("ZYN_EVM_CURSOR", "/var/lib/zyn/evm-burn-cursor").into(),
        prices: env("ZYN_EVM_PRICES", "/var/lib/zyn/evm-burn-prices.json").into(),
        node: env("ZYN_BURN_NODE", "http://127.0.0.1:8599"),
        collection,
        agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
    })
}

impl Watch {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        for attempt in 0..5 {
            match self.agent.post(&self.rpc).send_json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })) {
                Ok(r) => {
                    let v: Value = r.into_json().map_err(|_| "the RPC answered something that is not JSON".to_string())?;
                    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
                        return Err(format!("the RPC refused {method}: {e}"));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
                Err(ureq::Error::Status(429, _)) if attempt < 4 => std::thread::sleep(Duration::from_millis(500 << attempt)),
                Err(e) => return Err(format!("the RPC is not answering: {e}")),
            }
        }
        Err("the RPC kept rate-limiting".into())
    }

    fn finalized(&self) -> Result<u64, String> {
        let b = self.call("eth_getBlockByNumber", json!(["finalized", false]))?;
        b["number"].as_str().and_then(|n| u64::from_str_radix(n.trim_start_matches("0x"), 16).ok()).ok_or_else(|| "the RPC has no finalized block".into())
    }

    fn sale(&self) -> Result<Option<Value>, String> {
        let m = zynzapd::object_client::call(&self.node, "zyn_market", json!({}))?;
        Ok(m.get("sales").and_then(Value::as_array).and_then(|a| a.iter().find(|s| s["collection"].as_str() == Some(&hex(&self.collection))).cloned()))
    }

    fn submit(&self, op: &Op) -> Result<String, String> {
        let token = std::env::var("ZYN_SUBMIT_TOKEN").map_err(|_| "set ZYN_SUBMIT_TOKEN (read it from the node's env file)".to_string())?;
        let r = zynzapd::object_client::call(&self.node, "zyn_submit", json!({ "token": token, "intent": hex(&ObjectVm::encode_intent(op)) }))?;
        Ok(r.get("receipts").and_then(Value::as_str).unwrap_or("").to_string())
    }

    /// The token's USD price now.
    fn token_usd(&self) -> Result<f64, String> {
        match &self.price {
            Price::Usd(p) => Ok(*p),
            Price::V4 { manager, pool, token_is_0, quote_decimals } => {
                // StateLibrary: a pool's state lives at keccak(id ‖ 6), slot0 first.
                let mut six = [0u8; 32];
                six[31] = 6;
                let slot = zyn_vm::eip712::keccak(&[pool, &six]);
                let data = format!("0x1e2eaeaf{}", hex(&slot));
                let r = self.call("eth_call", json!([{ "to": manager, "data": data }, "finalized"]))?;
                let word: [u8; 32] = r.as_str().and_then(unhex).and_then(|b| b.try_into().ok()).ok_or("the PoolManager answered no word")?;
                burn_credit::v4_price(&word, *token_is_0, self.decimals as i32, *quote_decimals).ok_or_else(|| "the pool has no price".into())
            }
        }
    }

    /// ZEC's USD price: Coinbase, Kraken and CoinGecko, two agreeing.
    fn zec_usd(&self) -> Result<f64, String> {
        let http = feeds::Http::default();
        let sources = feeds::lending_sources("ZEC");
        let ids: Vec<String> = sources.iter().filter_map(|s| if let feeds::Source::CoinGecko(id) = s { Some(id.clone()) } else { None }).collect();
        http.prime(&ids);
        let samples: Vec<f64> = sources.iter().filter_map(|s| http.fetch(s).ok()).collect();
        feeds::aggregate_quorum(samples.clone(), 0.02, 2).ok_or_else(|| format!("no two ZEC price sources agree ({samples:?})"))
    }

    /// The token's price in ZEC (WAD per whole token) now, and the lower
    /// median of the hour's readings burns are valued at (None until enough).
    fn credit_price(&self) -> Result<(u128, Option<u128>), String> {
        let (t, z) = (self.token_usd()?, self.zec_usd()?);
        let spot = burn_credit::wad(t / z).ok_or("the token is worth nothing in ZEC")?;
        // A fixed test price moves only with ZEC: still an hour's median.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        Ok((spot, burn_credit::record(&self.prices, now, spot)?))
    }

    fn cursor(&self) -> Option<u64> {
        std::fs::read_to_string(&self.cursor).ok().and_then(|s| s.trim().parse().ok())
    }

    fn once(&self) -> Result<(), String> {
        let chain = self.call("eth_chainId", json!([]))?;
        if chain.as_str().and_then(|c| u64::from_str_radix(c.trim_start_matches("0x"), 16).ok()) != Some(self.chain_id) {
            return Err(format!("the RPC is not chain {} ({chain})", self.chain_id));
        }
        let Some(sale) = self.sale()? else {
            return Err("there is no sale for this collection yet".into());
        };
        // The price first: without one nothing is read and the cursor stays.
        let (_, median) = self.credit_price()?;
        let Some(price) = median else {
            return Err(format!("valuing burns needs {} readings of the price; still gathering", burn_credit::MIN_READINGS));
        };
        let start = match self.cursor() {
            Some(c) => c + 1,
            None => self.from_block.ok_or("set ZYN_EVM_FROM_BLOCK for the first run")?,
        };
        let head = self.finalized()?;
        let over = sale["phase"].as_str() == Some("openmint") && sale["open_next"].as_u64() >= sale["open_count"].as_u64();
        let mut budget: i128 = sale["credit_budget"].as_str().and_then(|b| b.parse().ok()).unwrap_or(0);
        let dead_topic = format!("0x{}{EVM_DEAD}", "0".repeat(24));
        let mut a = start;
        while a <= head {
            let b = head.min(a + self.step - 1);
            let logs = self.call(
                "eth_getLogs",
                json!([{ "address": self.token, "fromBlock": format!("0x{a:x}"), "toBlock": format!("0x{b:x}"), "topics": [format!("0x{TRANSFER_TOPIC}"), Value::Null, dead_topic] }]),
            )?;
            for log in logs.as_array().cloned().unwrap_or_default() {
                let Some(hash) = log["transactionHash"].as_str() else { continue };
                let tx = self.call("eth_getTransactionByHash", json!([hash]))?;
                let Some(burn) = burn_credit::decode_evm_burn(&log, &tx, &self.token) else {
                    println!("{hash}: a transfer to 0x…dEaD without a commitment — not a burn for credit");
                    continue;
                };
                // A transaction that reverted emits no logs, but check anyway.
                let receipt = self.call("eth_getTransactionReceipt", json!([hash]))?;
                if receipt["status"].as_str() != Some("0x1") {
                    continue;
                }
                let id = burn_credit::evm_burn_id(self.chain_id, &burn.tx_hash, burn.log_index);
                let Some(worth) = burn_credit::credit_for(burn.amount, self.decimals, price) else {
                    println!("{hash}: burned {} — worth nothing at the price", burn.amount);
                    continue;
                };
                if over {
                    println!("{hash}: burned {} after the sale ended — no credit", burn.amount);
                    continue;
                }
                // Past the budget, a burn gets what is left of it (§182.4).
                let amount = worth.min(budget);
                if amount <= 0 {
                    println!("{hash}: burned {} with the sale's credit budget spent — no credit", burn.amount);
                    continue;
                }
                let receipts = self.submit(&Op::PostCredit {
                    program: program::sale_vouchers(),
                    collection: self.collection,
                    burn: id,
                    commitment: burn.commitment,
                    amount: zyn_vm::Fixed(amount),
                })?;
                if receipts.contains("already posted") {
                    continue;
                }
                if receipts.contains("Refused") {
                    println!("{hash}: not posted — {}", receipts.chars().take(160).collect::<String>());
                    continue;
                }
                budget -= amount;
                println!("{hash}: {amount} credit (of {worth}) for burn {}", hex(&id));
            }
            std::fs::write(&self.cursor, b.to_string()).map_err(|e| format!("{}: {e}", self.cursor.display()))?;
            a = b + 1;
        }
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("status");
    let result = watch().and_then(|w| match cmd {
        "status" => {
            println!("token       {} on chain {}", w.token, w.chain_id);
            match (w.token_usd(), w.zec_usd()) {
                (Ok(t), Ok(z)) => println!("price       ${t:.10} a token, ZEC ${z:.2}"),
                (t, z) => println!("price       unknown: {:?} {:?}", t.err(), z.err()),
            }
            match w.cursor() {
                Some(c) => println!("cursor      block {c}"),
                None => println!("cursor      none yet (starts at {:?})", w.from_block),
            }
            match w.sale()? {
                None => println!("no sale yet"),
                Some(s) => println!("sale        {} — credit budget {} (ZEC-wad)", s["phase"].as_str().unwrap_or("?"), s["credit_budget"].as_str().unwrap_or("0")),
            }
            Ok(())
        }
        "once" => w.once(),
        "run" => {
            let every: u64 = env("ZYN_BURN_POLL", "30").parse().unwrap_or(30);
            loop {
                if let Err(e) = w.once() {
                    eprintln!("zyn-evm-burn-watch: {e}");
                }
                std::thread::sleep(Duration::from_secs(every));
            }
        }
        other => Err(format!("unknown command {other}: status | once | run")),
    });
    if let Err(e) = result {
        eprintln!("zyn-evm-burn-watch {cmd}: {e}");
        std::process::exit(1);
    }
}
