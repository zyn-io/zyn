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
            inputs: vec![(input, nullifier(&ALICE_KEY, &input))],
            outputs: vec![note(5, BOB, 7), note(12, ALICE, 8)],
        },
        // Bob spending Alice's object: refused, and must move nothing.
        rejected: Op::Spend {
            spender: BOB,
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
