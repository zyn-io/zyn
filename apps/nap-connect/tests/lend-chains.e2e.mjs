//   node tests/lend-chains.e2e.mjs        (APP=… to point elsewhere; testnet)
//
// The bridge's deposits land on the bridge's own chains, whichever wallets a
// browser has and whatever network they are set to:
// * several Solana or Ethereum wallets and none linked: the page asks which
//   pays (it used to take whichever registered first);
// * an Ethereum wallet that answers the switch to Sepolia without switching
//   is refused before anything is sent; one that switches sends with
//   Sepolia's chain id in the transaction.
// Stand-in wallets; balances on devnet and Sepolia are real (unfunded) except
// where a check is told otherwise in the page.
import { chromium } from 'playwright-core';
const APP = process.env.APP || 'https://lend.zyn-network.com';
const SEPOLIA = '0xaa36a7';
const VAULT = '0xe17bb5943b34133f6a8a5d221be4ed9493b3ef27';
let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) failed++; };
const b = await chromium.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });

async function page(setup, arg) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
  await ctx.addInitScript(setup, arg);
  const p = await ctx.newPage();
  return p;
}
const note = (p) => p.textContent('#br-note').then((t) => t.trim());
const waitNote = (p, re) => p.waitForFunction((src) => new RegExp(src).test(document.querySelector('#br-note')?.textContent || ''), re.source, { timeout: 30000 }).catch(() => {});

// Two Solana wallets, none linked: asked which pays; the second is used.
{
  const p = await page(() => {
    const mk = (name, address) => {
      const account = { address, publicKey: new Uint8Array(32), chains: ['solana:devnet'], features: [] };
      return { name, icon: 'data:image/png;base64,iVBORw0KGgo=', version: '1.0.0', chains: ['solana:devnet'], accounts: [account], features: {
        'standard:connect': { version: '1.0.0', connect: async () => ({ accounts: [account] }) },
        'solana:signAndSendTransaction': { version: '1.0.0', signAndSendTransaction: async () => { throw new Error(`${name} was asked to send`); } },
        'solana:signTransaction': { version: '1.0.0', signTransaction: async () => { throw new Error(`${name} was asked to sign`); } },
      } };
    };
    const ws = [mk('Wallet A', 'cW6nGqrf19PKFZi1w1v6CDspwHaiABDpmQn7hYiuJdk'), mk('Wallet B', 'HBvV7YqSRSPW4YEBsDvpvF2PrUWFubqVbTNYafkddTsy')];
    window.addEventListener('wallet-standard:app-ready', (e) => e.detail.register(...ws));
  });
  await p.goto(`${APP}/#/bridge/deposit/sol?code=${'ab'.repeat(32)}`);
  await p.waitForSelector('#br-amount');
  await p.fill('#br-amount', '0.01');
  await p.click('#br-go');
  await p.waitForSelector('#br-note .br-pick', { timeout: 30000 }).catch(() => {});
  const choices = await p.$$eval('#br-note .br-pick', (bs) => bs.map((x) => x.textContent));
  ok(JSON.stringify(choices) === '["Wallet A","Wallet B"]', `two Solana wallets: asked which pays: ${JSON.stringify(choices)}`);
  await p.click('#br-note .br-pick:has-text("Wallet B")').catch(() => {});
  await waitNote(p, /holds|was asked/);
  ok(/^Wallet B (holds .* on devnet|was asked to sign)/.test(await note(p)), `the chosen one is used (its devnet balance, or its signature): ${await note(p)}`);
  await p.context().close();
}

// Ethereum wallets announced the EIP-6963 way, starting on mainnet (0x1).
// `switches`: whether wallet_switchEthereumChain really switches. Sepolia's
// balance is told as 1 ETH in the page, so the chain check is what decides.
const evm = (switches, names = ['Test EVM']) => page(([sw, ns]) => {
  window.__sent = [];
  const real = window.fetch;
  window.fetch = (url, init) => (/"eth_getBalance"/.test(String(init?.body || ''))
    ? Promise.resolve(new Response(JSON.stringify({ jsonrpc: '2.0', id: 1, result: '0xde0b6b3a7640000' }), { headers: { 'content-type': 'application/json' } }))
    : real(url, init));
  const provider = (name) => {
    let chain = '0x1';
    return { async request({ method, params }) {
      if (method === 'eth_requestAccounts' || method === 'eth_accounts') return ['0x2222222222222222222222222222222222222222'];
      if (method === 'eth_chainId') return chain;
      if (method === 'wallet_switchEthereumChain') { if (sw) chain = params[0].chainId; return null; }
      if (method === 'eth_sendTransaction') { window.__sent.push({ name, ...params[0] }); return '0x' + 'ab'.repeat(32); }
      throw Object.assign(new Error('unsupported ' + method), { code: 4200 });
    } };
  };
  window.addEventListener('eip6963:requestProvider', () => ns.forEach((n, i) => window.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: Object.freeze({ info: { uuid: `w${i}`, name: n, icon: '', rdns: `test.${i}` }, provider: provider(n) }) }))));
}, [switches, names]);

async function ethDeposit(p, pick) {
  await p.goto(`${APP}/#/bridge/deposit/eth?code=${'cd'.repeat(32)}`);
  await p.waitForSelector('#br-amount');
  await p.fill('#br-amount', '0.01');
  await p.click('#br-go');
  if (pick) { await p.waitForSelector('#br-note .br-pick', { timeout: 30000 }); await p.click(`#br-note .br-pick:has-text("${pick}")`); }
  await waitNote(p, /Sent\.|nothing was sent|holds/);
  return { text: await note(p), sent: await p.evaluate(() => window.__sent) };
}

{
  const p = await evm(false);
  const r = await ethDeposit(p);
  ok(!r.sent.length && /not on Sepolia.*nothing was sent/.test(r.text), `a wallet that stays on mainnet is refused before anything is sent: ${r.text}`);
  await p.context().close();
}
{
  const p = await evm(true);
  const r = await ethDeposit(p);
  const tx = r.sent[0] || {};
  ok(r.sent.length === 1 && tx.chainId === SEPOLIA && tx.to?.toLowerCase() === VAULT, `a wallet that switches sends to the Sepolia vault, Sepolia's chain id named: ${JSON.stringify({ chainId: tx.chainId, to: tx.to })}`);
  await p.context().close();
}
{
  const p = await evm(true, ['Wallet X', 'Wallet Y']);
  const r = await ethDeposit(p, 'Wallet Y');
  ok(r.sent.length === 1 && r.sent[0].name === "Wallet Y", `two Ethereum wallets: asked which pays, and that one sends: ${JSON.stringify(r.sent.map((x) => x.name))} ${r.text}`);
  await p.context().close();
}
await b.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exit(failed ? 1 : 0);
