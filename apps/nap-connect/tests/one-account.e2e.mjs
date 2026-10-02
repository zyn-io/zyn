// One Zyn account everywhere: the same Nap, unlocked once, connects to every
// venue on Zyn — Lend, Jade, and Cave's items and market (cavebro.win) — and
// each one is handed the same account, the one the phrase derives. Testnet.
//
//   PHRASE_FILE=<a testnet wallet's phrase>  node tests/one-account.e2e.mjs
//   (LEND, JADE, CAVE, NAP to point elsewhere)
import { chromium } from 'playwright-core';
import { readFileSync } from 'node:fs';
import { unlockNap } from './unlock.mjs';

const NAP = process.env.NAP || 'https://nap.zyn-network.com';
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const S = process.env.SHOTS || '/tmp';
const { deriveIdentity } = await import('../../nap-web/src/core.js');
const expected = deriveIdentity(WORDS).accountHex;

const VENUES = [
  { name: 'Lend', url: `${process.env.LEND || 'https://lend.zyn-network.com'}/#/markets`, open: '#connect', option: '#wallet-list button:has-text("Nap in the browser")' },
  { name: 'Jade', url: `${process.env.JADE || 'https://jade.zyn-network.com'}/?network=testnet`, open: '#wallet-button', option: '.jade-wallet-option:has-text("Nap in the browser")' },
  { name: 'Cave items', url: `${process.env.CAVE || 'https://cavebro.win'}/items?network=testnet`, open: '[data-wallet-button]', option: '.zwallet-option:has-text("Nap in the browser")' },
  { name: 'Cave market', url: `${process.env.CAVE || 'https://cavebro.win'}/market/?network=testnet`, open: '[data-wallet-button]', option: '.zwallet-option:has-text("Nap in the browser")' },
];

let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) failed++; };
const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
// What each site is told: the account in Nap's answer, as it arrives.
await ctx.addInitScript(() => {
  addEventListener('message', (e) => {
    try {
      const m = JSON.stringify(e.data).match(/"account":"([0-9a-f]{64})"/);
      if (m) window.__zynAccount = m[1];
    } catch { /* not ours */ }
  });
});

/// Nap's window for this click: a new one, or the one already open.
async function napWindow(page, option) {
  const before = new Set(ctx.pages());
  await page.click(option);
  for (let i = 0; i < 60; i++) {
    const nap = ctx.pages().filter((p) => p.url().startsWith(NAP));
    const fresh = nap.find((p) => !before.has(p));
    const win = fresh || nap.at(-1);
    if (win && await win.$('#setup-create, #pw, #yes, #words')) return win;
    await page.waitForTimeout(500);
  }
  throw new Error("Nap's window did not open");
}

const seen = [];
for (const v of VENUES) {
  const page = await ctx.newPage();
  page.on('pageerror', (e) => console.log(`${v.name} error:`, e.message));
  try {
    await page.goto(v.url, { waitUntil: 'domcontentloaded' });
    await page.click(v.open);
    await page.waitForSelector(v.option, { timeout: 30000 });
    const win = await napWindow(page, v.option);
    const how = await unlockNap(win, WORDS);
    // A site approved before (the same origin) connects without a second card.
    const told = () => page.waitForFunction(() => !!window.__zynAccount, null, { timeout: 60000 }).then(() => 'told');
    const first = await Promise.race([win.waitForSelector('#yes', { timeout: 60000 }).then(() => 'card'), told()]);
    if (first === 'card') {
      const card = (await win.textContent('main')).replace(/\s+/g, ' ');
      ok(card.includes(new URL(v.url).host), `${v.name}: Nap's card names ${new URL(v.url).host}${how === 'card' ? ' (Nap already unlocked)' : ''}`);
      await win.click('#yes');
      await told();
    } else ok(true, `${v.name}: ${new URL(v.url).host} was approved before; no second card`);
    const account = await page.evaluate(() => window.__zynAccount);
    seen.push(account);
    ok(account === expected, `${v.name}: connected as ${account.slice(0, 8)}…${account.slice(-6)}`);
    await page.screenshot({ path: `${S}/one-account-${v.name.replace(/\s+/g, '-').toLowerCase()}.png` });
    if (v.name === 'Lend') {
      // Lend's portfolio shows the same account, and the Zcash address Nap shares
      // is the one the phrase gives in any Zcash wallet (account 0).
      await page.goto(v.url.replace('#/markets', '#/portfolio'));
      await page.waitForFunction(() => /^[0-9a-f]{64}$/.test(document.querySelector('#pf-id-zyn')?.textContent || ''), null, { timeout: 30000 });
      const shown = await page.evaluate(() => ({ zyn: document.querySelector('#pf-id-zyn').textContent, zec: document.querySelector('#pf-id-zec').textContent }));
      const base = `${new URL(v.url).origin}/vendor/zec/`;
      const z = await import(`data:text/javascript,${encodeURIComponent(await (await fetch(`${base}nap_zec.js`)).text())}`);
      await z.default({ module_or_path: Buffer.from(await (await fetch(`${base}nap_zec_bg.wasm`)).arrayBuffer()) });
      const own = new z.ZecWallet(z.spendingKeyFromPhrase(WORDS, '', 'testnet', 0), 'testnet').address();
      ok(shown.zyn === expected && shown.zec === own, `Lend portfolio: Zyn ${shown.zyn.slice(0, 8)}…, Zcash ${shown.zec.slice(0, 16)}… — the phrase's own (account 0)`);
    }
  } catch (e) {
    ok(false, `${v.name}: ${e.message.split('\n')[0]}`);
  }
}
ok(seen.length === VENUES.length && new Set(seen).size === 1, `one account on all ${VENUES.length} venues: ${expected.slice(0, 8)}…${expected.slice(-6)}`);
await b.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exitCode = failed ? 1 : 0;
