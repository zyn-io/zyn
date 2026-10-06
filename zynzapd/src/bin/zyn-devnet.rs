//! `zyn-devnet` — set up a local devnet for rehearsals: credit test ZEC.zy,
//! register a collection, set fees. It only talks to a node on 127.0.0.1, so
//! it cannot touch testnet or mainnet by mistake.
//!
//! ```text
//!   zyn-devnet credit <account> <amount> [asset]     a sealed deposit note
//!   zyn-devnet register-collection [asset] [creator] Collection Z's terms
//!   zyn-devnet fees <bridge bps>
//!
//!   ZYN_DEVNET_NODE   the devnet node's JSON-RPC (http://127.0.0.1:8399)
//!   ZYN_SUBMIT_TOKEN  the devnet node's operator token
//! ```
//!
//! A credit is built exactly as the deposit bridge builds one
//! (`objectvm::ingress::credit`): the note sealed to the account's registered
//! viewing key, so the wallet finds it like any deposit.

use objectvm::{ObjectVm, Op};
use serde_json::{json, Value};
use zyn_vm::note_enc::{seal, StaticSecret};
use zyn_vm::spec::MicrochainVm;
use zyn_vm::{program, Fixed};

/// ZEC.zy, and Collection Z's asset and treasury as registered on mainnet.
const ZEC: &str = "0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201";
const CAVEZ: &str = "024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e";
const TREASURY: &str = "b3e4c493bb79349270260f7d2e2882b4f1a736348935493331e18c95098cfe99";

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn h32(s: &str) -> Result<[u8; 32], String> {
    let v: Option<Vec<u8>> = (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect();
    v.filter(|v| v.len() == 32).and_then(|v| v.try_into().ok()).ok_or_else(|| format!("{s:?} is not 32 hex bytes"))
}

fn random32() -> [u8; 32] {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b
}

fn node() -> Result<String, String> {
    let n = std::env::var("ZYN_DEVNET_NODE").unwrap_or_else(|_| "http://127.0.0.1:8399".into());
    if !(n.starts_with("http://127.0.0.1:") || n.starts_with("http://localhost:")) {
        return Err(format!("{n} is not a local node; zyn-devnet only talks to a devnet on this machine"));
    }
    Ok(n)
}

fn submit(op: &Op) -> Result<String, String> {
    let token = std::env::var("ZYN_SUBMIT_TOKEN").map_err(|_| "set ZYN_SUBMIT_TOKEN to the devnet node's token")?;
    let r = zynzapd::object_client::call(&node()?, "zyn_submit", json!({ "token": token, "intent": hex(&ObjectVm::encode_intent(op)) }))?;
    Ok(r.get("receipts").and_then(Value::as_str).unwrap_or("").chars().take(200).collect())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("credit") => {
            let account = h32(args.get(1).ok_or("credit <account> <amount> [asset]")?)?;
            let amount = zynzapd::object_client::parse_amount(args.get(2).ok_or("credit <account> <amount> [asset]")?)?;
            let asset = h32(args.get(3).map(String::as_str).unwrap_or(ZEC))?;
            let v = zynzapd::object_client::call(&node()?, "zyn_viewingKey", json!({ "account": hex(&account) }))?;
            let to = h32(v.get("viewing").and_then(Value::as_str).ok_or("the account has not registered a viewing key")?)?;
            // With a bridge fee set, the note is the deposit less the fee, as
            // the bridge credits it.
            let m = zynzapd::object_client::call(&node()?, "zyn_market", json!({}))?;
            let bps = m.pointer("/fees/bridge_bps").and_then(Value::as_u64).unwrap_or(0) as u32;
            let gross = amount;
            let fee = objectvm::market::fee_of(gross, bps).ok_or("the fee does not compute")?;
            let net = Fixed(gross.0 - fee.0);
            let nonce = random32();
            let mut plain = Vec::with_capacity(80);
            plain.extend_from_slice(&asset);
            plain.extend_from_slice(&net.0.to_be_bytes());
            plain.extend_from_slice(&nonce);
            let enc = seal(&to, StaticSecret::from(random32()), &plain).map_err(|_| "could not seal the note")?;
            let op = if fee.0 > 0 {
                Op::DepositWithFee { program: program::deposit(), asset, gross, amount: net, owner: account, nonce, enc, external_ref: random32() }
            } else {
                Op::Deposit { program: program::deposit(), asset, amount: net, owner: account, nonce, enc, external_ref: random32() }
            };
            println!("credit {} to {}: {}", args[2], &hex(&account)[..12], submit(&op)?);
        }
        Some("register-collection") => {
            let asset = h32(args.get(1).map(String::as_str).unwrap_or(CAVEZ))?;
            let creator = h32(args.get(2).map(String::as_str).unwrap_or(TREASURY))?;
            let op = Op::RegisterCollection { program: program::register_collection(), asset, creator, quote: h32(ZEC)?, fee_bps: 100, cap: 4_444 };
            println!("register-collection {}: {}", &hex(&asset)[..12], submit(&op)?);
        }
        Some("fees") => {
            let bridge_bps: u32 = args.get(1).and_then(|b| b.parse().ok()).ok_or("fees <bridge bps>")?;
            println!("fees {bridge_bps}: {}", submit(&Op::SetFees { program: program::configure_fees(), bridge_bps })?);
        }
        _ => return Err("usage: zyn-devnet credit <account> <amount> [asset] | register-collection [asset] [creator] | fees <bps>".into()),
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("zyn-devnet: {e}");
        std::process::exit(1);
    }
}
