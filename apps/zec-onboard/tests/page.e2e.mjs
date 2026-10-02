// The guide in a real browser: both widths, both themes, the network switch,
// ticks surviving a reload, and the two tools.
//
//   SITE=http://localhost:8788 node tests/page.e2e.mjs
//
// playwright-core comes from apps/nap-connect; Chrome for Testing from
// Playwright's cache (CHROMIUM overrides).

import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import { mkdirSync } from 'node:fs';

const require = createRequire(new URL('../../nap-connect/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const CHROMIUM = process.env.CHROMIUM || `${homedir()}/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing`;
const SITE = process.env.SITE || 'http://localhost:8788';
const SHOTS = process.env.SHOTS || '/tmp/zec-onboard';
mkdirSync(SHOTS, { recursive: true });

let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'} ${what}`); if (!ok) failed++; };

const browser = await chromium.launch({ executablePath: CHROMIUM, headless: true });

for (const [w, h, scheme] of [[1280, 900, 'light'], [390, 844, 'light'], [390, 844, 'dark'], [1280, 900, 'dark']]) {
  const ctx = await browser.newContext({ viewport: { width: w, height: h }, colorScheme: scheme, deviceScaleFactor: 2 });
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('console', (m) => { if (m.type() === 'error' && !/shots\/|Failed to load resource/.test(m.text())) errors.push(m.text()); });
  await page.goto(SITE, { waitUntil: 'networkidle' });
  const tag = `${w}-${scheme}`;
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${tag}: no horizontal scroll`);
  await page.screenshot({ path: `${SHOTS}/hero-${tag}.png` });
  await page.screenshot({ path: `${SHOTS}/full-${tag}.png`, fullPage: true });

  if (w === 1280 && scheme === 'light') {
    // Testnet swaps the copy.
    await page.click('.net button[data-net="test"]');
    check(await page.isVisible('text=Valar testnet faucet'), 'testnet: faucet step shows');
    check(!(await page.isVisible('text=such as Coinbase')), 'testnet: exchange step hidden');

    // Ticks survive a reload, and so does the network.
    await page.click('button.done[data-step="1"]');
    await page.click('button.done[data-step="3"]');
    await page.reload({ waitUntil: 'networkidle' });
    check((await page.textContent('#count')) === '2 of 6 done', 'ticks survive a reload');
    check((await page.getAttribute('html', 'data-net')) === 'test', 'network survives a reload');
    check((await page.getAttribute('button.done[data-step="3"]', 'aria-pressed')) === 'true', 'step 3 shows done');

    // Address checker.
    const addr = async (a) => {
      await page.fill('#addr', a);
      await page.click('#addr-form button');
      await page.waitForTimeout(50);
      return page.textContent('#addr-out');
    };
    check(/Transparent address.*testnet/.test(await addr('tmAMWUnnjjpM7TDb81PykSxGJpThsw453ND')), 'checker: tm address');
    check(/mainnet address; this page is set to testnet/.test(await addr('t1PKtYdJJHhc3Pxowmznkg7vdTwnhEsCvR4')), 'checker: wrong network warned');
    check(/checksum/.test(await addr('t1PKtYdJJHhc3Pxowmznkg7vdTwnhEsCvR5')), 'checker: typo caught');
    await page.click('.net button[data-net="main"]');
    check(/Unified address, mainnet/.test(await addr('u1kc38pww6c8ahz0z20g9kv9s7afrc2npmr8s6nngw00qufveq43q5m2vh8u7wxpaltl0vjs7etr7y7d0pcm5dtc24pqhu5qfe5cwtyc2u')), 'checker: u1 address');

    // Transaction lookup.
    await page.fill('#txid', 'zz');
    await page.click('#tx-form button');
    check(/64 characters/.test(await page.textContent('#tx-out')), 'lookup: bad id refused');
    const tx = 'a11ea66a'.padEnd(64, '0');
    await page.fill('#txid', tx);
    await page.click('#tx-form button');
    check((await page.getAttribute('#tx-out a', 'href')) === `https://mainnet.zcashexplorer.app/transactions/${tx}`, 'lookup: mainnet explorer link');

    // Info bubble opens, Escape closes it.
    await page.click('.chain .i');
    check(await page.isVisible('.chain .bubble'), 'bubble opens');
    await page.keyboard.press('Escape');
    check(!(await page.isVisible('.chain .bubble')), 'Escape closes the bubble');

    // The film loads and is a minute long.
    const filmDur = await page.evaluate(() => new Promise((r) => {
      const v = document.querySelector('.film video');
      v.preload = 'metadata';
      v.addEventListener('loadedmetadata', () => r(v.duration), { once: true });
      v.load();
      setTimeout(() => r(-1), 10000);
    }));
    check(Math.abs(filmDur - 60) < 1, `film loads (${filmDur.toFixed ? filmDur.toFixed(1) : filmDur}s)`);
    check(await page.evaluate(() => { const i = new Image(); i.src = document.querySelector('.film video').poster; return new Promise((r) => { i.onload = () => r(i.naturalWidth > 0); i.onerror = () => r(false); }); }), 'film poster loads');

    await page.click('#reset');
    check((await page.textContent('#count')) === '0 of 6 done', 'clear progress');
  }
  if (w === 390) {
    await page.click('#s1 .i');
    const box = await page.locator('#s1 .bubble').boundingBox();
    check(box && box.x >= 0 && box.x + box.width <= 390, `${tag}: bubble stays on screen`);
    await page.screenshot({ path: `${SHOTS}/bubble-${tag}.png` });
  }
  check(errors.length === 0, `${tag}: no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
  await ctx.close();
}

await browser.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exit(failed ? 1 : 0);
