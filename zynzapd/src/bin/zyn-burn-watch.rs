//! `zyn-burn-watch` — Solana burns of the promotional token become Collection
//! Z tickets on Zyn (DECISIONS §145).
//!
//! ```text
//!   zyn-burn-watch status    the rate, the cap, how many burn tickets are left
//!   zyn-burn-watch cap <n>   SetVoucherCap: how many burn tickets the sale takes
//!   zyn-burn-watch once      post every finalized burn since the cursor, then stop
//!   zyn-burn-watch run       the same, every ZYN_BURN_POLL seconds
//! ```
//!
//! It only reads Solana. A burn is posted once whatever happens — the chain
//! refuses a burn id it has seen — so restarting, or losing the cursor, is
//! safe: the worst case is re-reading history.
//!
//! ```text
//!   ZYN_BURN_RPC         Solana JSON-RPC (https://api.mainnet-beta.solana.com)
//!   ZYN_BURN_MINT        the token's mint address                      (required)
//!   ZYN_BURN_PER_TICKET  base units burned per ticket                  (required)
//!   ZYN_BURN_NODE        the object ledger's JSON-RPC  (http://127.0.0.1:8599)
//!   ZYN_SUBMIT_TOKEN     the operator door's token (read from the node's env)
//!   ZYN_BURN_COLLECTION  the sale's collection                (Collection Z)
//!   ZYN_BURN_CURSOR      file holding the newest signature handled
//!                                            (/var/lib/zyn/burn-cursor)
//!   ZYN_BURN_POLL        seconds between reads in `run`                (30)
//! ```

use std::time::Duration;

use objectvm::{ObjectVm, Op};
use serde_json::{json, Value};
use zyn_vm::spec::MicrochainVm;
use zyn_vm::program;
use zynzapd::burn_watch::{burn_id, decode_burn, tickets_for};

fn env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

struct Watch {
    rpc: String,
    mint: String,
    per_ticket: u64,
    node: String,
    collection: [u8; 32],
    cursor: std::path::PathBuf,
    agent: ureq::Agent,
}

fn watch() -> Result<Watch, String> {
    let mint = env("ZYN_BURN_MINT", "");
    if mint.is_empty() {
        return Err("set ZYN_BURN_MINT to the token's mint address".into());
    }
    let per_ticket: u64 = env("ZYN_BURN_PER_TICKET", "").parse().map_err(|_| "set ZYN_BURN_PER_TICKET to the base units one ticket costs".to_string())?;
    if per_ticket == 0 {
        return Err("ZYN_BURN_PER_TICKET must be above zero".into());
    }
    let collection = match std::env::var("ZYN_BURN_COLLECTION").ok().filter(|v| !v.is_empty()) {
        None => zynzapd::cave_z::collection_id(),
        Some(h) => {
            let b: Vec<u8> = (0..h.len() / 2).filter_map(|i| u8::from_str_radix(h.get(i * 2..i * 2 + 2)?, 16).ok()).collect();
            <[u8; 32]>::try_from(b).map_err(|_| "ZYN_BURN_COLLECTION must be 64 hex characters".to_string())?
        }
    };
    Ok(Watch {
        rpc: env("ZYN_BURN_RPC", "https://api.mainnet-beta.solana.com"),
        mint,
        per_ticket,
        node: env("ZYN_BURN_NODE", "http://127.0.0.1:8599"),
        collection,
        cursor: env("ZYN_BURN_CURSOR", "/var/lib/zyn/burn-cursor").into(),
        agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
    })
}

impl Watch {
    fn solana(&self, method: &str, params: Value) -> Result<Value, String> {
        for attempt in 0..4 {
            match self.agent.post(&self.rpc).send_json(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })) {
                Ok(r) => {
                    let v: Value = r.into_json().map_err(|_| "Solana answered something that is not JSON".to_string())?;
                    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
                        return Err(format!("Solana refused {method}: {e}"));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
                Err(ureq::Error::Status(429, _)) if attempt < 3 => std::thread::sleep(Duration::from_millis(500 << attempt)),
                Err(e) => return Err(format!("Solana is not answering: {e}")),
            }
        }
        Err("Solana kept rate-limiting".into())
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

    /// Finalized signatures touching the mint since the cursor, oldest first,
    /// with the memo the RPC reports for each — only those with a `zyn:` memo
    /// are fetched in full.
    fn new_signatures(&self) -> Result<Vec<String>, String> {
        let until = std::fs::read_to_string(&self.cursor).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let mut out = Vec::new();
        let mut before: Option<String> = None;
        loop {
            let mut opts = json!({ "commitment": "finalized", "limit": 1000 });
            if let Some(u) = &until {
                opts["until"] = json!(u);
            }
            if let Some(b) = &before {
                opts["before"] = json!(b);
            }
            let page = self.solana("getSignaturesForAddress", json!([self.mint, opts]))?;
            let rows = page.as_array().cloned().unwrap_or_default();
            for r in &rows {
                let (Some(sig), memo) = (r["signature"].as_str(), r["memo"].as_str().unwrap_or("")) else { continue };
                if r["err"].is_null() && memo.contains("zyn:") {
                    out.push(sig.to_string());
                }
            }
            if rows.len() < 1000 {
                break;
            }
            before = rows.last().and_then(|r| r["signature"].as_str()).map(String::from);
        }
        // The newest signature seen at all moves the cursor, burn or not.
        out.reverse();
        Ok(out)
    }

    fn newest(&self) -> Result<Option<String>, String> {
        let page = self.solana("getSignaturesForAddress", json!([self.mint, { "commitment": "finalized", "limit": 1 }]))?;
        Ok(page.get(0).and_then(|r| r["signature"].as_str()).map(String::from))
    }

    fn once(&self) -> Result<(), String> {
        let Some(sale) = self.sale()? else {
            return Err("there is no sale for this collection yet".into());
        };
        // Read the head first: anything after it is next round's.
        let head = self.newest()?;
        let open = sale["phase"].as_str() == Some("open");
        for sig in self.new_signatures()? {
            let Some(tx) = Some(self.solana("getTransaction", json!([sig, { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0, "commitment": "finalized" }]))?).filter(|v| !v.is_null()) else {
                continue;
            };
            let Some(b) = decode_burn(&tx, &self.mint) else { continue };
            let count = tickets_for(b.amount, self.per_ticket);
            if count == 0 {
                println!("{sig}: burned {} — less than one ticket", b.amount);
                continue;
            }
            if !open {
                println!("{sig}: burned {} after the sale closed — no tickets", b.amount);
                continue;
            }
            let receipts = self.submit(&Op::PostVoucher {
                program: program::sale_vouchers(),
                collection: self.collection,
                burn: burn_id(&sig),
                commitment: b.commitment,
                count,
            })?;
            if receipts.contains("already posted") {
                continue;
            }
            if receipts.contains("Refused") {
                // Cap reached, a reused commitment: said once, and the burn
                // stays unposted — the chain is the record, not this log.
                println!("{sig}: not posted — {}", receipts.chars().take(160).collect::<String>());
                continue;
            }
            println!("{sig}: {count} ticket(s) for burn {}", hex(&burn_id(&sig)));
        }
        if let Some(h) = head {
            std::fs::write(&self.cursor, h).map_err(|e| format!("{}: {e}", self.cursor.display()))?;
        }
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("status");
    let result = watch().and_then(|w| match cmd {
        "status" => {
            let sale = w.sale()?;
            println!("mint        {}", w.mint);
            println!("per ticket  {} base units", w.per_ticket);
            println!("collection  {}", hex(&w.collection));
            match sale {
                None => println!("no sale yet"),
                Some(s) => println!(
                    "sale        {} — burn tickets {} of {}",
                    s["phase"].as_str().unwrap_or("?"),
                    s["voucher_tickets"],
                    s["voucher_cap"]
                ),
            }
            Ok(())
        }
        "cap" => {
            let cap: u32 = args.get(2).and_then(|n| n.parse().ok()).ok_or("cap <number of burn tickets>")?;
            let r = w.submit(&Op::SetVoucherCap { program: program::sale_vouchers(), collection: w.collection, cap })?;
            println!("cap: {}", r.chars().take(160).collect::<String>());
            Ok(())
        }
        "once" => w.once(),
        "run" => {
            let every: u64 = env("ZYN_BURN_POLL", "30").parse().unwrap_or(30);
            loop {
                if let Err(e) = w.once() {
                    eprintln!("zyn-burn-watch: {e}");
                }
                std::thread::sleep(Duration::from_secs(every));
            }
        }
        other => Err(format!("unknown command {other}: status | cap <n> | once | run")),
    });
    if let Err(e) = result {
        eprintln!("zyn-burn-watch {cmd}: {e}");
        std::process::exit(1);
    }
}
