// Zyn Lend's portfolio: sign in with an Ethereum wallet, see the portfolio, link a
// Solana wallet to the same account, then sign in with the Solana wallet
// alone and find the same account; unlink it and it opens its own again.
// Stand-in wallets (Wallet Standard and EIP-6963) sign deterministically; the
// Solana one signs in through `solana:signIn` with a real ed25519 key, and
// adds a field to the message the way some wallets do.
//
//   node tests/app-portfolio.e2e.mjs                 (APP=… KEYRING=… to point elsewhere)
//
// KEYRING=http://127.0.0.1:8595 sends the keyring calls to a local
// zyn-receipts instead of the node (everything else goes to the node).
import { chromium } from 'playwright-core';
const APP = process.env.APP || 'https://lend.zyn-network.com';
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
  // A real ed25519 key per run (from the salt), so the page can check the
  // signature against the address.
  const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  const b58 = (bytes) => { let n = 0n; for (const b of bytes) n = n * 256n + BigInt(b); let o = ''; while (n > 0n) { o = B58[Number(n % 58n)] + o; n /= 58n; } for (const b of bytes) { if (b) break; o = '1' + o; } return o; };
  const key = (async () => {
    const seed = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(`sol${salt}`)));
    const der = Uint8Array.from([...'302e020100300506032b657004220420'.match(/../g).map((x) => parseInt(x, 16)), ...seed]);
    const k = await crypto.subtle.importKey('pkcs8', der, { name: 'Ed25519' }, true, ['sign']);
    const x = (await crypto.subtle.exportKey('jwk', k)).x;
    const pub = Uint8Array.from(atob(x.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
    window.__solAddress = b58(pub);
    return { k, account: { address: b58(pub), publicKey: pub, chains: ['solana:devnet'], features: [] } };
  })();
  const sol = { name: 'Test Solana', icon: 'data:image/png;base64,iVBORw0KGgo=', version: '1.0.0', chains: ['solana:devnet'], get accounts() { return []; }, features: {
    'standard:connect': { version: '1.0.0', connect: async () => ({ accounts: [(await key).account] }) },
    'solana:signIn': { version: '1.0.0', signIn: async (input) => {
      const { k, account } = await key;
      // The sign-in format, with a chain id the site did not ask for.
      const fields = [`URI: ${input.uri}`, `Version: ${input.version}`, 'Chain ID: solana:devnet', `Nonce: ${input.nonce}`, `Issued At: ${input.issuedAt}`];
      const text = `${input.domain} wants you to sign in with your Solana account:\n${account.address}\n\n${input.statement}\n\n${fields.join('\n')}`;
      window.__siws = text;
      const signedMessage = new TextEncoder().encode(text);
      const signature = new Uint8Array(await crypto.subtle.sign({ name: 'Ed25519' }, k, signedMessage));
      return [{ account, signedMessage, signature }];
    } },
    'solana:signMessage': { version: '1.0.0', signMessage: async ({ message }) => { const { k } = await key; return [{ signedMessage: message, signature: new Uint8Array(await crypto.subtle.sign({ name: 'Ed25519' }, k, message)) }]; } },
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
// Its addresses: the Zyn account in full, and its own Zcash ones, derived on the page.
await page.waitForFunction(() => /^utest1/.test(document.querySelector('#pf-id-zec')?.textContent || ''), null, { timeout: 90000 });
const ids = await page.evaluate(() => ({ zyn: document.querySelector('#pf-id-zyn').textContent, u: document.querySelector('#pf-id-zec').textContent, t: document.querySelector('#pf-id-zect').textContent }));
ok(/^[0-9a-f]{64}$/.test(ids.zyn) && ids.zyn.startsWith(acct.slice(0, 5)) && /^tm/.test(ids.t), `addresses: Zyn ${ids.zyn.slice(0, 8)}…, Zcash ${ids.u.slice(0, 14)}…, ${ids.t}`);
// Export: the warning first, the words only when asked.
await page.click('#pf-export-go');
ok(/account's key, on Zyn and on Zcash/.test(await page.textContent('#pf-export')) && !(await page.$('#pf-export .words')), 'export warns before showing anything');
await page.click('#pf-export-show');
const words = (await page.textContent('#pf-export .words')).trim();
const exp = (await page.textContent('#pf-export')).replace(/\s+/g, ' ');
ok(words.split(/\s+/).length === 24 && /Birthday height\s*4,320,000/.test(exp) && /Account\s*0/.test(exp) && /Zashi/.test(exp), `export shows 24 words, account 0 and the birthday: ${exp.slice(exp.indexOf('Network'), exp.indexOf('Network') + 90)}`);
// The words, restored on their own, give the same Zcash addresses.
{
  const base = `${APP}/vendor/zec/`;
  const z = await import(`data:text/javascript,${encodeURIComponent(await (await fetch(`${base}nap_zec.js`)).text())}`);
  await z.default({ module_or_path: Buffer.from(await (await fetch(`${base}nap_zec_bg.wasm`)).arrayBuffer()) });
  const w = new z.ZecWallet(z.spendingKeyFromPhrase(words, '', 'testnet', 0), 'testnet');
  w.setTransparent(z.transparentKeyFromPhrase(words, '', 'testnet', 0));
  ok(w.address() === ids.u && w.transparentAddress() === ids.t, 'the exported words restore the same Zcash addresses (ZIP-32 / BIP-44, account 0)');
}
await page.click('#pf-export-hide');
// Link the Solana wallet.
await page.click('[data-link="sol"]');
await page.waitForSelector('#wallet-list button:has-text("Test Solana")');
await page.click('#wallet-list button:has-text("Test Solana")');
await page.waitForFunction(() => /linked/.test(document.querySelector('#toast').textContent) || /keyring|refused|Error|another/i.test(document.querySelector('#w-note').textContent), null, { timeout: 90000 }).catch(() => {});
ok(/is linked/.test(await page.textContent('#toast')), `the Solana wallet is linked: ${(await page.textContent('#toast')).trim()} ${(await page.textContent('#w-note')).trim()}`);
await page.waitForTimeout(800);
const solAddr = await page.evaluate(() => window.__solAddress);
const solShort = `${solAddr.slice(0, 6)}…${solAddr.slice(-4)}`;
ok(/^lend\.zyn-network\.com wants you to sign in with your Solana account:\n/.test(await page.evaluate(() => window.__siws)), 'the Solana wallet signed in through solana:signIn');
const both = (await page.textContent('.pf-wallets')).replace(/\s+/g, ' ');
ok(both.includes(solShort) && /0x2222…2222/.test(both), `both wallets show on the portfolio: ${both.slice(0, 160)}`);
await page.screenshot({ path: `${S}/app-portfolio.png`, fullPage: true });
// Sign out; sign in with the Solana wallet alone: the same account.
await page.click('#connect'); await page.click('#w-out');
await page.click('#connect');
await page.waitForSelector('#wallet-list button:has-text("Test Solana")');
await page.click('#wallet-list button:has-text("Test Solana")');
await signedIn();
ok((await page.textContent('#connect')).trim() === acct, `the Solana wallet now opens the same account (${acct})`);
// Unlink it: it opens its own account again, without a "different account"
// warning (the page knows it was unlinked here).
await page.click('#connect');
await page.waitForSelector('[data-unlink="sol"]');
await page.click('[data-unlink="sol"]');
await page.waitForSelector('#approve[open]');
ok(/Unlink Test Solana/.test(await page.textContent('#a-title')), 'unlinking asks first');
await page.click('#a-yes');
await page.waitForFunction(() => /unlinked/.test(document.querySelector('#toast').textContent) || /refused|Error/i.test(document.querySelector('#w-note').textContent), null, { timeout: 60000 }).catch(() => {});
ok(/is unlinked/.test(await page.textContent('#toast')), `the Solana wallet is unlinked: ${(await page.textContent('#toast')).trim()} ${(await page.textContent('#w-note')).trim()}`);
await page.click('#w-out');
await page.click('#connect');
await page.waitForSelector('#wallet-list button:has-text("Test Solana")');
await page.click('#wallet-list button:has-text("Test Solana")');
await signedIn();
const own = (await page.textContent('#connect')).trim();
ok(own !== acct && !(await page.evaluate(() => document.querySelector('#approve').open)), `unlinked, the Solana wallet opens its own account (${own}) without a warning`);
// A wallet that opens a different account than it did here before is warned.
await page.click('#connect'); await page.click('#w-out');
await page.evaluate(() => { for (const k of Object.keys(localStorage)) if (k.startsWith('zyn-lend.account.v2.evm.')) localStorage.setItem(k, '00'.repeat(8)); });
await page.click('#connect');
await page.waitForSelector('#wallet-list button:has-text("Test EVM")');
await page.click('#wallet-list button:has-text("Test EVM")');
// Wait for this warning itself, not just an open dialog: the unlink dialog's
// title can still be in place while sign-in runs.
await page.waitForFunction(() => document.querySelector('#approve').open && /different account/i.test(document.querySelector('#a-title').textContent), null, { timeout: 90000 }).catch(() => {});
ok(/different account/i.test(await page.textContent('#a-title')) && /its signature changed/.test(await page.textContent('#a-note')), `a changed account is flagged before it opens: ${await page.textContent('#a-title')}`);
await page.click('#a-yes');
await signedIn();
ok((await page.textContent('#connect')).trim() === acct, 'and opens once the holder says so');
// Sending checks its inputs before anything is signed.
await page.goto(`${APP}/#/portfolio`);
ok(!errs.length, `no page errors ${errs.join(' | ')}`);
await b.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exit(failed ? 1 : 0);
