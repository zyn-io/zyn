// A site swaps in a pool through Nap in the browser — the path a graduated
// launchpad coin trades on (`zyn_swap`), for any pair, whatever it is quoted
// in. A bare page served here stands in for the site; Nap is the real one.
//
//   PHRASE_FILE=<wallet holding what it sells>  POOL=<pool hex>  SELL=<asset hex>
//   AMOUNT=0.1  IN=TESTZ.zy OUT=TGRAD  node tests/swap.e2e.mjs
import { chromium } from 'playwright-core';
import { unlockNap } from './unlock.mjs';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';

const NAP = process.env.NAP || 'https://nap.cavebro.win';
const NODE = process.env.NODE_URL || 'https://zyn-rpc.cavebro.win/object';
const S = process.env.SHOTS || '/tmp';
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const { POOL, SELL } = process.env;
const AMOUNT = process.env.AMOUNT || '0.1';
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };

const rpc = async (method, params = []) => (await (await fetch(NODE, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) })).json()).result;
const pool = (await rpc('zyn_market')).pools.find((p) => p.id === POOL);
if (!pool) throw new Error('no such pool on the ledger');
const sellA = pool.a === SELL;

const site = `<!doctype html><meta charset="utf-8"><title>site</title>
<button id="connect">Connect</button><button id="swap">Swap</button><button id="collect">Collect</button><pre id="out"></pre>
<script type="module">
import { webWallet } from './nap-connect.js';
const nap = webWallet({ url: ${JSON.stringify(NAP)} });
const out = (x) => { document.querySelector('#out').textContent = JSON.stringify(x); };
const ask = (method, params) => nap.request({ method, params }).then(out, (e) => out({ error: e.message }));
document.querySelector('#connect').onclick = () => ask('nap_connect', { network: 'testnet' });
document.querySelector('#swap').onclick = () => ask('zyn_swap', ${JSON.stringify({ pool: POOL, sellA, amount: AMOUNT, slippageBps: 300 })});
document.querySelector('#collect').onclick = () => ask('zyn_collect');
</script>`;
const server = createServer((req, res) => {
  if (req.url === '/nap-connect.js') { res.writeHead(200, { 'content-type': 'text/javascript' }); res.end(readFileSync(new URL('../nap-connect.js', import.meta.url))); return; }
  res.writeHead(200, { 'content-type': 'text/html' }); res.end(site);
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const SITE = `http://127.0.0.1:${server.address().port}`;

const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext();
const page = await ctx.newPage();
await page.goto(SITE);
const answer = async () => {
  await page.waitForFunction(() => document.querySelector('#out').textContent, null, { timeout: 180000 });
  const t = await page.textContent('#out');
  await page.evaluate(() => { document.querySelector('#out').textContent = ''; });
  return JSON.parse(t);
};

// Connect.
const [popup] = await Promise.all([ctx.waitForEvent('page'), page.click('#connect')]);
await unlockNap(popup, WORDS);
await popup.waitForSelector('#yes');
await popup.click('#yes');
const session = await answer();
ok(!!session.account, `connected: ${String(session.account).slice(0, 12)}… on ${session.network}`);

// Swap: Nap names what goes in and what comes out, as the chain prices it.
await page.click('#swap');
await popup.waitForSelector('#yes', { timeout: 60000 });
const approval = (await popup.textContent('main')).replace(/\s+/g, ' ');
ok((!process.env.IN || approval.includes(`Swap ${process.env.IN} for ${process.env.OUT}`)) && /At least/.test(approval), `Nap's approval: ${approval.slice(0, 160)}`);
await popup.screenshot({ path: `${S}/swap-approve.png` });
await popup.click('#yes');
const placed = await answer();
ok(/^[0-9a-f]{64}$/.test(placed.order || ''), `the order is in the batch (${String(placed.order || placed.error).slice(0, 16)}…)`);

// Collect once the batch clears; collecting needs no prompt.
let got = 0;
for (let i = 0; i < 20 && !got; i++) {
  await page.waitForTimeout(15000);
  await page.click('#collect');
  got = (await answer()).collected || 0;
}
ok(got > 0, `collected the swap after its batch cleared (${got})`);
await b.close();
server.close();
