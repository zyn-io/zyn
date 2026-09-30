//! A liquidation on Zyn Lend, recorded from the ledger's own code.
//!
//! Live prices will not push a position past its line on cue, and posting a
//! false price for a real asset on the public testnet would mislead anyone
//! reading it. So this runs `objectvm` — the code the node runs — through a
//! scripted fall in ETH, and records each epoch twice: what the borrower's
//! wallet sees, and what everyone sees (`lend_view`, the node's own public
//! answer). The lending app plays the recording at `#/demo`.
//!
//!   cargo run -p zynzapd --bin zyn-lend-demo > ../zyn-lend/app/public/demo/liquidation.json

use objectvm::lend::{leftover_source, lend_source, lot_source, Lend};
use objectvm::{market, Limits, ObjectVm, Op, Output, Receipt};
use serde_json::{json, Value};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};
use zynzapd::lend_view::{lend_markets_json, lend_position_json};

/// The testnet ids, so the app names them as it does on testnet 212.
const ETH: [u8; 32] = id("56737e143259c754d165074ad15058139bc0b116f5a22b2f3d50129a2a7a24a6");
const SOL: [u8; 32] = id("9204324e0511b4d1a90d0ff36242f044e894e1b71489d47abd824df273092ad1");

const ALICE: AccountId = [0xA1; 32]; // lends SOL
const ERIN: AccountId = [0xE1; 32]; // lends SOL too
const BOB: AccountId = [0xB0; 32]; // borrows SOL against ETH, and is liquidated
const FRANK: AccountId = [0xF1; 32]; // borrows too, safely
const CAROL: AccountId = [0xC0; 32]; // bids in the auction
const DAVE: AccountId = [0xD0; 32]; // made the ETH/SOL pool

const WAD: i128 = 1_000_000_000_000_000_000;
const EPOCH_SECONDS: u64 = 60;

const fn id(h: &str) -> [u8; 32] {
    let b = h.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = nib(b[2 * i]) << 4 | nib(b[2 * i + 1]);
        i += 1;
    }
    out
}
const fn nib(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        _ => c - b'a' + 10,
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn usd(cents: i128) -> Fixed {
    Fixed(cents * WAD / 100)
}

struct Demo {
    vm: ObjectVm,
    seq: u64,
    salt: u64,
    time: u64,
    eth: i128,
    sol: i128,
    /// What a public explorer showed for each intent since the last step.
    public_txs: Vec<&'static str>,
    steps: Vec<Value>,
}

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

impl Demo {
    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let kind = match &op {
            Op::LendSupply { .. } | Op::LendWithdraw { .. } | Op::LendOpen { .. } | Op::LendRepay { .. } | Op::LendAddCollateral { .. } | Op::LendRemoveCollateral { .. } | Op::LendBid { .. } => "Lending",
            Op::LendPrices { .. } => "Lending prices",
            Op::Claim { .. } => "Claim",
            Op::Seal => "Seal",
            _ => "Setup",
        };
        let r = self.vm.apply(self.seq, &op).into_iter().next().expect("a receipt");
        assert!(!ObjectVm::rejected(&r), "{r:?} for {op:?}");
        assert_eq!(self.vm.conserved(), Ok(()), "conservation after {op:?}");
        if kind != "Setup" && kind != "Seal" {
            self.public_txs.push(kind);
        }
        r
    }

    fn fresh(&mut self) -> Hash {
        self.salt += 1;
        let mut h = [0x5a; 32];
        h[..8].copy_from_slice(&self.salt.to_be_bytes());
        h
    }

    fn mint(&mut self, owner: AccountId, asset: [u8; 32], whole: i64) {
        let nonce = self.fresh();
        self.apply(Op::Mint { asset, amount: Fixed::whole(whole), owner, nonce });
    }

    fn note(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> Output {
        Output { note: Note { asset, amount, owner, nonce: self.fresh() }, enc: enc() }
    }

    fn balance(&self, owner: AccountId, asset: [u8; 32]) -> i128 {
        self.vm.live.values().filter(|n| n.owner == owner && n.asset == asset).map(|n| n.amount.0).sum()
    }

    fn fund(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> (Vec<(Hash, Hash)>, Vec<Output>) {
        let mut picked = Vec::new();
        let mut sum = 0i128;
        let notes: Vec<(Hash, Note)> = self.vm.live.iter().map(|(c, n)| (*c, *n)).collect();
        for (c, n) in notes {
            if sum >= amount.0 {
                break;
            }
            if n.owner == owner && n.asset == asset {
                picked.push((c, nullifier(&[owner[0]; 32], &c)));
                sum += n.amount.0;
            }
        }
        assert!(sum >= amount.0, "{owner:?} holds too little");
        let change = if sum > amount.0 { vec![self.note(owner, asset, Fixed(sum - amount.0))] } else { Vec::new() };
        (picked, change)
    }

    fn claim(&mut self, owner: AccountId, sources: &[Hash]) {
        let mut ids = Vec::new();
        let mut outputs = Vec::new();
        for s in sources {
            for k in 0..3 {
                let id = market::claim_id(s, k);
                if let Some(c) = self.vm.market.claims.get(&id).copied() {
                    assert_eq!(c.owner, owner);
                    ids.push(id);
                    outputs.push(self.note(owner, c.asset, c.amount));
                }
            }
        }
        assert!(!ids.is_empty(), "nothing to claim");
        self.apply(Op::Claim { owner, program: program::claim(), claims: ids, outputs });
    }

    /// Post prices for this epoch, as the operator does every half minute.
    fn prices(&mut self, eth_cents: i128, sol_cents: i128) {
        self.time += EPOCH_SECONDS;
        self.eth = eth_cents;
        self.sol = sol_cents;
        let at = self.time;
        self.apply(Op::LendPrices { program: program::lend_prices(), at, prices: vec![(ETH, usd(eth_cents)), (SOL, usd(sol_cents))] });
    }

    fn seal(&mut self) {
        assert!(matches!(self.apply(Op::Seal), Receipt::Sealed(_)));
    }

    fn supply(&mut self, who: AccountId, whole: i64) {
        let (inputs, change) = self.fund(who, SOL, Fixed::whole(whole));
        let nonce = self.fresh();
        self.apply(Op::LendSupply { lender: who, program: program::lend_supply(), asset: SOL, amount: Fixed::whole(whole), nonce, inputs, change });
        self.claim(who, &[lend_source(&who, &nonce)]);
    }

    fn open(&mut self, who: AccountId, eth: Fixed, sol: Fixed) -> Hash {
        let (inputs, change) = self.fund(who, ETH, eth);
        let nonce = self.fresh();
        self.apply(Op::LendOpen { borrower: who, program: program::lend_open(), collateral_asset: ETH, collateral: eth, borrow_asset: SOL, amount: sol, nonce, inputs, change });
        let pos = lend_source(&who, &nonce);
        self.claim(who, &[pos]);
        pos
    }

    /// One frame of the recording: Bob's view and the public view, now.
    fn step(&mut self, title: &str, story: &str, bob: &Hash) {
        let owner = lend_position_json(&self.vm, bob);
        let txs = std::mem::take(&mut self.public_txs);
        self.steps.push(json!({
            "title": title,
            "story": story,
            "epoch": self.vm.epoch,
            "prices": { "ETH": self.eth as f64 / 100.0, "SOL": self.sol as f64 / 100.0 },
            "owner": owner,
            "bob": { "ETH": self.balance(BOB, ETH).to_string(), "SOL": self.balance(BOB, SOL).to_string() },
            "public": lend_markets_json(&self.vm),
            "public_txs": txs,
        }));
    }
}

/// The whole run, as the recording the app plays.
fn record() -> Value {
    let mut d = Demo { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, salt: 0, time: 1_790_000_000, eth: 0, sol: 0, public_txs: Vec::new(), steps: Vec::new() };

    // The market: coins for everyone, an ETH/SOL pool at the posted price,
    // both assets listed with testnet's risk numbers and caps.
    // Each holds what their part needs, so Bob's wallet reads true.
    for (who, eth, sol) in [(ALICE, 0, 300), (ERIN, 0, 200), (BOB, 2, 0), (FRANK, 1, 0), (CAROL, 0, 100), (DAVE, 10, 225)] {
        if eth > 0 {
            d.mint(who, ETH, eth);
        }
        if sol > 0 {
            d.mint(who, SOL, sol);
        }
    }
    let (ia, mut ch) = d.fund(DAVE, ETH, Fixed::whole(10));
    let (ib, chb) = d.fund(DAVE, SOL, Fixed::whole(225));
    ch.extend(chb);
    let nonce = d.fresh();
    let (a, b, amt_a, amt_b) = if ETH < SOL { (ETH, SOL, 10, 225) } else { (SOL, ETH, 225, 10) };
    d.apply(Op::CreatePool { provider: DAVE, program: program::add_liquidity(), asset_a: a, asset_b: b, amount_a: Fixed::whole(amt_a), amount_b: Fixed::whole(amt_b), fee_bps: 30, inputs: [ia, ib].concat(), change: ch, nonce });
    d.apply(Op::LendList { program: program::lend_list(), asset: ETH, max_ltv_bps: 7_500, liquidation_bps: 8_000, penalty_bps: 500 });
    d.apply(Op::LendList { program: program::lend_list(), asset: SOL, max_ltv_bps: 6_500, liquidation_bps: 7_000, penalty_bps: 700 });
    d.prices(270_000, 12_000);
    for (asset, coll, bor) in [(ETH, 50, 20), (SOL, 1_000, 500)] {
        d.apply(Op::LendLimits { program: program::lend_list(), asset, collateral_cap: Fixed::whole(coll), borrow_cap: Fixed::whole(bor), min_borrow_usd: usd(100) });
    }
    assert!(d.vm.market.lend.guarded);
    d.public_txs.clear();

    // Lenders, then two borrowers.
    d.supply(ALICE, 300);
    d.supply(ERIN, 200);
    d.seal();
    let bob = d.open(BOB, Fixed::whole(2), Fixed::whole(30));
    let frank = d.open(FRANK, Fixed::whole(1), Fixed::whole(8));
    d.seal();
    d.step(
        "Loans open",
        "Alice and Erin supply 500 SOL. Bob borrows 30 SOL against 2 ETH: 67% LTV, max 75%, liquidation at 80%. Frank borrows 8 SOL against 1 ETH.",
        &bob,
    );

    // ETH falls about 3% an epoch: small enough that no post is held for
    // confirmation, the way a real slide arrives.
    let slide: [(i128, &str); 6] = [
        (260_000, "Bob's LTV: 69%."),
        (251_000, "LTV 72%. Public pool figures are rounded and do not move."),
        (243_000, "LTV 74%."),
        (236_000, "LTV 76%."),
        (229_000, "LTV 79%."),
        (222_000, "LTV 81%, past 80%. At the seal the position becomes an auction lot."),
    ];
    for (px, story) in slide {
        d.prices(px, 12_000);
        d.seal();
        let whole = px / 100;
        d.step(&format!("ETH ${},{:03}", whole / 1000, whole % 1000), story, &bob);
    }
    let lot = *d.vm.market.lend.lots.get(&bob).expect("Bob's position is a lot");
    let lot_id = lot_source(&bob);
    assert!(d.vm.market.lend.positions.contains_key(&frank), "Frank is untouched");

    // The auction steps down a point an epoch; nobody bids for two.
    for n in 0..2 {
        d.prices(222_000 - 500 * (n + 1), 12_000);
        d.seal();
        d.step(
            if n == 0 { "Auction, epoch 1" } else { "Auction, epoch 2" },
            if n == 0 {
                "The lot starts at 102% of the oracle rate and drops 1% per epoch to an 85% floor. Size and price are public; the owner is not."
            } else {
                "No bids yet. All bids in an epoch clear at one price."
            },
            &bob,
        );
    }

    // Carol bids: enough SOL to cover debt and penalty, with room to spare.
    let owed = d.vm.market.lend.listed[&SOL].pool.owed(lot.debt_shares.0).expect("owed");
    let pay = Fixed(owed * 12 / 10);
    let (inputs, change) = d.fund(CAROL, SOL, pay);
    let nonce = d.fresh();
    let carol_eth = d.balance(CAROL, ETH);
    d.apply(Op::LendBid { bidder: CAROL, program: program::lend_bid(), lot: lot_id, pay, nonce, inputs, change });
    d.prices(220_500, 12_000);
    d.seal();
    assert!(!d.vm.market.lend.lots.contains_key(&bob), "the lot settled");
    d.claim(CAROL, &[lend_source(&CAROL, &nonce)]);
    let bought = d.balance(CAROL, ETH) - carol_eth;
    d.step(
        "Filled",
        "Carol bids. The lot sells only enough ETH to cover the 30 SOL debt plus the 5% penalty, which goes to lenders. Her unused SOL is refunded.",
        &bob,
    );

    // Bob collects what the sale did not need, from an id only he derives.
    let bob_eth = d.balance(BOB, ETH);
    d.claim(BOB, &[leftover_source(&bob)]);
    let back = d.balance(BOB, ETH) - bob_eth;
    d.prices(220_500, 12_000);
    d.seal();
    d.step(
        "Leftover claimed",
        "Bob claims the remaining ETH under a claim id derived from his key, not the lot id. Frank's position is unaffected.",
        &bob,
    );

    let pool = d.vm.market.lend.listed[&SOL].pool;
    let out = json!({
        "about": "A scripted run of Zyn Lend's ledger code (objectvm, the code the node runs): prices are made up, every rule is real. Each step records what Bob's wallet sees and what the node publishes to everyone (zynzapd::lend_view).",
        "generated_by": "cargo run -p zynzapd --bin zyn-lend-demo",
        "assets": { "ETH": hex(&ETH), "SOL": hex(&SOL) },
        "ids": { "position": hex(&bob), "lot": hex(&lot_id), "leftover": hex(&leftover_source(&bob)) },
        "outcome": {
            "debt_repaid_sol": owed.to_string(),
            "carol_bought_eth": bought.to_string(),
            "bob_got_back_eth": back.to_string(),
            "bob_collateral_eth": lot.collateral.0.to_string(),
            "pool_value_sol": pool.value().expect("value").to_string(),
            "supplied_sol": (500 * WAD).to_string(),
            "lot_opened_epoch": lot.opened,
            "start_price": Lend::lot_price(&lot, lot.opened).expect("price").to_string(),
        },
        "steps": d.steps,
    });
    assert_eq!(bought + back, lot.collateral.0, "every ETH of the lot went to Carol or back to Bob");
    assert!(pool.value().expect("value") > 500 * WAD, "lenders whole, and then some");
    out
}

fn main() {
    println!("{}", serde_json::to_string_pretty(&record()).expect("json"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recording still plays out against today's ledger: every rule it
    /// shows passes, and no step's public view names Bob or his position.
    #[test]
    fn the_liquidation_recording_still_holds() {
        let r = record();
        let steps = r["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 11);
        let (bob, pos) = (hex(&BOB), r["ids"]["position"].as_str().unwrap().to_string());
        for s in steps {
            let public = s["public"].to_string();
            assert!(!public.contains(&bob) && !public.contains(&pos), "the public view names nobody: {}", s["title"]);
        }
        assert!(steps.iter().any(|s| s["public"]["lots"].as_array().is_some_and(|l| !l.is_empty())), "a lot was public");
    }
}
