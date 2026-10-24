# Zyn Pay — Plan 1: ledger module and node — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scheduled payments, one-off payments and refunds as object-ledger rules (`objectvm/src/pay.rs`), with the node posting the pay clock and serving read-by-id RPC.

**Architecture:** A new `pay` module inside the object ledger's `Market` state, built the way Zyn Lend was added: new ops (codes 69–74), funding through `plan_funding`/`apply_funding`, payouts as market claims the payee collects with the existing `Claim` op, fees to the ZYN pot (`market.to_pol`), a state section joined only once non-empty and a `MARKET19` encoding only once used, so every root sealed before keeps. The operator posts `PayClock` like `LendTime`.

**Tech Stack:** Rust (objectvm, zyn-vm, zynzapd), cargo tests.

**Spec:** `docs/superpowers/specs/2026-10-23-zyn-scheduled-payments-design.md`

**This plan covers** the spec's "Ledger changes" and the node. Plan 2 (Nap: consent cards, Payments list, Nap Connect methods) and Plan 3 (`@zyn-io/pay` merchant package and `zyn-pay serve`) are written once this lands, against its real ops and RPC.

## Global Constraints

- Op codes 69–74, in this order: `PayCreate` 69, `PayTopUp` 70, `PayCancel` 71, `PayOnce` 72, `PayRefund` 73, `PayClock` 74.
- Programs: `pay-create`, `pay-manage` (top-up, cancel), `pay-once`, `pay-refund`, `pay-clock` — added to `zyn_vm::program` and to `is_named`.
- Fee: `PAY_FEE_BPS = 50` (0.5%) of each scheduled or one-off payment, taken from the payee's side, rounded down, to the ZYN pot (`market.to_pol`). Refunds carry no fee and do not return it.
- `MAX_DUE_PER_TICK = 256` payouts per `PayClock`, oldest due first; the rest at the next tick, never skipped.
- Clock rule (follows lending's §179.6, replacing the spec's "120 s ahead of the epoch", because the ledger has no epoch timestamp): forward only; at most `MAX_PAY_CLOCK_STEP_SECS = 3600` per post; the first post sets it.
- Months follow the UTC calendar: period `k` is due at `first_due` plus `k × n` months, clamped to the month's last day, same time of day — computed from `first_due` each time, so a 31st schedule pays 28 Feb then 31 Mar, never drifting.
- Nothing is partly paid. A due schedule whose pot holds less than one period becomes `Unfunded`.
- A history with no pay op must reach exactly the roots it reached before this change (testnet 212 replays from genesis).
- Payout matching uses ids, not new claim fields (deviation from the spec's "claims gain a reference", which would change the claim encoding and every old root): a payment's id is `pay_payment_id(schedule, period)` or `pay_once_id(payer, nonce)`, its payee claim is `claim_id(payment_id, 0)`, and the schedule's `reference` is read through `zyn_paySchedules`.
- Copy in errors and RPC: plain sentences, as the rest of objectvm.

## Review Focus

1. **Node down for days, daily schedule** — every missed period is paid in order once the clock catches up (one payout per period), bounded per tick, not one lump and not skipped. Test: `catch_up_pays_each_missed_period_in_order` (Task 5).
2. **Month ends and leap years** — a schedule on 31 Jan pays 28 Feb (29 Feb in 2028), 31 Mar, 30 Apr. Test: `month_schedule_clamps_without_drift` (Task 1).
3. **Refunding more than was paid, in pieces** — two partial refunds whose sum passes the payment are refused at the second; a refund after the schedule was cancelled still works. Test: `partial_refunds_cannot_exceed_the_payment` (Task 6).
4. **Tiny amounts** — a payment whose fee rounds to zero pays the payee in full and the pot nothing; conservation holds. Test: `fee_rounds_down_to_nothing_on_dust` (Task 5).
5. **Overflow** — `amount × fund_periods` that overflows i128 is refused, not wrapped. Test: `funding_overflow_is_refused` (Task 4).

---

## File Structure

- Create `objectvm/src/pay.rs` — pay state (`Pay`, `Schedule`, `Every`, `ScheduleState`, `PaymentRecord`), ids, calendar math, the state section, encode/decode, holdings, and the `impl ObjectVm` apply functions for the six ops.
- Modify `objectvm/src/lib.rs` — `pub mod pay;`, the six `Op` variants, `Receipt::Paid`, `spent`/`created` plumbing, apply dispatch, `intent_authorities`, `encode_intent`/`decode_intent`, `typed_intent` for pay ops.
- Modify `objectvm/src/market.rs` — `pub pay: crate::pay::Pay` on `Market`, `is_empty`, `holdings`, `leaf` section, `MARKET19` encode/decode, `fresh_source` check.
- Modify `zyn-vm/src/program.rs` — five program ids and `is_named`.
- Create `objectvm/tests/pay.rs` — behaviour tests through `apply`.
- Modify `zynzapd/src/bin/zyn-object-node.rs` — pay clock thread, `zyn_paySchedules`, `zyn_payPayments`.
- Modify `DECISIONS.md`, `RUNBOOK.md`, `docs/MESSAGING.md` — the decision record, operating notes, approved claims.

---

### Task 1: Pay state, ids and calendar math

**Files:**
- Create: `objectvm/src/pay.rs`
- Modify: `objectvm/src/lib.rs` (add `pub mod pay;` beside `pub mod lend;`)
- Modify: `zyn-vm/src/program.rs`
- Test: unit tests at the bottom of `objectvm/src/pay.rs`

**Interfaces:**
- Produces:
  - `pub enum Every { Days(u32), Months(u32) }`
  - `pub enum ScheduleState { Active, Unfunded, Ended, Cancelled }`
  - `pub struct Schedule { payer, payee: AccountId, asset: Address, amount: Fixed, every: Every, first_due: u64, index: u32, remaining: Option<u32>, pot: Fixed, reference: Hash, state: ScheduleState }`
  - `pub struct PaymentRecord { payer, payee: AccountId, asset: Address, amount: Fixed, fee: Fixed, refunded: Fixed, refunds: u32, at: u64 }`
  - `pub struct Pay { pub time: u64, pub schedules: BTreeMap<Hash, Schedule>, pub payments: BTreeMap<Hash, PaymentRecord> }` with `is_empty()`
  - `pub fn schedule_id(payer: &AccountId, nonce: &Hash) -> Hash`
  - `pub fn pay_payment_id(schedule: &Hash, period: u32) -> Hash`
  - `pub fn pay_once_id(payer: &AccountId, nonce: &Hash) -> Hash`
  - `pub fn refund_source(payment: &Hash, n: u32) -> Hash`
  - `pub fn cancel_source(schedule: &Hash) -> Hash`
  - `pub fn due_at(first_due: u64, every: Every, k: u32) -> Option<u64>`
  - `pub fn fee_of(amount: Fixed) -> Fixed`
  - constants `PAY_FEE_BPS: i128 = 50`, `MAX_DUE_PER_TICK: usize = 256`, `MAX_PAY_CLOCK_STEP_SECS: u64 = 3600`
  - programs `pay_create()`, `pay_manage()`, `pay_once()`, `pay_refund()`, `pay_clock()`

- [ ] **Step 1: Add the program ids**

In `zyn-vm/src/program.rs`, after `lend_insure()`:

```rust
/// Start a scheduled payment, funded at approval (Zyn Pay).
pub fn pay_create() -> ProgramId {
    named(b"pay-create")
}

/// Top up or cancel a scheduled payment (Zyn Pay).
pub fn pay_manage() -> ProgramId {
    named(b"pay-manage")
}

/// A one-off payment with a refundable record (Zyn Pay).
pub fn pay_once() -> ProgramId {
    named(b"pay-once")
}

/// A payee's refund of a payment it received (Zyn Pay).
pub fn pay_refund() -> ProgramId {
    named(b"pay-refund")
}

/// The pay clock, posted by the operator (Zyn Pay).
pub fn pay_clock() -> ProgramId {
    named(b"pay-clock")
}
```

and add `pay_create(), pay_manage(), pay_once(), pay_refund(), pay_clock(),` to the array in `is_named`.

- [ ] **Step 2: Write the failing calendar and fee tests**

Create `objectvm/src/pay.rs` with only the tests module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // 2026-01-31 12:00:00 UTC
    const JAN31: u64 = 1_769_860_800;

    fn ymd(secs: u64) -> (i64, u32, u32) {
        let days = (secs / 86_400) as i64;
        civil_from_days(days)
    }

    #[test]
    fn month_schedule_clamps_without_drift() {
        let every = Every::Months(1);
        assert_eq!(ymd(due_at(JAN31, every, 0).unwrap()), (2026, 1, 31));
        assert_eq!(ymd(due_at(JAN31, every, 1).unwrap()), (2026, 2, 28));
        assert_eq!(ymd(due_at(JAN31, every, 2).unwrap()), (2026, 3, 31));
        assert_eq!(ymd(due_at(JAN31, every, 3).unwrap()), (2026, 4, 30));
        // 2028 is a leap year.
        assert_eq!(ymd(due_at(JAN31, every, 25).unwrap()), (2028, 2, 29));
        // Time of day is kept.
        assert_eq!(due_at(JAN31, every, 1).unwrap() % 86_400, JAN31 % 86_400);
    }

    #[test]
    fn day_schedule_steps_exactly() {
        assert_eq!(due_at(JAN31, Every::Days(7), 3), Some(JAN31 + 3 * 7 * 86_400));
    }

    #[test]
    fn zero_period_has_no_due_dates() {
        assert_eq!(due_at(JAN31, Every::Days(0), 1), None);
        assert_eq!(due_at(JAN31, Every::Months(0), 1), None);
    }

    #[test]
    fn fee_is_half_a_percent_rounded_down() {
        assert_eq!(fee_of(Fixed(10_000)), Fixed(50));
        assert_eq!(fee_of(Fixed(199)), Fixed(0));
        assert_eq!(fee_of(Fixed(0)), Fixed(0));
    }

    #[test]
    fn ids_are_distinct_per_domain() {
        let a = [1u8; 32];
        let n = [2u8; 32];
        assert_ne!(schedule_id(&a, &n), pay_once_id(&a, &n));
        assert_ne!(pay_payment_id(&schedule_id(&a, &n), 0), pay_payment_id(&schedule_id(&a, &n), 1));
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p objectvm --lib pay::tests`
Expected: compile errors (`due_at`, `Every`, `fee_of`, … not found).

- [ ] **Step 4: Write the state, ids and calendar math above the tests**

```rust
//! Zyn Pay: scheduled payments funded at approval, one-off payments, and
//! refunds that go back to the payer without the payee learning who paid
//! (spec 2026-10-23-zyn-scheduled-payments-design.md).
//!
//! Value on this ledger lives in notes, and spending a note needs a nullifier
//! only its owner can derive. So the ledger cannot take money from a payer
//! later: approval moves the funded periods out of the payer's notes into the
//! schedule's pot, and the pay clock pays out of the pot.

use std::collections::BTreeMap;

use zyn_vm::commit::{Encoder, Hash};
use zyn_vm::derive::Address;
use zyn_vm::spec::AccountId;
use zyn_vm::Fixed;

/// 0.5% of each payment, from the payee's side, to the ZYN pot.
pub const PAY_FEE_BPS: i128 = 50;
/// Most payouts one `PayClock` makes; the rest wait for the next (S5).
pub const MAX_DUE_PER_TICK: usize = 256;
/// One post moves the pay clock at most an hour (as lending's, §179.6).
pub const MAX_PAY_CLOCK_STEP_SECS: u64 = 3600;

const DAY: u64 = 86_400;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Every {
    Days(u32),
    Months(u32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScheduleState {
    Active,
    Unfunded,
    Ended,
    Cancelled,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Schedule {
    pub payer: AccountId,
    pub payee: AccountId,
    pub asset: Address,
    /// Per period.
    pub amount: Fixed,
    pub every: Every,
    pub first_due: u64,
    /// Periods paid so far; the next due is `due_at(first_due, every, index)`.
    pub index: u32,
    /// Periods still to pay; `None` runs until cancelled.
    pub remaining: Option<u32>,
    /// Set aside and not yet paid.
    pub pot: Fixed,
    /// The payee's own id for this subscription.
    pub reference: Hash,
    pub state: ScheduleState,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PaymentRecord {
    pub payer: AccountId,
    pub payee: AccountId,
    pub asset: Address,
    /// What the payer paid, fee included.
    pub amount: Fixed,
    pub fee: Fixed,
    pub refunded: Fixed,
    pub refunds: u32,
    /// Pay-clock time it was made.
    pub at: u64,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Pay {
    pub time: u64,
    pub schedules: BTreeMap<Hash, Schedule>,
    pub payments: BTreeMap<Hash, PaymentRecord>,
}

impl Pay {
    pub fn is_empty(&self) -> bool {
        self.time == 0 && self.schedules.is_empty() && self.payments.is_empty()
    }
}

fn id(domain: &[u8], parts: &[&[u8]]) -> Hash {
    let mut e = Encoder::new();
    e.bytes(domain);
    for p in parts {
        e.bytes(p);
    }
    e.leaf()
}

pub fn schedule_id(payer: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.pay.schedule.v1", &[payer, nonce])
}

pub fn pay_payment_id(schedule: &Hash, period: u32) -> Hash {
    id(b"objectvm.pay.payment.v1", &[schedule, &period.to_be_bytes()])
}

pub fn pay_once_id(payer: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.pay.once.v1", &[payer, nonce])
}

pub fn refund_source(payment: &Hash, n: u32) -> Hash {
    id(b"objectvm.pay.refund.v1", &[payment, &n.to_be_bytes()])
}

pub fn cancel_source(schedule: &Hash) -> Hash {
    id(b"objectvm.pay.cancel.v1", &[schedule])
}

pub fn fee_of(amount: Fixed) -> Fixed {
    Fixed(amount.0.saturating_mul(PAY_FEE_BPS) / 10_000)
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month, day) → days since 1970-01-01.
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = m as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// When period `k` (from 0) is due. `None` for a zero period or overflow.
pub fn due_at(first_due: u64, every: Every, k: u32) -> Option<u64> {
    match every {
        Every::Days(0) | Every::Months(0) => None,
        Every::Days(n) => first_due.checked_add((n as u64).checked_mul(k as u64)?.checked_mul(DAY)?),
        Every::Months(n) => {
            let (y, m, d) = civil_from_days((first_due / DAY) as i64);
            let months = (m as i64 - 1).checked_add((n as i64).checked_mul(k as i64)?)?;
            let ny = y.checked_add(months.div_euclid(12))?;
            let nm = (months.rem_euclid(12) + 1) as u32;
            let nd = d.min(days_in_month(ny, nm));
            let day = days_from_civil(ny, nm, nd);
            u64::try_from(day).ok()?.checked_mul(DAY)?.checked_add(first_due % DAY)
        }
    }
}
```

and in `objectvm/src/lib.rs`, after `pub mod lend;`, add `pub mod pay;`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p objectvm --lib pay::tests`
Expected: 5 passed.

- [ ] **Step 6: Commit**

```bash
git add objectvm/src/pay.rs objectvm/src/lib.rs zyn-vm/src/program.rs
git commit -m "Zyn Pay: pay state, ids, UTC month calendar and the 0.5% fee (objectvm/src/pay.rs)"
```

---

### Task 2: The six ops on the wire, authorities and receipts

**Files:**
- Modify: `objectvm/src/lib.rs` (the `Op` enum after `LendBid`; `Receipt` before `Refused`; `spent()`, `created()`; `intent_authorities`; `encode_intent`; `decode_intent`)
- Test: `objectvm/tests/pay.rs` (create it)

**Interfaces:**
- Consumes: `Every`, `pay_*` programs (Task 1).
- Produces:
  - `Op::PayCreate { payer: AccountId, program: ProgramId, payee: AccountId, asset: Address, amount: Fixed, every: Every, first_due: u64, periods: Option<u32>, fund_periods: u32, reference: Hash, nonce: Hash, inputs: Vec<(Hash, Hash)>, change: Vec<Output> }`
  - `Op::PayTopUp { payer: AccountId, program: ProgramId, schedule: Hash, periods: u32, inputs: Vec<(Hash, Hash)>, change: Vec<Output> }`
  - `Op::PayCancel { payer: AccountId, program: ProgramId, schedule: Hash }`
  - `Op::PayOnce { payer: AccountId, program: ProgramId, payee: AccountId, asset: Address, amount: Fixed, reference: Hash, nonce: Hash, inputs: Vec<(Hash, Hash)>, change: Vec<Output> }`
  - `Op::PayRefund { payee: AccountId, program: ProgramId, payment: Hash, amount: Fixed, inputs: Vec<(Hash, Hash)>, change: Vec<Output> }`
  - `Op::PayClock { program: ProgramId, at: u64 }`
  - `Receipt::Paid { id: Hash }`
  - In `objectvm/tests/pay.rs`: the `Chain` test harness used by Tasks 4–7.

- [ ] **Step 1: Write the failing round-trip and authority test**

Create `objectvm/tests/pay.rs`:

```rust
//! Zyn Pay (spec 2026-10-23-zyn-scheduled-payments-design.md): scheduled
//! payments funded at approval, one-off payments, refunds. Every intent goes
//! through `apply`, and after each one the books must still balance.

use objectvm::pay::{self, Every, ScheduleState};
use objectvm::{market, Limits, ObjectVm, Op, Output, Receipt};
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
            c.ok(Op::Register { account: who, program: program::register(), viewing: viewing_public(&StaticSecret::from([who[0]; 32])).to_bytes() });
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
```

Note: if `program::register()` is named differently in `zyn_vm::program`, use the program the existing tests pass to `Op::Register` (see `objectvm/tests/end_to_end.rs`), and if `ViewingKey` is built differently there, copy that construction.

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p objectvm --test pay pay_ops_round_trip_and_name_their_signer`
Expected: compile errors (`Op::PayCreate` not found).

- [ ] **Step 3: Add the variants, receipt and plumbing**

In `objectvm/src/lib.rs`, add `use crate::pay::Every;` near the other `use` lines, then in `pub enum Op` after `LendBid { … },`:

```rust
    /// Zyn Pay: start a scheduled payment. Moves `amount × fund_periods` out
    /// of the payer's notes into the schedule's pot; the pay clock pays the
    /// payee from it on each due date.
    PayCreate {
        payer: AccountId,
        program: ProgramId,
        payee: AccountId,
        asset: Address,
        amount: Fixed,
        every: Every,
        first_due: u64,
        /// Total periods; `None` runs until cancelled.
        periods: Option<u32>,
        fund_periods: u32,
        reference: Hash,
        nonce: Hash,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Zyn Pay: add `periods` more to a schedule's pot.
    PayTopUp {
        payer: AccountId,
        program: ProgramId,
        schedule: Hash,
        periods: u32,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Zyn Pay: end a schedule now; its pot comes back to the payer.
    PayCancel {
        payer: AccountId,
        program: ProgramId,
        schedule: Hash,
    },
    /// Zyn Pay: a one-off payment the payee can refund.
    PayOnce {
        payer: AccountId,
        program: ProgramId,
        payee: AccountId,
        asset: Address,
        amount: Fixed,
        reference: Hash,
        nonce: Hash,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Zyn Pay: the payee pays back up to what a payment brought it; the
    /// ledger routes it to the payer, whom the payee never names.
    PayRefund {
        payee: AccountId,
        program: ProgramId,
        payment: Hash,
        amount: Fixed,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Zyn Pay: the operator's pay clock. Pays what is due. Operator only.
    PayClock {
        program: ProgramId,
        at: u64,
    },
```

In `pub enum Receipt`, before `Refused(&'static str),`:

```rust
    /// A Zyn Pay intent applied; `id` is the schedule or payment it touched.
    Paid { id: Hash },
```

In `Op::spent()`, add to the `inputs` arm list:

```rust
            | Op::PayCreate { inputs, .. }
            | Op::PayTopUp { inputs, .. }
            | Op::PayOnce { inputs, .. }
            | Op::PayRefund { inputs, .. }
```

In `Op::created()`, add to the `change` arm list:

```rust
            | Op::PayCreate { change, .. }
            | Op::PayTopUp { change, .. }
            | Op::PayOnce { change, .. }
            | Op::PayRefund { change, .. }
```

In `intent_authorities`, before the operator-only comment:

```rust
            Op::PayCreate { payer, .. } | Op::PayTopUp { payer, .. } | Op::PayCancel { payer, .. } | Op::PayOnce { payer, .. } => vec![*payer],
            Op::PayRefund { payee, .. } => vec![*payee],
            // PayClock stays operator-only, like LendTime.
```

In `encode_intent`, before `Op::Seal =>`:

```rust
            Op::PayCreate { payer, program, payee, asset, amount, every, first_due, periods, fund_periods, reference, nonce, inputs, change } => {
                e.u8(69).bytes(payer).bytes(&program.0).bytes(payee).bytes(asset).fixed(*amount);
                pay::encode_every(&mut e, *every);
                e.u64(*first_due);
                match periods {
                    None => e.u8(0),
                    Some(n) => e.u8(1).u32(*n),
                };
                e.u32(*fund_periods).bytes(reference).bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::PayTopUp { payer, program, schedule, periods, inputs, change } => {
                e.u8(70).bytes(payer).bytes(&program.0).bytes(schedule).u32(*periods);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::PayCancel { payer, program, schedule } => {
                e.u8(71).bytes(payer).bytes(&program.0).bytes(schedule);
            }
            Op::PayOnce { payer, program, payee, asset, amount, reference, nonce, inputs, change } => {
                e.u8(72).bytes(payer).bytes(&program.0).bytes(payee).bytes(asset).fixed(*amount).bytes(reference).bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::PayRefund { payee, program, payment, amount, inputs, change } => {
                e.u8(73).bytes(payee).bytes(&program.0).bytes(payment).fixed(*amount);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::PayClock { program, at } => {
                e.u8(74).bytes(&program.0).u64(*at);
            }
```

In `decode_intent`, before `_ => None,`:

```rust
            69 => Some(Op::PayCreate {
                payer: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                payee: d.account().ok()?,
                asset: d.hash().ok()?,
                amount: d.fixed().ok()?,
                every: pay::decode_every(d)?,
                first_due: d.u64().ok()?,
                periods: match d.u8().ok()? {
                    0 => None,
                    1 => Some(d.u32().ok()?),
                    _ => return None,
                },
                fund_periods: d.u32().ok()?,
                reference: d.hash().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            70 => Some(Op::PayTopUp {
                payer: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                schedule: d.hash().ok()?,
                periods: d.u32().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            71 => Some(Op::PayCancel { payer: d.account().ok()?, program: ProgramId(d.hash().ok()?), schedule: d.hash().ok()? }),
            72 => Some(Op::PayOnce {
                payer: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                payee: d.account().ok()?,
                asset: d.hash().ok()?,
                amount: d.fixed().ok()?,
                reference: d.hash().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            73 => Some(Op::PayRefund {
                payee: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                payment: d.hash().ok()?,
                amount: d.fixed().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            74 => Some(Op::PayClock { program: ProgramId(d.hash().ok()?), at: d.u64().ok()? }),
```

In `objectvm/src/pay.rs` (above the tests), add the `Every` wire helpers:

```rust
pub(crate) fn encode_every(e: &mut Encoder, every: Every) {
    match every {
        Every::Days(n) => e.u8(0).u32(n),
        Every::Months(n) => e.u8(1).u32(n),
    };
}

pub(crate) fn decode_every(d: &mut zyn_vm::read::Decoder) -> Option<Every> {
    match d.u8().ok()? {
        0 => Some(Every::Days(d.u32().ok()?)),
        1 => Some(Every::Months(d.u32().ok()?)),
        _ => None,
    }
}
```

In the apply dispatch (`match op` that holds `Op::LendInsure { … } => one(…)`), add temporary refusals so the crate compiles; Tasks 4–6 replace them:

```rust
            Op::PayCreate { .. } | Op::PayTopUp { .. } | Op::PayCancel { .. } | Op::PayOnce { .. } | Op::PayRefund { .. } | Op::PayClock { .. } => {
                one(Err("Zyn Pay is not switched on in this build"))
            }
```

If the dispatch's `one(...)` helper takes a `Result<Receipt, &'static str>`, this compiles as written; match the surrounding arms if it differs.

- [ ] **Step 4: Run the test**

Run: `cargo test -p objectvm --test pay pay_ops_round_trip_and_name_their_signer`
Expected: PASS.

- [ ] **Step 5: Run the whole objectvm suite to confirm nothing else moved**

Run: `cargo test -p objectvm`
Expected: all existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add objectvm/src/lib.rs objectvm/src/pay.rs objectvm/tests/pay.rs
git commit -m "Zyn Pay: six ops on the wire (codes 69-74), their signers, Receipt::Paid"
```

---

### Task 3: Pay state in the market — holdings, root section, MARKET19

**Files:**
- Modify: `objectvm/src/market.rs` (`Market` struct, `is_empty`, `holdings`, `leaf`, `encode`, `decode`)
- Modify: `objectvm/src/pay.rs` (section, encode, decode, holdings)
- Modify: `objectvm/src/market.rs:1430-1445` (`fresh_source`)
- Test: `objectvm/tests/pay.rs`

**Interfaces:**
- Consumes: `Pay`, `Schedule`, `PaymentRecord` (Task 1).
- Produces: `Market.pay: crate::pay::Pay`; `pay::section(&mut Encoder, &Pay)`, `pay::encode(&mut Encoder, &Pay)`, `pay::decode(&mut Decoder) -> Option<Pay>`, `Pay::holdings(&self, asset) -> Option<i128>`, `pay::source_in_use(&Pay, &Hash) -> bool`.

- [ ] **Step 1: Pin the root of a history with no pay op, on the code as it is**

Add to `objectvm/tests/pay.rs`:

```rust
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

const PINNED_ROOT: &str = "";

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
```

Run on the tree **before** making any change in this task: `git stash` your uncommitted work if any, then
`cargo test -p objectvm --test pay a_history_without_pay_keeps_its_root -- --nocapture`
Expected: FAIL with `left: "<64 hex chars>" right: ""`. Copy the left value into `PINNED_ROOT`. Run again: PASS. (Task 2's code is committed and adds no state, so the root it prints is the pre-Zyn-Pay root.)

- [ ] **Step 2: Write the failing state tests**

```rust
#[test]
fn pay_state_round_trips_and_counts_toward_conservation() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    // Put a schedule straight into state to test encoding and holdings
    // before the ops exist (Task 4 makes it through PayCreate).
    c.vm.market.pay.schedules.insert([9; 32], pay::Schedule {
        payer: ALICE, payee: SHOP, asset: BOLD, amount: whole(10), every: Every::Months(1),
        first_due: NOV1, index: 0, remaining: Some(12), pot: whole(30), reference: [1; 32], state: ScheduleState::Active,
    });
    c.vm.issued.insert(BOLD, whole(130));
    assert_eq!(c.vm.market.holdings(&BOLD), Some(whole(30).0));
    assert_eq!(c.vm.conserved(), Ok(()));
    let back = ObjectVm::decode(&c.vm.encode()).expect("decodes");
    assert_eq!(back, c.vm);
    assert_ne!(hex(&c.vm.state_root()), PINNED_ROOT);
}
```

If `issued` or `conserved` is not public, use the visibility `objectvm/tests/fees.rs` uses for the same checks (it asserts conservation after each intent through `MicrochainVm`).

- [ ] **Step 3: Run it to see it fail**

Run: `cargo test -p objectvm --test pay pay_state_round_trips`
Expected: compile error, `no field pay on Market`.

- [ ] **Step 4: Add the state to `Market` and to `pay.rs`**

In `objectvm/src/market.rs`, in `pub struct Market`, after `pub lend: crate::lend::Lend,`:

```rust
    /// Zyn Pay: schedules, their pots, and refundable payment records.
    pub pay: crate::pay::Pay,
```

In `Market::is_empty`, add `&& self.pay.is_empty()` beside `&& self.lend.is_empty()`.

In `Market::holdings`, after `t = t.checked_add(self.lend.holdings(asset)?)?;`:

```rust
        t = t.checked_add(self.pay.holdings(asset)?)?;
```

In `Market::leaf`, after the open-mint terms block and before `e.leaf()`:

```rust
        // And Zyn Pay, once a schedule, payment or clock exists, so every
        // root sealed before keeps.
        if !self.pay.is_empty() {
            crate::pay::section(&mut e, &self.pay);
        }
```

In `Market::encode`, change the version ladder: add `let v19 = !self.pay.is_empty();` above `let v18`, make `let v18 = v19 || !self.open_terms.is_empty();`, add `if v19 { b"MARKET19" } else ` at the front of the tag chain, and after the `if v18 { … }` block:

```rust
        // MARKET19 (Zyn Pay) once pay has state: it carries MARKET18's tail too.
        if v19 {
            crate::pay::encode(e, &self.pay);
        }
```

In `Market::decode`, add `let v19 = tag == b"MARKET19";`, make `let v18 = v19 || tag == b"MARKET18";`, make sure the `match tag` that sets `(v3 … v10)` accepts `b"MARKET19"` wherever it accepts `b"MARKET18"`, and at the end, after the `if v18 { … }` block:

```rust
        if v19 {
            m.pay = crate::pay::decode(d)?;
        }
```

In `fresh_source` (market.rs), add to the `||` chain:

```rust
            || crate::pay::source_in_use(&self.market.pay, source)
```

In `objectvm/src/pay.rs`, above the tests:

```rust
impl Pay {
    /// Held for somebody: every schedule's pot. Payments already made are
    /// claims, counted by the market; records hold no value.
    pub fn holdings(&self, asset: &Address) -> Option<i128> {
        let mut t: i128 = 0;
        for s in self.schedules.values() {
            if &s.asset == asset {
                t = t.checked_add(s.pot.0)?;
            }
        }
        Some(t)
    }
}

/// Ids a pay op made, so no other intent reuses one as a claim source.
pub(crate) fn source_in_use(p: &Pay, source: &Hash) -> bool {
    p.schedules.contains_key(source) || p.payments.contains_key(source)
}

fn state_byte(s: ScheduleState) -> u8 {
    match s {
        ScheduleState::Active => 0,
        ScheduleState::Unfunded => 1,
        ScheduleState::Ended => 2,
        ScheduleState::Cancelled => 3,
    }
}

fn state_from(b: u8) -> Option<ScheduleState> {
    Some(match b {
        0 => ScheduleState::Active,
        1 => ScheduleState::Unfunded,
        2 => ScheduleState::Ended,
        3 => ScheduleState::Cancelled,
        _ => return None,
    })
}

fn schedule_bytes(e: &mut Encoder, id: &Hash, s: &Schedule) {
    e.bytes(id).bytes(&s.payer).bytes(&s.payee).bytes(&s.asset).fixed(s.amount);
    encode_every(e, s.every);
    e.u64(s.first_due).u32(s.index);
    match s.remaining {
        None => e.u8(0),
        Some(n) => e.u8(1).u32(n),
    };
    e.fixed(s.pot).bytes(&s.reference).u8(state_byte(s.state));
}

fn payment_bytes(e: &mut Encoder, id: &Hash, p: &PaymentRecord) {
    e.bytes(id).bytes(&p.payer).bytes(&p.payee).bytes(&p.asset).fixed(p.amount).fixed(p.fee).fixed(p.refunded).u32(p.refunds).u64(p.at);
}

/// The pay section of the market leaf: the clock in the clear, schedules and
/// payments as Merkle roots of their leaves. Ids carry the payer's nonce, so
/// a leaf cannot be recomputed from a guess at who paid whom.
pub(crate) fn section(e: &mut Encoder, p: &Pay) {
    let leaf = |f: &dyn Fn(&mut Encoder)| {
        let mut x = Encoder::new();
        f(&mut x);
        x.leaf()
    };
    let s: Vec<Hash> = p.schedules.iter().map(|(id, s)| leaf(&|x| schedule_bytes(x, id, s))).collect();
    let r: Vec<Hash> = p.payments.iter().map(|(id, r)| leaf(&|x| payment_bytes(x, id, r))).collect();
    e.bytes(b"objectvm.pay.v1")
        .u64(p.time)
        .u32(s.len() as u32)
        .bytes(&zyn_vm::commit::merkle_root(&s))
        .u32(r.len() as u32)
        .bytes(&zyn_vm::commit::merkle_root(&r));
}

pub(crate) fn encode(e: &mut Encoder, p: &Pay) {
    e.u64(p.time).u32(p.schedules.len() as u32);
    for (id, s) in &p.schedules {
        schedule_bytes(e, id, s);
    }
    e.u32(p.payments.len() as u32);
    for (id, r) in &p.payments {
        payment_bytes(e, id, r);
    }
}

pub(crate) fn decode(d: &mut zyn_vm::read::Decoder) -> Option<Pay> {
    let mut p = Pay { time: d.u64().ok()?, ..Pay::default() };
    for _ in 0..d.u32().ok()? {
        let id = d.hash().ok()?;
        let s = Schedule {
            payer: d.account().ok()?,
            payee: d.account().ok()?,
            asset: d.hash().ok()?,
            amount: d.fixed().ok()?,
            every: decode_every(d)?,
            first_due: d.u64().ok()?,
            index: d.u32().ok()?,
            remaining: match d.u8().ok()? {
                0 => None,
                1 => Some(d.u32().ok()?),
                _ => return None,
            },
            pot: d.fixed().ok()?,
            reference: d.hash().ok()?,
            state: state_from(d.u8().ok()?)?,
        };
        p.schedules.insert(id, s);
    }
    for _ in 0..d.u32().ok()? {
        let id = d.hash().ok()?;
        let r = PaymentRecord {
            payer: d.account().ok()?,
            payee: d.account().ok()?,
            asset: d.hash().ok()?,
            amount: d.fixed().ok()?,
            fee: d.fixed().ok()?,
            refunded: d.fixed().ok()?,
            refunds: d.u32().ok()?,
            at: d.u64().ok()?,
        };
        p.payments.insert(id, r);
    }
    Some(p)
}
```

- [ ] **Step 5: Run the pay tests and the replay/lending suites**

Run: `cargo test -p objectvm --test pay && cargo test -p objectvm --test lend_v2 --test journal_is_replayable --test market`
Expected: all pass; `a_history_without_pay_keeps_its_root` still matches `PINNED_ROOT`.

- [ ] **Step 6: Commit**

```bash
git add objectvm/src/market.rs objectvm/src/pay.rs objectvm/tests/pay.rs
git commit -m "Zyn Pay: pay state in the market — pots count toward conservation, section and MARKET19 only once used"
```

---

### Task 4: PayCreate, PayTopUp, PayCancel

**Files:**
- Modify: `objectvm/src/pay.rs` (apply functions)
- Modify: `objectvm/src/lib.rs` (dispatch arms replace Task 2's refusal for these three)
- Test: `objectvm/tests/pay.rs`

**Interfaces:**
- Consumes: `plan_funding`, `apply_funding`, `fresh_source` (market.rs), `market.pay(source, k, owner, asset, amount)`, `self.viewing` (registered keys).
- Produces: `ObjectVm::apply_pay_create(...)`, `apply_pay_top_up(...)`, `apply_pay_cancel(...)` returning `Result<Receipt, &'static str>`.

- [ ] **Step 1: Write the failing tests**

```rust
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
    c.clock(NOV1 + DAY);
    let op = base(&mut c, SHOP, Every::Days(1), NOV1, 1);
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
```

`c.clock(...)` is used before Task 5 implements the clock; for this test only, the first refusal checks need the clock to be set. Until Task 5 lands, mark `create_is_refused_when_it_cannot_work` `#[ignore]` on its last two lines' assertion by splitting them into a separate test `create_after_its_first_due_is_refused` that Task 5 un-ignores:

```rust
#[test]
#[ignore = "needs the pay clock (Task 5)"]
fn create_after_its_first_due_is_refused() {
    let mut c = Chain::new();
    c.mint(ALICE, BOLD, whole(100));
    c.clock(NOV1 + DAY);
    let nonce = c.fresh();
    let (inputs, change) = c.fund(ALICE, BOLD, whole(10));
    let op = Op::PayCreate { payer: ALICE, program: program::pay_create(), payee: SHOP, asset: BOLD, amount: whole(10), every: Every::Days(1), first_due: NOV1, periods: Some(1), fund_periods: 1, reference: [0; 32], nonce, inputs, change };
    assert_eq!(c.refused(op), "the first payment is already past");
}
```

and delete the last three lines (`c.clock…` through the final `assert_eq!`) from `create_is_refused_when_it_cannot_work`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p objectvm --test pay create_ top_up_ funding_overflow`
Expected: FAIL — refusals read "Zyn Pay is not switched on in this build".

- [ ] **Step 3: Implement**

In `objectvm/src/pay.rs`:

```rust
use crate::{ObjectVm, Output, Receipt};

impl ObjectVm {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_pay_create(
        &mut self,
        payer: &AccountId,
        payee: &AccountId,
        asset: &Address,
        amount: Fixed,
        every: Every,
        first_due: u64,
        periods: Option<u32>,
        fund_periods: u32,
        reference: &Hash,
        nonce: &Hash,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        if payer == payee {
            return Err("a schedule cannot pay its own payer");
        }
        if !self.viewing.contains_key(payee) {
            return Err("the payee has not registered where to be paid");
        }
        if amount.0 <= 0 {
            return Err("an amount must be above nothing");
        }
        if due_at(first_due, every, 1).is_none() {
            return Err("a schedule needs a period above nothing");
        }
        if fund_periods == 0 {
            return Err("fund at least one period");
        }
        if periods == Some(0) || periods.is_some_and(|p| fund_periods > p) {
            return Err("fund no more periods than the schedule has");
        }
        if first_due < self.market.pay.time {
            return Err("the first payment is already past");
        }
        let id = schedule_id(payer, nonce);
        self.fresh_source(&id)?;
        let pot = amount.0.checked_mul(fund_periods as i128).ok_or("that schedule is too large")?;
        let f = self.plan_funding(payer, inputs, change, &[(*asset, Fixed(pot))])?;
        self.apply_funding(payer, inputs, f);
        self.market.pay.schedules.insert(
            id,
            Schedule {
                payer: *payer,
                payee: *payee,
                asset: *asset,
                amount,
                every,
                first_due,
                index: 0,
                remaining: periods,
                pot: Fixed(pot),
                reference: *reference,
                state: ScheduleState::Active,
            },
        );
        Ok(Receipt::Paid { id })
    }

    pub(crate) fn apply_pay_top_up(
        &mut self,
        payer: &AccountId,
        schedule: &Hash,
        periods: u32,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let s = *self.market.pay.schedules.get(schedule).ok_or("no such schedule")?;
        if &s.payer != payer {
            return Err("not your schedule");
        }
        if matches!(s.state, ScheduleState::Ended | ScheduleState::Cancelled) {
            return Err("that schedule has ended");
        }
        if periods == 0 {
            return Err("fund at least one period");
        }
        let add = s.amount.0.checked_mul(periods as i128).ok_or("that schedule is too large")?;
        let pot = s.pot.0.checked_add(add).ok_or("that schedule is too large")?;
        if let Some(r) = s.remaining {
            if pot > s.amount.0.saturating_mul(r as i128) {
                return Err("fund no more periods than the schedule has");
            }
        }
        let f = self.plan_funding(payer, inputs, change, &[(s.asset, Fixed(add))])?;
        self.apply_funding(payer, inputs, f);
        let time = self.market.pay.time;
        let m = self.market.pay.schedules.get_mut(schedule).expect("checked above");
        m.pot = Fixed(pot);
        if m.state == ScheduleState::Unfunded {
            // Periods missed while unfunded are skipped, not charged later:
            // move to the latest one already due, which the next tick pays.
            while due_at(m.first_due, m.every, m.index + 1).is_some_and(|d| d <= time) {
                m.index += 1;
            }
            m.state = ScheduleState::Active;
        }
        Ok(Receipt::Paid { id: *schedule })
    }

    pub(crate) fn apply_pay_cancel(&mut self, payer: &AccountId, schedule: &Hash) -> Result<Receipt, &'static str> {
        let s = *self.market.pay.schedules.get(schedule).ok_or("no such schedule")?;
        if &s.payer != payer {
            return Err("not your schedule");
        }
        if matches!(s.state, ScheduleState::Ended | ScheduleState::Cancelled) {
            return Err("that schedule has ended");
        }
        self.market.pay(&cancel_source(schedule), 0, s.payer, s.asset, s.pot);
        let m = self.market.pay.schedules.get_mut(schedule).expect("checked above");
        m.pot = Fixed::ZERO;
        m.state = ScheduleState::Cancelled;
        Ok(Receipt::Paid { id: *schedule })
    }
}
```

`self.market.pay(...)` is the claim helper `Market::pay` in market.rs — it shares its name with the new `pay` field; Rust resolves `self.market.pay(..)` (call) and `self.market.pay.x` (field) separately, so both compile. If the compiler disagrees, call it as `market::Market::pay(&mut self.market, ...)`.

In `objectvm/src/lib.rs` dispatch, replace Task 2's combined refusal with per-op arms (keep the refusal for `PayOnce`, `PayRefund`, `PayClock` until Tasks 5–6):

```rust
            Op::PayCreate { payer, program, payee, asset, amount, every, first_due, periods, fund_periods, reference, nonce, inputs, change } => one(if *program != program::pay_create() {
                Err("wrong program for a scheduled payment")
            } else {
                self.apply_pay_create(payer, payee, asset, *amount, *every, *first_due, *periods, *fund_periods, reference, nonce, inputs, change)
            }),
            Op::PayTopUp { payer, program, schedule, periods, inputs, change } => one(if *program != program::pay_manage() {
                Err("wrong program for a top-up")
            } else {
                self.apply_pay_top_up(payer, schedule, *periods, inputs, change)
            }),
            Op::PayCancel { payer, program, schedule } => one(if *program != program::pay_manage() {
                Err("wrong program for a cancellation")
            } else {
                self.apply_pay_cancel(payer, schedule)
            }),
            Op::PayOnce { .. } | Op::PayRefund { .. } | Op::PayClock { .. } => one(Err("Zyn Pay is not switched on in this build")),
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p objectvm --test pay`
Expected: all pass except the ignored `create_after_its_first_due_is_refused`.

- [ ] **Step 5: Commit**

```bash
git add objectvm/src/pay.rs objectvm/src/lib.rs objectvm/tests/pay.rs
git commit -m "Zyn Pay: create (funded at approval), top up (resumes, skips missed periods), cancel (pot back to the payer)"
```

---

### Task 5: PayClock — paying what is due

**Files:**
- Modify: `objectvm/src/pay.rs`, `objectvm/src/lib.rs` (dispatch arm for `PayClock`)
- Test: `objectvm/tests/pay.rs`

**Interfaces:**
- Consumes: Tasks 1–4; `Market::pay`, `Market::to_pol`.
- Produces: `ObjectVm::apply_pay_clock(at: u64) -> Result<Receipt, &'static str>`; payment records `pay_payment_id(schedule, k)` with payee claim `claim_id(payment_id, 0)`.

- [ ] **Step 1: Write the failing tests**

```rust
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
```

Also remove `#[ignore]` from `create_after_its_first_due_is_refused`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p objectvm --test pay`
Expected: the new tests and `create_after_its_first_due_is_refused` FAIL ("Zyn Pay is not switched on in this build").

- [ ] **Step 3: Implement the clock**

In `objectvm/src/pay.rs`, inside `impl ObjectVm`:

```rust
    /// The pay clock: pays every period due at or before `at`, oldest due
    /// first, at most `MAX_DUE_PER_TICK`; the rest at the next post.
    pub(crate) fn apply_pay_clock(&mut self, at: u64) -> Result<Receipt, &'static str> {
        let now = self.market.pay.time;
        if at < now {
            return Err("the pay clock cannot run backwards");
        }
        if now > 0 && at > now.saturating_add(MAX_PAY_CLOCK_STEP_SECS) {
            return Err("the pay clock moves at most an hour per post");
        }
        self.market.pay.time = at;
        // Every (due, schedule) pair due by `at`, in due order. One schedule
        // may contribute several periods after a long gap.
        let mut due: Vec<(u64, Hash)> = Vec::new();
        for (id, s) in &self.market.pay.schedules {
            if s.state != ScheduleState::Active {
                continue;
            }
            let mut k = s.index;
            let mut left = s.remaining;
            while left != Some(0) {
                match due_at(s.first_due, s.every, k) {
                    Some(d) if d <= at => due.push((d, *id)),
                    _ => break,
                }
                k += 1;
                left = left.map(|r| r - 1);
                if due.len() > MAX_DUE_PER_TICK * 4 {
                    break;
                }
            }
        }
        due.sort();
        due.truncate(MAX_DUE_PER_TICK);
        for (when, id) in due {
            self.pay_one_period(&id, when)?;
        }
        Ok(Receipt::Paid { id: [0u8; 32] })
    }

    fn pay_one_period(&mut self, id: &Hash, when: u64) -> Result<(), &'static str> {
        let s = *self.market.pay.schedules.get(id).ok_or("no such schedule")?;
        // An earlier payout in this tick may have ended or emptied it.
        if s.state != ScheduleState::Active {
            return Ok(());
        }
        if s.pot.0 < s.amount.0 {
            self.market.pay.schedules.get_mut(id).expect("present").state = ScheduleState::Unfunded;
            return Ok(());
        }
        let payment = pay_payment_id(id, s.index);
        let fee = fee_of(s.amount);
        let net = Fixed(s.amount.0 - fee.0);
        self.market.pay(&payment, 0, s.payee, s.asset, net);
        self.market.to_pol(s.asset, fee).ok_or("fee overflow")?;
        self.market.pay.payments.insert(
            payment,
            PaymentRecord { payer: s.payer, payee: s.payee, asset: s.asset, amount: s.amount, fee, refunded: Fixed::ZERO, refunds: 0, at: when },
        );
        let m = self.market.pay.schedules.get_mut(id).expect("present");
        m.pot = Fixed(m.pot.0 - m.amount.0);
        m.index += 1;
        if let Some(r) = m.remaining.as_mut() {
            *r -= 1;
            if *r == 0 {
                // Nothing more to pay; anything left in the pot goes back.
                let rest = m.pot;
                m.pot = Fixed::ZERO;
                m.state = ScheduleState::Ended;
                let (payer, asset) = (m.payer, m.asset);
                self.market.pay(&cancel_source(id), 0, payer, asset, rest);
            }
        }
        Ok(())
    }
```

Note `market.pay` silently ignores a zero amount, so an ended schedule with an empty pot creates no claim.

In `objectvm/src/lib.rs` dispatch, replace `Op::PayClock` in the remaining refusal with:

```rust
            Op::PayClock { program, at } => one(if *program != program::pay_clock() {
                Err("wrong program for the pay clock")
            } else {
                self.apply_pay_clock(*at)
            }),
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p objectvm --test pay`
Expected: all pass. If `catch_up_pays_each_missed_period_in_order` shows fewer than 5 payments, check that `clock_to` steps an hour at a time (it does) and that the loop in `apply_pay_clock` starts from `s.index`.

- [ ] **Step 5: Commit**

```bash
git add objectvm/src/pay.rs objectvm/src/lib.rs objectvm/tests/pay.rs
git commit -m "Zyn Pay: the pay clock pays each due period once, oldest first, 256 per tick, fee to the ZYN pot"
```

---

### Task 6: PayOnce and PayRefund

**Files:**
- Modify: `objectvm/src/pay.rs`, `objectvm/src/lib.rs` (dispatch arms)
- Test: `objectvm/tests/pay.rs`

**Interfaces:**
- Produces: `ObjectVm::apply_pay_once(...)`, `apply_pay_refund(...)`; one-off payment id `pay_once_id(payer, nonce)`; refund claim `claim_id(refund_source(payment, n), 0)` owned by the payer.

- [ ] **Step 1: Write the failing tests**

```rust
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
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p objectvm --test pay one_off refund`
Expected: FAIL ("Zyn Pay is not switched on in this build").

- [ ] **Step 3: Implement**

In `objectvm/src/pay.rs`, inside `impl ObjectVm`:

```rust
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_pay_once(
        &mut self,
        payer: &AccountId,
        payee: &AccountId,
        asset: &Address,
        amount: Fixed,
        nonce: &Hash,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        if payer == payee {
            return Err("a payment cannot pay its own payer");
        }
        if !self.viewing.contains_key(payee) {
            return Err("the payee has not registered where to be paid");
        }
        if amount.0 <= 0 {
            return Err("an amount must be above nothing");
        }
        let id = pay_once_id(payer, nonce);
        self.fresh_source(&id)?;
        let f = self.plan_funding(payer, inputs, change, &[(*asset, amount)])?;
        self.apply_funding(payer, inputs, f);
        let fee = fee_of(amount);
        self.market.pay(&id, 0, *payee, *asset, Fixed(amount.0 - fee.0));
        self.market.to_pol(*asset, fee).ok_or("fee overflow")?;
        let at = self.market.pay.time;
        self.market.pay.payments.insert(
            id,
            PaymentRecord { payer: *payer, payee: *payee, asset: *asset, amount, fee, refunded: Fixed::ZERO, refunds: 0, at },
        );
        Ok(Receipt::Paid { id })
    }

    pub(crate) fn apply_pay_refund(
        &mut self,
        payee: &AccountId,
        payment: &Hash,
        amount: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let r = *self.market.pay.payments.get(payment).ok_or("no such payment")?;
        if &r.payee != payee {
            return Err("only the payee refunds a payment");
        }
        if amount.0 <= 0 {
            return Err("an amount must be above nothing");
        }
        let after = r.refunded.0.checked_add(amount.0).ok_or("refund overflow")?;
        if after > r.amount.0 {
            return Err("more than is left to refund");
        }
        let f = self.plan_funding(payee, inputs, change, &[(r.asset, amount)])?;
        self.apply_funding(payee, inputs, f);
        self.market.pay(&refund_source(payment, r.refunds), 0, r.payer, r.asset, amount);
        let m = self.market.pay.payments.get_mut(payment).expect("checked above");
        m.refunded = Fixed(after);
        m.refunds += 1;
        Ok(Receipt::Paid { id: *payment })
    }
```

In `objectvm/src/lib.rs` dispatch, replace the remaining refusal with:

```rust
            Op::PayOnce { payer, program, payee, asset, amount, reference: _, nonce, inputs, change } => one(if *program != program::pay_once() {
                Err("wrong program for a payment")
            } else {
                self.apply_pay_once(payer, payee, asset, *amount, nonce, inputs, change)
            }),
            Op::PayRefund { payee, program, payment, amount, inputs, change } => one(if *program != program::pay_refund() {
                Err("wrong program for a refund")
            } else {
                self.apply_pay_refund(payee, payment, *amount, inputs, change)
            }),
```

`PayOnce`'s `reference` is carried in the signed intent and the journal for the payee's matching; the ledger does not store it (the payee knows its own reference from checkout).

- [ ] **Step 4: Run the whole suite**

Run: `cargo test -p objectvm`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add objectvm/src/pay.rs objectvm/src/lib.rs objectvm/tests/pay.rs
git commit -m "Zyn Pay: one-off payments and refunds routed to the payer the payee never names"
```

---

### Task 7: Typed intents for the pay ops (S13)

**Files:**
- Modify: `objectvm/src/lib.rs` (`impl MicrochainVm for ObjectVm`: add `fn typed_intent`)
- Test: `objectvm/tests/pay.rs`

**Interfaces:**
- Consumes: `zyn_vm::eip712::{TypedData, Value}`.
- Produces: `ObjectVm::typed_intent(&Op) -> TypedData`: named fields for the six pay ops; every other op keeps the default opaque digest exactly.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_wallet_sees_every_field_of_a_schedule_and_other_ops_stay_opaque() {
    let op = Op::PayCreate { payer: ALICE, program: program::pay_create(), payee: SHOP, asset: BOLD, amount: whole(10), every: Every::Months(1), first_due: NOV1, periods: Some(12), fund_periods: 3, reference: [1; 32], nonce: [2; 32], inputs: vec![([3; 32], [4; 32])], change: Vec::new() };
    let t = ObjectVm::typed_intent(&op);
    let s = format!("{t:?}");
    for f in ["ZynPayCreate", "payee", "asset", "amountPerPeriod", "everyUnit", "everyCount", "firstDue", "periods", "fundPeriods", "reference"] {
        assert!(s.contains(f), "missing {f} in {s}");
    }
    let other = Op::Mint { asset: BOLD, amount: whole(1), owner: ALICE, nonce: [9; 32] };
    assert!(format!("{:?}", ObjectVm::typed_intent(&other)).contains("OpaqueIntent"));
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p objectvm --test pay a_wallet_sees_every_field`
Expected: FAIL (`ZynPayCreate` missing: the default renders `OpaqueIntent`).

- [ ] **Step 3: Implement**

In `impl MicrochainVm for ObjectVm` (lib.rs), add:

```rust
    /// S13 for Zyn Pay: a wallet shows every field that changes the outcome.
    /// Other intents keep the default opaque digest, byte for byte.
    fn typed_intent(intent: &Op) -> zyn_vm::eip712::TypedData {
        use zyn_vm::eip712::{TypedData, Value};
        fn word(b: &[u8; 32]) -> Value {
            Value::Bytes32(*b)
        }
        fn uint(n: u128) -> Value {
            let mut w = [0u8; 32];
            w[16..].copy_from_slice(&n.to_be_bytes());
            Value::Uint256(w)
        }
        fn fixed(f: Fixed) -> Value {
            let mut w = [0u8; 32];
            w[16..].copy_from_slice(&f.0.to_be_bytes());
            Value::Int256(w)
        }
        match intent {
            Op::PayCreate { payer, payee, asset, amount, every, first_due, periods, fund_periods, reference, nonce, .. } => {
                let (unit, count) = match every {
                    pay::Every::Days(n) => ("days", *n),
                    pay::Every::Months(n) => ("months", *n),
                };
                TypedData::new("ZynPayCreate")
                    .field("payer", word(payer))
                    .field("payee", word(payee))
                    .field("asset", word(asset))
                    .field("amountPerPeriod", fixed(*amount))
                    .field("everyUnit", Value::String(unit.to_string()))
                    .field("everyCount", uint(count as u128))
                    .field("firstDue", uint(*first_due as u128))
                    .field("periods", uint(periods.map(|p| p as u128).unwrap_or(0)))
                    .field("fundPeriods", uint(*fund_periods as u128))
                    .field("reference", word(reference))
                    .field("nonce", word(nonce))
            }
            Op::PayTopUp { payer, schedule, periods, .. } => TypedData::new("ZynPayTopUp")
                .field("payer", word(payer))
                .field("schedule", word(schedule))
                .field("periods", uint(*periods as u128)),
            Op::PayCancel { payer, schedule, .. } => TypedData::new("ZynPayCancel")
                .field("payer", word(payer))
                .field("schedule", word(schedule)),
            Op::PayOnce { payer, payee, asset, amount, reference, nonce, .. } => TypedData::new("ZynPayOnce")
                .field("payer", word(payer))
                .field("payee", word(payee))
                .field("asset", word(asset))
                .field("amount", fixed(*amount))
                .field("reference", word(reference))
                .field("nonce", word(nonce)),
            Op::PayRefund { payee, payment, amount, .. } => TypedData::new("ZynPayRefund")
                .field("payee", word(payee))
                .field("payment", word(payment))
                .field("amount", fixed(*amount)),
            _ => {
                use sha2::{Digest, Sha256};
                let mut h = Sha256::new();
                h.update(Self::encode_intent(intent));
                let digest: [u8; 32] = h.finalize().into();
                TypedData::new("OpaqueIntent").field("intent", Value::Bytes32(digest))
            }
        }
    }
```

`periods: 0` in the typed data means "until cancelled" (`periods: None`); Plan 2's consent card shows it that way. Add `sha2` to `objectvm/Cargo.toml` `[dependencies]` with the version `zyn-vm/Cargo.toml` uses, if objectvm does not already depend on it. If `Value::Uint256`/`Int256` hold a different word type than `[u8; 32]`, follow `swapvm/src/typed.rs`, which builds the same values.

- [ ] **Step 4: Run the suite**

Run: `cargo test -p objectvm`
Expected: all pass.

- [ ] **Step 5: Check signing did not move for existing intents**

Run: `cargo test -p zynzapd` and `cd apps/nap-web && npm test`
Expected: pass. Existing ops still render `OpaqueIntent` with the same digest, so their signing payloads are unchanged.

- [ ] **Step 6: Commit**

```bash
git add objectvm/src/lib.rs objectvm/Cargo.toml objectvm/tests/pay.rs
git commit -m "Zyn Pay: typed intents for the pay ops (S13); every other intent keeps its opaque digest"
```

---

### Task 8: The node — pay clock thread and read-by-id RPC

**Files:**
- Modify: `zynzapd/src/bin/zyn-object-node.rs` (clock thread beside the lending clock thread near line 3340; RPC arms beside `"zyn_lendPositions"` near line 1135; JSON helpers beside `lend_position_json`)
- Test: unit tests in the same file's `#[cfg(test)] mod tests` (create the module if absent)

**Interfaces:**
- Consumes: `ObjectVm.market.pay`, `apply_operator(&op, &node, &journal, &pub_log, &dir, chain)`, `now_secs()`, `field(body, name)`, `unhex`, `hex`, `err`.
- Produces:
  - env `ZYN_PAY_TICK_SECS` (default 10, minimum 5).
  - RPC `zyn_paySchedules { ids: "hex,hex,…" }` → `[{ id, payee, asset, amount, every: {unit, count}, firstDue, nextDue, index, remaining, pot, reference, state }]` (no payer field).
  - RPC `zyn_payPayments { ids }` → `[{ id, payee, asset, amount, fee, refunded, at }]` (no payer field).
  - Unknown ids are skipped; at most 256 ids per call.

- [ ] **Step 1: Write the failing tests for the JSON builders**

```rust
#[cfg(test)]
mod pay_rpc_tests {
    use super::*;
    use objectvm::pay::{Every, PaymentRecord, Schedule, ScheduleState};

    #[test]
    fn schedule_json_omits_the_payer_and_shows_the_next_due() {
        let s = Schedule {
            payer: [0xA1; 32], payee: [0x5E; 32], asset: [0xB1; 32], amount: zyn_vm::Fixed(10), every: Every::Months(1),
            first_due: 1_793_491_200, index: 1, remaining: Some(11), pot: zyn_vm::Fixed(20), reference: [1; 32], state: ScheduleState::Active,
        };
        let v = pay_schedule_json(&[9; 32], &s);
        assert!(v.get("payer").is_none());
        assert_eq!(v["state"], "active");
        assert_eq!(v["every"]["unit"], "months");
        assert_eq!(v["nextDue"], objectvm::pay::due_at(s.first_due, s.every, 1).unwrap());
    }

    #[test]
    fn payment_json_omits_the_payer() {
        let r = PaymentRecord { payer: [0xA1; 32], payee: [0x5E; 32], asset: [0xB1; 32], amount: zyn_vm::Fixed(10), fee: zyn_vm::Fixed(0), refunded: zyn_vm::Fixed(0), refunds: 0, at: 5 };
        let v = pay_payment_json(&[8; 32], &r);
        assert!(v.get("payer").is_none());
        assert_eq!(v["at"], 5);
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p zynzapd --bin zyn-object-node pay_rpc_tests`
Expected: compile error (`pay_schedule_json` not found).

- [ ] **Step 3: Implement the JSON builders, RPC arms and the clock thread**

Beside `lend_position_json`:

```rust
fn fixed_str(f: zyn_vm::Fixed) -> String {
    f.0.to_string()
}

fn pay_schedule_json(id: &[u8; 32], s: &objectvm::pay::Schedule) -> serde_json::Value {
    use objectvm::pay::{due_at, Every, ScheduleState};
    let (unit, count) = match s.every {
        Every::Days(n) => ("days", n),
        Every::Months(n) => ("months", n),
    };
    let next = match s.state {
        ScheduleState::Active | ScheduleState::Unfunded => due_at(s.first_due, s.every, s.index),
        _ => None,
    };
    serde_json::json!({
        "id": hex(id),
        "payee": hex(&s.payee),
        "asset": hex(&s.asset),
        "amount": fixed_str(s.amount),
        "every": { "unit": unit, "count": count },
        "firstDue": s.first_due,
        "nextDue": next,
        "index": s.index,
        "remaining": s.remaining,
        "pot": fixed_str(s.pot),
        "reference": hex(&s.reference),
        "state": match s.state {
            ScheduleState::Active => "active",
            ScheduleState::Unfunded => "unfunded",
            ScheduleState::Ended => "ended",
            ScheduleState::Cancelled => "cancelled",
        },
    })
}

fn pay_payment_json(id: &[u8; 32], r: &objectvm::pay::PaymentRecord) -> serde_json::Value {
    serde_json::json!({
        "id": hex(id),
        "payee": hex(&r.payee),
        "asset": hex(&r.asset),
        "amount": fixed_str(r.amount),
        "fee": fixed_str(r.fee),
        "refunded": fixed_str(r.refunded),
        "at": r.at,
    })
}
```

Match how other JSON in this file prints `Fixed` amounts (search `lend_position_json`); if it uses a decimal formatter, use the same one instead of `fixed_str`.

Beside `"zyn_lendPositions"`:

```rust
        // Zyn Pay, by id. A schedule id is `schedule_id(payer, nonce)` and a
        // payment id derives from it or from the payer's nonce, so only the
        // payer and the payee it was handed to can ask. The payer is never
        // in the answer.
        "zyn_paySchedules" | "zyn_payPayments" => {
            let Some(list) = field(body, "ids") else {
                return err("no ids");
            };
            let n = node.lock().unwrap();
            let pay = &n.state().market.pay;
            let mut rows = Vec::new();
            for id in list.split(',').filter(|s| !s.is_empty()).take(256) {
                let Some(key) = unhex(id).and_then(|b| <[u8; 32]>::try_from(b).ok()) else {
                    return err("an id is not 32 hex bytes");
                };
                if method == "zyn_paySchedules" {
                    if let Some(s) = pay.schedules.get(&key) {
                        rows.push(pay_schedule_json(&key, s));
                    }
                } else if let Some(r) = pay.payments.get(&key) {
                    rows.push(pay_payment_json(&key, r));
                }
            }
            format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}}", serde_json::Value::Array(rows))
        }
```

Use the variable that holds the method name in this `match` (it is the scrutinee; replace `method` with its actual name).

Beside the lending clock thread, a pay clock thread:

```rust
    // Zyn Pay: the pay clock, posted every ZYN_PAY_TICK_SECS (default 10 s).
    // Each post moves it at most an hour, so after downtime it catches up
    // over several posts, paying each due period in order.
    {
        let every: u64 = std::env::var("ZYN_PAY_TICK_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(10).max(5);
        let node = Arc::clone(&node);
        let journal = Arc::clone(&journal);
        let pub_log = Arc::clone(&pub_log);
        let dir = dir.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(every));
            let last = node.lock().map(|n| n.state().market.pay.time).unwrap_or(0);
            // Nothing scheduled and no clock yet: stay silent, so a chain
            // with no pay state keeps every root it had.
            let idle = node.lock().map(|n| n.state().market.pay.is_empty()).unwrap_or(true);
            if idle {
                continue;
            }
            let now = now_secs();
            let at = if last == 0 { now } else { now.min(last.saturating_add(objectvm::pay::MAX_PAY_CLOCK_STEP_SECS)) };
            if at <= last {
                continue;
            }
            let op = Op::PayClock { program: zyn_vm::program::pay_clock(), at };
            for r in apply_operator(&op, &node, &journal, &pub_log, &dir, chain) {
                if let objectvm::Receipt::Refused(why) = r {
                    eprintln!("zyn-object-node: pay clock refused: {why}");
                }
            }
        });
    }
```

Important: because the clock stays silent while pay state is empty, the first `PayCreate` on a chain is checked against `pay.time == 0`, so its `first_due` cannot be refused as past; the first clock post after it sets the time. That is intended.

Place the block where `node`, `journal`, `pub_log`, `dir` and `chain` are in scope (the lending thread's surroundings); clone them the same way that thread does.

- [ ] **Step 4: Run the tests and build**

Run: `cargo test -p zynzapd --bin zyn-object-node pay_rpc_tests && cargo build -p zynzapd --bin zyn-object-node`
Expected: PASS and a clean build.

- [ ] **Step 5: Commit**

```bash
git add zynzapd/src/bin/zyn-object-node.rs
git commit -m "Zyn Pay: the node posts the pay clock and serves schedules and payments by id, never the payer"
```

---

### Task 9: Record it — DECISIONS, RUNBOOK, MESSAGING

**Files:**
- Modify: `DECISIONS.md` (new section at the end, next free number)
- Modify: `RUNBOOK.md` (a "Zyn Pay" section after the lending sections)
- Modify: `docs/MESSAGING.md` (two rows in "Say / do not say")
- Modify: `docs/superpowers/specs/2026-10-23-zyn-scheduled-payments-design.md` (the two deviations)

- [ ] **Step 1: DECISIONS**

Append (use the next section number after the last one in the file; shown here as §NNN):

```markdown
## NNN. Zyn Pay: scheduled payments funded at approval (23 Oct 2026)

| # | Decision | Status |
|---|---|---|
| NNN.1 | **Funded at approval, not debited later.** Notes need their owner's nullifier to spend, so the ledger cannot take from a payer on a date; `PayCreate` moves `amount × fund_periods` into the schedule's pot and the pay clock pays from it. Rejected: delegating a nullifier key to a service, which would show it every note the payer holds | ✅ |
| NNN.2 | **The pay clock is the operator's**, like `LendTime`: forward only, at most an hour per post, silent until pay has state so older roots keep. Each due period is paid once, oldest first, at most 256 per post | ✅ |
| NNN.3 | **Missed periods are not charged later.** A schedule whose pot runs short turns unfunded; a top-up resumes it at the latest period already due | ✅ |
| NNN.4 | **Refunds name the payment, never the payer.** The ledger keeps the payer on the payment record (operator-visible, tier 2) and routes the refund as a claim | ✅ |
| NNN.5 | **0.5% of each payment to the ZYN pot**, from the payee's side, rounded down; refunds carry none | ✅ |
| NNN.6 | **Matching by id, not by new claim fields.** Payment ids derive from the schedule and period; a payee's claim is `claim_id(payment, 0)`. Rejected: a reference on every claim, which would change the claim encoding and every root sealed before | ✅ |
| NNN.7 | **Verifier first** (§51.8): every signer and replica takes this build before the sequencer emits a pay op; testnet 212 first, mainnet 26460 after the NU7 switch | 🔨 |
```

- [ ] **Step 2: RUNBOOK**

Add a "Zyn Pay" section: `ZYN_PAY_TICK_SECS` (default 10, min 5); the clock is silent until the first pay op; read schedules with `curl -s localhost:<rpc>/ -d '{"method":"zyn_paySchedules","ids":"<hex>"}'`; rollout order (replicas and ZynScan, then signers, then the sequencer).

- [ ] **Step 3: MESSAGING**

Add rows to "Say / do not say":

```markdown
| a payment schedule **pays from funds set aside at approval**; cancel any time and the rest comes back | auto-debit; we charge your wallet | the ledger cannot take from notes; the payer sets the money aside first |
| who pays whom on a schedule is **not shown to the public**; the operator sees it today | private subscriptions; anonymous payments | tier 2, as everywhere on Zyn |
```

- [ ] **Step 4: Spec deviations**

In the spec, under "The clock", replace the 120-seconds sentence with the hour-per-post rule and its reason (no epoch timestamp in the ledger); under "Ops", replace "The claim carries the schedule's reference and the period number" with matching by payment id as in DECISIONS NNN.6.

- [ ] **Step 5: Commit**

```bash
git add DECISIONS.md RUNBOOK.md docs/MESSAGING.md docs/superpowers/specs/2026-10-23-zyn-scheduled-payments-design.md
git commit -m "Zyn Pay: decisions, runbook and approved claims; spec follows the hour-per-post clock and id matching"
```

---

### Task 10: Testnet rollout (the user runs the deploy)

**Files:**
- Create: `ops/zyn-pay-rollout.sh` (prints and runs the verifier-first order; asks before each host)

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# Zyn Pay rollout, verifier first (DECISIONS §51.8, NNN.7).
# Usage: ops/zyn-pay-rollout.sh testnet
# Builds the object node and ZynScan for each host's arch, then installs
# replicas and ZynScan first, signers next, the sequencer last.
set -euo pipefail
net="${1:?testnet or mainnet}"
[ "$net" = testnet ] || { echo "mainnet waits for the NU7 switch"; exit 1; }
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p zynzapd --bin zyn-object-node --bin zynscan
step() { read -r -p "$1 — go? [y/N] " a; [ "$a" = y ]; }
step "1/3 Hetzner: zynscan + replica" && ssh root@168.119.53.39 'systemctl stop zynscan-testnet zyn-object-replica-testnet || true'
echo "Copy target/aarch64-unknown-linux-gnu/release/{zynscan,zyn-object-node} to Hetzner per RUNBOOK §4 and start the replica units, then:"
step "2/3 signers updated (per RUNBOOK custody section)" || exit 1
step "3/3 sequencer: restart zyn-object-testnet on Hetzner" && ssh root@168.119.53.39 'systemctl restart zyn-object-testnet'
```

Adjust unit names and the copy step to the exact ones in `RUNBOOK.md` (sections 4 and 5) before handing it over; the script never runs without the user saying yes at each step.

- [ ] **Step 2: Hand over**

Tell the user: run `ops/zyn-pay-rollout.sh testnet`. After it, a live check: from the test wallets, Alice subscribes SHOP for 1 BOLD.zy daily with 2 periods funded, the clock pays period 0 at `first_due`, `zyn_paySchedules` shows index 1 and pot 1, cancel returns 1.

- [ ] **Step 3: Commit**

```bash
git add ops/zyn-pay-rollout.sh
git commit -m "Zyn Pay: verifier-first testnet rollout script (the user runs it)"
```

---

## Self-Review

- **Spec coverage:** ledger state and ops (Tasks 1–6), clock (5), fee (5–6), rollout (10), S13 (7), node reads (8), disclosures and copy (9). Nap and the merchant package are Plans 2 and 3, stated in the header. Spec deviations (clock rule, id matching) are recorded in Global Constraints and Task 9.
- **Review Focus:** each of the five has its test in the owning task: catch-up (5), month ends (1), partial refunds (6), dust fee (5), overflow (4).
- **Names:** `apply_pay_create/top_up/cancel/once/refund/clock`, `pay_payment_id`, `pay_once_id`, `refund_source`, `cancel_source`, `due_at`, `fee_of`, `MAX_DUE_PER_TICK`, `MAX_PAY_CLOCK_STEP_SECS` are used identically across tasks.
