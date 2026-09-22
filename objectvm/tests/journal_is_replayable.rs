//! A root is only meaningful next to the intents that produced it.
//!
//! The object node used to seal epochs and publish roots while writing no
//! journal, so nothing outside it could check a single claim it made. That is
//! the difference between a devnet and a chain: not whether it runs, but
//! whether anyone else can reproduce what it says.
//!
//! This is the property a verifier actually tests before endorsing: the epoch
//! file parses, its records are in order, and its last record is the seal.
//! An epoch whose last record is not a seal is one a replica refuses — and
//! refusing costs one pass, where publishing it halts every replica that
//! reads the bundle.

use zyn::journal::{read_epoch, Journal};
use zyn_vm::spec::MicrochainVm;

use objectvm::{Limits, ObjectVm, Op};
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::{program, Fixed};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("objvm-jrnl-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Write an epoch the way the node does: every intent, then the seal.
#[test]
fn a_journalled_epoch_parses_and_ends_with_its_seal() {
    let dir = tmp("replay");
    let chain = 212u32;
    let mut j = Journal::open(&dir, chain).expect("journal");
    let mut vm = ObjectVm::genesis(chain, Limits { max_io: 8 });

    let alice = StaticSecret::from([0x11; 32]);
    let ops = vec![
        Op::Register {
            account: [0xA1; 32],
            program: program::register_viewing_key(),
            viewing: viewing_public(&alice),
        },
        Op::Deposit {
            program: program::deposit(),
            asset: [0x7a; 32],
            amount: Fixed::whole(5),
            owner: [0xA1; 32],
            nonce: [0xD1; 32],
            enc: {
                let mut plain = Vec::new();
                plain.extend_from_slice(&[0x7a; 32]);
                plain.extend_from_slice(&Fixed::whole(5).0.to_be_bytes());
                plain.extend_from_slice(&[0xD1; 32]);
                seal(
                    &viewing_public(&alice),
                    StaticSecret::from([0x41; 32]),
                    &plain,
                )
                .unwrap()
            },
            external_ref: [0xEE; 32],
        },
    ];

    let mut seq = 0u64;
    for op in &ops {
        seq += 1;
        vm.apply(seq, op);
        j.record(1, seq, &ObjectVm::encode_intent(op))
            .expect("record");
    }
    seq += 1;
    j.record(1, seq, &ObjectVm::encode_intent(&ObjectVm::seal_intent()))
        .expect("seal");
    j.sync().expect("sync");

    let raw = std::fs::read(
        dir.join(format!("journal/chain-{chain}/epoch-1.intents")),
    )
    .expect("the epoch file exists");

    let f = read_epoch(&raw).expect("it parses");
    assert_eq!(f.chain_id, chain);
    assert_eq!(f.epoch, 1);
    assert_eq!(f.records.len(), ops.len() + 1, "every intent, and the seal");

    // Strictly increasing, which is what read_epoch enforces and what the
    // duplicate-append bug broke on chain 11 (A125).
    for w in f.records.windows(2) {
        assert!(w[0].0 < w[1].0, "sequence numbers must increase");
    }

    // The last record must be the seal — the exact test a publisher runs
    // before it will build a bundle.
    let last = &f.records.last().expect("non-empty").1;
    let decoded = <ObjectVm as MicrochainVm>::decode_intent(&mut zyn_vm::read::Decoder::new(last))
        .expect("the last record decodes");
    assert!(
        <ObjectVm as MicrochainVm>::is_seal(&decoded),
        "the last record of a sealed epoch must be the seal"
    );

    // And replaying the journal reaches the same root the node published.
    let mut replay = ObjectVm::genesis(chain, Limits { max_io: 8 });
    let mut n = 0u64;
    for (_, bytes) in &f.records {
        let mut d = zyn_vm::read::Decoder::new(bytes);
        let op = <ObjectVm as MicrochainVm>::decode_intent(&mut d).expect("decodes");
        if <ObjectVm as MicrochainVm>::is_seal(&op) {
            continue;
        }
        n += 1;
        replay.apply(n, &op);
    }
    assert_eq!(
        replay.state_root(),
        vm.state_root(),
        "a replay of the journal must reach the root the node served"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
