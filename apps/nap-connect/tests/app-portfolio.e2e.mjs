// The Zyn app: sign in with an Ethereum wallet, see the portfolio, link a
// Solana wallet to the same account, then sign in with the Solana wallet
// alone and find the same account. Stand-in wallets (Wallet Standard and
// EIP-6963) sign deterministically.
//
//   node tests/app-portfolio.e2e.mjs                 (APP=… KEYRING=… to point elsewhere)
//
// KEYRING=http://127.0.0.1:8595 sends the keyring calls to a local
// zyn-receipts instead of the node (everything else goes to the node).
import { chromium } from 'playwright-core';
const APP = process.env.APP || 'https://app.zyn-network.com';
const KEYRING = process.env.KEYRING || '';
const S = process.env.SHOTS || '/tmp';
let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) failed++; };
const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
await ctx.addInitScript(() => {
  const hash = async (bytes) => new Uint8Array(await crypto.subtle.digest('SHA-512', bytes));
  const salt = localStorage.getItem('test.salt') || String(Math.random());
  localStorage.setItem('test.salt', salt);
  const evm = {
    async request({ method, params }) {
      if (method === 'eth_requestAccounts') return ['0x2222222222222222222222222222222222222222'];
      if (method === 'eth_chainId') return '0xaa36a7';
      if (method === 'personal_sign') { const h = await hash(new TextEncoder().encode(params[0] + salt)); return '0x' + [...h, 27].map((x) => x.toString(16).padStart(2, '0')).join('').slice(0, 130); }
      throw Object.assign(new Error('unsupported ' + method), { code: 4200 });
    },
  };
  window.addEventListener('eip6963:requestProvider', () => window.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: Object.freeze({ info: { uuid: 'test-evm', name: 'Test EVM', icon: '', rdns: 'test' }, provider: evm }) })));
  const account = { address: 'HBvV7YqSRSPW4YEBsDvpvF2PrUWFubqVbTNYafkddTsy', publicKey: new Uint8Array(32), chains: ['solana:devnet'], features: [] };
  const sol = { name: 'Test Solana', icon: 'data:image/png;base64,iVBORw0KGgo=', version: '1.0.0', chains: ['solana:devnet'], accounts: [account], features: {
    'standard:connect': { version: '1.0.0', connect: async () => ({ accounts: [account] }) },
    'solana:signMessage': { version: '1.0.0', signMessage: async ({ message }) => [{ signedMessage: message, signature: (await hash(new Uint8Array([...message, ...new TextEncoder().encode(salt)]))).slice(0, 64) }] },
    'solana:signAndSendTransaction': { version: '1.0.0', signAndSendTransaction: async () => [{ signature: new Uint8Array(64).fill(7) }] },
  } };
  window.addEventListener('wallet-standard:app-ready', (e) => e.detail.register(sol));
});
if (KEYRING) {
  await ctx.route('https://zyn-rpc.cavebro.win/object', async (route) => {
    const body = route.request().postDataJSON?.() || {};
    if (!/^zyn_keyring/.test(body.method || '')) return route.continue();
    const r = await fetch(KEYRING, { method: 'POST', body: JSON.stringify(body) });
    const t = await r.text();
    route.fulfill({ contentType: 'application/json', body: r.ok ? `{"jsonrpc":"2.0","id":${JSON.stringify(body.id)},"result":${t}}` : JSON.stringify({ jsonrpc: '2.0', id: body.id, error: { code: -32000, message: t } }) });
  });
}
const page = await ctx.newPage();
const errs = []; page.on('pageerror', (e) => errs.push(e.message));
const signedIn = async () => { try { await page.waitForFunction(() => /^[0-9a-f]{5}…/.test(document.querySelector("#connect").textContent.trim()), null, { timeout: 90000 }); } catch { console.log("NOTE:", await page.textContent("#w-note"), errs); throw new Error("not signed in"); } };
await page.goto(`${APP}/#/portfolio`);
await page.waitForTimeout(1200);
ok(/Connect wallet/.test(await page.textContent('#pf')), 'the portfolio asks for a wallet first');
await page.click('#pf-connect');
await page.waitForSelector('#wallet-list button:has-text("Test EVM")');
await page.click('#wallet-list button:has-text("Test EVM")');
await signedIn();
const acct = (await page.textContent('#connect')).trim();
await page.waitForFunction(() => /Nothing here yet|SOL\.zy|ETH\.zy|ZEC\.zy/.test(document.querySelector('#pf-coins')?.textContent || ''), null, { timeout: 90000 }).catch(() => {});
const pf = (await page.textContent('#pf')).replace(/\s+/g, ' ');
ok(/Zyn account/.test(pf) && /0x2222…2222/.test(pf) && /Link a wallet/.test(pf), `the portfolio shows the account, the Ethereum wallet, and offers to link Solana: ${pf.slice(0, 140)}`);
ok(/Nothing here yet|\.zy/.test(await page.textContent('#pf-coins')), 'assets are read from the account');
// Link the Solana wallet.
await page.click('[data-link="sol"]');
await page.waitForSelector('#wallet-list button:has-text("Test Solana")');
await page.click('#wallet-list button:has-text("Test Solana")');
await page.waitForFunction(() => /linked/.test(document.querySelector('#toast').textContent) || /keyring|refused|Error|another/i.test(document.querySelector('#w-note').textContent), null, { timeout: 90000 }).catch(() => {});
ok(/is linked/.test(await page.textContent('#toast')), `the Solana wallet is linked: ${(await page.textContent('#toast')).trim()} ${(await page.textContent('#w-note')).trim()}`);
await page.waitForTimeout(800);
const both = (await page.textContent('.pf-wallets')).replace(/\s+/g, ' ');
ok(/HBvV7Y…dTsy/.test(both) && /0x2222…2222/.test(both), `both wallets show on the portfolio: ${both.slice(0, 160)}`);
await page.screenshot({ path: `${S}/app-portfolio.png`, fullPage: true });
// Sign out; sign in with the Solana wallet alone: the same account.
await page.click('#connect'); await page.click('#w-out');
await page.click('#connect');
await page.waitForSelector('#wallet-list button:has-text("Test Solana")');
await page.click('#wallet-list button:has-text("Test Solana")');
await signedIn();
ok((await page.textContent('#connect')).trim() === acct, `the Solana wallet now opens the same account (${acct})`);
// Sending checks its inputs before anything is signed.
await page.goto(`${APP}/#/portfolio`);
ok(!errs.length, `no page errors ${errs.join(' | ')}`);
await b.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exit(failed ? 1 : 0);
