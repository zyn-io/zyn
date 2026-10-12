//   node tests/lend-wallets.e2e.mjs        (APP=… to point elsewhere)
//
// Zyn Lend with wallets a holder already has: a stand-in Solana wallet (Wallet
// Standard) and a stand-in Ethereum wallet (EIP-6963), both signing
// deterministically, open a Zyn account in the page. The Ethereum one signs
// the app's EIP-4361 sign-in and writes v as 27/28 at first and as 0/1
// later — the same account either way. The Solana one signs plain words (a
// Solana sign-in's fixed date goes stale), and an account it opened under the
// old sign-in (2–12 Oct 2026) moves over with one more signature.
import { chromium } from 'playwright-core';
const APP = process.env.APP || 'https://lend.zyn-network.com';
const S = process.env.SHOTS || '/tmp';
// What the chain has of BOLD.zy (§68) right now: the page must draw either way.
const BOLD_ID = '8c1a1b0616cef63250e9f5bab8b20908320cc47ead0f60215a229d34b5c93ac2';
const rpc = async (method) => (await (await fetch(process.env.RPC || 'https://zyn-rpc.cavebro.win/object', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params: [] }) })).json()).result;
const boldPool = ((await rpc('zyn_lendMarkets').catch(() => ({})))?.pools || []).some((p) => p.asset === BOLD_ID);
// BOLD_LISTED=1 has the page see BOLD.zy listed before the bridge does, to try the deposit steps.
const boldListed = !!process.env.BOLD_LISTED || ((await rpc('zyn_bridged').catch(() => [])) || []).some((r) => r.asset === BOLD_ID);
console.log(`      (the chain ${boldPool ? 'has' : 'has no'} BOLD pool; the bridge ${boldListed ? 'lists' : 'does not list'} BOLD.zy)`);
let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) failed++; };
const b = await chromium.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
await ctx.addInitScript(() => {
  const hash = async (bytes) => new Uint8Array(await crypto.subtle.digest('SHA-512', bytes));
  window.__calls = [];
  window.__msgs = [];
  window.__vlow = false;
  // Ethereum, announced the EIP-6963 way.
  const evm = {
    async request({ method, params }) {
      window.__calls.push(method);
      if (method === 'eth_requestAccounts') return ['0x1111111111111111111111111111111111111111'];
      if (method === 'eth_chainId') return '0xaa36a7';
      if (method === 'personal_sign') {
        const text = new TextDecoder().decode(Uint8Array.from(params[0].slice(2).match(/../g), (x) => parseInt(x, 16)));
        window.__msgs.push(text);
        // r‖s from the message, v from its low bit — written 27/28, or 0/1.
        const h = await hash(new TextEncoder().encode(text));
        const v = (h[0] & 1) + (window.__vlow ? 0 : 27);
        return '0x' + [...h.slice(0, 64), v].map((x) => x.toString(16).padStart(2, '0')).join('');
      }
      if (method === 'eth_sendTransaction') { window.__sent = params[0]; (window.__sends ||= []).push(params[0]); return '0x' + 'ab'.repeat(32); }
      // ERC-20 reads (BOLD): plenty held, nothing approved yet; every transaction mined.
      if (method === 'eth_call') return params[0].data.startsWith('0x70a08231') ? '0x' + (10n ** 24n).toString(16) : '0x0';
      if (method === 'eth_getTransactionReceipt') return { status: '0x1' };
      throw Object.assign(new Error('unsupported ' + method), { code: 4200 });
    },
  };
  window.addEventListener('eip6963:requestProvider', () => window.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: Object.freeze({ info: { uuid: 'test-evm', name: 'Test EVM', icon: '', rdns: 'test' }, provider: evm }) })));
  // Solana, registered the Wallet Standard way.
  const account = { address: 'HBvV7YqSRSPW4YEBsDvpvF2PrUWFubqVbTNYafkddTsy', publicKey: new Uint8Array(32), chains: ['solana:devnet'], features: [] };
  const sol = { name: 'Test Solana', icon: 'data:image/png;base64,iVBORw0KGgo=', version: '1.0.0', chains: ['solana:devnet'], accounts: [account], features: {
    'standard:connect': { version: '1.0.0', connect: async () => ({ accounts: [account] }) },
    'solana:signMessage': { version: '1.0.0', signMessage: async ({ message }) => { window.__msgs.push(new TextDecoder().decode(message)); return [{ signedMessage: message, signature: (await hash(message)).slice(0, 64) }]; } },
    'solana:signAndSendTransaction': { version: '1.0.0', signAndSendTransaction: async () => [{ signature: new Uint8Array(64).fill(7) }] },
  } };
  window.addEventListener('wallet-standard:app-ready', (e) => e.detail.register(sol));
});
if (process.env.BOLD_LISTED) {
  await ctx.route('https://zyn-rpc.cavebro.win/object', async (route) => {
    const body = JSON.parse(route.request().postData() || '{}');
    const r = await fetch(route.request().url(), { method: 'POST', headers: { 'content-type': 'application/json' }, body: route.request().postData() });
    const out = await r.json();
    if (body.method === 'zyn_bridged' && Array.isArray(out.result) && !out.result.some((x) => x.asset === BOLD_ID)) out.result.push({ asset: BOLD_ID, network: 'eip155:11155111', origin: 'd58c9e3eebe83fda55d59c186e86b37aa5d0f79a', decimals: 18, issued: '0' });
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(out) });
  });
}
const page = await ctx.newPage();
const errs = []; page.on('pageerror', (e) => errs.push(e.message));
await page.goto(`${APP}/#/markets`);
await page.waitForTimeout(1500);
// BOLD.zy is the fourth market, with its price risk said on the card. Until
// the chain opens a BOLD pool the card says so instead of showing figures.
await page.waitForFunction(() => document.querySelectorAll('.core').length === 4 && /\d/.test(document.querySelector('#t-epoch')?.textContent || ''), null, { timeout: 30000 }).catch(() => {});
await page.waitForTimeout(1500);
const boldCard = (await page.textContent('.core[aria-label="BOLD.zy market"]').catch(() => '')).replace(/\s+/g, ' ');
ok(/Liquity BOLD, bridged to Zyn/.test(boldCard) && /BOLD can trade away from \$1; it is priced at its market value\./.test(boldCard), `BOLD.zy is a market: ${boldCard.slice(0, 90)}`);
ok(/borrow up to 75%, liquidated at 78%/.test(boldCard) || /borrow up to \d+%/.test(boldCard), `BOLD's risk is shown: ${(boldCard.match(/As collateral[^.]*/) || [''])[0]}`);
ok(/\$\d\.\d{4}|—/.test(boldCard.slice(0, 40)), `BOLD is priced at its traded value, to four places: ${boldCard.slice(0, 30)}`);
ok(boldPool !== /Pool not open yet/.test(boldCard), `the BOLD card ${boldPool ? 'shows its pool' : 'says the pool is not open yet'}`);
ok(/ETH\/BOLD/.test(await page.textContent('#check')) && /ZEC\/BOLD/.test(await page.textContent('#check')), 'the price checks cover the BOLD pairs');
ok((await page.textContent('#connect')).trim() === 'Connect wallet', `the header offers any wallet: ${(await page.textContent('#connect')).trim()}`);
// A visitor following a deposit link is told whose account it credits.
const CODE = 'c0de'.repeat(16);
await page.goto(`${APP}/#/bridge/deposit/eth?code=${CODE}`);
await page.waitForTimeout(600);
ok(/credits the account that made this link \(code c0dec0…c0de\)/.test(await page.textContent('#br-body')), 'a deposit link says it credits the account that made it');
await page.goto(`${APP}/#/markets`);
await page.waitForTimeout(400);
await page.click('#connect');
await page.waitForSelector('#wallet-list button', { timeout: 15000 });
ok(await page.isVisible('#w-key') && /only sign it on lend\.zyn-network\.com/.test(await page.textContent('#w-key')), 'the connect dialog says the signature is the account key');
await page.waitForTimeout(1200);
const list = (await page.textContent('#wallet-list')).replace(/\s+/g, ' ');
ok(/Solana.*Test Solana/.test(list) && /Ethereum.*Test EVM/.test(list) && /Nap/.test(list), `wallets grouped by chain, Nap kept: ${list.slice(0, 160)}`);
await page.click('#wallet-list button:has-text("Test EVM")');
await page.waitForFunction(() => /^[0-9a-f]{5}…/.test(document.querySelector('#connect').textContent.trim()) || /declined|Error|cannot/i.test(document.querySelector('#w-note').textContent), null, { timeout: 60000 }).catch(() => {});
const head = (await page.textContent('#connect')).trim();
ok(/^[0-9a-f]{5}…/.test(head), `an Ethereum wallet opened a Zyn account: ${head} ${(await page.textContent('#w-note')).trim()}`);
ok((await page.evaluate(() => window.__calls.filter((c) => c === 'personal_sign').length)) === 2, 'signed twice the first time on this device (stable check)');
const siwe = await page.evaluate(() => window.__msgs[0]);
ok(/^lend\.zyn-network\.com wants you to sign in with your Ethereum account:\n0x1111111111111111111111111111111111111111\n\n.+\n\nURI: https:\/\/lend\.zyn-network\.com\nVersion: 1\nChain ID: 1\nNonce: zynaccount1\nIssued At: 2026-10-02T00:00:00Z$/.test(siwe), `the Ethereum wallet signs an EIP-4361 sign-in: ${JSON.stringify(siwe).slice(0, 90)}`);
ok(await page.evaluate(() => window.__msgs[0] === window.__msgs[1]), 'the sign-in is the same bytes each time');
const acctEvm = head;
// The same wallet again, now writing v as 0/1: the same account, one signature.
await page.click('#connect'); await page.click('#w-out');
await page.evaluate(() => { window.__calls = []; window.__vlow = true; });
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
// Amounts are read as Nap reads them.
await page.goto(`${APP}/#/lend/ETH`); await page.waitForTimeout(800);
for (const bad of ['.5', '1e3', '0x10']) {
  await page.fill('#lend-amount', bad);
  ok(await page.isDisabled('#lend-go') && /decimal/.test(await page.textContent('#lend-note')), `"${bad}" is not an amount: ${await page.textContent('#lend-note')}`);
}
// BOLD in Lend: the price risk shows, and without a pool the form says it is not open.
await page.goto(`${APP}/#/lend/BOLD`); await page.waitForTimeout(600);
await page.fill('#lend-amount', '1'); await page.waitForTimeout(200);
ok(await page.isVisible('#lend-stable') && (boldPool || (await page.isDisabled('#lend-go') && /BOLD\.zy pool is not open yet/.test(await page.textContent('#lend-note')))), `lending BOLD: ${await page.textContent('#lend-note')}`);
await page.goto(`${APP}/#/borrow/ETH/BOLD`); await page.waitForTimeout(600);
ok(await page.isVisible('#borrow-stable') && (await page.inputValue('#borrow-asset input:checked')) === 'BOLD', 'BOLD can be borrowed against ETH, with its price risk shown');
await page.goto(`${APP}/#/lend/ETH`); await page.waitForTimeout(600);
// A write goes through this page's own approval.
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
// A deposit link opened while connected is ignored: the deposit carries this
// account's own code.
await page.goto(`${APP}/#/bridge/deposit/eth?code=${CODE}`); await page.waitForTimeout(800);
ok(!/made this link/.test(await page.textContent('#br-body')), 'a connected holder is not offered the link\'s code');
await page.check('input[name="br-mode"][value="in"]'); await page.waitForTimeout(300);
await page.fill('#br-amount', '0.001'); await page.click('#br-go');
await page.waitForFunction(() => !!window.__sent || /error|refused/i.test(document.querySelector('#br-note').textContent), null, { timeout: 30000 }).catch(() => {});
const sent = await page.evaluate(() => window.__sent);
ok(sent && sent.from === '0x1111111111111111111111111111111111111111' && /^0x52d29193/.test(sent.data) && !sent.data.includes(CODE), `the ETH deposit is sent by the connected wallet with this account's code, not the link's: ${JSON.stringify(sent)?.slice(0, 90)}`);
// A deposit offers a receipt, and asking for one gets an answer from the
// receipt service (here a refusal: the stand-in wallet's deposit is not real).
ok(await page.isVisible('.br-receipt .rc-go'), 'a deposit offers a receipt');
await page.click('.br-receipt .rc-go');
await page.waitForFunction(() => (document.querySelector('.br-receipt .rc-say')?.textContent || '').trim().length > 0, null, { timeout: 60000 }).catch(() => {});
const said = (await page.textContent('.br-receipt .rc-say')).trim();
ok(/not succeeded|not final|not a deposit|minted/i.test(said), `the receipt service answers: ${said.slice(0, 100)}`);
await page.screenshot({ path: `${S}/lend-evm.png` });
// BOLD in and out through the same vault: a test faucet, then approve and
// deposit from the connected wallet, two steps; out to that wallet.
await page.goto(`${APP}/#/bridge`); await page.waitForTimeout(800);
await page.check('input[name="br-mode"][value="in"]'); await page.check('#br-asset input[value="bold"]'); await page.waitForTimeout(1500);
ok((await page.textContent('#br-go')).trim() === 'Approve and deposit' && /BOLD can trade away from \$1/.test(await page.textContent('#br-body')), `BOLD deposits in two steps: ${(await page.textContent('#br-go')).trim()}`);
await page.evaluate(() => { window.__sends = []; });
await page.click('#br-faucet');
await page.waitForFunction(() => (window.__sends || []).length > 0, null, { timeout: 15000 }).catch(() => {});
const mint = await page.evaluate(() => window.__sends[0]);
ok(mint && mint.to.toLowerCase() === '0xd58c9e3eebe83fda55d59c186e86b37aa5d0f79a' && mint.data === `0x40c10f19${'1111111111111111111111111111111111111111'.padStart(64, '0')}${(1000n * 10n ** 18n).toString(16).padStart(64, '0')}`, `Get test BOLD mints 1,000 to the connected wallet: ${JSON.stringify(mint)?.slice(0, 80)}`);
await page.evaluate(() => { window.__sends = []; });
if (!boldListed) {
  ok(await page.isDisabled('#br-go') && /not list BOLD\.zy/.test(await page.textContent('#br-body')), `before the bridge lists BOLD.zy, deposits wait: ${await page.textContent('#br-wait')}`);
} else {
  await page.fill('#br-amount', '2'); await page.click('#br-go');
  await page.waitForFunction(() => (window.__sends || []).length >= 2 || /error|refused|Update Nap|another/i.test(document.querySelector('#br-note').textContent), null, { timeout: 30000 }).catch(() => {});
  const [ap, dep] = await page.evaluate(() => window.__sends);
  const two = (2n * 10n ** 18n).toString(16).padStart(64, '0');
  ok(ap && ap.to.toLowerCase() === '0xd58c9e3eebe83fda55d59c186e86b37aa5d0f79a' && ap.data === `0x095ea7b3${'e17bb5943b34133f6a8a5d221be4ed9493b3ef27'.padStart(64, '0')}${two}`, `step 1 approves the vault for 2 BOLD: ${ap?.data?.slice(0, 20)}`);
  ok(dep && dep.to.toLowerCase() === '0xe17bb5943b34133f6a8a5d221be4ed9493b3ef27' && /^0xdedd8ed5[0-9a-f]{64}0{63}2/.test(dep.data) && dep.data.endsWith(two) && !dep.data.includes(CODE), `step 2 deposits under this account's code, asset 2: ${dep?.data?.slice(0, 20)} — ${await page.textContent('#br-note')}`);
}
await page.check('input[name="br-mode"][value="out"]'); await page.check('#br-asset input[value="bold"]'); await page.waitForTimeout(300);
ok((await page.inputValue('#br-to')) === '0x1111111111111111111111111111111111111111' && /BOLD\.zy/.test(await page.textContent('#br-body')), 'a BOLD.zy withdrawal goes to the connected wallet by default');
await page.check('input[name="br-channel"][value="private"]'); await page.waitForTimeout(300);
ok(await page.isHidden('#br-asset input[value="bold"] >> xpath=..'), 'BOLD has no private channel');
await page.check('input[name="br-channel"][value="transparent"]');
// Solana opens its own account.
await page.click('#connect'); await page.click('#w-out');
await page.click('#connect');
await page.waitForSelector('#wallet-list button:has-text("Test Solana")');
await page.click('#wallet-list button:has-text("Test Solana")');
await page.waitForFunction(() => /^[0-9a-f]{5}…/.test(document.querySelector('#connect').textContent.trim()), null, { timeout: 60000 }).catch(() => {});
const sh = (await page.textContent('#connect')).trim();
ok(/^[0-9a-f]{5}…/.test(sh) && sh !== acctEvm, `a Solana wallet opens its own account: ${sh}`);
const solText = await page.evaluate(() => window.__msgs.at(-1));
ok(solText === 'Open my Zyn account\n\nThis signature is the key to the account. Sign it only on lend.zyn-network.com.\n\nWallet: HBvV7YqSRSPW4YEBsDvpvF2PrUWFubqVbTNYafkddTsy\nVersion: 2', `the Solana wallet signs plain words, no sign-in date to go stale: ${JSON.stringify(solText).slice(0, 90)}`);
// Its account from before 12 Oct: one more signature of the old sign-in, then
// the new one opens that account.
await page.click('#connect'); await page.click('#w-out');
await page.click('#connect');
await page.waitForSelector('#wallet-list button:has-text("Test Solana: account from before 12 Oct")');
await page.click('#wallet-list button:has-text("Test Solana: account from before 12 Oct")');
await page.waitForSelector('#approve[open]', { timeout: 30000 });
ok(/before 12 Oct/.test(await page.textContent('#a-title')), 'moving an older account asks first, and says the wallet will warn about the date');
await page.click('#a-yes');
await page.waitForFunction((was) => { const t = document.querySelector('#connect').textContent.trim(); return /^[0-9a-f]{5}…/.test(t) && t !== was; }, sh, { timeout: 60000 }).catch(() => {});
const old = (await page.textContent('#connect')).trim();
const oldText = await page.evaluate(() => window.__msgs.at(-1));
ok(/^[0-9a-f]{5}…/.test(old) && old !== sh && /^lend\.zyn-network\.com wants you to sign in with your Solana account:\nHBvV7YqSRSPW4YEBsDvpvF2PrUWFubqVbTNYafkddTsy\n[\s\S]*Nonce: zynaccount1\nIssued At: 2026-10-02T00:00:00Z$/.test(oldText), `the old sign-in, signed once more, opens the older account: ${old}`);
await page.click('#connect'); await page.click('#w-out');
await page.click('#connect');
await page.waitForSelector('#wallet-list button:has-text("Test Solana")');
await page.click('#wallet-list button >> text="Test Solana"');
await page.waitForFunction(() => /^[0-9a-f]{5}…/.test(document.querySelector('#connect').textContent.trim()), null, { timeout: 60000 }).catch(() => {});
ok((await page.textContent('#connect')).trim() === old, 'from then on the new sign-in opens the older account');
// Unlinked again, so the next run starts where this one did.
await page.click('#connect'); await page.click('[data-unlink="sol"]'); await page.click('#a-yes');
await page.waitForTimeout(1500);
await page.click('#connect').catch(() => {}); await page.click('#w-out').catch(() => {});
// Under Private, BTC is not offered while Mezo's channel is closed.
await page.goto(`${APP}/#/bridge`); await page.waitForTimeout(600);
await page.check('#br-asset input[value="btc"]');
await page.check('input[name="br-channel"][value="private"]'); await page.waitForTimeout(300);
ok(await page.isHidden('#br-asset input[value="btc"] >> xpath=..') && (await page.inputValue('#br-asset input:checked')) === 'sol', 'private BTC is not offered');
ok(!errs.length, `no page errors ${errs.join(' | ')}`);
await b.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exit(failed ? 1 : 0);
