// Zyn Pay's consent cards in connect-ops.js, against a stand-in chain: what
// Nap refuses before a card, what each card says, the bytes it signs, and
// that a site is handed only the schedules it asked for.
//
// The whole file runs in UTC+8. The zone is set before anything is imported
// (the imports below are dynamic for that reason), and the first test checks
// the zone really took, so a date computed in local time cannot pass by luck.
process.env.TZ = 'Asia/Shanghai';

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { x25519 } from '@noble/curves/ed25519.js';

const store = new Map();
globalThis.localStorage = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) };

const { METHODS } = await import('../src/connect-ops.js');
const { SCOPES, withConnection, withScope, withPaySchedule, paySchedulesOf } = await import('../src/connect-grants.js');
const { deriveIdentity } = await import('../src/core.js');
const mk = await import('../src/market.js');
const { commitment, hex, nullifierOf, sealNote, unhex, u32be } = await import('../src/object.js');
const pay = await import('../src/pay.js');
const ledger = await import('../src/ledger.js');

const WAD = 10n ** 18n;
const ZEC = '0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201';
const BOLD = 'b01d'.repeat(16);
const ITEM = '17e3'.repeat(16);
const PAYEE = 'cc'.repeat(32);
const NOBODY = 'dd'.repeat(32);
const REF = 'ee'.repeat(32);
const config = {
  id: 'testnet', chain: 212, rpcUrl: 'http://chain.test/', zecAsset: ZEC, collections: [],
  bridges: { eth: { chainId: 11155111, tokens: [{ address: `0x${'b0'.repeat(20)}`, symbol: 'BOLD' }] } },
};
const identity = deriveIdentity('abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art');
const counter = `nap.market.n.${config.chain}.${identity.accountHex}`;
const NOW = Date.UTC(2026, 9, 24) / 1000; // 24 Oct 2026
const NOV1 = Date.UTC(2026, 10, 1) / 1000;
const sid = (n) => hex(mk.scheduleId(identity.account, mk.marketNonce(identity.secretKey, n)));
const PRIVACY = "Who pays whom is not shown to the public; Zyn's operator sees it.";

/// What a site's record holds: the schedule ids it created, as connect-app keeps them.
const siteWith = (ids = []) => { const made = [...ids]; return { list: () => [...made], add: (id) => { made.push(id); }, made }; };
const ctxWith = (site = siteWith()) => ({ config, identity, now: () => NOW, paySite: site });

// The stand-in chain.
let notes; // what this wallet holds: [{asset, amount, nonce}]
let schedules; // id -> node row
let frames; // frames submitted
let receipt; // what the next submit answers
let claims; // id -> zyn_claims row
let payDown; // null, 'outage' (the node errs) or 'unserved' (a node without Pay)
let scheduleAsks; // the ids each zyn_paySchedules call named
const reset = () => {
  notes = [];
  schedules = new Map();
  frames = [];
  receipt = '[Paid { id: .. }]';
  claims = new Map();
  payDown = null;
  scheduleAsks = [];
  store.clear();
  store.set(counter, '5');
  ledger.forgetPayUnserved();
};
reset();

const viewingKey = x25519.getPublicKey(identity.viewing);
function published() {
  return {
    outputs: notes.map((n, i) => {
      const note = { asset: unhex(n.asset), amount: n.amount, nonce: new Uint8Array(32).fill(i + 1) };
      const enc = sealNote(viewingKey, new Uint8Array(32).fill(0x40 + i), note);
      return { commitment: hex(commitment(note, identity.account)), epk: hex(enc.epk), ciphertext: hex(enc.ciphertext) };
    }),
    nullifiers: [],
  };
}
/// The one note `i` this wallet holds, as `fund` spends it.
const input = (i) => {
  const n = notes[i];
  const c = commitment({ asset: unhex(n.asset), amount: n.amount, nonce: new Uint8Array(32).fill(i + 1) }, identity.account);
  return { commitment: c, nullifier: nullifierOf(identity.nullifier, c) };
};

globalThis.fetch = async (url, init) => {
  const body = JSON.parse(init.body);
  const answer = (result) => new Response(JSON.stringify({ jsonrpc: '2.0', id: body.id, result }), { status: 200 });
  const refuse = (message) => new Response(JSON.stringify({ jsonrpc: '2.0', id: body.id, error: { code: -32000, message } }), { status: 200 });
  const ids = String(body.ids || '').split(',').filter(Boolean);
  switch (body.method) {
    case 'zyn_market': return answer({ epoch: 5, epoch_seconds: 10, pools: [], curves: [] });
    case 'zyn_bridged': return answer([{ asset: BOLD, network: 'eip155:11155111', decimals: 18, origin: 'b0'.repeat(20) }]);
    case 'zyn_offers': return answer({ offers: [] });
    case 'zyn_items': return answer(String(body.assets).split(',').filter((a) => a === ITEM).map((asset) => ({ asset, collection: 'c0'.repeat(32), serial: 7 })));
    case 'zyn_published': return answer(published());
    case 'zyn_viewingKey': return body.account === NOBODY ? refuse('that account has not registered where to be paid, so nothing can be sent to it yet') : answer({ viewing: '11'.repeat(32) });
    case 'zyn_paySchedules':
    case 'zyn_payPayments':
      if (payDown === 'outage') return refuse('the node is busy');
      if (payDown === 'unserved') return new Response(JSON.stringify({ jsonrpc: '2.0', id: body.id, error: { code: -32600, message: `this chain is an object ledger and does not serve ${body.method}. It serves zyn_status, zyn_claims` } }), { status: 200 });
      if (body.method === 'zyn_payPayments') return answer([]);
      scheduleAsks.push(ids);
      return answer(ids.filter((i) => schedules.has(i)).map((i) => schedules.get(i)));
    case 'zyn_claims': return answer(ids.filter((i) => claims.has(i)).map((i) => claims.get(i)));
    case 'zyn_status': return answer({ chain: 212, epoch: 5 });
    case 'zyn_submitSigned': frames.push(body.frame); return answer({ receipts: receipt });
    default: return answer([]);
  }
};

const createReq = (o = {}) => ({ payee: PAYEE, asset: 'BOLD', amount: '2', every: { months: 1 }, firstDue: NOV1, periods: 12, fundPeriods: 3, reference: REF, ...o });
const row = (n, o = {}) => ({ id: sid(n), payee: PAYEE, asset: BOLD, amount: (2n * WAD).toString(), every: { unit: 'months', count: 1 }, firstDue: NOV1, nextDue: NOV1, index: 0, remaining: 12, pot: (4n * WAD).toString(), reference: REF, state: 'active', ...o });
/// The intent a frame carries: its last bytes, after their length.
const carries = (frame, intent) => frame.endsWith(hex(u32be(intent.length)) + hex(intent));

test('schedule_text_is_utc', () => {
  // The zone took: 23:00 UTC on 1 Nov is already 2 Nov in Shanghai.
  assert.equal(new Date(Date.UTC(2026, 10, 1, 23)).getDate(), 2, 'the test runs in UTC+8');
  const lateOnNov1 = Date.UTC(2026, 10, 1, 23) / 1000;
  const lateOnOct31 = Date.UTC(2026, 9, 31, 20) / 1000;
  const s = { amount: 2n * WAD, asset: 'BOLD', every: { months: 1 }, periods: 12 };
  assert.equal(pay.scheduleText({ ...s, firstDue: lateOnNov1 }), '2 BOLD every month, 12 times, from 1 Nov 2026');
  assert.equal(pay.scheduleText({ ...s, firstDue: lateOnOct31 }), '2 BOLD every month, 12 times, from 31 Oct 2026');
  assert.equal(pay.scheduleText({ ...s, periods: null, firstDue: NOV1 }), '2 BOLD every month, from 1 Nov 2026, until cancelled');
  assert.equal(pay.scheduleText({ amount: WAD / 2n, asset: 'ZEC.zy', every: { unit: 'days', count: 7 }, periods: 1, firstDue: NOV1 }), '0.5 ZEC.zy every 7 days, once, from 1 Nov 2026');
  // Months follow the UTC calendar: the 31st pays on the last day of February.
  const jan31 = Date.UTC(2027, 0, 31, 23) / 1000;
  assert.equal(pay.dueAt(jan31, { months: 1 }, 1), Date.UTC(2027, 1, 28, 23) / 1000);
  assert.equal(pay.dateText(pay.fundedUntil({ amount: '1', pot: '2', index: 0, firstDue: jan31, every: { unit: 'months', count: 1 }, remaining: null, state: 'active' })), '28 Feb 2027');
  // Funded until: the last period the pot pays, counted from the next one.
  assert.equal(pay.fundedUntil({ amount: '2', pot: '6', index: 2, firstDue: NOV1, every: { unit: 'months', count: 1 }, remaining: 10, state: 'active' }), Date.UTC(2027, 2, 1) / 1000);
  assert.equal(pay.fundedUntil({ amount: '2', pot: '1', index: 2, firstDue: NOV1, every: { unit: 'months', count: 1 }, remaining: 10, state: 'active' }), null);
});

test('runs out soon: 7 days before the last funded payment, or one period if shorter', () => {
  const monthly = { amount: '2', pot: '4', index: 0, firstDue: NOV1, every: { unit: 'months', count: 1 }, remaining: 12, state: 'active' };
  const last = Date.UTC(2026, 11, 1) / 1000;
  assert.equal(pay.fundedUntil(monthly), last);
  assert.equal(pay.runsOutSoon(monthly, last - 8 * 86400), false);
  assert.equal(pay.runsOutSoon(monthly, last - 6 * 86400), true);
  const daily = { ...monthly, every: { unit: 'days', count: 1 } };
  const lastDay = NOV1 + 86400;
  assert.equal(pay.runsOutSoon(daily, lastDay - 2 * 86400), false);
  assert.equal(pay.runsOutSoon(daily, lastDay - 3600), true);
  // Funded to its last payment, or no longer running: nothing to top up.
  assert.equal(pay.runsOutSoon({ ...monthly, remaining: 2 }, last), false);
  assert.equal(pay.runsOutSoon({ ...monthly, state: 'cancelled' }, last), false);
  assert.equal(pay.runsOutSoon({ ...monthly, state: 'unfunded' }, last), false);
});

test('create_refuses_when_holdings_cannot_fund', async () => {
  reset();
  notes = [{ asset: BOLD, amount: 6n * WAD }];
  // Twelve months asked, three held: refused before any card, saying what it needs.
  await assert.rejects(METHODS.zyn_payCreate.prepare(ctxWith(), createReq({ fundPeriods: 12 })), (e) => e.code === -32602 && /does not hold enough BOLD\.zy \(that needs 24\)/.test(e.message));
  assert.ok(await METHODS.zyn_payCreate.prepare(ctxWith(), createReq({ fundPeriods: 3 })));
});

test('bad_requests_never_reach_a_card', async () => {
  reset();
  notes = [{ asset: BOLD, amount: 100n * WAD }];
  store.set(counter, '5');
  const mineAndThisSites = siteWith([sid(1)]);
  schedules.set(sid(1), row(1));
  schedules.set(sid(2), row(2));
  const table = [
    ['zyn_payCreate', null, /params must be an object/],
    ['zyn_payCreate', createReq({ payee: 'xyz' }), /payee/],
    ['zyn_payCreate', createReq({ payee: identity.accountHex }), /a schedule cannot pay its own payer/],
    ['zyn_payCreate', createReq({ payee: NOBODY }), /the payee has not registered where to be paid/],
    ['zyn_payCreate', createReq({ asset: 'DOGE' }), /asset/],
    ['zyn_payCreate', createReq({ asset: 'aa'.repeat(32) }), /asset/],
    ['zyn_payCreate', createReq({ asset: ITEM }), /^items cannot be paid by schedule$/],
    ['zyn_payCreate', createReq({ asset: 42 }), /asset/],
    ['zyn_payCreate', createReq({ amount: '-1' }), /amount/],
    ['zyn_payCreate', createReq({ amount: '0.0009' }), /^an amount below the minimum$/],
    ['zyn_payCreate', createReq({ amount: 2 }), /^amount: a decimal string$/],
    ['zyn_payCreate', createReq({ amount: 0.1 + 0.2 }), /^amount: a decimal string$/],
    ['zyn_payCreate', createReq({ amount: '1e18' }), /^amount: /],
    ['zyn_payCreate', createReq({ amount: '0x10' }), /^amount: /],
    ['zyn_payCreate', createReq({ amount: `0.${'1'.repeat(19)}` }), /^amount: /],
    ['zyn_payOnce', { payee: PAYEE, asset: 'BOLD', amount: 1, reference: REF }, /^amount: a decimal string$/],
    ['zyn_payCreate', createReq({ every: { months: 0 } }), /every/],
    ['zyn_payCreate', createReq({ every: { days: 1, months: 1 } }), /every/],
    ['zyn_payCreate', createReq({ every: { weeks: 1 } }), /every/],
    ['zyn_payCreate', createReq({ every: { days: 1.5 } }), /every/],
    ['zyn_payCreate', createReq({ every: undefined }), /every/],
    ['zyn_payCreate', createReq({ firstDue: NOW - 60 }), /firstDue is in the past/],
    ['zyn_payCreate', createReq({ firstDue: NOV1 * 1000 }), /firstDue/],
    ['zyn_payCreate', createReq({ firstDue: '1793491200' }), /firstDue/],
    ['zyn_payCreate', createReq({ periods: -1 }), /periods/],
    ['zyn_payCreate', createReq({ periods: 2.5 }), /periods/],
    ['zyn_payCreate', createReq({ periods: 0 }), /periods/],
    ['zyn_payCreate', createReq({ fundPeriods: 0 }), /fundPeriods/],
    ['zyn_payCreate', createReq({ fundPeriods: 13 }), /^fund no more periods than the schedule has$/],
    ['zyn_payCreate', createReq({ reference: 'abc' }), /reference/],
    ['zyn_payOnce', { payee: PAYEE, asset: 'BOLD', amount: '0.0001', reference: REF }, /^an amount below the minimum$/],
    ['zyn_payOnce', { payee: PAYEE, asset: ITEM, amount: '1', reference: REF }, /^items cannot be paid this way$/],
    ['zyn_payOnce', { payee: PAYEE, asset: 'BOLD', amount: '1' }, /reference/],
    ['zyn_payTopUp', { schedule: 'nope', periods: 1 }, /schedule/],
    ['zyn_payTopUp', { schedule: sid(1), periods: 0 }, /periods/],
    ['zyn_payTopUp', { schedule: sid(1), periods: -2 }, /periods/],
    ['zyn_payTopUp', { schedule: sid(1), periods: 11 }, /^fund no more periods than the schedule has$/],
    ['zyn_payTopUp', { schedule: sid(2), periods: 1 }, /This site has no schedule with that id/],
    ['zyn_payCancel', { schedule: 'AB'.repeat(32) }, /schedule/],
    ['zyn_payCancel', { schedule: sid(2) }, /This site has no schedule with that id/],
  ];
  for (const [method, params, want] of table) {
    let plan = null;
    await assert.rejects((async () => { plan = await METHODS[method].prepare(ctxWith(mineAndThisSites), params); })(), (e) => {
      assert.equal(e.code, -32602, `${method} ${JSON.stringify(params)}: a parameter error, not "${e.message}"`);
      assert.match(e.message, want, `${method} ${JSON.stringify(params)}`);
      return true;
    });
    assert.equal(plan, null, 'no card was made');
  }
  // An ended schedule says so in the ledger's words.
  schedules.set(sid(1), row(1, { state: 'ended', pot: '0' }));
  await assert.rejects(METHODS.zyn_payCancel.prepare(ctxWith(mineAndThisSites), { schedule: sid(1) }), (e) => e.code === -32602 && e.message === 'that schedule has ended');
  await assert.rejects(METHODS.zyn_payTopUp.prepare(ctxWith(mineAndThisSites), { schedule: sid(1), periods: 1 }), (e) => e.code === -32602 && e.message === 'that schedule has ended');
  assert.equal(frames.length, 0, 'nothing was signed');
});

test('create_card_states_set_aside_schedule_and_privacy', async () => {
  reset();
  notes = [{ asset: BOLD, amount: 6n * WAD }];
  const plan = await METHODS.zyn_payCreate.prepare(ctxWith(), createReq());
  assert.equal(plan.title, 'Scheduled payment');
  assert.equal(plan.danger, false);
  assert.deepEqual(plan.rows, [
    ['To', 'cccccccc…cccccc'],
    ['Amount', '2 BOLD.zy'],
    ['Schedule', '2 BOLD.zy every month, 12 times, from 1 Nov 2026'],
    ['Set aside now', '6 BOLD.zy (3 payments)'],
    ['Funded until', '1 Jan 2027'],
    ['Fee', '0.5%, paid by the recipient'],
  ]);
  assert.equal(plan.note, `Cancel any time in Nap; what has not been paid comes back. ${PRIVACY}`);
  // The asset may be named by symbol or by address; the card is the same.
  const byId = await METHODS.zyn_payCreate.prepare(ctxWith(), createReq({ asset: BOLD, periods: null }));
  assert.equal(Object.fromEntries(byId.rows).Schedule, '2 BOLD.zy every month, from 1 Nov 2026, until cancelled');
  // Every card carries the privacy sentence, and none says "auto-debit".
  for (const p of [plan, byId]) {
    assert.ok(p.note.includes(PRIVACY));
    assert.doesNotMatch(JSON.stringify([p.title, p.rows, p.note]), /auto-?debit/i);
  }
  const once = await METHODS.zyn_payOnce.prepare(ctxWith(), { payee: PAYEE, asset: 'BOLD', amount: '1.5', reference: REF });
  assert.equal(once.title, 'Payment');
  assert.deepEqual(once.rows, [['To', 'cccccccc…cccccc'], ['Amount', '1.5 BOLD.zy']]);
  assert.equal(once.note, PRIVACY);
});

test('create_run_submits_the_vector_bytes', async () => {
  reset();
  // Exactly what three periods need, so no change note (whose keys are fresh).
  notes = [{ asset: BOLD, amount: 6n * WAD }];
  const site = siteWith();
  const plan = await METHODS.zyn_payCreate.prepare(ctxWith(site), createReq());
  const out = await plan.run(() => {});
  const nonce = mk.marketNonce(identity.secretKey, 5);
  const want = mk.payCreateIntent({
    payer: identity.account, payee: unhex(PAYEE), asset: unhex(BOLD), amount: 2n * WAD, every: { months: 1 }, firstDue: NOV1,
    periods: 12, fundPeriods: 3, reference: unhex(REF), nonce, funding: { inputs: [input(0)], change: [] },
  });
  assert.equal(frames.length, 1);
  assert.ok(carries(frames[0], want), 'the frame carries payCreateIntent for the same inputs');
  const schedule = hex(mk.scheduleId(identity.account, nonce));
  assert.deepEqual(out, { schedule, payment: null });
  assert.deepEqual(site.made, [schedule], 'the site\'s record keeps the schedule it asked for');

  // A one-off: its own intent, its payment id; nothing kept for the site.
  reset();
  notes = [{ asset: BOLD, amount: 3n * WAD / 2n }];
  const site2 = siteWith();
  const once = await (await METHODS.zyn_payOnce.prepare(ctxWith(site2), { payee: PAYEE, asset: 'BOLD', amount: '1.5', reference: REF })).run(() => {});
  const wantOnce = mk.payOnceIntent({ payer: identity.account, payee: unhex(PAYEE), asset: unhex(BOLD), amount: 3n * WAD / 2n, reference: unhex(REF), nonce, funding: { inputs: [input(0)], change: [] } });
  assert.ok(carries(frames[0], wantOnce));
  assert.deepEqual(once, { payment: hex(mk.payOnceId(identity.account, nonce)) });
  assert.deepEqual(site2.made, []);
});

test('top up card says what it adds and the date it then runs to', async () => {
  reset();
  notes = [{ asset: BOLD, amount: 4n * WAD }];
  schedules.set(sid(1), row(1));
  const site = siteWith([sid(1)]);
  const plan = await METHODS.zyn_payTopUp.prepare(ctxWith(site), { schedule: sid(1), periods: 2 });
  assert.equal(plan.title, 'Top up payment');
  assert.deepEqual(plan.rows, [['To', 'cccccccc…cccccc'], ['Adds', '4 BOLD.zy (2 payments)'], ['Funded until (after)', '1 Feb 2027']]);
  assert.ok(plan.note.includes(PRIVACY));
  assert.deepEqual(await plan.run(() => {}), { schedule: sid(1) });
  assert.ok(carries(frames[0], mk.payTopUpIntent({ payer: identity.account, schedule: unhex(sid(1)), periods: 2, funding: { inputs: [input(0)], change: [] } })));
  // More than it holds is refused before the card.
  await assert.rejects(METHODS.zyn_payTopUp.prepare(ctxWith(site), { schedule: sid(1), periods: 3 }), /does not hold enough BOLD\.zy \(that needs 6\)/);
});

test('pay_schedules_needs_the_payments_scope_and_returns_only_this_sites_schedules', async () => {
  reset();
  const m = METHODS.zyn_paySchedules;
  assert.equal(m.kind, 'read');
  assert.equal(m.scope, 'payments');
  assert.ok(SCOPES.payments.what && SCOPES.payments.shows && SCOPES.payments.risk);
  // Two sites, each made one schedule from this wallet; a hostile record
  // also names an id that is not this wallet's.
  schedules.set(sid(1), row(1));
  schedules.set(sid(2), row(2));
  schedules.set('ab'.repeat(32), row(9, { id: 'ab'.repeat(32) }));
  const A = 'https://shop.example', B = 'https://other.example';
  let list = withScope(withConnection(withConnection({}, A, 'testnet', identity.accountHex), B, 'testnet', identity.accountHex), A, 'testnet', identity.accountHex, 'payments');
  list = withPaySchedule(list, A, 'testnet', identity.accountHex, sid(1));
  list = withPaySchedule(list, A, 'testnet', identity.accountHex, 'ab'.repeat(32));
  list = withPaySchedule(list, B, 'testnet', identity.accountHex, sid(2));
  const siteOf = (origin) => ({ list: () => paySchedulesOf(list, origin, 'testnet', identity.accountHex), add: () => {} });
  const a = await m.run(ctxWith(siteOf(A)));
  assert.deepEqual(a.schedules.map((s) => s.id), [sid(1)]);
  assert.equal(a.schedules[0].pot, (4n * WAD).toString());
  const b = await m.run(ctxWith(siteOf(B)));
  assert.deepEqual(b.schedules.map((s) => s.id), [sid(2)]);
  // A site that made none sees none; without a site record, none either.
  assert.deepEqual((await m.run(ctxWith(siteOf('https://new.example')))).schedules, []);
  assert.deepEqual((await m.run({ config, identity })).schedules, []);
});

test('cancel_card_shows_what_comes_back', async () => {
  reset();
  schedules.set(sid(1), row(1, { pot: (4n * WAD).toString() }));
  const site = siteWith([sid(1)]);
  const plan = await METHODS.zyn_payCancel.prepare(ctxWith(site), { schedule: sid(1) });
  assert.equal(plan.title, 'Cancel payment');
  assert.deepEqual(plan.rows, [['To', 'cccccccc…cccccc'], ['Comes back now', '4 BOLD.zy']]);
  assert.equal(plan.note, `Nothing more will be paid on this schedule. ${PRIVACY}`);
  assert.deepEqual(await plan.run(() => {}), { schedule: sid(1) });
  assert.ok(carries(frames[0], mk.payCancelIntent({ payer: identity.account, schedule: unhex(sid(1)) })));
  // An unfunded schedule cancels too, with nothing to come back.
  schedules.set(sid(1), row(1, { pot: '0', state: 'unfunded' }));
  assert.deepEqual((await METHODS.zyn_payCancel.prepare(ctxWith(site), { schedule: sid(1) })).rows[1], ['Comes back now', '0 BOLD.zy']);
});

test('ledger_refusals_show_their_sentence', async () => {
  const sentences = [
    'the pay clock has not started', 'an amount below the minimum', 'items cannot be paid by schedule', 'items cannot be paid this way',
    'the payee has not registered where to be paid', 'fund no more periods than the schedule has', 'nothing left to pay on that schedule',
    'that schedule has ended', 'not your schedule',
  ];
  for (const sentence of sentences) {
    reset();
    notes = [{ asset: BOLD, amount: 6n * WAD }];
    receipt = `[Refused("${sentence}")]`;
    const site = siteWith();
    const plan = await METHODS.zyn_payCreate.prepare(ctxWith(site), createReq());
    await assert.rejects(plan.run(() => {}), (e) => e.message.includes(sentence));
    assert.deepEqual(site.made, [], 'a refused schedule is not kept for the site');
  }
  // The top-up and cancel paths carry it the same way.
  reset();
  notes = [{ asset: BOLD, amount: 2n * WAD }];
  schedules.set(sid(1), row(1));
  receipt = '[Refused("not your schedule")]';
  const site = siteWith([sid(1)]);
  await assert.rejects((await METHODS.zyn_payTopUp.prepare(ctxWith(site), { schedule: sid(1), periods: 1 })).run(() => {}), /not your schedule/);
  await assert.rejects((await METHODS.zyn_payCancel.prepare(ctxWith(site), { schedule: sid(1) })).run(() => {}), /not your schedule/);
});

test('no pay method is public, and every write is a card', () => {
  for (const name of ['zyn_payCreate', 'zyn_payOnce', 'zyn_payTopUp', 'zyn_payCancel']) {
    assert.equal(METHODS[name].kind, 'write', name);
    assert.equal(typeof METHODS[name].prepare, 'function', name);
  }
});

const UNREAD = 'The chain could not be read; try again';

test('an outage reading schedules is an error to retry, not "no schedules"', async () => {
  reset();
  schedules.set(sid(1), row(1));
  const site = siteWith([sid(1)]);
  payDown = 'outage';
  const retry = (e) => {
    assert.notEqual(e.code, -32602, `not a parameter error: "${e.message}"`);
    assert.equal(e.code, -32603);
    assert.equal(e.message, UNREAD);
    return true;
  };
  await assert.rejects(METHODS.zyn_paySchedules.run(ctxWith(site)), retry);
  await assert.rejects(METHODS.zyn_payTopUp.prepare(ctxWith(site), { schedule: sid(1), periods: 1 }), retry);
  await assert.rejects(METHODS.zyn_payCancel.prepare(ctxWith(site), { schedule: sid(1) }), retry);
  assert.equal(frames.length, 0, 'nothing was signed');
  // Back up: the same calls work.
  payDown = null;
  assert.deepEqual((await METHODS.zyn_paySchedules.run(ctxWith(site))).schedules.map((x) => x.id), [sid(1)]);
});

test('a node without Pay answers no schedules, quietly', async () => {
  reset();
  payDown = 'unserved';
  const site = siteWith([sid(1)]);
  assert.deepEqual(await METHODS.zyn_paySchedules.run(ctxWith(site)), { schedules: [] });
  await assert.rejects(METHODS.zyn_payCancel.prepare(ctxWith(site), { schedule: sid(1) }), (e) => e.code === -32602 && e.message === 'This wallet has no schedule with that id');
});

test('a top-up or cancel asks the chain for that one schedule only', async () => {
  reset();
  notes = [{ asset: BOLD, amount: 2n * WAD }];
  for (const n of [1, 2, 3]) schedules.set(sid(n), row(n));
  const site = siteWith([sid(1), sid(2)]);
  await METHODS.zyn_payTopUp.prepare(ctxWith(site), { schedule: sid(2), periods: 1 });
  await METHODS.zyn_payCancel.prepare(ctxWith(site), { schedule: sid(1) });
  assert.deepEqual(scheduleAsks, [[sid(2)], [sid(1)]]);
});

test('zyn_owed leaves pay returns out; zyn_collect still collects them', async () => {
  reset();
  const swap = hex(mk.claimId(mk.orderId(identity.account, mk.marketNonce(identity.secretKey, 0)), 0));
  const back = hex(mk.claimId(mk.cancelSource(unhex(sid(1))), 0));
  for (const id of [swap, back]) claims.set(id, { id, asset: ZEC, amount: '5' });
  const site = siteWith([sid(1)]);
  assert.deepEqual(await METHODS.zyn_owed.run(ctxWith(site)), { owed: [{ id: swap, asset: ZEC, amount: '5' }] });
  assert.deepEqual(await METHODS.zyn_collect.run(ctxWith(site)), { collected: 2 });
  assert.equal(frames.length, 1, 'both in one claim');
});

test('an unfunded top-up that pays an overdue period says so, never a past funded-until', async () => {
  reset();
  notes = [{ asset: BOLD, amount: 6n * WAD }];
  // Waiting for funds since 1 Dec; topped up on 15 Dec: the resume pays 1 Dec's period now.
  const DEC15 = Date.UTC(2026, 11, 15) / 1000;
  schedules.set(sid(1), row(1, { state: 'unfunded', index: 1, remaining: 11, pot: '0' }));
  const site = siteWith([sid(1)]);
  const ctx = { ...ctxWith(site), now: () => DEC15 };
  const one = await METHODS.zyn_payTopUp.prepare(ctx, { schedule: sid(1), periods: 1 });
  assert.deepEqual(one.rows, [['To', 'cccccccc…cccccc'], ['Adds', '2 BOLD.zy (1 payment)'], ['Pays now', '1 overdue payment']]);
  const three = await METHODS.zyn_payTopUp.prepare(ctx, { schedule: sid(1), periods: 3 });
  assert.deepEqual(three.rows, [['To', 'cccccccc…cccccc'], ['Adds', '6 BOLD.zy (3 payments)'], ['Pays now', '1 overdue payment'], ['Funded until (after)', '1 Feb 2027']]);
  assert.ok(three.note.includes(PRIVACY));
  // Missed since 1 Nov: the periods skipped are not charged; one is paid now.
  schedules.set(sid(1), row(1, { state: 'unfunded', index: 0, remaining: 12, pot: '0' }));
  assert.deepEqual((await METHODS.zyn_payTopUp.prepare(ctx, { schedule: sid(1), periods: 2 })).rows.slice(2), [['Pays now', '1 overdue payment'], ['Funded until (after)', '1 Jan 2027']]);
  // The count follows afterTopUp.
  const after = pay.afterTopUp(row(1, { state: 'unfunded', index: 1, remaining: 11, pot: '0' }), 3, DEC15);
  assert.equal(after.paysNow, 1);
  assert.deepEqual(pay.topUpCard(row(1), 'BOLD.zy', 2, { ...after, paysNow: 2 }).rows[2], ['Pays now', '2 overdue payments']);
  // Nothing overdue (the 24 Oct case above): no such row.
  assert.equal(pay.afterTopUp(row(1), 1, NOW).paysNow, 0);
});
