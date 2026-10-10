//! `zyn-burn-watch` — Solana burns of the promotional token become Collection
//! Z tickets on Zyn (DECISIONS §145), or, with `ZYN_BURN_POST=credit`, sale
//! credit worth what they burned (§182).
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
//!   ZYN_BURN_POOL        the token's pool against bridged ZEC on Solana:
//!                        <token vault>,<ZEC vault> (its two reserve accounts).
//!                        Set, a ticket costs one ticket price in ZEC at the
//!                        median of the last hour's pool readings
//!   ZYN_BURN_ZEC_DECIMALS  bridged ZEC's decimal places                 (8)
//!   ZYN_BURN_PRICES      file holding the last hour's readings
//!                                         (/var/lib/zyn/burn-prices.json)
//!   ZYN_BURN_PER_TICKET  base units burned per ticket, when no pool is set
//!   ZYN_BURN_POST        tickets (§145) or credit (§182)             (tickets)
//!   ZYN_BURN_CURVE       credit: the token's pump.fun bonding curve account,
//!                        priced at its virtual reserves (instead of a pool)
//!   ZYN_BURN_DECIMALS    credit: the token's decimal places              (6)
//!   ZYN_BURN_ZEC_PER_TOKEN  credit: a fixed price in ZEC for a test token
//!                        with no market (testnet only)
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
use zynzapd::burn_credit;
use zynzapd::burn_watch::{burn_id, decode_burn, tickets_for};

fn env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// How a ticket is priced: a fixed number of base units, or the token's
/// price in bridged ZEC from its pool.
enum Rate {
    Fixed(u64),
    Pool { token_vault: String, zec_vault: String, zec_decimals: u32, prices: std::path::PathBuf },
    /// A pump.fun bonding curve against bridged ZEC (credit only).
    Curve { account: String, zec_decimals: u32, prices: std::path::PathBuf },
    /// A fixed price in ZEC (WAD per whole token), for a test token.
    ZecPerToken(u128),
}

/// The window the median rate is taken over.
const PRICE_WINDOW_SECS: u64 = 3_600;

struct Watch {
    rpc: String,
    mint: String,
    rate: Rate,
    /// Post credit (§182) rather than tickets.
    credit: bool,
    decimals: u32,
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
    let pool = env("ZYN_BURN_POOL", "");
    let curve = env("ZYN_BURN_CURVE", "");
    let credit = match env("ZYN_BURN_POST", "tickets").as_str() {
        "tickets" => false,
        "credit" => true,
        other => return Err(format!("ZYN_BURN_POST is tickets or credit, not {other}")),
    };
    let zec_decimals: u32 = env("ZYN_BURN_ZEC_DECIMALS", "8").parse().map_err(|_| "ZYN_BURN_ZEC_DECIMALS must be a number".to_string())?;
    let fixed_price = env("ZYN_BURN_ZEC_PER_TOKEN", "");
    let rate = if !fixed_price.is_empty() {
        if !credit {
            return Err("ZYN_BURN_ZEC_PER_TOKEN prices credit; set ZYN_BURN_POST=credit".into());
        }
        let p: f64 = fixed_price.parse().map_err(|_| "ZYN_BURN_ZEC_PER_TOKEN is a number of ZEC".to_string())?;
        Rate::ZecPerToken(burn_credit::wad(p).ok_or("ZYN_BURN_ZEC_PER_TOKEN must be above nothing")?)
    } else if !curve.is_empty() {
        if !credit {
            return Err("ZYN_BURN_CURVE prices credit; set ZYN_BURN_POST=credit".into());
        }
        Rate::Curve { account: curve, zec_decimals, prices: env("ZYN_BURN_PRICES", "/var/lib/zyn/burn-prices.json").into() }
    } else if pool.is_empty() {
        if credit {
            return Err("credit needs a price: set ZYN_BURN_CURVE or ZYN_BURN_POOL".into());
        }
        let per_ticket: u64 = env("ZYN_BURN_PER_TICKET", "").parse().map_err(|_| "set ZYN_BURN_POOL to the token's pool, or ZYN_BURN_PER_TICKET to the base units one ticket costs".to_string())?;
        if per_ticket == 0 {
            return Err("ZYN_BURN_PER_TICKET must be above zero".into());
        }
        Rate::Fixed(per_ticket)
    } else {
        let (token_vault, zec_vault) = pool.split_once(',').ok_or("ZYN_BURN_POOL is <token vault>,<ZEC vault>")?;
        Rate::Pool {
            token_vault: token_vault.trim().to_string(),
            zec_vault: zec_vault.trim().to_string(),
            zec_decimals,
            prices: env("ZYN_BURN_PRICES", "/var/lib/zyn/burn-prices.json").into(),
        }
    };
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
        rate,
        credit,
        decimals: env("ZYN_BURN_DECIMALS", "6").parse().map_err(|_| "ZYN_BURN_DECIMALS must be a number".to_string())?,
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

    /// A pool reserve, in base units.
    fn reserve(&self, account: &str) -> Result<u128, String> {
        let v = self.solana("getTokenAccountBalance", json!([account, { "commitment": "finalized" }]))?;
        v["value"]["amount"].as_str().and_then(|a| a.parse().ok()).ok_or_else(|| format!("{account} is not a token account"))
    }

    /// The pool's price for one ticket now, recorded with the readings of
    /// the last hour; returns the spot and the median the burns are priced at
    /// (None until there are enough readings).
    fn read_rate(&self, sale: &Value) -> Result<(u64, Option<u64>), String> {
        let Rate::Pool { token_vault, zec_vault, zec_decimals, prices } = &self.rate else { return Err("not priced by a pool".into()) };
        let ticket: u128 = sale["ticket_price"].as_str().and_then(|t| t.parse().ok()).ok_or("the sale has no ticket price")?;
        let spot = zynzapd::burn_watch::per_ticket_at(ticket, self.reserve(token_vault)?, self.reserve(zec_vault)?, *zec_decimals).ok_or("the pool is empty or prices a ticket under one base unit")?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut readings: Vec<(u64, u64)> = std::fs::read(prices).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        readings.retain(|(t, _)| now.saturating_sub(*t) <= PRICE_WINDOW_SECS);
        readings.push((now, spot));
        std::fs::write(prices, serde_json::to_vec(&readings).unwrap()).map_err(|e| format!("{}: {e}", prices.display()))?;
        let rates: Vec<u64> = readings.iter().map(|(_, r)| *r).collect();
        Ok((spot, zynzapd::burn_watch::median_rate(&rates)))
    }

    /// The token's price in ZEC (WAD per whole token) now, from the curve's
    /// virtual reserves or the pool's two balances.
    fn zec_per_token(&self) -> Result<u128, String> {
        match &self.rate {
            Rate::Curve { account, zec_decimals, .. } => {
                let v = self.solana("getAccountInfo", json!([account, { "encoding": "base64", "commitment": "finalized" }]))?;
                let data = v["value"]["data"][0].as_str().ok_or_else(|| format!("{account} is not an account"))?;
                let bytes = base64_decode(data).ok_or("the curve's data is not base64")?;
                let (price, complete) = burn_credit::pump_curve_price(&bytes, self.decimals, *zec_decimals).ok_or("the curve has no price")?;
                if complete {
                    return Err("the curve has completed; price the token from its pool (ZYN_BURN_POOL)".into());
                }
                Ok(price)
            }
            Rate::Pool { token_vault, zec_vault, zec_decimals, .. } => {
                let (t, z) = (self.reserve(token_vault)?, self.reserve(zec_vault)?);
                if t == 0 || *zec_decimals > 18 {
                    return Err("the pool is empty".into());
                }
                z.checked_mul(10u128.pow(18 - zec_decimals))
                    .and_then(|z| z.checked_mul(10u128.checked_pow(self.decimals)?))
                    .map(|n| n / t)
                    .filter(|p| *p > 0)
                    .ok_or_else(|| "the pool prices the token at nothing".to_string())
            }
            Rate::ZecPerToken(p) => Ok(*p),
            Rate::Fixed(_) => Err("credit needs a price".into()),
        }
    }

    /// The price burns are valued at this pass: the lower median of the
    /// hour's readings, None while still gathering.
    fn credit_price(&self) -> Result<(u128, Option<u128>), String> {
        let spot = self.zec_per_token()?;
        // A fixed test price needs no hour of readings.
        if let Rate::ZecPerToken(p) = self.rate {
            return Ok((p, Some(p)));
        }
        let (Rate::Curve { prices, .. } | Rate::Pool { prices, .. }) = &self.rate else { return Err("credit needs a price".into()) };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        Ok((spot, burn_credit::record(prices, now, spot)?))
    }

    /// One pass in credit mode (§182): every finalized burn since the cursor
    /// becomes credit worth what it burned, out of what is left of the budget.
    fn once_credit(&self, sale: &Value) -> Result<(), String> {
        let (_, median) = self.credit_price()?;
        // Too few readings yet: nothing is read and the cursor stays.
        let Some(price) = median else {
            return Err(format!("valuing burns needs {} readings of the price; still gathering", burn_credit::MIN_READINGS));
        };
        let head = self.newest()?;
        let over = sale["phase"].as_str() == Some("openmint") && sale["open_next"].as_u64() >= sale["open_count"].as_u64();
        let mut budget: i128 = sale["credit_budget"].as_str().and_then(|b| b.parse().ok()).unwrap_or(0);
        for sig in self.new_signatures()? {
            let Some(tx) = Some(self.solana("getTransaction", json!([sig, { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0, "commitment": "finalized" }]))?).filter(|v| !v.is_null()) else {
                continue;
            };
            let Some(b) = decode_burn(&tx, &self.mint) else { continue };
            let Some(worth) = burn_credit::credit_for(b.amount as u128, self.decimals, price) else {
                println!("{sig}: burned {} — worth nothing at the price", b.amount);
                continue;
            };
            if over {
                println!("{sig}: burned {} after the sale ended — no credit", b.amount);
                continue;
            }
            // Past the budget, a burn gets what is left of it (§182.4).
            let amount = worth.min(budget);
            if amount <= 0 {
                println!("{sig}: burned {} with the sale's credit budget spent — no credit", b.amount);
                continue;
            }
            let receipts = self.submit(&Op::PostCredit {
                program: program::sale_vouchers(),
                collection: self.collection,
                burn: burn_id(&sig),
                commitment: b.commitment,
                amount: zyn_vm::Fixed(amount),
            })?;
            if receipts.contains("already posted") {
                continue;
            }
            if receipts.contains("Refused") {
                println!("{sig}: not posted — {}", receipts.chars().take(160).collect::<String>());
                continue;
            }
            budget -= amount;
            println!("{sig}: {amount} credit (of {worth}) for burn {}", hex(&burn_id(&sig)));
        }
        if let Some(h) = head {
            std::fs::write(&self.cursor, h).map_err(|e| format!("{}: {e}", self.cursor.display()))?;
        }
        Ok(())
    }

    /// Base units one ticket costs for the burns this pass posts.
    fn per_ticket(&self, sale: &Value) -> Result<u64, String> {
        match &self.rate {
            Rate::Fixed(n) => Ok(*n),
            Rate::Curve { .. } | Rate::ZecPerToken(_) => Err("this prices credit, not tickets".into()),
            Rate::Pool { .. } => {
                let (_, median) = self.read_rate(sale)?;
                // Too few readings yet: the burns wait, unposted, for the next pass.
                median.ok_or_else(|| format!("pricing burns needs {} readings of the pool; still gathering", zynzapd::burn_watch::MIN_READINGS))
            }
        }
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
        if self.credit {
            return self.once_credit(&sale);
        }
        // The rate first: without one, nothing is read and the cursor stays,
        // so the burns wait for a priced pass.
        let per_ticket = self.per_ticket(&sale)?;
        // Read the head first: anything after it is next round's.
        let head = self.newest()?;
        let open = sale["phase"].as_str() == Some("open");
        for sig in self.new_signatures()? {
            let Some(tx) = Some(self.solana("getTransaction", json!([sig, { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0, "commitment": "finalized" }]))?).filter(|v| !v.is_null()) else {
                continue;
            };
            let Some(b) = decode_burn(&tx, &self.mint) else { continue };
            let count = tickets_for(b.amount, per_ticket);
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
            if w.credit {
                println!("posts       credit (§182)");
                match w.credit_price() {
                    Ok((spot, median)) => println!("price       {spot} ZEC-wad per token now; burns valued at {}", median.map_or("— (gathering readings)".into(), |m| format!("{m} (hour's lower median)"))),
                    Err(e) => println!("price       unknown: {e}"),
                }
                if let Some(s) = &sale {
                    println!("budget      {} (ZEC-wad left)", s["credit_budget"].as_str().unwrap_or("0"));
                }
            }
            match &w.rate {
                Rate::Curve { account, .. } => println!("curve       {account}"),
                Rate::ZecPerToken(p) => println!("fixed       {p} ZEC-wad per token (test)"),
                Rate::Fixed(n) => println!("per ticket  {n} base units (fixed)"),
                Rate::Pool { token_vault, zec_vault, .. } => {
                    println!("pool        token {token_vault} · ZEC {zec_vault}");
                    match sale.as_ref().map(|s| w.read_rate(s)) {
                        Some(Ok((spot, median))) => println!("per ticket  {spot} base units now; burns priced at {}", median.map_or("— (gathering readings)".into(), |m| format!("{m} (hour's median)"))),
                        Some(Err(e)) => println!("per ticket  unknown: {e}"),
                        None => println!("per ticket  — (no sale)"),
                    }
                }
            }
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

/// Standard base64, as Solana's RPC returns account data.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| *c != b'=') {
        acc = (acc << 6) | A.iter().position(|x| *x == c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}
