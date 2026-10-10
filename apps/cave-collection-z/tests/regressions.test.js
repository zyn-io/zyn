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

test('a burn asks for no more credit than the sale has left, and never shares a code', () => {
  // Past the budget a burn gets only what is left, and the tokens are gone either way.
  assert.match(zstore, /if \(want > budget\) return fail\(/);
  assert.match(zstore, /try \{ sale = await chainSale\(\); \}/, 'the sale is read again right before a burn');
  // A reused memo is refused by the chain; the second burn earned nothing.
  assert.match(zstore, /ask\('zyn_burnMemo', \{ fresh: true \}\)/);
});

test('counts are whole numbers, checked before they reach BigInt or Nap', () => {
  for (const id of ['count-in', 'mint-in']) assert.match(zstore, new RegExp(`whole\\('${id}'`), `${id} is checked`);
  // Credit is a decimal of ZEC, read to WAD without a float.
  assert.match(zstore, /if \(!\/\^\\d\*\\\.\?\\d\+\$\/\.test\(v \|\| ''\)\) return null;/);
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
  assert.match(whitelist, /if \(digging && digHandle === handle\) return digging;/);
  assert.doesNotMatch(whitelist, /Reload and try again/);
  assert.doesNotMatch(whitelist, /Places are given, not bought/);
  // Since §168 the list gates the sale: the page must not say it gates nothing.
  assert.doesNotMatch(whitelist, /gates nothing|anyone can buy/);
  assert.match(whitelist, /Only accounts on this list can buy tickets/);
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
  const kept = [...m.keys()].filter((k) => k.startsWith('role:lookout:@alice:'));
  assert.equal(kept.length, 1);
  assert.equal(JSON.parse(m.get(kept[0])).note, 'run a mainnet replica');
  // Someone else's entry under the same handle does not block this one.
  assert.deepEqual(await post({ ...good, note: 'squatter' }), { ok: true });
  assert.equal([...m.keys()].filter((k) => k.startsWith('role:lookout:@alice:')).length, 2);
  // The honeypot answers as accepted and stores nothing.
  assert.deepEqual(await post({ ...good, role: 'digger', burrow: 'x' }), { ok: true });
  assert.equal([...m.keys()].filter((k) => k.startsWith('role:digger:')).length, 0);
});

test('a proposal text is kept only once the chain has a proposal with its hash', async () => {
  const fn = await import('../functions/api/proposal-text.js');
  const m = new Map();
  const env = { CAVE_WHITELIST: { get: async (k) => m.get(k) ?? null, put: async (k, v) => { m.set(k, v); } } };
  const { createHash } = await import('node:crypto');
  const words = 'Fund a mainnet replica for a year.';
  const hash = createHash('sha256').update(words).digest('hex');
  let onChain = [];
  const chain = async () => new Response(JSON.stringify({ result: { set: true, proposals: onChain.map((text) => ({ text })) } }));
  const post = (text, network = 'testnet') => fn.onRequestPost({ env, request: new Request('https://cavebro.win/api/proposal-text', { method: 'POST', body: JSON.stringify({ text, network }) }) }, chain);
  // Not proposed yet: refused, and nothing is stored.
  assert.equal((await post(`  ${words}  `)).status, 409);
  assert.equal(m.size, 0, 'an unproposed text leaves nothing behind');
  onChain = [hash];
  const r = await (await post(`  ${words}  `)).json();
  assert.equal(r.hash, hash, 'trimmed, as Nap hashes it');
  const get = (h, network = 'testnet') => fn.onRequestGet({ env, request: new Request(`https://cavebro.win/api/proposal-text?hash=${h}&network=${network}`) });
  assert.equal((await (await get(hash)).json()).text, words);
  assert.equal((await get(hash, 'mainnet')).status, 404, 'kept per network');
  assert.equal((await post('x'.repeat(4001))).status, 400);
  assert.equal((await get('0'.repeat(64))).status, 404);
});

test('a proposal made closes its form, so the list behind it can be used', async () => {
  const zstore = await read('zstore.js');
  const sent = zstore.slice(zstore.indexOf("await ask('zyn_daoPropose'"));
  assert.ok(sent.slice(0, 600).includes('propDialog.close();'), 'the dialog closes once Nap has proposed');
});
