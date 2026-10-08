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
        Chain::on(212)
    }

    fn on(chain: u32) -> Chain {
        let mut c = Chain { vm: ObjectVm::genesis(chain, Limits { max_io: 16 }), seq: 0, salt: 0 };
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

// ---------------------------------------------------------------------------
// Strict rules (§173): the audit's findings, each now refused.

impl Chain {
    fn clock(&mut self, height: u32) {
        let op = Op::DaoClock { program: program::dao_configure(), height, hash: [height as u8 | 1; 32] };
        assert!(ObjectVm::intent_authorities(&op).is_empty());
        self.ok(op);
    }

    fn terms(&mut self, backers: u32, window_blocks: u64, quorum_bps: u32) -> Receipt {
        self.apply(Op::SetDaoTerms { program: program::dao_configure(), collection: CAVE, backers, window_blocks, quorum_bps })
    }

    fn propose(&mut self, who: AccountId, item: Hash, text: u8, pay: Option<Payment>) -> Receipt {
        self.apply(Op::Propose { proposer: who, program: program::dao(), collection: CAVE, item, text: [text; 32], pay })
    }
}

#[test]
fn the_clock_starts_strict_rules_and_only_moves_forward() {
    let mut c = Chain::new();
    c.dao(1, 5, 1_000);
    assert_eq!(c.refused(Op::SetDaoTerms { program: program::dao_configure(), collection: CAVE, backers: 1, window_blocks: 8, quorum_bps: 500 }), "start the Zcash clock first");
    c.clock(100);
    let op = Op::DaoClock { program: program::dao_configure(), height: 100, hash: [7; 32] };
    assert_eq!(c.refused(op), "the Zcash clock only moves forward");
    let op = Op::SetDao { program: program::dao_configure(), collection: CAVE, backers: 1, voting_epochs: 5, quorum_bps: 0 };
    assert_eq!(c.refused(op), "the Zcash clock has started: a DAO's terms are set with SetDaoTerms");
    assert!(matches!(c.terms(1, 8, 500), Receipt::DaoSet { .. }));
    assert!(c.vm.market.daos[&CAVE].strict.is_some());
}

#[test]
fn the_operator_cannot_drain_a_strict_treasury() {
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    let b = c.item(CAVE, 2, BOB);
    c.ok(Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: TICKET, claim_price: TICKET, open_price: TICKET, treasury_bps: 1_000 });
    c.dao(2, 5, 1_000);
    c.clock(1_000);
    c.terms(2, 10, 5_000);
    c.buy(CAROL, 20);
    let full = c.treasury();
    assert!(full.0 > 0);

    // The old path: mint itself an item, retune, propose, vote, seal. Each
    // step is refused or no longer enough.
    let nonce = c.fresh();
    let op = Op::MintItem { program: program::mint_item(), collection: CAVE, serial: 9, owner: TREASURY, nonce, enc: enc() };
    assert_eq!(c.refused(op), "a DAO collection's items come only through its sale");
    let grab = Payment { asset: ZEC, amount: full, to: TREASURY };
    assert!(matches!(c.propose(ALICE, a, 1, Some(grab)), Receipt::Proposed { id: 1, .. }));
    // Terms are fixed once there is a proposal.
    assert_eq!(c.refused(Op::SetDaoTerms { program: program::dao_configure(), collection: CAVE, backers: 1, window_blocks: 1, quorum_bps: 0 }), "a DAO's terms are fixed once it has a proposal");
    let op = c.vote(BOB, 1, vec![b], BACK);
    c.ok(op);
    assert!(matches!(c.state(1), State::Voting { until: 1_010 }));
    let op = c.vote(ALICE, 1, vec![a], YES);
    c.ok(op);
    // Sealing a thousand epochs does not move a window measured in Zcash blocks.
    c.seal(1_000);
    assert!(matches!(c.state(1), State::Voting { .. }));
    assert_eq!(c.treasury(), full);
    // Zcash moves on; with 1 yes of 2 outstanding at a 50% quorum it passes.
    c.clock(1_010);
    c.seal(1);
    assert_eq!(c.state(1), State::Passed);
}

#[test]
fn mainnet_terms_have_floors() {
    let mut c = Chain::on(objectvm::dao::MAINNET);
    c.clock(10);
    for (b, w, q) in [(2, 8_064, 1_000), (10, 2_000, 1_000), (10, 8_064, 100), (10, 40_000, 1_000)] {
        assert!(matches!(c.terms(b, w, q), Receipt::Refused(_)), "{b} {w} {q}");
    }
    assert!(matches!(c.terms(10, 8_064, 1_000), Receipt::DaoSet { .. }));
}

#[test]
fn a_strict_proposal_keeps_its_terms_and_its_quorum_base() {
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    let mut carol = Vec::new();
    for s in 2..=5 {
        carol.push(c.item(CAVE, s, CAROL));
    }
    c.clock(1);
    c.terms(1, 10, 5_000);
    // Opened with 5 outstanding: quorum 3 (50%, rounded up).
    c.propose(ALICE, a, 1, None);
    if let Some(t) = c.vm.market.daos[&CAVE].proposals[&1].terms {
        assert_eq!((t.base, t.backers, t.window, t.quorum_bps), (5, 1, 10, 5_000));
    } else {
        panic!("a strict proposal records its terms");
    }
    let op = c.vote(ALICE, 1, vec![a], YES);
    c.ok(op);
    // Redeeming two of Carol's items mid-vote does not lower the quorum.
    let n0 = nullifier(&[CAROL[0]; 32], &carol[0]);
    let n1 = nullifier(&[CAROL[0]; 32], &carol[1]);
    c.ok(Op::Redeem { holder: CAROL, program: program::redeem_item(), asset: CAVE, inputs: vec![(carol[0], n0), (carol[1], n1)] });
    c.clock(20);
    c.seal(1);
    assert_eq!(c.state(1), State::Failed, "1 vote of a quorum of 3 fixed when the vote opened");
}

#[test]
fn one_open_proposal_per_item_and_at_most_sixteen() {
    let mut c = Chain::new();
    let mut items = Vec::new();
    for s in 1..=18 {
        items.push(c.item(CAVE, s, ALICE));
    }
    c.clock(1);
    c.terms(1, 10, 500);
    c.propose(ALICE, items[0], 1, None);
    assert_eq!(c.propose(ALICE, items[0], 2, None), Receipt::Refused("that item already has an open proposal"));
    for (i, it) in items.iter().enumerate().take(16).skip(1) {
        assert!(matches!(c.propose(ALICE, *it, i as u8, None), Receipt::Proposed { .. }));
    }
    assert_eq!(c.propose(ALICE, items[16], 99, None), Receipt::Refused("the DAO has 16 open proposals; wait for one to close"));
    // Once they close, the item may propose again.
    c.clock(11);
    c.seal(1);
    assert!(matches!(c.propose(ALICE, items[0], 3, None), Receipt::Proposed { .. }));
}

#[test]
fn closed_strict_proposals_are_pruned_to_the_last_64() {
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    c.clock(1);
    c.terms(1, 1, 500);
    let mut h = 1;
    for i in 0..70u32 {
        assert!(matches!(c.propose(ALICE, a, (i % 250) as u8, None), Receipt::Proposed { .. }));
        h += 1;
        c.clock(h);
        c.seal(1);
    }
    let d = &c.vm.market.daos[&CAVE];
    assert_eq!(d.proposals.len(), objectvm::dao::KEEP_CLOSED);
    assert_eq!(*d.proposals.keys().next().unwrap(), 7, "the oldest six are gone");
    assert_eq!(d.strict.unwrap().next_id, 71, "numbers are never reused");
}

#[test]
fn a_dao_with_an_open_first_rules_proposal_waits_before_going_strict() {
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    c.dao(2, 5, 1_000);
    c.propose(ALICE, a, 1, None);
    c.clock(1);
    assert_eq!(c.terms(1, 10, 500), Receipt::Refused("close the DAO's open proposals before moving it to strict rules"));
    c.seal(6);
    assert_eq!(c.state(1), State::Lapsed);
    assert!(matches!(c.terms(1, 10, 500), Receipt::DaoSet { .. }));
    assert!(matches!(c.propose(ALICE, a, 2, None), Receipt::Proposed { id: 2, .. }));
}

#[test]
fn registering_the_same_pass_again_is_refused() {
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    c.dao(2, 10, 1_000);
    let op = c.pass(ALICE, a, b"t");
    c.ok(op);
    let op = c.pass(ALICE, a, b"t");
    assert_eq!(c.refused(op), "that item already has that pass");
    let op = c.pass(ALICE, a, b"u");
    c.ok(op);
}

// ---------------------------------------------------------------------------
// The Zyn track (§176): members vote on Zyn's issues; the team's own account
// casts one vote, a veto or a weighted share.
// ---------------------------------------------------------------------------

const TEAM: AccountId = [0x7A; 32];

impl Chain {
    fn track(&mut self, rule: u8, share_bps: u32) -> Receipt {
        let op = Op::SetZynTrack { program: program::dao_configure(), collection: CAVE, team: TEAM, rule, share_bps };
        assert!(ObjectVm::intent_authorities(&op).is_empty(), "only the operator starts the track");
        self.apply(op)
    }

    fn propose_zyn(&mut self, who: AccountId, item: Hash, text: u8) -> Receipt {
        let op = Op::ProposeZyn { proposer: who, program: program::dao(), collection: CAVE, item, text: [text; 32] };
        assert_eq!(ObjectVm::intent_authorities(&op), vec![who]);
        self.apply(op)
    }

    fn team_vote(&mut self, team: AccountId, proposal: u32, choice: u8) -> Receipt {
        let op = Op::TeamVote { team, program: program::dao(), collection: CAVE, proposal, choice };
        assert_eq!(ObjectVm::intent_authorities(&op), vec![team], "the team's account signs its vote");
        self.apply(op)
    }

    /// Four members' items and a strict DAO with a Zyn track under `rule`;
    /// one Zyn proposal open for votes (1 backer, 10 blocks, 25% quorum).
    fn zyn_vote(rule: u8, share_bps: u32) -> (Chain, Vec<Hash>) {
        let mut c = Chain::new();
        let items: Vec<Hash> = [ALICE, BOB, CAROL, BOB].iter().enumerate().map(|(i, w)| c.item(CAVE, i as u32 + 1, *w)).collect();
        c.clock(100);
        c.terms(1, 10, 2_500);
        assert!(matches!(c.track(rule, share_bps), Receipt::ZynTrackSet { .. }));
        assert!(matches!(c.propose_zyn(ALICE, items[0], 9), Receipt::Proposed { id: 1, .. }));
        assert!(matches!(c.state(1), State::Voting { .. }));
        (c, items)
    }

    fn close(&mut self) -> State {
        self.clock(200);
        self.seal(1);
        self.state(1)
    }
}

#[test]
fn the_zyn_track_needs_strict_rules_and_is_fixed_by_its_first_proposal() {
    use objectvm::dao::{RULE_VETO, RULE_WEIGHTED};
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    let b = c.item(CAVE, 2, BOB);
    c.dao(1, 5, 1_000);
    assert_eq!(c.track(RULE_VETO, 0).to_string_refused(), "the Zyn track needs a DAO under strict rules");
    c.clock(100);
    c.terms(1, 10, 1_000);
    assert_eq!(c.propose_zyn(ALICE, a, 1).to_string_refused(), "this DAO has no Zyn track");
    for (rule, share) in [(RULE_VETO, 5_000), (RULE_WEIGHTED, 0), (RULE_WEIGHTED, 10_001), (7, 0)] {
        assert!(matches!(c.track(rule, share), Receipt::Refused(_)), "{rule} {share}");
    }
    // Retunable until its first proposal...
    assert!(matches!(c.track(RULE_WEIGHTED, 5_000), Receipt::ZynTrackSet { .. }));
    assert!(matches!(c.track(RULE_VETO, 0), Receipt::ZynTrackSet { .. }));
    // ...and a refused proposal fixes nothing.
    assert!(matches!(c.propose(ALICE, a, 1, None), Receipt::Proposed { id: 1, .. }));
    assert_eq!(c.propose_zyn(ALICE, a, 2).to_string_refused(), "that item already has an open proposal");
    assert!(!c.vm.market.daos[&CAVE].zyn.unwrap().frozen);
    assert!(matches!(c.track(RULE_VETO, 0), Receipt::ZynTrackSet { .. }));
    assert!(matches!(c.propose_zyn(BOB, b, 3), Receipt::Proposed { id: 2, .. }));
    assert!(c.vm.market.daos[&CAVE].zyn.unwrap().frozen);
    assert_eq!(c.track(RULE_WEIGHTED, 5_000).to_string_refused(), "the Zyn track's rule is fixed once it has a proposal; only its team account can change");
    // The team's account can still be replaced (a key rotated), the rule kept.
    let op = Op::SetZynTrack { program: program::dao_configure(), collection: CAVE, team: CAROL, rule: RULE_VETO, share_bps: 0 };
    assert!(matches!(c.ok(op), Receipt::ZynTrackSet { .. }));
    let z = c.vm.market.daos[&CAVE].zyn.unwrap();
    assert_eq!((z.team, z.rule, z.frozen), (CAROL, RULE_VETO, true));
    // A Zyn proposal pays nothing; a collection's proposal has no team vote.
    let z = &c.vm.market.daos[&CAVE].proposals[&2];
    assert_eq!((z.pay, z.zyn), (None, Some(0)));
    assert_eq!(c.vm.market.daos[&CAVE].proposals[&1].zyn, None);
}

#[test]
fn under_a_veto_the_members_decide_and_the_team_can_only_block() {
    use objectvm::dao::RULE_VETO;
    // Members carry it, the team blocks: it fails.
    let (mut c, it) = Chain::zyn_vote(RULE_VETO, 0);
    let op = c.vote(BOB, 1, vec![it[1], it[3]], YES);
    c.ok(op);
    assert!(matches!(c.team_vote(TEAM, 1, NO), Receipt::TeamVoted { choice: NO, .. }));
    assert_eq!(c.close(), State::Failed);

    // Members carry it and the team stays out: it passes.
    let (mut c, it) = Chain::zyn_vote(RULE_VETO, 0);
    let op = c.vote(BOB, 1, vec![it[1]], YES);
    c.ok(op);
    assert_eq!(c.close(), State::Passed);

    // The team says yes, but most members say no: it fails.
    let (mut c, it) = Chain::zyn_vote(RULE_VETO, 0);
    let op = c.vote(ALICE, 1, vec![it[0]], YES);
    c.ok(op);
    let op = c.vote(BOB, 1, vec![it[1], it[3]], NO);
    c.ok(op);
    c.team_vote(TEAM, 1, YES);
    assert_eq!(c.close(), State::Failed);

    // The team's yes does not stand in for the members' quorum (25% of 4 = 1).
    let (mut c, _) = Chain::zyn_vote(RULE_VETO, 0);
    c.team_vote(TEAM, 1, YES);
    assert_eq!(c.close(), State::Failed);
}

#[test]
fn a_weighted_team_vote_is_its_share_of_the_tally() {
    use objectvm::dao::RULE_WEIGHTED;
    // At 50%, the team's yes and one member's beat three members' no.
    let (mut c, it) = Chain::zyn_vote(RULE_WEIGHTED, 5_000);
    let op = c.vote(ALICE, 1, vec![it[0]], YES);
    c.ok(op);
    let op = c.vote(BOB, 1, vec![it[1], it[3]], NO);
    c.ok(op);
    let op = c.vote(CAROL, 1, vec![it[2]], NO);
    c.ok(op);
    c.team_vote(TEAM, 1, YES);
    assert_eq!(c.close(), State::Passed);

    // At 50%, every member's yes ties the team's no: it fails.
    let (mut c, it) = Chain::zyn_vote(RULE_WEIGHTED, 5_000);
    for (w, i) in [(ALICE, 0), (BOB, 1), (CAROL, 2), (BOB, 3)] {
        let op = c.vote(w, 1, vec![it[i]], YES);
        c.ok(op);
    }
    c.team_vote(TEAM, 1, NO);
    assert_eq!(c.close(), State::Failed);

    // At 40%, the same members outvote the team.
    let (mut c, it) = Chain::zyn_vote(RULE_WEIGHTED, 4_000);
    for (w, i) in [(ALICE, 0), (BOB, 1), (CAROL, 2), (BOB, 3)] {
        let op = c.vote(w, 1, vec![it[i]], YES);
        c.ok(op);
    }
    c.team_vote(TEAM, 1, NO);
    assert_eq!(c.close(), State::Passed);
}

#[test]
fn only_the_team_votes_as_the_team_once_on_zyn_proposals_while_open() {
    use objectvm::dao::RULE_VETO;
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    let b = c.item(CAVE, 2, BOB);
    c.clock(100);
    c.terms(2, 10, 2_500);
    c.track(RULE_VETO, 0);
    c.propose(ALICE, a, 1, None);
    c.propose_zyn(BOB, b, 2);
    assert_eq!(c.team_vote(TEAM, 1, NO).to_string_refused(), "the team votes only on Zyn proposals");
    assert_eq!(c.team_vote(TEAM, 2, NO).to_string_refused(), "this proposal is not open for votes");
    let op = c.vote(ALICE, 2, vec![a], BACK);
    c.ok(op);
    assert_eq!(c.team_vote(ALICE, 2, NO).to_string_refused(), "only the team's account casts the team's vote");
    assert_eq!(c.team_vote(TEAM, 2, BACK).to_string_refused(), "yes or no");
    c.team_vote(TEAM, 2, NO);
    assert_eq!(c.team_vote(TEAM, 2, YES).to_string_refused(), "the team has already voted on this proposal");
    c.clock(300);
    c.seal(1);
    assert_eq!(c.team_vote(TEAM, 2, NO).to_string_refused(), "the team has already voted on this proposal");
}

#[test]
fn a_chain_without_a_zyn_track_saves_as_it_did() {
    let mut c = Chain::new();
    let a = c.item(CAVE, 1, ALICE);
    c.clock(100);
    c.terms(1, 10, 1_000);
    c.propose(ALICE, a, 1, None);
    let saved = c.vm.encode();
    assert!(saved.windows(8).any(|w| w == b"MARKET15"), "no track, no new format");
    assert!(!saved.windows(8).any(|w| w == b"MARKET16"));
    c.track(objectvm::dao::RULE_VETO, 0);
    assert!(c.vm.encode().windows(8).any(|w| w == b"MARKET16"));
}

trait RefusedText {
    fn to_string_refused(&self) -> String;
}

impl RefusedText for Receipt {
    fn to_string_refused(&self) -> String {
        match self {
            Receipt::Refused(why) => why.to_string(),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}
