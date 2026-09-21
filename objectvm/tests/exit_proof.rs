//! Getting out when nobody is left to ask.
//!
//! The account ledger's escape hatch enumerates balances and proves the list
//! hashes into an anchored root. An object ledger has no enumerable balance,
//! so that proof has no translation and a holder would have nothing to show a
//! custodian if the sequencer vanished. A106 makes designing the replacement a
//! precondition of switching the chain over, which is what this is.
//!
//! These tests are adversarial on purpose. This verifier decides who the vault
//! pays, so every way of lying to it is worth a named test — a verifier that
//! only has a happy-path test is a verifier nobody has tried to rob.

use objectvm::exit::{prove, ExitError, ExitProof};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{commitment, nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x7a; 32];
const ALICE: AccountId = [0xA1; 32];
const MALLORY: AccountId = [0x4D; 32];
/// Alice's nullifier key. Recognises her spends; cannot authorise one.
const ALICE_NK: [u8; 32] = [0x31; 32];
const MALLORY_NK: [u8; 32] = [0x66; 32];

fn view(b: u8) -> StaticSecret {
    StaticSecret::from([b; 32])
}

fn note(owner: AccountId, amount: i64, nonce: u8) -> Note {
    Note {
        asset: ZEC,
        amount: Fixed::whole(amount),
        owner,
        nonce: [nonce; 32],
    }
}

fn sealed(to: &StaticSecret, eph: u8, n: &Note) -> zyn_vm::note_enc::EncryptedNote {
    let mut plain = Vec::with_capacity(80);
    plain.extend_from_slice(&n.asset);
    plain.extend_from_slice(&n.amount.0.to_be_bytes());
    plain.extend_from_slice(&n.nonce);
    seal(&viewing_public(to), StaticSecret::from([eph; 32]), &plain).unwrap()
}

/// A chain where Alice and Mallory are registered and Alice holds one note.
fn chain() -> (ObjectVm, u64, Note) {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let mut seq = 0u64;
    for (who, v, nk) in [(ALICE, 0x11u8, ALICE_NK), (MALLORY, 0x22, MALLORY_NK)] {
        seq += 1;
        let r = vm.apply(
            seq,
            &Op::Register {
                account: who,
                program: program::register_viewing_key(),
                viewing: viewing_public(&view(v)),
            },
        );
        assert!(matches!(r[0], Receipt::Registered { .. }), "{:?}", r[0]);
    }
    let n = note(ALICE, 9, 0xD1);
    seq += 1;
    let r = vm.apply(
        seq,
        &Op::Deposit {
            program: program::deposit(),
            asset: n.asset,
            amount: n.amount,
            owner: n.owner,
            nonce: n.nonce,
            enc: sealed(&view(0x11), 0x41, &n),
            external_ref: [0xEE; 32],
        },
    );
    assert!(matches!(r[0], Receipt::Deposited { .. }), "{:?}", r[0]);
    (vm, seq, n)
}

fn anchored(vm: &ObjectVm) -> [u8; 32] {
    vm.state_root()
}

/// The whole point: a holder proves what the vault owes them, and anyone can
/// check it without asking the sequencer anything.
#[test]
fn a_holder_can_prove_what_the_vault_owes_them() {
    let (vm, _, n) = chain();
    let root = anchored(&vm);
    let p = prove(&vm, &n, root).expect("Alice holds this note");

    assert_eq!(p.verify(), Ok(()));
    assert_eq!(p.claim(), Ok((ZEC, Fixed::whole(9))));
    // The claim names the note's owner. A custodian pays that account's
    // bound destination, not whoever produced the proof.
    assert_eq!(p.note.owner, ALICE);
}

/// The attack the whole design turns on. Alice spends her note, then tries to
/// exit with it anyway.
#[test]
fn a_spent_note_cannot_be_claimed() {
    let (mut vm, mut seq, n) = chain();
    let c = commitment(&n).unwrap();

    // She spends it to Mallory.
    let out = note(MALLORY, 9, 0xD2);
    seq += 1;
    let r = vm.apply(
        seq,
        &Op::Spend {
            spender: ALICE,
            program: program::private_transfer(),
            inputs: vec![(c, nullifier(&ALICE_NK, &c))],
            outputs: vec![Output {
                enc: sealed(&view(0x22), 0x42, &out),
                note: out,
            }],
        },
    );
    assert!(matches!(r[0], Receipt::Spent { .. }), "{:?}", r[0]);

    // The commitment tree is append-only, so the commitment is still there.
    // The live set is what moved, and it is what the proof now rests on — so
    // a spent note cannot even be given a proof, let alone verify one.
    assert!(vm.commitments.leaves().contains(&c), "still in the tree");
    assert_eq!(prove(&vm, &n, anchored(&vm)), Err(ExitError::NotLive));
}

/// A note that was never on the chain is not a claim, however well-formed.
#[test]
fn a_note_that_was_never_deposited_cannot_be_claimed() {
    let (vm, _, _) = chain();
    let invented = note(ALICE, 1_000_000, 0x77);
    assert_eq!(
        prove(&vm, &invented, anchored(&vm)),
        Err(ExitError::NotInCommitmentTree)
    );
}

/// Forging the live path is the direct attack now. It fails because the live
/// root is a section of the root Zcash anchored.
///
/// Tested with more than one live object on purpose: a one-leaf tree has the
/// leaf as its root, so *any* path opens it and a test built on a single
/// object would pass no matter what the verifier did.
#[test]
fn a_forged_live_path_does_not_open_the_anchored_root() {
    let (mut vm, mut seq, n) = chain();
    seq += 1;
    let other = note(MALLORY, 4, 0xB7);
    vm.apply(
        seq,
        &Op::Deposit {
            program: program::deposit(),
            asset: other.asset,
            amount: other.amount,
            owner: other.owner,
            nonce: other.nonce,
            enc: sealed(&view(0x22), 0x43, &other),
            external_ref: [0xE1; 32],
        },
    );
    assert!(vm.live_commitments().len() > 1, "more than one leaf, so the path matters");

    let mut p = prove(&vm, &n, anchored(&vm)).unwrap();
    assert_eq!(p.verify(), Ok(()));

    p.live_path = vec![];
    assert_eq!(p.verify(), Err(ExitError::NotLive));

    // And a path borrowed from the other object does not open this one.
    let borrowed = prove(&vm, &other, anchored(&vm)).unwrap();
    p.live_path = borrowed.live_path;
    assert_eq!(p.verify(), Err(ExitError::NotLive));
}

/// A proof has to hang off a root Zcash actually saw. Otherwise it is
/// internally consistent and describes a chain nobody agreed to — which is
/// precisely what a failing sequencer would hand out.
#[test]
fn a_proof_against_an_unanchored_root_is_refused() {
    let (vm, _, n) = chain();
    let mut p = prove(&vm, &n, anchored(&vm)).unwrap();
    p.anchored = [0xAB; 32];
    assert_eq!(p.verify(), Err(ExitError::SectionsDoNotOpenAnchor));
}

/// Swapping in a different commitments root to make a foreign note open must
/// break the anchor, not the path.
#[test]
fn rewriting_a_section_breaks_the_anchor() {
    let (vm, _, n) = chain();
    let root = anchored(&vm);
    let mut p = prove(&vm, &n, root).unwrap();
    p.sections[objectvm::SECTION_COMMITMENTS] = [0xCC; 32];
    // The path stops opening first, which is the earlier and clearer refusal.
    assert_eq!(p.verify(), Err(ExitError::NotInCommitmentTree));
}

/// Too few sections is a malformed proof, not a panic. A verifier that indexes
/// blindly is a verifier that can be crashed by a crafted claim.
#[test]
fn a_truncated_section_list_is_refused_not_panicked_on() {
    let (vm, _, n) = chain();
    let mut p = prove(&vm, &n, anchored(&vm)).unwrap();
    p.sections.truncate(2);
    assert_eq!(p.verify(), Err(ExitError::MalformedSections));
    p.sections.clear();
    assert_eq!(p.verify(), Err(ExitError::MalformedSections));
}

/// A proof survives the round trip through a file, because the whole point is
/// that a holder keeps it and produces it later — possibly much later, on a
/// chain that no longer runs.
#[test]
fn a_proof_is_still_good_after_the_chain_moves_on() {
    let (mut vm, mut seq, n) = chain();
    let root = anchored(&vm);
    let kept: ExitProof = prove(&vm, &n, root).unwrap();

    // Mallory deposits, the tree grows, the state root changes.
    seq += 1;
    let other = note(MALLORY, 4, 0xB7);
    vm.apply(
        seq,
        &Op::Deposit {
            program: program::deposit(),
            asset: other.asset,
            amount: other.amount,
            owner: other.owner,
            nonce: other.nonce,
            enc: sealed(&view(0x22), 0x43, &other),
            external_ref: [0xE1; 32],
        },
    );
    assert_ne!(vm.state_root(), root, "the chain moved");

    // The kept proof still verifies against the root it was made for. It has
    // to: the holder's evidence cannot depend on the sequencer still agreeing
    // with them, since the sequencer being gone is the reason they need it.
    assert_eq!(kept.verify(), Ok(()));
    assert_eq!(kept.claim(), Ok((ZEC, Fixed::whole(9))));
}

/// An unregistered owner can still prove what they hold.
///
/// This used to be impossible, and the impossibility was a bug: the first
/// design bound the proof to a registered nullifier key, so an object sent to
/// someone who had never registered could never be exited by them, and the
/// sender caused that loss rather than the recipient. Resting the proof on
/// the live set removes the requirement entirely (A119).
#[test]
fn an_unregistered_owner_can_still_prove_what_they_hold() {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let n = note(ALICE, 5, 0xC4);
    // Mint straight to an unregistered owner.
    let r = vm.apply(
        1,
        &Op::Deposit {
            program: program::deposit(),
            asset: n.asset,
            amount: n.amount,
            owner: n.owner,
            nonce: n.nonce,
            enc: sealed(&view(0x11), 0x41, &n),
            external_ref: [0xEE; 32],
        },
    );
    assert!(matches!(r[0], Receipt::Deposited { .. }));
    let p = prove(&vm, &n, vm.state_root()).expect("holding it is enough");
    assert_eq!(p.verify(), Ok(()));
    assert_eq!(p.claim(), Ok((ZEC, Fixed::whole(5))));
}
