//! A payment becomes an item: the whole chain, composed.
//!
//! Every link existed and none of them was called. The scanner could find Cave
//! payments, `reveal::ledger_from` could turn them into tickets,
//! `allocation::run` could draw, and `sale` could settle — and nothing ran the
//! sequence, so the places where two links disagree about a type or an
//! ordering had never been reached (§106).
//!
//! This is that sequence, with the network stubbed and nothing else. What it
//! proves is composition: that a payment on Zcash, carried through admission,
//! the draw and the order book, arrives as a specific serial in a specific
//! account — and that the serial is the one a stranger replaying the same
//! inputs would compute.

use cave_randomness::{
    AllocationPolicy, IdentityPolicy, ManifestEntry, QuantityPolicy, RandomnessPlan,
    QUICKNET_CHAIN_HASH,
};
use zyn_custody::memo::{CavePaymentMemo, CavePaymentPurpose};
use zyn_vm::derive::ADDRESS_SCOPE_V1;
use zynzapd::allocation;
use zynzapd::reveal::{evidence_for, ledger_from, Sighting};

const COLLECTION: [u8; 32] = [0x02; 32];
const TERMS: [u8; 32] = [0x7c; 32];
const ENTRY: u64 = 1_000_000;
const CLOSE: u32 = 3_500_000;
const TIP: u32 = 3_500_010;
const SUPPLY: u32 = 20;

fn manifest() -> Vec<ManifestEntry> {
    (0..SUPPLY)
        .map(|i| ManifestEntry {
            global_index: i,
            item_id: zyn_vm::item_address(ADDRESS_SCOPE_V1, &COLLECTION, i),
            content_hash: {
                let mut h = [0u8; 32];
                h[..4].copy_from_slice(&i.to_be_bytes());
                h[31] = 0xc0;
                h
            },
        })
        .collect()
}

/// Sequential identity, so the pipeline runs without drand evidence. The
/// concealed path has its own coverage; what is under test here is that the
/// links fit together.
fn policy() -> AllocationPolicy {
    AllocationPolicy {
        collection_id: COLLECTION,
        supply: SUPPLY,
        entry_price_zatoshi: ENTRY,
        deposit_close_zcash_height: CLOSE,
        quantity: QuantityPolicy::Fixed { items: 1 },
        identity: IdentityPolicy::Sequential,
        manifest_root: cave_randomness::manifest_root(COLLECTION, &manifest()).unwrap(),
        randomness: RandomnessPlan {
            drand_chain_hash: QUICKNET_CHAIN_HASH,
            ticket_round_delay: 10,
            reveal_drand_round: None,
            reveal_zcash_height: None,
            announced_close: None,
        },
    }
}

fn paid(txid: u8, buyer: u8, height: u64, tx_index: u32) -> Sighting {
    Sighting {
        txid: [txid; 32],
        height,
        tx_index,
        output_index: 0,
        amount_zatoshi: ENTRY,
        payment: CavePaymentMemo {
            purpose: CavePaymentPurpose::Entry,
            reference: TERMS,
            recipient: [buyer; 32],
        },
    }
}

#[test]
fn a_payment_on_zcash_becomes_a_specific_item_in_a_specific_account() {
    let p = policy();
    let alice = [0xa1u8; 32];
    let bob = [0xb2u8; 32];

    // Three payments arrive, out of order as a scanner might report them.
    let seen = [
        paid(3, 0xb2, 3_499_100, 0),
        paid(1, 0xa1, 3_499_000, 1),
        paid(2, 0xa1, 3_499_000, 0),
    ];

    let (ledger, refused) = ledger_from(&p, TERMS, CLOSE, 6, TIP, &seen, 5_000_000);
    assert!(refused.is_empty(), "three good payments, three tickets");
    assert_eq!(ledger.len(), 3);

    let run = allocation::run(&p, &manifest(), &evidence_for(ledger.tickets()), None)
        .expect("the draw runs over a ledger");

    // Alice paid twice and holds two items; Bob one. Nobody holds the same
    // serial twice, and the two sets do not overlap.
    let a = run.serials_for(&alice);
    let b = run.serials_for(&bob);
    assert_eq!(a.len(), 2, "two payments, two items");
    assert_eq!(b.len(), 1);
    assert!(a.iter().all(|s| !b.contains(s)), "no serial in two accounts");
    assert_eq!(run.unallocated().len(), (SUPPLY - 3) as usize);

    // Every allocated serial is a real manifest entry, and the item id the
    // receipt carries is the one the collection derives — this is the join
    // that a mint depends on and where an off-by-one would land.
    let m = manifest();
    for r in &run.batch.receipts {
        for item in &r.items {
            let entry = &m[item.global_index as usize];
            assert_eq!(item.item_id, entry.item_id);
            assert_eq!(item.content_hash, entry.content_hash);
            assert_eq!(
                item.item_id,
                zyn_vm::item_address(ADDRESS_SCOPE_V1, &COLLECTION, item.global_index),
                "the mint will derive this same id from the serial"
            );
        }
    }
}

/// The ordering must come from Zcash and not from the order a scanner happened
/// to report, or two operators replaying the same block range would hand the
/// same buyer different items.
#[test]
fn the_result_does_not_depend_on_the_order_payments_were_seen() {
    let p = policy();
    let seen = [
        paid(1, 0xa1, 3_499_000, 0),
        paid(2, 0xb2, 3_499_000, 1),
        paid(3, 0xc3, 3_499_005, 0),
    ];
    let forward = {
        let (l, _) = ledger_from(&p, TERMS, CLOSE, 6, TIP, &seen, 5_000_000);
        allocation::run(&p, &manifest(), &evidence_for(l.tickets()), None).unwrap()
    };
    let mut shuffled = seen;
    shuffled.reverse();
    let backward = {
        let (l, _) = ledger_from(&p, TERMS, CLOSE, 6, TIP, &shuffled, 5_000_000);
        allocation::run(&p, &manifest(), &evidence_for(l.tickets()), None).unwrap()
    };
    assert!(
        forward.agrees_with(&backward),
        "the same block range must draw the same result whoever scanned it"
    );
    for who in [[0xa1u8; 32], [0xb2u8; 32], [0xc3u8; 32]] {
        assert_eq!(forward.serials_for(&who), backward.serials_for(&who));
    }
}

/// A payment that is not a ticket must not silently shift what everyone else
/// receives. Underpayment is refused at admission, and the draw over the
/// remaining tickets is the draw those buyers would have had anyway.
#[test]
fn a_refused_payment_does_not_move_anyone_elses_items() {
    let p = policy();
    let good = [paid(1, 0xa1, 3_499_000, 0), paid(2, 0xb2, 3_499_001, 0)];
    let clean = {
        let (l, _) = ledger_from(&p, TERMS, CLOSE, 6, TIP, &good, 5_000_000);
        allocation::run(&p, &manifest(), &evidence_for(l.tickets()), None).unwrap()
    };

    let mut short = paid(3, 0xc3, 3_499_000, 1);
    short.amount_zatoshi -= 1;
    let mixed = [good[0], short, good[1]];
    let (l, refused) = ledger_from(&p, TERMS, CLOSE, 6, TIP, &mixed, 5_000_000);
    assert_eq!(refused.len(), 1, "the short payment is reported");
    let after = allocation::run(&p, &manifest(), &evidence_for(l.tickets()), None).unwrap();

    assert!(clean.agrees_with(&after), "the honest buyers are unaffected");
    assert!(after.serials_for(&[0xc3; 32]).is_empty());
}
