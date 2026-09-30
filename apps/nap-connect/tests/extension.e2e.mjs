// The Nap Wallet extension on its own — no desktop app: the popup restores a
// wallet and reads testnet, and a site finds the extension, connects through
// its connect window (unlocked with the password the popup kept), and has a
// swap approved there.
//
//   PHRASE_FILE=<funded testnet phrase>  POOL=<pool hex>  SELL=<asset hex>
//   AMOUNT=0.01  IN=TESTZ.zy OUT=TGRAD  node tests/extension.e2e.mjs
//
// Needs a Chromium that honours --load-extension (CHROMIUM=…, default:
// Playwright's Chrome for Testing; branded Chrome no longer loads unpacked
// extensions from the command line). Build first: `npm run build` in
// apps/extension.
import { chromium } from 'playwright-core';
import { unlockNap } from './unlock.mjs';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';

const EXT = new URL('../../extension/dist', import.meta.url).pathname;
const CHROMIUM = process.env.CHROMIUM || `${homedir()}/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing`;
const S = process.env.SHOTS || '/tmp';
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const PASSWORD = 'extension-e2e-password';
const { POOL, SELL } = process.env;
const AMOUNT = process.env.AMOUNT || '0.01';
const NODE = process.env.NODE_URL || 'https://zyn-rpc.cavebro.win/object';
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };

const rpc = async (method, params = []) => (await (await fetch(NODE, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) })).json()).result;
const pool = POOL && (await rpc('zyn_market')).pools.find((p) => p.id === POOL);
const sellA = pool ? pool.a === SELL : false;

// A bare site that finds Nap the way every site does.
const site = `<!doctype html><meta charset="utf-8"><title>site</title>
<button id="find">Find</button><button id="connect">Connect</button><button id="swap">Swap</button><button id="collect">Collect</button><pre id="out"></pre>
<script type="module">
import { discover, connect } from './nap-connect.js';
let nap = null;
const out = (x) => { document.querySelector('#out').textContent = JSON.stringify(x); };
const ask = (method, params) => nap.provider.request({ method, params }).then(out, (e) => out({ error: e.message, code: e.code }));
document.querySelector('#find').onclick = async () => { const list = await discover(); nap = list.find((w) => w.info.rdns === 'win.cavebro.nap.extension'); out(list.map((w) => w.info.name + ' / ' + w.info.detail)); };
document.querySelector('#connect').onclick = () => connect(nap, { network: 'testnet' }).then((c) => out(c.session), (e) => out({ error: e.message }));
document.querySelector('#swap').onclick = () => ask('zyn_swap', ${JSON.stringify({ pool: POOL, sellA, amount: AMOUNT, slippageBps: 300 })});
document.querySelector('#collect').onclick = () => ask('zyn_collect');
</script>`;
const server = createServer((req, res) => {
  if (req.url === '/nap-connect.js') { res.writeHead(200, { 'content-type': 'text/javascript' }); res.end(readFileSync(new URL('../nap-connect.js', import.meta.url))); return; }
  res.writeHead(200, { 'content-type': 'text/html' }); res.end(site);
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const SITE = `http://127.0.0.1:${server.address().port}`;

const ctx = await chromium.launchPersistentContext('', {
  executablePath: CHROMIUM,
  headless: true,
  args: ['--headless=new', `--disable-extensions-except=${EXT}`, `--load-extension=${EXT}`],
});
try {
  const worker = ctx.serviceWorkers()[0] || await ctx.waitForEvent('serviceworker');
  const id = new URL(worker.url()).host;
  ok(/^[a-p]{32}$/.test(id), `the extension loaded (${id})`);

  // The popup, as a page: restore, keep with a password, read testnet.
  const popup = await ctx.newPage();
  await popup.setViewportSize({ width: 400, height: 600 });
  await popup.goto(`chrome-extension://${id}/index.html?popup=1`);
  await popup.evaluate(() => localStorage.setItem('nap.network', 'testnet'));
  await popup.reload();
  await popup.waitForSelector('#restore-form');
  await popup.fill('#words', WORDS);
  await popup.fill('#keep-password', PASSWORD);
  await popup.click('#restore-form button[type=submit]');
  await popup.waitForFunction(() => /TESTZ\.zy|TGRAD|ZEC\.zy/.test(document.querySelector('#coins')?.textContent || ''), null, { timeout: 90000 });
  const coins = (await popup.textContent('#coins')).replace(/\s+/g, ' ');
  ok(/TESTZ\.zy/.test(coins) && /TGRAD/.test(coins), `the popup reads the wallet from testnet by itself: ${coins.slice(0, 90)}…`);
  ok(await popup.evaluate(() => document.documentElement.scrollWidth <= 400), 'the popup fits its 400px width');
  await popup.screenshot({ path: `${S}/ext-popup.png` });
  ok(await popup.evaluate(() => !!localStorage.getItem('nap.kept.v1') && !/[a-z]+ [a-z]+ [a-z]+/.test(localStorage.getItem('nap.kept.v1'))), 'the key is kept sealed on the extension, never the phrase');

  // A site finds the extension and connects.
  const page = await ctx.newPage();
  await page.goto(SITE);
  const answer = async () => {
    await page.waitForFunction(() => document.querySelector('#out').textContent, null, { timeout: 180000 });
    const t = await page.textContent('#out');
    await page.evaluate(() => { document.querySelector('#out').textContent = ''; });
    return JSON.parse(t);
  };
  await page.click('#find');
  const found = await answer();
  ok(found[0] === 'Nap Wallet / Extension', `the site finds the extension first: ${JSON.stringify(found)}`);
  const [win] = await Promise.all([ctx.waitForEvent('page'), page.click('#connect')]);
  await win.waitForLoadState();
  ok(win.url() === `chrome-extension://${id}/connect.html`, "Nap's own window opens for the site");
  // The popup's setup already opened a session for this browser, so the
  // window may skip the password and show the card straight away.
  await unlockNap(win, WORDS, { password: PASSWORD });
  await win.waitForSelector('#yes');
  const card = (await win.textContent('main')).replace(/\s+/g, ' ');
  ok(card.includes(new URL(SITE).host), `the connect card names the site: ${card.slice(0, 80)}…`);
  await win.click('#yes');
  const session = await answer();
  ok(/^[0-9a-f]{64}$/.test(session.account || '') && session.network === 'testnet', `connected: ${String(session.account).slice(0, 12)}… on ${session.network}`);

  if (POOL) {
    await page.click('#swap');
    await win.waitForSelector('#yes', { timeout: 60000 });
    const approval = (await win.textContent('main')).replace(/\s+/g, ' ');
    ok((!process.env.IN || approval.includes(`Swap ${process.env.IN} for ${process.env.OUT}`)) && approval.toLowerCase().includes(new URL(SITE).host.toLowerCase()), `the swap is approved in Nap's window: ${approval.slice(0, 110)}…`);
    await win.screenshot({ path: `${S}/ext-approve.png` });
    await win.click('#yes');
    const placed = await answer();
    ok(/^[0-9a-f]{64}$/.test(placed.order || ''), `signed in the extension and placed (${String(placed.order || placed.error).slice(0, 16)}…)`);
    let got = 0;
    for (let i = 0; i < 20 && !got; i++) {
      await page.waitForTimeout(15000);
      await page.click('#collect');
      got = (await answer()).collected || 0;
    }
    ok(got > 0, `collected after the batch cleared (${got})`);
  }

  // Closing Nap's window does not lock the extension (the toolbar's session
  // lives on until Lock or 15 idle minutes): the next request reopens the
  // window and is answered without a password. Lock ends that.
  await win.close();
  const [again] = await Promise.all([ctx.waitForEvent('page', { timeout: 30000 }), page.click('#collect')]);
  ok(again.url() === `chrome-extension://${id}/connect.html`, 'after closing it, the next request reopens Nap');
  await unlockNap(again, WORDS, { password: PASSWORD }).catch(() => {});
  const after = await answer();
  ok(typeof after.collected === 'number', `unlocked again, the request is answered: ${JSON.stringify(after)}`);
} finally {
  await ctx.close();
  server.close();
}
