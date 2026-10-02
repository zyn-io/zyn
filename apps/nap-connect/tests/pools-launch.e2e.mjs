// Pools and the launchpad, the edges the trade tests do not reach: bad and
// oversized amounts on every form, selling more than is held, the create
// form's limits, a graduated coin, the portfolio, a reload mid-session, and
// what a phone shows. Testnet; nothing is signed — every bad input must be
// refused before Nap is asked, and nothing may read NaN or undefined.
//
//   PHRASE_FILE=<a funded testnet wallet>  node tests/pools-launch.e2e.mjs
//   (JADE=https://cave-jade.pages.dev NAP=https://nap.zyn-network.com)
import { chromium } from 'playwright-core';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { unlockNap } from './unlock.mjs';

const JADE = process.env.JADE || 'https://cave-jade.pages.dev';
const NAP = process.env.NAP || 'https://nap.zyn-network.com';
const CHROMIUM = process.env.CHROMIUM || `${homedir()}/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing`;
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const S = process.env.SHOTS || '/tmp/pools-launch';
mkdirSync(S, { recursive: true });
const ZTEST = '690c71d89e13ce552cd1170780f0d5fb8ab495c02248ab8e12aa8c9ba5a8d476';
const TGRAD = '17057e59a4c736aaa498c052a3bf4c38c73a836559abf52ebe509385295a1614';
let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) { failed++; process.exitCode = 1; } };
const errors = [];
const watch = (p, n) => {
  p.on('pageerror', (e) => errors.push(`${n}: ${e.message}`));
  p.on('console', (m) => { if (m.type() === 'error' && !/frame-ancestors|Failed to load resource/.test(m.text())) errors.push(`${n} console: ${m.text().slice(0, 160)}`); });
};
const clean = (t) => !/\bNaN\b|undefined|\[object Object\]|Infinity/.test(t);
const b = await chromium.launch({ executablePath: CHROMIUM, headless: true });

// ---------------------------------------------------------------------------
// Jade.

const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
const page = await ctx.newPage();
watch(page, 'jade');
let popup = null;
async function connectJade() {
  await page.click('#wallet-button');
  await page.waitForSelector('.jade-wallet-option');
  [popup] = await Promise.all([ctx.waitForEvent('page'), page.click('.jade-wallet-option:has-text("Nap in the browser")')]);
  watch(popup, 'nap');
  await unlockNap(popup, WORDS);
  await popup.waitForSelector('#yes').then(() => popup.click('#yes')).catch(() => {});
  await page.waitForFunction(() => !/Connect wallet/.test(document.querySelector('#wallet-button').textContent), null, { timeout: 60000 });
}
/// Whether the trade form lets `value` through: what it quotes, what it says,
/// and whether Submit would ask Nap. Submitting is left to the page: if it asks
/// Nap for something refusable, the popup shows a card and we reject it.
async function tryTrade(value) {
  await page.fill('#trade-input', value);
  await page.waitForTimeout(350);
  const out = await page.textContent('#trade-output');
  const status = await page.textContent('#trade-status');
  const disabled = await page.$eval('.trade-submit', (e) => e.disabled);
  let askedNap = false;
  if (!disabled) {
    const before = popup ? await popup.textContent('main').catch(() => '') : '';
    await page.click('.trade-submit');
    await page.waitForTimeout(2500);
    if (popup && !popup.isClosed() && (await popup.$('#yes'))) { askedNap = true; await popup.click('#no').catch(() => {}); }
    else if (popup && (await popup.textContent('main').catch(() => '')) !== before) askedNap = true;
  }
  const after = await page.textContent('#trade-status');
  return { out, status, after, disabled, askedNap };
}

await page.goto(`${JADE}/coin/${ZTEST}?network=testnet&nap=${NAP}`, { waitUntil: 'domcontentloaded' });
await page.waitForSelector('#coin-title:has-text("Zyn Test Stone")', { timeout: 30000 });
await page.waitForFunction(() => /ZEC/.test(document.querySelector('#coin-mcap').textContent), null, { timeout: 30000 });
await connectJade();
await page.waitForFunction(() => /You hold/.test(document.querySelector('#trade-balance').textContent), null, { timeout: 60000 });
const report = {};
for (const [side, values] of [['buy', ['abc', '0', '-1', '1e3', '0.0000001', '999']], ['sell', ['abc', '0', '-5', '1.5', '99999999999']]]) {
  await page.click(`[data-trade="${side}"]`);
  await page.waitForTimeout(300);
  for (const v of values) {
    const r = await tryTrade(v);
    report[`${side} ${v}`] = r;
    ok(clean(r.out + r.status + r.after), `jade ${side} "${v}": no NaN/undefined (${r.out.trim()} | ${r.after.trim().slice(0, 70)})`);
    ok(!r.askedNap, `jade ${side} "${v}": refused before Nap is asked${r.askedNap ? ' — Nap was asked' : ''}`);
  }
}
await page.screenshot({ path: `${S}/jade-bad-inputs.png`, fullPage: true });

// TGRAD graduated into a pool paired with TESTZ.zy, not ZEC.zy. Jade lists
// only coins priced in ZEC.zy, so its page must say that plainly — and show
// nothing left over from the coin before it.
await page.goto(`${JADE}/coin/${TGRAD}?network=testnet&nap=${NAP}`, { waitUntil: 'domcontentloaded' });
await page.waitForSelector('#coin-title', { timeout: 30000 });
await page.waitForTimeout(4000);
const nf = `${await page.textContent('#coin-title')} ${await page.textContent('#coin-description')}`;
ok(/not found/i.test(nf) && /priced in ZEC\.zy/.test(nf), `jade, a coin it does not list: says why (${nf.trim().slice(0, 90)})`);
ok(/^[—\s]*$/.test(await page.textContent('#coin-mcap')) && /^[—\s]*$/.test(await page.textContent('#trade-output')), 'jade, a coin it does not list: nothing of the last coin stays on screen');
await page.screenshot({ path: `${S}/jade-not-listed.png`, fullPage: true });

// The create form's limits (nothing is submitted: every case must be refused on the page).
await page.goto(`${JADE}/launch?network=testnet&nap=${NAP}`, { waitUntil: 'domcontentloaded' });
await page.waitForSelector('#stone-form');
const txt = `${S}/not-an-image.txt`; writeFileSync(txt, 'hello');
const big = `${S}/big.png`; writeFileSync(big, Buffer.concat([Buffer.from('89504e470d0a1a0a', 'hex'), Buffer.alloc(3 * 1024 * 1024)]));
for (const [label, fill] of [
  ['empty', {}],
  ['no symbol', { name: 'Test' }],
  ['long symbol', { name: 'Test', symbol: 'ABCDEFGHIJKLMNOP' }],
  ['bad symbol', { name: 'Test', symbol: 'a b<' }],
  ['text as image', { name: 'Test', symbol: 'TST', image: txt }],
  ['3 MB image', { name: 'Test', symbol: 'TST', image: big }],
]) {
  await page.fill('#stone-name', fill.name || '');
  await page.fill('#stone-symbol', fill.symbol || '');
  if (fill.image) await page.setInputFiles('#stone-image', fill.image); else await page.setInputFiles('#stone-image', []);
  const before = popup && !popup.isClosed() ? await popup.textContent('main').catch(() => '') : '';
  await page.click('#launch-stone').catch(() => {});
  await page.waitForTimeout(1500);
  let asked = false;
  if (popup && !popup.isClosed() && (await popup.$('#yes'))) { asked = true; await popup.click('#no').catch(() => {}); }
  else if (popup && !popup.isClosed() && (await popup.textContent('main').catch(() => '')) !== before) asked = true;
  const said = await page.textContent('#stone-status');
  ok(!asked, `jade create, ${label}: refused on the page (${said.trim().slice(0, 80) || 'no message'})`);
  ok(clean(said), `jade create, ${label}: no NaN/undefined`);
}
await page.screenshot({ path: `${S}/jade-create.png`, fullPage: true });

// The portfolio, connected.
await page.goto(`${JADE}/portfolio?network=testnet&nap=${NAP}`, { waitUntil: 'domcontentloaded' });
await page.waitForTimeout(6000);
const port = await page.textContent('main');
ok(clean(port), `jade portfolio: no NaN/undefined (${(await page.textContent('#portfolio-status')).trim().slice(0, 70)})`);
await page.screenshot({ path: `${S}/jade-portfolio.png`, fullPage: true });

// A phone: the coin page's trade panel is reachable.
const phone = await b.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
const pp = await phone.newPage();
watch(pp, 'jade phone');
await pp.goto(`${JADE}/coin/${ZTEST}?network=testnet`, { waitUntil: 'domcontentloaded' });
await pp.waitForSelector('#trade-input', { timeout: 30000 });
await pp.waitForTimeout(3000);
const box = await pp.$eval('.trade-submit', (e) => { const r = e.getBoundingClientRect(); return { l: r.left, r: r.right, w: document.documentElement.clientWidth, sw: document.documentElement.scrollWidth }; });
ok(box.l >= 0 && box.r <= box.w + 1 && box.sw <= box.w + 1, `jade on a phone: the trade button fits (${Math.round(box.l)}–${Math.round(box.r)} of ${box.w}, page ${box.sw})`);
await pp.screenshot({ path: `${S}/jade-phone.png`, fullPage: true });
await phone.close();
await ctx.close();

// ---------------------------------------------------------------------------
// Nap Market.

const mctx = await b.newContext({ viewport: { width: 1280, height: 950 } });
const m = await mctx.newPage();
watch(m, 'market');
await m.goto(`${NAP}/market.html`);
await m.evaluate(() => localStorage.setItem('nap.network', 'testnet'));
await m.goto(`${NAP}/market.html`);
await m.click('#open-wallet');
await m.waitForSelector('#words, #kept-password');
if (await m.$('#use-phrase')) await m.click('#use-phrase');
await m.fill('#words', WORDS);
await m.click('#phrase-form button[type=submit]');
await m.waitForFunction(() => /Up to date/.test(document.querySelector('#notice')?.textContent || ''), null, { timeout: 120000 })
  .catch(async () => { throw new Error(`the market never said Up to date; it says: ${(await m.textContent('#notice').catch(() => '?')).slice(0, 160)} | ${(await m.textContent('#view').catch(() => '?')).replace(/\s+/g, ' ').slice(0, 200)}`); });
const tab = async (v) => { await m.click(`#tabs [data-view="${v}"]`); await m.waitForTimeout(600); };
const notice = () => m.textContent('#notice').catch(() => '');

await tab('swap');
for (const v of ['abc', '0', '-1', '1e3', '999999']) {
  await m.fill('#amount', v);
  await m.waitForTimeout(300);
  const q = await m.textContent('#quote');
  const before = await notice();
  await m.click('#swap-form button[type=submit]');
  await m.waitForTimeout(2500);
  const said = await notice();
  ok(clean(q + said), `market swap "${v}": no NaN/undefined (quote ${q})`);
  ok(!/Order placed/.test(said) || said === before, `market swap "${v}": no order placed (${said.trim().slice(0, 80)})`);
}
await m.screenshot({ path: `${S}/market-swap-bad.png`, fullPage: true });

await tab('pools');
ok(clean(await m.textContent('main')), 'market pools: no NaN/undefined');
const selects = await m.$$eval('#create-pool select[name="a"] option', (o) => o.map((x) => x.value).filter(Boolean));
if (selects.length) {
  await m.selectOption('#create-pool select[name="a"]', selects[0]);
  await m.selectOption('#create-pool select[name="b"]', selects[0]);
  await m.fill('#create-pool input[name="amount_a"]', '0.001');
  await m.fill('#create-pool input[name="amount_b"]', '0.001');
  const before = await notice();
  await m.click('#create-pool button[type=submit]');
  await m.waitForTimeout(2500);
  const said = await notice();
  ok(!/Pool open/.test(said) && said !== before, `market: a pool of an asset with itself is refused (${said.trim().slice(0, 80)})`);
}
await m.screenshot({ path: `${S}/market-pools.png`, fullPage: true });

await tab('launchpad');
const lp = await m.textContent('main');
ok(clean(lp), 'market launchpad: no NaN/undefined');
await m.screenshot({ path: `${S}/market-launchpad.png`, fullPage: true });

await tab('owed');
ok(clean(await m.textContent('main')), 'market owed: no NaN/undefined');

// A reload keeps the network and asks to open the wallet again (or opens it
// from this browser's session where there is one).
await m.reload();
await m.waitForTimeout(3000);
ok(/testnet/i.test(await m.textContent('header')), 'market: a reload stays on testnet');
await mctx.close();
await b.close();

ok(errors.length === 0, `no page errors${errors.length ? `:\n  ${errors.join('\n  ')}` : ''}`);
writeFileSync(`${S}/report.json`, JSON.stringify(report, null, 1));
console.log(failed ? `${failed} failed` : 'all passed', `— screenshots in ${S}`);
