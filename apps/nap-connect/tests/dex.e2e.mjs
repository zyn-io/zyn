// Nap Market — the DEX — end to end on testnet, through the real page
// (nap.cavebro.win/market.html): open a pool, collect its shares, swap in a
// batch, add and remove liquidity, collecting what each leaves owed.
//
//   PHRASE_FILE=<wallet holding both assets>  A=<asset hex>  B=<asset hex>
//   AMOUNT_A=0.1 AMOUNT_B=0.004  node tests/dex.e2e.mjs
import { chromium } from 'playwright-core';
import { readFileSync } from 'node:fs';

const NAP = process.env.NAP || 'https://nap.cavebro.win';
const S = process.env.SHOTS || '/tmp';
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const A = process.env.A;
const B = process.env.B;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };

const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const page = await (await b.newContext({ viewport: { width: 1280, height: 950 } })).newPage();
page.on('pageerror', (e) => console.log('page error:', e.message));

const notice = async (re, timeout = 120000) => {
  await page.waitForFunction((src) => new RegExp(src).test(document.querySelector('#notice')?.textContent || ''), re.source, { timeout });
  return page.textContent('#notice');
};
const tab = async (v) => { await page.click(`#tabs [data-view="${v}"]`); await page.waitForTimeout(400); };
const refresh = async () => { await page.click('#refresh'); await notice(/Up to date/); };
/// Collect everything owed, once something is (a batch clears at the seal).
async function collect(what) {
  for (let i = 0; i < 20; i++) {
    await tab('owed');
    if (await page.locator('#collect').count()) break;
    await page.waitForTimeout(6000);
    await refresh();
  }
  let said = '';
  for (let i = 0; i < 10; i++) {
    await page.click('#collect');
    said = await notice(/Collected|refused|not|One moment/i);
    if (!/One moment/.test(said)) break;
    await page.waitForTimeout(3000);
    await tab('owed');
  }
  ok(/Collected/.test(said), `collected ${what}`);
  // The page refreshes after an action; let it land before the next one.
  await page.waitForTimeout(4000);
}
const bal = async (asset) => page.evaluate((a) => [...document.querySelectorAll('select[name="a"] option')].find((o) => o.value === a)?.textContent || '', asset);

await page.goto(`${NAP}/market.html`);
await page.evaluate(() => localStorage.setItem('nap.network', 'testnet'));
await page.goto(`${NAP}/market.html`);
await page.click('#open-wallet');
await page.fill('#words', WORDS);
await page.click('#phrase-form button[type=submit]');
await notice(/Up to date/);

// Open the pool.
await tab('pools');
ok(/SOL\.zy|ETH\.zy/.test(await bal(A) + await bal(B)), `the page names the bridged assets: ${(await bal(A)).trim()} and ${(await bal(B)).trim()}`);
if (!(await page.locator('.mk-pool').count())) {
  await page.selectOption('#create-pool select[name="a"]', A);
  await page.selectOption('#create-pool select[name="b"]', B);
  await page.fill('#create-pool input[name="amount_a"]', process.env.AMOUNT_A || '0.1');
  await page.fill('#create-pool input[name="amount_b"]', process.env.AMOUNT_B || '0.004');
  await page.click('#create-pool button[type=submit]');
  ok(/Pool open/.test(await notice(/Pool open|refused|exists|enough/i)), 'the pool opens');
  await collect('the pool shares');
  await tab('pools');
} else {
  console.log('the pool already exists; continuing from the swap');
}
await page.waitForSelector('.mk-pool', { timeout: 60000 });
const poolId = await page.getAttribute('.mk-pool [data-trade]', 'data-trade');
ok(!!poolId, `the pool is listed (${String(poolId).slice(0, 10)}…)`);
await page.screenshot({ path: `${S}/dex-pool.png`, fullPage: true });

// Swap in a batch.
await tab('swap');
// Sell the asset named SELL (default SOL.zy): a pool orders its pair by
// address, so the page's first side may be either.
const SELL = process.env.SELL || 'SOL.zy';
if (!(await page.textContent('.mk-leg:not(.mk-leg-out) b')).includes(SELL)) await page.click('#flip');
await page.fill('#amount', process.env.SWAP || '0.01');
await page.waitForTimeout(300);
const quote = await page.textContent('#quote');
ok(Number(quote.replace(/,/g, '')) > 0, `the page quotes ${quote} for the swap`);
await page.click('#swap-form button[type=submit]');
const placed = await notice(/Order placed|refused|enough|Not/i);
ok(/Order placed/.test(placed), `the order goes into the batch — ${placed}`);
await collect('the swap, after its batch cleared');
await page.screenshot({ path: `${S}/dex-swapped.png`, fullPage: true });

// Add liquidity, then take some out.
await tab('pools');
await page.click(`[data-add="${poolId}"]`);
await page.fill(`#add-${poolId} input[name="a"]`, process.env.ADD_A || '0.02');
await page.waitForTimeout(300);
const matched = await page.inputValue(`#add-${poolId} input[name="b"]`);
if (!matched) await page.fill(`#add-${poolId} input[name="b"]`, process.env.ADD_B || '0.0008');
await page.click(`#add-${poolId} button[type=submit]`);
ok(/Liquidity added/.test(await notice(/Liquidity added|refused|enough/i)), 'liquidity is added');
await collect('the new shares');
await tab('pools');
await page.click(`[data-remove="${poolId}"]`);
await page.fill(`#remove-${poolId} input[name="s"]`, process.env.REMOVE || '0.001');
await page.click(`#remove-${poolId} button[type=submit]`);
ok(/Liquidity removed/.test(await notice(/Liquidity removed|refused|enough/i)), 'liquidity is removed');
await collect('both sides of what was removed');
await tab('pools');
await page.screenshot({ path: `${S}/dex-done.png`, fullPage: true });
await b.close();
