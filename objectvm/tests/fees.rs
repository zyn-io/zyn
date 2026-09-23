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

fn with_collection(c: &mut Chain, fee_bps: u32, items: u8) -> Vec<Hash> {
    c.ok(Op::RegisterCollection {
        program: program::register_collection(),
        asset: CAVE,
        creator: TREASURY,
        quote: ZEC,
        fee_bps,
    });
    let mut out = Vec::new();
    for i in 0..items {
        let note = Note { asset: CAVE, amount: Fixed(1), owner: ALICE, nonce: [i + 1; 32] };
        c.ok(Op::Mint { asset: CAVE, amount: Fixed(1), owner: ALICE, nonce: note.nonce });
        out.push(commitment(&note).unwrap());
    }
    out
}

/// §69.9 on the object ledger: 250 bps a side on a 10 ZEC sale. The buyer
/// pays 10.25, the seller gets 9.75, and the 0.5 taken splits 0.25 to the
/// ZYN pot, 0.2 to the floor and 0.05 to the creator as a claim.
#[test]
fn a_resale_pays_the_zyn_pot_the_floor_and_the_creator() {
    let mut c = Chain::new();
    c.deposit(BOB, 20);
    let items = with_collection(&mut c, 250, 3);
    let item = items[0];
    c.ok(Op::PlaceOffer {
        seller: ALICE,
        program: program::place_offer(),
        item,
        price_asset: ZEC,
        price_amount: Fixed::whole(10),
        expires_at_epoch: 100,
    });
    let (bob_c, _) = c.held(BOB, ZEC)[0];
    let outputs = vec![
        c.out(ALICE, ZEC, Fixed(9_750_000_000_000_000_000)),
        c.out(BOB, CAVE, Fixed(1)),
        c.out(BOB, ZEC, Fixed(20_000_000_000_000_000_000 - 10_250_000_000_000_000_000)),
    ];
    c.ok(Op::TakeOffer {
        buyer: BOB,
        program: program::take_offer(),
        item,
        item_nullifier: nullifier(&[1; 32], &item),
        payment: vec![(bob_c, nullifier(&[2; 32], &bob_c))],
        outputs,
    });
    assert_eq!(c.pot(), Fixed(250_000_000_000_000_000));
    assert_eq!(c.vm.market.collections[&CAVE].floor, Fixed(200_000_000_000_000_000));
    let claim = c.vm.market.claims[&market::claim_id(&market::sale_fee_source(&item), 0)];
    assert_eq!((claim.owner, claim.amount), (TREASURY, Fixed(50_000_000_000_000_000)));
}

/// A seller short-changed by more than the fee, and a listing in another
/// asset, are both refused: neither can be a way around the fee.
#[test]
fn the_fee_cannot_be_dodged() {
    let mut c = Chain::new();
    c.deposit(BOB, 20);
    let items = with_collection(&mut c, 250, 1);
    let r = c.apply(Op::PlaceOffer {
        seller: ALICE,
        program: program::place_offer(),
        item: items[0],
        price_asset: [0x99; 32],
        price_amount: Fixed::whole(10),
        expires_at_epoch: 100,
    });
    assert_eq!(r, Receipt::Refused("this collection trades in its quote asset"));

    c.ok(Op::PlaceOffer {
        seller: ALICE,
        program: program::place_offer(),
        item: items[0],
        price_asset: ZEC,
        price_amount: Fixed::whole(10),
        expires_at_epoch: 100,
    });
    let (bob_c, _) = c.held(BOB, ZEC)[0];
    // Buyer pays the bare price and keeps the fee: not conserved.
    let outputs = vec![
        c.out(ALICE, ZEC, Fixed(9_750_000_000_000_000_000)),
        c.out(BOB, CAVE, Fixed(1)),
        c.out(BOB, ZEC, Fixed(10_250_000_000_000_000_000)),
    ];
    let r = c.apply(Op::TakeOffer {
        buyer: BOB,
        program: program::take_offer(),
        item: items[0],
        item_nullifier: nullifier(&[1; 32], &items[0]),
        payment: vec![(bob_c, nullifier(&[2; 32], &bob_c))],
        outputs,
    });
    assert!(ObjectVm::rejected(&r), "{r:?}");
}

/// The floor is only a floor if holders can take it: burning one of three
/// items pays a third of it.
#[test]
fn a_holder_redeems_items_for_their_share_of_the_floor() {
    let mut c = Chain::new();
    let items = with_collection(&mut c, 250, 3);
    // Seed the floor through the one path that funds it: a sale.
    c.deposit(BOB, 100);
    c.ok(Op::PlaceOffer {
        seller: ALICE,
        program: program::place_offer(),
        item: items[2],
        price_asset: ZEC,
        price_amount: Fixed::whole(60),
        expires_at_epoch: 100,
    });
    let (bob_c, _) = c.held(BOB, ZEC)[0];
    let outputs = vec![
        // 60 at 250 bps a side: the seller gets 58.5, the buyer pays 61.5.
        c.out(ALICE, ZEC, Fixed(58_500_000_000_000_000_000)),
        c.out(BOB, CAVE, Fixed(1)),
        c.out(BOB, ZEC, Fixed(38_500_000_000_000_000_000)),
    ];
    c.ok(Op::TakeOffer {
        buyer: BOB,
        program: program::take_offer(),
        item: items[2],
        item_nullifier: nullifier(&[1; 32], &items[2]),
        payment: vec![(bob_c, nullifier(&[2; 32], &bob_c))],
        outputs,
    });
    let floor = c.vm.market.collections[&CAVE].floor;
    assert_eq!(floor, Fixed(1_200_000_000_000_000_000), "40% of 3.0 taken");

    let r = c.ok(Op::Redeem {
        holder: ALICE,
        program: program::redeem_item(),
        asset: CAVE,
        inputs: vec![(items[0], nullifier(&[1; 32], &items[0]))],
    });
    let Receipt::Redeemed { paid, source } = r else { panic!() };
    assert_eq!(paid, Fixed(400_000_000_000_000_000), "a third of the floor");
    assert_eq!(c.vm.issued[&CAVE], Fixed(2));
    assert_eq!(c.vm.market.claims[&market::claim_id(&source, 0)].owner, ALICE);
    let CollectionTerms { floor, .. } = c.vm.market.collections[&CAVE];
    assert_eq!(floor, Fixed(800_000_000_000_000_000));
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
    });
    assert_eq!(r, Receipt::Refused("that collection already has its terms"), "terms do not move under holders");
}
