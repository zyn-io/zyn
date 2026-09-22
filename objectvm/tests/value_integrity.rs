//! Can value be created from nothing?
//!
//! Conservation here is a sum: inputs minus outputs must be zero, per asset.
//! A sum is only a safe test of conservation if every term is non-negative —
//! otherwise a negative output pays for an inflated positive one and the
//! books balance while the holder walks away richer.

use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{commitment, nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ZEC: [u8; 32] = [0x7a; 32];
const ALICE: AccountId = [0xA1; 32];
const SINK: AccountId = [0x00; 32];
const NK: [u8; 32] = [0x31; 32];

fn view(b: u8) -> StaticSecret {
    StaticSecret::from([b; 32])
}
fn note(owner: AccountId, amount: i128, nonce: u8) -> Note {
    Note {
        asset: ZEC,
        amount: Fixed(amount),
        owner,
        nonce: [nonce; 32],
    }
}
fn out(n: Note) -> Output {
    let mut plain = Vec::with_capacity(80);
    plain.extend_from_slice(&n.asset);
    plain.extend_from_slice(&n.amount.0.to_be_bytes());
    plain.extend_from_slice(&n.nonce);
    Output {
        enc: seal(&viewing_public(&view(0x11)), StaticSecret::from([0x42; 32]), &plain).unwrap(),
        note: n,
    }
}

fn funded() -> (ObjectVm, u64, Note) {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let mut seq = 1;
    vm.apply(
        seq,
        &Op::Register {
            account: ALICE,
            program: program::register_viewing_key(),
            viewing: viewing_public(&view(0x11)),
        },
    );
    let n = note(ALICE, Fixed::whole(10).0, 0xD1);
    seq += 1;
    let r = vm.apply(
        seq,
        &Op::Deposit {
            program: program::deposit(),
            asset: n.asset,
            amount: n.amount,
            owner: n.owner,
            nonce: n.nonce,
            enc: out(n).enc,
            external_ref: [0xEE; 32],
        },
    );
    assert!(matches!(r[0], Receipt::Deposited { .. }), "{:?}", r[0]);
    (vm, seq, n)
}

/// **Value must not be creatable with a negative output.**
///
/// Spend 10, create 1,000 for yourself and −990 for anywhere. The sum is
/// zero, so a conservation check that only adds will pass it — and the
/// attacker holds a live note worth a hundred times what they put in, which
/// they can sell, or exit with against the vault.
#[test]
fn a_negative_output_cannot_pay_for_an_inflated_one() {
    let (mut vm, mut seq, n) = funded();
    let c = commitment(&n).unwrap();
    seq += 1;

    let r = vm.apply(
        seq,
        &Op::Spend {
            spender: ALICE,
            program: program::private_transfer(),
            inputs: vec![(c, nullifier(&NK, &c))],
            outputs: vec![
                out(note(ALICE, Fixed::whole(1000).0, 0xE1)),
                out(note(SINK, -Fixed::whole(990).0, 0xE2)),
            ],
        },
    );

    assert!(
        matches!(r[0], Receipt::Refused(_)),
        "a negative output minted value: {:?}",
        r[0]
    );
    assert!(
        !vm.live
            .values()
            .any(|note| note.amount.0 > Fixed::whole(10).0),
        "no live note may be worth more than was ever deposited"
    );
    assert_eq!(vm.conserved(), Ok(()));
}

/// A deposit of a negative amount would issue negative supply.
#[test]
fn a_negative_deposit_is_refused() {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    vm.apply(
        1,
        &Op::Register {
            account: ALICE,
            program: program::register_viewing_key(),
            viewing: viewing_public(&view(0x11)),
        },
    );
    let n = note(ALICE, -5, 0xD9);
    let r = vm.apply(
        2,
        &Op::Deposit {
            program: program::deposit(),
            asset: n.asset,
            amount: n.amount,
            owner: n.owner,
            nonce: n.nonce,
            enc: out(n).enc,
            external_ref: [0xE9; 32],
        },
    );
    assert!(matches!(r[0], Receipt::Refused(_)), "{:?}", r[0]);
}

/// Summing attacker-chosen amounts must not overflow. An arithmetic panic in
/// the VM is a halt, and the input that causes it is free to construct.
#[test]
fn enormous_outputs_cannot_panic_the_vm() {
    let (mut vm, mut seq, n) = funded();
    let c = commitment(&n).unwrap();
    seq += 1;

    let r = vm.apply(
        seq,
        &Op::Spend {
            spender: ALICE,
            program: program::private_transfer(),
            inputs: vec![(c, nullifier(&NK, &c))],
            outputs: vec![
                out(note(ALICE, i128::MAX, 0xF1)),
                out(note(ALICE, i128::MAX, 0xF2)),
            ],
        },
    );
    assert!(matches!(r[0], Receipt::Refused(_)), "{:?}", r[0]);
}

/// The same, through the claim path, where the seller's payment is summed.
#[test]
fn enormous_payments_cannot_panic_the_claim_path() {
    let (mut vm, mut seq, n) = funded();
    let c = commitment(&n).unwrap();

    seq += 1;
    let r = vm.apply(
        seq,
        &Op::PlaceOffer {
            seller: ALICE,
            program: program::place_offer(),
            item: c,
            price_asset: ZEC,
            price_amount: Fixed::whole(1),
            expires_at_epoch: u64::MAX,
        },
    );
    assert_eq!(r[0], Receipt::Listed { item: c });

    seq += 1;
    let r = vm.apply(
        seq,
        &Op::TakeOffer {
            buyer: ALICE,
            program: program::take_offer(),
            item: c,
            item_nullifier: nullifier(&NK, &c),
            payment: vec![],
            outputs: vec![
                out(note(ALICE, i128::MAX, 0xF3)),
                out(note(ALICE, i128::MAX, 0xF4)),
            ],
        },
    );
    assert!(matches!(r[0], Receipt::Refused(_)), "{:?}", r[0]);
}

/// A negative withdrawal subtracts a negative from issued supply, raising the
/// amount the vault believes it backs, and records a debt of less than
/// nothing. Do it once and the chain will let you withdraw the difference.
#[test]
fn a_negative_withdrawal_cannot_inflate_the_backed_supply() {
    let (mut vm, mut seq, n) = funded();
    let c = commitment(&n).unwrap();
    let issued_before = vm.issued.get(&ZEC).copied().unwrap_or(Fixed::ZERO);

    seq += 1;
    let r = vm.apply(
        seq,
        &Op::Withdraw {
            holder: ALICE,
            program: program::withdraw(),
            inputs: vec![(c, nullifier(&NK, &c))],
            asset: ZEC,
            amount: Fixed(-Fixed::whole(1000).0),
            destination: [0x0D; 32],
            outputs: vec![out(note(ALICE, Fixed::whole(10).0 + Fixed::whole(1000).0, 0xE7))],
        },
    );
    assert!(matches!(r[0], Receipt::Refused(_)), "{:?}", r[0]);
    assert_eq!(
        vm.issued.get(&ZEC).copied().unwrap_or(Fixed::ZERO),
        issued_before,
        "issued supply must not move on a refused withdrawal"
    );
    assert!(vm.exits.is_empty(), "and no debt may be recorded");
}

/// A negative asking price is met by any payment at all, including none,
/// because the claim path checks only that the seller was paid *at least*
/// the asking price.
#[test]
fn a_listing_cannot_ask_a_negative_price() {
    let (mut vm, mut seq, n) = funded();
    let c = commitment(&n).unwrap();
    seq += 1;
    let r = vm.apply(
        seq,
        &Op::PlaceOffer {
            seller: ALICE,
            program: program::place_offer(),
            item: c,
            price_asset: ZEC,
            price_amount: Fixed(-1),
            expires_at_epoch: u64::MAX,
        },
    );
    assert!(matches!(r[0], Receipt::Refused(_)), "{:?}", r[0]);
    assert!(vm.offers.is_empty());
}
