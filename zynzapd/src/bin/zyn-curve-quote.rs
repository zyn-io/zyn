//! `zyn-curve-quote` — name a coin coins launch in, and size its curves
//! (DECISIONS §166): `SetCurveQuote`, through the node's operator door.
//!
//! ```text
//!   zyn-curve-quote <asset> <graduation mc> <bond>   submit it
//!   zyn-curve-quote <asset> 0 0                      stop new coins in it
//!   zyn-curve-quote --print <asset> <mc> <bond>      print the intent only
//!
//!   ZYN_OBJECT_NODE   the node's JSON-RPC   (http://127.0.0.1:8499)
//!   ZYN_SUBMIT_TOKEN  the operator's token  (from the node's env file;
//!                     run under systemd-run with it, never on a command line)
//! ```
//!
//! Amounts are whole coins with decimals ("23", "0.025"), as the ledger holds
//! every asset at 18 decimals. It prints the unit the size gives and what a
//! curve then raises, before anything is sent.

use objectvm::curve::{cost_at, QuoteTerms, GRADUATE_AT, GRADUATION_MC_UNITS};
use objectvm::{ObjectVm, Op};
use serde_json::json;
use zyn_vm::program;
use zyn_vm::spec::MicrochainVm;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn whole(raw: i128) -> String {
    let w = zyn_vm::fixed::WAD;
    let frac = format!("{:018}", raw % w);
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() { format!("{}", raw / w) } else { format!("{}.{frac}", raw / w) }
}

fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let print = args.first().map(|a| a == "--print").unwrap_or(false);
    if print {
        args.remove(0);
    }
    let [asset, mc, bond] = <[String; 3]>::try_from(args).map_err(|_| "usage: zyn-curve-quote [--print] <asset hex> <graduation mc> <bond>".to_string())?;
    let quote: [u8; 32] = (0..32)
        .map(|i| asset.get(2 * i..2 * i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
        .collect::<Option<Vec<u8>>>()
        .filter(|_| asset.len() == 64)
        .and_then(|v| v.try_into().ok())
        .ok_or("the asset is 64 hex characters")?;
    // Zero is a size here (stop new coins) and a bond (none).
    let amount = |t: &str| if t.trim().trim_end_matches('0').trim_end_matches('.').is_empty() { Ok(zyn_vm::Fixed::ZERO) } else { zynzapd::object_client::parse_amount(t) };
    let graduation_mc = amount(&mc)?;
    let bond = amount(&bond)?;
    let op = Op::SetCurveQuote { program: program::configure_curves(), quote, graduation_mc, bond };
    if graduation_mc.0 == 0 {
        println!("stop new coins in {asset}");
    } else {
        let unit = QuoteTerms::unit_for(graduation_mc);
        let q = QuoteTerms { unit, bond };
        println!("asset      {asset}");
        println!("unit       {unit}  (ZEC.zy's is 10000000000)");
        println!("graduates  at {} fully diluted, raising {}", whole(unit * GRADUATION_MC_UNITS), whole(cost_at(0, GRADUATE_AT, unit)));
        println!("bond       {}", whole(bond.0));
        println!("min buy    {}", whole(objectvm::curve::MIN_BUY_UNITS * unit));
        if !q.valid() {
            return Err("the ledger would refuse these terms (unit 10^6–10^18, bond at most 10^8 units)".into());
        }
    }
    let intent = hex(&ObjectVm::encode_intent(&op));
    if print {
        println!("intent     {intent}");
        return Ok(());
    }
    let node = std::env::var("ZYN_OBJECT_NODE").unwrap_or_else(|_| "http://127.0.0.1:8499".into());
    let token = std::env::var("ZYN_SUBMIT_TOKEN").map_err(|_| "set ZYN_SUBMIT_TOKEN (from the node's env file)")?;
    let r = zynzapd::object_client::call(&node, "zyn_submit", json!({ "token": token, "intent": intent }))?;
    println!("receipts   {}", r.get("receipts").and_then(|v| v.as_str()).unwrap_or(""));
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("zyn-curve-quote: {e}");
        std::process::exit(1);
    }
}
