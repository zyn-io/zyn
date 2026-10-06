//! A collection's primary sale end to end (DECISIONS §144): tickets bought in
//! ZEC.zy, a close, a posted draw, claims proved against it, and an open mint
//! — on a real ledger, with the books checked after every intent.

use objectvm::market::{self, item_asset, ITEM_UNIT};
use objectvm::sale::{draw_leaf, open_leaf, ticket_id, treasury_source, Phase};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
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
const OPEN: Fixed = Fixed(1_330_000 * ZAT); // 0.0133

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

#[test]
fn tickets_a_draw_claims_and_an_open_mint_on_one_ledger() {
    let mut c = Chain::new();
    for who in [ALICE, BOB, CAROL] {
        c.mint(who, 1);
    }
    c.ok(Op::RegisterCollection { program: program::register_collection(), asset: CAVE, creator: TREASURY, quote: ZEC, fee_bps: 100, cap: 12 });
    let sale = Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: TICKET, claim_price: CLAIM, open_price: OPEN, treasury_bps: 1_000 };
    assert_eq!(ObjectVm::intent_authorities(&sale), Vec::<AccountId>::new(), "only the operator opens a sale");
    assert!(matches!(c.ok(sale), Receipt::SaleOpened { .. }));

    // Tickets: 90% into the floor, 10% owed to the creator, by consensus.
    let (a_nonce, op) = c.buy(ALICE, 3);
    assert_eq!(ObjectVm::intent_authorities(&op), vec![ALICE]);
    assert!(matches!(c.ok(op), Receipt::TicketsBought { count: 3, .. }));
    assert_eq!(c.floor().0, floor_share(Fixed(TICKET.0 * 3)));
    let owed = c.vm.market.claims[&market::claim_id(&treasury_source(&CAVE, &ticket_id(&ALICE, &a_nonce, 0)), 0)];
    assert_eq!((owed.owner, owed.amount.0), (TREASURY, TICKET.0 * 3 / 10));
    assert_eq!(c.balance(ALICE, ZEC).0, Fixed::whole(1).0 - TICKET.0 * 3);
    let (b_nonce, op) = c.buy(BOB, 2);
    c.ok(op);
    // A nonce buys once.
    let (inputs, change) = c.fund(BOB, TICKET);
    assert_eq!(c.refused(Op::BuyTickets { buyer: BOB, program: program::sale_tickets(), collection: CAVE, count: 1, nonce: b_nonce, inputs, change }), "that nonce was already used");
    assert_eq!(c.vm.market.sales[&CAVE].tickets, 5);
    assert_eq!(c.vm.market.tickets[&ticket_id(&BOB, &b_nonce, 1)].index, 4, "tickets keep the order they were bought in");

    // Claims wait for the draw; the draw waits for the close.
    let early = c.claim_op(ALICE, a_nonce, 0, &[3], vec![]);
    assert_eq!(c.refused(early), "claims are open between the draw and the open mint");
    assert_eq!(c.refused(Op::PostDraw { program: program::sale_draw(), collection: CAVE, root: [9; 32] }), "the draw follows the close, once");
    c.ok(Op::CloseSale { program: program::sale_close(), collection: CAVE, zcash_height: 3_600_000, zcash_hash: [0x5a; 32] });
    let (late_nonce, late) = c.buy(CAROL, 1);
    let _ = late_nonce;
    assert_eq!(c.refused(late), "the sale is closed");

    // The draw, as cave-randomness would post it: ticket → serials won.
    let won: Vec<(Hash, Vec<u32>)> = vec![
        (ticket_id(&ALICE, &a_nonce, 0), vec![3]),
        (ticket_id(&ALICE, &a_nonce, 1), vec![7, 1]),
        (ticket_id(&ALICE, &a_nonce, 2), vec![10]),
        (ticket_id(&BOB, &b_nonce, 0), vec![5]),
        (ticket_id(&BOB, &b_nonce, 1), vec![0, 2, 9]),
    ];
    let leaves: Vec<Hash> = won.iter().map(|(t, s)| draw_leaf(t, s)).collect();
    let root = merkle_root(&leaves);
    assert!(matches!(c.ok(Op::PostDraw { program: program::sale_draw(), collection: CAVE, root }), Receipt::DrawPosted { .. }));
    assert_eq!(c.vm.market.sales[&CAVE].phase, Phase::Drawn);

    // A winner claims what they drew, pays the claim price, and holds it.
    let floor_before = c.floor();
    let op = c.claim_op(ALICE, a_nonce, 1, &[7, 1], merkle_proof(&leaves, 1));
    assert_eq!(ObjectVm::intent_authorities(&op), vec![ALICE]);
    match c.ok(op) {
        Receipt::ItemsClaimed { created } => assert_eq!(created.len(), 2),
        other => panic!("{other:?}"),
    }
    assert_eq!(c.balance(ALICE, item_asset(&CAVE, 7)), ITEM_UNIT);
    assert_eq!(c.balance(ALICE, item_asset(&CAVE, 1)), ITEM_UNIT);
    assert_eq!(c.floor().0 - floor_before.0, floor_share(Fixed(CLAIM.0 * 2)));
    assert_eq!(c.vm.market.collections[&CAVE].minted, 2);

    // Nobody gets what the draw did not give them.
    let wrong = c.claim_op(ALICE, a_nonce, 0, &[4], merkle_proof(&leaves, 0));
    assert_eq!(c.refused(wrong), "that is not what this ticket drew");
    let extra = c.claim_op(ALICE, a_nonce, 0, &[3, 4], merkle_proof(&leaves, 0));
    assert_eq!(c.refused(extra), "that is not what this ticket drew");
    let again = c.claim_op(ALICE, a_nonce, 1, &[7, 1], merkle_proof(&leaves, 1));
    assert_eq!(c.refused(again), "that ticket was already claimed");
    // Bob cannot claim Alice's ticket: its id is hers alone.
    let theft = c.claim_op(BOB, a_nonce, 0, &[3], merkle_proof(&leaves, 0));
    assert_eq!(c.refused(theft), "no such ticket is yours");
    // Items go to the claimant or not at all.
    let mut to_carol = c.claim_op(ALICE, a_nonce, 0, &[3], merkle_proof(&leaves, 0));
    if let Op::ClaimItems { items, .. } = &mut to_carol {
        items[0].note.owner = CAROL;
    }
    assert_eq!(c.refused(to_carol), "an item output must be that serial, one unit, to the claimant");
    let bob = c.claim_op(BOB, b_nonce, 1, &[0, 2, 9], merkle_proof(&leaves, 4));
    c.ok(bob);

    // Claims end; the rest sell in a posted order at the open price.
    let rest = [3u32, 10, 5, 4, 6, 8, 11];
    let open: Vec<Hash> = rest.iter().enumerate().map(|(i, s)| open_leaf(i as u32, *s)).collect();
    c.ok(Op::PostOpenMint { program: program::sale_open_mint(), collection: CAVE, root: merkle_root(&open), count: rest.len() as u32 });
    let late_claim = c.claim_op(ALICE, a_nonce, 0, &[3], merkle_proof(&leaves, 0));
    assert_eq!(c.refused(late_claim), "claims are open between the draw and the open mint");
    let mint_open = |c: &mut Chain, who: AccountId, position: usize, serial: u32| {
        let item = c.note(who, item_asset(&CAVE, serial), ITEM_UNIT);
        let nonce = c.fresh();
        let (inputs, change) = c.fund(who, OPEN);
        Op::MintOpen { buyer: who, program: program::sale_mint(), collection: CAVE, serial, proof: merkle_proof(&open, position), item, nonce, inputs, change }
    };
    let skip = mint_open(&mut c, CAROL, 1, 10);
    assert_eq!(c.refused(skip), "that is not the next item in the open mint");
    let first = mint_open(&mut c, CAROL, 0, 3);
    c.ok(first);
    assert_eq!(c.balance(CAROL, item_asset(&CAVE, 3)), ITEM_UNIT);
    assert_eq!(c.vm.market.sales[&CAVE].open_next, 1);
    let second = mint_open(&mut c, CAROL, 1, 10);
    c.ok(second);
    assert_eq!(c.vm.market.collections[&CAVE].minted, 7);
}

#[test]
fn a_sale_joins_the_state_only_once_it_exists() {
    let mut c = Chain::new();
    c.ok(Op::RegisterCollection { program: program::register_collection(), asset: CAVE, creator: TREASURY, quote: ZEC, fee_bps: 100, cap: 4 });
    let before = c.vm.state_root();
    c.ok(Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: TICKET, claim_price: CLAIM, open_price: OPEN, treasury_bps: 1_000 });
    assert_ne!(c.vm.state_root(), before, "an open sale is committed");
    // Prices are fixed once tickets are sold.
    c.mint(ALICE, 1);
    let (_, op) = c.buy(ALICE, 1);
    c.ok(op);
    let again = Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: Fixed(1), claim_price: CLAIM, open_price: OPEN, treasury_bps: 0 };
    assert_eq!(c.refused(again), "the sale has started; its prices are fixed");
    // Limits hold.
    let (inputs, change) = c.fund(ALICE, TICKET);
    let too_many = Op::BuyTickets { buyer: ALICE, program: program::sale_tickets(), collection: CAVE, count: 21, nonce: [7; 32], inputs, change };
    assert_eq!(c.refused(too_many), "buy between 1 and 20 tickets at a time");
    let greedy = Op::SetSale { program: program::sale_open(), collection: [0xCB; 32], ticket_price: TICKET, claim_price: CLAIM, open_price: OPEN, treasury_bps: 2_001 };
    assert_eq!(c.refused(greedy), "register the collection first");
}

/// Burn tickets (§145): a burn recorded once becomes tickets in the same
/// order, capped, paying nothing in; its winner claims by revealing the
/// secret from their own account, and nobody else can use that secret.
#[test]
fn burn_tickets_join_the_draw_and_only_their_burner_claims_them() {
    use objectvm::sale::{voucher_commitment, voucher_ticket_id};
    let mut c = Chain::new();
    for who in [ALICE, BOB] {
        c.mint(who, 1);
    }
    c.ok(Op::RegisterCollection { program: program::register_collection(), asset: CAVE, creator: TREASURY, quote: ZEC, fee_bps: 100, cap: 12 });
    c.ok(Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: TICKET, claim_price: CLAIM, open_price: OPEN, treasury_bps: 1_000 });
    let secret = [0x5e; 32];
    let commitment = voucher_commitment(&ALICE, &secret);
    let post = |burn: u8, commitment: Hash, count: u32| Op::PostVoucher { program: program::sale_vouchers(), collection: CAVE, burn: [burn; 32], commitment, count };
    assert_eq!(ObjectVm::intent_authorities(&post(1, commitment, 1)), Vec::<AccountId>::new(), "only the operator posts a burn");

    // No cap, no burn tickets.
    assert!(c.refused(post(1, commitment, 2)).contains("all taken"));
    c.ok(Op::SetVoucherCap { program: program::sale_vouchers(), collection: CAVE, cap: 3 });

    // A paid ticket first, then the burn: both in one order.
    let (b_nonce, op) = c.buy(BOB, 1);
    c.ok(op);
    let floor = c.floor();
    assert_eq!(c.ok(post(1, commitment, 2)), Receipt::VoucherPosted { first: voucher_ticket_id(&commitment, 0), count: 2 });
    assert_eq!(c.floor(), floor, "a burn ticket pays nothing in");
    assert_eq!(c.vm.market.tickets[&voucher_ticket_id(&commitment, 1)].index, 2);
    assert!(c.refused(post(1, voucher_commitment(&ALICE, &[9; 32]), 1)).contains("already posted"), "a burn counts once");
    assert!(c.refused(post(2, commitment, 1)).contains("already used"), "a commitment counts once");
    assert!(c.refused(post(3, voucher_commitment(&BOB, &[8; 32]), 2)).contains("all taken"), "the cap holds");
    c.ok(Op::SetVoucherCap { program: program::sale_vouchers(), collection: CAVE, cap: 2 });
    assert!(c.refused(Op::SetVoucherCap { program: program::sale_vouchers(), collection: CAVE, cap: 1 }).contains("already posted"));

    // Close and draw: Bob's ticket wins [0], Alice's burn tickets [1] and [2, 3].
    c.ok(Op::CloseSale { program: program::sale_close(), collection: CAVE, zcash_height: 10, zcash_hash: [7; 32] });
    assert!(c.refused(post(4, voucher_commitment(&BOB, &[6; 32]), 1)).contains("closed"));
    let wins: Vec<(Hash, Vec<u32>)> = vec![
        (ticket_id(&BOB, &b_nonce, 0), vec![0]),
        (voucher_ticket_id(&commitment, 0), vec![1]),
        (voucher_ticket_id(&commitment, 1), vec![2, 3]),
    ];
    let leaves: Vec<Hash> = wins.iter().map(|(t, s)| draw_leaf(t, s)).collect();
    c.ok(Op::PostDraw { program: program::sale_draw(), collection: CAVE, root: merkle_root(&leaves) });

    let voucher_claim = |c: &mut Chain, buyer: AccountId, k: u32, serials: &[u32], i: usize| {
        let items = c.items(buyer, serials);
        let (inputs, change) = c.fund(buyer, Fixed(CLAIM.0 * serials.len() as i128));
        Op::ClaimVoucherItems {
            buyer,
            program: program::sale_claim(),
            collection: CAVE,
            secret,
            k,
            serials: serials.to_vec(),
            proof: merkle_proof(&leaves, i),
            items,
            inputs,
            change,
        }
    };
    // Bob saw the secret nowhere yet — but even holding it, it is Alice's.
    let stolen = voucher_claim(&mut c, BOB, 0, &[1], 1);
    assert!(c.refused(stolen).contains("no such ticket"));
    let op = voucher_claim(&mut c, ALICE, 0, &[1], 1);
    assert!(matches!(c.ok(op), Receipt::ItemsClaimed { .. }));
    // The secret is public now; the burn's other ticket is still only Alice's.
    let stolen = voucher_claim(&mut c, BOB, 1, &[2, 3], 2);
    assert!(c.refused(stolen).contains("no such ticket"));
    let op = voucher_claim(&mut c, ALICE, 1, &[2, 3], 2);
    c.ok(op);
    assert_eq!(c.balance(ALICE, item_asset(&CAVE, 3)), ITEM_UNIT);
    let again = voucher_claim(&mut c, ALICE, 1, &[2, 3], 2);
    assert!(c.refused(again).contains("already claimed"));
    // Paid tickets claim as before, beside them.
    let op = c.claim_op(BOB, b_nonce, 0, &[0], merkle_proof(&leaves, 0));
    c.ok(op);
    assert_eq!(c.vm.market.sales[&CAVE].phase, Phase::Drawn);
}

/// A sale that closed with no tickets drew nothing and owes nobody, so it may
/// be set again; once a ticket exists, it may not.
#[test]
fn an_empty_closed_sale_can_be_set_again_and_a_sold_one_cannot() {
    let mut c = Chain::new();
    c.mint(ALICE, 1);
    c.ok(Op::RegisterCollection { program: program::register_collection(), asset: CAVE, creator: TREASURY, quote: ZEC, fee_bps: 100, cap: 12 });
    let set = || Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: TICKET, claim_price: CLAIM, open_price: OPEN, treasury_bps: 1_000 };
    c.ok(set());
    c.ok(Op::CloseSale { program: program::sale_close(), collection: CAVE, zcash_height: 10, zcash_hash: [7; 32] });
    assert_eq!(c.vm.market.sales[&CAVE].phase, Phase::Closed);
    c.ok(set());
    assert_eq!(c.vm.market.sales[&CAVE].phase, Phase::Open, "set again");
    assert_eq!(c.vm.market.sales[&CAVE].close_zcash_height, 0);
    let (_, op) = c.buy(ALICE, 1);
    c.ok(op);
    assert!(c.refused(set()).contains("prices are fixed"));
    c.ok(Op::CloseSale { program: program::sale_close(), collection: CAVE, zcash_height: 11, zcash_hash: [8; 32] });
    assert!(c.refused(set()).contains("prices are fixed"), "not after a close with tickets in it");
}


/// A gated sale (§168): only listed accounts buy or claim burn tickets, and
/// each pays the ticket price for its first `base` tickets, then one more
/// ticket price per ticket — the 11th at 0.02, the 12th at 0.03.
#[test]
fn a_gated_sale_sells_only_to_its_list_at_rising_prices() {
    use objectvm::sale::{allow_key, gated_price, gated_total, voucher_commitment};
    let mut c = Chain::new();
    for who in [ALICE, BOB, CAROL] {
        c.mint(who, 5);
    }
    c.ok(Op::RegisterCollection { program: program::register_collection(), asset: CAVE, creator: TREASURY, quote: ZEC, fee_bps: 100, cap: 40 });
    c.ok(Op::SetSale { program: program::sale_open(), collection: CAVE, ticket_price: TICKET, claim_price: CLAIM, open_price: OPEN, treasury_bps: 1_000 });
    let state = c.vm.state_root();
    assert!(!c.vm.encode().windows(8).any(|w| w == b"MARKET13"));

    // Operator-only, and only on the program that opens sales.
    let gate = Op::GateSale { program: program::sale_open(), collection: CAVE, base: 10 };
    assert_eq!(ObjectVm::intent_authorities(&gate), Vec::<AccountId>::new());
    assert!(c.refused(Op::GateSale { program: program::sale_tickets(), collection: CAVE, base: 10 }).contains("wrong program"));
    c.ok(gate);
    assert_ne!(c.vm.state_root(), state, "the gate is in the root");
    assert!(c.vm.encode().windows(8).any(|w| w == b"MARKET13"));
    let allow = |keys: Vec<Hash>| Op::AllowBuyers { program: program::sale_open(), collection: CAVE, keys };
    assert_eq!(c.ok(allow(vec![allow_key(&CAVE, &ALICE), allow_key(&CAVE, &BOB)])), Receipt::BuyersAllowed { collection: CAVE, added: 2 });
    assert_eq!(c.ok(allow(vec![allow_key(&CAVE, &ALICE)])), Receipt::BuyersAllowed { collection: CAVE, added: 0 }, "add-only");

    // Carol is not on the list.
    let nonce = c.fresh();
    let (inputs, change) = c.fund(CAROL, TICKET);
    let why = c.refused(Op::BuyTickets { buyer: CAROL, program: program::sale_tickets(), collection: CAVE, count: 1, nonce, inputs, change });
    assert!(why.contains("only accounts on the sale's list"), "{why}");

    // Alice: 10 at 0.01, then 0.02, 0.03 …
    assert_eq!((gated_price(TICKET, 10, 10), gated_price(TICKET, 10, 11), gated_price(TICKET, 10, 12)), (Some(TICKET.0), Some(2 * TICKET.0), Some(3 * TICKET.0)));
    let buy = |c: &mut Chain, who: AccountId, count: u32, pay: i128| {
        let nonce = c.fresh();
        let (inputs, change) = c.fund(who, Fixed(pay));
        c.apply(Op::BuyTickets { buyer: who, program: program::sale_tickets(), collection: CAVE, count, nonce, inputs, change })
    };
    let floor = c.floor();
    assert!(matches!(buy(&mut c, ALICE, 10, 10 * TICKET.0), Receipt::TicketsBought { count: 10, .. }));
    assert_eq!(c.floor().0 - floor.0, floor_share(Fixed(10 * TICKET.0)));
    // Paying for two more at 0.01 each is not enough: they cost 0.02 + 0.03.
    assert!(matches!(buy(&mut c, ALICE, 2, 2 * TICKET.0), Receipt::Refused(_)));
    assert_eq!(gated_total(TICKET, 10, 10, 2), Some(5 * TICKET.0));
    let floor = c.floor();
    assert!(matches!(buy(&mut c, ALICE, 2, 5 * TICKET.0), Receipt::TicketsBought { count: 2, .. }));
    assert_eq!(c.floor().0 - floor.0, floor_share(Fixed(5 * TICKET.0)));
    assert_eq!(c.vm.market.sale_gates[&CAVE].allowed[&allow_key(&CAVE, &ALICE)], 12);
    // Bob's count is his own.
    assert!(matches!(buy(&mut c, BOB, 1, TICKET.0), Receipt::TicketsBought { count: 1, .. }));

    // The gate is set before the first ticket, never after.
    assert!(c.refused(Op::GateSale { program: program::sale_open(), collection: CAVE, base: 20 }).contains("before its first ticket"));

    // Burn tickets: posted for anyone's commitment, claimed only from a listed account.
    c.ok(Op::SetVoucherCap { program: program::sale_vouchers(), collection: CAVE, cap: 4 });
    let (sb, sc) = ([0x5b; 32], [0x5c; 32]);
    c.ok(Op::PostVoucher { program: program::sale_vouchers(), collection: CAVE, burn: [1; 32], commitment: voucher_commitment(&BOB, &sb), count: 1 });
    c.ok(Op::PostVoucher { program: program::sale_vouchers(), collection: CAVE, burn: [2; 32], commitment: voucher_commitment(&CAROL, &sc), count: 1 });
    c.ok(Op::CloseSale { program: program::sale_close(), collection: CAVE, zcash_height: 10, zcash_hash: [7; 32] });
    let wins: Vec<(Hash, Vec<u32>)> = vec![
        (objectvm::sale::voucher_ticket_id(&voucher_commitment(&BOB, &sb), 0), vec![30]),
        (objectvm::sale::voucher_ticket_id(&voucher_commitment(&CAROL, &sc), 0), vec![31]),
    ];
    let leaves: Vec<Hash> = wins.iter().map(|(t, s)| draw_leaf(t, s)).collect();
    c.ok(Op::PostDraw { program: program::sale_draw(), collection: CAVE, root: merkle_root(&leaves) });
    let claim = |c: &mut Chain, who: AccountId, secret: Hash, serial: u32, i: usize| {
        let items = c.items(who, &[serial]);
        let (inputs, change) = c.fund(who, CLAIM);
        c.apply(Op::ClaimVoucherItems { buyer: who, program: program::sale_claim(), collection: CAVE, secret, k: 0, serials: vec![serial], proof: merkle_proof(&leaves, i), items, inputs, change })
    };
    assert!(matches!(claim(&mut c, BOB, sb, 30, 0), Receipt::ItemsClaimed { .. }));
    match claim(&mut c, CAROL, sc, 31, 1) {
        Receipt::Refused(why) => assert!(why.contains("only accounts on the sale's list"), "{why}"),
        r => panic!("{r:?}"),
    }
    // Added to the list later, Carol claims.
    c.ok(allow(vec![allow_key(&CAVE, &CAROL)]));
    assert!(matches!(claim(&mut c, CAROL, sc, 31, 1), Receipt::ItemsClaimed { .. }));
}
