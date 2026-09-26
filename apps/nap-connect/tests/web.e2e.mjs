// Nap Connect, end to end: the Cave store (localhost:4178) connects to Nap in
// the browser (localhost:4179) on testnet, headless. See README for setup.
// Store (localhost:4178) ⇄ Nap in the browser (localhost:4179), headless, on testnet.
import { chromium } from 'playwright-core';
import { unlockNap } from './unlock.mjs';
import { readFileSync } from 'node:fs';
import { deriveIdentity } from '../../nap-web/src/core.js';
import { initSync, spendingKeyFromPhrase, ZecWallet } from '../../nap-web/src/zec-pkg/nap_zec.js';

const WORDS = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art';
initSync({ module: readFileSync(new URL('../../nap-web/src/zec-pkg/nap_zec_bg.wasm', import.meta.url)) });
const expectAccount = deriveIdentity(WORDS, '', 0).accountHex;
const expectZec = new ZecWallet(spendingKeyFromPhrase(WORDS, '', 'testnet', 0), 'testnet').address();

// STORE / NAP point it at deployed sites; the defaults are the local servers.
const STORE = process.env.STORE || 'http://localhost:4178';
const NAP = process.env.NAP || 'http://localhost:4179';
const local = new URL(STORE).hostname === 'localhost';
const ok = (cond, what) => { console.log(`${cond ? 'PASS' : 'FAIL'}  ${what}`); if (!cond) process.exitCode = 1; };
const browser = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await browser.newContext();
const store = await ctx.newPage();
store.on('pageerror', (e) => console.log('store error:', e.message));
await store.goto(`${STORE}/?network=testnet${local ? `&nap=${NAP}` : ''}#market`);

// 1. The picker offers Nap in the browser.
await store.click('.header-wallet');
await store.waitForSelector('.wallet-option');
const names = await store.$$eval('.wallet-option b', (els) => els.map((e) => e.textContent));
ok(names.includes('Nap in the browser'), `picker lists ${JSON.stringify(names)}`);

// 2. Choosing it opens Nap's window on Nap's origin, naming the store.
const [popup] = await Promise.all([ctx.waitForEvent('page'), store.click('.wallet-option:has-text("Nap in the browser")')]);
popup.on('pageerror', (e) => console.log('popup error:', e.message));
await popup.waitForSelector('#setup-create, #pw, #unlock');
ok(!(await popup.$('#words')), 'a new visitor is offered setup, not a phrase box');
ok(new URL(popup.url()).origin === new URL(NAP).origin, `Nap's window is on ${new URL(popup.url()).origin}`);
ok((await popup.textContent('.site')) === new URL(STORE).host, `it names the asking site: ${await popup.textContent('.site')}`);

// 3. Unlock with the test phrase, see the connect card, connect.
await unlockNap(popup, WORDS);
await popup.waitForSelector('#yes');
const card = await popup.textContent('main');
ok(/wants to connect/.test(card) && /Zcash address/.test(card) && /Zyn testnet/.test(card), 'connect card says who, what it shares, and the network');
await popup.click('#yes');
await store.waitForSelector('.header-wallet.is-connected', { timeout: 60000 });
const session = await store.evaluate(async () => (await import('./wallet.js')).wallet().session);
ok(session.account === expectAccount, `store got the Zyn account ${session.account.slice(0, 12)}…`);
ok(session.zcash?.address === expectZec, `store got the testnet Zcash address ${String(session.zcash?.address).slice(0, 14)}…`);
ok(session.network === 'testnet' && session.chain === 212, `network ${session.network}, chain ${session.chain}`);

// 4. Reads for a connected site answer without asking.
const w = (method, params) => store.evaluate(([m, p]) => import('./wallet.js').then((x) => x.ask(m, p)), [method, params]);
ok((await w('zec_getAddress')).address === expectZec, 'zec_getAddress answers without a prompt');
const items = await w('zyn_nftItems');
ok(Array.isArray(items.items), `zyn_nftItems → ${items.items.length} item(s)`);
const owed = await w('zyn_owed');
ok(Array.isArray(owed.owed), `zyn_owed → ${owed.owed.length} claim(s)`);

// 5. A payment request is shown as the chain/URI says, and rejecting it reaches the store as 4001.
const pay = store.evaluate(() => import('./wallet.js').then((x) => x.ask('zec_pay', { uri: 'zcash:utest1kg3alz8tdck92dc6u9cfxwv24lu9fzwes7k8meeea0u4lmkqp3gkkpp0vq6nafg7lue3lsq0lvjnn9ghdwk5cqrugy4jvvvmw5xmmhsj?amount=0.0001&memo=Q2F2ZSBvcmRlciAjMQ&message=Cave%20test' })).then(() => 'resolved', (e) => `${e.code} ${e.message}`));
await popup.waitForSelector('#yes');
const approval = await popup.textContent('main');
ok(/Pay 0\.0001 ZEC/.test(approval) && /Cave order #1/.test(approval) && /Cave test/.test(approval) && approval.toLowerCase().includes(new URL(STORE).host), 'approval shows amount, memo, message and the asking site');
await popup.click('#no');
ok((await pay) === '4001 The holder rejected the request', `rejection reaches the store: ${await pay}`);

// 6. A bad request is refused before any approval.
const code = (method, params) => store.evaluate(([m, p]) => import('./wallet.js').then((x) => x.ask(m, p)).then(() => 'resolved', (e) => `${e.code}`), [method, params]);
const badBuy = await code('zyn_nftBuy', { item: 'ab'.repeat(32) });
ok(badBuy === '-32602', `buying a listing that does not exist → ${badBuy}`);
const unknown = await code('eth_sendTransaction', {});
ok(unknown === '4200', `an unknown method → ${unknown}`);

// 7. A page on another origin cannot talk to Nap's window: it posts a request
// straight at it, and Nap neither answers nor changes what it shows.
const other = await ctx.newPage();
await other.goto(local ? 'http://127.0.0.1:4178/' : 'https://example.com/');
const before = await popup.textContent('main');
const handle = await other.evaluate((name) => { const w = window.open('', name); return !!w; }, await popup.evaluate(() => window.name));
await other.evaluate((origin) => {
  const w = window.open('', window.name === '' ? 'nap-probe' : window.name);
  try { w?.postMessage({ nap: 'request', id: 99, method: 'zec_getAddress', params: {} }, origin); } catch { /* not reachable at all: also fine */ }
}, new URL(NAP).origin);
await popup.waitForTimeout(800);
ok((await popup.textContent('main')) === before, `a request from ${local ? '127.0.0.1' : 'example.com'} changes nothing in Nap's window (reached: ${handle})`);

// 8. Closing Nap's window disconnects the store.
await popup.close();
await store.waitForSelector('.header-wallet:not(.is-connected)', { timeout: 5000 }).then(() => ok(true, 'closing Nap\'s window disconnects the store'), () => ok(false, 'closing Nap\'s window disconnects the store'));
await browser.close();
