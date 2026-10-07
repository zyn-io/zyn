// Bugs found in the line-by-line review of 26 Sep 2026, pinned so they stay
// fixed. Each test names what went wrong for a holder.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const read = (f) => readFile(new URL(`../${f}`, import.meta.url), 'utf8');
const zstore = await read('zstore.js');
const whitelist = await read('whitelist.html');
const fn = await read('functions/api/whitelist.js');
const index = await read('index.html');
const styles = await read('styles.css');

test('a payment is sent in full, not cut to 8 decimals', () => {
  // A 1% fee on an 8-decimal price has 10: cut, Nap refused every buy of it.
  assert.doesNotMatch(zstore, /pay: zec\(/, 'no payment goes through the display rounding');
  assert.equal((zstore.match(/pay: exact\(/g) || []).length, 3, 'item buy, market buy and tickets pay exactly');
});

test('a burn is capped at the burn tickets left, and never shares a code', () => {
  // Over the cap the chain refuses the whole burn and the tokens are gone.
  assert.match(zstore, /const n = whole\('burn-in', max\);/);
  assert.match(zstore, /try \{ sale = await chainSale\(\); \}/, 'the sale is read again right before a burn');
  // A reused memo is refused by the chain; the second burn earned nothing.
  assert.match(zstore, /ask\('zyn_burnMemo', \{ fresh: true \}\)/);
});

test('counts are whole numbers, checked before they reach BigInt or Nap', () => {
  for (const id of ['count-in', 'burn-in', 'mint-in']) assert.match(zstore, new RegExp(`whole\\('${id}'`), `${id} is checked`);
  assert.doesNotMatch(zstore, /Math\.min\(20, Number\(el\('count-in'\)\.value\)\)/);
});

test('moving between pages carries each page\'s own styles', () => {
  // Only <main> used to move, and /mint rendered in /items' layout.
  assert.match(zstore, /querySelectorAll\('head style'\)\.forEach\(\(st\) => st\.remove\(\)\)/);
  assert.match(zstore, /next\.querySelectorAll\('head style'\)/);
});

test('a read from a page since left writes nothing', () => {
  const reads = zstore.match(/if \(gen !== generation\) return;/g) || [];
  assert.ok(reads.length >= 15, `generation checked after awaits (${reads.length})`);
});

test('"nothing was paid" is said only when Nap said no', () => {
  assert.match(zstore, /const failText = \(e, no\) => \(e\?\.code === 4001 \? no : e\?\.message/);
  assert.doesNotMatch(zstore, /e\.code === 4001 \? 'Not approved\. Nothing was paid\.' : e\.message/);
});

test('the whitelist runs one dig at a time and never says "reload"', () => {
  assert.match(whitelist, /if \(dig && digHandle === handle\) return dig;/);
  assert.doesNotMatch(whitelist, /Reload and try again/);
  assert.doesNotMatch(whitelist, /Places are given, not bought/, 'the list gates nothing (§144.1)');
  assert.match(whitelist, /inset: -20% 0 auto;/, 'the glow stays inside a phone screen');
});

test('the whitelist function refuses a body that is not a form', () => {
  assert.match(fn, /if \(!form \|\| typeof form !== 'object' \|\| Array\.isArray\(form\)\)/);
  assert.match(fn, /\.slice\(0, 300\)/, 'the link has a length cap');
});

test('the landing page no longer says sealed', () => {
  assert.doesNotMatch(index, /sealed|unrevealed/i);
  // Waitlist stage: the button leads to the list, not the sale.
  assert.match(index, /<a class="enter-button" href="\/whitelist">/);
});

test('nothing widens a phone screen', () => {
  assert.match(styles, /html \{ scroll-behavior: smooth; overflow-x: clip; \}/);
});

test('a role sign-up keeps to its role, its limits and one entry per contact', async () => {
  const { onRequestPost } = await import('../functions/api/roles.js');
  const m = new Map();
  const env = { CAVE_WHITELIST: { get: async (k) => m.get(k) ?? null, put: async (k, v) => { m.set(k, v); } } };
  const post = (body) => onRequestPost({ env, request: new Request('https://cavebro.win/api/roles', { method: 'POST', body: JSON.stringify(body), headers: { 'cf-connecting-ip': '1.2.3.4' } }) }).then((r) => r.json());
  const good = { role: 'lookout', contact: '@alice', note: 'run a mainnet replica', dwell: 5000 };
  assert.equal((await post({ ...good, role: 'admin' })).ok, false);
  assert.equal((await post({ ...good, dwell: 10 })).ok, false);
  assert.equal((await post({ ...good, note: 'x'.repeat(281) })).ok, false);
  assert.deepEqual(await post(good), { ok: true });
  assert.deepEqual(await post(good), { ok: true, already: true });
  assert.equal(JSON.parse(m.get('role:lookout:@alice')).note, 'run a mainnet replica');
  // The honeypot answers as accepted and stores nothing.
  assert.deepEqual(await post({ ...good, role: 'digger', burrow: 'x' }), { ok: true });
  assert.equal(m.get('role:digger:@alice'), undefined);
});
