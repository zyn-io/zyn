//! `zyn-team-vote` — the team's vote on a Zyn proposal (§176).
//!
//! Members vote on Zyn's issues with their items; the team casts one vote of
//! its own, signed by the team's account, and the DAO's Zyn track decides what
//! it weighs (a veto, or a weighted share). This is the only tool that signs
//! as the team, and it is kept apart from the operator's `zyn-sale-draw`: the
//! operator runs the ledger, the team account votes.
//!
//! ```text
//!   zyn-team-vote account                the team account this key or these
//!                                        shares sign for: what `dao-zyn` takes
//!   zyn-team-vote status                 the Zyn track and its open proposals
//!   zyn-team-vote <proposal> yes | no    cast the team's vote, once
//! ```
//!
//! ```text
//!   ZYN_TEAM_NODE         object node                 (http://127.0.0.1:8212)
//!   ZYN_TEAM_CHAIN        chain id                    (212)
//!   ZYN_TEAM_COLLECTION   the DAO's collection        (Collection Z)
//!   ZYN_TEAM_KEY          a file holding the team account's Ed25519 secret
//!                         (64 hex) — a test team's single key
//!   ZYN_TEAM_SHARES       or: the team account's public.bin directory, and
//!   ZYN_TEAM_CUSTODIANS   the custodians holding its shares (host:port,…),
//!   ZYN_TEAM_THRESHOLD    of which this many sign      (2)
//! ```
//!
//! The key file is never printed. With custodians, no share leaves its box.

use objectvm::Op;
use zyn_vm::program;
use zynzapd::object_client::{call, submit, submit_threshold};

const CAVE_Z: &str = "024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e";

fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
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

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

/// How the team signs: one key (a test team) or its custodians.
enum Team {
    Key(ed25519_dalek::SigningKey),
    Custodians {
        public: zyn_custody::treasury::PublicKeyPackage<zyn_custody::treasury::ZynAccount>,
        custodians: Vec<String>,
        threshold: usize,
    },
}

impl Team {
    fn from_env() -> Result<Team, String> {
        if let Ok(path) = std::env::var("ZYN_TEAM_KEY") {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
            let secret = unhex32(&text).ok_or_else(|| format!("{path}: not a 64-hex Ed25519 secret"))?;
            return Ok(Team::Key(ed25519_dalek::SigningKey::from_bytes(&secret)));
        }
        let dir = std::env::var("ZYN_TEAM_SHARES").map_err(|_| "set ZYN_TEAM_KEY, or ZYN_TEAM_SHARES and ZYN_TEAM_CUSTODIANS".to_string())?;
        let public = zyn_custody::shares::load_public::<zyn_custody::treasury::ZynAccount>(std::path::Path::new(&dir))
            .map_err(|e| format!("{dir}/public.bin: {e}"))?;
        let custodians: Vec<String> = env("ZYN_TEAM_CUSTODIANS", "").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        let threshold: usize = env("ZYN_TEAM_THRESHOLD", "2").parse().map_err(|_| "ZYN_TEAM_THRESHOLD is a number")?;
        if custodians.len() < threshold {
            return Err(format!("{} custodian(s) named, {threshold} must sign", custodians.len()));
        }
        Ok(Team::Custodians { public, custodians, threshold })
    }

    fn account(&self) -> Result<[u8; 32], String> {
        match self {
            // An account is the hash of its key, not the key (zyn_vm::auth::account_of).
            Team::Key(k) => Ok(zyn_vm::auth::account_of(zyn_vm::auth::Scheme::Ed25519, &k.verifying_key().to_bytes())),
            Team::Custodians { public, .. } => zyn_custody::treasury::account_of_group(public).map_err(|e| format!("{e:?}")),
        }
    }
}

fn dao(node: &str, collection: &str) -> Result<serde_json::Value, String> {
    call(node, "zyn_dao", serde_json::json!({ "collection": collection }))
}

fn status(node: &str, collection: &str) -> Result<(), String> {
    let d = dao(node, collection)?;
    let Some(z) = d.get("zyn").filter(|z| !z.is_null()) else {
        println!("no Zyn track on this DAO");
        return Ok(());
    };
    println!("team   {}", z["team"].as_str().unwrap_or("?"));
    println!("rule   {} {}", z["rule"].as_str().unwrap_or("?"), if z["rule"] == "weighted" { format!("({} bps)", z["share_bps"]) } else { String::new() });
    println!("fixed  {}", z["frozen"]);
    for p in d["proposals"].as_array().into_iter().flatten().filter(|p| p["track"] == "zyn") {
        println!(
            "#{:<4} {:<8} yes {:<4} no {:<4} team {}  text {}",
            p["id"], p["state"].as_str().unwrap_or("?"), p["yes"], p["no"], p["team"].as_str().unwrap_or("-"), p["text"].as_str().unwrap_or("?")
        );
    }
    Ok(())
}

fn vote(node: &str, chain: u32, collection_hex: &str, proposal: u32, choice: u8) -> Result<(), String> {
    let team = Team::from_env()?;
    let account = team.account()?;
    // Check before signing: the chain would refuse anyway, but a threshold
    // signature costs every custodian a round, and a typo should cost nobody.
    let d = dao(node, collection_hex)?;
    let z = d.get("zyn").filter(|z| !z.is_null()).ok_or("this DAO has no Zyn track")?;
    if z["team"].as_str() != Some(hex(&account).as_str()) {
        return Err(format!("this key is {}, not the track's team account {}", hex(&account), z["team"].as_str().unwrap_or("?")));
    }
    let p = d["proposals"].as_array().into_iter().flatten().find(|p| p["id"].as_u64() == Some(proposal as u64)).ok_or("no such proposal")?;
    if p["track"] != "zyn" {
        return Err("the team votes only on Zyn proposals".into());
    }
    if p["state"] != "voting" {
        return Err(format!("proposal #{proposal} is {}, not open for votes", p["state"].as_str().unwrap_or("?")));
    }
    let collection = unhex32(collection_hex).ok_or("the collection is 64 hex characters")?;
    let op = Op::TeamVote { team: account, program: program::dao(), collection, proposal, choice };
    let receipts = match &team {
        Team::Key(k) => submit(node, chain, k, op)?,
        Team::Custodians { public, custodians, threshold } => {
            let request = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
            submit_threshold(node, chain, custodians, public, *threshold, request, op)?
        }
    };
    println!("team vote on #{proposal}: {}", receipts.chars().take(160).collect::<String>());
    Ok(())
}

fn main() {
    let node = env("ZYN_TEAM_NODE", "http://127.0.0.1:8212");
    let chain: u32 = env("ZYN_TEAM_CHAIN", "212").parse().unwrap_or(212);
    let collection = env("ZYN_TEAM_COLLECTION", CAVE_Z);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: zyn-team-vote account | status | <proposal> yes|no";
    let result = match args.as_slice() {
        [] => status(&node, &collection),
        [s] if s == "status" => status(&node, &collection),
        [s] if s == "account" => Team::from_env().and_then(|t| t.account()).map(|a| println!("{}", hex(&a))),
        [id, side] => match (id.parse::<u32>(), side.as_str()) {
            (Ok(n), "yes") => vote(&node, chain, &collection, n, objectvm::dao::YES),
            (Ok(n), "no") => vote(&node, chain, &collection, n, objectvm::dao::NO),
            _ => Err(usage.into()),
        },
        _ => Err(usage.into()),
    };
    if let Err(e) = result {
        eprintln!("zyn-team-vote: {e}");
        std::process::exit(1);
    }
}
