//! From a Zcash block to an object only its holder can find.
//!
//! The unit tests in `object_ingress` feed `plan` a `WatcherAction` written by
//! hand. That proves the translation and nothing about whether the two halves
//! actually meet, because a hand-written action is exactly the thing the
//! bridge was missing — the point is that a *real* [`Watcher`], applying its
//! real confirmation-depth rule to blocks, produces actions this consumes.
//!
//! So this drives the production watcher. What stays synthetic is the Zcash
//! node itself: `ChainView` is the seam the real `zebra::Observed` implements,
//! and a test cannot mine blocks. Everything above that seam — when a deposit
//! becomes confirmed, what order actions come in, what a rescan does — is the
//! code that runs in production (A116).

use std::cell::Cell;

use objectvm::{Limits, ObjectVm, Op, Receipt};
use zyn_custody::watcher::{ChainView, ObservedDeposit, Watcher};
use zynzapd::object_ingress::{plan, solvent, Planned};

use zyn_vm::note_enc::{viewing_public, StaticSecret};
use zyn_vm::scan::{PublishedOutput, Scanner};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const SEED: &[u8] = b"a bridge seed that lives where the vault's secrets live";
const ZEC: [u8; 32] = [0x7a; 32];
const ALICE: AccountId = [0xA1; 32];
const CONFIRMATIONS: u64 = 5;

/// Blocks with deposits in them. The tip moves; the history does not.
struct Blocks {
    tip: Cell<u64>,
    /// (height, txid, amount)
    deposits: Vec<(u64, [u8; 32], i64)>,
}

impl ChainView for Blocks {
    fn tip(&self) -> u64 {
        self.tip.get()
    }
    fn deposits_at(&self, height: u64) -> Vec<ObservedDeposit> {
        self.deposits
            .iter()
            .filter(|(h, _, _)| *h == height)
            .map(|(h, txid, amount)| ObservedDeposit {
                txid: *txid,
                account: ALICE,
                amount: Fixed::whole(*amount),
                height: *h,
                asset: None,
            })
            .collect()
    }
    fn balance_at(&self, height: u64) -> Option<Fixed> {
        // What the vault holds: everything mined at or below this height.
        Some(Fixed::whole(
            self.deposits
                .iter()
                .filter(|(h, _, _)| *h <= height)
                .map(|(_, _, a)| *a)
                .sum(),
        ))
    }
}

fn alice_view() -> StaticSecret {
    StaticSecret::from([0x11; 32])
}

fn registered_vm() -> ObjectVm {
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    vm.apply(
        1,
        &Op::Register {
            account: ALICE,
            program: program::register_viewing_key(),
            viewing: viewing_public(&alice_view()),
        },
    );
    vm
}

/// Apply everything the watcher produced, and report what the ledger did.
fn drive(vm: &mut ObjectVm, seq: &mut u64, chain: &Blocks, watcher: &mut Watcher) -> Vec<Planned> {
    let actions = watcher.poll(chain).expect("the chain view answered");
    let planned = plan(vm, SEED, ZEC, &actions);
    for p in &planned {
        match p {
            Planned::Submit(op) => {
                *seq += 1;
                let r = vm.apply(*seq, op);
                assert!(
                    matches!(r[0], Receipt::Deposited { .. }),
                    "the ledger refused a confirmed deposit: {:?}",
                    r[0]
                );
            }
            Planned::Attestation { asset, observed } => {
                let id = asset.unwrap_or(ZEC);
                assert!(
                    solvent(vm, &id, *observed),
                    "the vault holds less than the ledger issued"
                );
            }
            _ => {}
        }
    }
    planned
}

/// The whole bridge, end to end: ZEC lands on Zcash, waits its
/// confirmations, and becomes an object Alice's wallet finds without asking
/// anyone what she owns.
#[test]
fn confirmed_zec_becomes_an_object_only_its_holder_can_find() {
    let chain = Blocks {
        tip: Cell::new(100),
        deposits: vec![(100, [0xD1; 32], 3)],
    };
    let mut vm = registered_vm();
    let mut seq = 1u64;
    let mut watcher = Watcher::new(CONFIRMATIONS, 0);

    // One confirmation deep. Real money, not yet safe to credit.
    let planned = drive(&mut vm, &mut seq, &chain, &mut watcher);
    assert!(
        !planned.iter().any(|p| matches!(p, Planned::Submit(_))),
        "a deposit one block deep must not be credited"
    );
    assert!(vm.commitments.is_empty(), "and no object exists yet");

    // Four more blocks and it is confirmed.
    chain.tip.set(104);
    let planned = drive(&mut vm, &mut seq, &chain, &mut watcher);
    let submitted: Vec<&Planned> = planned
        .iter()
        .filter(|p| matches!(p, Planned::Submit(_)))
        .collect();
    assert_eq!(submitted.len(), 1, "exactly one credit: {planned:?}");

    // The attestation came first, as the ordering requires.
    assert!(
        matches!(planned[0], Planned::Attestation { .. }),
        "an attestation must precede the credits it covers"
    );

    // And Alice reconstructs it from what anyone could download.
    let Planned::Submit(op) = submitted[0] else { unreachable!() };
    let Op::Deposit { enc, .. } = &**op else { unreachable!() };
    let commitment = vm.commitments.leaves()[0];
    let wallet = Scanner::new(alice_view(), [0x31; 32]).scan(
        &ALICE,
        &[PublishedOutput {
            commitment,
            enc: enc.clone(),
        }],
        0,
        &[],
    );
    assert_eq!(wallet.live.len(), 1, "Alice must find her own deposit");
    assert_eq!(wallet.balance(&ZEC), Fixed::whole(3).0);
    assert_eq!(vm.conserved(), Ok(()), "and the ledger still balances");

    // A stranger with the same published data sees nothing.
    let stranger = Scanner::new(StaticSecret::from([0x99; 32]), [0x98; 32]).scan(
        &ALICE,
        &[PublishedOutput {
            commitment,
            enc: enc.clone(),
        }],
        0,
        &[],
    );
    assert!(stranger.live.is_empty(), "a stranger must learn nothing");
}

/// The failure a bridge restart causes if the nonce is random: polling the
/// same range again must not mint a second object for one deposit.
#[test]
fn rescanning_the_same_blocks_credits_nothing_twice() {
    let chain = Blocks {
        tip: Cell::new(110),
        deposits: vec![(100, [0xD1; 32], 3), (101, [0xD2; 32], 4)],
    };
    let mut vm = registered_vm();
    let mut seq = 1u64;

    let mut watcher = Watcher::new(CONFIRMATIONS, 0);
    drive(&mut vm, &mut seq, &chain, &mut watcher);
    assert_eq!(vm.commitments.len(), 2);
    let root = vm.state_root();

    // The bridge dies and restarts knowing nothing: a fresh watcher rescans
    // from zero, the way one resumed from a lost cursor would.
    let mut restarted = Watcher::new(CONFIRMATIONS, 0);
    let planned = drive(&mut vm, &mut seq, &chain, &mut restarted);

    assert_eq!(vm.commitments.len(), 2, "no second object for one deposit");
    assert_eq!(vm.state_root(), root, "and the ledger did not move");
    assert_eq!(
        planned
            .iter()
            .filter(|p| matches!(p, Planned::AlreadyCredited { .. }))
            .count(),
        2,
        "both are recognised as already credited: {planned:?}"
    );
}

/// A deposit for someone who has not registered stays in the vault and stays
/// pending. It must not be credited to an object nobody can open, and it must
/// not be forgotten once the watcher has marked its height scanned.
#[test]
fn a_deposit_for_an_unregistered_holder_stays_pending_across_polls() {
    let chain = Blocks {
        tip: Cell::new(110),
        deposits: vec![(100, [0xD1; 32], 3)],
    };
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let mut seq = 0u64;
    let mut watcher = Watcher::new(CONFIRMATIONS, 0);

    let planned = drive(&mut vm, &mut seq, &chain, &mut watcher);
    let pending: Vec<[u8; 32]> = planned
        .iter()
        .filter_map(|p| match p {
            Planned::AwaitingViewingKey { txid, .. } => Some(*txid),
            _ => None,
        })
        .collect();
    assert_eq!(pending, vec![[0xD1; 32]], "it must be reported, not dropped");
    assert!(vm.commitments.is_empty());

    // The watcher will not report it again — it has marked the height
    // scanned and remembers crediting it — so the pending list is the only
    // record. Once Alice registers, replaying it credits her.
    let again = watcher.poll(&chain).expect("chain answered");
    assert!(again.is_empty(), "the watcher does not repeat itself");

    seq += 1;
    vm.apply(
        seq,
        &Op::Register {
            account: ALICE,
            program: program::register_viewing_key(),
            viewing: viewing_public(&alice_view()),
        },
    );
    // Replaying the pending deposit from its txid alone rebuilds the same
    // object the first attempt would have made — which is what makes a
    // pending list sufficient to recover from.
    let replay = zyn_custody::watcher::WatcherAction::Credit {
        account: ALICE,
        amount: Fixed::whole(3),
        index: 0,
        external_ref: [0xD1; 32],
        asset: None,
    };
    let planned = plan(&vm, SEED, ZEC, &[replay]);
    let Planned::Submit(op) = &planned[0] else {
        panic!("a registered holder's pending deposit must become creditable: {planned:?}")
    };
    seq += 1;
    let r = vm.apply(seq, op);
    assert!(matches!(r[0], Receipt::Deposited { .. }), "refused: {:?}", r[0]);
    assert_eq!(vm.commitments.len(), 1);
}

// ---------------------------------------------------------------------------
// A117: the vault must not forget what it owes.
// ---------------------------------------------------------------------------

use zynzapd::object_ingress::pending::{Debt, Pending};

/// The failure A116's test exposed. A deposit arrives for someone who has not
/// registered, the watcher marks the height scanned and will never report it
/// again, and the bridge restarts. Without a written record the money is in
/// the vault and nothing anywhere says whose it is.
#[test]
fn a_pending_deposit_survives_the_restart_that_used_to_lose_it() {
    let chain = Blocks {
        tip: Cell::new(110),
        deposits: vec![(100, [0xD1; 32], 3)],
    };
    let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let mut seq = 0u64;
    let mut watcher = Watcher::new(CONFIRMATIONS, 0);
    let mut owed = Pending::new();

    let planned = drive(&mut vm, &mut seq, &chain, &mut watcher);
    owed.record(&planned, 100);
    assert_eq!(owed.len(), 1, "the debt is recorded");

    // The bridge dies. Everything in memory goes; only the file survives.
    let saved = owed.encode();
    let mut owed = Pending::decode(&saved).expect("the debts file must reload");
    assert_eq!(owed.debts()[0].amount, Fixed::whole(3));
    assert_eq!(owed.debts()[0].height, 100, "and the way back to the evidence");

    // The watcher, resumed from its cursor, will not mention it again.
    let mut resumed = Watcher::resume(CONFIRMATIONS, 0, 105);
    assert!(
        resumed.poll(&chain).expect("chain answered").is_empty(),
        "the watcher does not repeat itself -- the file is the only record"
    );

    // Alice registers, and the debt is payable from the file alone.
    seq += 1;
    vm.apply(
        seq,
        &Op::Register {
            account: ALICE,
            program: program::register_viewing_key(),
            viewing: viewing_public(&alice_view()),
        },
    );
    let retry = owed.replayable(&vm);
    assert_eq!(retry.len(), 1, "it is now creditable");

    let planned = plan(&vm, SEED, ZEC, &retry);
    let Planned::Submit(op) = &planned[0] else { panic!("{planned:?}") };
    seq += 1;
    assert!(matches!(vm.apply(seq, op)[0], Receipt::Deposited { .. }));
    assert!(owed.settle(&[0xD1; 32]), "and the debt is discharged");
    assert!(owed.is_empty());
}

/// A debt is only discharged against a receipt. Until the holder registers it
/// stays owed, however many times the bridge restarts and retries.
#[test]
fn an_unregistered_holders_debt_is_not_replayable_and_is_not_dropped() {
    let vm = ObjectVm::genesis(212, Limits { max_io: 8 });
    let mut owed = Pending::new();
    owed.record(
        &[Planned::AwaitingViewingKey {
            account: ALICE,
            amount: Fixed::whole(3),
            txid: [0xD1; 32],
        }],
        100,
    );
    for _ in 0..3 {
        assert!(owed.replayable(&vm).is_empty(), "nobody to pay yet");
        assert_eq!(owed.len(), 1, "and it is still owed");
    }
}

/// Recording the same deposit twice is one debt, not two. A rewind replays a
/// range on purpose, so this is the ordinary case rather than a corner.
#[test]
fn replaying_a_range_does_not_double_the_debt() {
    let mut owed = Pending::new();
    let p = [Planned::AwaitingViewingKey {
        account: ALICE,
        amount: Fixed::whole(3),
        txid: [0xD1; 32],
    }];
    owed.record(&p, 100);
    owed.record(&p, 100);
    assert_eq!(owed.len(), 1);
}

/// The height is what makes a lost list recoverable: rewind to the earliest
/// debt and the range is read again.
#[test]
fn the_earliest_debt_says_how_far_to_rewind() {
    let mut owed = Pending::new();
    owed.record(
        &[Planned::AwaitingViewingKey { account: ALICE, amount: Fixed::whole(1), txid: [0xD2; 32] }],
        140,
    );
    owed.record(
        &[Planned::AwaitingViewingKey { account: ALICE, amount: Fixed::whole(1), txid: [0xD1; 32] }],
        100,
    );
    assert_eq!(owed.earliest_height(), Some(100));
    assert_eq!(owed.debts()[0].height, 100, "oldest first");
}

/// A half-read debts file reports a shorter list of what the vault owes, and
/// the missing entries are invisible rather than wrong. It must be refused.
#[test]
fn a_truncated_or_padded_debts_file_is_refused_not_guessed() {
    let mut owed = Pending::new();
    owed.record(
        &[Planned::AwaitingViewingKey { account: ALICE, amount: Fixed::whole(3), txid: [0xD1; 32] }],
        100,
    );
    let good = owed.encode();
    assert!(Pending::decode(&good).is_some());

    for cut in 1..good.len() {
        assert!(
            Pending::decode(&good[..cut]).is_none(),
            "a file cut at {cut} must not decode"
        );
    }
    let mut padded = good.clone();
    padded.push(0);
    assert!(Pending::decode(&padded).is_none(), "trailing bytes are not a debts file");

    let mut wrong_magic = good.clone();
    wrong_magic[0] = b'X';
    assert!(Pending::decode(&wrong_magic).is_none());
}

/// An empty list round-trips, so "nothing owed" is a state the file can hold
/// rather than an absence indistinguishable from a lost file.
#[test]
fn an_empty_debts_file_round_trips() {
    let owed = Pending::new();
    let back = Pending::decode(&owed.encode()).expect("empty is a real state");
    assert!(back.is_empty());
    assert_eq!(back.earliest_height(), None);
}

/// Every field has to survive the round trip; an amount that reloads wrong is
/// a debt paid in the wrong quantity.
#[test]
fn every_field_survives_the_round_trip() {
    let mut owed = Pending::new();
    owed.record(
        &[Planned::AwaitingViewingKey {
            account: [0xAB; 32],
            amount: Fixed::whole(123_456),
            txid: [0xCD; 32],
        }],
        4_371_399,
    );
    let back = Pending::decode(&owed.encode()).unwrap();
    assert_eq!(
        back.debts()[0],
        Debt { account: [0xAB; 32], amount: Fixed::whole(123_456), txid: [0xCD; 32], height: 4_371_399 }
    );
}
