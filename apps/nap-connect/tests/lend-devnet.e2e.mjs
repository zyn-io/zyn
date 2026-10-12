//   KEY=<throwaway.json> node tests/lend-devnet.e2e.mjs     (APP=… to point elsewhere)
//
// A testnet SOL deposit goes to devnet whatever network the wallet is set
// to: the wallet only signs (solana:signTransaction) and the page sends it
// to devnet itself — Phantom sends a signAndSend to its own network. A
// stand-in wallet signs for real with a throwaway key (an Ed25519 JWK file,
// { address, d, x }). Unfunded, the page says so before the wallet is
// asked; told it holds SOL, its deposit reaches devnet, which checks the
// signature and then finds nothing to debit.
import { chromium } from 'playwright-core';
import { readFileSync } from 'node:fs';
const APP = process.env.APP || 'https://lend.zyn-network.com';
const key = JSON.parse(readFileSync(process.env.KEY, 'utf8'));
let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) failed++; };
const b = await chromium.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });

async function attempt(rich) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
  await ctx.addInitScript((k) => {
    window.__calls = [];
    const account = { address: k.address, publicKey: new Uint8Array(32), chains: ['solana:mainnet', 'solana:devnet'], features: [] };
    const signer = crypto.subtle.importKey('jwk', { kty: 'OKP', crv: 'Ed25519', d: k.d, x: k.x }, { name: 'Ed25519' }, false, ['sign']);
    const sol = { name: 'Test Devnet', icon: 'data:image/png;base64,iVBORw0KGgo=', version: '1.0.0', chains: ['solana:mainnet', 'solana:devnet'], accounts: [account], features: {
      'standard:connect': { version: '1.0.0', connect: async () => ({ accounts: [account] }) },
      // Like Phantom: a signAndSend would go to the wallet's own network.
      'solana:signAndSendTransaction': { version: '1.0.0', signAndSendTransaction: async () => { window.__calls.push('signAndSend'); throw new Error('sent on the wallet\'s own network'); } },
      'solana:signTransaction': { version: '1.0.0', signTransaction: async ({ transaction, chain }) => {
        window.__calls.push(`sign ${chain}`);
        const tx = new Uint8Array(transaction);
        const sig = new Uint8Array(await crypto.subtle.sign({ name: 'Ed25519' }, await signer, tx.slice(65)));
        const out = tx.slice(); out.set(sig, 1);
        return [{ signedTransaction: out }];
      } },
    } };
    window.addEventListener('wallet-standard:app-ready', (e) => e.detail.register(sol));
  }, key);
  // Told it is funded: inside the page, devnet's getBalance answers 10 SOL;
  // every other request goes from Chrome to devnet as it would live.
  if (rich) {
    await ctx.addInitScript(() => {
      const real = window.fetch;
      window.fetch = (url, init) => {
        if (String(url).startsWith('https://api.devnet.solana.com') && /"getBalance"/.test(String(init?.body || ''))) {
          return Promise.resolve(new Response(JSON.stringify({ jsonrpc: '2.0', id: 1, result: { context: { slot: 1 }, value: 10_000_000_000 } }), { headers: { 'content-type': 'application/json' } }));
        }
        return real(url, init);
      };
    });
  }
  const page = await ctx.newPage();
  const sent = [];
  page.on('request', (r) => { if (r.url().startsWith('https://api.devnet.solana.com') && /sendTransaction/.test(r.postData() || '')) sent.push(r.postData()); });
  // A deposit handed over by Nap's popup: no Zyn account needed in the page.
  await page.goto(`${APP}/#/bridge/deposit/sol?code=${'ab'.repeat(32)}`);
  await page.waitForSelector('#br-amount');
  await page.fill('#br-amount', '0.01');
  await page.click('#br-go');
  await page.waitForFunction(() => /holds|debit|credit|Sent|refused|error/i.test(document.querySelector('#br-note')?.textContent || ''), null, { timeout: rich ? 8000 : 60000 }).catch(() => {});
  const noteText = (await page.textContent('#br-note')).trim();
  const calls = await page.evaluate(() => window.__calls);
  await ctx.close();
  return { noteText, calls, sent };
}

const poor = await attempt(false);
ok(/holds 0 SOL on devnet; this needs 0\.01 SOL/.test(poor.noteText) && /faucet\.solana\.com/.test(poor.noteText), `unfunded on devnet: said before the wallet is asked: ${poor.noteText}`);
ok(!poor.calls.length, `the wallet was not asked: ${JSON.stringify(poor.calls)}`);

const rich = await attempt(true);
ok(JSON.stringify(rich.calls) === '["sign solana:devnet"]', `the wallet only signs, for devnet: ${JSON.stringify(rich.calls)}`);
ok(rich.sent.length === 1, `the page sent it to devnet itself: ${rich.sent.length} request`);
// Devnet's verdict on what the page sent, asked again from here (headless
// Chrome can sit on devnet's answer; a real browser has it in ~100 ms): it
// parses the transaction and checks the signature, then finds nothing to debit.
const verdict = rich.sent[0] ? await (await fetch('https://api.devnet.solana.com', { method: 'POST', headers: { 'content-type': 'application/json' }, body: rich.sent[0] })).text() : '';
ok(/AccountNotFound|prior credit/.test(verdict), `devnet takes the page's signed deposit as valid and finds nothing to debit: ${verdict.slice(0, 120)}`);
await b.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exit(failed ? 1 : 0);
