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
await page.waitForSelector(`[data-coin="${TOKEN}"]`, { timeout: 20000 });
ok(/Zyn Test Stone/.test(await page.textContent(`[data-coin="${TOKEN}"]`)) && /ZEC/.test(await page.textContent(`[data-coin="${TOKEN}"] .j-card-cap`)), 'the yard lists ZTEST with its market cap, from the public chain, without a wallet');
await page.screenshot({ path: `${S}/jade-yard.png` });
await page.click(`[data-coin="${TOKEN}"]`);
await page.waitForSelector('#coin-title:has-text("Zyn Test Stone")');
await page.waitForFunction(() => /ZEC/.test(document.querySelector('#coin-mcap').textContent));
const drawn = await page.waitForSelector('#chart canvas', { timeout: 20000 }).then(() => true, () => false);
ok(/%/.test(await page.textContent('#curve-pct')) && drawn, `coin page: cap ${await page.textContent('#coin-mcap')}, curve ${await page.textContent('#curve-pct')}, chart drawn`);
await page.fill('#trade-input', '0.001');
await page.waitForTimeout(200);
ok(/ZTEST/.test(await page.textContent('#trade-output')) && /ZTEST/.test(await page.textContent('#trade-min')), `quote: ${await page.textContent('#trade-output')}, at least ${await page.textContent('#trade-min')}`);
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
await page.waitForFunction(() => /You hold/.test(document.querySelector('#trade-balance').textContent), null, { timeout: 60000 });
ok(true, `connected: ${(await page.textContent('#wallet-button')).trim()} — ${await page.textContent('#trade-balance')}`);
// Buy: Nap shows what the chain says, with the floor; approve; the page collects after the seal.
async function trade(label, check) {
  await page.click('.trade-submit');
  await popup.waitForSelector('#yes', { timeout: 60000 });
  const approval = await popup.textContent('main');
  ok(check(approval) && /At least/.test(approval) && approval.includes(new URL(JADE).host), `${label}: Nap shows it as the chain prices it, with its floor, and names the site`);
  await popup.screenshot({ path: `${S}/jade-approve-${label}.png` });
  await popup.click('#yes');
  await page.waitForFunction(() => /Placed|refused|Not enough|error|not answering/i.test(document.querySelector('#trade-status').textContent), null, { timeout: 90000 }).catch(() => {});
  ok(/Placed/.test(await page.textContent('#trade-status')), `${label}: ${await page.textContent('#trade-status')}`);
  await page.waitForFunction(() => /Filled|Settled/.test(document.querySelector('#trade-status').textContent), null, { timeout: 180000 }).catch(() => {});
  ok(/Filled/.test(await page.textContent('#trade-status')), `${label}, collected without a prompt: ${await page.textContent('#trade-status')}`);
}
await trade('buy', (t) => /Buy ZTEST with 0\.001 ZEC\.zy/.test(t) && /If nobody else buys this epoch/.test(t));
await page.click('[data-trade="sell"]');
await page.fill('#trade-input', '12500');
await page.waitForTimeout(200);
await trade('sell', (t) => /Sell 12,500 ZTEST/.test(t));
await page.screenshot({ path: `${S}/jade-coin.png`, fullPage: true });
await page.goto(`${JADE}/portfolio?network=testnet&nap=${NAP}`).catch(() => {});
await b.close();
