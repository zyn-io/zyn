// Trading Collection Z items end to end on testnet, through two real Nap
// wallets in two browsers: a seller lists two items on cavebro.win; a buyer
// takes one on cavebro.win's market and the other on the Cave store; the
// items move, and the seller collects what the market owes them.
//
//   SITE=https://cavebro.win STORE=https://cave-market.pages.dev NAP=https://nap.cavebro.win
//   SELLER_PHRASE=<file> BUYER_PHRASE=<file>  node tests/store-trade.e2e.mjs
import { chromium } from 'playwright-core';
import { readFileSync } from 'node:fs';

const SITE = process.env.SITE || 'https://cavebro.win';
const STORE = process.env.STORE || 'https://cave-market.pages.dev';
const NAP = process.env.NAP || 'https://nap.cavebro.win';
const S = process.env.SHOTS || '/tmp';
const PRICE = '0.002';
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };
const q = '?network=testnet';

const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });

async function person(phraseFile) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => console.log('page error:', e.message));
  return { ctx, page, words: readFileSync(phraseFile, 'utf8').trim(), popup: null };
}

/// Connect through the page's wallet button and Nap in the browser.
async function connectNap(p, buttonSel, optionSel) {
  await p.page.click(buttonSel);
  await p.page.waitForSelector(optionSel);
  const [popup] = await Promise.all([p.ctx.waitForEvent('page'), p.page.click(`${optionSel}:has-text("Nap in the browser")`)]);
  p.popup = popup;
  await popup.waitForSelector('#unlock');
  await popup.fill('#words', p.words);
  await popup.click('#unlock button[type=submit]');
  await popup.waitForSelector('#yes');
  await popup.click('#yes');
}

async function approve(p, expect) {
  await p.popup.waitForSelector('#yes', { timeout: 60000 });
  const text = await p.popup.textContent('main');
  ok(expect.test(text), `Nap shows ${expect}`);
  await p.popup.click('#yes');
}

const seller = await person(process.env.SELLER_PHRASE);
const buyer = await person(process.env.BUYER_PHRASE);

// --- The seller's shelf on cavebro.win.
await seller.page.goto(`${SITE}/items/${q}`);
await connectNap(seller, '#connect', '.zwallet-option');
await seller.page.waitForSelector('#s-shelf:not([hidden]) .item', { timeout: 90000 });
const cards = await seller.page.locator('#shelf .item').count();
ok(cards >= 2, `the seller's shelf shows ${cards} items, with their art`);
ok(await seller.page.locator('#shelf .item img[src^="/blobs/"]').count() === cards, 'every card shows the item\'s own image');
const hrefs = await seller.page.locator('#shelf .item').evaluateAll((as) => as.map((a) => a.getAttribute('href')));

// --- List two items, one at a time, each from its own page.
const listed = [];
for (const href of hrefs.slice(0, 2)) {
  await seller.page.goto(`${SITE}${href}`);
  await seller.page.waitForSelector('#sell:not([hidden])', { timeout: 60000 });
  await seller.page.fill('#price', PRICE);
  await seller.page.click('#list');
  await approve(seller, new RegExp(`List Zyn (Mole|Owl|Moth) #\\d+`));
  await seller.page.waitForFunction(() => /Listed/.test(document.querySelector('#out').textContent), null, { timeout: 60000 });
  listed.push({ href, name: (await seller.page.textContent('#name')).trim() });
  ok(true, `listed ${listed.at(-1).name} at ${PRICE}`);
}
await seller.page.waitForTimeout(15000); // the listings rest once the epoch seals

// --- The buyer, on cavebro.win's market: public book, then buy the first.
await buyer.page.goto(`${SITE}/market/${q}`);
await buyer.page.waitForSelector('#s-book:not([hidden])', { timeout: 60000 });
const rows = await buyer.page.locator('#rows tr').count();
ok(rows >= 2, `the public book shows ${rows} listings without a wallet`);
await buyer.page.locator('#rows button', { hasText: 'Connect to buy' }).first().click();
await buyer.page.waitForSelector('.zwallet-option');
const [bp] = await Promise.all([buyer.ctx.waitForEvent('page'), buyer.page.click('.zwallet-option:has-text("Nap in the browser")')]);
buyer.popup = bp;
await bp.waitForSelector('#unlock'); await bp.fill('#words', buyer.words); await bp.click('#unlock button[type=submit]');
await bp.waitForSelector('#yes'); await bp.click('#yes');
await buyer.page.waitForSelector('#rows button:has-text("Buy")', { timeout: 60000 });
const firstName = listed[0].name;
const row = buyer.page.locator('#rows tr', { hasText: firstName });
await row.locator('button', { hasText: 'Buy' }).click();
await approve(buyer, new RegExp(`Buy ${firstName}`));
await buyer.page.waitForFunction(() => /Bought/.test(document.querySelector('#out').textContent), null, { timeout: 60000 });
ok(true, `bought ${firstName} on cavebro.win`);

// --- The buyer, on the Cave store: the second listing.
const store = await buyer.ctx.newPage();
await store.goto(`${STORE}/${q}#market`);
await store.waitForSelector('.trade-card', { timeout: 60000 });
ok(await store.locator('.trade-card', { hasText: listed[1].name }).count() === 1, `the Cave store lists ${listed[1].name}`);
await store.click('.header-wallet');
await store.waitForSelector('.wallet-option, [data-wallet-choice]', { timeout: 15000 }).catch(() => {});
const optionSel = await store.locator('.wallet-option').count() ? '.wallet-option' : '[data-wallet-choice]';
const [sp] = await Promise.all([buyer.ctx.waitForEvent('page'), store.click(`${optionSel}:has-text("Nap in the browser")`)]);
await sp.waitForSelector('#unlock'); await sp.fill('#words', buyer.words); await sp.click('#unlock button[type=submit]');
await sp.waitForSelector('#yes'); await sp.click('#yes');
await store.waitForSelector('.trade-card button:has-text("Buy with Nap")', { timeout: 60000 });
await store.locator('.trade-card', { hasText: listed[1].name }).locator('button').click();
await sp.waitForSelector('#yes', { timeout: 60000 });
ok(new RegExp(`Buy ${listed[1].name}`).test(await sp.textContent('main')), `the Cave store's buy is shown in Nap as ${listed[1].name}`);
await sp.click('#yes');
await store.waitForFunction(() => /Bought/.test(document.querySelector('#trade-note-market')?.textContent || ''), null, { timeout: 60000 });
ok(true, `bought ${listed[1].name} on the Cave store`);

// --- Both items moved; the seller is paid.
await buyer.page.waitForTimeout(15000);
await buyer.page.goto(`${SITE}/items/${q}`);
await buyer.page.click('#connect');
await buyer.page.waitForSelector('.zwallet-option');
const [bp2] = await Promise.all([buyer.ctx.waitForEvent('page'), buyer.page.click('.zwallet-option:has-text("Nap in the browser")')]);
await bp2.waitForSelector('#unlock'); await bp2.fill('#words', buyer.words); await bp2.click('#unlock button[type=submit]');
await bp2.waitForSelector('#yes'); await bp2.click('#yes');
await buyer.page.waitForSelector('#s-shelf:not([hidden]), #s-empty:not([hidden])', { timeout: 90000 });
const shelf = await buyer.page.textContent('#shelf');
ok(shelf.includes(firstName) && shelf.includes(listed[1].name), `the buyer's shelf now holds ${firstName} and ${listed[1].name}`);

await seller.page.goto(`${SITE}/items/${q}`);
await connectNap(seller, '#connect', '.zwallet-option');
await seller.page.waitForSelector('#owed:not([hidden])', { timeout: 90000 }).catch(() => {});
ok(!(await seller.page.locator('#owed').isHidden()), `the seller is owed: ${(await seller.page.textContent('#owed span')).trim()}`);
await seller.page.click('#collect');
await seller.page.waitForFunction(() => /Collected/.test(document.querySelector('#out').textContent), null, { timeout: 90000 }).catch(() => {});
ok(/Collected/.test(await seller.page.textContent('#out')), 'the seller collected the proceeds without a prompt');
await seller.page.screenshot({ path: `${S}/store-seller.png`, fullPage: true });
await b.close();
