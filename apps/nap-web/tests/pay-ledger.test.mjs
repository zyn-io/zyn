// Nap's reads of Zyn Pay, recovery of its nonces after a restore, and
// collecting what pay returns to the payer — against a stand-in chain.
import { test } from 'node:test';
import assert from 'node:assert/strict';

const store = new Map();
globalThis.localStorage = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) };

const L = await import('../src/ledger.js');
const mk = await import('../src/market.js');
const { hex } = await import('../src/object.js');
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
const reset = () => { schedules = new Map(); payments = new Map(); claims = new Map(); store.clear(); };
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
  for (let k = 0; k < 3; k++) { const id = hex(mk.payPaymentId(unhexId(s.id), k)); payments.set(id, { id, payee: s.payee, asset: ZEC, amount: '1', fee: '0', refunded: '0', at: k, reference: s.reference }); }
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
  const pid = mk.payPaymentId(unhexId(s.id), 0);
  payments.set(hex(pid), { id: hex(pid), payee: s.payee, asset: ZEC, amount: '10', fee: '0', refunded: '4', at: 1, reference: s.reference });
  const rid = hex(mk.claimId(mk.refundSource(pid, 1), 0));
  claims.set(rid, claim(rid));
  const owed = await L.loadOwed(config, identity);
  const row = owed.find((r) => r.id === rid);
  assert.ok(row);
  assert.equal(row.kind, 'Payment refunded');
});
