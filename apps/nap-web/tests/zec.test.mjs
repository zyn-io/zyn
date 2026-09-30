// The built Zcash core, as the page loads it: keys must be the desktop's.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { initSync, spendingKeyFromPhrase, ZecWallet, blocksRequest, replyBody } from '../src/zec-pkg/nap_zec.js';

initSync({ module: readFileSync(new URL('../src/zec-pkg/nap_zec_bg.wasm', import.meta.url)) });
const WORDS = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art';
const hex = (b) => Buffer.from(b).toString('hex');

// zynzapd::wallet::recovery_tests pins the same two keys. If the browser
// derived anything else, it would show an empty wallet to someone whose ZEC
// sits at the address desktop Nap gave them.
test('the browser derives the desktop Zcash keys', () => {
  assert.equal(hex(spendingKeyFromPhrase(WORDS, '', 'mainnet', 0)), 'ba36a915cf195348f4f8bcd584e2635735778190071db2f2af78d80b44a97bcb');
  assert.equal(hex(spendingKeyFromPhrase(WORDS, '', 'testnet', 0)), '13548ce4ac2ec6cc6b93a8f66f50ddce51e9e156cc084d3cef279873446f1b1b');
});

test('addresses are for their own network', () => {
  const main = new ZecWallet(spendingKeyFromPhrase(WORDS, '', 'mainnet', 0), 'mainnet');
  const test_ = new ZecWallet(spendingKeyFromPhrase(WORDS, '', 'testnet', 0), 'testnet');
  assert.match(main.address(), /^u1[a-z0-9]+$/);
  assert.match(test_.address(), /^utest1[a-z0-9]+$/);
});

test('a bad phrase and a server refusal are errors, not garbage', () => {
  assert.throws(() => spendingKeyFromPhrase('abandon art', '', 'mainnet', 0), /recovery phrase/);
  const refusal = new Uint8Array([1, 4, 0, ...Buffer.from('nope')]);
  assert.throws(() => replyBody(refusal), /nope/);
  assert.deepEqual([...replyBody(new Uint8Array([0, 7]))], [7]);
  // BLOCKS: op 2, from u64 LE, count u32 LE.
  assert.deepEqual([...blocksRequest(258, 100)], [2, 2, 1, 0, 0, 0, 0, 0, 0, 100, 0, 0, 0]);
});
