//! A collection's DAO on the ledger (DECISIONS §172): passes that lapse with
//! their item, proposals backed and voted by items once each, a treasury the
//! creator shares fill and only a passed proposal empties — with the books,
//! the state's round trip and the intent's round trip checked after every op.

use objectvm::dao::{pass_of, payment_source, Payment, State, BACK, NO, YES};
use objectvm::market::{claim_id, item_asset, ITEM_UNIT};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{commitment, nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x10; 32];
const CAVE: [u8; 32] = [0xCA; 32];
const OTHER: [u8; 32] = [0x0C; 32];
const TREASURY: AccountId = [0x7E; 32];
const ALICE: AccountId = [0xA1; 32];
const BOB: AccountId = [0xB0; 32];
const CAROL: AccountId = [0xC0; 32];
const ZAT: i128 = 10_000_000_000;
const TICKET: Fixed = Fixed(1_000_000 * ZAT); // 0.01

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
        let mut c = Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, salt: 0 };
        c.ok(Op::RegisterCollection { program: program::register_collection(), asset: CAVE, creator: TREASURY, quote: ZEC, fee_bps: 100, cap: 20 });
        c.ok(Op::RegisterCollection { program: program::register_collection(), asset: OTHER, creator: TREASURY, quote: ZEC, fee_bps: 100, cap: 20 });
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

    fn seal(&mut self, epochs: u64) {
        for _ in 0..epochs {
            self.ok(Op::Seal);
        }
    }

    /// Mint `serial` of `collection` to `owner`; its commitment.
    fn item(&mut self, collection: [u8; 32], serial: u32, owner: AccountId) -> Hash {
        let nonce = self.fresh();
        self.ok(Op::MintItem { program: program::mint_item(), collection, serial, owner, nonce, enc: enc() });
        commitment(&Note { asset: item_asset(&collection, serial), amount: ITEM_UNIT, owner, nonce }).unwrap()
    }

    /// Move an item note to `to`; its new commitment.
    fn give(&mut self, from: AccountId, c: Hash, to: AccountId, collection: [u8; 32], serial: u32) -> Hash {
        let nonce = self.fresh();
        let note = Note { asset: item_asset(&collection, serial), amount: ITEM_UNIT, owner: to, nonce };
        self.ok(Op::Spend { spender: from, program: program::private_transfer(), inputs: vec![(c, nullifier(&[from[0]; 32], &c))], outputs: vec![Output { note, enc: enc() }] });
        commitment(&note).unwrap()
    }

    fn dao(&mut self, backers: u32, voting_epochs: u64, quorum_bps: u32) {
        let op = Op::SetDao { program: program::dao_configure(), collection: CAVE, backers, voting_epochs, quorum_bps };
        assert!(ObjectVm::intent_authorities(&op).is_empty(), "only the operator sets a DAO");
        assert!(matches!(self.ok(op), Receipt::DaoSet { .. }));
    }

    fn pass(&mut self, holder: AccountId, item: Hash, token: &[u8]) -> Op {
        Op::MemberPass { holder, program: program::dao(), collection: CAVE, item, pass: pass_of(token) }
    }

    fn vote(&mut self, voter: AccountId, proposal: u32, items: Vec<Hash>, choice: u8) -> Op {
        Op::Vote { voter, program: program::dao(), collection: CAVE, proposal, items, choice }
    }

    fn state(&self, n: u32) -> State {
        self.vm.market.daos[&CAVE].proposals[&n].state
    }

    fn treasury(&self) -> Fixed {
        self.vm.market.daos[&CAVE].treasury.get(&ZEC).copied().unwrap_or(Fixed::ZERO)
    }

    /// Buy tickets in CAVE's sale, so its treasury share flows.
    fn buy(&mut self, buyer: AccountId, count: u32) {
        let mint = self.fresh();
        self.ok(Op::Mint { asset: ZEC, amount: Fixed(TICKET.0 * count as i128), owner: buyer, nonce: mint });
        let c = commitment(&Note { asset: ZEC, amount: Fixed(TICKET.0 * count as i128), owner: buyer, nonce: mint }).unwrap();
        let nonce = self.fresh();
        self.ok(Op::BuyTickets { buyer, program: program::sale_tickets(), collection: CAVE, count, nonce, inputs: vec![(c, nullifier(&[buyer[0]; 32], &c))], change: vec![] });
    }
}

#[test]
fn a_pass_is_valid_while_its_item_is_held() {
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    let o = c.item(OTHER, 1, ALICE);
    let op = c.pass(ALICE, a, b"alice-token");
    assert_eq!(c.refused(op), "that collection has no DAO");
    c.dao(2, 10, 1_000);
    assert_eq!(ObjectVm::intent_authorities(&c.pass(ALICE, a, b"t")), vec![ALICE], "the holder signs their pass");

    let op = c.pass(BOB, a, b"bob-token");
    assert_eq!(c.refused(op), "not the holder's item");
    let op = c.pass(ALICE, o, b"other");
    assert_eq!(c.refused(op), "not an item of that collection");
    let op = c.pass(ALICE, a, b"alice-token");
    assert!(matches!(c.ok(op), Receipt::PassSet { .. }));
    assert!(c.vm.pass_valid(&CAVE, &pass_of(b"alice-token")));
    assert!(!c.vm.pass_valid(&CAVE, &pass_of(b"guess")));

    // One pass, one item: another item cannot take a live pass.
    let b = c.item(CAVE, 2, BOB);
    let op = c.pass(BOB, b, b"alice-token");
    assert_eq!(c.refused(op), "that pass is another item's");

    // Selling the item retires its pass, with nobody acting; the buyer makes
    // their own, and may reuse the token now that its item is gone.
    let moved = c.give(ALICE, a, CAROL, CAVE, 1);
    assert!(!c.vm.pass_valid(&CAVE, &pass_of(b"alice-token")));
    let op = c.pass(CAROL, moved, b"alice-token");
    c.ok(op);
    assert!(c.vm.pass_valid(&CAVE, &pass_of(b"alice-token")));
    assert_eq!(c.vm.market.daos[&CAVE].passes.len(), 1, "the dead pass was retired");
}

#[test]
fn shares_fill_the_treasury_and_only_a_passed_proposal_pays_from_it() {
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    let b1 = c.item(CAVE, 2, BOB);
    let b2 = c.item(CAVE, 3, BOB);
    let k = c.item(CAVE, 4, CAROL);
    c.ok(Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: TICKET, claim_price: TICKET, open_price: TICKET, treasury_bps: 1_000 });

    // Before the DAO, the share is a claim for the creator.
    c.buy(CAROL, 1);
    assert_eq!(c.vm.market.claims.values().filter(|x| x.owner == TREASURY).count(), 1);
    c.dao(2, 5, 5_000);
    // After it, the share is the treasury's and no new creator claim appears.
    c.buy(CAROL, 10);
    assert_eq!(c.treasury(), Fixed(TICKET.0));
    assert_eq!(c.vm.market.claims.values().filter(|x| x.owner == TREASURY).count(), 1);

    // Alice proposes paying Carol 0.006 ZEC.zy; her item backs it first.
    let pay = Payment { asset: ZEC, amount: Fixed(TICKET.0 * 6 / 10), to: CAROL };
    let text = [0x7a; 32];
    let op = Op::Propose { proposer: ALICE, program: program::dao(), collection: CAVE, item: a, text, pay: Some(pay) };
    assert_eq!(ObjectVm::intent_authorities(&op), vec![ALICE]);
    assert!(matches!(c.ok(op), Receipt::Proposed { id: 1, .. }));
    assert!(matches!(c.state(1), State::Backing { .. }));
    let op = c.vote(ALICE, 1, vec![a], BACK);
    assert_eq!(c.refused(op), "every item has already backed this proposal");
    let op = c.vote(ALICE, 1, vec![a], YES);
    assert_eq!(c.refused(op), "this proposal is not open for votes");

    // A second backer opens the vote.
    let op = c.vote(BOB, 1, vec![b1], BACK);
    c.ok(op);
    assert!(matches!(c.state(1), State::Voting { .. }));

    // Two items of Bob's vote yes at once; the same item cannot vote twice,
    // nor again after it changes hands.
    let op = c.vote(BOB, 1, vec![b1, b1], YES);
    assert_eq!(c.refused(op), "the same item twice");
    let op = c.vote(BOB, 1, vec![b1, b2], YES);
    assert!(matches!(c.ok(op), Receipt::Voted { items: 2, .. }));
    let b1_moved = c.give(BOB, b1, CAROL, CAVE, 2);
    let op = c.vote(CAROL, 1, vec![b1_moved], NO);
    assert_eq!(c.refused(op), "every item has already voted on this proposal");
    // A mix of an item that voted and one that did not counts only the new one.
    let op = c.vote(CAROL, 1, vec![b1_moved, k], NO);
    assert!(matches!(c.ok(op), Receipt::Voted { items: 1, .. }));

    // 2 yes, 1 no, 3 of 4 outstanding voted (quorum 50%): it passes when the
    // vote closes, and Carol is owed the payment from the treasury.
    let before = c.treasury();
    c.seal(6);
    assert_eq!(c.state(1), State::Passed);
    assert_eq!(c.treasury(), Fixed(before.0 - pay.amount.0));
    let claim = &c.vm.market.claims[&claim_id(&payment_source(&CAVE, 1), 0)];
    assert_eq!((claim.owner, claim.asset, claim.amount), (CAROL, ZEC, pay.amount));
    let op = c.vote(ALICE, 1, vec![a], YES);
    assert_eq!(c.refused(op), "this proposal is not open for votes");
}

#[test]
fn proposals_lapse_fail_or_go_unfunded() {
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    let b = c.item(CAVE, 2, BOB);
    for s in 3..=6 {
        c.item(CAVE, s, CAROL);
    }
    c.dao(2, 3, 5_000);

    // Too few backers in time: lapsed.
    let op = Op::Propose { proposer: ALICE, program: program::dao(), collection: CAVE, item: a, text: [1; 32], pay: None };
    c.ok(op);
    c.seal(3);
    assert_eq!(c.state(1), State::Lapsed);
    let op = c.vote(BOB, 1, vec![b], BACK);
    assert_eq!(c.refused(op), "this proposal is not gathering backers");

    // Backed, but too few vote (2 of 6 items, quorum 3): failed.
    let op = Op::Propose { proposer: ALICE, program: program::dao(), collection: CAVE, item: a, text: [2; 32], pay: None };
    c.ok(op);
    let op = c.vote(BOB, 2, vec![b], BACK);
    c.ok(op);
    let op = c.vote(ALICE, 2, vec![a], YES);
    c.ok(op);
    let op = c.vote(BOB, 2, vec![b], YES);
    c.ok(op);
    c.seal(3);
    assert_eq!(c.state(2), State::Failed);

    // Passed, but the treasury is empty: unfunded, and nothing is paid.
    let pay = Payment { asset: ZEC, amount: Fixed::whole(1), to: BOB };
    let op = Op::Propose { proposer: ALICE, program: program::dao(), collection: CAVE, item: a, text: [3; 32], pay: Some(pay) };
    c.ok(op);
    let op = c.vote(BOB, 3, vec![b], BACK);
    c.ok(op);
    let carol: Vec<Hash> = c.vm.live.iter().filter(|(_, n)| n.owner == CAROL).map(|(h, _)| *h).collect();
    let op = c.vote(CAROL, 3, carol, YES);
    assert!(matches!(c.ok(op), Receipt::Voted { items: 4, .. }));
    c.seal(3);
    assert_eq!(c.state(3), State::Unfunded);
    assert!(!c.vm.market.claims.contains_key(&claim_id(&payment_source(&CAVE, 3), 0)));
}

#[test]
fn a_dao_needs_a_collection_and_sane_terms() {
    let mut c = Chain::new();
    let op = Op::SetDao { program: program::dao_configure(), collection: [9; 32], backers: 2, voting_epochs: 10, quorum_bps: 1_000 };
    assert_eq!(c.refused(op), "that is not a registered collection");
    let op = Op::SetDao { program: program::dao_configure(), collection: CAVE, backers: 0, voting_epochs: 10, quorum_bps: 1_000 };
    assert!(c.refused(op).starts_with("backers and voting time must be positive"));
    let op = Op::SetDao { program: program::dao(), collection: CAVE, backers: 2, voting_epochs: 10, quorum_bps: 1_000 };
    assert_eq!(c.refused(op), "wrong program for a DAO's terms");
}
