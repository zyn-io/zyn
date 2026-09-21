//! Collection Z on the object ledger, at the size it actually ships at.
//!
//! Every other test here uses two or three objects, which proves the rules and
//! nothing about whether they survive 4,444 of them. This one mints the real
//! collection, sells an item through the listing and claim path, and measures
//! what it cost — because the failure that matters at this size is not a wrong
//! answer, it is a right answer that arrives too slowly to ship.
//!
//! The numbers it prints are the point. A state root recomputed per intent
//! over a growing commitment tree is the obvious way to be quadratic, and
//! finding that here is cheap; finding it after 4,444 items have owners is
//! not (DECISIONS §133.1).

use std::time::Instant;

use objectvm::{ingress, Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{commitment, nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

/// The real collection size.
const COLLECTION: usize = 4_444;

const CAVE: [u8; 32] = [0xC0; 32];
const ZEC: [u8; 32] = [0x7a; 32];
const STORE: AccountId = [0x57; 32];
const BUYER: AccountId = [0xB0; 32];

fn item_note(serial: usize, owner: AccountId) -> Note {
    // A distinct nonce per item, as a mint would derive rather than invent.
    let mut nonce = [0u8; 32];
    nonce[..8].copy_from_slice(&(serial as u64).to_be_bytes());
    nonce[8] = 1; // never zero, whatever the serial
    Note {
        asset: CAVE,
        amount: Fixed::whole(1),
        owner,
        nonce,
    }
}

fn sealed_to(who: &StaticSecret, eph: u8, note: &Note) -> zyn_vm::note_enc::EncryptedNote {
    let mut plain = Vec::with_capacity(80);
    plain.extend_from_slice(&note.asset);
    plain.extend_from_slice(&note.amount.0.to_be_bytes());
    plain.extend_from_slice(&note.nonce);
    seal(&viewing_public(who), StaticSecret::from([eph; 32]), &plain).unwrap()
}

fn out(note: Note, who: &StaticSecret, eph: u8) -> Output {
    Output {
        enc: sealed_to(who, eph, &note),
        note,
    }
}

/// Mint the collection, sell one item, and say what each step cost.
#[test]
fn collection_z_mints_and_sells_at_full_size() {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let store_view = StaticSecret::from([0x57; 32]);
    let buyer_view = StaticSecret::from([0xB0; 32]);
    let mut seq = 0u64;
    let step = |vm: &mut ObjectVm, seq: &mut u64, op: &Op| {
        *seq += 1;
        vm.apply(*seq, op)
    };

    // Both parties publish where to pay them, which on an object ledger is
    // what having an address means.
    for (who, view) in [(STORE, &store_view), (BUYER, &buyer_view)] {
        let r = step(
            &mut vm,
            &mut seq,
            &Op::Register {
                account: who,
                program: program::register_viewing_key(),
                viewing: viewing_public(view),
            },
        );
        assert!(matches!(r[0], Receipt::Registered { .. }));
    }

    // ---- mint 4,444 items -------------------------------------------------
    let started = Instant::now();
    // Timed in two halves, because the number that matters is not how long
    // 4,444 mints take — it is whether the last thousand cost more than the
    // first. A per-item cost that grows is quadratic, and quadratic at this
    // size is merely slow while quadratic at ten times this size is fatal.
    let mut first_half = std::time::Duration::ZERO;
    let mut minted: Vec<[u8; 32]> = Vec::with_capacity(COLLECTION);
    for serial in 0..COLLECTION {
        if serial == COLLECTION / 2 {
            first_half = started.elapsed();
        }
        let note = item_note(serial, STORE);
        let r = step(
            &mut vm,
            &mut seq,
            &Op::Deposit {
                program: program::deposit(),
                asset: note.asset,
                amount: note.amount,
                owner: note.owner,
                nonce: note.nonce,
                enc: sealed_to(&store_view, (serial % 251) as u8 + 1, &note),
                // One item, one external reference — the mint's own serial
                // stands in for the Zcash txid a deposit would carry.
                external_ref: {
                    let mut r = [0u8; 32];
                    r[..8].copy_from_slice(&(serial as u64).to_be_bytes());
                    r[8] = 0xAA;
                    r
                },
            },
        );
        match &r[0] {
            Receipt::Deposited { commitment, .. } => minted.push(*commitment),
            other => panic!("mint {serial} refused: {other:?}"),
        }
    }
    let minting = started.elapsed();

    assert_eq!(minted.len(), COLLECTION);
    assert_eq!(vm.commitments.len(), COLLECTION);
    assert_eq!(vm.conserved(), Ok(()), "a full collection still balances");

    // ---- what the chain costs at this size --------------------------------
    let t = Instant::now();
    let root = vm.state_root();
    let root_time = t.elapsed();

    let t = Instant::now();
    let encoded = vm.encode();
    let encode_time = t.elapsed();

    let t = Instant::now();
    let restored = ObjectVm::decode(&encoded).expect("a full collection must reload");
    let decode_time = t.elapsed();
    assert_eq!(restored.state_root(), root, "and reload to the same root");

    eprintln!(
        "\n  {COLLECTION} items\n    mint        {:>8.2?}  ({:.2?}/item)\n    state_root  {:>8.2?}\n    encode      {:>8.2?}  ({} KB)\n    decode      {:>8.2?}",
        minting,
        minting / COLLECTION as u32,
        root_time,
        encode_time,
        encoded.len() / 1024,
        decode_time
    );

    // ---- sell one --------------------------------------------------------
    // The buyer needs ZEC first.
    let credit = ingress::credit(
        &vm,
        ZEC,
        Fixed::whole(2),
        BUYER,
        [0xF0; 32],
        [0x99; 32],
        StaticSecret::from([0x98; 32]),
    )
    .expect("the buyer registered, so they are creditable");
    let funds = match &step(&mut vm, &mut seq, &credit)[0] {
        Receipt::Deposited { commitment, .. } => *commitment,
        other => panic!("funding refused: {other:?}"),
    };

    let listed = minted[1177];
    let r = step(
        &mut vm,
        &mut seq,
        &Op::PlaceOffer {
            seller: STORE,
            program: program::place_offer(),
            item: listed,
            price_asset: ZEC,
            price_amount: Fixed::whole(2),
            expires_at_epoch: u64::MAX,
        },
    );
    assert_eq!(r[0], Receipt::Listed { item: listed });
    assert_eq!(vm.live[&listed].owner, STORE, "listing moved nothing");

    let t = Instant::now();
    let r = step(
        &mut vm,
        &mut seq,
        &Op::TakeOffer {
            buyer: BUYER,
            program: program::take_offer(),
            item: listed,
            item_nullifier: nullifier(&[0x5A; 32], &listed),
            payment: vec![(funds, nullifier(&[0xB9; 32], &funds))],
            outputs: vec![
                out(
                    Note {
                        asset: ZEC,
                        amount: Fixed::whole(2),
                        owner: STORE,
                        nonce: [0x21; 32],
                    },
                    &store_view,
                    0x22,
                ),
                out(item_note(1177, BUYER), &buyer_view, 0x23),
            ],
        },
    );
    let claim_time = t.elapsed();
    assert!(matches!(r[0], Receipt::Taken { .. }), "claim refused: {:?}", r[0]);

    let second_half = minting - first_half;
    let growth = second_half.as_secs_f64() / first_half.as_secs_f64();
    eprintln!(
        "    claim       {:>8.2?}  (against a {}-leaf tree)\n    first half  {:>8.2?}\n    second half {:>8.2?}  ({:.2}x — 1.0 is linear)\n",
        claim_time, COLLECTION, first_half, second_half, growth
    );
    // Generous, because a test that fails on scheduler noise gets deleted.
    // Quadratic over these halves would be ~3x, not 1.5x.
    assert!(
        growth < 2.0,
        "minting the second half cost {growth:.2}x the first — the per-item \
         cost is growing with the tree, which does not survive a larger \
         collection"
    );

    // ---- and it is genuinely theirs --------------------------------------
    let bought = commitment(&item_note(1177, BUYER)).unwrap();
    assert!(vm.live.contains_key(&bought), "the buyer holds the item");
    assert!(!vm.live.contains_key(&listed), "the store's copy is consumed");
    assert!(!vm.offers.contains_key(&listed), "and the listing is gone");
    assert_eq!(vm.conserved(), Ok(()), "after a sale, still balanced");

    // Nothing published names either party.
    let sections: Vec<u8> = vm.sections().concat();
    assert!(!sections.windows(32).any(|w| w == BUYER), "no buyer in the root");
    assert!(!sections.windows(32).any(|w| w == STORE), "no seller in the root");
}
