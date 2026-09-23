// Nap Connect through the extension: discovery, the connect gate, and the
// extension's own approval window. Needs a Chromium that still honours
// --load-extension (CHROMIUM=…; branded Chrome no longer does). See README.
import { chromium } from 'playwright-core';
const EXT = new URL('../../extension/dist', import.meta.url).pathname;
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };
const ctx = await chromium.launchPersistentContext('', {
  executablePath: process.env.CHROMIUM,
  headless: true,
  args: ['--headless=new', `--disable-extensions-except=${EXT}`, `--load-extension=${EXT}`],
});
const store = await ctx.newPage();
await store.goto('http://localhost:4178/?network=testnet&nap=http://localhost:4179#market');
await store.waitForTimeout(800);
await store.click('.header-wallet');
await store.waitForSelector('.wallet-option');
const names = await store.$$eval('.wallet-option b', (els) => els.map((e) => e.textContent));
ok(names[0] === 'Nap Wallet' && names.includes('Nap in the browser'), `picker lists ${JSON.stringify(names)}`);
const [confirm] = await Promise.all([ctx.waitForEvent('page'), store.click('.wallet-option:has-text("Nap Wallet")')]);
await confirm.waitForSelector('#desc');
await confirm.waitForFunction(() => document.querySelector('#desc').textContent.length > 20);
const desc = await confirm.textContent('#desc');
const origin = await confirm.textContent('#origin');
ok(confirm.url().startsWith('chrome-extension://') && /confirm\.html/.test(confirm.url()), 'approval opens in the extension\'s own window');
ok(origin === 'http://localhost:4178' && /Connect .* to Nap/.test(desc) && /not your balances or keys/.test(desc), `it names the site and what connecting shares`);
await confirm.click('#approve');
await store.waitForFunction(() => /not running|Not connected/.test(document.querySelector('.wallet-status')?.textContent || ''), null, { timeout: 10000 });
ok(/Nap Wallet app is not running/.test(await store.textContent('.wallet-status')), `with desktop Nap off, the store says so: "${await store.textContent('.wallet-status')}"`);
// An unconnected read of the holder's data asks to connect first (another
// origin: localhost:4178 was just connected, and asking again would be noise).
const fresh = await ctx.newPage();
await fresh.goto('http://127.0.0.1:4178/');
await fresh.waitForTimeout(500);
const [again] = await Promise.all([ctx.waitForEvent('page'), fresh.evaluate(() => { window.zyn.request({ method: 'zyn_account' }).catch(() => {}); })]);
await again.waitForFunction(() => document.querySelector('#desc').textContent.length > 20);
ok(/Connect http:\/\/127\.0\.0\.1:4178/.test(await again.textContent('#desc')), `an account read from a site that is not connected prompts: "${(await again.textContent('#desc')).slice(0, 60)}…"`);
await again.click('#reject');
await ctx.close();
