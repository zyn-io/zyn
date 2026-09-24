//! `zyn-sale-draw` — run Collection Z's lottery on the object ledger (§144).
//!
//! The operator's half of the sale, in the order it happens:
//!
//! ```text
//!   zyn-sale-draw status      the sale's phase, tickets and what comes next
//!   zyn-sale-draw open        SetSale at the terms' prices (once)
//!   zyn-sale-draw close       CloseSale, once Zcash reaches the announced close
//!   zyn-sale-draw draw        run the draw over every ticket and post its root
//!   zyn-sale-draw open-mint   end claims; post the order the rest sell in
//! ```
//!
//! Everything it decides can be re-derived by anyone: the tickets are public
//! (`zyn_tickets`), the announcement is published, the drand beacon and the
//! Zcash reveal block are public, and the engine is `cave-randomness`. The
//! draw and open-mint files it writes into the node's data directory are
//! served by the node (`zyn_saleDraw`) so wallets can prove their claims; the
//! chain accepts a claim only against the root posted here.
//!
//! ```text
//!   ZYN_SALE_POLICY        published | rehearsal         (published)
//!   ZYN_SALE_NETWORK       mainnet | testnet             (mainnet)
//!   ZYN_SALE_NODE          the node's JSON-RPC           (http://127.0.0.1:8599)
//!   ZYN_SUBMIT_TOKEN       the operator door's token     (required to write)
//!   ZYN_SALE_DATA          the node's data directory     (required for draw/open-mint)
//!   ZYN_SALE_ANNOUNCEMENT  the close announcement, JSON  (required for close/draw)
//!   ZYN_SALE_MANIFEST      the collection's content.json (required for draw)
//!   ZYN_ZEBRA_HOST / _PORT / _USER / _PASS               (a Zcash node)
//!   ZYN_SALE_CLAIM_WINDOW  rehearsal only: claim window in blocks (2);
//!                          the published sale uses the terms' 16,128
//! ```

use std::collections::BTreeSet;
use std::io::Read;

use objectvm::{ObjectVm, Op};
use serde_json::Value;
use zyn_vm::spec::MicrochainVm;
use zyn_vm::{program, Fixed};
use zynzapd::sale_draw::{self, ZynTicket};

fn env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

const ZAT: i128 = 10_000_000_000;
/// drand quicknet: genesis time and seconds per round.
const QUICKNET_GENESIS: u64 = 1_692_803_367;
const QUICKNET_PERIOD: u64 = 3;

struct Ctx {
    policy: cave_randomness::AllocationPolicy,
    terms: cave_randomness::SaleTerms,
    network: zyn_custody::zebra::Network,
    node: String,
    rehearsal: bool,
}

fn ctx() -> Result<Ctx, String> {
    let rehearsal = env("ZYN_SALE_POLICY", "published") == "rehearsal";
    let policy = match env("ZYN_SALE_POLICY", "published").as_str() {
        "published" => zynzapd::cave_z::policy(),
        "rehearsal" => zynzapd::cave_z::rehearsal_policy(),
        other => return Err(format!("ZYN_SALE_POLICY is published or rehearsal, not {other}")),
    };
    let terms = zynzapd::cave_z::terms(&policy);
    let network = match env("ZYN_SALE_NETWORK", "mainnet").as_str() {
        "mainnet" => zyn_custody::zebra::Network::Mainnet,
        "testnet" => zyn_custody::zebra::Network::Testnet,
        other => return Err(format!("ZYN_SALE_NETWORK is mainnet or testnet, not {other}")),
    };
    Ok(Ctx { policy, terms, network, node: env("ZYN_SALE_NODE", "http://127.0.0.1:8599"), rehearsal })
}

impl Ctx {
    fn collection(&self) -> [u8; 32] {
        self.policy.collection_id
    }

    fn sale(&self) -> Result<Option<Value>, String> {
        let m = zynzapd::object_client::call(&self.node, "zyn_market", serde_json::json!({}))?;
        Ok(m.get("sales")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find(|s| s["collection"].as_str() == Some(&hex(&self.collection()))).cloned()))
    }

    fn tickets(&self) -> Result<Vec<Value>, String> {
        let v = zynzapd::object_client::call(&self.node, "zyn_tickets", serde_json::json!({ "collection": hex(&self.collection()) }))?;
        Ok(v.as_array().cloned().unwrap_or_default())
    }

    fn submit(&self, what: &str, op: &Op) -> Result<(), String> {
        let token = std::env::var("ZYN_SUBMIT_TOKEN").map_err(|_| "set ZYN_SUBMIT_TOKEN (read it from the node's env file)".to_string())?;
        let intent = hex(&ObjectVm::encode_intent(op));
        let r = zynzapd::object_client::call(&self.node, "zyn_submit", serde_json::json!({ "token": token, "intent": intent }))?;
        let receipts = r.get("receipts").and_then(Value::as_str).unwrap_or("");
        if receipts.contains("Refused") {
            return Err(format!("{what}: the chain refused — {receipts}"));
        }
        println!("{what}: {}", receipts.chars().take(160).collect::<String>());
        Ok(())
    }

    fn zebra(&self) -> Result<zyn_custody::zebra::Zebra, String> {
        let port: u16 = env("ZYN_ZEBRA_PORT", if self.network == zyn_custody::zebra::Network::Testnet { "18232" } else { "8232" })
            .parse()
            .map_err(|_| "ZYN_ZEBRA_PORT must be a port".to_string())?;
        let auth = match (std::env::var("ZYN_ZEBRA_USER").ok(), std::env::var("ZYN_ZEBRA_PASS").ok()) {
            (Some(u), Some(p)) => Some((Box::leak(u.into_boxed_str()) as &str, Box::leak(p.into_boxed_str()) as &str)),
            _ => None,
        };
        zyn_custody::zebra::Zebra::connect(&env("ZYN_ZEBRA_HOST", "127.0.0.1"), port, auth, self.network)
            .map_err(|e| format!("cannot reach the Zcash node: {e}"))
    }

    fn announcement(&self) -> Result<(cave_randomness::CloseAnnouncement, cave_randomness::ResolvedClose), String> {
        let path = env("ZYN_SALE_ANNOUNCEMENT", "");
        if path.is_empty() {
            return Err("set ZYN_SALE_ANNOUNCEMENT to the published close announcement".into());
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
        let d: Value = serde_json::from_str(&raw).map_err(|e| format!("{path}: {e}"))?;
        let n = |k: &str| d.get(k).and_then(Value::as_u64).ok_or_else(|| format!("{path}: {k} is missing"));
        let a = cave_randomness::CloseAnnouncement {
            // Bound to the policy this binary holds, never to a digest in the file.
            policy_digest: self.policy.digest().map_err(|e| e.to_string())?,
            announced_at_zcash_height: n("announced_at_zcash_height")? as u32,
            announced_at_drand_round: n("announced_at_drand_round")?,
            deposit_close_zcash_height: n("deposit_close_zcash_height")? as u32,
        };
        let resolved = a.resolve_unproven(&self.policy).map_err(|e| format!("the announcement does not resolve: {e}"))?;
        Ok((a, resolved))
    }

    fn data_file(&self, kind: &str) -> Result<std::path::PathBuf, String> {
        let dir = env("ZYN_SALE_DATA", "");
        if dir.is_empty() {
            return Err("set ZYN_SALE_DATA to the node's data directory, which serves the draw".into());
        }
        Ok(std::path::Path::new(&dir).join(format!("sale-{}-{kind}.json", hex(&self.collection()))))
    }
}

fn status(c: &Ctx) -> Result<(), String> {
    println!("collection {}", hex(&c.collection()));
    println!("policy     {} ({})", hex(&c.policy.digest().map_err(|e| e.to_string())?), env("ZYN_SALE_POLICY", "published"));
    match c.sale()? {
        None => println!("no sale yet — `zyn-sale-draw open` sets it at the terms' prices"),
        Some(s) => {
            println!("phase      {}", s["phase"].as_str().unwrap_or("?"));
            println!("tickets    {}", s["tickets"]);
            let next = match s["phase"].as_str() {
                Some("open") => "close, once Zcash reaches the announced close height",
                Some("closed") => "draw, once the reveal block and drand round exist",
                Some("drawn") => "winners claim; open-mint when the claim window ends",
                Some("openmint") => "the open mint sells the rest in its posted order",
                _ => "?",
            };
            println!("next       {next}");
        }
    }
    Ok(())
}

fn open(c: &Ctx) -> Result<(), String> {
    let op = Op::SetSale {
        program: program::sale_open(),
        collection: c.collection(),
        ticket_price: Fixed(c.terms.entry_price_zatoshi as i128 * ZAT),
        claim_price: Fixed(c.terms.claim_price_zatoshi as i128 * ZAT),
        open_price: Fixed(c.terms.open_mint_price_zatoshi as i128 * ZAT),
        treasury_bps: c.terms.treasury_bps as u32,
    };
    c.submit("open", &op)
}

fn close(c: &Ctx) -> Result<(), String> {
    let (_, resolved) = c.announcement()?;
    let zebra = c.zebra()?;
    let tip = zebra.block_count().map_err(|e| e.to_string())?;
    let at = resolved.deposit_close_zcash_height as u64;
    if tip < at {
        return Err(format!("Zcash is at {tip}; the sale closes at {at} — {} blocks to go", at - tip));
    }
    // The reveal's drand round is fixed by the announcement, not by the
    // close. If it is already public when the sale closes, the draw's drand
    // half was knowable while tickets were still on sale, and only the Zcash
    // block would stand between a buyer and the result. Refuse that close.
    let now_round = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs())
        .saturating_sub(QUICKNET_GENESIS)
        / QUICKNET_PERIOD
        + 1;
    if resolved.reveal_drand_round <= now_round {
        return Err(format!(
            "the reveal's drand round {} is already public (now {now_round}); announce a later round before closing",
            resolved.reveal_drand_round
        ));
    }
    let hash = zebra.block_hash(at).map_err(|e| e.to_string())?;
    c.submit("close", &Op::CloseSale { program: program::sale_close(), collection: c.collection(), zcash_height: at as u32, zcash_hash: hash })
}

fn get(url: &str) -> Result<Vec<u8>, String> {
    let r = ureq::get(url).timeout(std::time::Duration::from_secs(20)).call().map_err(|e| format!("{url}: {e}"))?;
    let mut body = Vec::new();
    r.into_reader().read_to_end(&mut body).map_err(|e| format!("{url}: {e}"))?;
    Ok(body)
}

fn load_manifest(path: &str) -> Result<std::collections::BTreeMap<u32, [u8; 32]>, String> {
    if path.is_empty() {
        return Err("set ZYN_SALE_MANIFEST to the collection's content.json".into());
    }
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| format!("{path}: {e}"))?;
    let mut out = std::collections::BTreeMap::new();
    for (k, v) in doc.as_object().ok_or("content.json is not an object")? {
        let serial: u32 = k.parse().map_err(|_| format!("manifest key {k} is not a serial"))?;
        let content = v.get("content").and_then(Value::as_str).and_then(unhex32).ok_or_else(|| format!("serial {serial} has no content"))?;
        out.insert(serial, content);
    }
    Ok(out)
}

fn draw(c: &Ctx) -> Result<(), String> {
    let sale = c.sale()?.ok_or("there is no sale")?;
    if sale["phase"].as_str() != Some("closed") {
        return Err(format!("the draw follows the close; the sale is {}", sale["phase"]));
    }
    let (announcement, resolved) = c.announcement()?;
    if sale["close_zcash_height"].as_u64() != Some(resolved.deposit_close_zcash_height as u64) {
        return Err("the chain closed at a different height than the announcement names".into());
    }
    let zebra = c.zebra()?;
    let tip = zebra.block_count().map_err(|e| e.to_string())?;
    if tip < resolved.reveal_zcash_height as u64 {
        return Err(format!("the reveal block is {}; Zcash is at {tip}", resolved.reveal_zcash_height));
    }
    let block = zebra.block_hash(resolved.reveal_zcash_height as u64).map_err(|e| e.to_string())?;
    let beacon = zynzapd::reveal::beacon(resolved.reveal_drand_round, c.policy.randomness.drand_chain_hash, &get)
        .map_err(|e| format!("drand round {} is not available yet: {e}", resolved.reveal_drand_round))?;
    let reveal = zynzapd::reveal::reveal(beacon, resolved.reveal_zcash_height, block)?;

    let content = load_manifest(&env("ZYN_SALE_MANIFEST", ""))?;
    let manifest = zynzapd::cave_z::manifest_from(&content);
    let root = cave_randomness::manifest_root(c.policy.collection_id, &manifest).map_err(|e| e.to_string())?;
    if root != c.policy.manifest_root {
        return Err(format!("the manifest rebuilds {} but the policy commits {}", hex(&root), hex(&c.policy.manifest_root)));
    }
    let tickets: Vec<ZynTicket> = c
        .tickets()?
        .iter()
        .map(|t| Some(ZynTicket { id: unhex32(t["ticket"].as_str()?)?, index: t["index"].as_u64()? as u32 }))
        .collect::<Option<_>>()
        .ok_or("the node returned a malformed ticket")?;
    if tickets.len() as u64 != sale["tickets"].as_u64().unwrap_or(0) {
        return Err("the ticket list is not the whole sale".into());
    }
    let d = sale_draw::run(&c.policy, &manifest, &tickets, &resolved, &reveal, announcement.announced_at_drand_round)?;
    let file = c.data_file("draw")?;
    std::fs::write(&file, serde_json::to_vec_pretty(&sale_draw::draw_json(&c.collection(), &resolved, &reveal, &d)).unwrap()).map_err(|e| format!("{}: {e}", file.display()))?;
    println!("draw: {} tickets, {} won, {} serials left for the open mint; written to {}", tickets.len(), d.won.len(), d.batch.unused_indices.len(), file.display());
    c.submit("post draw", &Op::PostDraw { program: program::sale_draw(), collection: c.collection(), root: d.root })
}

fn open_mint(c: &Ctx) -> Result<(), String> {
    let sale = c.sale()?.ok_or("there is no sale")?;
    if sale["phase"].as_str() != Some("drawn") {
        return Err(format!("the open mint follows the draw; the sale is {}", sale["phase"]));
    }
    // Winners have the claim window the terms promise, counted from the
    // reveal block; the rest go on sale only after it.
    let (_, resolved) = c.announcement()?;
    let window = if c.rehearsal {
        env("ZYN_SALE_CLAIM_WINDOW", "2").parse::<u64>().map_err(|_| "ZYN_SALE_CLAIM_WINDOW must be a block count".to_string())?
    } else {
        c.terms.claim_window_blocks as u64
    };
    let ends = resolved.reveal_zcash_height as u64 + window;
    let tip = c.zebra()?.block_count().map_err(|e| e.to_string())?;
    if tip < ends {
        return Err(format!("claims stay open until Zcash {ends}; it is at {tip} — {} blocks to go", ends - tip));
    }
    let file = c.data_file("draw")?;
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?).map_err(|e| e.to_string())?;
    if d["root"].as_str() != sale["draw_root"].as_str() {
        return Err("the draw file is not the draw the chain holds".into());
    }
    let claimed: BTreeSet<[u8; 32]> =
        c.tickets()?.iter().filter(|t| t["claimed"].as_bool() == Some(true)).filter_map(|t| unhex32(t["ticket"].as_str()?)).collect();
    let mut order: Vec<u32> = d["unused"].as_array().ok_or("the draw file has no unused list")?.iter().filter_map(|v| v.as_u64().map(|x| x as u32)).collect();
    for w in d["won"].as_array().ok_or("the draw file has no wins")? {
        let t = w["ticket"].as_str().and_then(unhex32).ok_or("a malformed win")?;
        if !claimed.contains(&t) {
            order.extend(w["serials"].as_array().into_iter().flatten().filter_map(|v| v.as_u64().map(|x| x as u32)));
        }
    }
    let out = c.data_file("open")?;
    std::fs::write(&out, serde_json::to_vec_pretty(&sale_draw::open_json(&c.collection(), &order)).unwrap()).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("open mint: {} items in order; written to {}", order.len(), out.display());
    c.submit(
        "post open mint",
        &Op::PostOpenMint { program: program::sale_open_mint(), collection: c.collection(), root: sale_draw::open_root(&order), count: order.len() as u32 },
    )
}

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    let result = ctx().and_then(|c| match cmd.as_str() {
        "status" => status(&c),
        "open" => open(&c),
        "close" => close(&c),
        "draw" => draw(&c),
        "open-mint" => open_mint(&c),
        other => Err(format!("unknown command {other}: status | open | close | draw | open-mint")),
    });
    if let Err(e) = result {
        eprintln!("zyn-sale-draw {cmd}: {e}");
        std::process::exit(1);
    }
}
