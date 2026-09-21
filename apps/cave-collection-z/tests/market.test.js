// The second half of the journey: own → price → sell → buy → leave.
//
// These pages are the first in the store that *move* anything, so the tests
// are about the two ways a page like this goes wrong: calling a method the
// wallet does not have (the page is dead and nobody finds out until a user
// clicks), and writing without going through the wallet at all.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const read = (f) => readFile(new URL(`../${f}`, import.meta.url), 'utf8');
const items = await read('items.html');
const item = await read('item.html');
const market = await read('market.html');
const exit = await read('exit.html');
const background = await readFile(
  new URL('../../extension/src/background.js', import.meta.url),
  'utf8',
);
const build = await read('scripts/build.mjs');

const pages = { items, item, market, exit };

test('every method these pages call exists on the provider', () => {
  // The failure this catches is silent in a browser: a typo'd method is a
  // rejected promise the page reports as "the wallet could not answer",
  // which reads as the user's problem rather than ours.
  for (const [name, page] of Object.entries(pages)) {
    // Every zyn_* string literal on the page, not just the ones written
    // inline as `method:`. Two of these pages route their writes through a
    // shared helper that takes the method as an argument, and a test that
    // only saw the inline form would pass while the write path was dead.
    const called = [...new Set([...page.matchAll(/'(zyn_[a-zA-Z]+)'/g)].map((m) => m[1]))];
    assert.ok(called.length, `${name} calls at least one provider method`);
    for (const m of called) {
      assert.match(background, new RegExp(`\\b${m}\\s*:`), `provider exposes ${m} (called by ${name})`);
    }
  }
});

test('the whole journey is reachable', () => {
  // Each capability, and the page that carries it. A capability with no page
  // is exactly the gap these were built to close (A118).
  const carries = [
    [items, 'zyn_assets'],       // what you hold
    [item, 'zyn_placeOffer'],    // price it
    [item, 'zyn_cancelOffer'],   // take it back
    [item, 'zyn_takeOffer'],     // buy this one
    [market, 'zyn_offers'],      // browse the book
    [market, 'zyn_takeOffer'],   // buy from the book
    [exit, 'zyn_withdraw'],      // leave
    [exit, 'zyn_bind'],          // somewhere to leave to
  ];
  for (const [page, method] of carries) {
    assert.match(page, new RegExp(`'${method}'`), `a page carries ${method}`);
  }
});

test('the pages never reach a node or a service directly', () => {
  // Everything goes through the wallet. A page that talked to a node would
  // be telling that node which items a reader is interested in, which is the
  // linkage the whole design exists to avoid.
  for (const [name, page] of Object.entries(pages)) {
    assert.doesNotMatch(page, /fetch\(/, `${name} makes no direct request`);
    assert.doesNotMatch(page, /127\.0\.0\.1|localhost|:8096|:8099/, `${name} names no node`);
  }
});

test('an item id out of the address bar is validated before use', () => {
  // The id is attacker-supplied text. It is put into a URL and into a
  // provider call, so it has to be exactly 32 bytes of hex or nothing.
  assert.match(item, /\/\^\[0-9a-f\]\{64\}\$\/i/);
});

test('prices and amounts are refused before they reach the wallet', () => {
  // Not because the page is the authority — the chain is — but because a
  // refusal that arrives after an approval prompt has already asked the user
  // to sign is a worse experience than one that arrives instantly.
  for (const page of [item, exit]) {
    assert.match(page, /\/\^\\d\+\(\\\.\\d\+\)\?\$\//);
    assert.match(page, /Number\([a-z]+\) <= 0/);
  }
});

test('the market never offers someone their own listing', () => {
  // The chain refuses it, but a button that exists in order to fail is a
  // button that lies about what it does.
  assert.match(market, /o\.mine/);
  assert.match(market, /Yours/);
});

test('a write is followed by a re-read, never an optimistic update', () => {
  // Whether an item sold is the chain's answer. A page that removed the row
  // itself would show a sale that did not happen.
  for (const page of [item, market, exit]) {
    assert.match(page, /await read\(\)/);
  }
});

test('an unreadable balance is not shown as zero', () => {
  // "0" and "we could not ask" look identical and mean opposite things; one
  // of them tells somebody their money is gone.
  assert.match(exit, /record\.known === false/);
});

test('every new page is staged into the build', () => {
  for (const page of ['items', 'item', 'market', 'exit']) {
    assert.match(build, new RegExp(`'${page}'`), `${page} is built`);
  }
});

test('the pages link to each other, so no capability is an orphan', () => {
  // A page nothing links to is a page nobody finds.
  assert.match(items, /href="\/market"/);
  assert.match(items, /\/item\?id=\$\{/, 'the shelf links each card to its own page');
  assert.match(market, /href="\/items"/);
  assert.match(market, /\/item\?id=\$\{/, 'each row links to the object it sells');
  assert.match(exit, /href="\/deposit"/);
  assert.match(item, /href="\/items"/);
});
