// Collection Z's sale end to end on testnet, through the real mint page and a
// real Nap in the browser (DECISIONS §144, §145): tickets bought on
// cavebro.win/mint, the draw's wins claimed there, then the open mint.
// Each phase waits for the sale to reach its stage; the operator's half
// (open, close, draw, open mint) runs beside it on the node's box.
//
//   PHASE=buy|claim|mint  BUYER_PHRASE=<file>  node tests/sale.e2e.mjs
import { chromium } from 'playwright-core';
import { readFileSync } from 'node:fs';

const SITE = process.env.SITE || 'https://cavebro.win';
const RPC = process.env.RPC || 'https://zyn-rpc.cavebro.win/object';
const S = process.env.SHOTS || '/tmp';
const PHASE = process.env.PHASE || 'buy';
const TICKETS = Number(process.env.TICKETS || 3);
const CAVE_Z = '024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e';
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };

async function phase() {
  const r = await (await fetch(RPC, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'zyn_market', params: [] }) })).json();
  return (r.result?.sales || []).find((s) => s.collection === CAVE_Z) || null;
}

const want = { buy: 'open', claim: 'drawn', mint: 'openmint' }[PHASE];
for (let i = 0; ; i++) {
  const s = await phase();
  if (s?.phase === want) break;
  if (i === 0) console.log(`waiting for the sale to be ${want} (now ${s?.phase || 'none'})…`);
  await new Promise((r) => setTimeout(r, 15000));
}

const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
const page = await ctx.newPage();
page.on('pageerror', (e) => console.log('page error:', e.message));
const words = readFileSync(process.env.BUYER_PHRASE, 'utf8').trim();
let nap = null;

async function connect() {
  await page.click('[data-wallet-button]');
  await page.waitForSelector('.zwallet-option');
  const before = new Set(ctx.pages());
  await page.click('.zwallet-option:has-text("Nap in the browser")');
  nap = await ctx.waitForEvent('page', { timeout: 15000 }).catch(() => ctx.pages().find((x) => !before.has(x)));
  await nap.waitForSelector('#unlock, #yes', { timeout: 30000 });
  if (await nap.locator('#unlock').isVisible().catch(() => false)) {
    await nap.fill('#words', words);
    await nap.click('#unlock button[type=submit]');
  }
  await nap.waitForSelector('#yes', { timeout: 30000 });
  await nap.click('#yes');
  await page.waitForFunction(() => /Nap ·/.test(document.querySelector('[data-wallet-button]')?.textContent || ''), null, { timeout: 30000 });
}

async function approve(expect) {
  await nap.waitForSelector('#yes', { timeout: 90000 });
  const text = await nap.textContent('main');
  ok(expect.test(text), `Nap shows ${expect}`);
  await nap.click('#yes');
}

await page.goto(`${SITE}/mint/?network=testnet`);
await connect();

if (PHASE === 'buy') {
  await page.waitForSelector('#s-open:not([hidden])', { timeout: 60000 });
  ok(/ZEC\.zy/.test(await page.textContent('#total')), 'the page quotes tickets in ZEC.zy');
  await page.fill('#count-in', String(TICKETS));
  await page.click('#buy');
  await approve(new RegExp(`Buy ${TICKETS} tickets`));
  await page.waitForFunction(() => /bought/.test(document.querySelector('#out')?.textContent || ''), null, { timeout: 120000 });
  ok(true, `the page says ${await page.textContent('#out')}`);
  await page.waitForSelector('#mine-box:not([hidden]) #mine li', { timeout: 120000 });
  ok(await page.locator('#mine li').count() >= TICKETS, `the page lists the wallet's ${TICKETS} tickets, waiting for the draw`);
  await page.screenshot({ path: `${S}/sale-bought.png`, fullPage: true });
}

if (PHASE === 'claim') {
  await page.waitForSelector('#s-drawn:not([hidden])', { timeout: 60000 });
  await page.waitForSelector('#won-art figure img', { timeout: 120000 });
  const won = await page.locator('#won-art figure').count();
  ok(won >= 1, `the page shows ${won} item(s) this wallet drew, with their art`);
  await page.screenshot({ path: `${S}/sale-drawn.png`, fullPage: true });
  await page.click('#claim');
  await approve(/Claim \d+ item/);
  await page.waitForFunction(() => /Claimed/.test(document.querySelector('#out-claim')?.textContent || ''), null, { timeout: 300000 });
  ok(true, `the page says ${await page.textContent('#out-claim')}`);
}

if (PHASE === 'mint') {
  await page.waitForSelector('#s-openmint:not([hidden])', { timeout: 60000 });
  const left = Number(await page.textContent('#left'));
  ok(left > 0, `the open mint shows ${left} left`);
  await page.fill('#mint-in', '1');
  await page.click('#mint');
  await approve(/Mint 1 from the open mint/);
  await page.waitForFunction(() => /Minted Zyn/.test(document.querySelector('#out-mint')?.textContent || ''), null, { timeout: 300000 });
  ok(true, `the page says ${await page.textContent('#out-mint')}`);
  await page.screenshot({ path: `${S}/sale-minted.png`, fullPage: true });
}

// What the wallet now holds, on its shelf.
// Followed by the store's own link, which keeps the Nap connection (a fresh
// page load would start a new, unconnected session).
if (PHASE !== 'buy') {
  await page.click('.nav a[href="/items"]');
  await page.waitForSelector('#s-shelf:not([hidden]) .item', { timeout: 90000 });
  const shelf = await page.locator('#shelf .item .sym').allTextContents();
  ok(shelf.length >= 1, `the shelf holds ${shelf.join(', ')}`);
  await page.screenshot({ path: `${S}/sale-shelf-${PHASE}.png`, fullPage: true });
}

await b.close();
