// Zyn Lend, as a holder uses it: the app at lend.zyn-network.com, Nap in the
// browser in its own window, testnet 212. Lend SOL.zy, borrow SOL.zy against
// ETH.zy, collect, and find the position under Positions — every step
// approved in Nap, which shows the numbers the chain will use.
//
//   PHRASE_FILE=~/.zyn/test-wallets/bidder.phrase node tests/lend.e2e.mjs
//
// The wallet needs a little SOL.zy and ETH.zy (testnet-wallet.local.mjs).

import { chromium } from 'playwright-core';
import { readFileSync, mkdirSync } from 'node:fs';
import { unlockNap } from './unlock.mjs';

const APP = process.env.APP || 'https://lend.zyn-network.com';
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const S = process.env.SHOTS || '/tmp/lend-e2e';
mkdirSync(S, { recursive: true });
const ETH = '56737e143259c754d165074ad15058139bc0b116f5a22b2f3d50129a2a7a24a6';
const SOL = '9204324e0511b4d1a90d0ff36242f044e894e1b71489d47abd824df273092ad1';

let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) failed++; };

const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
const page = await ctx.newPage();
page.on('pageerror', (e) => console.log('app error:', e.message));
const toast = () => page.textContent('#toast');

// The markets, from the chain.
await page.goto(`${APP}/#/markets`);
await page.waitForFunction(() => /\d/.test(document.querySelector('.core-price')?.textContent || ''), null, { timeout: 30000 });
await page.waitForTimeout(3000);
ok(await page.isHidden('#preview'), 'the app reads the chain, not examples');
const solCore = await page.textContent('.core:nth-child(3) .core-stats');
ok(/Supplied\s*[1-9]/.test(solCore.replace(/\s+/g, ' ')), `SOL.zy pool has lenders: ${solCore.replace(/\s+/g, ' ').trim().slice(0, 80)}`);
await page.screenshot({ path: `${S}/lend-markets.png`, fullPage: true });

// Connect Nap in the browser.
await page.click('#connect');
await page.waitForSelector('#wallet-list button');
const [popup] = await Promise.all([ctx.waitForEvent('page'), page.click('#wallet-list button:has-text("Nap in the browser")')]);
await unlockNap(popup, WORDS);
await popup.waitForSelector('#yes');
await popup.click('#yes');
await page.waitForFunction(() => document.querySelector('#connect').classList.contains('on'), null, { timeout: 60000 });
ok(true, `connected: ${(await page.textContent('#connect')).trim()}`);

// Approve whatever Nap shows next, and say what it showed.
async function approve(label, check) {
  await popup.waitForSelector('#yes', { timeout: 60000 }).catch(async (e) => {
    console.log(`${label}: Nap shows ${(await popup.textContent('body').catch(() => '(closed)'))?.replace(/\s+/g, ' ').slice(0, 300)}`);
    console.log(`${label}: the app says ${await toast()}`);
    throw e;
  });
  const shown = (await popup.textContent('main')).replace(/\s+/g, ' ');
  ok(check(shown), `${label}: Nap shows ${shown.slice(0, 140)}…`);
  await popup.screenshot({ path: `${S}/lend-approve-${label}.png` });
  await popup.click('#yes');
}

// Lend 0.1 SOL.zy.
await page.goto(`${APP}/#/lend/SOL`);
await page.fill('#lend-amount', '0.1');
await page.waitForTimeout(300);
await page.click('#lend-go');
await approve('lend', (t) => t.includes('Lend SOL.zy') && t.includes('0.1 SOL.zy') && /lSOL\.zy shares/.test(t));
await page.waitForFunction(() => /Lent|refused|declined|error/i.test(document.querySelector('#toast').textContent), null, { timeout: 90000 }).catch(() => {});
ok(/Lent/.test(await toast()), `lend: ${await toast()}`);

// Borrow SOL.zy against ETH.zy, well inside the line.
await page.goto(`${APP}/#/borrow/ETH/SOL`);
await page.fill('#coll-amount', '0.0015');
await page.fill('#borrow-amount', '0.02');
await page.waitForTimeout(400);
ok(!(await page.isDisabled('#borrow-go')), `the form allows it: health ${await page.textContent('#health-num')}, ${await page.textContent('#borrow-note')}`);
await page.click('#borrow-go');
await approve('borrow', (t) => t.includes('Borrow SOL.zy against ETH.zy') && /Health/.test(t) && /Liquidated if/.test(t) && /auctioned/.test(t));
await page.waitForFunction(() => /Position opened|refused|declined|error/i.test(document.querySelector('#toast').textContent), null, { timeout: 90000 }).catch(() => {});
ok(/Position opened/.test(await toast()), `borrow: ${await toast()}`);

// After the seal: collect, and the position is there — found by Nap alone.
await page.goto(`${APP}/#/positions`);
let found = false;
for (let i = 0; i < 12 && !found; i++) {
  await page.waitForTimeout(10000);
  await page.click('#pos-collect');
  await page.waitForTimeout(3000);
  await page.goto(`${APP}/#/markets`);
  await page.goto(`${APP}/#/positions`);
  await page.waitForTimeout(4000);
  found = (await page.$$('.pos[data-id]')).length > 0;
}
const card = found ? (await page.textContent('.pos[data-id]')).replace(/\s+/g, ' ').trim() : '';
ok(found && /SOL\.zy against .*ETH\.zy/.test(card), `the position, from this wallet only: ${card.slice(0, 120)}`);
await page.screenshot({ path: `${S}/lend-positions.png`, fullPage: true });

// Close it: repay everything and take the collateral back, in one approval.
// This also leaves the wallet as it found it, so the test can run again.
if (found) {
  const closing = await page.getAttribute('.pos[data-id]', 'data-id');
  await page.click(`.pos[data-id="${closing}"] button[data-act="repay"]`);
  await approve('close', (t) => /Repay SOL\.zy and close/.test(t) && /You get back/.test(t) && /ETH\.zy/.test(t));
  await page.waitForFunction(() => /Repaid|refused|declined|error/i.test(document.querySelector('#toast').textContent), null, { timeout: 90000 }).catch(() => {});
  ok(/Repaid and closed/.test(await toast()), `close: ${await toast()}`);
  let gone = false;
  for (let i = 0; i < 12 && !gone; i++) {
    await page.waitForTimeout(10000);
    await page.click('#pos-collect');
    await page.waitForTimeout(3000);
    await page.goto(`${APP}/#/markets`);
    await page.goto(`${APP}/#/positions`);
    await page.waitForTimeout(4000);
    gone = !(await page.$(`.pos[data-id="${closing}"]`));
  }
  ok(gone, 'the closed position is gone and its collateral collected');
}

await b.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exit(failed ? 1 : 0);
