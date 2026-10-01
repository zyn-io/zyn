//   node tests/lend-wallets.e2e.mjs        (APP=… to point elsewhere)
//
// Zyn Lend with wallets a holder already has: a stand-in Solana wallet (Wallet
// Standard) and a stand-in Ethereum wallet (EIP-6963), both signing
// deterministically, open a Zyn account in the page.
import { chromium } from 'playwright-core';
const APP = process.env.APP || 'https://lend.zyn-network.com';
const S = process.env.SHOTS || '/tmp';
let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) failed++; };
const b = await chromium.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
await ctx.addInitScript(() => {
  const hash = async (bytes) => new Uint8Array(await crypto.subtle.digest('SHA-512', bytes));
  window.__calls = [];
  // Ethereum, announced the EIP-6963 way.
  const evm = {
    async request({ method, params }) {
      window.__calls.push(method);
      if (method === 'eth_requestAccounts') return ['0x1111111111111111111111111111111111111111'];
      if (method === 'eth_chainId') return '0xaa36a7';
      if (method === 'personal_sign') { const h = await hash(new TextEncoder().encode(params[0])); return '0x' + [...h, 27].map((x) => x.toString(16).padStart(2, '0')).join('').slice(0, 130); }
      if (method === 'eth_sendTransaction') { window.__sent = params[0]; return '0x' + 'ab'.repeat(32); }
      throw Object.assign(new Error('unsupported ' + method), { code: 4200 });
    },
  };
  window.addEventListener('eip6963:requestProvider', () => window.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: Object.freeze({ info: { uuid: 'test-evm', name: 'Test EVM', icon: '', rdns: 'test' }, provider: evm }) })));
  // Solana, registered the Wallet Standard way.
  const account = { address: 'HBvV7YqSRSPW4YEBsDvpvF2PrUWFubqVbTNYafkddTsy', publicKey: new Uint8Array(32), chains: ['solana:devnet'], features: [] };
  const sol = { name: 'Test Solana', icon: 'data:image/png;base64,iVBORw0KGgo=', version: '1.0.0', chains: ['solana:devnet'], accounts: [account], features: {
    'standard:connect': { version: '1.0.0', connect: async () => ({ accounts: [account] }) },
    'solana:signMessage': { version: '1.0.0', signMessage: async ({ message }) => [{ signedMessage: message, signature: (await hash(message)).slice(0, 64) }] },
    'solana:signAndSendTransaction': { version: '1.0.0', signAndSendTransaction: async () => [{ signature: new Uint8Array(64).fill(7) }] },
  } };
  window.addEventListener('wallet-standard:app-ready', (e) => e.detail.register(sol));
});
const page = await ctx.newPage();
const errs = []; page.on('pageerror', (e) => errs.push(e.message));
await page.goto(`${APP}/#/markets`);
await page.waitForTimeout(1500);
ok((await page.textContent('#connect')).trim() === 'Connect wallet', `the header offers any wallet: ${(await page.textContent('#connect')).trim()}`);
await page.click('#connect');
await page.waitForSelector('#wallet-list button', { timeout: 15000 });
await page.waitForTimeout(1200);
const list = (await page.textContent('#wallet-list')).replace(/\s+/g, ' ');
ok(/Solana.*Test Solana/.test(list) && /Ethereum.*Test EVM/.test(list) && /Nap/.test(list), `wallets grouped by chain, Nap kept: ${list.slice(0, 160)}`);
await page.click('#wallet-list button:has-text("Test EVM")');
await page.waitForFunction(() => /^[0-9a-f]{5}…/.test(document.querySelector('#connect').textContent.trim()) || /declined|Error|cannot/i.test(document.querySelector('#w-note').textContent), null, { timeout: 60000 }).catch(() => {});
const head = (await page.textContent('#connect')).trim();
ok(/^[0-9a-f]{5}…/.test(head), `an Ethereum wallet opened a Zyn account: ${head} ${(await page.textContent('#w-note')).trim()}`);
ok((await page.evaluate(() => window.__calls.filter((c) => c === 'personal_sign').length)) === 2, 'signed twice the first time on this device (stable check)');
const acctEvm = head;
// The same wallet again: the same account, one signature.
await page.click('#connect'); await page.click('#w-out');
await page.evaluate(() => { window.__calls = []; });
await page.click('#connect');
await page.waitForSelector('#wallet-list button:has-text("Test EVM")');
await page.click('#wallet-list button:has-text("Test EVM")');
await page.waitForFunction(() => /^[0-9a-f]{5}…/.test(document.querySelector('#connect').textContent.trim()), null, { timeout: 60000 }).catch(() => {});
ok((await page.textContent('#connect')).trim() === acctEvm, 'signing again opens the same account');
ok((await page.evaluate(() => window.__calls.filter((c) => c === 'personal_sign').length)) === 1, 'once known, one signature');
// Recovery words, for Nap.
await page.click('#connect'); await page.click('#w-words');
const words = (await page.textContent('.words')).trim().split(/\s+/);
ok(words.length === 24, `24 recovery words shown on request (${words.length})`);
await page.keyboard.press('Escape');
// A write goes through this page's own approval.
await page.goto(`${APP}/#/lend/ETH`); await page.waitForTimeout(800);
await page.fill('#lend-amount', '0.001'); await page.click('#lend-go');
await page.waitForTimeout(4000);
const asked = await page.evaluate(() => document.querySelector('#approve').open);
const t = (await page.textContent('#toast')).trim();
ok(asked || /does not hold enough|hold/i.test(t), `lending asks this page's approval, or refuses an empty account first: ${asked ? await page.textContent('#a-title') : t}`);
if (asked) await page.click('#a-no');
// The bridge pays to and from the wallet the account was opened with.
await page.goto(`${APP}/#/bridge`); await page.waitForTimeout(800);
await page.check('input[name="br-mode"][value="out"]'); await page.check('#br-asset input[value="eth"]'); await page.waitForTimeout(300);
ok((await page.inputValue('#br-to')) === '0x1111111111111111111111111111111111111111', 'an ETH withdrawal goes to the connected wallet by default');
await page.check('input[name="br-mode"][value="in"]'); await page.waitForTimeout(300);
await page.fill('#br-amount', '0.001'); await page.click('#br-go');
await page.waitForFunction(() => !!window.__sent || /error|refused/i.test(document.querySelector('#br-note').textContent), null, { timeout: 30000 }).catch(() => {});
const sent = await page.evaluate(() => window.__sent);
ok(sent && sent.from === '0x1111111111111111111111111111111111111111' && /^0x52d29193/.test(sent.data), `the ETH deposit is sent by the connected wallet with this account's code: ${JSON.stringify(sent)?.slice(0, 90)}`);
await page.screenshot({ path: `${S}/lend-evm.png` });
// Solana opens its own account.
await page.click('#connect'); await page.click('#w-out');
await page.click('#connect');
await page.waitForSelector('#wallet-list button:has-text("Test Solana")');
await page.click('#wallet-list button:has-text("Test Solana")');
await page.waitForFunction(() => /^[0-9a-f]{5}…/.test(document.querySelector('#connect').textContent.trim()), null, { timeout: 60000 }).catch(() => {});
const sh = (await page.textContent('#connect')).trim();
ok(/^[0-9a-f]{5}…/.test(sh) && sh !== acctEvm, `a Solana wallet opens its own account: ${sh}`);
ok(!errs.length, `no page errors ${errs.join(' | ')}`);
await b.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exit(failed ? 1 : 0);
