import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { formatHeight, normalizeStatus, shortHash, STALE_AFTER_MS } from '../status.js';

const sample = {
  network: 'main',
  height: 3_487_459,
  estimatedHeight: 3_487_461,
  tip: '000000000039a1c000c527d13c04f107f753af0c7cd097724549d419e0082d43',
  sampledAt: '2026-09-18T07:34:45.000Z',
  latencyMs: 27.6,
};

test('normalizes the narrow public status contract', () => {
  const status = normalizeStatus(sample, Date.parse(sample.sampledAt) + 1_000);
  assert.equal(status.state, 'online');
  assert.equal(status.gap, 2);
  assert.equal(status.latencyMs, 28);
  assert.equal(formatHeight(status.height), '3,487,459');
  assert.equal(shortHash(status.tip), '000000000039a1c0…4549d419e0082d43');
});

test('treats old evidence as stale instead of online', () => {
  const status = normalizeStatus(sample, Date.parse(sample.sampledAt) + STALE_AFTER_MS + 1);
  assert.equal(status.state, 'stale');
});

test('rejects wrong-network and malformed evidence', () => {
  assert.throws(() => normalizeStatus({ ...sample, network: 'test' }), /mainnet/);
  assert.throws(() => normalizeStatus({ ...sample, tip: 'not-a-hash' }), /block hash/);
  assert.throws(() => normalizeStatus({ ...sample, estimatedHeight: sample.height - 1 }), /estimated height/);
});

test('production frontend only connects to the sanitized status endpoint', async () => {
  const source = await readFile(new URL('../app.js', import.meta.url), 'utf8');
  assert.match(source, /fetch\('\/api\/status'/);
  assert.doesNotMatch(source, /getblockchaininfo|sendrawtransaction|zec-rpc\.cavebro\.win/);
});

test('page names the private boundary plainly', async () => {
  const html = await readFile(new URL('../index.html', import.meta.url), 'utf8');
  assert.match(html, /public<\/span><b>signal<\/b>/);
  assert.match(html, /private<\/span><b>rpc<\/b>/);
  assert.match(html, /every other door stays shut/i);
});
