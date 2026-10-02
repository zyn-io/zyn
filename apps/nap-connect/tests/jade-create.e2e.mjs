// Jade, creating a coin end to end on testnet: the form with a picture,
// Nap's approval, the coin on chain, and its picture attached to the site's
// content store under the creator's proof.
//
//   JADE=https://jade.zyn-network.com NAP=https://nap.zyn-network.com
//   PHRASE_FILE=<a testnet wallet's phrase>  IMAGE=<a png>  node tests/jade-create.e2e.mjs
import { chromium } from 'playwright-core';
import { unlockNap } from './unlock.mjs';
import { readFileSync } from 'node:fs';

const JADE = process.env.JADE || 'https://jade.zyn-network.com';
const NAP = process.env.NAP || 'https://nap.zyn-network.com';
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const IMAGE = process.env.IMAGE;
const S = process.env.SHOTS || '/tmp';
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };
const stamp = Date.now().toString(36).slice(-4).toUpperCase();
const NAME = `E2E Coin ${stamp}`;
const SYMBOL = `E2E${stamp}`.slice(0, 8);

const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
const page = await ctx.newPage();
page.on('pageerror', (e) => console.log('jade error:', e.message));
await page.goto(`${JADE}/launch?network=testnet&nap=${NAP}`);

// Connect through Nap in the browser.
await page.click('#wallet-button');
await page.waitForSelector('.jade-wallet-option');
const [popup] = await Promise.all([ctx.waitForEvent('page'), page.click('.jade-wallet-option:has-text("Nap in the browser")')]);
await unlockNap(popup, WORDS);
await popup.waitForSelector('#yes');
await popup.click('#yes');
await page.waitForFunction(() => !/Connect wallet/.test(document.querySelector('#wallet-button').textContent), null, { timeout: 60000 });
ok(true, 'connected');

// Fill the form, picture included; the page shrinks it before anything leaves.
await page.fill('#create-name', NAME);
await page.fill('#create-symbol', SYMBOL);
await page.fill('#create-description', 'Made by the end-to-end test. Safe to ignore.');
await page.setInputFiles('#create-image', IMAGE);
await page.waitForFunction(() => document.querySelector('#preview-thumb img'), null, { timeout: 10000 }).catch(() => {});
ok(await page.locator('#preview-thumb img').count() === 1, 'the preview shows the picture');
ok(await page.isEnabled('#create-go'), 'the create button is enabled for a valid ticker and name');

// Create: Nap shows the coin and the picture, and names the site.
await page.click('#create-go');
await popup.waitForSelector('#yes', { timeout: 60000 });
const approval = await popup.textContent('main');
ok(approval.includes(SYMBOL) && approval.includes(new URL(JADE).host), `Nap shows ${SYMBOL} and names the site`);
ok(await popup.locator('main img').count() >= 1, 'Nap shows the picture it is about to attach');
await popup.screenshot({ path: `${S}/jade-create-approve.png` });
await popup.click('#yes');
// "Created." starts the success line; the idle hint also contains the word, lower case.
await page.waitForFunction(() => { const t = document.querySelector('#create-status').textContent; return t.startsWith('Created') || /refused|rejected|not enough|failed|did not/i.test(t); }, null, { timeout: 120000 }).catch(() => {});
const status = await page.textContent('#create-status');
ok(status.startsWith('Created'), `created: ${status}`);
ok(/Picture and description attached/.test(status), 'the picture was attached under the creator\'s proof');

// The page moves to the coin once the epoch seals; its picture is served.
await page.waitForURL(/\/coin\/[0-9a-f]{64}/, { timeout: 60000 }).catch(() => {});
const token = (page.url().match(/\/coin\/([0-9a-f]{64})/) || [])[1];
ok(!!token, `the coin page opened: ${token ? token.slice(0, 12) : 'no'}…`);
if (token) {
  await page.waitForSelector('#coin-title', { timeout: 30000 });
  await page.waitForFunction((n) => document.querySelector('#coin-title').textContent.includes(n), NAME, { timeout: 60000 }).catch(() => {});
  ok((await page.textContent('#coin-title')).includes(NAME), `the chain lists "${NAME}"`);
  await page.waitForSelector('#coin-thumb img', { timeout: 20000 }).catch(() => {});
  ok(await page.locator('#coin-thumb img').count() === 1, 'the coin page shows its picture');
  const content = await (await fetch(`${JADE}/api/content/${token}`)).json();
  ok(/^data:image\//.test(content.image || '') && /end-to-end test/.test(content.description || ''), 'the content store serves the picture and description');
  const again = await fetch(`${JADE}/api/content/${token}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ content: { description: 'hijack', image: '' }, proof: { publicKey: '00'.repeat(32), nonce: '00'.repeat(32), signature: '00'.repeat(64) } }) });
  ok(again.status === 409 || again.status === 403, `a second attach is refused (${again.status})`);
  await page.screenshot({ path: `${S}/jade-create-coin.png`, fullPage: true });
}
await b.close();
