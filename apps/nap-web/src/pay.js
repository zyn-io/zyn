// Zyn Pay, as Nap shows it: the words and dates on a payment's card, checked
// site requests, and when a schedule's money runs out. No chain calls and no
// DOM: the Nap Connect cards (connect-ops.js) and the wallet's own Payments
// view share these, so a payment reads the same wherever it is approved.
//
// Dates are the ledger's (objectvm/src/pay.rs `due_at`): unix seconds, months
// on the UTC calendar, a schedule started on the 31st paying on the last day of
// shorter months. They are shown in UTC too, never the device's zone: a holder
// in UTC+8 approving "from 1 Nov" must not see 2 Nov.

import { parseAmount } from './core.js';
import { MIN_PAY_AMOUNT } from './market.js';

/// Shown inline on every payment card (spec "What is visible to whom").
export const PRIVACY = "Who pays whom is not shown to the public; Zyn's operator sees it.";
export const FEE_TEXT = '0.5%, paid by the recipient';

/// A request a site got wrong: EIP-1193's invalid-params code, plain words.
export class PayParamError extends Error {
  constructor(message) { super(message); this.code = -32602; }
}
const bad = (m) => new PayParamError(m);

const DAY = 86_400;
const U32 = 0xffffffff;
const WAD = 10n ** 18n;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
/// The furthest ahead a first payment may be: a site sending milliseconds
/// for seconds would otherwise ask for a date tens of thousands of years out.
const MAX_AHEAD = 10 * 366 * DAY;

/// An amount in base units (1 token = 1e18), exactly: a card never rounds.
export const wadText = (raw) => {
  const v = BigInt(raw);
  const f = (v % WAD).toString().padStart(18, '0').replace(/0+$/, '');
  return `${v / WAD}${f ? `.${f}` : ''}`;
};
export const shortId = (h) => `${String(h).slice(0, 8)}…${String(h).slice(-6)}`;

/// `{days}` / `{months}` (a request) or `{unit, count}` (a node row), as one shape.
export function everyOf(e) {
  if (e && typeof e === 'object' && (e.unit === 'days' || e.unit === 'months')) return { unit: e.unit, count: e.count };
  if (e && typeof e === 'object' && e.days !== undefined) return { unit: 'days', count: e.days };
  return { unit: 'months', count: e?.months };
}

/// When period `k` (from 0) is due, as the ledger computes it; null where the
/// ledger has none (a zero period, or past the calendar).
export function dueAt(firstDue, every, k) {
  const { unit, count } = everyOf(every);
  if (!Number.isSafeInteger(firstDue) || firstDue < 0 || !Number.isInteger(count) || count < 1 || !Number.isInteger(k) || k < 0) return null;
  if (unit === 'days') {
    const t = firstDue + count * k * DAY;
    return Number.isSafeInteger(t) ? t : null;
  }
  const start = new Date(Math.floor(firstDue / DAY) * DAY * 1000);
  const months = start.getUTCMonth() + count * k;
  const y = start.getUTCFullYear() + Math.floor(months / 12);
  const m = months % 12;
  const last = new Date(Date.UTC(y, m + 1, 0)).getUTCDate();
  const day = Date.UTC(y, m, Math.min(start.getUTCDate(), last));
  if (!Number.isFinite(day)) return null;
  const t = day / 1000 + (firstDue % DAY);
  return Number.isSafeInteger(t) ? t : null;
}

/// "1 Nov 2026", in UTC.
export function dateText(seconds) {
  if (seconds == null) return '—';
  const d = new Date(seconds * 1000);
  return `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()}`;
}

export function everyText(every) {
  const { unit, count } = everyOf(every);
  const one = unit === 'days' ? 'day' : 'month';
  return count === 1 ? `every ${one}` : `every ${count} ${one}s`;
}

/// "2 BOLD every month, 12 times, from 1 Nov 2026", or "…, until cancelled".
/// `asset` is the name to show; `amount` is per period, in base units.
export function scheduleText({ amount, asset, every, periods, firstDue }) {
  const head = `${wadText(amount)} ${asset} ${everyText(every)}`;
  const from = `from ${dateText(firstDue)}`;
  if (periods == null) return `${head}, ${from}, until cancelled`;
  return `${head}, ${periods === 1 ? 'once' : `${periods} times`}, ${from}`;
}

/// Periods the pot pays from the next one on: what it holds, capped at what
/// is left of the term.
function fundedPeriods(s) {
  const amount = BigInt(s.amount);
  if (amount <= 0n) return 0;
  let n = BigInt(s.pot) / amount;
  if (s.remaining != null && n > BigInt(s.remaining)) n = BigInt(s.remaining);
  return Number(n);
}
const running = (s) => s.state === undefined || s.state === 'active' || s.state === 'unfunded';

/// The date of the last period the money set aside pays, or null when it pays
/// none (or the schedule has stopped).
export function fundedUntil(s) {
  if (!running(s)) return null;
  const n = fundedPeriods(s);
  if (n < 1) return null;
  return dueAt(Number(s.firstDue), s.every, Number(s.index) + n - 1);
}

/// Whether to remind the payer to top up: from 7 days before the last funded
/// payment, or one period before it if a period is shorter. Not for a schedule
/// funded to its end, nor one already waiting for funds (that has its own notice).
export function runsOutSoon(s, now) {
  if (s.state !== 'active') return false;
  const n = fundedPeriods(s);
  if (s.remaining != null && n >= Number(s.remaining)) return false;
  const until = fundedUntil(s);
  if (until == null) return true;
  const k = Number(s.index) + n - 1;
  const next = dueAt(Number(s.firstDue), s.every, k + 1);
  const period = next == null ? 7 * DAY : next - until;
  return now >= until - Math.min(7 * DAY, period);
}

/// The schedule as a top-up of `periods` leaves it, worked out as the ledger
/// does (`apply_pay_top_up`), with `now` standing in for the pay clock: an
/// unfunded schedule resumes at the latest period already due. Throws the
/// ledger's own sentence for a top-up it would refuse.
export function afterTopUp(s, periods, now) {
  if (s.state === 'ended' || s.state === 'cancelled') throw bad('that schedule has ended');
  let index = Number(s.index);
  let remaining = s.remaining == null ? null : Number(s.remaining);
  if (s.state === 'unfunded') {
    for (;;) {
      if (remaining != null && remaining <= 1) break;
      const d = dueAt(Number(s.firstDue), s.every, index + 1);
      if (d == null || d > now) break;
      index += 1;
      if (remaining != null) remaining -= 1;
    }
  }
  if (remaining === 0) throw bad('nothing left to pay on that schedule');
  const amount = BigInt(s.amount);
  const pot = BigInt(s.pot) + amount * BigInt(periods);
  if (remaining != null && pot > amount * BigInt(remaining)) throw bad('fund no more periods than the schedule has');
  return { ...s, index, remaining, pot: pot.toString(), state: 'active' };
}

// ---------------------------------------------------------------------------
// Site requests. Each field is checked before anything is read from the chain,
// so a malformed request never reaches a card.

const isId = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
const isCount = (v) => Number.isInteger(v) && v >= 1 && v <= U32;
/// A ledger address, or a coin's symbol (ZEC, ETH, SOL, BOLD, with or without .zy).
const isAsset = (v) => isId(v) || (typeof v === 'string' && /^[A-Za-z][A-Za-z0-9]{0,11}(\.zy)?$/.test(v));

function payee(p) {
  if (!isId(p.payee)) throw bad('payee must be a Zyn account: 64 lowercase hex characters');
  return p.payee;
}
function asset(p) {
  if (!isAsset(p.asset)) throw bad('asset must be ZEC, ETH, SOL, BOLD or a 64-hex asset address');
  return p.asset;
}
function amount(p) {
  let v;
  try { v = parseAmount(String(p.amount ?? '')); } catch (e) { throw bad(`amount: ${e.message}`); }
  if (v < MIN_PAY_AMOUNT) throw bad('an amount below the minimum');
  return v;
}
function reference(p) {
  if (!isId(p.reference)) throw bad('reference must be 64 lowercase hex characters');
  return p.reference;
}
function schedule(p) {
  if (!isId(p.schedule)) throw bad('schedule must be a schedule id: 64 lowercase hex characters');
  return p.schedule;
}
function every(p) {
  const e = p.every;
  const keys = e && typeof e === 'object' && !Array.isArray(e) ? Object.keys(e) : [];
  if (keys.length !== 1 || !['days', 'months'].includes(keys[0]) || !isCount(e[keys[0]])) {
    throw bad('every must be {days: n} or {months: n}, n a whole number from 1');
  }
  return { [keys[0]]: e[keys[0]] };
}

/// A site's pay request, checked: `kind` is create, once, topUp or cancel, and
/// `now` (unix seconds) is what a first payment may not be before. Returns
/// clean fields (amounts as BigInt base units) or throws a PayParamError.
export function parsePayRequest(p, { kind = 'create', now = Math.floor(Date.now() / 1000) } = {}) {
  if (!p || typeof p !== 'object' || Array.isArray(p)) throw bad('params must be an object');
  if (kind === 'cancel') return { schedule: schedule(p) };
  if (kind === 'topUp') {
    const s = schedule(p);
    if (!isCount(p.periods)) throw bad('periods must be a whole number from 1');
    return { schedule: s, periods: p.periods };
  }
  if (kind === 'once') return { payee: payee(p), asset: asset(p), amount: amount(p), reference: reference(p) };
  if (kind !== 'create') throw new Error(`no pay request called ${kind}`);
  const out = { payee: payee(p), asset: asset(p), amount: amount(p), every: every(p) };
  if (!Number.isSafeInteger(p.firstDue) || p.firstDue < 0) throw bad('firstDue must be unix seconds, as a number');
  if (p.firstDue < now) throw bad('firstDue is in the past');
  if (p.firstDue > now + MAX_AHEAD) throw bad('firstDue must be unix seconds within the next 10 years');
  if (dueAt(p.firstDue, out.every, 1) == null) throw bad('that schedule runs past the calendar');
  out.firstDue = p.firstDue;
  if (p.periods == null) out.periods = null;
  else if (isCount(p.periods)) out.periods = p.periods;
  else throw bad('periods must be a whole number from 1, or left out to pay until cancelled');
  if (!isCount(p.fundPeriods)) throw bad('fundPeriods must be a whole number from 1');
  if (out.periods != null && p.fundPeriods > out.periods) throw bad('fund no more periods than the schedule has');
  out.fundPeriods = p.fundPeriods;
  if (out.amount * BigInt(out.fundPeriods) >= 1n << 127n) throw bad('that schedule is too large');
  out.reference = reference(p);
  return out;
}

// ---------------------------------------------------------------------------
// The cards: title, rows and note, as Nap Connect and the wallet show them.

const payments = (n) => (n === 1 ? '1 payment' : `${n} payments`);

/// A new schedule. `name` is the asset's name to show (BOLD.zy).
export function createCard({ payee: to, name, amount: per, every: ev, firstDue, periods, fundPeriods }) {
  const until = fundedUntil({ amount: per, pot: per * BigInt(fundPeriods), index: 0, firstDue, every: ev, remaining: periods });
  return {
    title: 'Scheduled payment',
    rows: [
      ['To', shortId(to)],
      ['Amount', `${wadText(per)} ${name}`],
      ['Schedule', scheduleText({ amount: per, asset: name, every: ev, periods, firstDue })],
      ['Set aside now', `${wadText(per * BigInt(fundPeriods))} ${name} (${payments(fundPeriods)})`],
      ['Funded until', dateText(until)],
      ['Fee', FEE_TEXT],
    ],
    note: `Cancel any time in Nap; what has not been paid comes back. ${PRIVACY}`,
  };
}

export function onceCard({ payee: to, name, amount: a }) {
  return { title: 'Payment', rows: [['To', shortId(to)], ['Amount', `${wadText(a)} ${name}`]], note: PRIVACY };
}

/// A top-up of `periods` to schedule row `s`; `after` is `afterTopUp(s, periods, now)`.
export function topUpCard(s, name, periods, after) {
  return {
    title: 'Top up payment',
    rows: [
      ['To', shortId(s.payee)],
      ['Adds', `${wadText(BigInt(s.amount) * BigInt(periods))} ${name} (${payments(periods)})`],
      ['Funded until (after)', dateText(fundedUntil(after))],
    ],
    note: PRIVACY,
  };
}

/// Cancelling schedule row `s`: its pot comes back, as of now.
export function cancelCard(s, name) {
  return {
    title: 'Cancel payment',
    rows: [['To', shortId(s.payee)], ['Comes back now', `${wadText(s.pot)} ${name}`]],
    note: `Nothing more will be paid on this schedule. ${PRIVACY}`,
  };
}
