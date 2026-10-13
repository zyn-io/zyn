#![allow(dead_code)]
//! Open-mint terms (DECISIONS §187): a sale that goes straight to its open
//! mint, where accounts on the sale's list pay less and no account mints more
//! than a cap — on a real ledger, with the books checked after every intent.

use objectvm::market::{item_asset, ITEM_UNIT};
#[allow(unused_imports)]
use objectvm::sale::{allow_key, draw_leaf, open_leaf, ticket_id, treasury_source, Phase};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
#[allow(unused_imports)]
use zyn_vm::commit::{merkle_proof, merkle_root, Hash, ProofStep};
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x10; 32];
const CAVE: [u8; 32] = [0xCA; 32];
const TREASURY: AccountId = [0x7E; 32];
const ALICE: AccountId = [0xA1; 32];
const BOB: AccountId = [0xB0; 32];
const CAROL: AccountId = [0xC0; 32];
const ZAT: i128 = 10_000_000_000;
const TICKET: Fixed = Fixed(1_000_000 * ZAT); // 0.01
const CLAIM: Fixed = Fixed(500_000 * ZAT); // 0.005
const OPEN: Fixed = Fixed(5_000_000 * ZAT); // 0.05
const LISTED: Fixed = Fixed(1_000_000 * ZAT); // 0.01

struct Chain {
    vm: ObjectVm,
    seq: u64,
    salt: u64,
}

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

impl Chain {
    fn new() -> Chain {
        Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, salt: 0 }
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

    fn balance(&self, owner: AccountId, asset: [u8; 32]) -> Fixed {
        Fixed(self.vm.live.values().filter(|n| n.owner == owner && n.asset == asset).map(|n| n.amount.0).sum())
    }

    fn fund(&mut self, owner: AccountId, amount: Fixed) -> (Vec<(Hash, Hash)>, Vec<Output>) {
        let mut picked = Vec::new();
        let mut sum = 0i128;
        let notes: Vec<(Hash, Note)> = self.vm.live.iter().map(|(c, n)| (*c, *n)).collect();
        for (c, n) in notes {
            if sum >= amount.0 {
                break;
            }
            if n.owner == owner && n.asset == ZEC {
                picked.push((c, nullifier(&[owner[0]; 32], &c)));
                sum += n.amount.0;
            }
        }
        assert!(sum >= amount.0, "too little to fund that");
        let change = if sum > amount.0 { vec![self.note(owner, ZEC, Fixed(sum - amount.0))] } else { Vec::new() };
        (picked, change)
    }

    fn buy(&mut self, buyer: AccountId, count: u32) -> (Hash, Op) {
        let nonce = self.fresh();
        let (inputs, change) = self.fund(buyer, Fixed(TICKET.0 * count as i128));
        (nonce, Op::BuyTickets { buyer, program: program::sale_tickets(), collection: CAVE, count, nonce, inputs, change })
    }

    fn items(&mut self, owner: AccountId, serials: &[u32]) -> Vec<Output> {
        serials.iter().map(|s| self.note(owner, item_asset(&CAVE, *s), ITEM_UNIT)).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn claim_op(&mut self, buyer: AccountId, nonce: Hash, k: u32, serials: &[u32], proof: Vec<ProofStep>) -> Op {
        let items = self.items(buyer, serials);
        let (inputs, change) = self.fund(buyer, Fixed(CLAIM.0 * serials.len() as i128));
        Op::ClaimItems { buyer, program: program::sale_claim(), collection: CAVE, nonce, k, serials: serials.to_vec(), proof, items, inputs, change }
    }

    fn floor(&self) -> Fixed {
        self.vm.market.collections[&CAVE].floor
    }
}

fn floor_share(x: Fixed) -> i128 {
    x.0 - x.0 * 1_000 / 10_000
}


/// The plan for Collection Z (§187): no tickets, no draw — set, closed empty,
/// open mint at 0.05, 0.01 for accounts on the list, at most `cap` each.
#[test]
fn listed_accounts_mint_for_less_and_nobody_mints_past_the_cap() {
    let mut c = Chain::new();
    for who in [ALICE, BOB, CAROL] {
        c.mint(who, 5);
    }
    c.ok(Op::RegisterCollection { program: program::register_collection(), asset: CAVE, creator: TREASURY, quote: ZEC, fee_bps: 100, cap: 12 });
    c.ok(Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: TICKET, claim_price: CLAIM, open_price: OPEN, treasury_bps: 1_000 });
    c.ok(Op::GateSale { program: program::sale_open(), collection: CAVE, base: 10 });
    let allow = |keys: Vec<Hash>| Op::AllowBuyers { program: program::sale_open(), collection: CAVE, keys };
    c.ok(allow(vec![allow_key(&CAVE, &ALICE)]));
    c.ok(Op::CloseSale { program: program::sale_close(), collection: CAVE, zcash_height: 10, zcash_hash: [7; 32] });

    // Terms: operator-only, on the open-mint program, listed price ≤ open price, cap ≥ 1.
    let terms = |listed: Fixed, cap: u32| Op::SetOpenTerms { program: program::sale_open_mint(), collection: CAVE, listed_price: listed, cap };
    assert_eq!(ObjectVm::intent_authorities(&terms(LISTED, 2)), Vec::<AccountId>::new());
    assert!(c.refused(Op::SetOpenTerms { program: program::sale_open(), collection: CAVE, listed_price: LISTED, cap: 2 }).contains("wrong program"));
    assert!(c.refused(terms(Fixed(OPEN.0 + 1), 2)).contains("at most the open price"));
    assert!(c.refused(terms(LISTED, 0)).contains("at least one"));
    let (state, saved) = (c.vm.state_root(), c.vm.encode());
    assert!(!saved.windows(8).any(|w| w == b"MARKET18"), "no terms, no MARKET18");
    assert_eq!(c.ok(terms(LISTED, 2)), Receipt::OpenTermsSet { collection: CAVE, listed_price: LISTED, cap: 2 });
    assert_ne!(c.vm.state_root(), state, "the terms are in the root");
    assert!(c.vm.encode().windows(8).any(|w| w == b"MARKET18"));

    // Straight to the open mint: the order the rest sell in.
    let order = [4u32, 9, 1, 7, 0, 11, 3];
    let open: Vec<Hash> = order.iter().enumerate().map(|(i, s)| open_leaf(i as u32, *s)).collect();
    c.ok(Op::PostOpenMint { program: program::sale_open_mint(), collection: CAVE, root: merkle_root(&open), count: order.len() as u32 });
    let mint = |c: &mut Chain, who: AccountId, pay: Fixed| {
        let position = c.vm.market.sales[&CAVE].open_next as usize;
        let serial = order[position];
        let item = c.note(who, item_asset(&CAVE, serial), ITEM_UNIT);
        let nonce = c.fresh();
        let (inputs, change) = c.fund(who, pay);
        c.apply(Op::MintOpen { buyer: who, program: program::sale_mint(), collection: CAVE, serial, proof: merkle_proof(&open, position), item, nonce, inputs, change })
    };

    // Alice is listed: 0.01, and her payment splits like any sale payment.
    let floor = c.floor();
    assert!(matches!(mint(&mut c, ALICE, LISTED), Receipt::ItemsClaimed { .. }));
    assert_eq!(c.floor().0 - floor.0, floor_share(LISTED));
    // Carol is not: 0.01 does not pay for her item; 0.05 does.
    assert!(matches!(mint(&mut c, CAROL, LISTED), Receipt::Refused(_)));
    assert!(matches!(mint(&mut c, CAROL, OPEN), Receipt::ItemsClaimed { .. }));
    // Terms are fixed from the first mint.
    assert!(c.refused(terms(LISTED, 20)).contains("before the first mint"));
    // The cap counts each account: Alice's second is her last.
    assert!(matches!(mint(&mut c, ALICE, LISTED), Receipt::ItemsClaimed { .. }));
    match mint(&mut c, ALICE, LISTED) {
        Receipt::Refused(why) => assert!(why.contains("as many as one account may"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(c.vm.market.open_terms[&CAVE].minted[&allow_key(&CAVE, &ALICE)], 2);
    // Bob is listed during the open mint and mints at the listed price.
    assert!(matches!(mint(&mut c, BOB, LISTED), Receipt::Refused(_)));
    c.ok(allow(vec![allow_key(&CAVE, &BOB)]));
    assert!(matches!(mint(&mut c, BOB, LISTED), Receipt::ItemsClaimed { .. }));
    assert_eq!(c.vm.market.sales[&CAVE].open_next, 4);
    assert_eq!(c.vm.market.collections[&CAVE].minted, 4);
}

/// Without terms, the open mint is as before: one price, no cap, and the
/// list closes at the open mint.
#[test]
fn without_terms_the_open_mint_is_unchanged() {
    let mut c = Chain::new();
    c.mint(ALICE, 5);
    c.ok(Op::RegisterCollection { program: program::register_collection(), asset: CAVE, creator: TREASURY, quote: ZEC, fee_bps: 100, cap: 12 });
    c.ok(Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: TICKET, claim_price: CLAIM, open_price: OPEN, treasury_bps: 1_000 });
    c.ok(Op::GateSale { program: program::sale_open(), collection: CAVE, base: 10 });
    c.ok(Op::CloseSale { program: program::sale_close(), collection: CAVE, zcash_height: 10, zcash_hash: [7; 32] });
    let order = [2u32, 5, 8];
    let open: Vec<Hash> = order.iter().enumerate().map(|(i, s)| open_leaf(i as u32, *s)).collect();
    c.ok(Op::PostOpenMint { program: program::sale_open_mint(), collection: CAVE, root: merkle_root(&open), count: 3 });
    assert!(c.refused(Op::AllowBuyers { program: program::sale_open(), collection: CAVE, keys: vec![allow_key(&CAVE, &ALICE)] }).contains("list is closed"));
    for position in 0..3 {
        let serial = order[position];
        let item = c.note(ALICE, item_asset(&CAVE, serial), ITEM_UNIT);
        let nonce = c.fresh();
        let (inputs, change) = c.fund(ALICE, OPEN);
        c.ok(Op::MintOpen { buyer: ALICE, program: program::sale_mint(), collection: CAVE, serial, proof: merkle_proof(&open, position), item, nonce, inputs, change });
    }
    assert!(!c.vm.encode().windows(8).any(|w| w == b"MARKET18"));
}
