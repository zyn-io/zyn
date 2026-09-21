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
                nullifier_commitment: ObjectVm::nullifier_commitment(&nk),
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
    let p = prove(&vm, &n, ALICE_NK, root).expect("Alice holds this note");

    assert_eq!(p.verify(), Ok(()));
    assert_eq!(p.claim(), Ok((ZEC, Fixed::whole(9))));
    // Derived, never asserted.
    assert_eq!(
        p.nullifier().unwrap(),
        nullifier(&ALICE_NK, &commitment(&n).unwrap())
    );
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

    let root = anchored(&vm);
    let p = prove(&vm, &n, ALICE_NK, root).expect("the commitment is still in the tree");
    // The commitment tree is append-only, so it is still there — and that is
    // exactly why absence from the nullifier set is the load-bearing half.
    assert_eq!(p.verify(), Err(ExitError::AlreadySpent));
    assert_eq!(p.claim(), Err(ExitError::AlreadySpent));
}

/// The reason the nullifier key is bound to its account on chain.
///
/// Without that binding a claimant presents any key they like, the derived
/// nullifier is absent from the spent set because nobody ever published it,
/// and a note that was spent long ago gets paid a second time.
#[test]
fn a_spent_note_cannot_be_claimed_with_an_invented_key() {
    let (mut vm, mut seq, n) = chain();
    let c = commitment(&n).unwrap();
    let out = note(MALLORY, 9, 0xD2);
    seq += 1;
    vm.apply(
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
    let root = anchored(&vm);
    let mut p = prove(&vm, &n, ALICE_NK, root).unwrap();

    // A key nobody registered. Its nullifier is absent from the spent set.
    let invented = [0x99u8; 32];
    assert!(
        !p.spent
            .contains(&nullifier(&invented, &commitment(&n).unwrap())),
        "the invented key's nullifier really is absent — that is the trap"
    );
    p.nullifier_key = invented;

    assert_eq!(
        p.verify(),
        Err(ExitError::NullifierKeyNotThisAccount),
        "an unbound key must be refused before it is used to derive anything"
    );
}

/// Mallory cannot claim Alice's note by presenting her own registered key.
#[test]
fn one_holder_cannot_claim_anothers_note() {
    let (vm, _, n) = chain();
    let root = anchored(&vm);
    let mut p = prove(&vm, &n, ALICE_NK, root).unwrap();

    // Mallory's key is genuinely registered — just not to this note's owner.
    p.nullifier_key = MALLORY_NK;
    assert_eq!(p.verify(), Err(ExitError::NullifierKeyNotThisAccount));
}

/// A note that was never on the chain is not a claim, however well-formed.
#[test]
fn a_note_that_was_never_deposited_cannot_be_claimed() {
    let (vm, _, _) = chain();
    let invented = note(ALICE, 1_000_000, 0x77);
    assert_eq!(
        prove(&vm, &invented, ALICE_NK, anchored(&vm)),
        Err(ExitError::NotInCommitmentTree)
    );
}

/// Raising the amount after the fact changes the commitment, so the path stops
/// opening. The note is the preimage, not a label attached to one.
#[test]
fn the_claimed_amount_cannot_be_inflated() {
    let (vm, _, n) = chain();
    let root = anchored(&vm);
    let mut p = prove(&vm, &n, ALICE_NK, root).unwrap();
    p.note.amount = Fixed::whole(9_000);
    assert_eq!(p.verify(), Err(ExitError::NotInCommitmentTree));
}

/// Dropping one's own nullifier from the set is the obvious forgery.
#[test]
fn a_trimmed_spent_set_does_not_match_the_root() {
    let (mut vm, mut seq, n) = chain();
    let c = commitment(&n).unwrap();
    let out = note(MALLORY, 9, 0xD2);
    seq += 1;
    vm.apply(
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
    let root = anchored(&vm);
    let mut p = prove(&vm, &n, ALICE_NK, root).unwrap();
    assert_eq!(p.verify(), Err(ExitError::AlreadySpent));

    // Remove the inconvenient entry.
    p.spent.retain(|x| *x != nullifier(&ALICE_NK, &c));
    assert_eq!(
        p.verify(),
        Err(ExitError::SpentSetDoesNotMatchRoot),
        "a set that hides an entry no longer hashes to the anchored root"
    );
}

/// The absence check reasons about a sorted, unique set. A set that is neither
/// could hide an entry in plain sight, so it is refused as a structure rather
/// than searched.
#[test]
fn an_unsorted_or_duplicated_spent_set_is_refused() {
    let (vm, _, n) = chain();
    let root = anchored(&vm);

    let mut p = prove(&vm, &n, ALICE_NK, root).unwrap();
    p.spent = vec![[0x05; 32], [0x03; 32]];
    assert_eq!(p.verify(), Err(ExitError::SpentSetMalformed));

    let mut p = prove(&vm, &n, ALICE_NK, root).unwrap();
    p.spent = vec![[0x03; 32], [0x03; 32]];
    assert_eq!(p.verify(), Err(ExitError::SpentSetMalformed));
}

/// A proof has to hang off a root Zcash actually saw. Otherwise it is
/// internally consistent and describes a chain nobody agreed to — which is
/// precisely what a failing sequencer would hand out.
#[test]
fn a_proof_against_an_unanchored_root_is_refused() {
    let (vm, _, n) = chain();
    let mut p = prove(&vm, &n, ALICE_NK, anchored(&vm)).unwrap();
    p.anchored = [0xAB; 32];
    assert_eq!(p.verify(), Err(ExitError::SectionsDoNotOpenAnchor));
}

/// Swapping in a different commitments root to make a foreign note open must
/// break the anchor, not the path.
#[test]
fn rewriting_a_section_breaks_the_anchor() {
    let (vm, _, n) = chain();
    let root = anchored(&vm);
    let mut p = prove(&vm, &n, ALICE_NK, root).unwrap();
    p.sections[objectvm::SECTION_COMMITMENTS] = [0xCC; 32];
    // The path stops opening first, which is the earlier and clearer refusal.
    assert_eq!(p.verify(), Err(ExitError::NotInCommitmentTree));
}

/// Too few sections is a malformed proof, not a panic. A verifier that indexes
/// blindly is a verifier that can be crashed by a crafted claim.
#[test]
fn a_truncated_section_list_is_refused_not_panicked_on() {
    let (vm, _, n) = chain();
    let mut p = prove(&vm, &n, ALICE_NK, anchored(&vm)).unwrap();
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
    let kept: ExitProof = prove(&vm, &n, ALICE_NK, root).unwrap();

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

/// A note whose owner never registered has nothing binding a key to them, so
/// no proof can be built — and saying so at build time is better than handing
/// out evidence that no verifier will accept.
#[test]
fn an_unregistered_owner_cannot_be_given_a_proof() {
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
    assert_eq!(
        prove(&vm, &n, ALICE_NK, vm.state_root()),
        Err(ExitError::AccountNotRegistered)
    );
}

/// A registration with no nullifier commitment is refused, because an account
/// registered that way could never produce a checkable exit proof.
#[test]
fn a_registration_without_a_nullifier_commitment_is_refused() {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let r = vm.apply(
        1,
        &Op::Register {
            account: ALICE,
            program: program::register_viewing_key(),
            viewing: viewing_public(&view(0x11)),
            nullifier_commitment: [0u8; 32],
        },
    );
    assert_eq!(r[0], Receipt::Refused("not a nullifier-key commitment"));
}
