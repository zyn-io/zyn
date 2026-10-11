// cavebro.win end to end, as a first-time visitor finds it (no sale needed):
// the landing page, then the store with a Nap that does not exist yet —
// made through the connect window's own setup path — on testnet and
// mainnet, every page while connected, refusing, closing Nap's window,
// reconnecting with the password, and the same through the Nap extension.
// Nothing is bought or signed; the wallets are fresh and hold nothing.
//
//   node tests/cave.e2e.mjs            (SITE=https://cavebro.win NAP=https://nap.zyn-network.com)
//   EXT=0 node tests/cave.e2e.mjs      skip the extension part
//
// Uses Playwright's Chrome for Testing (branded Chrome no longer loads an
// unpacked extension); build apps/extension first for the extension part.
import { chromium } from 'playwright-core';
import { homedir } from 'node:os';
import { mkdirSync } from 'node:fs';

const SITE = process.env.SITE || 'https://cavebro.win';
const NAP = new URL(process.env.NAP || 'https://nap.zyn-network.com').origin;
const CHROMIUM = process.env.CHROMIUM || `${homedir()}/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing`;
const EXT = new URL('../../extension/dist', import.meta.url).pathname;
const S = process.env.SHOTS || '/tmp/cave-e2e';
mkdirSync(S, { recursive: true });
const PASSWORD = 'cave-e2e-password';
let failed = 0;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) { failed++; process.exitCode = 1; } };
const errors = [];
const watch = (page, name) => {
  page.on('pageerror', (e) => errors.push(`${name}: ${e.message}`));
  page.on('console', (m) => { if (m.type() === 'error' && !/frame-ancestors|Failed to load resource/.test(m.text())) errors.push(`${name} console: ${m.text().slice(0, 160)}`); });
};

// ---------------------------------------------------------------------------
// The landing page.

async function landing(browser) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  watch(page, 'landing');
  await page.goto(SITE, { waitUntil: 'networkidle' });
  const srcs = await page.$$eval('.holo-card .card-face > img', (els) => els.map((e) => ({ src: e.getAttribute('src'), w: e.naturalWidth })));
  ok(srcs.length === 3 && srcs.every((s) => /-original\.png$/.test(s.src) && s.w > 0), `the three cards show the revealed art (${srcs.map((s) => s.src.split('/').pop()).join(', ')})`);
  ok(!(await page.$('.edge-mask')), 'no silhouette mask over the art');
  const text = (await page.textContent('body')).toLowerCase();
  ok(!/shadow|silhouette|unrevealed|undisclosed/.test(text), 'no copy still talks about shadows or unrevealed art');
  await page.screenshot({ path: `${S}/landing.png` });
  await page.close();
}

// ---------------------------------------------------------------------------
// Connecting through Nap in the browser.

/// Open the store's picker and choose `name`; returns Nap's window.
async function choose(ctx, store, name) {
  await store.click('[data-wallet-button]');
  await store.waitForSelector('.zwallet-option', { timeout: 20000 });
  const names = await store.$$eval('.zwallet-option', (els) => els.map((e) => e.textContent.trim()));
  ok(names.includes(name), `the picker offers ${name} (${names.join(', ')})`);
  const [win] = await Promise.all([ctx.waitForEvent('page'), store.click(`.zwallet-option:has-text("${name}")`)]);
  watch(win, `${name} window`);
  await win.waitForLoadState();
  return win;
}

/// First visit: Nap's window says there is no wallet, sets one up in a tab,
/// then asks for that password by itself.
async function firstConnect(ctx, store, net) {
  const win = await choose(ctx, store, 'Nap in the browser');
  await win.waitForSelector('#setup-create', { timeout: 20000 });
  const main = await win.textContent('main');
  ok(/no Nap wallet yet/i.test(main) && !(await win.$('#words')), `${net}: a new visitor is offered setup, not a phrase box`);
  ok(new URL(win.url()).origin === NAP, `${net}: Nap's window is on ${new URL(win.url()).origin}`);
  ok((await win.textContent('.site')) === new URL(SITE).host, `${net}: it names the asking site (${await win.textContent('.site')})`);
  await win.screenshot({ path: `${S}/${net}-connect-new.png` });
  const [tab] = await Promise.all([ctx.waitForEvent('page'), win.click('#setup-create')]);
  watch(tab, 'setup tab');
  await tab.waitForSelector('#create-form', { timeout: 20000 });
  ok(await tab.$eval('#keep-password', (e) => e.required), `${net}: the setup tab requires a password`);
  await tab.check('#ack');
  await tab.fill('#keep-password', PASSWORD);
  await tab.click('#create-form .primary');
  await tab.waitForSelector('.wallet', { timeout: 60000 });
  ok(/Go back to the Nap window/.test(await tab.textContent('#notice')), `${net}: the tab says to go back to Nap's window`);
  await tab.close();
  await win.bringToFront();
  await win.waitForSelector('#pw', { timeout: 15000 });
  ok(true, `${net}: Nap's window switched to the password by itself`);
  ok(!!(await win.$('#new-wallet')), `${net}: the password screen offers a new wallet too`);
  await unlockAndApprove(win, store, net);
  return win;
}

async function unlockAndApprove(win, store, net) {
  await win.fill('#pw', PASSWORD);
  await win.click('#unlock button[type=submit]');
  // A site approved before on this network connects without asking again.
  const connected = () => store.waitForFunction(() => /Nap ·/.test(document.querySelector('[data-wallet-button]')?.textContent || ''), null, { timeout: 60000 }).then(() => 'connected');
  const first = await Promise.race([win.waitForSelector('#yes', { timeout: 60000 }).then(() => 'card'), connected()]);
  if (first === 'card') {
    const card = await win.textContent('main');
    ok(/wants to connect/.test(card) && new RegExp(`Zyn ${net}`).test(card), `${net}: the connect card names the site and Zyn ${net}`);
    await win.click('#yes');
  } else ok(true, `${net}: a site approved before connects without a second card`);
  await store.waitForFunction(() => /Nap ·/.test(document.querySelector('[data-wallet-button]')?.textContent || ''), null, { timeout: 30000 });
  ok(true, `${net}: the store shows it is connected`);
}

/// Around the store by its own links, the way people move: the connection
/// holds, each page keeps its own layout, nothing reads "undefined", and
/// bad input is refused on the page.
async function tour(store, net) {
  const layout = {
    '/market': () => getComputedStyle(document.querySelector('.book')).overflowX === 'auto',
    '/mint': () => getComputedStyle(document.querySelector('.total')).paddingTop === '15px',
    '/items': () => getComputedStyle(document.querySelector('.shelf')).display === 'grid',
  };
  for (const path of ['/market', '/mint', '/items']) {
    const before = errors.length;
    await store.click(`.topbar-nav a[href="${path}"]`);
    await store.waitForFunction((p) => location.pathname.replace(/\/$/, '') === p, path, { timeout: 15000 });
    await store.waitForTimeout(3500);
    const text = (await store.textContent('main').catch(() => '')) || '';
    await store.screenshot({ path: `${S}/${net}${path.replace('/', '-')}.png`, fullPage: true });
    ok(text.trim().length > 40 && errors.length === before, `${net} ${path}: renders (${text.trim().replace(/\s+/g, ' ').slice(0, 60)}…)`);
    ok(!/undefined|NaN|\[object Object\]/.test(text), `${net} ${path}: no undefined/NaN/[object Object]`);
    ok(await store.evaluate(layout[path]), `${net} ${path}: keeps its own layout after moving in place`);
    ok(/Nap ·/.test(await store.textContent('[data-wallet-button]')), `${net} ${path}: still connected`);
    if (net === 'testnet') ok(!!(await store.$('.testnet-banner')), `${net} ${path}: says it is testnet`);
  }
  // The open mint (testnet is in it) refuses a count that is not whole.
  await store.click('.topbar-nav a[href="/mint"]');
  await store.waitForTimeout(3500);
  if (await store.isVisible('#mint-in').catch(() => false)) {
    await store.fill('#mint-in', '1.5');
    ok(await store.$eval('#mint', (b) => b.disabled), `${net} /mint: 1.5 items cannot be minted`);
    await store.fill('#mint-in', '2');
    ok(await store.$eval('#mint', (b) => !b.disabled && /Mint 2 for/.test(b.textContent)), `${net} /mint: 2 can, and the button says what it costs`);
  }
  // An item page for something that does not exist says so.
  const q = net === 'testnet' ? '&network=testnet' : '';
  for (const bad of ['/item?serial=99999', '/item?serial=', '/item?serial=1.5']) {
    await store.goto(`${SITE}${bad}${q}`, { waitUntil: 'networkidle' });
    await store.waitForTimeout(1200);
    ok(await store.isVisible('#s-noid').catch(() => false), `${net} ${bad}: "no such item", not a made-up one`);
  }
  // The pages that load in full.
  for (const path of ['/deposit', '/exit']) {
    const before = errors.length;
    await store.goto(`${SITE}${path}${net === 'testnet' ? '?network=testnet' : ''}`, { waitUntil: 'networkidle' });
    await store.waitForTimeout(1500);
    const text = (await store.textContent('main').catch(() => '')) || '';
    ok(text.trim().length > 40 && errors.length === before && !/bound to your own key/.test(text), `${net} ${path}: renders, current copy`);
  }
  // The members' chat: the door in its own panel, and the microphone allowed
  // on /chat (voice rooms) while the rest of the store keeps it off.
  const before = errors.length;
  await store.goto(`${SITE}/chat${q.replace('&', '?')}`, { waitUntil: 'networkidle' });
  await store.waitForTimeout(1500);
  ok(errors.length === before && await store.isVisible('#enter'), `${net} /chat: the door renders`);
  ok(await store.evaluate(() => getComputedStyle(document.querySelector('.register')).maxWidth === '940px' && getComputedStyle(document.querySelector('.ledger')).borderTopStyle === 'solid'), `${net} /chat: keeps its own layout`);
  const policy = async (p) => (await store.request.get(`${SITE}${p}`)).headers()['permissions-policy'] || '';
  ok(/microphone=\(self\)/.test(await policy('/chat/')) && /microphone=\(\)/.test(await policy('/items/')), `${net} /chat: only it may use the microphone`);
}

async function webPath(browser, net) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const store = await ctx.newPage();
  watch(store, `store ${net}`);
  const q = net === 'testnet' ? '?network=testnet' : '';
  await store.goto(`${SITE}/items${q}`, { waitUntil: 'networkidle' });
  const win = await firstConnect(ctx, store, net);
  await tour(store, net);

  // A reload loses the page's link to Nap's window, not the window: Connect
  // finds it again by name, still unlocked, and connects without a password.
  await store.goto(`${SITE}/items${q}`, { waitUntil: 'networkidle' });
  await store.click('[data-wallet-button]');
  await store.waitForSelector('.zwallet-option', { timeout: 20000 });
  const pages = ctx.pages().length;
  await store.click('.zwallet-option:has-text("Nap in the browser")');
  await store.waitForFunction(() => /Nap ·/.test(document.querySelector('[data-wallet-button]')?.textContent || ''), null, { timeout: 20000 })
    .then(() => ok(true, `${net}: after a reload, Connect finds Nap's open window — no password, no new window`), () => ok(false, `${net}: after a reload, Connect finds Nap's open window`));
  ok(ctx.pages().length === pages, `${net}: no second Nap window was opened`);

  // Closing Nap's window: the store stops treating the connection as live.
  await win.close();
  await store.goto(`${SITE}/items${q}`, { waitUntil: 'networkidle' });
  await store.click('[data-wallet-button]');
  await store.waitForSelector('.zwallet-option, .zwallet-off', { timeout: 20000 });
  ok(!!(await store.$('.zwallet-option')), `${net}: after Nap's window closes, the store asks to connect again`);
  await store.keyboard.press('Escape');

  // Refusing: nothing is shared, and the store says so.
  const win2 = await choose(ctx, store, 'Nap in the browser');
  await win2.waitForSelector('#pw', { timeout: 20000 });
  ok(true, `${net}: a returning visitor is asked only for the password`);
  await win2.click('#deny');
  await store.waitForFunction(() => /Not connected|Nothing was shared/.test(document.querySelector('#zwallet-status')?.textContent || ''), null, { timeout: 15000 })
    .then(() => ok(true, `${net}: refusing reaches the store as "not connected"`), () => ok(false, `${net}: refusing reaches the store as "not connected"`));
  await win2.close().catch(() => {});
  await store.keyboard.press('Escape');

  // Reconnecting with the password.
  const win3 = await choose(ctx, store, 'Nap in the browser');
  await win3.waitForSelector('#pw', { timeout: 20000 });
  await unlockAndApprove(win3, store, net);
  await ctx.close();
}

// ---------------------------------------------------------------------------
// Through the Nap extension: the store finds it, and its own connect window
// sets a wallet up the same way.

async function extensionPath() {
  const ctx = await chromium.launchPersistentContext('', {
    executablePath: CHROMIUM, headless: true,
    args: [`--disable-extensions-except=${EXT}`, `--load-extension=${EXT}`, '--headless=new'],
    viewport: { width: 1280, height: 900 },
  });
  let [sw] = ctx.serviceWorkers(); if (!sw) sw = await ctx.waitForEvent('serviceworker');
  const store = await ctx.newPage();
  watch(store, 'store (extension)');
  await store.goto(`${SITE}/items?network=testnet`, { waitUntil: 'networkidle' });
  const win = await choose(ctx, store, 'Nap Wallet');
  ok(new URL(win.url()).protocol === 'chrome-extension:', `the extension's own connect window opens (${new URL(win.url()).protocol})`);
  await win.waitForSelector('#setup-create', { timeout: 20000 });
  ok(!(await win.$('#words')), 'extension: a new visitor is offered setup, not a phrase box');
  const [tab] = await Promise.all([ctx.waitForEvent('page'), win.click('#setup-create')]);
  await tab.waitForSelector('#create-form', { timeout: 20000 });
  await tab.check('#ack');
  await tab.fill('#keep-password', PASSWORD);
  await tab.click('#create-form .primary');
  await tab.waitForSelector('.wallet', { timeout: 60000 });
  await tab.close();
  await win.waitForSelector('#pw, #yes', { timeout: 15000 });
  if (await win.$('#pw')) { await win.fill('#pw', PASSWORD); await win.click('#unlock button[type=submit]'); }
  await win.waitForSelector('#yes', { timeout: 60000 });
  await win.click('#yes');
  await store.waitForFunction(() => /Nap ·/.test(document.querySelector('[data-wallet-button]')?.textContent || ''), null, { timeout: 30000 })
    .then(() => ok(true, 'extension: the store is connected'), () => ok(false, 'extension: the store is connected'));
  await tour(store, 'testnet');
  await ctx.close();
}

const browser = await chromium.launch({ executablePath: CHROMIUM, headless: true });
try {
  await landing(browser);
  await webPath(browser, 'testnet');
  await webPath(browser, 'mainnet');
} catch (e) { ok(false, `stopped: ${e.message.split('\n')[0]}`); }
await browser.close();
if (process.env.EXT !== '0') {
  try { await extensionPath(); } catch (e) { ok(false, `extension stopped: ${e.message.split('\n')[0]}`); }
}
ok(errors.length === 0, `no page errors${errors.length ? `:\n  ${errors.join('\n  ')}` : ''}`);
console.log(failed ? `${failed} failed` : 'all passed', `— screenshots in ${S}`);
