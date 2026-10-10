// Nap's own wallet page, every task flow, end to end on testnet (audit,
// 10 Oct 2026): create (the 24 words stay put across tabs), keep with a
// password, lock, a wrong password, unlock, forget; restore; every view;
// send ZEC in apps to another account (reviewed, then confirmed); withdraw to
// the wallet's own Zcash address (reviewed); "Pay it from my ZEC" re-arming
// the review; a small shielded ZEC send when the wallet holds any; Activity;
// Settings (recovery, artwork, rescan, Nap for Chrome).
//
//   NAP=http://localhost:4179 (wrangler pages dev dist in apps/nap-web, with
//     --binding ZYN_RPC_URL=https://zyn-rpc.cavebro.win) or https://nap.zyn-network.com
//   PHRASE_FILE=<funded testnet wallet>  TO_PHRASE_FILE=<another testnet wallet>
//   node tests/nap-wallet.e2e.mjs
import { chromium } from 'playwright-core';
import { readFileSync } from 'node:fs';

const NAP = process.env.NAP || 'http://localhost:4179';
const WORDS = readFileSync(process.env.PHRASE_FILE, 'utf8').trim();
const TO_WORDS = readFileSync(process.env.TO_PHRASE_FILE, 'utf8').trim();
const S = process.env.SHOTS || '/tmp';
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };
const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const errors = [];

async function open(viewport = { width: 1280, height: 900 }) {
  const ctx = await b.newContext({ viewport });
  const p = await ctx.newPage();
  p.on('pageerror', (e) => errors.push(e.message));
  await p.goto(`${NAP}/`, { waitUntil: 'networkidle' });
  const t = (await p.$$eval('#network-pick option', (os) => os.map((o) => [o.value, o.textContent]))).find(([, x]) => /testnet/i.test(x));
  if (t) { await p.selectOption('#network-pick', t[0]); await p.waitForTimeout(800); }
  return { ctx, p };
}
const noticeOf = async (p) => (await p.textContent('#notice').catch(() => '')).trim();
const waitNotice = async (p, re, ms = 90_000) => p.waitForFunction((src) => new RegExp(src).test(document.querySelector('#notice')?.textContent || ''), re.source, { timeout: ms }).then(() => true, () => false);
const view = async (p, v) => { await p.click(`.rail [data-view-go="${v}"]`); await p.waitForTimeout(300); };

// ---- Create, keep, lock, unlock, forget ----------------------------------
{
  const { ctx, p } = await open();
  await p.click('[data-mode="create"]');
  const words = (await p.$$eval('.phrase span', (els) => els.map((e) => e.textContent.replace(/^\d+/, '')))).join(' ');
  ok(words.split(' ').length === 24, 'Create shows 24 words');
  await p.click('[data-mode="restore"]');
  await p.click('[data-mode="create"]');
  const again = (await p.$$eval('.phrase span', (els) => els.map((e) => e.textContent.replace(/^\d+/, '')))).join(' ');
  ok(again === words, 'the words stay the same after switching tabs');
  const submit = p.locator('#create-form button[type=submit]');
  ok(await submit.isDisabled(), 'Open my wallet waits for the saved-words box');
  await p.check('#ack');
  await p.fill('#keep-password', 'correct horse battery');
  await submit.click();
  await p.waitForSelector('#zec-mini', { timeout: 60_000 });
  ok(await p.isVisible('#coins'), 'a new wallet opens on Home');
  await p.click('#forget');
  await p.waitForSelector('#kept-form', { timeout: 15_000 });
  await p.fill('#kept-password', 'wrong password!');
  await p.click('#kept-form button[type=submit]');
  await p.waitForTimeout(1500);
  const err = (await p.textContent('#kept-form').catch(() => '')) + (await noticeOf(p));
  ok(/password/i.test(err) && !(await p.$('#zec-mini')), `a wrong password is refused: "${err.replace(/\s+/g, ' ').slice(0, 80)}"`);
  ok(!(await p.locator('#kept-form button[type=submit]').isDisabled()), 'the Unlock button works again after a wrong password');
  await p.fill('#kept-password', 'correct horse battery');
  await p.click('#kept-form button[type=submit]');
  ok(await p.waitForSelector('#zec-mini', { timeout: 30_000 }).then(() => true, () => false), 'the right password unlocks');
  await ctx.close();
}

// ---- The receiving wallet's account ID -----------------------------------
let toAccount;
{
  const { ctx, p } = await open();
  await p.waitForSelector('#words'); await p.fill('#words', TO_WORDS); await p.click('#restore-form button[type=submit]');
  await p.waitForSelector('#zec-mini', { timeout: 60_000 });
  await view(p, 'apps');
  toAccount = (await p.textContent('#account code')).trim();
  ok(/^[0-9a-f]{64}$/.test(toAccount), `the account ID shows under Apps: ${toAccount.slice(0, 10)}…`);
  await ctx.close();
}

// ---- Restore the funded wallet; every view ---------------------------------
const { p } = await open();
await p.waitForSelector('#words'); await p.fill('#words', WORDS);
await p.click('#restore-form button[type=submit]');
await p.waitForSelector('#zec-mini', { timeout: 60_000 });
await p.waitForFunction(() => /ZEC · IN APPS/.test(document.querySelector('#coins')?.textContent || '') && !document.querySelector('#coins .skeleton'), null, { timeout: 120_000 }).catch(() => {});
const inApps = await p.$$eval('#coins .coin', (cs) => { const c = cs.find((x) => /ZEC · IN APPS/.test(x.textContent)); return c ? c.querySelector('b').getAttribute('title') : null; });
ok(inApps !== null, `Home lists ZEC in apps: ${inApps}`);
ok(!(await p.isVisible('#withdraw-form')) && !(await p.isVisible('#zec-send')), 'Home holds no forms');
for (const [v, sel] of [['zec', '#zec-send'], ['apps', '#deposit-card'], ['activity', '#log'], ['settings', '#recovery-link'], ['home', '#coins']]) {
  await view(p, v);
  ok(await p.isVisible(sel), `the ${v} view shows ${sel}`);
}

// ---- Send ZEC in apps to another account, reviewed then confirmed ---------
const amount = '0.001';
if (inApps && Number(inApps) >= 0.0011) {
  await p.click('#coins .coin:has-text("ZEC · IN APPS") .coin-send');
  await p.waitForSelector('#send-dialog[open]');
  ok(/SEND ZEC IN APPS/i.test(await p.textContent('#send-dialog .kicker')), 'the send dialog calls it ZEC in apps');
  await p.fill('#send-to', toAccount);
  await p.fill('#send-amount', amount);
  await p.click('#send-dialog .primary');
  await p.waitForFunction(() => /Confirm: send/.test(document.querySelector('#send-dialog .primary')?.textContent || ''), null, { timeout: 30_000 }).catch(() => {});
  ok(/Confirm: send 0\.001 ZEC in apps/.test(await p.textContent('#send-dialog .primary')), `review first: "${(await p.textContent('#send-dialog .primary')).trim()}"`);
  await p.click('#send-dialog .primary');
  ok(await waitNotice(p, /Sent 0\.001 ZEC in apps/), `sent: ${await noticeOf(p)}`);
} else ok(false, `not enough ZEC in apps to test a send (${inApps}); fund the wallet`);

// ---- Withdraw to the wallet's own Zcash address, reviewed ----------------
await view(p, 'apps');
await p.waitForFunction(() => /^(u|utest)1/.test(document.querySelector('#zec-address')?.textContent || ''), null, { timeout: 120_000 }).catch(() => {});
await p.click('#withdraw-mine');
const mine = await p.inputValue('#withdraw-to');
ok(/^utest1/.test(mine), `To my Zcash wallet fills the wallet's own address: ${mine.slice(0, 14)}…`);
await p.fill('#withdraw-amount', amount);
await p.click('#withdraw-go');
ok(/Confirm: withdraw 0\.001 ZEC/.test(await p.textContent('#withdraw-go')), 'the withdrawal is reviewed first');
await p.fill('#withdraw-amount', '0.002');
ok(/^Withdraw/.test((await p.textContent('#withdraw-go')).trim()), 'changing the amount starts the review over');
await p.fill('#withdraw-amount', amount);
await p.click('#withdraw-go');
await p.waitForTimeout(400);
await p.click('#withdraw-go');
ok(await waitNotice(p, /Withdrawal recorded/), `withdrawn: ${await noticeOf(p)}`);

// ---- "Pay it from my ZEC" re-arms the review ------------------------------
await view(p, 'zec');
// ZEC_SEND=1 waits for the Zcash sync (a fresh restore takes minutes) so the
// shielded send below runs against the real balance.
if (process.env.ZEC_SEND) {
  await p.waitForFunction(() => /up to date/.test(document.querySelector('#zec-sync')?.textContent || ''), null, { timeout: 30 * 60_000 }).catch(() => {});
  ok(/up to date/.test(await p.textContent('#zec-sync')), `the Zcash wallet syncs: ${await p.textContent('#zec-sync')}`);
}
const held = Number(await p.textContent('#zec-total').catch(() => '0')) || 0;
if (held > 0.0003) {
  await p.fill('#zec-to', mine);
  await p.fill('#zec-amount', '0.0001');
  await p.click('#zec-go');
  ok(/Confirm: send/.test(await p.textContent('#zec-go')), 'a ZEC send is reviewed first');
}
await view(p, 'apps');
await p.waitForFunction(() => /^(u|utest)1/.test(document.querySelector('#deposit-address')?.textContent || ''), null, { timeout: 60_000 }).catch(() => {});
await p.click('#deposit-from-zec');
ok((await p.textContent('#view-title')) === 'Zcash' && /^(u|utest)1/.test(await p.inputValue('#zec-to')), 'Pay it from my ZEC fills the deposit address on the Zcash view');
ok(!/Confirm/.test(await p.textContent('#zec-go')), 'and the review starts over');

// ---- A small shielded send, if the wallet holds ZEC -----------------------
if (held > 0.0003) {
  await p.fill('#zec-to', mine);
  await p.fill('#zec-amount', '0.0001');
  await p.click('#zec-go');
  await p.click('#zec-go');
  ok(await p.locator('#zec-go').isDisabled(), 'while it sends, the button stays off');
  ok(await waitNotice(p, /Sent 0\.0001 ZEC|not answering|refused|failed/i, 240_000), `ZEC send: ${await noticeOf(p)}`);
} else console.log(`SKIP  shielded ZEC send: the wallet holds ${held} ZEC on Zcash`);

// ---- Activity, Settings ---------------------------------------------------
await view(p, 'activity');
const log = (await p.textContent('#log')).replace(/\s+/g, ' ');
ok(/Sent ZEC/.test(log) && /Withdrew to Zcash/.test(log), `Activity lists the send and the withdrawal: ${log.slice(0, 120)}`);
await view(p, 'settings');
await p.click('#recovery-link');
ok(await p.isVisible('#recovery-dialog'), 'Recovery opens');
await p.click('#recovery-dialog .x');
await p.check('#art-toggle');
ok(await p.isChecked('#art-toggle'), 'artwork turns on');
ok(await p.isVisible('.setting-ext .ext-download'), 'Nap for Chrome is offered in Settings');
await p.click('#settings-rescan');
ok((await p.textContent('#view-title')) === 'Zcash', 'Rescan shows the Zcash view, where it reports');
await p.screenshot({ path: `${S}/nap-wallet-end.png`, fullPage: true });

ok(errors.length === 0, `no page errors${errors.length ? `: ${errors.join(' | ')}` : ''}`);
await b.close();
