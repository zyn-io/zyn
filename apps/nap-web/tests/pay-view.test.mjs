// The wallet's Payments view, without the DOM: one row per schedule, the
// notices (runs out soon, waiting for funds), past payments, and what a
// cancel brought back. app.js renders these models; they live in pay.js.
import { test } from 'node:test';
import assert from 'node:assert/strict';

const store = new Map();
globalThis.localStorage = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) };

const pay = await import('../src/pay.js');
const L = await import('../src/ledger.js');
const mk = await import('../src/market.js');
const { hex, unhex } = await import('../src/object.js');
const { deriveIdentity } = await import('../src/core.js');

const WAD = 10n ** 18n;
const DAY = 86_400;
const ZEC = '0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201';
const BOLD = 'b01d'.repeat(16);
const PAYEE = 'cc'.repeat(32);
const NOV1 = Date.UTC(2026, 10, 1) / 1000;
const JAN1 = Date.UTC(2027, 0, 1) / 1000;
const names = { [ZEC]: 'ZEC.zy', [BOLD]: 'BOLD.zy' };
const nameOf = (a) => names[a] || `${a.slice(0, 6)}…`;

const config = { id: 'testnet', chain: 212, rpcUrl: 'http://chain.test/', zecAsset: ZEC, collections: [], bridges: {} };
const identity = deriveIdentity('abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art');
const sid = (n) => hex(mk.scheduleId(identity.account, mk.marketNonce(identity.secretKey, n)));

/// 2 BOLD.zy a month from 1 Nov 2026, 12 times, 3 set aside: funded until 1 Jan 2027.
const schedule = (o = {}) => ({
  id: sid(0), payee: PAYEE, asset: BOLD, amount: (2n * WAD).toString(), every: { unit: 'months', count: 1 },
  firstDue: NOV1, nextDue: NOV1, index: 0, remaining: 12, pot: (6n * WAD).toString(), reference: 'ee'.repeat(32), state: 'active', ...o,
});

test('one row per schedule: to, amount, next, funded until, and what can be done', () => {
  const view = pay.paymentsView({ schedules: [schedule()], nameOf, now: NOV1 - 30 * DAY });
  assert.equal(view.empty, false);
  assert.equal(view.rows.length, 1);
  const r = view.rows[0];
  assert.equal(r.id, sid(0));
  assert.equal(r.to, 'cccccccc…cccccc');
  assert.equal(r.amount, '2 BOLD.zy every month');
  assert.equal(r.next, '1 Nov 2026');
  assert.equal(r.fundedUntil, '1 Jan 2027');
  assert.deepEqual(r.actions, ['Top up', 'Cancel']);
});

test('next reads Ended, Cancelled or Waiting for funds; stopped schedules cannot be managed', () => {
  const row = (state) => pay.scheduleRow(schedule({ state, nextDue: state === 'unfunded' ? NOV1 : null, pot: '0' }), 'BOLD.zy');
  assert.equal(row('ended').next, 'Ended');
  assert.equal(row('cancelled').next, 'Cancelled');
  assert.equal(row('unfunded').next, 'Waiting for funds');
  assert.deepEqual(row('ended').actions, []);
  assert.deepEqual(row('cancelled').actions, []);
  assert.deepEqual(row('unfunded').actions, ['Top up', 'Cancel']);
  assert.equal(row('cancelled').fundedUntil, '—');
});

test('running schedules come first, soonest next', () => {
  const view = pay.paymentsView({
    schedules: [schedule({ id: 'a'.repeat(64), state: 'ended', nextDue: null }), schedule({ id: 'b'.repeat(64), nextDue: NOV1 + 9 * DAY }), schedule({ id: 'c'.repeat(64) })],
    nameOf, now: NOV1 - DAY,
  });
  assert.deepEqual(view.rows.map((r) => r.id[0]), ['c', 'b', 'a']);
});

test('notice: 8 days before the last funded payment, none', () => {
  const view = pay.paymentsView({ schedules: [schedule()], nameOf, now: JAN1 - 8 * DAY });
  assert.deepEqual(view.notices, []);
  assert.equal(view.badge, 0);
});

test('notice: 6 days before the last funded payment, runs out', () => {
  const view = pay.paymentsView({ schedules: [schedule()], nameOf, now: JAN1 - 6 * DAY });
  assert.equal(view.notices.length, 1);
  assert.deepEqual(view.notices[0], { id: sid(0), kind: 'soon', text: 'Payment to cccccccc…cccccc runs out on 1 Jan 2027', action: 'Top up' });
  assert.equal(view.badge, 1);
});

test('notice: an unfunded schedule is waiting for funds', () => {
  const view = pay.paymentsView({ schedules: [schedule({ state: 'unfunded', pot: '0', index: 3, remaining: 9 })], nameOf, now: Date.UTC(2027, 1, 2) / 1000 });
  assert.equal(view.notices.length, 1);
  assert.deepEqual(view.notices[0], { id: sid(0), kind: 'waiting', text: 'Payment to cccccccc…cccccc is waiting for funds', action: null });
  assert.equal(view.badge, 1);
});

test('stopped schedules have no notice', () => {
  const view = pay.paymentsView({ schedules: [schedule({ state: 'cancelled', pot: '0' }), schedule({ state: 'ended', pot: '0' })], nameOf, now: JAN1 });
  assert.deepEqual(view.notices, []);
});

test('empty: No payments yet.', () => {
  const view = pay.paymentsView({ schedules: [], nameOf, now: NOV1 });
  assert.equal(view.empty, true);
  assert.equal(pay.EMPTY_PAYMENTS, 'No payments yet.');
});

test('past payments: date, amount, refunded if any', () => {
  const p = { id: 'f'.repeat(64), payee: PAYEE, asset: BOLD, amount: (2n * WAD).toString(), fee: '0', refunded: '0', refunds: 0, at: NOV1, reference: 'ee'.repeat(32) };
  assert.deepEqual(pay.paymentRow(p, 'BOLD.zy'), { date: '1 Nov 2026', amount: '2 BOLD.zy', refunded: '' });
  assert.deepEqual(pay.paymentRow({ ...p, refunded: (WAD / 2n).toString(), refunds: 1 }, 'BOLD.zy'), { date: '1 Nov 2026', amount: '2 BOLD.zy', refunded: '0.5 BOLD.zy refunded' });
});

test('the in-app dialogs show the same rows as the Connect cards', () => {
  const s = schedule();
  const now = NOV1 - DAY;
  const top = pay.topUpReview(s, 'BOLD.zy', 2, now);
  const { needs, ...card } = top;
  assert.deepEqual(card, pay.topUpCard(s, 'BOLD.zy', 2, pay.afterTopUp(s, 2, now)));
  assert.deepEqual(top.rows.at(-1), ['Funded until (after)', '1 Mar 2027']);
  assert.match(top.note, /Who pays whom is not shown to the public; Zyn's operator sees it\./);
  assert.equal(needs, 4n * WAD);
  assert.throws(() => pay.topUpReview(s, 'BOLD.zy', 10, now), /fund no more periods than the schedule has/);
  assert.throws(() => pay.topUpReview(s, 'BOLD.zy', 0, now), /whole number from 1/);
  const cancel = pay.cancelCard(s, 'BOLD.zy');
  assert.deepEqual(cancel.rows, [['To', 'cccccccc…cccccc'], ['Comes back now', '6 BOLD.zy']]);
  assert.match(cancel.note, /Who pays whom is not shown to the public; Zyn's operator sees it\./);
});

test('cancel_shows_what_came_back', async () => {
  // The stand-in chain: one cancelled schedule, its pot waiting as a claim.
  store.clear();
  const s = schedule({ id: sid(4), state: 'cancelled', pot: '0', index: 2, remaining: 10, nextDue: null });
  const back = hex(mk.claimId(mk.cancelSource(unhex(s.id)), 0));
  const claims = new Map([[back, { id: back, asset: BOLD, amount: (20n * WAD).toString() }]]);
  globalThis.fetch = async (_url, init) => {
    const body = JSON.parse(init.body);
    const answer = (result) => new Response(JSON.stringify({ jsonrpc: '2.0', id: body.id, result }), { status: 200 });
    const ids = String(body.ids || '').split(',').filter(Boolean);
    switch (body.method) {
      case 'zyn_paySchedules': return answer(ids.filter((i) => i === s.id).map(() => s));
      case 'zyn_claims': return answer(ids.filter((i) => claims.has(i)).map((i) => claims.get(i)));
      default: return answer([]);
    }
  };
  const owed = await L.loadOwed(config, identity);
  assert.equal(owed.find((o) => o.id === back)?.kind, 'Payment returned');

  assert.deepEqual(pay.cancelReturn(s, owed, nameOf), { label: 'Payment returned', amount: '20 BOLD.zy' });
  const view = pay.paymentsView({ schedules: [s], owed, nameOf, now: NOV1 });
  assert.deepEqual(view.returned, [{ id: back, label: 'Payment returned', amount: '20 BOLD.zy' }]);
  assert.equal(view.rows[0].next, 'Cancelled');
  // Nothing came back (yet): nothing is claimed to have.
  assert.equal(pay.cancelReturn(s, [], nameOf), null);
});
