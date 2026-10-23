//! Zyn Pay (spec 2026-10-23-zyn-scheduled-payments-design.md): scheduled
//! payments funded at approval, one-off payments, refunds. Every intent goes
//! through `apply`, and after each one the books must still balance.

use objectvm::pay::{self, Every, ScheduleState};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const BOLD: [u8; 32] = [0xB1; 32];
const ALICE: AccountId = [0xA1; 32];
const SHOP: AccountId = [0x5E; 32];
const DAY: u64 = 86_400;
// 2026-11-01 00:00:00 UTC
const NOV1: u64 = 1_793_491_200;

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

struct Chain {
    vm: ObjectVm,
    seq: u64,
    salt: u64,
}

impl Chain {
    fn new() -> Chain {
        let mut c = Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, salt: 0 };
        for who in [ALICE, SHOP] {
            c.ok(Op::Register { account: who, program: program::register_viewing_key(), viewing: viewing_public(&StaticSecret::from([who[0]; 32])) });
        }
        c
    }

    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let r = self.vm.apply(self.seq, &op);
        assert_eq!(self.vm.conserved(), Ok(()), "after {op:?}");
        assert_eq!(self.vm.reconciled(), Ok(()));
        let back = ObjectVm::decode(&self.vm.encode()).expect("state decodes");
        assert_eq!(back, self.vm, "state round trip");
        let wire = ObjectVm::encode_intent(&op);
        let mut d = zyn_vm::read::Decoder::new(&wire);
        assert_eq!(ObjectVm::decode_intent(&mut d).as_ref(), Some(&op), "intent round trip");
        r.into_iter().next().unwrap()
    }

    fn ok(&mut self, op: Op) -> Receipt {
        let r = self.apply(op);
        assert!(!ObjectVm::rejected(&r), "{r:?}");
        r
    }

    fn refused(&mut self, op: Op) -> &'static str {
        match self.apply(op) {
            Receipt::Refused(why) => why,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn fresh(&mut self) -> Hash {
        self.salt += 1;
        let mut h = [0x77u8; 32];
        h[..8].copy_from_slice(&self.salt.to_be_bytes());
        h
    }

    fn mint(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) {
        let nonce = self.fresh();
        self.ok(Op::Mint { asset, amount, owner, nonce });
    }

    fn balance(&self, owner: AccountId, asset: [u8; 32]) -> i128 {
        self.vm.live.values().filter(|n| n.owner == owner && n.asset == asset).map(|n| n.amount.0).sum()
    }

    /// Inputs covering `amount` of the owner's notes, and the change note.
    fn fund(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> (Vec<(Hash, Hash)>, Vec<Output>) {
        let mut picked = Vec::new();
        let mut sum = 0i128;
        let notes: Vec<(Hash, Note)> = self.vm.live.iter().map(|(c, n)| (*c, *n)).collect();
        for (c, n) in notes {
            if sum >= amount.0 {
                break;
            }
            if n.owner == owner && n.asset == asset {
                picked.push((c, nullifier(&[owner[0]; 32], &c)));
                sum += n.amount.0;
            }
        }
        assert!(sum >= amount.0, "too little to fund it");
        let change = if sum > amount.0 {
            let nonce = self.fresh();
            vec![Output { note: Note { asset, amount: Fixed(sum - amount.0), owner, nonce }, enc: enc() }]
        } else {
            Vec::new()
        };
        (picked, change)
    }

    fn claimable(&self, owner: AccountId, asset: [u8; 32]) -> i128 {
        self.vm.market.claims.values().filter(|c| c.owner == owner && c.asset == asset).map(|c| c.amount.0).sum()
    }

    /// Alice subscribes to SHOP: `amount` every `every`, `periods` total,
    /// `fund` periods set aside now. Returns the schedule id.
    fn subscribe(&mut self, amount: Fixed, every: Every, first_due: u64, periods: Option<u32>, fund: u32) -> Hash {
        let nonce = self.fresh();
        let total = Fixed(amount.0 * fund as i128);
        let (inputs, change) = self.fund(ALICE, BOLD, total);
        self.ok(Op::PayCreate {
            payer: ALICE,
            program: program::pay_create(),
            payee: SHOP,
            asset: BOLD,
            amount,
            every,
            first_due,
            periods,
            fund_periods: fund,
            reference: [0xEF; 32],
            nonce,
            inputs,
            change,
        });
        pay::schedule_id(&ALICE, &nonce)
    }

    fn clock(&mut self, at: u64) -> Receipt {
        self.apply(Op::PayClock { program: program::pay_clock(), at })
    }

    /// Move the clock to `to` in steps the clock rule allows.
    fn clock_to(&mut self, to: u64) {
        let mut at = self.vm.market.pay.time;
        while at < to {
            at = if at == 0 { to } else { (at + pay::MAX_PAY_CLOCK_STEP_SECS).min(to) };
            assert!(!ObjectVm::rejected(&self.clock(at)));
        }
    }
}

fn whole(n: i128) -> Fixed {
    Fixed(n * 1_000_000_000_000_000_000)
}

#[test]
fn pay_ops_round_trip_and_name_their_signer() {
    let ops = [
        Op::PayCreate { payer: ALICE, program: program::pay_create(), payee: SHOP, asset: BOLD, amount: whole(10), every: Every::Months(1), first_due: NOV1, periods: Some(12), fund_periods: 3, reference: [1; 32], nonce: [2; 32], inputs: vec![([3; 32], [4; 32])], change: Vec::new() },
        Op::PayTopUp { payer: ALICE, program: program::pay_manage(), schedule: [5; 32], periods: 2, inputs: vec![([3; 32], [4; 32])], change: Vec::new() },
        Op::PayCancel { payer: ALICE, program: program::pay_manage(), schedule: [5; 32] },
        Op::PayOnce { payer: ALICE, program: program::pay_once(), payee: SHOP, asset: BOLD, amount: whole(1), reference: [1; 32], nonce: [6; 32], inputs: vec![([3; 32], [4; 32])], change: Vec::new() },
        Op::PayRefund { payee: SHOP, program: program::pay_refund(), payment: [7; 32], amount: whole(1), inputs: vec![([3; 32], [4; 32])], change: Vec::new() },
        Op::PayClock { program: program::pay_clock(), at: NOV1 },
    ];
    let signers = [vec![ALICE], vec![ALICE], vec![ALICE], vec![ALICE], vec![SHOP], vec![]];
    for (op, want) in ops.iter().zip(signers) {
        let wire = ObjectVm::encode_intent(op);
        let mut d = zyn_vm::read::Decoder::new(&wire);
        assert_eq!(ObjectVm::decode_intent(&mut d).as_ref(), Some(op));
        assert_eq!(ObjectVm::intent_authorities(op), want, "{op:?}");
    }
}

/// Mint, spend, register, seal: a history with no pay op. Its root was
/// computed by the code before Zyn Pay; it must not move.
#[test]
fn a_history_without_pay_keeps_its_root() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(5));
    assert!(matches!(c.apply(Op::Seal), Receipt::Sealed(_)));
    let root = c.vm.state_root();
    assert_eq!(hex(&root), PINNED_ROOT);
}

const PINNED_ROOT: &str = "2294e3f2275bf1c3735731c1007c277024aa7ecee520460b3527dc208a04c2dd";

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn pay_state_round_trips_and_counts_toward_conservation() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    // Put a schedule straight into state to test encoding and holdings
    // before the ops exist (Task 4 makes it through PayCreate).
    c.vm.market.pay.schedules.insert(
        [9; 32],
        pay::Schedule {
            payer: ALICE,
            payee: SHOP,
            asset: BOLD,
            amount: whole(10),
            every: Every::Months(1),
            first_due: NOV1,
            index: 0,
            remaining: Some(12),
            pot: whole(30),
            reference: [1; 32],
            state: ScheduleState::Active,
        },
    );
    c.vm.issued.insert(BOLD, whole(130));
    assert_eq!(c.vm.market.holdings(&BOLD), Some(whole(30).0));
    assert_eq!(c.vm.conserved(), Ok(()));
    let back = ObjectVm::decode(&c.vm.encode()).expect("decodes");
    assert_eq!(back, c.vm);
    assert_ne!(hex(&c.vm.state_root()), PINNED_ROOT);
}

#[test]
fn create_sets_aside_the_funded_periods() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    let id = c.subscribe(whole(10), Every::Months(1), NOV1, Some(12), 3);
    let s = c.vm.market.pay.schedules[&id];
    assert_eq!(s.pot, whole(30));
    assert_eq!(s.state, ScheduleState::Active);
    assert_eq!(c.balance(ALICE, BOLD), whole(70).0);
}

#[test]
fn create_is_refused_when_it_cannot_work() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    let nobody: AccountId = [0x0D; 32];
    let base = |c: &mut Chain, payee: AccountId, every: Every, first_due: u64, fund: u32| {
        let nonce = c.fresh();
        let (inputs, change) = c.fund(ALICE, BOLD, whole(10));
        Op::PayCreate { payer: ALICE, program: program::pay_create(), payee, asset: BOLD, amount: whole(10), every, first_due, periods: Some(1), fund_periods: fund, reference: [0; 32], nonce, inputs, change }
    };
    let op = base(&mut c, nobody, Every::Days(1), NOV1, 1);
    assert_eq!(c.refused(op), "the payee has not registered where to be paid");
    let op = base(&mut c, SHOP, Every::Days(0), NOV1, 1);
    assert_eq!(c.refused(op), "a schedule needs a period above nothing");
    let op = base(&mut c, SHOP, Every::Days(1), NOV1, 0);
    assert_eq!(c.refused(op), "fund at least one period");
    let op = base(&mut c, ALICE, Every::Days(1), NOV1, 1);
    assert_eq!(c.refused(op), "a schedule cannot pay its own payer");
}

#[test]
fn create_after_its_first_due_is_refused() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    c.clock(NOV1 + DAY);
    let nonce = c.fresh();
    let (inputs, change) = c.fund(ALICE, BOLD, whole(10));
    let op = Op::PayCreate { payer: ALICE, program: program::pay_create(), payee: SHOP, asset: BOLD, amount: whole(10), every: Every::Days(1), first_due: NOV1, periods: Some(1), fund_periods: 1, reference: [0; 32], nonce, inputs, change };
    assert_eq!(c.refused(op), "the first payment is already past");
}

#[test]
fn funding_overflow_is_refused() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(1));
    let nonce = c.fresh();
    let (inputs, change) = c.fund(ALICE, BOLD, whole(1));
    let op = Op::PayCreate { payer: ALICE, program: program::pay_create(), payee: SHOP, asset: BOLD, amount: Fixed(i128::MAX / 2), every: Every::Days(1), first_due: NOV1, periods: None, fund_periods: 3, reference: [0; 32], nonce, inputs, change };
    assert_eq!(c.refused(op), "that schedule is too large");
}

#[test]
fn top_up_adds_periods_and_cancel_returns_the_pot() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    let id = c.subscribe(whole(10), Every::Months(1), NOV1, None, 1);
    let (inputs, change) = c.fund(ALICE, BOLD, whole(20));
    c.ok(Op::PayTopUp { payer: ALICE, program: program::pay_manage(), schedule: id, periods: 2, inputs, change });
    assert_eq!(c.vm.market.pay.schedules[&id].pot, whole(30));
    // Only the payer may cancel.
    assert_eq!(c.refused(Op::PayCancel { payer: SHOP, program: program::pay_manage(), schedule: id }), "not your schedule");
    c.ok(Op::PayCancel { payer: ALICE, program: program::pay_manage(), schedule: id });
    let s = c.vm.market.pay.schedules[&id];
    assert_eq!(s.state, ScheduleState::Cancelled);
    assert_eq!(s.pot, Fixed::ZERO);
    assert_eq!(c.claimable(ALICE, BOLD), whole(30).0);
    // A cancelled schedule takes no top-up and no second cancel.
    let (inputs, change) = c.fund(ALICE, BOLD, whole(10));
    assert_eq!(c.refused(Op::PayTopUp { payer: ALICE, program: program::pay_manage(), schedule: id, periods: 1, inputs, change }), "that schedule has ended");
    assert_eq!(c.refused(Op::PayCancel { payer: ALICE, program: program::pay_manage(), schedule: id }), "that schedule has ended");
}

fn top_up(c: &mut Chain, id: Hash, periods: u32, who: AccountId) -> Op {
    let (inputs, change) = c.fund(ALICE, BOLD, whole(10 * periods as i128));
    Op::PayTopUp { payer: who, program: program::pay_manage(), schedule: id, periods, inputs, change }
}

#[test]
fn resume_after_a_gap_keeps_the_end_date() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    let id = c.subscribe(whole(10), Every::Days(1), NOV1, Some(4), 1);
    // Period 0 is paid, then the pot is empty and the next due turns it unfunded.
    c.clock_to(NOV1 + DAY);
    assert_eq!(c.vm.market.pay.schedules[&id].state, ScheduleState::Unfunded);
    assert!(c.vm.market.pay.payments.contains_key(&pay::pay_payment_id(&id, 0)));
    c.clock_to(NOV1 + 5 * DAY);
    // Periods 1 and 2 are skipped; only the last of the four remains.
    let op = top_up(&mut c, id, 2, ALICE);
    assert_eq!(c.refused(op), "fund no more periods than the schedule has");
    let op = top_up(&mut c, id, 1, ALICE);
    c.ok(op);
    let s = c.vm.market.pay.schedules[&id];
    assert_eq!((s.index, s.remaining, s.state, s.pot), (3, Some(1), ScheduleState::Active, whole(10)));
    c.clock_to(NOV1 + 6 * DAY);
    let s = c.vm.market.pay.schedules[&id];
    assert_eq!((s.state, s.pot), (ScheduleState::Ended, Fixed::ZERO));
    for k in [0u32, 3] {
        assert!(c.vm.market.pay.payments.contains_key(&pay::pay_payment_id(&id, k)), "period {k}");
    }
    for k in [1u32, 2] {
        assert!(!c.vm.market.pay.payments.contains_key(&pay::pay_payment_id(&id, k)), "period {k}");
    }
    for k in 4..8u32 {
        assert!(!c.vm.market.pay.payments.contains_key(&pay::pay_payment_id(&id, k)), "period {k}");
    }
}

#[test]
fn top_up_after_the_term_has_run_out_is_refused() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    let id = c.subscribe(whole(10), Every::Days(1), NOV1, Some(1), 1);
    c.clock_to(NOV1 + 5 * DAY);
    let before = c.vm.market.pay.schedules[&id];
    assert_eq!(before.state, ScheduleState::Ended);
    let op = top_up(&mut c, id, 1, ALICE);
    assert_eq!(c.refused(op), "that schedule has ended");
    assert_eq!(c.vm.market.pay.schedules[&id], before);
}

#[test]
fn top_up_cannot_go_past_the_term() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    let id = c.subscribe(whole(10), Every::Days(1), NOV1, Some(3), 2);
    let op = top_up(&mut c, id, 2, ALICE);
    assert_eq!(c.refused(op), "fund no more periods than the schedule has");
}

#[test]
fn create_refuses_a_term_it_cannot_fund() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    for (periods, fund) in [(Some(0), 1), (Some(2), 3)] {
        let nonce = c.fresh();
        let (inputs, change) = c.fund(ALICE, BOLD, whole(30));
        let op = Op::PayCreate { payer: ALICE, program: program::pay_create(), payee: SHOP, asset: BOLD, amount: whole(10), every: Every::Days(1), first_due: NOV1, periods, fund_periods: fund, reference: [0; 32], nonce, inputs, change };
        assert_eq!(c.refused(op), "fund no more periods than the schedule has");
    }
}

#[test]
fn unknown_schedules_and_wrong_programs_are_refused() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    let id = c.subscribe(whole(10), Every::Days(1), NOV1, None, 1);
    let ghost = [0x66; 32];
    let op = top_up(&mut c, ghost, 1, ALICE);
    assert_eq!(c.refused(op), "no such schedule");
    assert_eq!(c.refused(Op::PayCancel { payer: ALICE, program: program::pay_manage(), schedule: ghost }), "no such schedule");
    let wrong = program::pay_clock();
    let nonce = c.fresh();
    let (inputs, change) = c.fund(ALICE, BOLD, whole(10));
    let op = Op::PayCreate { payer: ALICE, program: wrong, payee: SHOP, asset: BOLD, amount: whole(10), every: Every::Days(1), first_due: NOV1, periods: None, fund_periods: 1, reference: [0; 32], nonce, inputs, change };
    assert_eq!(c.refused(op), "wrong program for a scheduled payment");
    let (inputs, change) = c.fund(ALICE, BOLD, whole(10));
    let op = Op::PayTopUp { payer: ALICE, program: wrong, schedule: id, periods: 1, inputs, change };
    assert_eq!(c.refused(op), "wrong program for a top-up");
    assert_eq!(c.refused(Op::PayCancel { payer: ALICE, program: wrong, schedule: id }), "wrong program for a cancellation");
}

fn paid_to(c: &Chain, id: &Hash, k: u32) -> pay::PaymentRecord {
    c.vm.market.pay.payments[&pay::pay_payment_id(id, k)]
}

#[test]
fn the_clock_pays_each_period_once_less_the_fee() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    let id = c.subscribe(whole(10), Every::Days(30), NOV1, Some(2), 2);
    c.clock_to(NOV1 - 1);
    assert_eq!(c.claimable(SHOP, BOLD), 0, "nothing before it is due");
    c.clock_to(NOV1);
    let fee = pay::fee_of(whole(10));
    assert_eq!(c.claimable(SHOP, BOLD), whole(10).0 - fee.0);
    assert_eq!(c.vm.market.pol.get(&BOLD).copied(), Some(fee));
    assert_eq!(paid_to(&c, &id, 0).amount, whole(10));
    // Same time again pays nothing more.
    c.clock_to(NOV1 + 60);
    assert_eq!(c.claimable(SHOP, BOLD), whole(10).0 - fee.0);
    c.clock_to(NOV1 + 30 * DAY);
    let s = c.vm.market.pay.schedules[&id];
    assert_eq!(s.state, ScheduleState::Ended);
    assert_eq!(s.pot, Fixed::ZERO);
    assert_eq!(c.claimable(SHOP, BOLD), 2 * (whole(10).0 - fee.0));
}

#[test]
fn catch_up_pays_each_missed_period_in_order() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    let id = c.subscribe(whole(1), Every::Days(1), NOV1, None, 10);
    c.clock_to(NOV1 - 1);
    // Node down four days: one post may move an hour, so catching up takes
    // many posts, each paying what has fallen due.
    c.clock_to(NOV1 + 4 * DAY);
    // One payment per missed period, each stamped with its own due date.
    for k in 0..5u32 {
        assert_eq!(paid_to(&c, &id, k).at, NOV1 + k as u64 * DAY);
        assert_eq!(paid_to(&c, &id, k).amount, whole(1));
    }
    assert!(!c.vm.market.pay.payments.contains_key(&pay::pay_payment_id(&id, 5)));
    assert_eq!(c.vm.market.pay.schedules[&id].index, 5);
    assert_eq!(c.vm.market.pay.schedules[&id].pot, whole(5));
}

#[test]
fn an_empty_pot_turns_unfunded_and_a_top_up_resumes_without_back_charges() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    let id = c.subscribe(whole(10), Every::Days(1), NOV1, None, 1);
    c.clock_to(NOV1);
    c.clock_to(NOV1 + DAY);
    assert_eq!(c.vm.market.pay.schedules[&id].state, ScheduleState::Unfunded);
    c.clock_to(NOV1 + 3 * DAY + 60);
    let (inputs, change) = c.fund(ALICE, BOLD, whole(10));
    c.ok(Op::PayTopUp { payer: ALICE, program: program::pay_manage(), schedule: id, periods: 1, inputs, change });
    c.clock_to(NOV1 + 3 * DAY + 120);
    // Periods 1 and 2 were missed while unfunded and are never charged;
    // period 3 (due NOV1 + 3 days) is paid.
    assert!(!c.vm.market.pay.payments.contains_key(&pay::pay_payment_id(&id, 1)));
    assert!(!c.vm.market.pay.payments.contains_key(&pay::pay_payment_id(&id, 2)));
    assert!(c.vm.market.pay.payments.contains_key(&pay::pay_payment_id(&id, 3)));
}

#[test]
fn at_most_256_payouts_per_tick_oldest_first() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(1_000));
    c.clock_to(NOV1 - 3600);
    let mut ids = Vec::new();
    for i in 0..300u64 {
        ids.push(c.subscribe(whole(1), Every::Days(1), NOV1 - 3600 + i, Some(1), 1));
    }
    c.clock(NOV1);
    let paid = c.vm.market.pay.payments.len();
    assert_eq!(paid, pay::MAX_DUE_PER_TICK);
    // The 256 paid are the 256 due earliest.
    assert!(ids[..256].iter().all(|id| c.vm.market.pay.payments.contains_key(&pay::pay_payment_id(id, 0))));
    c.clock(NOV1 + 1);
    assert_eq!(c.vm.market.pay.payments.len(), 300);
}

#[test]
fn the_clock_only_moves_forward_and_an_hour_at_a_time() {
    let mut c = Chain::new();
    c.clock(NOV1);
    assert_eq!(c.refused(Op::PayClock { program: program::pay_clock(), at: NOV1 - 1 }), "the pay clock cannot run backwards");
    assert_eq!(c.refused(Op::PayClock { program: program::pay_clock(), at: NOV1 + 3601 }), "the pay clock moves at most an hour per post");
}

#[test]
fn fee_rounds_down_to_nothing_on_dust() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, Fixed(1_000));
    c.subscribe(Fixed(100), Every::Days(1), NOV1, Some(1), 1);
    c.clock_to(NOV1);
    assert_eq!(c.claimable(SHOP, BOLD), 100);
    assert_eq!(c.vm.market.pol.get(&BOLD).copied().unwrap_or(Fixed::ZERO), Fixed::ZERO);
}

fn pay_once(c: &mut Chain, amount: Fixed) -> Hash {
    let nonce = c.fresh();
    let (inputs, change) = c.fund(ALICE, BOLD, amount);
    c.ok(Op::PayOnce { payer: ALICE, program: program::pay_once(), payee: SHOP, asset: BOLD, amount, reference: [3; 32], nonce, inputs, change });
    pay::pay_once_id(&ALICE, &nonce)
}

fn refund(c: &mut Chain, payment: Hash, amount: Fixed) -> Receipt {
    c.mint(SHOP, BOLD, amount);
    let (inputs, change) = c.fund(SHOP, BOLD, amount);
    c.apply(Op::PayRefund { payee: SHOP, program: program::pay_refund(), payment, amount, inputs, change })
}

#[test]
fn a_one_off_payment_pays_the_payee_less_the_fee() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(50));
    let id = pay_once(&mut c, whole(20));
    let fee = pay::fee_of(whole(20));
    assert_eq!(c.claimable(SHOP, BOLD), whole(20).0 - fee.0);
    assert_eq!(c.vm.market.pay.payments[&id].payer, ALICE);
}

#[test]
fn a_refund_goes_back_to_the_payer_unnamed() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(50));
    let id = pay_once(&mut c, whole(20));
    assert!(!ObjectVm::rejected(&refund(&mut c, id, whole(5))));
    assert_eq!(c.claimable(ALICE, BOLD), whole(5).0);
    assert_eq!(c.vm.market.pay.payments[&id].refunded, whole(5));
}

#[test]
fn partial_refunds_cannot_exceed_the_payment() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(50));
    let sid = c.subscribe(whole(10), Every::Days(1), NOV1, None, 2);
    c.clock_to(NOV1);
    let payment = pay::pay_payment_id(&sid, 0);
    c.ok(Op::PayCancel { payer: ALICE, program: program::pay_manage(), schedule: sid });
    // A refund after cancelling still works.
    assert!(!ObjectVm::rejected(&refund(&mut c, payment, whole(6))));
    match refund(&mut c, payment, whole(5)) {
        Receipt::Refused(why) => assert_eq!(why, "more than is left to refund"),
        other => panic!("{other:?}"),
    }
    assert!(!ObjectVm::rejected(&refund(&mut c, payment, whole(4))));
}

#[test]
fn only_the_payee_refunds() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(50));
    let id = pay_once(&mut c, whole(20));
    let (inputs, change) = c.fund(ALICE, BOLD, whole(1));
    assert_eq!(c.refused(Op::PayRefund { payee: ALICE, program: program::pay_refund(), payment: id, amount: whole(1), inputs, change }), "only the payee refunds a payment");
}

/// Applies `op`, expects the refusal `why`, and checks that nothing but the
/// intent bookkeeping (sequence, accumulator, count) moved: the encoded state
/// is otherwise byte-identical.
fn refused_leaves_state(c: &mut Chain, op: &Op, why: &str) {
    let mut expected = c.vm.clone();
    c.seq += 1;
    match c.vm.apply(c.seq, op).into_iter().next().unwrap() {
        Receipt::Refused(got) => assert_eq!(got, why),
        other => panic!("{other:?}"),
    }
    expected.seq = c.vm.seq;
    expected.intent_acc = c.vm.intent_acc;
    expected.epoch_intents = c.vm.epoch_intents;
    assert_eq!(c.vm.encode(), expected.encode(), "a refused op leaves state byte-identical");
}

#[test]
fn a_one_off_payment_refused_for_fee_overflow_changes_nothing() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(50));
    c.vm.market.pol.insert(BOLD, Fixed(i128::MAX));
    let nonce = c.fresh();
    let (inputs, change) = c.fund(ALICE, BOLD, whole(20));
    let op = Op::PayOnce { payer: ALICE, program: program::pay_once(), payee: SHOP, asset: BOLD, amount: whole(20), reference: [3; 32], nonce, inputs, change };
    refused_leaves_state(&mut c, &op, "fee overflow");
}

#[test]
fn a_clock_tick_refused_for_fee_overflow_changes_nothing() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    c.subscribe(whole(10), Every::Days(1), NOV1, None, 2);
    c.clock_to(NOV1 - 1);
    c.vm.market.pol.insert(BOLD, Fixed(i128::MAX));
    let op = Op::PayClock { program: program::pay_clock(), at: NOV1 };
    refused_leaves_state(&mut c, &op, "fee overflow");
}
