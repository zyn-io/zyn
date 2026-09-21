import assert from 'node:assert/strict';
import { test } from 'node:test';

import { shapeStatus } from '../functions/api/status.js';
import { normalizeStatus } from '../status.js';

/// A real `getblockchaininfo` result, taken from the mainnet node on
/// 18 Sep 2026 and trimmed of nothing — the point of the first test is that
/// the endpoint drops all of this on its own.
const LIVE = {
  chain: 'main',
  blocks: 3_487_497,
  headers: 3_487_497,
  difficulty: 217676167.3140142,
  verificationprogress: 0.9999997132614843,
  chainwork: 0,
  pruned: false,
  size_on_disk: 278907129012,
  commitments: 0,
  bestblockhash: '0000000000311ec0376a562f299d0605e751ce98cb02d1bda966069939786539',
  estimatedheight: 3_487_498,
  chainSupply: { chainValue: 16938847.0405448, chainValueZat: 1693884704054480, monitored: true },
  valuePools: [{ id: 'transparent', chainValue: 11956521.5438787, monitored: true }],
};

test('the shape is the allowlist: six fields, and the node tells the browser nothing else', () => {
  const shaped = shapeStatus(LIVE, new Date('2026-09-18T10:00:00Z'), 28);
  assert.deepEqual(Object.keys(shaped).sort(), [
    'estimatedHeight', 'height', 'latencyMs', 'network', 'sampledAt', 'tip',
  ]);
  assert.equal(shaped.height, 3_487_497);
  assert.equal(shaped.estimatedHeight, 3_487_498);
  assert.equal(shaped.network, 'main');
  assert.equal(shaped.latencyMs, 28);
});

test('what the endpoint serves is what the page accepts', () => {
  const shaped = shapeStatus(LIVE, new Date(), 28);
  const status = normalizeStatus(shaped);
  assert.equal(status.state, 'online');
  assert.equal(status.gap, 1);
  assert.equal(status.stale, false);
});

test('an estimate behind the tip reads as caught up, not as a negative gap', () => {
  // The page refuses estimatedHeight < height, so passing this through raw
  // would take a synced node off the board.
  const shaped = shapeStatus({ ...LIVE, estimatedheight: LIVE.blocks - 2 }, new Date(), 9);
  assert.equal(shaped.estimatedHeight, LIVE.blocks);
  assert.equal(normalizeStatus(shaped).gap, 0);
});

test('a missing estimate falls back to the tip rather than to NaN', () => {
  const { estimatedheight, ...withoutEstimate } = LIVE;
  assert.equal(shapeStatus(withoutEstimate, new Date(), 9).estimatedHeight, LIVE.blocks);
});

test('a tip that is not a block hash is refused here, not rendered there', () => {
  for (const bad of ['', 'deadbeef', null, 'z'.repeat(64)]) {
    assert.throws(() => shapeStatus({ ...LIVE, bestblockhash: bad }, new Date(), 9), TypeError);
  }
});

test('a result that is not a result is refused', () => {
  for (const bad of [null, undefined, 'main', 42]) {
    assert.throws(() => shapeStatus(bad, new Date(), 9), TypeError);
  }
  assert.throws(() => shapeStatus({ ...LIVE, blocks: 0 }, new Date(), 9), TypeError);
});

test('nothing served to a visitor names a node they could call', async () => {
  const { readFile, readdir } = await import('node:fs/promises');
  // The page once printed `zec-rpc.cavebro.win` as a header coordinate. It
  // was decorative, and it was also the whole address.
  for (const name of await readdir(new URL('../dist/', import.meta.url))) {
    if (!/\.(html|js|css)$/.test(name)) continue;
    const served = await readFile(new URL(`../dist/${name}`, import.meta.url), 'utf8');
    assert.doesNotMatch(served, /zec-rpc|jsonrpc|getblockchaininfo|95\.111\.240\.24/i, name);
  }
});

test('the upstream is configuration, never a literal in the function', async () => {
  const { readFile } = await import('node:fs/promises');
  const source = await readFile(new URL('../functions/api/status.js', import.meta.url), 'utf8');
  assert.match(source, /env\.ZEC_RPC_URL/);
  assert.doesNotMatch(source, /https:\/\/zec-rpc/);
  // An upstream error echoed to the caller is how the address gets out.
  assert.doesNotMatch(source, /error:\s*(String\(|err|e\.message)/);
});
