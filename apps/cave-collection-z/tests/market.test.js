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
const deposit = await read('deposit.html');
const zstore = await read('zstore.js');
const napOps = await readFile(new URL('../../nap-web/src/connect-ops.js', import.meta.url), 'utf8').catch(() => null);
const background = await readFile(new URL('../../extension/src/background.js', import.meta.url), 'utf8').catch(() => null);

// The wallets live beside the store in the monorepo. Checked out on its own,
// these cross-app assertions skip with a reason rather than passing quietly.
const WALLET = napOps && background
  ? false
  : 'apps/nap-web or apps/extension is not checked out — run in the monorepo to check the store against both Naps';

const build = await read('scripts/build.mjs');
const walletCalls = [...new Set([...zstore.matchAll(/ask\('(zyn_[a-zA-Z]+)'/g)].map((m) => m[1]))];

test('every method the store asks Nap for exists in both Naps', { skip: WALLET }, () => {
  // A typo'd method is a rejected promise the page reports as "Nap could
  // not answer", which reads as the holder's problem rather than ours.
  assert.ok(walletCalls.length >= 6, `the store asks for ${walletCalls.join(', ')}`);
  for (const m of walletCalls) {
    assert.match(napOps, new RegExp(`\\n  ${m}: \\{`), `Nap in the browser serves ${m}`);
    assert.match(background, new RegExp(`\\b${m}\\s*:`), `the extension serves ${m}`);
  }
});

test('the whole journey is reachable', () => {
  // Each capability, and where it lives now: the store for holding and
  // trading, Nap itself for bringing ZEC in and taking it out.
  for (const m of ['zyn_nftItems', 'zyn_nftList', 'zyn_nftCancel', 'zyn_nftBuy', 'zyn_owed', 'zyn_collect']) {
    assert.ok(walletCalls.includes(m), `the store carries ${m}`);
  }
  assert.match(zstore, /method: 'zyn_offers'/, 'the market reads the public book');
  for (const page of [deposit, exit]) assert.match(page, /href="https:\/\/nap\.cavebro\.win"/, 'deposit and exit hand off to Nap');
  for (const page of [items, item, market]) assert.match(page, /<script type="module" src="\/zstore\.js/);
});

test('nothing speaks the retired wallet API', () => {
  for (const [name, page] of Object.entries({ items, item, market, exit, deposit, zstore })) {
    assert.doesNotMatch(page, /isZynZap|zyn_assets|zyn_placeOffer|zyn_takeOffer|zyn_cancelOffer|zyn_withdraw|zyn_bind/, `${name} is on the object ledger`);
  }
});

test('the store reads only public things itself, and nothing with an account in it', () => {
  // The public book and the collection index are the same for everyone, so
  // fetching them says nothing about the reader. Anything about the holder is
  // asked of their own Nap.
  const fetches = [...zstore.matchAll(/fetch\(([^,)]+)/g)].map((m) => m[1].trim());
  // The index, the chain's book, this site's own next page (in-place
  // navigation), the burn token's config, and the Solana relay — which carries
  // a Solana address the holder is about to make public by burning (§145).
  assert.deepEqual([...new Set(fetches)].sort(), ["'/collection-z.json'", 'RPC', 'url.pathname', "NETWORK === 'testnet' ? '/burn-testnet.json' : '/burn.json'", '`/api/solana?cluster=${cluster === \'devnet\' ? \'devnet\' : \'mainnet\'}`'].sort());
  assert.match(zstore, /url\.origin !== location\.origin \|\| !ROUTES\[path\]/, 'only this site\'s own store pages are fetched in place');
  assert.match(zstore, /method: 'zyn_offers', params: \[\]/);
  assert.match(zstore, /method: 'zyn_market', params: \[\]/, 'the sale is read from the public market');
  assert.doesNotMatch(zstore, /127\.0\.0\.1:\d|:8096|:8099/);
  assert.match(zstore, /LOCAL && initial\.get\('nap'\)/, 'the Nap override is for localhost only');
});

test('an item named in the address bar is validated before use', () => {
  assert.match(zstore, /Number\.isSafeInteger\(serial\) \|\| serial < 0/);
  assert.match(zstore, /isHex\(params\.get\('collection'\)\)/);
});

test('prices are refused before they reach Nap', () => {
  assert.match(zstore, /\/\^\\d\+\(\\\.\\d\{1,8\}\)\?\$\//);
  assert.match(zstore, /Number\(price\) <= 0/);
});

test('the market never offers someone their own listing', () => {
  assert.match(zstore, /yours\.has\(o\.asset\)/);
  assert.match(zstore, />Yours</);
});

test('a write is followed by a re-read, never an optimistic update', () => {
  const writes = (zstore.match(/await ask\('zyn_nft(List|Cancel|Buy)'/g) || []).length;
  const rereads = (zstore.match(/setTimeout\(read, 12000\)/g) || []).length;
  assert.ok(writes >= 4 && rereads >= writes, `${writes} writes, ${rereads} re-reads`);
});

test('every page is staged into the build, with the module and the index', () => {
  for (const page of ['items', 'item', 'market', 'exit']) assert.match(build, new RegExp(`'${page}'`));
  assert.match(build, /'zstore\.js'/);
  assert.match(build, /nap-connect\/nap-connect\.js/);
  assert.match(build, /collection-z\.json/);
});

test('the pages link to each other, so no capability is an orphan', () => {
  assert.match(items, /href="\/market"/);
  assert.match(market, /href="\/items"/);
  assert.match(zstore, /\/item\?collection=\$\{it\.collection\}&serial=\$\{it\.serial\}/, 'the shelf links each card to its own page');
  assert.match(zstore, /\/item\?collection=\$\{o\.collection\}&serial=\$\{o\.serial\}/, 'each row links to the object it sells');
  assert.match(item, /href="\/items"/);
});

// ---------------------------------------------------------------------------
// The headers a site that talks to a wallet should have.
// ---------------------------------------------------------------------------

const { readdir } = await import('node:fs/promises');

test('the built site ships a policy, and it names every inline script', async () => {
  // The node status page shipped a full policy and this one -- the one that
  // takes money -- shipped none, so the checkout could be framed and a click
  // on an invisible overlay was a click on "pay".
  const headers = await readFile(new URL('../dist/_headers', import.meta.url), 'utf8');
  for (const h of [
    'Content-Security-Policy',
    'X-Frame-Options: DENY',
    'X-Content-Type-Options: nosniff',
    'Referrer-Policy: no-referrer',
  ]) {
    assert.ok(headers.includes(h), `${h} is served`);
  }
  assert.ok(headers.includes("frame-ancestors 'none'"), 'the checkout cannot be framed');
  assert.ok(headers.includes("object-src 'none'"));
  assert.ok(headers.includes("base-uri 'none'"));

  // Every inline script on the site must be named by a hash, or the page is
  // broken in production and nowhere else.
  const { createHash } = await import('node:crypto');
  const pages = (await readdir(new URL('../', import.meta.url))).filter((f) => f.endsWith('.html'));
  let checked = 0;
  for (const page of pages) {
    const html = await readFile(new URL('../' + page, import.meta.url), 'utf8');
    for (const m of html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)) {
      if (!m[1].trim()) continue;
      const hash = createHash('sha256').update(m[1], 'utf8').digest('base64');
      assert.ok(
        headers.includes(`'sha256-${hash}'`),
        `${page}'s inline script is named in the policy`,
      );
      checked += 1;
    }
  }
  assert.ok(checked >= 2, `every page's inline script was checked (saw ${checked})`);
});

test('the pages load no script from anywhere else', () => {
  // The policy is only as good as this: with no external scripts, anything
  // the browser is asked to run that is not hashed above was injected.
  for (const [name, page] of Object.entries({ items, item, market, exit, deposit })) {
    assert.doesNotMatch(page, /<script[^>]+src=["']https?:/, `${name} loads no remote script`);
  }
});
