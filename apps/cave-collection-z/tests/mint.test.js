import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const page = await readFile(new URL('../mint.html', import.meta.url), 'utf8');
const zstore = await readFile(new URL('../zstore.js', import.meta.url), 'utf8');
const napOps = await readFile(new URL('../../nap-web/src/connect-ops.js', import.meta.url), 'utf8').catch(() => null);
const background = await readFile(new URL('../../extension/src/background.js', import.meta.url), 'utf8').catch(() => null);

// The wallet provider lives beside the store in the monorepo. When the store
// is checked out on its own, these cross-repo assertions cannot run — so they
// skip with a reason rather than passing quietly.
const WALLET = napOps && background
  ? false
  : 'apps/nap-web and apps/extension are not checked out — run in the monorepo to check the store against both Naps';

// The sale runs on the object ledger (§144): tickets bought in ZEC.zy through
// Nap, a draw posted as a root, claims proved against it, then an open mint.
const SALE = ['zyn_saleTickets', 'zyn_buyTickets', 'zyn_claimItems', 'zyn_mintOpen'];

test('the checkout is the store\'s mint page, on Nap', () => {
  assert.match(page, /<body data-page="mint">/);
  assert.match(page, /<script type="module" src="\/zstore\.js/);
  assert.match(zstore, /mint: mintPage/);
  assert.match(zstore, /'\/mint\/': 'mint'/);
});

test('every step of the sale is a Nap call, served by both Naps', { skip: WALLET }, () => {
  for (const m of SALE) {
    assert.match(zstore, new RegExp(`'${m}'`), `the page asks for ${m}`);
    assert.match(napOps, new RegExp(`\\n  ${m}: \\{`), `Nap in the browser serves ${m}`);
    assert.match(background, new RegExp(`\\b${m}\\s*:`), `the extension serves ${m}`);
  }
});

test('the retired checkout is gone: no payment address, no relay, no reservation', () => {
  assert.doesNotMatch(page + zstore, /\/api\/sale|zyn_receiving|isZynZap|Invitation code/);
});

test('it states the odds where the money is asked for', () => {
  // The weights were published before the sale; a buyer should not have to
  // find them in a document to know what a ticket buys.
  for (const s of ['80%', '19%', '1%', 'one item', 'three items']) assert.ok(page.includes(s), s);
});

test('it does not promise an item at payment time', () => {
  // The draw runs once, after the close (§104.1). A checkout that implied
  // otherwise would be lying about the one thing the design is built around.
  assert.match(page, /decided after the door shuts/i);
  assert.match(page, /runs\s+once/i);
});

test('the price the holder approves is the price the page showed', () => {
  // Nap refuses a buy whose `pay` differs from the chain's price, so a page
  // that quietly showed a stale price cannot get it approved.
  assert.match(zstore, /zyn_buyTickets', \(\) => \{[\s\S]*?pay: zec\(/);
});

test('the built site serves the checkout at /mint', async () => {
  const built = await readFile(new URL('../dist/mint/index.html', import.meta.url), 'utf8');
  assert.equal(built, page, 'dist/mint/index.html is the page');
});

test('it names where the money goes', () => {
  assert.match(page, /90% funds the pool/i);
  assert.match(page, /10% is the treasury/i);
});

test('the published terms page carries bytes that reproduce the digest', async () => {
  const page = await readFile(new URL('../terms.html', import.meta.url), 'utf8');
  const hex = (page.match(/id="bytes">([\s\S]*?)<\/div>/) || [])[1].replace(/\s+/g, '');
  assert.ok(/^[0-9a-f]+$/.test(hex) && hex.length % 2 === 0, 'the page carries hex bytes');

  // The whole point of publishing bytes is that someone can recompute the
  // digest from them. If this ever fails, the page is claiming a commitment it
  // does not carry.
  const { createHash } = await import('node:crypto');
  const payload = Buffer.from(hex, 'hex');
  const domain = Buffer.from('cave.sale.terms.v1');
  const len = (n) => { const b = Buffer.alloc(4); b.writeUInt32BE(n); return b; };
  const digest = createHash('sha256')
    .update(len(domain.length)).update(domain)
    .update(len(payload.length)).update(payload)
    .digest('hex');
  assert.equal(digest, 'f2c3242060baf60e0c193b41dc726b6677d85c158a62ba200c948d48a177a59e');
  assert.ok(page.includes(digest), 'and states the digest it reproduces');
});

test('the raw bytes served beside the page are the same bytes', async () => {
  const page = await readFile(new URL('../terms.html', import.meta.url), 'utf8');
  const inPage = (page.match(/id="bytes">([\s\S]*?)<\/div>/) || [])[1].replace(/\s+/g, '');
  const raw = (await readFile(new URL('../dist/cave-sale-terms-v1.hex', import.meta.url), 'utf8')).trim();
  assert.equal(raw, inPage, 'the downloadable file and the page must not drift');
});

test('the terms page says what it does not prove', async () => {
  // It is our own web server: it cannot date its own contents. Saying so is
  // the difference between publishing and claiming.
  const page = await readFile(new URL('../terms.html', import.meta.url), 'utf8');
  assert.match(page, /does not yet prove/i);
  assert.match(page, /has not happened yet/i);
});

test('the landing page offers the terms', async () => {
  const landing = await readFile(new URL('../index.html', import.meta.url), 'utf8');
  assert.match(landing, /href="\/terms"/);
});
