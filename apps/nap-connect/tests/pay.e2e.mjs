// Zyn Pay through Nap Connect, as a site uses it: pay-page.html served locally,
// Nap in the browser in its own window, testnet 212. Subscribe (2 BOLD a day,
// 3 times, 2 set aside), wait for the pay clock to pay period 0, top up one
// period, cancel and collect what comes back (found by its claim id), then pay once and find the reference on the chain.
// Every step is approved in Nap, which shows the card the chain will act on.
//
//   PAYEE=<64 hex> PHRASE_FILE=~/.zyn/test-wallets/bidder.phrase \
//   NAP_LOCAL=apps/nap-web/dist node apps/nap-connect/tests/pay.e2e.mjs
//
// PAYEE is a registered Zyn account other than the wallet's own (Nap refuses
// the wallet itself and an account that has not registered). The wallet needs
// about 7 BOLD.zy (4 set aside, 2 topped up, 0.01 paid once). Run after `ops/zyn-pay-rollout.sh testnet`: without ZYN_PAY=on
// the node answers "the pay clock has not started" and this prints SKIP and
// exits 0 without passing anything.
//
// NAP_LOCAL / NAP_UPSTREAM route nap.zyn-network.com as in lend.e2e.mjs.
// RPC overrides the node (default https://zyn-rpc.cavebro.win/object).
// The schedule's first payment is 120 s out; the pay clock moves an hour at most
// per post and posts every 10 s, so the wait is a few minutes (PAY_WAIT_MS caps it).

import { chromium } from 'playwright-core';
import { readFileSync, mkdirSync, existsSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { join, extname, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { unlockNap } from './unlock.mjs';
import { claimId, cancelSource } from '../../nap-web/src/market.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const PAYEE = (process.env.PAYEE || '').trim();
if (!/^[0-9a-f]{64}$/.test(PAYEE)) {
  console.error('Set PAYEE=<64 lowercase hex>: a registered Zyn account other than the test wallet.');
  process.exit(2);
}
if (!process.env.PHRASE_FILE) {
  console.error('Set PHRASE_FILE=<file with the test wallet\'s phrase>.');
  process.exit(2);
}
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const RPC = process.env.RPC || 'https://zyn-rpc.cavebro.win/object';
const S = process.env.SHOTS || '/tmp/pay-e2e';
const WAIT_MS = Number(process.env.PAY_WAIT_MS || 10 * 60 * 1000);
mkdirSync(S, { recursive: true });

const rpc = async (method, fields = {}) => {
  const body = await (await fetch(RPC, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params: [], ...fields }) })).json();
  if (body.error) throw new Error(body.error.message || 'the chain refused');
  return body.result;
};
const NOT_ON = /pay clock has not started|does not serve zyn_pay/i;
const skip = async (b, why) => {
  console.log(`SKIP: Zyn Pay is not switched on for this chain (ZYN_PAY=on)${why ? ` (${why})` : ''}`);
  await b?.close().catch(() => {});
  process.exit(0);
};

// A node without Zyn Pay does not serve its reads at all.
const zero = '0'.repeat(64);
await rpc('zyn_paySchedules', { ids: zero }).catch((e) => { if (NOT_ON.test(e.message)) return skip(null, e.message); });

let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) failed++; };
const squash = (t) => (t || '').replace(/\s+/g, ' ').trim();
const WAD = 10n ** 18n;

// pay-page.html and the one nap-connect.js, from this checkout.
const TYPES = { '.html': 'text/html', '.js': 'text/javascript' };
const files = { '/': join(HERE, 'pay-page.html'), '/pay-page.html': join(HERE, 'pay-page.html'), '/nap-connect.js': join(HERE, '..', 'nap-connect.js') };
const server = createServer((req, res) => {
  const f = files[new URL(req.url, 'http://x').pathname];
  if (!f) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': `${TYPES[extname(f)]}; charset=utf-8` }).end(readFileSync(f));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const APP = `http://127.0.0.1:${server.address().port}`;

const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const NAP_LOCAL = process.env.NAP_LOCAL || '';
const NAP_UPSTREAM = process.env.NAP_UPSTREAM || (NAP_LOCAL ? 'https://nap.cavebro.win' : '');
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 }, ...(NAP_UPSTREAM ? { serviceWorkers: 'block' } : {}) });
if (NAP_UPSTREAM) {
  const NTYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm', '.png': 'image/png', '.webp': 'image/webp', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.webmanifest': 'application/manifest+json' };
  await ctx.route('https://nap.zyn-network.com/**', async (route) => {
    const req = route.request();
    const { pathname, search } = new URL(req.url());
    const pg = pathname === '/' ? '/index.html' : extname(pathname) ? pathname : `${pathname}.html`;
    const file = NAP_LOCAL ? join(NAP_LOCAL, pg) : '';
    if (file && existsSync(file) && statSync(file).isFile()) {
      return route.fulfill({ status: 200, contentType: NTYPES[extname(file)] || 'application/octet-stream', body: readFileSync(file) });
    }
    const r = await fetch(`${NAP_UPSTREAM}${pathname}${search}`, { method: req.method(), headers: { 'content-type': req.headers()['content-type'] || 'application/json' }, body: ['GET', 'HEAD'].includes(req.method()) ? undefined : req.postDataBuffer() });
    route.fulfill({ status: r.status, contentType: r.headers.get('content-type') || undefined, body: Buffer.from(await r.arrayBuffer()) });
  });
}
const page = await ctx.newPage();
page.on('pageerror', (e) => console.log('page error:', e.message));
await page.goto(`${APP}/pay-page.html?payee=${PAYEE}`);
await page.waitForFunction(() => !document.querySelector('#connect').disabled, null, { timeout: 15000 });

let popup = null;
const finish = async () => {
  await b.close().catch(() => {});
  server.close();
  console.log(failed ? `\n${failed} FAILED` : '\nall passed');
  process.exit(failed ? 1 : 0);
};

// Click a button; run `during` while Nap's window is asked something; return
// what the page got back ({ok, result} or {ok:false, message}). A node without
// Zyn Pay ends the run as a skip.
async function act(id, during) {
  await page.click(`#${id}`);
  if (during) await during();
  await page.waitForFunction(() => document.querySelector('#out').dataset.state !== 'busy', null, { timeout: 180000 });
  const last = await page.evaluate(() => window.__last);
  if (last && !last.ok && NOT_ON.test(last.message || '')) await skip(b, last.message);
  return last;
}

// Approve the card Nap shows next, and say what it showed. `own` is the card
// kicker: the approval card says "<host> ASKS", the scope card "SHARE".
async function approve(label, check) {
  await popup.waitForSelector('#yes', { timeout: 90000 }).catch(async (e) => {
    console.log(`${label}: Nap shows ${squash(await popup.textContent('body').catch(() => '(closed)')).slice(0, 300)}`);
    throw e;
  });
  const shown = squash(await popup.textContent('main'));
  ok(check(shown), `${label}: Nap shows ${shown.slice(0, 200)}`);
  await popup.screenshot({ path: `${S}/pay-${label.replace(/\W+/g, '-')}.png` }).catch(() => {});
  await popup.click('#yes');
}

try {
  // Connect, unlocking Nap on the way.
  const [win] = await Promise.all([ctx.waitForEvent('page'), page.click('#connect')]);
  popup = win;
  await unlockNap(popup, WORDS);
  await popup.waitForSelector('#yes');
  await popup.click('#yes');
  await page.waitForFunction(() => document.querySelector('#out').dataset.state !== 'busy', null, { timeout: 60000 });
  const conn = await page.evaluate(() => window.__last);
  ok(conn.ok && /^[0-9a-f]{64}$/.test(conn.result.account), `connected: ${conn.ok ? conn.result.account.slice(0, 12) : conn.message}`);
  ok(conn.result.account !== PAYEE, 'the payee is not the wallet itself');

  // 1. Subscribe: 2 BOLD a day, 3 times, 2 periods set aside.
  const sub = await act('subscribe', () => approve('subscribe', (t) =>
    t.includes('Scheduled payment') && t.includes(PAYEE.slice(0, 8)) && /2 BOLD every day, 3 times, from \d+ \w+ \d{4}/.test(t)
    && /Set aside now\s*4 BOLD \(2 payments\)/.test(t) && /Fee\s*0\.5%/.test(t) && /Cancel any time/.test(t)));
  ok(sub.ok && /^[0-9a-f]{64}$/.test(sub.result?.schedule || '') && sub.result.payment === null, `Subscribe returned a schedule id: ${sub.ok ? sub.result.schedule?.slice(0, 12) : sub.message}`);
  if (!sub.ok) { await finish(); }
  const sid = sub.result.schedule;
  const firstDue = sub.params.firstDue;

  // The payments scope is asked once, on the first Schedules read.
  const readSchedules = async (first) => {
    const r = await act('schedules', first ? () => approve('allow payments', (t) => /asks to see the payments you set up with it/.test(t)) : null);
    return r.ok ? (r.result.schedules || []).find((s) => s.id === sid) : null;
  };
  let row = null;
  // The chain seals a few seconds after the approval: read until the schedule is listed.
  for (let i = 0, t0 = Date.now(); Date.now() - t0 < 180000; i++) {
    row = await readSchedules(i === 0);
    if (row) break;
    await page.waitForTimeout(5000);
  }
  ok(!!row, 'zyn_paySchedules lists the new schedule');
  if (!row) { await finish(); }
  ok(BigInt(row.pot) === 4n * WAD, `pot is 2 periods (4 BOLD): ${row.pot}`);
  ok(Number(row.index) === 0 && row.state === 'active', `index 0, state ${row.state}`);
  ok(row.payee === PAYEE, 'the payee is the one asked for');
  ok(BigInt(row.amount) === 2n * WAD, 'amount per period is 2 BOLD');
  const asset = row.asset;

  // 2. The pay clock pays period 0 once firstDue has passed.
  const wait = Math.max(0, firstDue - Math.floor(Date.now() / 1000));
  console.log(`      (period 0 is due in ${wait} s; waiting for the pay clock, at most ${Math.round(WAIT_MS / 60000)} min)`);
  const t1 = Date.now();
  while (Date.now() - t1 < WAIT_MS) {
    row = await readSchedules(false);
    if (row && Number(row.index) >= 1) break;
    await page.waitForTimeout(15000);
  }
  ok(!!row && Number(row.index) === 1, `index is 1 after the first payment (${Math.round((Date.now() - t1) / 1000)} s of waiting)`);
  ok(!!row && BigInt(row.pot) === 2n * WAD, `pot is 1 period (2 BOLD): ${row?.pot}`);
  if (!row || Number(row.index) !== 1) { await finish(); }

  // 3. Top up one period: the card says what it adds; the pot is 2 periods again.
  const top = await act('topup', () => approve('top up', (t) =>
    t.includes('Top up payment') && t.includes(PAYEE.slice(0, 8)) && /Adds\s*2 BOLD \(1 payment\)/.test(t) && /Funded until \(after\)\s*\d+ \w+ \d{4}/.test(t)));
  ok(top.ok && top.result?.schedule === sid, `Top up returned the schedule: ${top.ok ? 'ok' : top.message}`);
  for (let i = 0, t0 = Date.now(); Date.now() - t0 < 180000; i++) {
    row = await readSchedules(false);
    if (row && BigInt(row.pot) === 4n * WAD) break;
    await page.waitForTimeout(5000);
  }
  ok(!!row && BigInt(row.pot) === 4n * WAD, `pot is 2 periods (4 BOLD) after the top-up: ${row?.pot}`);

  // 4. Cancel: the 2 periods come back as a claim with a known id; collecting takes it.
  const before = BigInt(row?.pot ?? 0);
  const cancel = await act('cancel', () => approve('cancel', (t) => t.includes('Cancel payment') && t.includes(PAYEE.slice(0, 8)) && /Comes back now\s*4 BOLD/.test(t)));
  ok(cancel.ok && cancel.result?.schedule === sid, `Cancel returned the schedule: ${cancel.ok ? 'ok' : cancel.message}`);
  const cid = Buffer.from(claimId(cancelSource(Uint8Array.from(Buffer.from(sid, 'hex'))), 0)).toString('hex');
  const owedHas = async () => {
    const r = await act('owed');
    return r.ok ? (r.result.owed || []).find((o) => o.id === cid) : null;
  };
  let claim = null;
  for (let i = 0, t0 = Date.now(); Date.now() - t0 < 180000; i++) {
    claim = await owedHas();
    if (claim) break;
    await page.waitForTimeout(5000);
  }
  ok(!!claim, `the cancel left claim ${cid.slice(0, 12)} of 4 BOLD: ${claim ? claim.amount : 'none'}`);
  ok(!claim || (claim.asset === asset && BigInt(claim.amount) === before), 'the claim returns the 2 periods in the pay asset');
  if (claim) {
    const col = await act('collect');
    ok(col.ok, `Collect: ${col.ok ? JSON.stringify(col.result) : col.message}`);
    let still = claim;
    for (let i = 0, t0 = Date.now(); still && Date.now() - t0 < 180000; i++) {
      await page.waitForTimeout(5000);
      still = (await act('owed')).result?.owed?.find((o) => o.id === cid);
    }
    ok(!still, 'the claim is collected in Nap (no longer owed)');
  }
  const gone = await readSchedules(false);
  ok(!gone || gone.state === 'cancelled', `the schedule is cancelled: ${gone ? gone.state : 'no longer listed'}`);

  // 5. Pay once: the payment record carries the reference.
  const once = await act('once', () => approve('pay once', (t) => t.includes('Payment') && t.includes(PAYEE.slice(0, 8)) && /Amount\s*0\.01 BOLD/.test(t)));
  ok(once.ok && /^[0-9a-f]{64}$/.test(once.result?.payment || ''), `Pay once returned a payment id: ${once.ok ? once.result.payment?.slice(0, 12) : once.message}`);
  if (once.ok) {
    let rec = null;
    for (let i = 0, t0 = Date.now(); !rec && Date.now() - t0 < 180000; i++) {
      rec = (await rpc('zyn_payPayments', { ids: once.result.payment }).catch(() => []))?.[0] || null;
      if (!rec) await page.waitForTimeout(5000);
    }
    ok(!!rec, 'the payment record is on the chain');
    ok(rec?.reference === once.params.reference, `the record carries the reference: ${rec?.reference?.slice(0, 12)}`);
    ok(rec && BigInt(rec.amount) === WAD / 100n, 'the record holds 0.01 BOLD');
  }
} catch (e) {
  console.log('FAIL ', e.message);
  console.log(`      page said: ${squash(await page.textContent('#out').catch(() => ''))}`);
  failed++;
}
await finish();
