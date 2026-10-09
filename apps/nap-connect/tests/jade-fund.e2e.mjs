// Jade brings a coin in (§178), end to end on testnet: a buy of a SOL-priced
// coin larger than the account's SOL.zy. Jade asks Nap for the deposit code,
// a Solana wallet sends the rest to the devnet vault with it, the bridge
// credits it, Jade collects, and the buy goes to Nap for approval.
//
// The Solana wallet is a stand-in registered through the Wallet Standard; it
// signs with the Solana CLI's devnet key, here in Node, and sends the
// transaction to devnet for real. The page sees only the signature.
//
//   JADE=http://localhost:4391 (wrangler pages dev public, in apps/jade)   NAP=https://nap.zyn-network.com
//   TOKEN=<a SOL-priced curve on testnet>   PHRASE_FILE=<a testnet wallet's phrase>
//   SOL_KEY=~/.config/solana/id.json (funded on devnet)   EXTRA=0.02 (SOL to bring in)
//   node tests/jade-fund.e2e.mjs
import { chromium } from 'playwright-core';
import { createPrivateKey, sign } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { unlockNap } from './unlock.mjs';
import { b58encode } from '../../nap-web/src/solana.js';

const JADE = process.env.JADE || 'http://localhost:4391';
const NAP = process.env.NAP || 'https://nap.zyn-network.com';
const TOKEN = process.env.TOKEN || '0ffd35a3b9cfdbd972c6d515d5abae89ccc0f0887aa56abcf8a6f9b429fc583d';
const SYM = process.env.SYMBOL || 'SOLTEST';
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const EXTRA = process.env.EXTRA || '0.02';
const DEVNET = 'https://api.devnet.solana.com';
const S = process.env.SHOTS || '/tmp';
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };

// The devnet key: its seed signs, its public half is the address.
const secret = Uint8Array.from(JSON.parse(readFileSync(process.env.SOL_KEY || `${homedir()}/.config/solana/id.json`, 'utf8')));
const key = createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.from(secret.slice(0, 32))]), format: 'der', type: 'pkcs8' });
const address = b58encode(secret.slice(32));

const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
let sent = null;
await ctx.exposeBinding('__solSend', async (_src, bytes) => {
  const tx = Uint8Array.from(bytes);
  const sig = sign(null, tx.slice(65), key);
  tx.set(sig, 1);
  const r = await fetch(DEVNET, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'sendTransaction', params: [Buffer.from(tx).toString('base64'), { encoding: 'base64', preflightCommitment: 'confirmed' }] }) }).then((x) => x.json());
  if (r.error) throw new Error(r.error.message);
  sent = r.result;
  return [...sig];
});
await ctx.addInitScript((addr) => {
  const account = { address: addr, publicKey: new Uint8Array(32), chains: ['solana:devnet'], features: ['solana:signAndSendTransaction'] };
  const wallet = {
    version: '1.0.0', name: 'Test Solana', icon: 'data:image/png;base64,iVBORw0KGgo=', chains: ['solana:devnet'], accounts: [account],
    features: {
      'standard:connect': { version: '1.0.0', connect: async () => ({ accounts: [account] }) },
      'solana:signAndSendTransaction': { version: '1.0.0', signAndSendTransaction: async ({ transaction }) => [{ signature: Uint8Array.from(await window.__solSend([...transaction])) }] },
    },
  };
  window.addEventListener('wallet-standard:app-ready', (e) => e.detail.register(wallet));
}, address);

const page = await ctx.newPage();
page.on('pageerror', (e) => console.log('jade error:', e.message));
await page.goto(`${JADE}/coin/${TOKEN}?network=testnet&nap=${NAP}`);
await page.waitForSelector(`#coin-title:has-text("${SYM}"), #coin-title`, { timeout: 30000 });
await page.click('#wallet-button');
await page.waitForSelector('.jade-wallet-option');
const [popup] = await Promise.all([ctx.waitForEvent('page'), page.click('.jade-wallet-option:has-text("Nap in the browser")')]);
await unlockNap(popup, WORDS);
await popup.waitForSelector('#yes');
await popup.click('#yes');
await page.waitForFunction(() => /SOL\.zy/.test(document.querySelector('#trade-balance').textContent), null, { timeout: 60000 });
const balText = await page.textContent('#trade-balance');
const held = Number((balText.match(/·\s*([\d.,]+)\s*SOL\.zy/) || [])[1]?.replace(/,/g, '') || 0);
const want = (held + Number(EXTRA)).toFixed(9).replace(/0+$/, '').replace(/\.$/, '');
ok(true, `connected — ${balText}; buying with ${want} SOL.zy`);

// The order is larger than the balance: the page offers to bring the rest.
await page.fill('#trade-input', want);
await page.waitForTimeout(300);
const box = await page.textContent('#trade-fund').catch(() => '');
const label = (await page.textContent('.trade-submit')).trim();
ok(!(await page.isHidden('#trade-fund')) && /from your Solana wallet/.test(box) && /Solana shows your wallet paying the Zyn vault/.test(box), `the fund box: ${box.replace(/\s+/g, ' ').trim()}`);
ok(/^Bring 0\.0\d+ SOL, then buy$/.test(label), `the button: ${label}`);
await page.screenshot({ path: `${S}/jade-fund-offer.png` });

const before = Date.now();
await page.click('.trade-submit');
await page.waitForFunction(() => /Sent .* SOL/.test(document.querySelector('#trade-status').textContent), null, { timeout: 90000 }).catch(() => {});
ok(!!sent && /Sent/.test(await page.textContent('#trade-status')), `sent on devnet: ${sent} — ${await page.textContent('#trade-status')}`);
ok(/explorer\.solana\.com\/tx\/.+cluster=devnet/.test(await page.getAttribute('#trade-fund-link a', 'href').catch(() => '')), 'the status links the deposit on the explorer');
await page.screenshot({ path: `${S}/jade-fund-sent.png` });

// The bridge credits it; Jade collects; then Nap asks to approve the buy.
await popup.waitForSelector('#yes', { timeout: 10 * 60_000 });
const approval = await popup.textContent('main');
ok(approval.includes(`Buy ${SYM}`) && approval.includes('SOL.zy'), `after ${Math.round((Date.now() - before) / 1000)} s the buy is in Nap: ${approval.replace(/\s+/g, ' ').slice(0, 120)}…`);
await popup.click('#yes');
await page.waitForFunction(() => /Filled|Not filled|Settled/.test(document.querySelector('#trade-status').textContent), null, { timeout: 240_000 }).catch(() => {});
ok(/Filled/.test(await page.textContent('#trade-status')), `the buy: ${await page.textContent('#trade-status')}`);
await page.screenshot({ path: `${S}/jade-fund-done.png` });
await b.close();
