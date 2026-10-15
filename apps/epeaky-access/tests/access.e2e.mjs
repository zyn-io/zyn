// End to end, locally: the den's page opens the access window, a stand-in
// wallet buys (Jupiter is real, the chain is a stand-in RPC that reports the
// buy), the claim registers the pass, and the den's page is let in.
//
//   DEN=http://localhost:4300   a den with entry [:invite, :external], issuer
//                               http://localhost:8788/ and the issuer token
//   ACCESS=http://localhost:8788  `wrangler pages dev` with .dev.vars:
//                               DEN_URL, DEN_ISSUER_TOKEN, SOLANA_RPC=http://localhost:8899
//   PLAYWRIGHT_FROM=…           a folder with playwright-core
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { encode, moveMessage } from '../public/shared.mjs';

const require = createRequire(process.env.PLAYWRIGHT_FROM ? `${process.env.PLAYWRIGHT_FROM.replace(/\/$/, '')}/package.json` : import.meta.url);
const { chromium } = require('playwright-core');
const CHROME = process.env.CHROMIUM || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const DEN = process.env.DEN || 'http://localhost:4300';
const ACCESS = process.env.ACCESS || 'http://localhost:8788';
const MINT = '6dkcEZs83NpCJmWveasCMu3Kmd2XkCzSVmNJJhQBLtx1';
// DEN_ENV: the den's .exs, to count its members from here.
const members = () => Number(execFileSync('mix', ['run', '--no-start', '-e', `Code.eval_file(${JSON.stringify(process.env.DEN_ENV)}); Application.ensure_all_started(:ecto_sqlite3); {:ok, _} = Den.Repo.start_link(); IO.puts(Den.Repo.aggregate("members", :count))`], { cwd: new URL('../../cave-den/server/', import.meta.url).pathname, encoding: 'utf8' }).trim().split('\n').at(-1));

let failed = 0;
const ok = (c, what) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${what}`); if (!c) failed++; };

// The stand-in wallet's key, made here and handed to the page.
const key = await crypto.subtle.generateKey({ name: 'Ed25519' }, true, ['sign', 'verify']);
const WALLET = encode(new Uint8Array(await crypto.subtle.exportKey('raw', key.publicKey)));
const PKCS8 = Buffer.from(await crypto.subtle.exportKey('pkcs8', key.privateKey)).toString('base64');
const SIG = encode(crypto.getRandomValues(new Uint8Array(64)));

// The stand-in chain: the recorded buy, paid by the stand-in wallet, at once.
const buy = JSON.parse(readFileSync(new URL('./fixtures/buy.json', import.meta.url)));
const BUYER = buy.transaction.message.accountKeys[0].pubkey;
buy.transaction.message.accountKeys[0].pubkey = WALLET;
for (const b of [...buy.meta.preTokenBalances, ...buy.meta.postTokenBalances]) if (b.owner === BUYER) b.owner = WALLET;
buy.transaction.signatures = [SIG];
let asked = 0;
const rpc = createServer((req, res) => {
  let body = '';
  req.on('data', (c) => (body += c));
  req.on('end', () => {
    const { method, params, id } = JSON.parse(body);
    let result = null;
    if (method === 'getAccountInfo') result = { value: { data: { parsed: { info: { extensions: [{ extension: 'transferFeeConfig', state: { newerTransferFee: { transferFeeBasisPoints: 300 } } }] } } } } };
    if (method === 'getTransaction' && params[0] === SIG) {
      asked++;
      // Not on chain at the first ask, as right after sending.
      if (asked > 1) result = { ...buy, blockTime: Math.floor(Date.now() / 1000) };
    }
    res.setHeader('content-type', 'application/json');
    res.end(JSON.stringify({ jsonrpc: '2.0', id, result }));
  });
}).listen(8899);

const b = await chromium.launch({ executablePath: CHROME, headless: true });
const ctx = await b.newContext();
const INIT = ({ pkcs8, wallet, sig }) => {
  if (!location.port || location.port !== '8788') return;
  const raw = Uint8Array.from(atob(pkcs8), (c) => c.charCodeAt(0));
  const ABC = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  const dec = (s) => { let n = 0n; for (const c of s) n = n * 58n + BigInt(ABC.indexOf(c)); const o = []; while (n > 0n) { o.unshift(Number(n & 255n)); n >>= 8n; } for (const c of s) { if (c !== '1') break; o.unshift(0); } return Uint8Array.from(o); };
  const account = { address: wallet, publicKey: dec(wallet), chains: ['solana:mainnet'], features: [] };
  window.__sent = [];
  const w = {
    version: '1.0.0', name: 'Test wallet', icon: 'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg"/>', chains: ['solana:mainnet'], accounts: [account],
    features: {
      'standard:connect': { version: '1.0.0', connect: async () => ({ accounts: [account] }) },
      'solana:signAndSendTransaction': { version: '1.0.0', supportedTransactionVersions: ['legacy', 0], signAndSendTransaction: async ({ transaction }) => { window.__sent.push(transaction.length); return [{ signature: dec(sig) }]; } },
      'solana:signMessage': { version: '1.0.0', signMessage: async ({ message }) => {
        const k = await crypto.subtle.importKey('pkcs8', raw, { name: 'Ed25519' }, false, ['sign']);
        return [{ signedMessage: message, signature: new Uint8Array(await crypto.subtle.sign('Ed25519', k, message)) }];
      } },
    },
  };
  const register = ({ register }) => register(w);
  window.addEventListener('wallet-standard:app-ready', (e) => register(e.detail));
};
await ctx.addInitScript(INIT, { pkcs8: PKCS8, wallet: WALLET, sig: SIG });

// Opened on its own, the access page offers nothing to buy.
const alone = await ctx.newPage();
await alone.goto(`${ACCESS}/`);
await alone.waitForFunction(() => document.getElementById('say').textContent.length > 0);
ok((await alone.textContent('#say')).includes("den's page") && !(await alone.isVisible('#buy')), 'opened without a pass, it says to open it from the den and shows no amounts');
await alone.close();

const den = await ctx.newPage();
await den.goto(`${DEN}/`);
await den.waitForSelector('#enter:not([hidden])', { timeout: 20000 });
ok((await den.textContent('#enter')).trim() === 'Get access with $EPEAKY', 'the den\'s door shows the access button with its label');

const [pop] = await Promise.all([den.waitForEvent('popup'), den.click('#enter')]);
await pop.waitForLoadState();
ok(!pop.url().includes('den-pass'), 'the pass hash is taken off the access page\'s address');
await pop.waitForFunction(() => document.querySelectorAll('.amount').length === 3);
ok(JSON.stringify(await pop.$$eval('.amount strong', (s) => s.map((x) => x.textContent))) === JSON.stringify(['Buy with 0.1 SOL', 'Buy with 0.25 SOL', 'Buy with 0.5 SOL']), 'three amounts: 0.1, 0.25, 0.5 SOL');
ok(await pop.$eval('.amount', (x) => x.disabled), 'no buy before a wallet is connected');
ok((await pop.textContent('#fee-note')).includes('3% fee'), 'the transfer fee is read from the mint');
await pop.waitForFunction(() => /\$EPEAKY · price impact/.test(document.querySelector('.amount span').textContent), null, { timeout: 15000 }).catch(() => {});
ok(/About .+ \$EPEAKY · price impact/.test(await pop.textContent('.amount span')), `a live quote from Jupiter (${await pop.textContent('.amount span')})`);

await pop.reload();
await pop.waitForFunction(() => document.querySelectorAll('.amount').length === 3);
ok(true, 'reloaded, the window keeps the pass and its amounts');
await pop.waitForFunction(() => /\$EPEAKY · price impact/.test(document.querySelector('.amount span').textContent), null, { timeout: 15000 }).catch(() => {});

await pop.click('#connect');
await pop.waitForSelector('#account:not([hidden])');
ok((await pop.textContent('#account')).includes(WALLET.slice(0, 4)), 'the connected wallet is shown');

await pop.click('.amount[data-sol="0.1"]');
const inside = await den.waitForSelector('#den:not([hidden])', { timeout: 60000 }).then(() => true, () => false);
ok(inside, 'after the buy and the signed message, the den lets the device in');
ok(asked >= 2, `the claim waited for the buy to land (${asked} asks)`);

// The same buy claimed for another device is refused.
const again = await fetch(`${ACCESS}/api/claim`, {
  method: 'POST', headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ pass: 'cd'.repeat(32), signature: SIG, wallet: WALLET, signed: encode(new Uint8Array(await crypto.subtle.sign('Ed25519', key.privateKey, new TextEncoder().encode(`Epeaky access\npass: ${'cd'.repeat(32)}\ntx: ${SIG}`)))) }),
}).then((r) => r.json());
ok(again.error === 'used', 'the same buy cannot let in a second device');

// A new device: the wallet that bought moves its access here, with no new buy.
const before = members();
const ctx2 = await b.newContext();
await ctx2.addInitScript(INIT, { pkcs8: PKCS8, wallet: WALLET, sig: SIG });
const den2 = await ctx2.newPage();
await den2.goto(`${DEN}/`);
await den2.waitForSelector('#enter:not([hidden])', { timeout: 20000 });
const [pop2] = await Promise.all([den2.waitForEvent('popup'), den2.click('#enter')]);
await pop2.waitForFunction(() => document.querySelectorAll('.amount').length === 3);
ok(await pop2.isHidden('#moving'), 'the move is offered only once a wallet is connected');
await pop2.click('#connect');
await pop2.waitForSelector('#moving:not([hidden])');
await pop2.click('#move');
ok(await den2.waitForSelector('#den:not([hidden])', { timeout: 30000 }).then(() => true, () => false), 'the new device is let in by moving the access, with no new buy');
ok(members() === before, `one pass in, one out: the den has as many members as before (${before})`);

const at = Date.now();
const p3 = '12'.repeat(32);
const soon = await fetch(`${ACCESS}/api/move`, {
  method: 'POST', headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ pass: p3, wallet: WALLET, at, signed: encode(new Uint8Array(await crypto.subtle.sign('Ed25519', key.privateKey, new TextEncoder().encode(moveMessage(p3, WALLET, at))))) }),
}).then((r) => r.json());
ok(soon.error === 'soon' && soon.next > Date.now(), 'a second move the same day waits');

await b.close();
rpc.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exit(failed ? 1 : 0);
