import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const page = await readFile(new URL('../mint.html', import.meta.url), 'utf8');
const relay = await readFile(new URL('../functions/api/sale/[[path]].js', import.meta.url), 'utf8');
const background = await readFile(new URL('../../extension/src/background.js', import.meta.url), 'utf8').catch(() => null);

// The wallet provider lives in a different repository. When the store is
// checked out on its own, these cross-repo assertions cannot run — so they
// skip with a reason rather than passing quietly. A silent pass here would
// mean the store could call a method the wallet does not have and nothing
// would say so, which is the exact failure this file exists to catch.
const WALLET = background
  ? false
  : 'apps/extension is not checked out — run in the monorepo to check the store against the wallet provider';


test('the checkout quotes against the collection receiving account, not the identity', async () => {
  // §97.5: the money and the item must land in an account with no other
  // history. Quoting against zyn_account would put the buyer's identity in the
  // journal and require a move afterwards — and the move is the leak.
  assert.match(page, /method: 'zyn_receivingAccount'/);
  assert.doesNotMatch(page, /method: 'zyn_account'/);
});

test('the methods it calls exist on the provider', { skip: WALLET }, () => {
  for (const method of ['zyn_receivingAccount', 'zyn_receivingRead']) {
    assert.match(page, new RegExp(`method: '${method}'`), `page calls ${method}`);
    assert.match(background, new RegExp(`${method}:`), `provider exposes ${method}`);
  }
});

test('the page reaches the sale only through the relay, never a service address', () => {
  assert.match(page, /fetch\('\/api\/sale'/);
  assert.doesNotMatch(page, /https?:\/\/[^'"]*:8096|ZYN_SALE|127\.0\.0\.1/);
});

test('the relay names the calls it allows and no prefix', () => {
  // A forwarded prefix would publish every route zyn-sale ever grows.
  for (const call of ['POST /quote', 'POST /settle', 'GET /order']) {
    assert.ok(relay.includes(`'${call}'`), `${call} is allowed`);
  }
  assert.match(relay, /env\.ZYN_SALE_URL/);
  assert.doesNotMatch(relay, /https?:\/\/\d/, 'the upstream must not be a literal');
});

test('the relay never echoes the upstream', () => {
  assert.match(relay, /the sale service is not answering/);
  assert.doesNotMatch(relay, /error:\s*(String\(|e\.message|err)/);
});

test('it states the odds where the money is asked for', () => {
  // The weights were published before the sale; a buyer should not have to
  // find them in a document to know what an entry buys.
  assert.match(page, /80%/);
  assert.match(page, /19%/);
  assert.match(page, /1%/);
  assert.match(page, /one item/);
  assert.match(page, /three items/);
});

test('it does not promise an item at payment time', () => {
  // The draw runs once, after the close (§104.1). A checkout that implied
  // otherwise would be lying about the one thing the design is built around.
  assert.match(page, /Nothing is assigned yet/i);
  assert.match(page, /draw runs once/i);
});

test('waiting for the draw reads as waiting, not as failure', () => {
  assert.match(page, /draw has not run/i);
  assert.match(page, /Paid and counted/);
});

test('the built site serves the checkout at /mint', async () => {
  const built = await readFile(new URL('../dist/mint/index.html', import.meta.url), 'utf8');
  assert.equal(built, page, 'dist/mint/index.html is the page');
});

test('the entry is paid, not merely proven', { skip: WALLET }, async () => {
  // The floor buyers are trusting is funded by the entries. A checkout that
  // proved a balance and moved nothing left the pool funded by nobody (§113.8).
  assert.match(page, /method: 'zyn_receivingPay'/);
  assert.match(page, /payment: signed\.frame/);
  assert.match(background, /zyn_receivingPay:/);
});

test('the payment is signed by the wallet and sent by the sale', { skip: WALLET }, () => {
  // Signing without sending: a payment that leaves the wallet but never
  // reaches the sale is a buyer who paid for nothing.
  assert.match(background, /zyn_receivingPay: \{ read: false/, 'paying is approval-gated');
  assert.doesNotMatch(page, /submit_raw|\/api\/submit/, 'the page never submits directly');
});

test('it names where the money goes', () => {
  assert.match(page, /90% of what you pay funds the pool/i);
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
  assert.equal(digest, 'a124489b1dabcefc2cc54575af51d76edf4afd865e4cc8a71530c752cc2e7af3');
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
