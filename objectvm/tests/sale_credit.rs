//! Sale credit (DECISIONS §182): the team pays ZEC.zy into a sale — floor and
//! treasury at once — and burns on other chains become credit worth what
//! they burned, out of that budget; the burner takes it up from their own
//! account and their tickets, claims and open mints are paid from it first.
//! On a real ledger, with the books checked after every intent.

use objectvm::market::{self, item_asset, ITEM_UNIT};
use objectvm::sale::{allow_key, credit_funding, credit_key, draw_leaf, open_leaf, ticket_id, treasury_source, voucher_commitment};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::{merkle_proof, merkle_root, Hash};
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x10; 32];
const CAVE: [u8; 32] = [0xCA; 32];
const DAO: AccountId = [0x7E; 32];
const TEAM: AccountId = [0x7A; 32];
const ALICE: AccountId = [0xA1; 32];
const BOB: AccountId = [0xB0; 32];
const ZAT: i128 = 10_000_000_000;
const TICKET: Fixed = Fixed(1_000_000 * ZAT); // 0.01
const CLAIM: Fixed = Fixed(500_000 * ZAT); // 0.005
const OPEN: Fixed = Fixed(1_330_000 * ZAT); // 0.0133
const SECRET: Hash = [0x5e; 32];

struct Chain {
    vm: ObjectVm,
    seq: u64,
    salt: u64,
}

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

fn zec(zats: i128) -> Fixed {
    Fixed(zats * ZAT)
}

fn floor_share(x: i128) -> i128 {
    x - x * 1_000 / 10_000
}

impl Chain {
    fn new() -> Chain {
        let mut c = Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, salt: 0 };
        for who in [TEAM, ALICE, BOB] {
            c.mint(who, 1);
        }
        c.ok(Op::RegisterCollection { program: program::register_collection(), asset: CAVE, creator: DAO, quote: ZEC, fee_bps: 100, cap: 12 });
        c.ok(Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: TICKET, claim_price: CLAIM, open_price: OPEN, treasury_bps: 1_000 });
        c
    }

    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let r = self.vm.apply(self.seq, &op);
        assert_eq!(self.vm.conserved(), Ok(()), "after {op:?}");
        assert_eq!(self.vm.reconciled(), Ok(()));
        assert_eq!(ObjectVm::decode(&self.vm.encode()).as_ref(), Some(&self.vm), "state round-trips");
        let wire = ObjectVm::encode_intent(&op);
        assert_eq!(ObjectVm::decode_intent(&mut zyn_vm::read::Decoder::new(&wire)).as_ref(), Some(&op), "intent round-trips");
        r.into_iter().next().unwrap()
    }

    fn ok(&mut self, op: Op) -> Receipt {
        let r = self.apply(op);
        assert!(!ObjectVm::rejected(&r), "{r:?}");
        r
    }

    fn refused(&mut self, op: Op) -> String {
        match self.apply(op) {
            Receipt::Refused(why) => why.to_string(),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn fresh(&mut self) -> Hash {
        self.salt += 1;
        let mut h = [0x3c; 32];
        h[..8].copy_from_slice(&self.salt.to_be_bytes());
        h
    }

    fn mint(&mut self, owner: AccountId, whole: i64) {
        let nonce = self.fresh();
        self.ok(Op::Mint { asset: ZEC, amount: Fixed::whole(whole), owner, nonce });
    }

    fn note(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> Output {
        Output { note: Note { asset, amount, owner, nonce: self.fresh() }, enc: enc() }
    }

    fn balance(&self, owner: AccountId) -> i128 {
        self.vm.live.values().filter(|n| n.owner == owner && n.asset == ZEC).map(|n| n.amount.0).sum()
    }

    /// Notes covering `amount`, or none at all for nothing.
    fn fund(&mut self, owner: AccountId, amount: i128) -> (Vec<(Hash, Hash)>, Vec<Output>) {
        if amount <= 0 {
            return (Vec::new(), Vec::new());
        }
        let mut picked = Vec::new();
        let mut sum = 0i128;
        let notes: Vec<(Hash, Note)> = self.vm.live.iter().map(|(c, n)| (*c, *n)).collect();
        for (c, n) in notes {
            if sum >= amount {
                break;
            }
            if n.owner == owner && n.asset == ZEC {
                picked.push((c, nullifier(&[owner[0]; 32], &c)));
                sum += n.amount.0;
            }
        }
        assert!(sum >= amount, "too little to fund that");
        let change = if sum > amount { vec![self.note(owner, ZEC, Fixed(sum - amount))] } else { Vec::new() };
        (picked, change)
    }

    fn credit(&self, who: AccountId) -> i128 {
        self.vm.market.credits.held.get(&credit_key(&CAVE, &who)).map_or(0, |c| c.0)
    }

    fn budget(&self) -> i128 {
        self.vm.market.credits.budgets.get(&CAVE).map_or(0, |b| b.0)
    }

    fn floor(&self) -> i128 {
        self.vm.market.collections[&CAVE].floor.0
    }

    fn fund_sale(&mut self, amount: Fixed) -> (Hash, Op) {
        let nonce = self.fresh();
        let (inputs, change) = self.fund(TEAM, amount.0);
        (nonce, Op::FundSaleCredit { funder: TEAM, program: program::sale_tickets(), collection: CAVE, amount, nonce, inputs, change })
    }

    /// Tickets paying `notes` from notes; the rest is the buyer's credit.
    fn buy(&mut self, buyer: AccountId, count: u32, notes: i128) -> (Hash, Op) {
        let nonce = self.fresh();
        let (inputs, change) = self.fund(buyer, notes);
        (nonce, Op::BuyTickets { buyer, program: program::sale_tickets(), collection: CAVE, count, nonce, inputs, change })
    }
}

fn post(burn: u8, commitment: Hash, amount: Fixed) -> Op {
    Op::PostCredit { program: program::sale_vouchers(), collection: CAVE, burn: [burn; 32], commitment, amount }
}

fn redeem(buyer: AccountId, secret: Hash) -> Op {
    Op::RedeemCredit { buyer, program: program::sale_claim(), collection: CAVE, secret }
}

#[test]
fn the_team_funds_burns_credit_and_credit_pays_before_notes() {
    let mut c = Chain::new();
    let plain = c.vm.state_root();

    // The team's funding is a sale payment: 90% into the floor, 10% to the
    // DAO, at once — and it becomes the budget burns are credited from.
    let (nonce, op) = c.fund_sale(zec(10_000_000));
    assert_eq!(ObjectVm::intent_authorities(&op), vec![TEAM]);
    assert!(matches!(c.ok(op), Receipt::SaleCreditFunded { .. }));
    assert_eq!(c.floor(), floor_share(zec(10_000_000).0));
    let owed = c.vm.market.claims[&market::claim_id(&treasury_source(&CAVE, &credit_funding(&TEAM, &nonce)), 0)];
    assert_eq!((owed.owner, owed.amount.0), (DAO, zec(1_000_000).0));
    assert_eq!(c.budget(), zec(10_000_000).0);
    assert_ne!(c.vm.state_root(), plain, "credit is committed");
    assert!(c.vm.encode().windows(8).any(|w| w == b"MARKET17"), "and saved in its own format");
    let (_, again) = {
        let (inputs, change) = c.fund(TEAM, zec(1).0);
        ((), Op::FundSaleCredit { funder: TEAM, program: program::sale_tickets(), collection: CAVE, amount: zec(1), nonce, inputs, change })
    };
    assert_eq!(c.refused(again), "that nonce was already used");

    // A burn becomes credit, operator only, once, within the budget.
    let commitment = voucher_commitment(&ALICE, &SECRET);
    assert_eq!(ObjectVm::intent_authorities(&post(1, commitment, zec(1))), Vec::<AccountId>::new(), "only the operator posts a burn");
    assert!(c.refused(post(1, commitment, zec(10_000_001))).contains("budget"));
    let floor = c.floor();
    c.ok(post(1, commitment, zec(3_500_000)));
    assert_eq!(c.floor(), floor, "credit moves no value: it was paid in when funded");
    assert_eq!(c.budget(), zec(6_500_000).0);
    assert!(c.refused(post(1, commitment, zec(1))).contains("already posted"), "a burn counts once");

    // Only the account the commitment names takes it up, once.
    assert!(c.refused(redeem(BOB, SECRET)).contains("no credit waits"));
    assert_eq!(ObjectVm::intent_authorities(&redeem(ALICE, SECRET)), vec![ALICE]);
    c.ok(redeem(ALICE, SECRET));
    assert_eq!(c.credit(ALICE), zec(3_500_000).0);
    assert!(c.refused(redeem(ALICE, SECRET)).contains("no credit waits"));

    // Tickets from credit alone: no notes, nothing new in the floor.
    let held = c.balance(ALICE);
    let (_, op) = c.buy(ALICE, 2, 0);
    c.ok(op);
    assert_eq!((c.credit(ALICE), c.balance(ALICE), c.floor()), (zec(1_500_000).0, held, floor));
    // Credit covering it all means no notes may be spent beside it.
    let (_, op) = c.buy(ALICE, 1, TICKET.0);
    assert_eq!(c.refused(op), "credit covers this payment; spend no notes");
    // Past the credit, notes pay the rest, split as any payment.
    let (_, op) = c.buy(ALICE, 3, zec(1_500_000).0);
    c.ok(op);
    assert_eq!(c.credit(ALICE), 0);
    assert!(!c.vm.market.credits.held.contains_key(&credit_key(&CAVE, &ALICE)), "spent credit leaves the state");
    assert_eq!(c.floor(), floor + floor_share(zec(1_500_000).0));
    assert_eq!(c.vm.market.sales[&CAVE].tickets, 5);
}

#[test]
fn credit_pays_claims_and_the_open_mint_and_ends_with_the_sale() {
    let mut c = Chain::new();
    let (_, op) = c.fund_sale(zec(5_000_000));
    c.ok(op);
    let (a_nonce, op) = c.buy(ALICE, 1, TICKET.0);
    c.ok(op);
    c.ok(Op::CloseSale { program: program::sale_close(), collection: CAVE, zcash_height: 10, zcash_hash: [7; 32] });
    let leaves = vec![draw_leaf(&ticket_id(&ALICE, &a_nonce, 0), &[4, 5])];
    c.ok(Op::PostDraw { program: program::sale_draw(), collection: CAVE, root: merkle_root(&leaves) });

    // A burn after the close still buys what the sale still sells.
    c.ok(post(2, voucher_commitment(&ALICE, &SECRET), CLAIM));
    c.ok(redeem(ALICE, SECRET));
    let items: Vec<Output> = [4u32, 5].iter().map(|s| c.note(ALICE, item_asset(&CAVE, *s), ITEM_UNIT)).collect();
    let (inputs, change) = c.fund(ALICE, CLAIM.0);
    let floor = c.floor();
    let claim = Op::ClaimItems { buyer: ALICE, program: program::sale_claim(), collection: CAVE, nonce: a_nonce, k: 0, serials: vec![4, 5], proof: merkle_proof(&leaves, 0), items, inputs, change };
    c.ok(claim);
    assert_eq!(c.floor(), floor + floor_share(CLAIM.0), "one claim from credit, one from notes");
    assert_eq!(c.credit(ALICE), 0);

    // The open mint, from Bob's credit.
    let open: Vec<Hash> = [0u32, 1].iter().enumerate().map(|(i, s)| open_leaf(i as u32, *s)).collect();
    c.ok(Op::PostOpenMint { program: program::sale_open_mint(), collection: CAVE, root: merkle_root(&open), count: 2 });
    c.ok(post(3, voucher_commitment(&BOB, &SECRET), OPEN));
    c.ok(redeem(BOB, SECRET));
    let item = c.note(BOB, item_asset(&CAVE, 0), ITEM_UNIT);
    let nonce = c.fresh();
    let floor = c.floor();
    c.ok(Op::MintOpen { buyer: BOB, program: program::sale_mint(), collection: CAVE, serial: 0, proof: merkle_proof(&open, 0), item, nonce, inputs: vec![], change: vec![] });
    assert_eq!((c.floor(), c.credit(BOB)), (floor, 0));
    let item = c.note(ALICE, item_asset(&CAVE, 1), ITEM_UNIT);
    let nonce = c.fresh();
    let (inputs, change) = c.fund(ALICE, OPEN.0);
    c.ok(Op::MintOpen { buyer: ALICE, program: program::sale_mint(), collection: CAVE, serial: 1, proof: merkle_proof(&open, 1), item, nonce, inputs, change });

    // Sold out: no more funding, no more credit.
    let (_, op) = c.fund_sale(zec(1));
    assert_eq!(c.refused(op), "the sale is over");
    assert_eq!(c.refused(post(4, voucher_commitment(&ALICE, &[1; 32]), zec(1))), "the sale is over");
}

/// A gated sale (§168) still gates: credit buys tickets only for a listed
/// account, at its rising price.
#[test]
fn credit_keeps_the_whitelist_and_the_rising_price() {
    let mut c = Chain::new();
    c.ok(Op::GateSale { program: program::sale_open(), collection: CAVE, base: 1 });
    c.ok(Op::AllowBuyers { program: program::sale_open(), collection: CAVE, keys: vec![allow_key(&CAVE, &ALICE)] });
    let (_, op) = c.fund_sale(zec(10_000_000));
    c.ok(op);
    for (burn, who) in [(5u8, ALICE), (6, BOB)] {
        c.ok(post(burn, voucher_commitment(&who, &SECRET), zec(3_000_000)));
        c.ok(redeem(who, SECRET));
    }
    let (_, op) = c.buy(BOB, 1, 0);
    assert_eq!(c.refused(op), "only accounts on the sale's list can buy tickets");
    // Alice's first ticket at 0.01, her second at 0.02: all 0.03 of credit.
    let (_, op) = c.buy(ALICE, 2, 0);
    c.ok(op);
    assert_eq!(c.credit(ALICE), 0);
}
