// Cave Jade end to end on testnet: the yard reads a curve token from the
// public chain with no wallet; a buy goes through Nap in the browser.
//
//   JADE=http://localhost:4180 (built Jade, SPA fallback)   NAP=http://localhost:4179 (nap-web preview)
//   TOKEN=<64-hex curve token on testnet>   PHRASE_FILE=<a funded testnet wallet's phrase>
//   node tests/jade.e2e.mjs
import { chromium } from 'playwright-core';
import { readFileSync } from 'node:fs';
const JADE = process.env.JADE || 'http://localhost:4180';
const NAP = process.env.NAP || 'http://localhost:4179';
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const TOKEN = process.env.TOKEN;
const S = process.env.SHOTS || '/tmp';
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };
const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 860 } });
const page = await ctx.newPage();
page.on('pageerror', (e) => console.log('jade error:', e.message));
await page.goto(`${JADE}/?network=testnet&nap=${NAP}`);
await page.click('[data-filter="trading"]');
await page.waitForSelector(`[data-coin="${TOKEN}"]`, { timeout: 20000 });
ok(/Zyn Test Stone/.test(await page.textContent(`[data-coin="${TOKEN}"]`)), 'the yard lists ZTEST from the public chain, without a wallet');
await page.screenshot({ path: `${S}/jade-yard.png` });
await page.click(`[data-coin="${TOKEN}"]`);
await page.waitForSelector('#coin-title:has-text("Zyn Test Stone")');
ok(/of 51,098 lots/.test(await page.textContent('#chart-title')), `coin page: ${await page.textContent('#chart-title')}`);
await page.fill('#trade-input', '0.001');
await page.waitForTimeout(200);
ok(/ZTEST/.test(await page.textContent('#trade-output')), `quote: ${await page.textContent('#trade-output')}, fee ${await page.textContent('#trade-fee')}`);
// Connect through Nap in the browser.
await page.click('#wallet-button');
await page.waitForSelector('.jade-wallet-option');
const [popup] = await Promise.all([ctx.waitForEvent('page'), page.click('.jade-wallet-option:has-text("Nap in the browser")')]);
await popup.waitForSelector('#unlock');
await popup.fill('#words', WORDS);
await popup.click('#unlock button[type=submit]');
await popup.waitForSelector('#yes');
await popup.click('#yes');
await page.waitForFunction(() => !/Connect wallet/.test(document.querySelector('#wallet-button').textContent), null, { timeout: 60000 });
ok(true, `connected: ${(await page.textContent('#wallet-button')).trim()}`);
// Buy: Nap shows what the chain says; approve.
await page.click('.trade-submit');
await popup.waitForSelector('#yes', { timeout: 60000 });
const approval = await popup.textContent('main');
ok(/Buy ZTEST with 0\.001 ZEC\.zy/.test(approval) && /If nobody else buys this epoch/.test(approval) && approval.includes(new URL(JADE).host), 'Nap shows the buy as the chain prices it, and names the site');
await popup.screenshot({ path: `${S}/jade-approve.png` });
await popup.click('#yes');
await page.waitForFunction(() => /Placed|refused|Not enough|error|not answering/i.test(document.querySelector('#trade-status').textContent), null, { timeout: 90000 }).catch(() => {});
if (!/Placed/.test(await page.textContent('#trade-status'))) console.log('status:', await page.textContent('#trade-status'), '| nap:', await popup.textContent('main').catch(() => '-'));
ok(true, `status: ${await page.textContent('#trade-status')}`);
await page.screenshot({ path: `${S}/jade-coin.png` });
await b.close();
