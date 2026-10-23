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
