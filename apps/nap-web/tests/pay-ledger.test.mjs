// Nap's reads of Zyn Pay, recovery of its nonces after a restore, and
// collecting what pay returns to the payer — against a stand-in chain.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';

// pocket.js reaches origin.js, which reads Vite's `import.meta.env`; outside
// Vite that is undefined, so the tests give it an empty one.
registerHooks({
  load(url, context, next) {
    const r = next(url, context);
    return url.endsWith('/src/origin.js') ? { ...r, source: String(r.source).replace(/import\.meta\.env/g, '({})') } : r;
  },
});

globalThis.location ??= { origin: 'http://nap.test', search: '' };
const store = new Map();
globalThis.localStorage = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) };

const L = await import('../src/ledger.js');
const mk = await import('../src/market.js');
const { hex, unhex } = await import('../src/object.js');
const { deriveIdentity } = await import('../src/core.js');

const ZEC = '0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201';
const config = { id: 'testnet', chain: 212, rpcUrl: 'http://chain.test/', zecAsset: ZEC, collections: [], bridges: {} };
const identity = deriveIdentity('abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art');
const counter = `nap.market.n.${config.chain}.${identity.accountHex}`;
const nonceAt = (n) => mk.marketNonce(identity.secretKey, n);
const sid = (n) => hex(mk.scheduleId(identity.account, nonceAt(n)));

let schedules; // id -> row
let payments; // id -> row
let claims; // id -> row
const reset = () => { schedules = new Map(); payments = new Map(); claims = new Map(); store.clear(); L.forgetPayUnserved(); };
reset();

globalThis.fetch = async (url, init) => {
  const body = JSON.parse(init.body);
  const answer = (result) => new Response(JSON.stringify({ jsonrpc: '2.0', id: body.id, result }), { status: 200 });
  const ids = String(body.ids || '').split(',').filter(Boolean);
  switch (body.method) {
    case 'zyn_paySchedules': return answer(ids.filter((i) => schedules.has(i)).map((i) => schedules.get(i)));
    case 'zyn_payPayments': return answer(ids.filter((i) => payments.has(i)).map((i) => payments.get(i)));
    case 'zyn_claims': return answer(ids.filter((i) => claims.has(i)).map((i) => claims.get(i)));
    case 'zyn_market': return answer({ epoch: 5, epoch_seconds: 10, pools: [], curves: [] });
    case 'zyn_status': return answer({ chain: 212, epoch: 5 });
    case 'zyn_published': return answer({ outputs: [], nullifiers: [] });
    default: return answer([]);
  }
};

const schedule = (n, o = {}) => ({ id: sid(n), payee: 'aa'.repeat(32), asset: ZEC, amount: '1000000000000000000', every: { unit: 'months', count: 1 }, firstDue: 100, nextDue: 200, index: 2, remaining: 10, pot: '0', reference: '00'.repeat(32), state: 'active', ...o });
const claim = (id) => ({ id, asset: ZEC, amount: '5' });

test('restored_wallet_finds_its_schedules', async () => {
  reset();
  schedules.set(sid(7), schedule(7));
  assert.equal(store.get(counter), undefined); // a fresh device
  const found = await L.loadSchedules(config, identity);
  assert.equal(found.length, 1);
  assert.equal(found[0].id, sid(7));
  assert.equal(Number(store.get(counter)), 8);
});

test('a one-off payment also restores the count', async () => {
  reset();
  payments.set(hex(mk.payOnceId(identity.account, nonceAt(3))), { id: hex(mk.payOnceId(identity.account, nonceAt(3))), payee: 'aa'.repeat(32), asset: ZEC, amount: '1', fee: '0', refunded: '0', at: 1, reference: '00'.repeat(32) });
  assert.equal(await L.recoverNonces(config, identity), 4);
  const once = await L.loadOneOffs(config, identity);
  assert.equal(once.length, 1);
});

test('loadSchedules returns only ids it asked for', async () => {
  reset();
  store.set(counter, '1');
  schedules.set(sid(0), schedule(0));
  const real = globalThis.fetch;
  globalThis.fetch = async (u, init) => {
    const r = await real(u, init);
    const b = JSON.parse(init.body);
    if (b.method !== 'zyn_paySchedules') return r;
    const j = await r.json();
    j.result.push(schedule(0, { id: 'ff'.repeat(32) }));
    return new Response(JSON.stringify(j));
  };
  try { assert.equal((await L.loadSchedules(config, identity)).length, 1); } finally { globalThis.fetch = real; }
});

test('loadPayments reads one id per elapsed period, newest first', async () => {
  reset();
  const s = schedule(0, { index: 3 });
  for (let k = 0; k < 3; k++) { const id = hex(mk.payPaymentId(unhex(s.id), k)); payments.set(id, { id, payee: s.payee, asset: ZEC, amount: '1', fee: '0', refunded: '0', at: k, reference: s.reference }); }
  const got = await L.loadPayments(config, s);
  assert.deepEqual(got.map((p) => p.at), [2, 1, 0]);
});
const unhexId = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));

test('cancel_returns_are_collected', async () => {
  reset();
  store.set(counter, '3');
  const id = hex(mk.claimId(mk.cancelSource(mk.scheduleId(identity.account, nonceAt(1))), 0));
  claims.set(id, claim(id));
  const owed = await L.loadOwed(config, identity);
  const row = owed.find((r) => r.id === id);
  assert.ok(row);
  assert.equal(row.kind, 'Payment returned');
});

test('refunds_are_collected', async () => {
  reset();
  store.set(counter, '2');
  const s = schedule(1, { index: 1 });
  schedules.set(s.id, s);
  const pid = mk.payPaymentId(unhex(s.id), 0);
  payments.set(hex(pid), { id: hex(pid), payee: s.payee, asset: ZEC, amount: '10', fee: '0', refunded: '4', refunds: 2, at: 1, reference: s.reference });
  const rids = [0, 1].map((n) => hex(mk.claimId(mk.refundSource(pid, n), 0)));
  const third = hex(mk.claimId(mk.refundSource(pid, 2), 0));
  for (const r of [...rids, third]) claims.set(r, claim(r));
  const owed = await L.loadOwed(config, identity);
  for (const r of rids) assert.equal(owed.find((x) => x.id === r)?.kind, 'Payment refunded');
  assert.equal(owed.find((x) => x.id === third), undefined); // refunds: 2 means claims 0 and 1
});

const withPayMethods = async (how, run) => {
  const real = globalThis.fetch;
  globalThis.fetch = async (u, init) => {
    const b = JSON.parse(init.body);
    if (!/^zyn_pay/.test(b.method)) return real(u, init);
    if (how === 'network') throw new TypeError('fetch failed');
    return new Response(JSON.stringify({ jsonrpc: '2.0', id: b.id, error: how === 'code' ? { code: -32601, message: 'Method not found' } : { code: -32600, message: 'this chain is an object ledger and does not serve zyn_paySchedules. It serves zyn_status' } }));
  };
  try { return await run(); } finally { globalThis.fetch = real; }
};

test('a node without the pay methods does not stop recovery from keeping the count', async () => {
  for (const how of ['code', 'text']) {
    reset();
    const id = hex(mk.claimId(mk.cancelSource(mk.scheduleId(identity.account, nonceAt(4))), 0));
    claims.set(id, claim(id));
    await withPayMethods(how, async () => { assert.equal(await L.recoverNonces(config, identity), 5); });
    assert.equal(store.get(counter), '5');
    assert.deepEqual(await withPayMethods(how, () => L.loadSchedules(config, identity)), []);
  }
});

test('a network failure on the pay methods still means the count is not kept', async () => {
  reset();
  await withPayMethods('network', () => L.recoverNonces(config, identity, { gap: 1 }));
  assert.equal(store.get(counter), undefined);
});

test('pay reads failing leave the other owed claims in view', async () => {
  reset();
  store.set(counter, '2');
  const id = hex(mk.claimId(mk.orderId(identity.account, nonceAt(0)), 0));
  claims.set(id, claim(id));
  const owed = await withPayMethods('network', () => L.loadOwed(config, identity));
  assert.ok(owed.find((r) => r.id === id));
});

test('recovery asks the pay methods only for pay ids', async () => {
  reset();
  const seen = [];
  const real = globalThis.fetch;
  globalThis.fetch = async (u, init) => { const b = JSON.parse(init.body); if (/^zyn_pay/.test(b.method)) seen.push(b); return real(u, init); };
  try { await L.recoverNonces(config, identity, { gap: 1 }); } finally { globalThis.fetch = real; }
  const per = seen.filter((b) => b.method === 'zyn_paySchedules');
  assert.ok(per.length >= 1);
  const ids = per.flatMap((b) => b.ids.split(','));
  assert.equal(ids.length, 128); // 64 nonces x (schedule, one-off), one page
});

test('loadPayments caps at 500, newest first', async () => {
  reset();
  const s = schedule(0, { index: 700 });
  for (let k = 0; k < 700; k++) { const id = hex(mk.payPaymentId(unhex(s.id), k)); payments.set(id, { id, at: k }); }
  const got = await L.loadPayments(config, s);
  assert.equal(got.length, 500);
  assert.equal(got[0].at, 699);
  assert.equal(got[499].at, 200);
});

test('an old ended schedule is checked for refunds once a day', async () => {
  const stampKey = `nap.pay.settled.${config.chain}.${identity.accountHex}`;
  const now = Math.floor(Date.now() / 1000);
  const setup = () => {
    reset();
    store.set(counter, '2');
    const s = schedule(1, { index: 1, state: 'ended', firstDue: now - 60 * 86400, nextDue: null });
    schedules.set(s.id, s);
    const pid = mk.payPaymentId(unhex(s.id), 0);
    payments.set(hex(pid), { id: hex(pid), payee: s.payee, asset: ZEC, amount: '10', fee: '0', refunded: '4', refunds: 1, at: 1, reference: s.reference });
    return hex(mk.claimId(mk.refundSource(pid, 0), 0));
  };
  let rid = setup();
  store.set(stampKey, String(now - 3600));
  assert.equal((await L.payRefundClaims(config, identity)).has(rid), false);
  assert.equal(store.get(stampKey), String(now - 3600));
  rid = setup();
  assert.equal((await L.payRefundClaims(config, identity)).has(rid), true);
  assert.ok(Number(store.get(stampKey)) >= now);
  rid = setup();
  store.set(stampKey, String(now - 2 * 86400));
  assert.equal((await L.payRefundClaims(config, identity)).has(rid), true);
  assert.ok(Number(store.get(stampKey)) >= now);
  // a failed read leaves the stamp alone
  setup();
  const real = globalThis.fetch;
  globalThis.fetch = async (u, init) => { if (JSON.parse(init.body).method === 'zyn_payPayments') throw new TypeError('fetch failed'); return real(u, init); };
  try { await L.payRefundClaims(config, identity); } finally { globalThis.fetch = real; }
  assert.equal(store.get(stampKey), undefined);
});

test('a hostile refund count is bounded', async () => {
  reset();
  store.set(counter, '8');
  for (let n = 0; n < 6; n++) {
    const id = hex(mk.payOnceId(identity.account, nonceAt(n)));
    payments.set(id, { id, payee: 'aa'.repeat(32), asset: ZEC, amount: '1', fee: '0', refunded: '1', refunds: n === 0 ? 4294967295 : 1024, at: 1, reference: '00'.repeat(32) });
  }
  const one = new Map([...payments].slice(0, 1));
  const keep = payments; payments = one;
  assert.equal((await L.payRefundClaims(config, identity)).size, 1024);
  payments = keep;
  assert.equal((await L.payRefundClaims(config, identity)).size, 4096);
  const bad = hex(mk.payOnceId(identity.account, nonceAt(7)));
  payments = new Map([[bad, { id: bad, refunds: 4294967296 }]]);
  assert.equal((await L.payRefundClaims(config, identity)).size, 0);
});

const onceRow = (n, o = {}) => { const id = hex(mk.payOnceId(identity.account, nonceAt(n))); return { id, payee: 'aa'.repeat(32), asset: ZEC, amount: '1', fee: '0', refunded: '1', refunds: 1, at: 1, reference: '00'.repeat(32), ...o }; };
const firstRefund = (pid) => hex(mk.claimId(mk.refundSource(unhex(pid), 0), 0));

test('a new refund is collected however many old ones there are', async () => {
  reset();
  store.set(counter, '8');
  // Five old one-offs with 1000 refunds each: 5000, past the 4096 cap.
  for (let n = 0; n < 5; n++) { const r = onceRow(n, { refunds: 1000, at: 10 + n }); payments.set(r.id, r); }
  const fresh = onceRow(5, { at: 1000 });
  payments.set(fresh.id, fresh);
  let got = await L.payRefundClaims(config, identity);
  assert.equal(got.size, 4096);
  assert.ok(got.has(firstRefund(fresh.id)), 'the newest one-off is read first');
  // A scheduled payment newer than the old one-offs, too.
  payments.delete(fresh.id);
  const s = schedule(6, { index: 1 });
  schedules.set(s.id, s);
  const pid = hex(mk.payPaymentId(unhex(s.id), 0));
  payments.set(pid, { ...onceRow(0), id: pid, at: 1000 });
  got = await L.payRefundClaims(config, identity);
  assert.equal(got.size, 4096);
  assert.ok(got.has(firstRefund(pid)), 'the newest scheduled payment is read first');
});

test('loadOwed reads pay records beside the claims, not after them', async () => {
  reset();
  store.set(counter, '2');
  let claimsAsked;
  const asked = new Promise((resolve) => { claimsAsked = resolve; });
  const real = globalThis.fetch;
  // The pay reads answer only once the claims have been asked: read one
  // after the other, this never finishes.
  globalThis.fetch = async (u, init) => {
    const b = JSON.parse(init.body);
    if (b.method === 'zyn_claims') claimsAsked();
    if (/^zyn_pay/.test(b.method)) await asked;
    return real(u, init);
  };
  let timer;
  try {
    await Promise.race([L.loadOwed(config, identity), new Promise((_, no) => { timer = setTimeout(() => no(new Error('the pay reads waited for the claim reads')), 3000); })]);
  } finally { clearTimeout(timer); globalThis.fetch = real; }
});

test('a node without Pay is not asked about it again this session', async () => {
  reset();
  store.set(counter, '2');
  const P = await import('../src/pocket.js');
  const asked = [];
  await withPayMethods('text', async () => {
    const inner = globalThis.fetch;
    globalThis.fetch = async (u, init) => { const b = JSON.parse(init.body); if (/^zyn_pay/.test(b.method)) asked.push(b.method); return inner(u, init); };
    try {
      assert.equal((await P.loadPocket(config, identity)).pay.failed, false, 'not served is not a failure');
      assert.ok(asked.length > 0, 'the first refresh asks');
      asked.length = 0;
      const again = await P.loadPocket(config, identity);
      assert.deepEqual(asked, [], 'the next refresh sends no pay requests to that node');
      assert.deepEqual(again.pay, { schedules: [], failed: false });
      // Another node is asked for itself.
      await P.loadPocket({ ...config, rpcUrl: 'http://other.test/' }, identity);
      assert.ok(asked.length > 0);
    } finally { globalThis.fetch = inner; }
  });
});
