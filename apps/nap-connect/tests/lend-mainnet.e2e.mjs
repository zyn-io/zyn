//   node tests/lend-mainnet.e2e.mjs        (APP=… to point elsewhere)
//
// Zyn Lend on mainnet (chain 26460), as a smoke test that sends nothing: the
// page opened with ?network=mainnet reads the live mainnet chain, says it is
// mainnet, lists only what the chain has listed, and its Bridge offers ZEC,
// the ETH pilot (its 0.01 ETH cap, payouts by the Safe's owners) and BOLD
// shut until the vault's Safe registers it — no faucet, no private channel,
// no receipts. A stand-in Ethereum wallet signs in and opens an account whose
// keyring lives on the testnet host; a stand-in Solana wallet is linked to it
// there and then opens the same account on testnet.
//
// No transaction reaches a chain: every zyn_submitSigned (an account's first
// registration) is answered here, and the stand-in wallets send nothing.
import { chromium } from 'playwright-core';
const APP = process.env.APP || 'https://lend.zyn-network.com';
const S = process.env.SHOTS || '/tmp';
const MAINNET = 'https://zyn-rpc.cavebro.win/';
const TESTNET = 'https://zyn-rpc.cavebro.win/object';
const VAULT = '0x235730d7738cc16d3b94d0e3d7097f9dde22a459';
const BOLD = '0x6440f144b7e50d6a8439336510312d2f54beb01d';
const rpc = async (url, method) => (await (await fetch(url, { method: 'POST', headers: { 'content-type': 'text/plain' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params: [] }) })).json()).result;
const status = await rpc(MAINNET, 'zyn_status');
const lending = await rpc(MAINNET, 'zyn_lendMarkets');
// What the vault holds under asset id 2 now: BOLD opens only once it is BOLD.
const reg = (await (await fetch('https://ethereum-rpc.publicnode.com', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_call', params: [{ to: VAULT, data: `0xf8a80dc8${'2'.padStart(64, '0')}` }, 'latest'] }) })).json()).result || '';
const boldRegistered = reg.length === 194 && `0x${reg.slice(26, 66)}` === BOLD && reg.endsWith('1');
console.log(`      (mainnet chain ${status.chain}, epoch ${status.epoch}; ${lending.pools.length} lending pools; the vault ${boldRegistered ? 'has' : 'has not'} registered BOLD)`);

let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) failed++; };
const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
await ctx.addInitScript(() => {
  const hash = async (bytes) => new Uint8Array(await crypto.subtle.digest('SHA-512', bytes));
  const salt = localStorage.getItem('test.salt') || String(Math.random());
  localStorage.setItem('test.salt', salt);
  window.__sends = [];
  window.__calls = [];
  const evm = {
    async request({ method, params }) {
      window.__calls.push(method);
      if (method === 'eth_requestAccounts') return ['0x3333333333333333333333333333333333333333'];
      if (method === 'eth_chainId') return '0x1';
      if (method === 'personal_sign') { const h = await hash(new TextEncoder().encode(params[0] + salt)); return '0x' + [...h.slice(0, 64), 27].map((x) => x.toString(16).padStart(2, '0')).join(''); }
      // Recorded, never sent: a stand-in hash comes back.
      if (method === 'eth_sendTransaction') { window.__sends.push(params[0]); return '0x' + 'cd'.repeat(32); }
      throw Object.assign(new Error('unsupported ' + method), { code: 4200 });
    },
  };
  window.addEventListener('eip6963:requestProvider', () => window.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: Object.freeze({ info: { uuid: 'test-evm', name: 'Test EVM', icon: '', rdns: 'test' }, provider: evm }) })));
  // A real ed25519 key per run (from the salt), signing in the Solana way.
  const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  const b58 = (bytes) => { let n = 0n; for (const x of bytes) n = n * 256n + BigInt(x); let o = ''; while (n > 0n) { o = B58[Number(n % 58n)] + o; n /= 58n; } for (const x of bytes) { if (x) break; o = '1' + o; } return o; };
  const key = (async () => {
    const seed = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(`sol${salt}`)));
    const der = Uint8Array.from([...'302e020100300506032b657004220420'.match(/../g).map((x) => parseInt(x, 16)), ...seed]);
    const k = await crypto.subtle.importKey('pkcs8', der, { name: 'Ed25519' }, true, ['sign']);
    const x = (await crypto.subtle.exportKey('jwk', k)).x;
    const pub = Uint8Array.from(atob(x.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
    return { k, account: { address: b58(pub), publicKey: pub, chains: ['solana:mainnet'], features: [] } };
  })();
  const sol = { name: 'Test Solana', icon: 'data:image/png;base64,iVBORw0KGgo=', version: '1.0.0', chains: ['solana:mainnet'], get accounts() { return []; }, features: {
    'standard:connect': { version: '1.0.0', connect: async () => ({ accounts: [(await key).account] }) },
    'solana:signMessage': { version: '1.0.0', signMessage: async ({ message }) => { const { k } = await key; return [{ signedMessage: message, signature: new Uint8Array(await crypto.subtle.sign({ name: 'Ed25519' }, k, message)) }]; } },
    // Wallets are listed by this feature; nothing here ever calls it.
    'solana:signAndSendTransaction': { version: '1.0.0', signAndSendTransaction: async () => { window.__sends.push('sol'); return [{ signature: new Uint8Array(64).fill(7) }]; } },
  } };
  window.addEventListener('wallet-standard:app-ready', (e) => e.detail.register(sol));
});
// Every call to Zyn, by host; a write is answered here and never sent.
const seen = [];
await ctx.route('https://zyn-rpc.cavebro.win/**', async (route) => {
  const req = route.request();
  let body = {};
  try { body = JSON.parse(req.postData() || '{}'); } catch { /* not JSON-RPC */ }
  const host = req.url().replace(/\/$/, '').endsWith('/object') ? 'testnet' : 'mainnet';
  seen.push({ host, method: body.method });
  if (body.method === 'zyn_submitSigned') return route.fulfill({ status: 200, contentType: 'application/json', headers: { 'access-control-allow-origin': '*' }, body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result: { receipts: '' } }) });
  return route.continue();
});
const page = await ctx.newPage();
const errs = []; page.on('pageerror', (e) => errs.push(e.message));
const signedIn = async () => { try { await page.waitForFunction(() => /^[0-9a-f]{5}…/.test(document.querySelector('#connect').textContent.trim()), null, { timeout: 90000 }); } catch { console.log('NOTE:', await page.textContent('#w-note'), errs); throw new Error('not signed in'); } };

// ------------------------------------------------------------ markets
await page.goto(`${APP}/?network=mainnet#/markets`);
await page.waitForFunction(() => /\d/.test(document.querySelector('#t-epoch')?.textContent || ''), null, { timeout: 30000 }).catch(() => {});
await page.waitForTimeout(800);
const epoch = Number((await page.textContent('#t-epoch')).replace(/\D/g, ''));
ok(Math.abs(epoch - status.epoch) <= 5, `reads the live mainnet chain: epoch #${epoch} (the node says ${status.epoch})`);
ok(seen.some((x) => x.host === 'mainnet' && x.method === 'zyn_lendMarkets') && !seen.some((x) => x.host === 'testnet' && x.method === 'zyn_lendMarkets'), 'lending markets are read from mainnet, not the testnet');
const pill = await page.$eval('#net', (el) => ({ text: el.textContent.trim(), main: el.classList.contains('main'), href: el.getAttribute('href'), shown: getComputedStyle(el).display !== 'none' }));
ok(pill.text === 'Mainnet' && pill.main && pill.shown && /network=testnet/.test(pill.href), `the header pill says Mainnet and switches to testnet: ${JSON.stringify(pill)}`);
ok(/Zyn mainnet/.test(await page.textContent('#foot-note')) && /real funds/.test(await page.textContent('#foot-note')), `the footer says mainnet: ${await page.textContent('#foot-note')}`);
ok(/Lend ZEC, ETH or BOLD/.test(await page.textContent('#lede-assets')), `mainnet lends ZEC, ETH and BOLD, no SOL: ${await page.textContent('#lede-assets')}`);
if (!lending.pools.length) {
  const empty = (await page.textContent('#cores')).replace(/\s+/g, ' ');
  ok(/Nothing listed yet/.test(empty) && /Lending opens as assets arrive/.test(empty) && (await page.$$('.core')).length === 0, `nothing listed yet: ${empty.slice(0, 80)}`);
} else {
  ok((await page.$$('.core')).length === lending.pools.length, `one card per pool the chain lists (${lending.pools.length})`);
}
ok(!/SOL/.test(await page.textContent('#check')), 'no SOL pairs among the price checks');
// The same view, on mobile width: the pill stays.
await page.setViewportSize({ width: 390, height: 800 });
ok(await page.isVisible('#net'), 'the Mainnet pill shows on a phone too');
await page.screenshot({ path: `${S}/lend-mainnet-phone.png` });
await page.setViewportSize({ width: 1280, height: 900 });

// ------------------------------------------------------------ lend / borrow
await page.goto(`${APP}/?network=mainnet#/lend/ETH`); await page.waitForTimeout(800);
const lendChips = await page.$$eval('#lend-asset input', (xs) => xs.map((x) => x.value));
ok(JSON.stringify(lendChips) === '["ZEC","ETH","BOLD"]', `lend offers ZEC, ETH, BOLD: ${lendChips}`);
ok(await page.isVisible('#lend-form [data-real]') && /Real funds/.test(await page.textContent('#lend-form [data-real]')), 'lending says real funds, inline');
if (!lending.pools.some((p) => p.asset === '9f5a2c088a0071ba7afcb2c74b9c7cc23f0f94c297d97aef68e50392c04acc6c')) {
  await page.fill('#lend-amount', '0.001'); await page.waitForTimeout(200);
  ok(await page.isDisabled('#lend-go') && /ETH\.zy pool is not open yet/.test(await page.textContent('#lend-note')), `an unlisted pool takes nothing: ${await page.textContent('#lend-note')}`);
}
await page.goto(`${APP}/?network=mainnet#/borrow`); await page.waitForTimeout(500);
ok(await page.isVisible('#borrow-form [data-real]'), 'borrowing says real funds, inline');

// ------------------------------------------------------------ bridge, signed out
await page.goto(`${APP}/?network=mainnet#/bridge`); await page.waitForTimeout(1500);
const brChips = await page.$$eval('#br-asset input', (xs) => xs.map((x) => x.value));
ok(JSON.stringify(brChips) === '["zec","eth","bold"]', `the mainnet bridge has ZEC, ETH and BOLD, no SOL or BTC: ${brChips}`);
ok(await page.isHidden('#br-channels'), 'no private channel on mainnet');
ok(await page.isVisible('#bridge-form [data-real]'), 'the bridge says real funds, inline');
await page.check('#br-asset input[value="eth"]'); await page.waitForTimeout(1500);
const pilotText = (await page.textContent('#br-pilot').catch(() => '')).replace(/\s+/g, ' ');
ok(/at most 0\.01 ETH in total/.test(pilotText) && /Withdrawals wait for the Safe's owners/.test(pilotText), `ETH shows the pilot cap and who pays out: ${pilotText.slice(0, 120)}`);
ok(/Ethereum/.test(await page.textContent('#br-read')) && /Pilot cap/.test(await page.textContent('#br-read')), `the readout names Ethereum and the cap: ${(await page.textContent('#br-read')).replace(/\s+/g, ' ')}`);
await page.check('#br-asset input[value="bold"]');
await page.waitForFunction(() => !/Checking/.test(document.querySelector('#br-wait')?.textContent || 'x'), null, { timeout: 20000 }).catch(() => {});
if (!boldRegistered) {
  ok(await page.isDisabled('#br-go') && /Safe has not registered BOLD/.test(await page.textContent('#br-wait')), `BOLD is shut until the Safe registers it: ${await page.textContent('#br-wait').catch(() => '')}`);
} else {
  console.log(`      (the vault has registered BOLD: the page says ${await page.textContent('#br-wait').catch(() => 'nothing')})`);
}
ok(!(await page.$('#br-faucet')), 'no BOLD faucet on mainnet');

// ------------------------------------------------------------ sign in
await page.goto(`${APP}/?network=mainnet#/portfolio`); await page.waitForTimeout(600);
await page.click('#pf-connect');
await page.waitForSelector('#wallet-list button:has-text("Test EVM")');
seen.length = 0;
await page.click('#wallet-list button:has-text("Test EVM")');
await signedIn();
const acct = (await page.textContent('#connect')).trim();
ok(/^[0-9a-f]{5}…/.test(acct), `a stand-in Ethereum wallet opened a mainnet account: ${acct}`);
const keyring = seen.filter((x) => /^zyn_keyring/.test(x.method || ''));
ok(keyring.length > 0 && keyring.every((x) => x.host === 'testnet'), `the keyring is asked on the testnet host only: ${keyring.map((x) => `${x.method}@${x.host}`).join(', ')}`);
ok(seen.some((x) => x.host === 'mainnet' && x.method === 'zyn_depositAddress'), 'the account itself is on mainnet');
await page.click('#connect');
ok(/on mainnet/.test(await page.textContent('#wallet-list')), `the wallet dialog names mainnet: ${(await page.textContent('#wallet-list')).replace(/\s+/g, ' ').slice(0, 70)}`);
await page.keyboard.press('Escape');
// Portfolio: Zcash mainnet addresses, the mainnet birthday, no private notes.
await page.waitForFunction(() => /^u1/.test(document.querySelector('#pf-id-zec')?.textContent || ''), null, { timeout: 90000 }).catch(() => {});
const ids = await page.evaluate(() => ({ u: document.querySelector('#pf-id-zec')?.textContent, t: document.querySelector('#pf-id-zect')?.textContent }));
ok(/^u1/.test(ids.u) && /^t1/.test(ids.t), `Zcash mainnet addresses: ${ids.u?.slice(0, 12)}…, ${ids.t}`);
await page.click('#pf-export-go'); await page.click('#pf-export-show');
const exp = (await page.textContent('#pf-export')).replace(/\s+/g, ' ');
ok(/Network\s*Zcash\b(?! testnet)/.test(exp) && /Birthday height\s*3,440,000/.test(exp), `export names Zcash mainnet and its birthday: ${exp.slice(exp.indexOf('Network'), exp.indexOf('Network') + 80)}`);
await page.click('#pf-export-hide');
ok(!/Private notes/.test(await page.textContent('#pf')), 'no private-notes card on mainnet');
ok(/Ethereum/.test(await page.textContent('.pf-wallets')) && !/Sepolia/.test(await page.textContent('.pf-wallets')), `the linked wallet is on Ethereum: ${(await page.textContent('.pf-wallets')).replace(/\s+/g, ' ').slice(0, 120)}`);

// ------------------------------------------------------------ bridge, signed in
await page.goto(`${APP}/?network=mainnet#/bridge`); await page.waitForTimeout(800);
await page.check('input[name="br-mode"][value="in"]'); await page.check('#br-asset input[value="zec"]');
await page.waitForFunction(() => /^u1/.test(document.querySelector('#br-addr')?.textContent || '') || /Unavailable/.test(document.querySelector('#br-addr')?.textContent || ''), null, { timeout: 60000 }).catch(() => {});
ok(/^u1/.test(await page.textContent('#br-addr')), `ZEC deposits to this account's mainnet deposit address: ${(await page.textContent('#br-addr')).slice(0, 16)}… ${await page.textContent('#br-note')}`);
await page.check('#br-asset input[value="eth"]'); await page.waitForTimeout(1200);
await page.fill('#br-amount', '0.02'); await page.click('#br-go');
await page.waitForFunction(() => /pilot/i.test(document.querySelector('#br-note').textContent), null, { timeout: 30000 }).catch(() => {});
ok(/pilot takes at most|pilot is full/i.test(await page.textContent('#br-note')) && !(await page.evaluate(() => window.__sends.length)), `over the cap is refused before the wallet is asked: ${await page.textContent('#br-note')}`);
await page.fill('#br-amount', '0.001'); await page.click('#br-go');
await page.waitForFunction(() => window.__sends.length > 0 || /error|refused|another|nothing/i.test(document.querySelector('#br-note').textContent), null, { timeout: 60000 }).catch(() => {});
const sent = await page.evaluate(() => window.__sends[0]);
ok(sent && sent.to.toLowerCase() === VAULT && /^0x52d29193[0-9a-f]{64}0{63}1$/.test(sent.data) && BigInt(sent.value) === 10n ** 15n, `the ETH deposit goes to the mainnet vault, asset 1 (stand-in wallet; nothing sent): ${JSON.stringify(sent)?.slice(0, 110)} ${await page.textContent('#br-note')}`);
await page.waitForTimeout(500);
ok(/etherscan\.io\/tx/.test(await page.innerHTML('#br-note')) && !/sepolia/.test(await page.innerHTML('#br-note')), 'the deposit links to Etherscan');
ok(!(await page.$('.br-receipt')), 'no receipt offered on mainnet');
await page.check('input[name="br-mode"][value="out"]'); await page.waitForTimeout(300);
ok((await page.inputValue('#br-to')) === '0x3333333333333333333333333333333333333333', 'an ETH withdrawal goes to the signed-in wallet by default');
if (!boldRegistered) {
  await page.check('input[name="br-mode"][value="in"]'); await page.check('#br-asset input[value="bold"]'); await page.waitForTimeout(1500);
  ok(await page.isDisabled('#br-go'), 'signed in, BOLD stays shut too');
}

// ------------------------------------------------------------ one account, both networks
// Link a stand-in Solana wallet here (the keyring is on the testnet host),
// then open the testnet page with it alone: the same account.
await page.goto(`${APP}/?network=mainnet#/portfolio`); await page.waitForTimeout(800);
seen.length = 0;
await page.click('[data-link="sol"]');
await page.waitForSelector('#wallet-list button:has-text("Test Solana")');
await page.click('#wallet-list button:has-text("Test Solana")');
await page.waitForFunction(() => /linked/.test(document.querySelector('#toast').textContent) || /keyring|refused|Error|another/i.test(document.querySelector('#w-note').textContent), null, { timeout: 90000 }).catch(() => {});
ok(/is linked/.test(await page.textContent('#toast')), `the Solana wallet is linked on mainnet: ${(await page.textContent('#toast')).trim()} ${(await page.textContent('#w-note')).trim()}`);
ok(seen.some((x) => x.method === 'zyn_keyringPut' && x.host === 'testnet') && !seen.some((x) => /^zyn_keyring/.test(x.method || '') && x.host === 'mainnet'), 'the link is kept on the testnet host');
await page.click('#connect'); await page.click('#w-out');
await page.goto(`${APP}/?network=testnet#/portfolio`); await page.waitForTimeout(800);
ok((await page.textContent('#net')).trim() === 'Testnet', 'the switch opens the testnet page');
await page.click('#pf-connect');
await page.waitForSelector('#wallet-list button:has-text("Test Solana")');
await page.click('#wallet-list button:has-text("Test Solana")');
await signedIn();
ok((await page.textContent('#connect')).trim() === acct, `the linked Solana wallet opens the same account on testnet (${acct})`);
// Clean up: unlink, so the stand-in's entry does not stay in the keyring.
await page.click('#connect');
await page.waitForSelector('[data-unlink="sol"]');
await page.click('[data-unlink="sol"]');
await page.waitForSelector('#approve[open]');
await page.click('#a-yes');
await page.waitForFunction(() => /unlinked/.test(document.querySelector('#toast').textContent) || /refused|Error/i.test(document.querySelector('#w-note').textContent), null, { timeout: 60000 }).catch(() => {});
ok(/is unlinked/.test(await page.textContent('#toast')), `unlinked again: ${(await page.textContent('#toast')).trim()}`);

const writes = seen.filter((x) => x.method === 'zyn_submitSigned');
console.log(`      (${writes.length} registration${writes.length === 1 ? '' : 's'} answered here, none sent)`);
ok(!errs.length, `no page errors ${errs.join(' | ')}`);
await b.close();
console.log(failed ? `${failed} failed` : 'all passed');
process.exit(failed ? 1 : 0);
