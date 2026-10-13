import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const page = await readFile(new URL('../mint.html', import.meta.url), 'utf8');
const zstore = await readFile(new URL('../zstore.js', import.meta.url), 'utf8');
const napOps = await readFile(new URL('../../nap-web/src/connect-ops.js', import.meta.url), 'utf8').catch(() => null);
// Since §148 the extension is Nap in the browser, packaged: its build ships
// nap-web's pages, and its connect window answers every method from the same
// connect-ops.js. So "both Naps serve it" is: connect-ops has it, the connect
// window dispatches connect-ops, and the extension build ships that window.
const connectApp = await readFile(new URL('../../nap-web/src/connect-app.js', import.meta.url), 'utf8').catch(() => null);
const extBuild = await readFile(new URL('../../extension/build.mjs', import.meta.url), 'utf8').catch(() => null);
const content = await readFile(new URL('../../extension/src/content.js', import.meta.url), 'utf8').catch(() => null);

// The wallet provider lives beside the store in the monorepo. When the store
// is checked out on its own, these cross-repo assertions cannot run — so they
// skip with a reason rather than passing quietly.
const WALLET = napOps && connectApp && extBuild && content
  ? false
  : 'apps/nap-web and apps/extension are not checked out — run in the monorepo to check the store against both Naps';

// The sale runs on the object ledger (§187): straight to an open mint through
// Nap, at this wallet's price (the whitelist's or the open one), paid in
// ZEC.zy or with burn credit.
const SALE = ['zyn_mintOpen', 'zyn_saleAccess', 'zyn_saleCredit', 'zyn_burnMemo'];

test('the checkout is the store\'s mint page, on Nap', () => {
  assert.match(page, /<body data-page="mint">/);
  assert.match(page, /<script type="module" src="\/zstore\.js/);
  assert.match(zstore, /mint: mintPage/);
  assert.match(zstore, /'\/mint\/': 'mint'/);
});

test('every step of the sale is a Nap call, served by both Naps', { skip: WALLET }, () => {
  for (const m of SALE) {
    assert.match(zstore, new RegExp(`'${m}'`), `the page asks for ${m}`);
    assert.match(napOps, new RegExp(`\\n  ${m}: \\{`), `Nap serves ${m}`);
  }
  assert.match(connectApp, /import \{[^}]*\bMETHODS\b[^}]*\} from '\.\/connect-ops\.js'/, 'the connect window answers from connect-ops');
  assert.match(extBuild, /'build:extension'/, 'the extension ships nap-web\'s pages (§148)');
});

test('the retired checkout is gone: no payment address, no relay, no reservation', () => {
  assert.doesNotMatch(page + zstore, /\/api\/sale|zyn_receiving|isZynZap|Invitation code/);
});

test('it states the prices and the cap where the money is asked for', () => {
  // Terms v4 (§187): 0.05 each, 0.01 for the whitelist, at most 20 per wallet.
  for (const s of ['0.05 ZEC.zy', '0.01 ZEC.zy', 'per wallet', 'membership in ZCaveDAO']) assert.ok(page.includes(s), s);
});

test('it says nobody picks which item comes next', () => {
  // The order is fixed once, from a reveal announced ahead (§187); the page
  // must not suggest that anyone chooses it.
  assert.match(page, /Nobody, the team included, picks which item comes next/);
  assert.match(page, /fixed once/);
});

test('the price shown is the chain\'s price for this wallet, which Nap pays', () => {
  // The page reads this wallet's open-mint terms through Nap, and Nap funds
  // exactly that price, so what the holder sees is what they approve.
  assert.match(zstore, /access\?\.open\?\.price \?\? sale\?\.open_price/);
  if (napOps) assert.match(napOps, /const price = BigInt\(a\.open\?\.price \?\? s\.sale\.open_price\)/);
});

test('the built site serves the checkout at /mint', async () => {
  const built = await readFile(new URL('../dist/mint/index.html', import.meta.url), 'utf8');
  assert.equal(built, page, 'dist/mint/index.html is the page');
});

test('it names where the money goes', () => {
  assert.match(page, /90% funds the pool/i);
  assert.match(page, /10% goes to the ZCaveDAO treasury/i);
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
  const domain = Buffer.from('cave.sale.terms.v4');
  const len = (n) => { const b = Buffer.alloc(4); b.writeUInt32BE(n); return b; };
  const digest = createHash('sha256')
    .update(len(domain.length)).update(domain)
    .update(len(payload.length)).update(payload)
    .digest('hex');
  assert.equal(digest, 'ec3b0c2ffc7ae98423479d71d788ed47f975549e66626dc6f9df1b221d5004e8');
  assert.ok(page.includes(digest), 'and states the digest it reproduces');
});

test('the raw bytes served beside the page are the same bytes', async () => {
  const page = await readFile(new URL('../terms.html', import.meta.url), 'utf8');
  const inPage = (page.match(/id="bytes">([\s\S]*?)<\/div>/) || [])[1].replace(/\s+/g, '');
  const raw = (await readFile(new URL('../dist/cave-sale-terms-v4.hex', import.meta.url), 'utf8')).trim();
  assert.equal(raw, inPage, 'the downloadable file and the page must not drift');
});

test('the terms on the page hash to the published v4 digest (§187), and v3 stays checkable', async () => {
  const { createHash } = await import('node:crypto');
  const len = (n) => { const b = Buffer.alloc(4); b.writeUInt32BE(n); return b; };
  const hash = (domain, raw) => { const p = Buffer.from(raw, 'hex'); const d = Buffer.from(domain); return createHash('sha256').update(len(d.length)).update(d).update(len(p.length)).update(p).digest('hex'); };
  const v4 = (await readFile(new URL('../dist/cave-sale-terms-v4.hex', import.meta.url), 'utf8')).trim();
  assert.equal(hash('cave.sale.terms.v4', v4), 'ec3b0c2ffc7ae98423479d71d788ed47f975549e66626dc6f9df1b221d5004e8');
  // v4 names the v3 it replaces, whose bytes stay checkable beside it.
  assert.ok(v4.includes('cd4cdc6ab086ed51aa318093d34972f9567bcd8d3b855d19bfdbfc857b44e315'));
  const v3 = (await readFile(new URL('../dist/cave-sale-terms-v3.hex', import.meta.url), 'utf8')).trim();
  assert.equal(hash('cave.sale.terms.v3', v3), 'cd4cdc6ab086ed51aa318093d34972f9567bcd8d3b855d19bfdbfc857b44e315');
  // v3 names the v2 it replaces, whose bytes stay checkable beside it.
  assert.ok(v3.includes('9267d58e849caf1bff4a622451185c609e476ac0d026e3c95761bee73cae3d3c'));
  const v2 = (await readFile(new URL('../dist/cave-sale-terms-v2.hex', import.meta.url), 'utf8')).trim();
  assert.equal(hash('cave.sale.terms.v2', v2), '9267d58e849caf1bff4a622451185c609e476ac0d026e3c95761bee73cae3d3c');
});

test('the replaced terms stay checkable', async () => {
  // v2 names the v1 digest it replaces (§175); its bytes must still hash to it.
  const { createHash } = await import('node:crypto');
  const raw = (await readFile(new URL('../dist/cave-sale-terms-v1.hex', import.meta.url), 'utf8')).trim();
  const payload = Buffer.from(raw, 'hex');
  const domain = Buffer.from('cave.sale.terms.v1');
  const len = (n) => { const b = Buffer.alloc(4); b.writeUInt32BE(n); return b; };
  const digest = createHash('sha256').update(len(domain.length)).update(domain).update(len(payload.length)).update(payload).digest('hex');
  assert.equal(digest, 'f2c3242060baf60e0c193b41dc726b6677d85c158a62ba200c948d48a177a59e');
  const page = await readFile(new URL('../terms.html', import.meta.url), 'utf8');
  // v4 replaced v3 (named whole on the page); v2 and v1 are named by their short forms.
  assert.ok(page.includes(`${digest.slice(0, 8)}…${digest.slice(-4)}`), 'the page still names v1');
  assert.ok(page.includes('cd4cdc6ab086ed51aa318093d34972f9567bcd8d3b855d19bfdbfc857b44e315'), 'and v3, which v4 replaces');
  assert.ok(page.includes('9267d58e…3d3c'), 'and v2, by its short form');
});

test('the terms page says what it does not prove', async () => {
  // It is our own web server: it cannot date its own contents. Saying so is
  // the difference between publishing and claiming.
  const page = await readFile(new URL('../terms.html', import.meta.url), 'utf8');
  assert.match(page, /does not yet prove/i);
  assert.match(page, /has not happened yet/i);
});

// Waitlist stage (6 Oct 2026): the landing page sends visitors to the list
// and names none of the sale's routes. The terms stay published at /terms.
test('the landing page points to the list, not to the sale', async () => {
  const landing = await readFile(new URL('../index.html', import.meta.url), 'utf8');
  assert.match(landing, /href="\/whitelist"/);
  assert.doesNotMatch(landing, /href="\/(mint|market|deposit|terms|items|exit)"/);
});
