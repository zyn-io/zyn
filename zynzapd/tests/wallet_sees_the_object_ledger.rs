//! The wallet's view of an object ledger, end to end.
//!
//! The ledger answers no question about an account, so "does the wallet see
//! its money" cannot be tested by asking the node. It is tested the way it
//! works: mint objects to a key, serve the feed every reader gets, and check
//! the holder reconstructs a balance from it while a stranger reconstructs
//! nothing.

use objectvm::{Limits, ObjectVm, Op};
use zynzapd::object_client::parse_published;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{commitment, nullifier, Note};
use zyn_vm::scan::Scanner;
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x7a; 32];
const ALICE: AccountId = [0xA1; 32];
const NK: [u8; 32] = [0x31; 32];

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

/// The feed the node serves — the same bytes to everyone.
fn feed(vm: &ObjectVm, encs: &[(Vec<u8>, zyn_vm::note_enc::EncryptedNote)], spent: &[[u8; 32]]) -> String {
    let outs: Vec<String> = encs
        .iter()
        .map(|(c, e)| {
            format!(
                "{{\"commitment\":\"{}\",\"epk\":\"{}\",\"ciphertext\":\"{}\"}}",
                hex(c),
                hex(&e.epk),
                hex(&e.ciphertext)
            )
        })
        .collect();
    let nulls: Vec<String> = spent.iter().map(|n| format!("\"{}\"", hex(n))).collect();
    let _ = vm;
    format!(
        "{{\"result\":{{\"outputs\":[{}],\"nullifiers\":[{}]}}}}",
        outs.join(","),
        nulls.join(",")
    )
}

fn sealed_note(to: &StaticSecret, eph: u8, n: &Note) -> zyn_vm::note_enc::EncryptedNote {
    let mut plain = Vec::with_capacity(80);
    plain.extend_from_slice(&n.asset);
    plain.extend_from_slice(&n.amount.0.to_be_bytes());
    plain.extend_from_slice(&n.nonce);
    seal(&viewing_public(to), StaticSecret::from([eph; 32]), &plain).unwrap()
}

/// Alice is paid twice and spends once. Her wallet must report what is left,
/// having asked the node nothing about herself.
#[test]
fn the_wallet_reconstructs_its_balance_from_the_public_feed() {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let alice = StaticSecret::from([0x11; 32]);
    let mut seq = 0u64;

    seq += 1;
    vm.apply(
        seq,
        &Op::Register {
            account: ALICE,
            program: program::register_viewing_key(),
            viewing: viewing_public(&alice),
        },
    );

    let mut encs = Vec::new();
    for (amount, nonce, eph) in [(5i64, 0xD1u8, 41u8), (12, 0xD2, 42)] {
        let n = Note {
            asset: ZEC,
            amount: Fixed::whole(amount),
            owner: ALICE,
            nonce: [nonce; 32],
        };
        let enc = sealed_note(&alice, eph, &n);
        seq += 1;
        vm.apply(
            seq,
            &Op::Deposit {
                program: program::deposit(),
                asset: n.asset,
                amount: n.amount,
                owner: n.owner,
                nonce: n.nonce,
                enc: enc.clone(),
                external_ref: [nonce; 32],
            },
        );
        encs.push((commitment(&n).unwrap().to_vec(), enc));
    }

    // Everything the node serves, to anybody.
    let body = feed(&vm, &encs, &[]);
    let (outputs, spent) = parse_published(&body).expect("the feed parses");
    assert_eq!(outputs.len(), 2);

    let w = Scanner::new(StaticSecret::from([0x11; 32]), NK).scan(&ALICE, &outputs, 0, &spent);
    assert_eq!(w.balance(&ZEC), Fixed::whole(17).0, "5 + 12");

    // A stranger with the identical bytes learns nothing.
    let stranger = Scanner::new(StaticSecret::from([0x99; 32]), [0x98; 32])
        .scan(&ALICE, &outputs, 0, &spent);
    assert!(stranger.live.is_empty(), "the feed is public and still private");

    // And once one note is spent, the balance falls — the half a wallet
    // cannot skip, or it offers to spend what is already gone.
    let first = commitment(&Note {
        asset: ZEC,
        amount: Fixed::whole(5),
        owner: ALICE,
        nonce: [0xD1; 32],
    })
    .unwrap();
    let body = feed(&vm, &encs, &[nullifier(&NK, &first)]);
    let (outputs, spent) = parse_published(&body).expect("parses");
    let w = Scanner::new(StaticSecret::from([0x11; 32]), NK).scan(&ALICE, &outputs, 0, &spent);
    assert_eq!(w.balance(&ZEC), Fixed::whole(12).0);
    assert_eq!(w.spent.len(), 1, "and it is remembered, not dropped");
}
