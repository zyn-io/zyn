// ZCaveDAO end to end on testnet, through the real /dao page and a real Nap in
// the browser (DECISIONS §172): membership checked by Nap, a member pass the
// chain then recognises, a proposal backed by the member's item, a yes vote,
// and the vote closing as passed — each step read back from the chain itself.
//
// Needs testnet's DAO on strict rules with 1 backer and a short window
// (zyn-sale-draw dao-terms 1 8 500: 8 Zcash blocks, one item meets a 5% quorum
// of up to 20) and the clock ticking every block (ZYN_DAO_CLOCK_EVERY=1), and a
// wallet holding a batch-one item. EXPECT=passed asserts the vote passes.
//
//   MEMBER_PHRASE=<file>  node tests/dao.e2e.mjs
import { chromium } from 'playwright-core';
import { unlockNap } from './unlock.mjs';
import { readFileSync } from 'node:fs';
import { daoPass } from '../../nap-web/src/market.js';
import { hex, unhex } from '../../nap-web/src/object.js';

const SITE = process.env.SITE || 'https://cavebro.win';
const RPC = process.env.RPC || 'https://zyn-rpc.cavebro.win/object';
const S = process.env.SHOTS || '/tmp';
const CAVE_Z = '024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e';
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };
const rpc = async (method, fields = {}) => (await (await fetch(RPC, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params: [], ...fields }) })).json()).result;
const until = async (what, test, ms = 300000) => {
  const end = Date.now() + ms;
  for (;;) {
    const v = await test();
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 10000));
  }
};

const dao = await rpc('zyn_dao', { collection: CAVE_Z });
if (!dao?.set) throw new Error('testnet has no DAO — run zyn-sale-draw dao-terms 1 8 500 on the node first');
const before = dao.proposals.length;

const b = await chromium.launch({ executablePath: process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
const page = await ctx.newPage();
page.on('pageerror', (e) => console.log('page error:', e.message));
const words = readFileSync(process.env.MEMBER_PHRASE, 'utf8').trim();
let nap = null;

async function approve(expect) {
  await nap.waitForSelector('#yes', { timeout: 90000 });
  const text = await nap.textContent('main');
  ok(expect.test(text), `Nap shows ${expect}`);
  await nap.click('#yes');
}

await page.goto(`${SITE}/dao/?network=testnet`);
ok(/ZCaveDAO/.test(await page.title()), 'the DAO page loads');
await page.waitForFunction(() => !document.querySelector('#f-treasury-box')?.hidden, null, { timeout: 30000 });
ok(/ZEC\.zy/.test(await page.textContent('#f-treasury')), `the DAO treasury reads from the chain: ${await page.textContent('#f-treasury')}`);

// Membership: Nap answers in the page.
await page.click('#check');
await page.waitForSelector('.zwallet-option');
const opened = new Set(ctx.pages());
await page.click('.zwallet-option:has-text("Nap in the browser")');
nap = await ctx.waitForEvent('page', { timeout: 15000 }).catch(() => ctx.pages().find((x) => !opened.has(x)));
await unlockNap(nap, words);
await nap.waitForSelector('#yes', { timeout: 30000 });
await nap.click('#yes');
await page.waitForFunction(() => /You are a member|No Cave item/.test(document.querySelector('#member-out')?.textContent || ''), null, { timeout: 60000 });
const member = await page.textContent('#member-out');
ok(/You are a member/.test(member), `the page says: ${member}`);

// A member pass, which the chain then recognises by its hash alone.
await page.waitForSelector('#get-pass:not([hidden])');
await page.click('#get-pass');
await approve(/Member pass/);
await page.waitForSelector('#pass-box:not([hidden])', { timeout: 60000 });
const token = (await page.textContent('#pass-code')).trim();
ok(/^[0-9a-f]{64}$/.test(token), 'the page shows a 64-hex pass code');
const pass = hex(daoPass(unhex(token)));
await until('the pass to be live', async () => (await rpc('zyn_memberPass', { collection: CAVE_Z, pass }))?.member);
ok(true, 'zyn_memberPass says the pass is a live member pass');
ok(!(await rpc('zyn_memberPass', { collection: CAVE_Z, pass: '00'.repeat(32) }))?.member, 'a made-up pass is not');

// A proposal: the text stored by hash, the proposer's item backing it.
const text = `e2e proposal ${new Date().toISOString()}: fund one testnet replica for a month.`;
await page.click('#new-prop');
await page.fill('#prop-form textarea[name=text]', text);
await page.click('#prop-send');
await approve(/Proposal/);
const mine = await until('the proposal to appear on the chain', async () => {
  const d = await rpc('zyn_dao', { collection: CAVE_Z });
  return d.proposals.length > before ? d.proposals[0] : null;
});
ok(['backing', 'voting'].includes(mine.state), `proposal #${mine.id} is on the chain (${mine.state})`);
// The page looks again on its own after a proposal (a reload would drop Nap).
await page.waitForFunction((t) => document.querySelector('#props')?.textContent.includes(t), text, { timeout: 180000 });
ok(true, 'the page shows the proposal\'s text, checked against its hash');

// The vote: yes with the member's items, then the close.
if (mine.state === 'backing') throw new Error('set the DAO to 1 backer for this test');
await page.waitForSelector(`[data-vote="yes"][data-p="${mine.id}"]`, { timeout: 60000 }).catch(async (e) => {
  console.log('props now:', (await page.innerHTML('#props')).replace(/\s+/g, ' ').slice(0, 900));
  console.log('member-out:', await page.textContent('#member-out'));
  throw e;
});
await page.click(`[data-vote="yes"][data-p="${mine.id}"]`);
await approve(new RegExp(`Vote yes on proposal #${mine.id}`));
const voted = await until('the yes vote to count', async () => {
  const p = (await rpc('zyn_dao', { collection: CAVE_Z })).proposals.find((x) => x.id === mine.id);
  return p.yes > 0 ? p : null;
});
ok(voted.yes > 0, `${voted.yes} yes counted for #${mine.id}`);
const closed = await until('the vote to close', async () => {
  const p = (await rpc('zyn_dao', { collection: CAVE_Z })).proposals.find((x) => x.id === mine.id);
  return ['passed', 'failed', 'unfunded'].includes(p.state) ? p : null;
}, 2400000);
// The ledger's rule (§172.6): more yes than no, and turnout at least the
// quorum of outstanding items, rounded up, and at least one.
// A strict proposal (§173) carries its own quorum, fixed when its vote opened.
const terms = await rpc('zyn_dao', { collection: CAVE_Z });
const quorum = closed.terms?.quorum ?? Math.max(1, Math.ceil((terms.outstanding * terms.quorum_bps) / 10000));
const expect = closed.yes > closed.no && closed.yes + closed.no >= quorum ? 'passed' : 'failed';
ok(closed.state === expect, `the vote closed ${closed.state} (${closed.yes} yes, ${closed.no} no, quorum ${quorum}), as the rule says`);
ok(process.env.EXPECT !== 'passed' || closed.state === 'passed', 'and it passed, as this run was set up to');
await page.screenshot({ path: `${S}/dao-e2e.png`, fullPage: true });
await b.close();
