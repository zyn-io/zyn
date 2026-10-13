//! `zyn-sale-draw` — run Collection Z's lottery on the object ledger (§144).
//!
//! The operator's half of the sale, in the order it happens:
//!
//! ```text
//!   zyn-sale-draw status      the sale's phase, tickets and what comes next
//!   zyn-sale-draw open        SetSale at the terms' prices (once)
//!   zyn-sale-draw gate <n>    whitelist only, rising past n tickets each (§168)
//!   zyn-sale-draw allow <f>   put the accounts in file f on the list
//!   zyn-sale-draw dao <backers> <voting epochs> <quorum bps>
//!                             give the collection its DAO (§172): its creator
//!                             shares go to the DAO's treasury from then on.
//!                             Refused once the Zcash clock has started
//!   zyn-sale-draw dao-terms <backers> <window blocks> <quorum bps>
//!                             strict terms (§173): windows in Zcash blocks,
//!                             fixed once the DAO has a proposal
//!   zyn-sale-draw dao-zyn <team account> veto | weighted <share bps>
//!                             the Zyn track (§176): members vote on Zyn's
//!                             issues, the team's account casts one vote —
//!                             a veto, or a weighted share. Fixed once the
//!                             track has a proposal
//!   zyn-sale-draw close       CloseSale, once Zcash reaches the announced close
//!   zyn-sale-draw draw        run the draw over every ticket and post its root
//!   zyn-sale-draw open-mint   end claims; post the order the rest sell in. A
//!                             sale closed with no tickets (§187) goes straight
//!                             here: the order is the reveal's shuffle of all
//!   zyn-sale-draw open-terms  the listed price and the cap per account (terms v4)
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
//!   ZYN_SALE_CLAIM_WINDOW  rehearsal only: claim window in blocks after the
//!                          close (2); the published sale uses the terms' 16,128
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
    /// The open-mint terms (§187): open and listed prices, the cap.
    v4: zynzapd::cave_z::TermsV4,
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
    let v4 = zynzapd::cave_z::terms_v4(&policy);
    let network = match env("ZYN_SALE_NETWORK", "mainnet").as_str() {
        "mainnet" => zyn_custody::zebra::Network::Mainnet,
        "testnet" => zyn_custody::zebra::Network::Testnet,
        other => return Err(format!("ZYN_SALE_NETWORK is mainnet or testnet, not {other}")),
    };
    Ok(Ctx { policy, terms, v4, network, node: env("ZYN_SALE_NODE", "http://127.0.0.1:8599"), rehearsal })
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
                Some("closed") if s["tickets"].as_u64() == Some(0) => "open-terms, then open-mint once the reveal block and drand round exist (no tickets: straight to the open mint)",
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

/// Serials of the collection that already exist on this chain. A sale can
/// never mint them: a winner drawn one could not claim it, and one in the
/// open-mint order would stop every place after it.
fn minted(c: &Ctx) -> Result<BTreeSet<u32>, String> {
    let collection = c.collection();
    let mut out = BTreeSet::new();
    let serials: Vec<u32> = (0..c.policy.supply).collect();
    for part in serials.chunks(400) {
        let assets: Vec<String> = part.iter().map(|s| hex(&objectvm::market::item_asset(&collection, *s))).collect();
        let rows = zynzapd::object_client::call(&c.node, "zyn_items", serde_json::json!({ "assets": assets.join(",") }))?;
        for r in rows.as_array().into_iter().flatten() {
            if r["collection"].as_str() == Some(&hex(&collection)) {
                if let Some(s) = r["serial"].as_u64() {
                    out.insert(s as u32);
                }
            }
        }
    }
    Ok(out)
}

fn open(c: &Ctx) -> Result<(), String> {
    // The published sale runs over an untouched collection. A rehearsal on a
    // chain whose test items already exist is told which, and goes on.
    let taken = minted(c)?;
    if !taken.is_empty() {
        let list: Vec<String> = taken.iter().take(12).map(u32::to_string).collect();
        if !c.rehearsal {
            return Err(format!("{} serial(s) of this collection already exist here ({}…) — the published sale runs over an untouched collection", taken.len(), list.join(", ")));
        }
        println!("rehearsal: {} serial(s) already exist ({}); a win of one cannot be claimed, and the open mint skips them", taken.len(), list.join(", "));
    }
    let op = Op::SetSale {
        program: program::sale_open(),
        collection: c.collection(),
        ticket_price: Fixed(c.terms.entry_price_zatoshi as i128 * ZAT),
        claim_price: Fixed(c.terms.claim_price_zatoshi as i128 * ZAT),
        // Terms v4 (§187): the open mint is the sale, at its price.
        open_price: Fixed(c.v4.open_price_zatoshi as i128 * ZAT),
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
    if tickets.is_empty() {
        return Err("the sale closed with no tickets, so there is nothing to draw — `open` sets it again, or `open-mint` sells the collection in order".into());
    }
    let d = sale_draw::run(&c.policy, &manifest, &tickets, &resolved, &reveal, announcement.announced_at_drand_round)?;
    let taken = minted(c)?;
    let blocked: Vec<u32> = d.won.iter().flat_map(|(_, s)| s.iter().copied()).filter(|s| taken.contains(s)).collect();
    if !blocked.is_empty() {
        if !c.rehearsal {
            return Err(format!("the draw hands out serial(s) that already exist ({blocked:?}) — nothing is posted; this chain's collection was not untouched"));
        }
        println!("rehearsal: the draw hands out serial(s) that already exist {blocked:?}; those wins cannot be claimed");
    }
    let file = c.data_file("draw")?;
    std::fs::write(&file, serde_json::to_vec_pretty(&sale_draw::draw_json(&c.collection(), &resolved, &reveal, &d)).unwrap()).map_err(|e| format!("{}: {e}", file.display()))?;
    println!("draw: {} tickets, {} won, {} serials left for the open mint; written to {}", tickets.len(), d.won.len(), d.batch.unused_indices.len(), file.display());
    c.submit("post draw", &Op::PostDraw { program: program::sale_draw(), collection: c.collection(), root: d.root })
}

/// The announced close's reveal, checked: the chain closed where the
/// announcement says, and the reveal's Zcash block and drand round exist.
fn reveal(c: &Ctx, sale: &Value) -> Result<(cave_randomness::CloseAnnouncement, cave_randomness::ResolvedClose, cave_randomness::RevealEvidence), String> {
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
    Ok((announcement, resolved, reveal))
}

/// The manifest, checked against the root the policy commits to.
fn manifest(c: &Ctx) -> Result<Vec<cave_randomness::ManifestEntry>, String> {
    let content = load_manifest(&env("ZYN_SALE_MANIFEST", ""))?;
    let manifest = zynzapd::cave_z::manifest_from(&content);
    let root = cave_randomness::manifest_root(c.policy.collection_id, &manifest).map_err(|e| e.to_string())?;
    if root != c.policy.manifest_root {
        return Err(format!("the manifest rebuilds {} but the policy commits {}", hex(&root), hex(&c.policy.manifest_root)));
    }
    Ok(manifest)
}

/// Set the open-mint terms (§187, terms v4): what accounts on the sale's list
/// pay, and how many one account mints at most. Before the first mint.
fn open_terms(c: &Ctx) -> Result<(), String> {
    c.submit(
        "open-mint terms",
        &Op::SetOpenTerms {
            program: program::sale_open_mint(),
            collection: c.collection(),
            listed_price: Fixed(c.v4.listed_price_zatoshi as i128 * ZAT),
            cap: c.v4.cap_per_account,
        },
    )
}

/// §187: a sale that closed with no tickets goes straight to its open mint.
/// No claims to wait for; the order is the reveal's shuffle of every serial,
/// as the terms say, written beside the draw's files for anyone to replay.
fn open_mint_without_tickets(c: &Ctx, sale: &Value) -> Result<(), String> {
    let (_, resolved, reveal) = reveal(c, sale)?;
    let manifest = manifest(c)?;
    let mut order = sale_draw::order_without_tickets(&c.policy, &manifest, &resolved, &reveal)?;
    let taken = minted(c)?;
    let before = order.len();
    order.retain(|s| !taken.contains(s));
    if order.len() != before {
        println!("open mint: {} serial(s) already exist and are left out", before - order.len());
    }
    let out = c.data_file("open")?;
    std::fs::write(&out, serde_json::to_vec_pretty(&sale_draw::open_json(&c.collection(), &order)).unwrap()).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("open mint: {} items in the revealed order; written to {}", order.len(), out.display());
    c.submit(
        "post open mint",
        &Op::PostOpenMint { program: program::sale_open_mint(), collection: c.collection(), root: sale_draw::open_root(&order), count: order.len() as u32 },
    )
}

fn open_mint(c: &Ctx) -> Result<(), String> {
    let sale = c.sale()?.ok_or("there is no sale")?;
    if sale["phase"].as_str() == Some("closed") && sale["tickets"].as_u64() == Some(0) {
        return open_mint_without_tickets(c, &sale);
    }
    if sale["phase"].as_str() != Some("drawn") {
        return Err(format!("the open mint follows the draw; the sale is {}", sale["phase"]));
    }
    // Winners have the claim window the published terms promise: 16,128
    // blocks after the close (cavebro.win/terms). The rest go on sale only
    // after it.
    let (_, resolved) = c.announcement()?;
    let window = if c.rehearsal {
        env("ZYN_SALE_CLAIM_WINDOW", "2").parse::<u64>().map_err(|_| "ZYN_SALE_CLAIM_WINDOW must be a block count".to_string())?
    } else {
        c.terms.claim_window_blocks as u64
    };
    let ends = resolved.deposit_close_zcash_height as u64 + window;
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
    // What already exists cannot be sold again; left in, it would stop the
    // open mint at its place. Skipping it is decided by public chain state,
    // so the order is still anyone's to recompute.
    let taken = minted(c)?;
    let before = order.len();
    order.retain(|s| !taken.contains(s));
    if order.len() != before {
        println!("open mint: {} serial(s) already exist and are left out", before - order.len());
    }
    let out = c.data_file("open")?;
    std::fs::write(&out, serde_json::to_vec_pretty(&sale_draw::open_json(&c.collection(), &order)).unwrap()).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("open mint: {} items in order; written to {}", order.len(), out.display());
    c.submit(
        "post open mint",
        &Op::PostOpenMint { program: program::sale_open_mint(), collection: c.collection(), root: sale_draw::open_root(&order), count: order.len() as u32 },
    )
}

/// Gate the sale to its list (§168): listed accounts only, the ticket price
/// for each one's first `base` tickets and one more ticket price per ticket
/// after. Before the first ticket.
fn gate(c: &Ctx) -> Result<(), String> {
    let base: u32 = std::env::args().nth(2).and_then(|b| b.parse().ok()).ok_or("usage: zyn-sale-draw gate <tickets at the base price>")?;
    c.submit("gate", &Op::GateSale { program: program::sale_open(), collection: c.collection(), base })
}

/// Give the collection its DAO (§172), or retune it. From the first time, the
/// treasury's shares of the sale and of resale fees go to the DAO's keyless
/// treasury instead of the creator's account.
fn dao(c: &Ctx) -> Result<(), String> {
    let usage = "usage: zyn-sale-draw dao <backers> <voting epochs> <quorum bps>";
    let arg = |i: usize| std::env::args().nth(i).ok_or(usage);
    let backers: u32 = arg(2)?.parse().map_err(|_| usage)?;
    let voting_epochs: u64 = arg(3)?.parse().map_err(|_| usage)?;
    let quorum_bps: u32 = arg(4)?.parse().map_err(|_| usage)?;
    c.submit("dao", &Op::SetDao { program: program::dao_configure(), collection: c.collection(), backers, voting_epochs, quorum_bps })
}

/// Strict DAO terms (§173), once the node's Zcash clock has started.
fn dao_terms(c: &Ctx) -> Result<(), String> {
    let usage = "usage: zyn-sale-draw dao-terms <backers> <window blocks> <quorum bps>";
    let arg = |i: usize| std::env::args().nth(i).ok_or(usage);
    let backers: u32 = arg(2)?.parse().map_err(|_| usage)?;
    let window_blocks: u64 = arg(3)?.parse().map_err(|_| usage)?;
    let quorum_bps: u32 = arg(4)?.parse().map_err(|_| usage)?;
    c.submit("dao-terms", &Op::SetDaoTerms { program: program::dao_configure(), collection: c.collection(), backers, window_blocks, quorum_bps })
}

/// The Zyn track (§176): who casts the team's vote, and what it weighs.
fn dao_zyn(c: &Ctx) -> Result<(), String> {
    let usage = "usage: zyn-sale-draw dao-zyn <team account> veto | weighted <share bps>";
    let arg = |i: usize| std::env::args().nth(i).ok_or(usage);
    let team = unhex32(&arg(2)?).ok_or("the team account is 64 hex characters")?;
    let (rule, share_bps) = match arg(3)?.as_str() {
        "veto" => (objectvm::dao::RULE_VETO, 0),
        "weighted" => (objectvm::dao::RULE_WEIGHTED, arg(4)?.parse().map_err(|_| usage)?),
        _ => return Err(usage.into()),
    };
    c.submit("dao-zyn", &Op::SetZynTrack { program: program::dao_configure(), collection: c.collection(), team, rule, share_bps })
}

/// Put accounts on the list: a file of Zyn account ids, one per line (the
/// whitelist's linked accounts). Each goes on blinded; already-listed ones are
/// left as they are.
fn allow(c: &Ctx) -> Result<(), String> {
    let path = std::env::args().nth(2).ok_or("usage: zyn-sale-draw allow <file of account ids>")?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    let mut keys = Vec::new();
    for (i, line) in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).enumerate() {
        let account = unhex32(line).ok_or_else(|| format!("{path}: line {} is not a 64-hex account", i + 1))?;
        keys.push(objectvm::sale::allow_key(&c.collection(), &account));
    }
    println!("allow: {} account(s)", keys.len());
    for chunk in keys.chunks(objectvm::sale::MAX_ALLOW_PER_OP) {
        c.submit("allow", &Op::AllowBuyers { program: program::sale_open(), collection: c.collection(), keys: chunk.to_vec() })?;
    }
    Ok(())
}

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    let result = ctx().and_then(|c| match cmd.as_str() {
        "status" => status(&c),
        "open" => open(&c),
        "close" => close(&c),
        "draw" => draw(&c),
        "open-mint" => open_mint(&c),
        "open-terms" => open_terms(&c),
        "gate" => gate(&c),
        "allow" => allow(&c),
        "dao" => dao(&c),
        "dao-terms" => dao_terms(&c),
        "dao-zyn" => dao_zyn(&c),
        other => Err(format!("unknown command {other}: status | open | gate | allow | dao | dao-terms | dao-zyn | close | draw | open-mint")),
    });
    if let Err(e) = result {
        eprintln!("zyn-sale-draw {cmd}: {e}");
        std::process::exit(1);
    }
}
