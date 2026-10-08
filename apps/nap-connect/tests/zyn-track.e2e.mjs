// The Zyn track end to end on testnet (DECISIONS §176), through the /dao page
// and Nap in the browser: a member proposes on Zyn, votes yes, the team's own
// account votes no with `zyn-team-vote`, and under a veto the vote closes
// failed — each step read back from the chain.
//
// Needs testnet's DAO on strict rules with 1 backer and a short window, its
// Zyn track set as a veto to the test team (zyn-sale-draw dao-zyn <team> veto),
// the clock ticking every block, a wallet holding a batch-one item, and the
// team key file.
//
//   MEMBER_PHRASE=<file> TEAM_KEY=<file> TEAM_VOTE=<zyn-team-vote binary> \
//   SITE=http://localhost:8788 NAP=http://localhost:5173  node tests/zyn-track.e2e.mjs
import { chromium } from 'playwright-core';
import { unlockNap } from './unlock.mjs';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

const SITE = process.env.SITE || 'https://cavebro.win';
const NAP = process.env.NAP || '';
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
const prop = async (id) => (await rpc('zyn_dao', { collection: CAVE_Z })).proposals.find((x) => x.id === id);

const dao = await rpc('zyn_dao', { collection: CAVE_Z });
if (!dao?.zyn) throw new Error('testnet has no Zyn track — run zyn-sale-draw dao-zyn <team> veto on the node first');
ok(dao.zyn.rule === 'veto', `the track is a veto, team ${dao.zyn.team.slice(0, 12)}…`);
const known = new Set(dao.proposals.map((p) => p.id));

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
  return text;
}

await page.goto(`${SITE}/dao/?network=testnet${NAP ? `&nap=${encodeURIComponent(NAP)}` : ''}`);
await page.waitForFunction(() => !document.querySelector('#f-treasury-box')?.hidden, null, { timeout: 30000 });
await page.click('#check');
await page.waitForSelector('.zwallet-option');
const opened = new Set(ctx.pages());
await page.click('.zwallet-option:has-text("Nap in the browser")');
nap = await ctx.waitForEvent('page', { timeout: 15000 }).catch(() => ctx.pages().find((x) => !opened.has(x)));
await unlockNap(nap, words);
await nap.waitForSelector('#yes', { timeout: 30000 });
await nap.click('#yes');
await page.waitForFunction(() => /You are a member|No Cave item/.test(document.querySelector('#member-out')?.textContent || ''), null, { timeout: 60000 }).catch(async (e) => {
  console.log('member-out:', await page.textContent('#member-out').catch(() => '?'));
  console.log('nap says:', (await nap.textContent('body').catch(() => '?')).replace(/\s+/g, ' ').slice(0, 600));
  await page.screenshot({ path: `${S}/zyn-track-fail-page.png` });
  await nap.screenshot({ path: `${S}/zyn-track-fail-nap.png` }).catch(() => {});
  throw e;
});
ok(/You are a member/.test(await page.textContent('#member-out')), 'a member');

// A Zyn proposal: the form offers the track because the chain has one, and
// hides the payment fields for it.
const text = `e2e Zyn proposal ${new Date().toISOString()}: bridge one more coin next.`;
await page.click('#new-prop');
ok(await page.isVisible('#prop-track-box'), 'the form offers Cave or Zyn');
await page.selectOption('#prop-form select[name=track]', 'zyn');
ok(!(await page.isVisible('#prop-form input[name=amount]')), 'a Zyn proposal has no payment fields');
ok(/team can veto, not pass/.test(await page.textContent('#prop-zyn-note')), 'the form says what the team\'s vote weighs');
await page.fill('#prop-form textarea[name=text]', text);
await page.click('#prop-send');
const card = await approve(/Zyn proposal/);
ok(/can veto, not pass/.test(card), 'Nap\'s card says the team can veto, not pass');
await nap.click('#yes');
const mine = await until('the Zyn proposal on the chain', async () => {
  const d = await rpc('zyn_dao', { collection: CAVE_Z });
  return d.proposals.find((p) => !known.has(p.id)) || null;
});
ok(mine.track === 'zyn' && mine.pay == null, `proposal #${mine.id} is on the Zyn track and pays nothing`);
ok(mine.team === 'none', 'the team has not voted yet');
if (mine.state !== 'voting') throw new Error('set the DAO to 1 backer for this test');
await page.waitForFunction((t) => document.querySelector('#props')?.textContent.includes(t), text, { timeout: 180000 });
ok(/Zyn · the team can veto, not pass · team has not voted/.test(await page.textContent('#props')), 'the page lists it as Zyn, team not voted');

// The member votes yes.
await page.click(`[data-vote="yes"][data-p="${mine.id}"]`);
await approve(new RegExp(`Vote yes on Zyn proposal #${mine.id}`));
await nap.click('#yes');
await until('the yes vote to count', async () => ((await prop(mine.id)).yes > 0 ? true : null));
ok(true, 'the member\'s yes counted');

// The team's account votes no, signed by its own key.
const out = execFileSync(process.env.TEAM_VOTE, [String(mine.id), 'no'], { env: { ...process.env, ZYN_TEAM_NODE: RPC, ZYN_TEAM_KEY: process.env.TEAM_KEY }, encoding: 'utf8' });
ok(/TeamVoted/.test(out), 'zyn-team-vote cast the team\'s no');
ok((await prop(mine.id)).team === 'no', 'the chain shows the team voted no');
let again = '';
try { execFileSync(process.env.TEAM_VOTE, [String(mine.id), 'yes'], { env: { ...process.env, ZYN_TEAM_NODE: RPC, ZYN_TEAM_KEY: process.env.TEAM_KEY }, encoding: 'utf8', stdio: 'pipe' }); } catch (e) { again = String(e.stderr || e.message); }
ok(/already voted/.test(again), 'the team cannot vote twice');

const closed = await until('the vote to close', async () => {
  const p = await prop(mine.id);
  return ['passed', 'failed'].includes(p.state) ? p : null;
}, 2400000);
ok(closed.state === 'failed', `the vote closed ${closed.state} (${closed.yes} yes, ${closed.no} no, team ${closed.team}): the members carried it, the team vetoed`);
await page.screenshot({ path: `${S}/zyn-track-e2e.png`, fullPage: true });
await b.close();
