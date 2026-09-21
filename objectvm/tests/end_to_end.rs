//! An object transaction, end to end: minted, spent, conserved, sealed — and
//! private in the sense tier 2 actually claims.
//!
//! The claim under test is narrow and worth stating exactly. A holder of the
//! **published sections** learns how many objects exist and how many have been
//! spent. They do not learn any amount, any owner, or which commitment a
//! nullifier retired. They do not learn it because the amounts are not in the
//! sections at all — not because they are hashed somewhere a determined reader
//! could grind.

use objectvm::{Limits, ObjectVm, Op, Receipt};
use zyn_vm::conformance::{check, Fixture};
use zyn_vm::object::{commitment, nullifier, Note};
use zyn_vm::program;
use zyn_vm::spec::{AccountId, MicrochainVm, SECTION_ACCOUNTS};
use zyn_vm::Fixed;

const ZEC: [u8; 32] = [0x7a; 32];
const ALICE: AccountId = [0xAA; 32];
const BOB: AccountId = [0xBB; 32];
const ALICE_KEY: [u8; 32] = [0xA1; 32];

fn note(amount: i64, owner: AccountId, nonce: u8) -> Note {
    Note {
        asset: ZEC,
        amount: Fixed::whole(amount),
        owner,
        nonce: [nonce; 32],
    }
}

fn mint(amount: i64, owner: AccountId, nonce: u8) -> Op {
    Op::Mint {
        asset: ZEC,
        amount: Fixed::whole(amount),
        owner,
        nonce: [nonce; 32],
    }
}

/// A chain with one 17 ZEC object belonging to Alice.
fn funded() -> (ObjectVm, [u8; 32]) {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let r = vm.apply(1, &mint(17, ALICE, 1));
    let c = match &r[0] {
        Receipt::Minted { commitment, .. } => *commitment,
        other => panic!("mint refused: {other:?}"),
    };
    (vm, c)
}

/// The whole loop: an object exists, is consumed, and its successors exist —
/// with value conserved across the step and the spend retired by a nullifier.
#[test]
fn an_object_is_minted_spent_and_conserved() {
    let (mut vm, input) = funded();
    assert_eq!(vm.conserved(), Ok(()), "a mint must leave the chain conserved");
    assert_eq!(vm.commitments.len(), 1);
    assert_eq!(vm.nullifiers.len(), 0);

    // Alice sends 5 to Bob and keeps 12.
    let to_bob = note(5, BOB, 2);
    let change = note(12, ALICE, 3);
    let n = nullifier(&ALICE_KEY, &input);

    let receipts = vm.apply(
        2,
        &Op::Spend {
            spender: ALICE,
            program: program::private_transfer(),
            inputs: vec![(input, n)],
            outputs: vec![to_bob, change],
        },
    );

    match &receipts[0] {
        Receipt::Spent { spent, created } => {
            assert_eq!(spent, &vec![n]);
            assert_eq!(created.len(), 2);
        }
        other => panic!("spend refused: {other:?}"),
    }

    assert_eq!(vm.conserved(), Ok(()), "value must survive the spend");
    assert_eq!(vm.commitments.len(), 3, "one minted, two created");
    assert_eq!(vm.nullifiers.len(), 1, "one retired");
    assert!(!vm.live.contains_key(&input), "the input is gone");
    assert!(vm.live.contains_key(&commitment(&to_bob).unwrap()));
    assert!(vm.live.contains_key(&commitment(&change).unwrap()));
}

/// The only thing a nullifier set is for.
#[test]
fn the_same_object_cannot_be_spent_twice() {
    let (mut vm, input) = funded();
    let n = nullifier(&ALICE_KEY, &input);
    let spend = Op::Spend {
        spender: ALICE,
        program: program::private_transfer(),
        inputs: vec![(input, n)],
        outputs: vec![note(17, BOB, 9)],
    };
    assert!(matches!(vm.apply(2, &spend)[0], Receipt::Spent { .. }));

    // Replayed verbatim: the object is no longer live.
    assert!(matches!(vm.apply(3, &spend)[0], Receipt::Refused(_)));
    assert_eq!(vm.nullifiers.len(), 1, "a refused replay adds nothing");
    assert_eq!(vm.conserved(), Ok(()));
}

/// Value may not be created by asking for more out than went in. The batch
/// check would catch it too, but only by discarding every intent beside it.
#[test]
fn a_spend_cannot_mint_value() {
    let (mut vm, input) = funded();
    let before = vm.state_root();
    let r = vm.apply(
        2,
        &Op::Spend {
            spender: ALICE,
            program: program::private_transfer(),
            inputs: vec![(input, nullifier(&ALICE_KEY, &input))],
            outputs: vec![note(18, ALICE, 4)], // one more than existed
        },
    );
    assert_eq!(r[0], Receipt::Refused("value not conserved"));
    assert_eq!(vm.conserved(), Ok(()));
    assert_ne!(before, vm.state_root(), "the sequence still moved");
    assert_eq!(vm.commitments.len(), 1, "but no commitment was appended");
}

/// Conformance S3: a rejected intent must leave the committed leaves
/// byte-identical. This is the rule that forces commitments to be appended on
/// acceptance rather than on submission, and it is the easiest one to break.
#[test]
fn a_refused_spend_moves_no_committed_leaf() {
    let (mut vm, input) = funded();
    let commitments_before = vm.commitments.root();
    let nullifiers_before = vm.nullifiers.root();
    let accounts_before = vm.sections()[SECTION_ACCOUNTS];

    // Bob does not own Alice's object.
    let r = vm.apply(
        2,
        &Op::Spend {
            spender: BOB,
            program: program::private_transfer(),
            inputs: vec![(input, nullifier(&ALICE_KEY, &input))],
            outputs: vec![note(17, BOB, 5)],
        },
    );
    assert_eq!(r[0], Receipt::Refused("not the spender's object"));

    assert_eq!(vm.commitments.root(), commitments_before);
    assert_eq!(vm.nullifiers.root(), nullifiers_before);
    assert_eq!(vm.sections()[SECTION_ACCOUNTS], accounts_before);
}

/// What tier 2 actually claims, tested as a claim rather than asserted in a
/// comment: the published sections carry no amount and no owner.
///
/// Two chains identical except for who holds what and how much must produce
/// *different* commitment roots — the commitment binds them — while a reader
/// with only the sections has nothing to open. The inverse is the real test:
/// changing an amount changes only the commitment root, and never reveals the
/// amount itself, because the amount appears nowhere in `sections()`.
#[test]
fn the_published_sections_carry_no_amount_or_owner() {
    let (vm, _) = funded();

    // Every section, as bytes. If an amount or an owner were published, it
    // would have to be in here somewhere.
    let published: Vec<u8> = vm.sections().concat();

    // Alice's id and the raw amount must not appear in the clear.
    assert!(
        !published.windows(32).any(|w| w == ALICE),
        "an owner id must not be readable from the sections"
    );
    let amount_bytes = Fixed::whole(17).0.to_be_bytes();
    assert!(
        !published.windows(16).any(|w| w == amount_bytes),
        "an amount must not be readable from the sections"
    );

    // And the preimage is genuinely bound: a different amount is a different
    // root, so the commitment is doing its job rather than being decorative.
    let mut other = ObjectVm::genesis(212, Limits { max_io: 8 });
    other.apply(1, &mint(18, ALICE, 1));
    assert_ne!(vm.commitments.root(), other.commitments.root());
}

/// A nullifier published beside the commitments must not identify which one it
/// retired. Without the owner's key there is no path between them.
#[test]
fn a_published_nullifier_does_not_name_its_commitment() {
    let (mut vm, input) = funded();
    let n = nullifier(&ALICE_KEY, &input);
    vm.apply(
        2,
        &Op::Spend {
            spender: ALICE,
            program: program::private_transfer(),
            inputs: vec![(input, n)],
            outputs: vec![note(17, BOB, 6)],
        },
    );

    // Everything a public reader holds.
    let spent: Vec<[u8; 32]> = vm.nullifiers.spent().to_vec();
    let existing: Vec<[u8; 32]> = vm.commitments.leaves().to_vec();

    assert_eq!(spent.len(), 1);
    assert!(
        !existing.contains(&spent[0]),
        "a nullifier must not be one of the commitments restated"
    );
    // The only way to link them is the key, which is not published anywhere.
    assert_eq!(nullifier(&ALICE_KEY, &input), spent[0]);
    assert_ne!(nullifier(&[0x99; 32], &input), spent[0]);
}

/// The suite the specification exists to be checked against. If this passes,
/// the object ledger is a Zyn VM in the same sense swapvm is — exit proofs,
/// sealing, determinism, conservation and all.
#[test]
fn objectvm_satisfies_the_conformance_suite() {
    let (vm, input) = funded();

    let violations = check(Fixture {
        state: vm,
        accepted: Op::Spend {
            spender: ALICE,
            program: program::private_transfer(),
            inputs: vec![(input, nullifier(&ALICE_KEY, &input))],
            outputs: vec![note(5, BOB, 7), note(12, ALICE, 8)],
        },
        // Bob spending Alice's object: refused, and must move nothing.
        rejected: Op::Spend {
            spender: BOB,
            program: program::private_transfer(),
            inputs: vec![(input, nullifier(&ALICE_KEY, &input))],
            outputs: vec![note(17, BOB, 10)],
        },
        sequence: vec![
            mint(3, BOB, 11),
            mint(4, ALICE, 12),
            Op::Seal,
            mint(5, BOB, 13),
        ],
    });

    assert!(
        violations.is_empty(),
        "conformance violations: {violations:#?}"
    );
}

/// The field is only worth carrying if an unknown value is refused. A
/// transaction that otherwise verifies but names a rule nobody checked is
/// worse than one that fails — it is a spend that obeyed no law at all.
#[test]
fn a_transaction_naming_an_unknown_program_is_refused() {
    let (mut vm, input) = funded();
    let before = vm.commitments.root();

    let r = vm.apply(
        2,
        &Op::Spend {
            spender: ALICE,
            program: zyn_vm::program::ProgramId([0x5A; 32]),
            inputs: vec![(input, nullifier(&ALICE_KEY, &input))],
            outputs: vec![note(17, BOB, 20)],
        },
    );
    assert_eq!(r[0], Receipt::Refused("unknown program"));
    assert_eq!(vm.commitments.root(), before, "and nothing moved");
}

/// A *named* rule this VM does not implement is refused too. The distinction
/// matters: "I have never heard of this" and "I know what that is and do not
/// do it" are different failures, and only the second is a VM that could be
/// taught.
#[test]
fn a_named_rule_this_vm_does_not_implement_is_refused() {
    let (mut vm, input) = funded();
    let r = vm.apply(
        2,
        &Op::Spend {
            spender: ALICE,
            program: zyn_vm::program::amm_swap(),
            inputs: vec![(input, nullifier(&ALICE_KEY, &input))],
            outputs: vec![note(17, BOB, 21)],
        },
    );
    assert_eq!(
        r[0],
        Receipt::Refused("this VM implements only private transfer")
    );
}

/// The id is part of the wire format, not a local field. If it did not survive
/// encoding, a replica would replay the transaction under whatever rule it
/// assumed — which is the failure the field exists to prevent.
#[test]
fn the_program_id_survives_the_wire() {
    let spend = Op::Spend {
        spender: ALICE,
        program: program::private_transfer(),
        inputs: vec![([1u8; 32], [2u8; 32])],
        outputs: vec![note(5, BOB, 22)],
    };
    let bytes = ObjectVm::encode_intent(&spend);
    let mut d = zyn_vm::read::Decoder::new(&bytes);
    assert_eq!(ObjectVm::decode_intent(&mut d), Some(spend));

    // And a different rule is a different encoding, so one cannot be replayed
    // as the other.
    let swapish = Op::Spend {
        spender: ALICE,
        program: program::amm_swap(),
        inputs: vec![([1u8; 32], [2u8; 32])],
        outputs: vec![note(5, BOB, 22)],
    };
    assert_ne!(ObjectVm::encode_intent(&swapish), bytes);
}

// ---------------------------------------------------------------- listings

const CAVE: [u8; 32] = [0xC0; 32];
const SELLER_KEY: [u8; 32] = [0x51; 32];

fn item(owner: AccountId, nonce: u8) -> Note {
    Note {
        asset: CAVE,
        amount: Fixed::whole(1),
        owner,
        nonce: [nonce; 32],
    }
}

/// Alice owns item #1177 and 20 ZEC of spending money sits with Bob.
fn market() -> (ObjectVm, [u8; 32], [u8; 32]) {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let listed = match &vm.apply(
        1,
        &Op::Mint {
            asset: CAVE,
            amount: Fixed::whole(1),
            owner: ALICE,
            nonce: [0x71; 32],
        },
    )[0]
    {
        Receipt::Minted { commitment, .. } => *commitment,
        o => panic!("{o:?}"),
    };
    let funds = match &vm.apply(2, &mint(20, BOB, 0x72))[0] {
        Receipt::Minted { commitment, .. } => *commitment,
        o => panic!("{o:?}"),
    };
    (vm, listed, funds)
}

fn list_it(vm: &mut ObjectVm, seq: u64, listed: [u8; 32], price: i64) -> Vec<Receipt> {
    vm.apply(
        seq,
        &Op::PlaceOffer {
            seller: ALICE,
            program: program::place_offer(),
            item: listed,
            price_asset: ZEC,
            price_amount: Fixed::whole(price),
            expires_at_epoch: u64::MAX,
        },
    )
}

/// A listing does not move the object. That is the whole difference from
/// escrow: no custody, nothing to return on a cancel, and the seller can still
/// prove ownership while it rests.
#[test]
fn listing_encumbers_without_moving_anything() {
    let (mut vm, listed, _) = market();
    let owner_before = vm.live[&listed].owner;

    assert_eq!(list_it(&mut vm, 3, listed, 4)[0], Receipt::Listed { item: listed });

    assert_eq!(vm.live[&listed].owner, owner_before, "still the seller's");
    assert!(vm.live.contains_key(&listed), "still live, not escrowed");
    assert!(vm.offers.contains_key(&listed), "and now encumbered");
    assert_eq!(vm.conserved(), Ok(()));
}

/// The encumbrance: still yours, still unspendable, because otherwise a seller
/// could take the money and move the item out from under the buyer.
#[test]
fn a_listed_object_cannot_be_spent_out_from_under_the_buyer() {
    let (mut vm, listed, _) = market();
    list_it(&mut vm, 3, listed, 4);

    let r = vm.apply(
        4,
        &Op::Spend {
            spender: ALICE,
            program: program::private_transfer(),
            inputs: vec![(listed, nullifier(&SELLER_KEY, &listed))],
            outputs: vec![item(BOB, 0x73)],
        },
    );
    assert_eq!(r[0], Receipt::Refused("that object is listed"));
    assert!(vm.live.contains_key(&listed), "and it is still there");
}

/// The claim: one transaction, both sides. The buyer cannot pay without
/// receiving, and the seller cannot be paid without parting with the item.
#[test]
fn the_buyer_claims_and_both_sides_settle_at_once() {
    let (mut vm, listed, funds) = market();
    list_it(&mut vm, 3, listed, 4);

    let receipts = vm.apply(
        4,
        &Op::TakeOffer {
            buyer: BOB,
            program: program::take_offer(),
            item: listed,
            item_nullifier: nullifier(&SELLER_KEY, &listed),
            payment: vec![(funds, nullifier(&[0xB1; 32], &funds))],
            outputs: vec![
                note(4, ALICE, 0x74),  // the price, to the seller
                item(BOB, 0x75),       // the item, to the buyer
                note(16, BOB, 0x76),   // change
            ],
        },
    );
    assert_eq!(receipts[0], Receipt::Taken { item: listed, created: match &receipts[0] {
        Receipt::Taken { created, .. } => created.clone(),
        _ => unreachable!(),
    }});

    assert_eq!(vm.conserved(), Ok(()), "a sale conserves value");
    assert!(!vm.live.contains_key(&listed), "the listed object is consumed");
    assert!(!vm.offers.contains_key(&listed), "and the listing is gone");
    assert!(vm.live.contains_key(&commitment(&item(BOB, 0x75)).unwrap()));
    assert_eq!(vm.nullifiers.len(), 2, "item and payment both retired");
}

/// Paying less than the asking price is not a sale.
#[test]
fn a_claim_that_underpays_is_refused() {
    let (mut vm, listed, funds) = market();
    list_it(&mut vm, 3, listed, 4);

    let r = vm.apply(
        4,
        &Op::TakeOffer {
            buyer: BOB,
            program: program::take_offer(),
            item: listed,
            item_nullifier: nullifier(&SELLER_KEY, &listed),
            payment: vec![(funds, nullifier(&[0xB1; 32], &funds))],
            outputs: vec![
                note(3, ALICE, 0x77), // one short
                item(BOB, 0x78),
                note(17, BOB, 0x79),
            ],
        },
    );
    assert_eq!(r[0], Receipt::Refused("the seller was not paid the asking price"));
    assert!(vm.live.contains_key(&listed), "and nothing settled");
    assert!(vm.offers.contains_key(&listed));
}

/// Paying the price but routing the item elsewhere is not a sale either.
#[test]
fn a_claim_that_does_not_deliver_the_item_is_refused() {
    let (mut vm, listed, funds) = market();
    list_it(&mut vm, 3, listed, 4);

    let r = vm.apply(
        4,
        &Op::TakeOffer {
            buyer: BOB,
            program: program::take_offer(),
            item: listed,
            item_nullifier: nullifier(&SELLER_KEY, &listed),
            payment: vec![(funds, nullifier(&[0xB1; 32], &funds))],
            outputs: vec![
                note(4, ALICE, 0x7A),
                item(ALICE, 0x7B), // back to the seller, not the buyer
                note(16, BOB, 0x7C),
            ],
        },
    );
    assert_eq!(r[0], Receipt::Refused("the buyer does not receive the object"));
}

/// Cancelling transfers nothing back, because nothing was transferred.
#[test]
fn cancelling_a_listing_moves_no_value() {
    let (mut vm, listed, _) = market();
    list_it(&mut vm, 3, listed, 4);
    let root_before = vm.commitments.root();

    assert_eq!(
        vm.apply(
            4,
            &Op::CancelOffer {
                seller: ALICE,
                program: program::cancel_offer(),
                item: listed,
            }
        )[0],
        Receipt::Delisted { item: listed }
    );
    assert!(!vm.offers.contains_key(&listed));
    assert_eq!(vm.commitments.root(), root_before, "no object moved");
    assert_eq!(vm.live[&listed].owner, ALICE, "it was never anywhere else");

    // And now it is spendable again.
    assert!(matches!(
        vm.apply(
            5,
            &Op::Spend {
                spender: ALICE,
                program: program::private_transfer(),
                inputs: vec![(listed, nullifier(&SELLER_KEY, &listed))],
                outputs: vec![item(BOB, 0x7D)],
            }
        )[0],
        Receipt::Spent { .. }
    ));
}

/// Only the seller may delist. Otherwise anyone could strip a listing and buy
/// the item elsewhere, or simply grief the market.
#[test]
fn a_stranger_cannot_cancel_someone_elses_listing() {
    let (mut vm, listed, _) = market();
    list_it(&mut vm, 3, listed, 4);
    let r = vm.apply(
        4,
        &Op::CancelOffer {
            seller: BOB,
            program: program::cancel_offer(),
            item: listed,
        },
    );
    assert_eq!(r[0], Receipt::Refused("not the seller's listing"));
    assert!(vm.offers.contains_key(&listed));
}

// ---------------------------------------------------------------- deposits

use zyn_vm::note_enc::{open, seal, viewing_public, EncryptedNote, StaticSecret};

const TXID: [u8; 32] = [0xD1; 32];

/// What the sequencer attaches so a depositor can find what was made for them.
fn sealed_for(to: &StaticSecret, eph: u8, note: &Note) -> EncryptedNote {
    let mut plain = Vec::new();
    plain.extend_from_slice(&note.asset);
    plain.extend_from_slice(&note.amount.0.to_be_bytes());
    plain.extend_from_slice(&note.nonce);
    seal(&viewing_public(to), StaticSecret::from([eph; 32]), &plain).unwrap()
}

fn deposit_of(amount: i64, owner: AccountId, nonce: u8, to: &StaticSecret, txid: [u8; 32]) -> Op {
    let note = Note { asset: ZEC, amount: Fixed::whole(amount), owner, nonce: [nonce; 32] };
    Op::Deposit {
        program: program::deposit(),
        asset: ZEC,
        amount: Fixed::whole(amount),
        owner,
        nonce: [nonce; 32],
        enc: sealed_for(to, nonce, &note),
        external_ref: txid,
    }
}

/// The whole point of the deposit path: ZEC arrives, an object exists, and
/// the depositor can *find* it — which is the part that does not happen by
/// itself on an object ledger.
#[test]
fn a_deposit_creates_an_object_its_owner_can_find() {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let alice_view = StaticSecret::from([0x0A; 32]);

    let d = deposit_of(17, ALICE, 0x31, &alice_view, TXID);
    let enc = match &d {
        Op::Deposit { enc, .. } => enc.clone(),
        _ => unreachable!(),
    };
    let r = vm.apply(1, &d);
    let c = match &r[0] {
        Receipt::Deposited { commitment, .. } => *commitment,
        o => panic!("deposit refused: {o:?}"),
    };
    assert_eq!(vm.conserved(), Ok(()));
    assert!(vm.live.contains_key(&c));

    // Alice scans the epoch's outputs and opens the one that is hers.
    let plain = open(&alice_view, &enc).expect("the depositor must be able to open it");
    assert_eq!(&plain[..32], &ZEC, "and learn which asset");
    assert_eq!(&plain[48..80], &[0x31u8; 32], "and the nonce she needs to spend it");
    assert_eq!(plain.len(), 80, "asset(32) + amount(16) + nonce(32)");

    // A stranger scanning the same output learns nothing.
    assert!(open(&StaticSecret::from([0xBB; 32]), &enc).is_err());
}

/// One Zcash transaction, one object. Without this the same intent replayed
/// mints the same ZEC twice — which is the only way an object ledger can
/// inflate against its backing.
#[test]
fn the_same_zcash_transaction_cannot_be_credited_twice() {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let view = StaticSecret::from([0x0A; 32]);

    assert!(matches!(
        vm.apply(1, &deposit_of(17, ALICE, 0x32, &view, TXID))[0],
        Receipt::Deposited { .. }
    ));
    // Same txid, different nonce, different owner — still refused.
    let r = vm.apply(2, &deposit_of(17, BOB, 0x33, &view, TXID));
    assert_eq!(r[0], Receipt::Refused("that deposit was already credited"));
    assert_eq!(vm.issued[&ZEC], Fixed::whole(17), "supply did not move");
    assert_eq!(vm.conserved(), Ok(()));
}

/// A deposit with no ciphertext is an object nobody can spend. The VM cannot
/// check that the ciphertext *opens* — it holds no viewing key — but it can
/// refuse an absent one.
#[test]
fn a_deposit_without_an_encrypted_note_is_refused() {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let r = vm.apply(
        1,
        &Op::Deposit {
            program: program::deposit(),
            asset: ZEC,
            amount: Fixed::whole(5),
            owner: ALICE,
            nonce: [0x34; 32],
            enc: EncryptedNote { epk: [0u8; 32], ciphertext: Vec::new() },
            external_ref: TXID,
        },
    );
    assert_eq!(r[0], Receipt::Refused("a deposit needs an encrypted note"));
}

/// §130.13: the backing signers compare this amount against what the named
/// Zcash transaction actually paid the vault. Both halves must be in the
/// journal for that check to have two sides.
#[test]
fn a_deposit_names_the_transaction_behind_it() {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let view = StaticSecret::from([0x0A; 32]);
    let mut d = deposit_of(9, ALICE, 0x35, &view, [0u8; 32]);
    if let Op::Deposit { external_ref, .. } = &mut d {
        *external_ref = [0u8; 32];
    }
    assert_eq!(
        vm.apply(1, &d)[0],
        Receipt::Refused("a deposit names the transaction behind it")
    );
}

/// The ciphertext rides inside the intent, so `intent_acc` commits it and a
/// sequencer that withheld one would be publishing a root nobody can rebuild.
#[test]
fn the_encrypted_note_survives_the_wire_and_is_committed() {
    let view = StaticSecret::from([0x0A; 32]);
    let d = deposit_of(4, ALICE, 0x36, &view, TXID);
    let bytes = ObjectVm::encode_intent(&d);
    let mut dec = zyn_vm::read::Decoder::new(&bytes);
    assert_eq!(ObjectVm::decode_intent(&mut dec), Some(d.clone()));

    // A different ciphertext is a different intent, so it cannot be swapped
    // after the fact without changing the accumulator.
    let other = deposit_of(4, ALICE, 0x37, &view, TXID);
    assert_ne!(ObjectVm::encode_intent(&other), bytes);
}
