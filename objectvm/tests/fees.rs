//! The bridge fee and the NFT resale fee, feeding the ZYN pot.
//!
//! Every intent goes through `apply`, and after each one the books must still
//! balance: live objects plus everything the market holds — the ZYN pot, the
//! collections' floors, unredeemed claims — equal issued supply. A fee that
//! moved value correctly and leaked it on the next intent would pass a test
//! that only checked outcomes.

use objectvm::market::{self, CollectionTerms};
use objectvm::{ingress, Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{commitment, nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x0e; 32];
const CAVE: [u8; 32] = [0xCA; 32];
const ALICE: AccountId = [0xA1; 32];
const BOB: AccountId = [0xB0; 32];
const TREASURY: AccountId = [0x7E; 32];

struct Chain {
    vm: ObjectVm,
    seq: u64,
    n: u64,
}

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

impl Chain {
    fn new() -> Chain {
        let mut c = Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, n: 0 };
        for who in [ALICE, BOB] {
            c.ok(Op::Register {
                account: who,
                program: program::register_viewing_key(),
                viewing: viewing_public(&StaticSecret::from([who[0]; 32])),
            });
        }
        c
    }

    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let r = self.vm.apply(self.seq, &op);
        assert_eq!(self.vm.conserved(), Ok(()), "after {op:?}");
        assert_eq!(self.vm.reconciled(), Ok(()));
        assert_eq!(ObjectVm::decode(&self.vm.encode()).as_ref(), Some(&self.vm), "state round-trips");
        let wire = ObjectVm::encode_intent(&op);
        assert_eq!(ObjectVm::decode_intent(&mut zyn_vm::read::Decoder::new(&wire)).as_ref(), Some(&op));
        r.into_iter().next().unwrap()
    }

    fn ok(&mut self, op: Op) -> Receipt {
        let r = self.apply(op);
        assert!(!ObjectVm::rejected(&r), "{r:?}");
        r
    }

    fn nonce(&mut self) -> Hash {
        self.n += 1;
        let mut h = [0x5a; 32];
        h[..8].copy_from_slice(&self.n.to_be_bytes());
        h
    }

    fn out(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> Output {
        Output { note: Note { asset, amount, owner, nonce: self.nonce() }, enc: enc() }
    }

    /// A real bridge credit, built the way the node builds one.
    fn deposit(&mut self, who: AccountId, whole: i64) -> Receipt {
        let txid = self.nonce();
        let nonce = self.nonce();
        let op = ingress::credit(&self.vm, ZEC, Fixed::whole(whole), who, txid, nonce, StaticSecret::from([0x33; 32])).unwrap();
        self.ok(op)
    }

    fn held(&self, who: AccountId, asset: [u8; 32]) -> Vec<(Hash, Note)> {
        self.vm.live.iter().filter(|(_, n)| n.owner == who && n.asset == asset).map(|(c, n)| (*c, *n)).collect()
    }

    fn balance(&self, who: AccountId, asset: [u8; 32]) -> Fixed {
        Fixed(self.held(who, asset).iter().map(|(_, n)| n.amount.0).sum())
    }

    fn pot(&self) -> Fixed {
        self.vm.market.pol.get(&ZEC).copied().unwrap_or(Fixed::ZERO)
    }

    fn set_fees(&mut self, bps: u32) {
        self.ok(Op::SetFees { program: program::configure_fees(), bridge_bps: bps });
    }
}

/// Nothing is charged until the operator says so, and a chain that never
/// says so keeps the roots it already had.
#[test]
fn without_a_schedule_nothing_is_charged_and_roots_do_not_move() {
    let mut c = Chain::new();
    c.deposit(ALICE, 10);
    assert_eq!(c.balance(ALICE, ZEC), Fixed::whole(10));
    assert!(c.vm.market.is_empty());
    assert_eq!(c.vm.sections().len(), 7);
}

/// 50 bps on a 10 ZEC deposit: 9.95 to the depositor, 0.05 to the pot, and
/// all 10 counted as issued because the vault holds all 10.
#[test]
fn a_deposit_pays_the_bridge_fee_into_the_zyn_pot() {
    let mut c = Chain::new();
    c.set_fees(50);
    c.deposit(ALICE, 10);
    assert_eq!(c.balance(ALICE, ZEC), Fixed(9_950_000_000_000_000_000));
    assert_eq!(c.pot(), Fixed(50_000_000_000_000_000));
    assert_eq!(c.vm.issued[&ZEC], Fixed::whole(10));
}

/// Once the bridge charges, a credit that skips the fee is refused — the
/// operator cannot quietly credit a friend in full.
#[test]
fn a_fee_free_credit_is_refused_once_the_bridge_charges() {
    let mut c = Chain::new();
    c.set_fees(50);
    let r = c.apply(Op::Deposit {
        program: program::deposit(),
        asset: ZEC,
        amount: Fixed::whole(10),
        owner: ALICE,
        nonce: [9; 32],
        enc: enc(),
        external_ref: [8; 32],
    });
    assert!(ObjectVm::rejected(&r));
    let r = c.apply(Op::DepositWithFee {
        program: program::deposit(),
        asset: ZEC,
        gross: Fixed::whole(10),
        amount: Fixed::whole(10),
        owner: ALICE,
        nonce: [9; 32],
        enc: enc(),
        external_ref: [8; 32],
    });
    assert_eq!(r, Receipt::Refused("the credit is not the deposit less the bridge fee"));
}

/// A withdrawal pays the fee on top: 5 leaves for Zcash, 0.025 stays in the
/// pot, and issued supply falls by exactly what the vault will pay out.
#[test]
fn a_withdrawal_pays_the_bridge_fee_on_top() {
    let mut c = Chain::new();
    c.deposit(ALICE, 10);
    c.set_fees(50);
    let (note_c, _) = c.held(ALICE, ZEC)[0];
    let change = c.out(ALICE, ZEC, Fixed(10_000_000_000_000_000_000 - 5_025_000_000_000_000_000));
    c.ok(Op::Withdraw {
        holder: ALICE,
        program: program::withdraw(),
        inputs: vec![(note_c, nullifier(&[1; 32], &note_c))],
        asset: ZEC,
        amount: Fixed::whole(5),
        destination: [0xDE; 32],
        outputs: vec![change],
    });
    assert_eq!(c.pot(), Fixed(25_000_000_000_000_000));
    assert_eq!(c.vm.exits[0].amount, Fixed::whole(5));
    assert_eq!(c.vm.issued[&ZEC], Fixed::whole(5));
}

/// Register Cave (cap 3) and mint `items` to Alice the way the primary sale
/// delivers them: sealed to her registered viewing key.
fn with_collection(c: &mut Chain, fee_bps: u32, items: u32) -> Vec<Hash> {
    c.ok(Op::RegisterCollection {
        program: program::register_collection(),
        asset: CAVE,
        creator: TREASURY,
        quote: ZEC,
        fee_bps,
        cap: 3,
    });
    (0..items).map(|serial| mint(c, serial, ALICE)).collect()
}

/// Mint, and prove the owner's wallet finds it: a scan with their viewing key
/// over what the chain publishes opens the note to the right item.
fn mint(c: &mut Chain, serial: u32, to: AccountId) -> Hash {
    let nonce = c.nonce();
    let op = objectvm::primary::mint_item(&c.vm, CAVE, serial, to, nonce, StaticSecret::from([0x44; 32])).unwrap();
    let Op::MintItem { enc, .. } = op.clone() else { unreachable!() };
    c.ok(op);
    let asset = market::item_asset(&CAVE, serial);
    let cm = commitment(&Note { asset, amount: market::ITEM_UNIT, owner: to, nonce }).unwrap();
    let found = zyn_vm::scan::Scanner::new(StaticSecret::from([to[0]; 32]), [0x31; 32]).scan(
        &to,
        &[zyn_vm::scan::PublishedOutput { commitment: cm, enc }],
        0,
        &[],
    );
    assert_eq!(found.live.len(), 1, "the owner's wallet must find the item");
    assert_eq!(found.live[0].note.asset, asset);
    cm
}

/// List at `whole` ZEC with the item's nullifier published, paid by claim.
fn list(c: &mut Chain, item: Hash, whole: i64, nonce: Hash) {
    c.ok(Op::List {
        seller: ALICE,
        program: program::place_offer(),
        item,
        item_nullifier: nullifier(&[1; 32], &item),
        price_asset: ZEC,
        price_amount: Fixed::whole(whole),
        expires_at_epoch: 100,
        nonce,
    });
}

/// The buyer's side: the item to themselves and their change, nothing that
/// names the seller.
fn buy(c: &mut Chain, item: Hash, pay_total: Fixed) -> Receipt {
    let asset = c.vm.live[&item].asset;
    let (bob_c, bob_note) = c.held(BOB, ZEC)[0];
    let outputs = vec![
        c.out(BOB, asset, market::ITEM_UNIT),
        c.out(BOB, ZEC, Fixed(bob_note.amount.0 - pay_total.0)),
    ];
    let listing = c.vm.market.listings[&item];
    c.apply(Op::TakeOffer {
        buyer: BOB,
        program: program::take_offer(),
        item,
        item_nullifier: listing.nullifier,
        payment: vec![(bob_c, nullifier(&[2; 32], &bob_c))],
        outputs,
    })
}

/// The whole cycle. Minted into Alice's wallet (her scan finds it), listed,
/// bought by Bob at 10 + 0.1 fee, Alice's 9.9 waiting as a claim only her
/// nonce finds, fees split 0.1 ZYN pot / 0.08 floor / 0.02 creator, and the
/// item Bob holds is the *same* item — serial 0 — not merely one like it.
#[test]
fn an_item_is_minted_listed_bought_and_paid_for() {
    let mut c = Chain::new();
    c.deposit(BOB, 20);
    let items = with_collection(&mut c, 100, 2);
    let item = items[0];

    let nonce = [0x77; 32];
    list(&mut c, item, 10, nonce);
    assert!(c.vm.market.listings.contains_key(&item));

    let r = buy(&mut c, item, Fixed(10_100_000_000_000_000_000));
    assert!(matches!(r, Receipt::Taken { .. }), "{r:?}");
    assert_eq!(c.held(BOB, market::item_asset(&CAVE, 0)).len(), 1, "Bob holds serial 0 itself");
    let proceeds = c.vm.market.claims[&market::claim_id(&market::listing_source(&ALICE, &nonce), 0)];
    assert_eq!((proceeds.owner, proceeds.amount), (ALICE, Fixed(9_900_000_000_000_000_000)));
    assert_eq!(c.pot(), Fixed(100_000_000_000_000_000));
    assert_eq!(c.vm.market.collections[&CAVE].floor, Fixed(80_000_000_000_000_000));
    let creator = c.vm.market.claims[&market::claim_id(&market::sale_fee_source(&item), 0)];
    assert_eq!((creator.owner, creator.amount), (TREASURY, Fixed(20_000_000_000_000_000)));
    assert!(!c.vm.market.listings.contains_key(&item));

    // Alice collects her proceeds.
    let out = c.out(ALICE, ZEC, Fixed(9_900_000_000_000_000_000));
    c.ok(Op::Claim {
        owner: ALICE,
        program: program::claim(),
        claims: vec![market::claim_id(&market::listing_source(&ALICE, &nonce), 0)],
        outputs: vec![out],
    });
    assert_eq!(c.balance(ALICE, ZEC), Fixed(9_900_000_000_000_000_000));
}

/// Short-paying, keeping the item's fee, a wrong nullifier, or listing in
/// another asset: every way around the fee or the seller is refused.
#[test]
fn a_sale_cannot_be_dodged() {
    let mut c = Chain::new();
    c.deposit(BOB, 20);
    let items = with_collection(&mut c, 100, 1);
    let item = items[0];
    let r = c.apply(Op::List {
        seller: ALICE,
        program: program::place_offer(),
        item,
        item_nullifier: nullifier(&[1; 32], &item),
        price_asset: [0x99; 32],
        price_amount: Fixed::whole(10),
        expires_at_epoch: 100,
        nonce: [1; 32],
    });
    assert_eq!(r, Receipt::Refused("this collection trades in its quote asset"));
    let r = c.apply(Op::PlaceOffer {
        seller: ALICE,
        program: program::place_offer(),
        item,
        price_asset: ZEC,
        price_amount: Fixed::whole(10),
        expires_at_epoch: 100,
    });
    assert_eq!(r, Receipt::Refused("list a collection's items with List"));

    list(&mut c, item, 10, [2; 32]);
    let r = buy(&mut c, item, Fixed::whole(10));
    assert!(ObjectVm::rejected(&r), "paying the bare price skips the fee: {r:?}");
}

/// Cancel lifts the listing and its claim path; the item is Alice's to send,
/// and a send keeps its identity.
#[test]
fn a_listing_is_cancelled_and_the_item_sent_whole() {
    let mut c = Chain::new();
    let items = with_collection(&mut c, 100, 1);
    let item = items[0];
    list(&mut c, item, 10, [3; 32]);
    c.ok(Op::CancelOffer { seller: ALICE, program: program::cancel_offer(), item });
    assert!(c.vm.market.listings.is_empty() && c.vm.offers.is_empty());
    let asset = market::item_asset(&CAVE, 0);
    let to_bob = c.out(BOB, asset, market::ITEM_UNIT);
    c.ok(Op::Spend {
        spender: ALICE,
        program: program::private_transfer(),
        inputs: vec![(item, nullifier(&[1; 32], &item))],
        outputs: vec![to_bob],
    });
    assert_eq!(c.held(BOB, asset).len(), 1);
    // A send cannot turn serial 0 into serial 1: they are different assets.
    let bob_item = c.held(BOB, asset)[0].0;
    let fake = c.out(ALICE, market::item_asset(&CAVE, 1), market::ITEM_UNIT);
    let r = c.apply(Op::Spend {
        spender: BOB,
        program: program::private_transfer(),
        inputs: vec![(bob_item, nullifier(&[2; 32], &bob_item))],
        outputs: vec![fake],
    });
    assert!(ObjectVm::rejected(&r));
}

/// The floor is only a floor if holders can take it: after a 60 ZEC sale
/// (1.2 taken, 0.48 to the floor) Alice burns one of two outstanding items for
/// half the floor.
#[test]
fn a_holder_redeems_items_for_their_share_of_the_floor() {
    let mut c = Chain::new();
    c.deposit(BOB, 100);
    let items = with_collection(&mut c, 100, 2);
    list(&mut c, items[1], 60, [4; 32]);
    let r = buy(&mut c, items[1], Fixed(60_600_000_000_000_000_000));
    assert!(!ObjectVm::rejected(&r), "{r:?}");
    let floor = c.vm.market.collections[&CAVE].floor;
    assert_eq!(floor, Fixed(480_000_000_000_000_000));

    let r = c.ok(Op::Redeem {
        holder: ALICE,
        program: program::redeem_item(),
        asset: CAVE,
        inputs: vec![(items[0], nullifier(&[1; 32], &items[0]))],
    });
    let Receipt::Redeemed { paid, source } = r else { panic!() };
    assert_eq!(paid, Fixed(240_000_000_000_000_000), "half: one of two outstanding");
    assert_eq!(c.vm.market.claims[&market::claim_id(&source, 0)].owner, ALICE);
    let CollectionTerms { floor, outstanding, .. } = c.vm.market.collections[&CAVE];
    assert_eq!((floor, outstanding), (Fixed(240_000_000_000_000_000), 1));
}

/// What holders were promised about supply holds: no minting past the cap,
/// no serial twice, no item for someone the chain cannot deliver it to, and
/// no item leaving for Zcash.
#[test]
fn supply_is_what_holders_were_promised() {
    let mut c = Chain::new();
    with_collection(&mut c, 100, 3);
    let r = c.apply(Op::MintItem {
        program: program::mint_item(),
        collection: CAVE,
        serial: 3,
        owner: ALICE,
        nonce: [5; 32],
        enc: enc(),
    });
    assert_eq!(r, Receipt::Refused("that serial is beyond the collection's cap"));
    let r = c.apply(Op::MintItem {
        program: program::mint_item(),
        collection: CAVE,
        serial: 0,
        owner: ALICE,
        nonce: [6; 32],
        enc: enc(),
    });
    assert_eq!(r, Receipt::Refused("that item was already minted"));
    assert!(objectvm::primary::mint_item(&c.vm, CAVE, 2, [0x55; 32], [7; 32], StaticSecret::from([1; 32])).is_err());
    assert!(c.vm.market.is_derived(&market::item_asset(&CAVE, 0)), "items cannot be withdrawn to Zcash");
}

/// Fees and collections are the operator's to set: neither names an account
/// that could sign for it, so the holders' door can never apply one.
#[test]
fn only_the_operator_sets_fees_or_terms() {
    assert!(ObjectVm::intent_authorities(&Op::SetFees { program: program::configure_fees(), bridge_bps: 50 }).is_empty());
    assert!(ObjectVm::intent_authorities(&Op::RegisterCollection {
        program: program::register_collection(),
        asset: CAVE,
        creator: TREASURY,
        quote: ZEC,
        fee_bps: 250,
        cap: 1,
    })
    .is_empty());
    let mut c = Chain::new();
    let r = c.apply(Op::SetFees { program: program::configure_fees(), bridge_bps: 201 });
    assert_eq!(r, Receipt::Refused("the bridge fee is above the ceiling"));
    with_collection(&mut c, 250, 1);
    let r = c.apply(Op::RegisterCollection {
        program: program::register_collection(),
        asset: CAVE,
        creator: BOB,
        quote: ZEC,
        fee_bps: 0,
        cap: 10,
    });
    assert_eq!(r, Receipt::Refused("that collection already has its terms"), "terms do not move under holders");
}
