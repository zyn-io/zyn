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
use zynzapd::object_verify::{fresh, verify_epoch, Verdict, DEFAULT_MAX_IO};

const CHAIN: u32 = 212;
const ALICE: AccountId = [0xA1; 32];
const ZEC: [u8; 32] = [0x7a; 32];

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("zyn-replica-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A node set up as `zyn-object-node` sets one up — same limit, the journal
/// as its recorder — so the roots it anchors are the roots a replica has to
/// agree with, not ones this file computed its own way.
fn node(dir: &std::path::Path) -> zyn::Node<ObjectVm> {
    let journal = Journal::open(dir, CHAIN).expect("journal opens");
    zyn::Node::new(
        CHAIN,
        Limits { max_io: DEFAULT_MAX_IO },
        zyn::epoch::EpochPolicy {
            intents_per_epoch: 1_000,
            epochs_per_anchor: u64::MAX,
            max_seconds_per_epoch: u64::MAX,
            max_seconds_per_anchor: u64::MAX,
        },
        zyn::Economics::flat(1),
    )
    .with_recorder(Box::new(zynzapd::boot::JournalRecorder(std::sync::Arc::new(
        std::sync::Mutex::new(journal),
    ))))
}

fn register(n: &mut zyn::Node<ObjectVm>, who: u8) {
    let step = n.submit_operator(
        Op::Register {
            account: [who; 32],
            program: program::register_viewing_key(),
            viewing: viewing_public(&StaticSecret::from([who; 32])),
        },
        0,
    );
    assert!(!step.receipts.iter().any(ObjectVm::rejected));
}

/// One sealed epoch — a registration and a credit — and the root the node
/// anchors for it.
fn sealed_epoch(dir: &std::path::Path) -> (Vec<u8>, [u8; 32]) {
    let mut n = node(dir);
    register(&mut n, 0xA1);
    let credit = objectvm::ingress::credit(
        n.state(),
        ZEC,
        Fixed::whole(4),
        ALICE,
        [0x01; 32],
        [0x02; 32],
        StaticSecret::from([0x03; 32]),
    )
    .expect("alice is registered");
    assert!(!n.submit_operator(credit, 0).receipts.iter().any(ObjectVm::rejected));
    n.seal_now(0).expect("seals");
    let anchor = n.anchor_now(0).expect("anchors");
    drop(n);
    let raw = std::fs::read(zyn::journal::epoch_path(dir, CHAIN, 0))
        .expect("the epoch file exists");
    (raw, anchor.checkpoint.state_root)
}

/// The honest case. Without this the rest prove only that the checker rejects
/// everything.
#[test]
fn an_honest_epoch_verifies() {
    let dir = tmp("honest");
    let (raw, root) = sealed_epoch(&dir);
    let mut vm = fresh(CHAIN, DEFAULT_MAX_IO);
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

    let mut vm = fresh(CHAIN, DEFAULT_MAX_IO);
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
    let mut v = fresh(CHAIN, DEFAULT_MAX_IO);
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
    let mut vm = fresh(CHAIN, DEFAULT_MAX_IO);
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
    let mut vm = fresh(CHAIN, DEFAULT_MAX_IO);
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

/// `n` consecutive sealed epochs under one anchor, as the node makes them.
fn sealed_epochs(dir: &std::path::Path, n: u64) -> (Vec<Vec<u8>>, [u8; 32]) {
    let mut node = node(dir);
    for epoch in 0..n {
        register(&mut node, epoch as u8 + 1);
        node.seal_now(0).expect("seals");
    }
    let anchor = node.anchor_now(0).expect("anchors");
    assert_eq!(anchor.epochs, n);
    drop(node);
    let files = (0..n)
        .map(|e| std::fs::read(zyn::journal::epoch_path(dir, CHAIN, e)).unwrap())
        .collect();
    (files, anchor.checkpoint.state_root)
}

/// One anchor a day and a seal a minute: every real anchor covers many
/// epochs, and all of them are needed to reach its root.
#[test]
fn an_anchor_covering_several_epochs_verifies() {
    let dir = tmp("multi");
    let (files, root) = sealed_epochs(&dir, 3);
    let mut vm = fresh(CHAIN, DEFAULT_MAX_IO);
    let mut seq = 0u64;
    assert_eq!(
        zynzapd::object_verify::verify_anchor(&files, &root, &mut vm, &mut seq),
        Verdict::Verified { root }
    );
}

/// Replaying only the anchored epoch — which the replica did — cannot reach
/// the root, and must say so rather than pass.
#[test]
fn skipping_a_covered_epoch_is_caught() {
    let dir = tmp("skip");
    let (files, root) = sealed_epochs(&dir, 3);
    let mut vm = fresh(CHAIN, DEFAULT_MAX_IO);
    let mut seq = 0u64;
    let only_last = vec![files[2].clone()];
    assert!(matches!(
        zynzapd::object_verify::verify_anchor(&only_last, &root, &mut vm, &mut seq),
        Verdict::Disagrees { .. }
    ));
}

/// A sealed journal opens with the DA key under its own path, and not
/// otherwise. Without the key it is "cannot check", not a disagreement.
#[test]
fn a_sealed_journal_opens_only_with_the_key_and_its_own_path() {
    let key = [0x5A; 32];
    let rel = "chain-212/7/intents/epoch-7.intents";
    let plain = b"a journal".to_vec();
    let sealed = zynzapd::dacrypt::seal(&key, CHAIN, &zynzapd::dacrypt::label_for(rel), &plain).unwrap();
    assert_ne!(sealed, plain, "sealed is not the plaintext");

    let open = zynzapd::object_verify::open_journal;
    assert_eq!(open(sealed.clone(), rel, CHAIN, Some(&key)).unwrap(), plain);
    assert!(open(sealed.clone(), rel, CHAIN, None).unwrap_err().contains("no DA key"));
    assert!(open(sealed.clone(), "chain-212/8/intents/epoch-8.intents", CHAIN, Some(&key)).is_err(),
        "a file moved to another epoch's directory must not open");
    assert!(open(sealed, rel, CHAIN, Some(&[0x5B; 32])).is_err(), "nor under another key");
    // An unsealed file passes through untouched.
    assert_eq!(open(plain.clone(), rel, CHAIN, None).unwrap(), plain);
}

/// The I/O limit is in the root. A verifier that assumes another one
/// disagrees with an honest chain, so the default must be the node's.
#[test]
fn the_verifier_must_run_the_chains_io_limit() {
    let dir = tmp("limit");
    let (raw, root) = sealed_epoch(&dir);
    let mut seq = 0u64;
    assert!(matches!(
        verify_epoch(&raw, &root, &mut fresh(CHAIN, 8), &mut seq),
        Verdict::Disagrees { .. }
    ));
}
