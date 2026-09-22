//! A replica is only worth running if it says no.
//!
//! Every test here builds a real journal the way the node writes one, then
//! asks the verifier about it. The passing case proves the check can be
//! satisfied; the rest prove it cannot be satisfied by a chain that cheated,
//! which is the only property that makes the passing case mean anything.

use objectvm::{Limits, ObjectVm, Op};
use zyn::journal::Journal;
use zyn_vm::note_enc::{viewing_public, StaticSecret};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};
use zynzapd::object_verify::{fresh, verify_epoch, Verdict};

const CHAIN: u32 = 212;
const ALICE: AccountId = [0xA1; 32];
const ZEC: [u8; 32] = [0x7a; 32];

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("zyn-replica-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Build one sealed epoch exactly as the node does, and return
/// `(journal bytes, the root the node would publish)`.
fn sealed_epoch(dir: &std::path::Path) -> (Vec<u8>, [u8; 32]) {
    let mut vm = ObjectVm::genesis(CHAIN, Limits { max_io: 8 });
    let mut j = Journal::open(dir, CHAIN).expect("journal opens");
    let mut seq = 0u64;

    let record = |vm: &mut ObjectVm, j: &mut Journal, seq: &mut u64, op: &Op| {
        *seq += 1;
        vm.apply(*seq, op);
        let encoded = zyn_vm::verify::encode_authorized::<ObjectVm>(
            &zyn_vm::verify::Authorized::operator(op.clone()),
        );
        j.record(0, *seq, &encoded).expect("records");
    };

    record(
        &mut vm,
        &mut j,
        &mut seq,
        &Op::Register {
            account: ALICE,
            program: program::register_viewing_key(),
            viewing: viewing_public(&StaticSecret::from([0xA1; 32])),
        },
    );
    let credit = objectvm::ingress::credit(
        &vm,
        ZEC,
        Fixed::whole(4),
        ALICE,
        [0x01; 32],
        [0x02; 32],
        StaticSecret::from([0x03; 32]),
    )
    .expect("alice is registered");
    record(&mut vm, &mut j, &mut seq, &credit);

    // The seal closes the epoch, and is itself part of what the root commits
    // to — which is why it has to be in the journal as well as applied.
    record(&mut vm, &mut j, &mut seq, &ObjectVm::seal_intent());
    j.sync().expect("flushes");

    let root = vm.state_root();
    let raw = std::fs::read(zyn::journal::epoch_path(dir, CHAIN, 0))
        .expect("the epoch file exists");
    (raw, root)
}

/// The honest case. Without this the rest prove only that the checker rejects
/// everything.
#[test]
fn an_honest_epoch_verifies() {
    let dir = tmp("honest");
    let (raw, root) = sealed_epoch(&dir);
    let mut vm = fresh(CHAIN);
    let mut seq = 0u64;
    assert_eq!(
        verify_epoch(&raw, &root, &mut vm, &mut seq),
        Verdict::Verified { root }
    );
}

/// The claim the whole thing rests on: a root the intents do not produce is
/// rejected. This is a sequencer publishing a root it did not earn.
#[test]
fn a_root_the_intents_do_not_produce_is_rejected() {
    let dir = tmp("wrongroot");
    let (raw, root) = sealed_epoch(&dir);
    let mut lie = root;
    lie[0] ^= 0x01;

    let mut vm = fresh(CHAIN);
    let mut seq = 0u64;
    match verify_epoch(&raw, &lie, &mut vm, &mut seq) {
        Verdict::Disagrees { why } => {
            assert!(why.contains("replaying"), "{why}");
        }
        other => panic!("a forged root must be rejected: {other:?}"),
    }
}

/// An epoch whose last record is not a seal is one the operator could still
/// be adding to, so a root over it commits to nothing.
#[test]
fn an_epoch_that_was_never_sealed_is_rejected() {
    let dir = tmp("unsealed");
    let mut vm = ObjectVm::genesis(CHAIN, Limits { max_io: 8 });
    let mut j = Journal::open(&dir, CHAIN).expect("journal opens");
    let op = Op::Register {
        account: ALICE,
        program: program::register_viewing_key(),
        viewing: viewing_public(&StaticSecret::from([0xA1; 32])),
    };
    vm.apply(1, &op);
    let encoded = zyn_vm::verify::encode_authorized::<ObjectVm>(
        &zyn_vm::verify::Authorized::operator(op),
    );
    j.record(0, 1, &encoded).expect("records");
    j.sync().expect("flushes");

    let raw = std::fs::read(zyn::journal::epoch_path(&dir, CHAIN, 0)).unwrap();
    let mut v = fresh(CHAIN);
    let mut seq = 0u64;
    match verify_epoch(&raw, &vm.state_root(), &mut v, &mut seq) {
        Verdict::Disagrees { why } => assert!(why.contains("seal"), "{why}"),
        other => panic!("an unsealed epoch must be rejected: {other:?}"),
    }
}

/// A journal with a record removed must not still reach the published root —
/// otherwise an operator could drop an intent they regretted.
#[test]
fn a_journal_with_an_intent_removed_is_rejected() {
    let dir = tmp("dropped");
    let (raw, root) = sealed_epoch(&dir);

    // Truncate to the header plus the first record, then re-seal nothing: the
    // file now claims fewer intents than the root was built from.
    let cut = raw.len() / 2;
    let mut vm = fresh(CHAIN);
    let mut seq = 0u64;
    assert!(
        matches!(
            verify_epoch(&raw[..cut], &root, &mut vm, &mut seq),
            Verdict::Disagrees { .. }
        ),
        "a truncated journal must not verify"
    );
}

/// Bytes that are not a journal at all are refused rather than panicked on —
/// the input comes from a mirror nobody is asked to trust.
#[test]
fn rubbish_from_a_mirror_is_refused_not_panicked_on() {
    let mut vm = fresh(CHAIN);
    let mut seq = 0u64;
    for bytes in [b"".to_vec(), b"ZYNJRNL2".to_vec(), vec![0xFF; 64]] {
        assert!(
            matches!(
                verify_epoch(&bytes, &[0u8; 32], &mut vm, &mut seq),
                Verdict::Disagrees { .. }
            ),
            "{bytes:?} must be refused"
        );
    }
}
