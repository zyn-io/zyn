import test from 'node:test';
import assert from 'node:assert/strict';
import { advanceLive, createLiveState, decimalToUnits, LiveApiError, unitsToDecimal } from '../src/live.js';
import { resolveBridgeRoute } from '../vite.config.js';

function fakeDaemon() {
  const chain = { owner: null, pool: '0.03000000', bound: false, exiting: [], offers: [], seq: 10, epoch: 5 };
  const calls = [];
  const overview = role => ({
    available: true,
    chain: 11,
    account: `${role === 'creator' ? 'cc' : role === 'a' ? 'aa' : 'bb'}${'0'.repeat(62)}`,
    status: { ok: true, seq: chain.seq, epoch: chain.epoch },
    record: { bound_here: role === 'a' && chain.bound, spendable: [{ asset: 1, symbol: 'ZEC.zy', amount: role === 'a' ? '0.02000000' : '0.02000000' }], exiting: chain.exiting },
    collections: [{ id: 42, pool: chain.pool, outstanding: chain.owner ? 1 : 0, redeem_price: '0.00500000', fee_bps: 100, phase_name: 'Open' }],
    items: chain.owner === role ? [{ asset: 9, collection: 42, content: 'cb08f65eec9fd64d89c240ab2a0f8ec0f0db77f0ede32b4196c28d47158c17dd', redeem_price: '0.00500000' }] : [],
  });
  return {
    calls,
    chain,
    async get(role, path) {
      calls.push({ method: 'GET', role, path });
      if (path === '/api/overview') return overview(role);
      if (path === '/api/offers') return { offers: chain.offers };
      if (path === '/api/anchors') return { anchors: [{ epoch: 30, txid: 'deadcafe', height: 123, root: 'aa', anchor_id: 'bb' }] };
      if (path === '/api/deposit-address') return { address: 'utest', memo: false };
      throw new Error(path);
    },
    async post(role, path, body) {
      calls.push({ method: 'POST', role, path, body });
      chain.seq++; chain.epoch++;
      if (path === '/api/fund') chain.pool = '0.03500000';
      if (path === '/api/mint') chain.owner = 'a';
      if (path === '/api/offer') chain.offers = [{ id: 77, maker: overview('a').account, offer_asset: 9, want_amount: '0.00600000' }];
      if (path === '/api/take') { chain.owner = 'b'; chain.offers = []; }
      if (path === '/api/force') return Object.assign((chain.owner = 'a', {}), { forced: true, txid: 'feedface' });
      if (path === '/api/redeem') chain.owner = null;
      if (path === '/api/bind') chain.bound = true;
      if (path === '/api/withdraw') chain.exiting.push({ amount: body.amount });
      return { ok: true, seq: chain.seq, epoch: chain.epoch };
    },
  };
}

test('decimal amounts round-trip without JavaScript floats', () => {
  assert.equal(decimalToUnits('0.006'), 600000n);
  assert.equal(unitsToDecimal(600000n), '0.00600000');
  assert.throws(() => decimalToUnits('0.000000001'), /more than 8/);
});

test('bridge admits only explicit role, method, and route triples', () => {
  assert.deepEqual(resolveBridgeRoute('GET', '/a/api/overview'), { role: 'a', path: '/api/overview', search: '' });
  assert.deepEqual(resolveBridgeRoute('POST', '/creator/api/mint'), { role: 'creator', path: '/api/mint', search: '' });
  assert.equal(resolveBridgeRoute('POST', '/a/api/overview'), null);
  assert.equal(resolveBridgeRoute('GET', '/a/api/withdraw'), null);
  assert.equal(resolveBridgeRoute('POST', '/a/api/not-real'), null);
  assert.equal(resolveBridgeRoute('POST', '/admin/api/mint'), null);
  assert.equal(resolveBridgeRoute('POST', '/a/api/mint/extra'), null);
});

test('live flow uses three wallets and the real forced-transfer payload', async () => {
  const api = fakeDaemon();
  let state = createLiveState({ collectionId: 42 });
  for (let n = 0; n < 8; n++) state = await advanceLive(state, api);
  assert.equal(state.step, 7);
  assert.equal(state.anchor.txid, 'deadcafe');
  assert.deepEqual(api.calls.filter(call => call.method === 'POST').map(call => `${call.role}:${call.path}`), [
    'a:/api/fund', 'creator:/api/mint', 'a:/api/offer', 'b:/api/take', 'b:/api/force', 'a:/api/redeem', 'a:/api/bind', 'a:/api/withdraw',
  ]);
  const force = api.calls.find(call => call.path === '/api/force');
  assert.deepEqual(force.body, { to: `aa${'0'.repeat(62)}`, asset: 9, amount: '1' });
  assert.ok(state.events.every(event => !String(event.id).startsWith('SIM')));
});

test('an uncertain fund is read back and never automatically retried', async () => {
  const api = fakeDaemon();
  let state = createLiveState({ collectionId: 42 });
  state = await advanceLive(state, api);
  state = await advanceLive(state, api);
  const originalPost = api.post.bind(api);
  let first = true;
  api.post = async (role, path, body) => {
    if (first && path === '/api/fund') {
      first = false;
      api.chain.pool = '0.03500000';
      api.calls.push({ method: 'POST', role, path, body });
      throw new LiveApiError('timeout', { uncertain: true });
    }
    return originalPost(role, path, body);
  };
  state = await advanceLive(state, api);
  assert.equal(state.unknown.path, '/api/fund');
  state = await advanceLive(state, api);
  assert.equal(state.unknown, null);
  assert.equal(state.claimFunded, true);
  state = await advanceLive(state, api);
  assert.equal(state.step, 2);
  assert.equal(api.calls.filter(call => call.path === '/api/fund').length, 1);
});
