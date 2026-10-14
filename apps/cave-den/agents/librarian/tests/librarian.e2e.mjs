// The librarian against a real den: an owner and a member in real browsers,
// the librarian as an agent of its own (added with "Add an agent"). It answers
// in the room asked, searches every room only for its owner, and forgets what
// was deleted.
//
//   (cd ../../server && mix phx.server)        # the dev den, its page built
//   ../../agent/build.sh && PLAYWRIGHT_FROM=… node tests/librarian.e2e.mjs
import { createRequire } from 'node:module';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const require = createRequire(process.env.PLAYWRIGHT_FROM ? `${process.env.PLAYWRIGHT_FROM.replace(/\/$/, '')}/package.json` : new URL('../../../web/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const DEN = process.env.DEN || 'http://localhost:4000';
const CHROME = process.env.CHROMIUM || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const SERVER = new URL('../../../server/', import.meta.url).pathname;
const AGENT_BIN = new URL('../../../agent/bin/epeaky-agent.mjs', import.meta.url).pathname;
const LIB = new URL('../epeaky-librarian.mjs', import.meta.url).pathname;
const HOME = mkdtempSync(join(tmpdir(), 'elib-'));
const env = { ...process.env, EPEAKY_HOME: HOME, EPEAKY_AGENT_BIN: AGENT_BIN };
const tag = Math.random().toString(36).slice(2, 8);

const host = (expr) => execFileSync('mix', ['run', '--no-start', '-e', `Application.ensure_all_started(:ecto_sqlite3); {:ok, _} = Den.Repo.start_link(); ${expr}`], { cwd: SERVER, encoding: 'utf8' }).trim().split('\n').at(-1);
const invite = () => `${DEN}/#invite=${host('IO.puts(hd(Den.Gate.Members.invite(1, 1)))')}`;
const ok = (c, w) => { console.log(`${new Date().toISOString().slice(11, 23)} ${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };
const until = (p, f, arg, ms = 30000) => p.waitForFunction(f, arg, { timeout: ms }).then(() => true, () => false);

const b = await chromium.launch({ executablePath: CHROME, headless: true });
async function person(name) {
  const p = await (await b.newContext({ viewport: { width: 1300, height: 900 } })).newPage();
  await p.goto(invite(), { waitUntil: 'networkidle' });
  await p.waitForSelector('#den:not([hidden])', { timeout: 20000 });
  await until(p, () => window.__den.inRoom('general'));
  await p.evaluate((n) => window.__den.setNick(n), name);
  return p;
}
/// The librarian's answer to the question `q` asked in `room`: its reply to it.
async function answerTo(p, room, q, ms = 90000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const text = await p.evaluate(async ([r, words]) => {
      const log = await window.__den.log(r);
      const asked = log.filter((x) => x.text === words).at(-1);
      const m = asked && log.find((x) => x.reply === asked.id && window.__den.nickFor(x.key) === 'librarian');
      return m ? m.text : null;
    }, [room, q]);
    if (text) return text;
    await new Promise((r) => setTimeout(r, 500));
  }
  return null;
}
const ask = async (p, room, q) => { await p.evaluate(([r, w]) => window.__den.say(r, w), [room, q]); return answerTo(p, room, q); };
const lastSeq = (p, room) => p.evaluate(async (r) => Math.max(0, ...(await window.__den.log(r)).map((x) => x.seq || 0)), room);

let lib;
try {
  const O = await person('Olu');
  const oKey = await O.evaluate(() => window.__den.me.key);
  const M = await person('Sam');
  host(`:ok = Den.Mod.set_owner("${oKey}"); IO.puts("ok")`);
  await O.reload({ waitUntil: 'networkidle' });
  await until(O, () => window.__den.inRoom('general') && window.__den.mod.owner === window.__den.me.key, null, 30000);

  // A second open room with a word only it holds.
  const other = await O.evaluate((t) => window.__den.push(window.__den.lobby, 'open_room', { title: `Ops ${t}` }), tag);
  await until(O, ([id]) => window.__den.inRoom(id), [other.id], 30000);

  // The librarian: "Add an agent", run as its own agent, then started.
  await O.click('#add-agent');
  await O.waitForFunction(() => /--owner /.test(document.querySelector('#ask input')?.value || ''), null, { timeout: 15000 });
  const shown = await O.inputValue('#ask input');
  await O.click('#ask [data-yes]').catch(() => {});
  execFileSync(process.execPath, [AGENT_BIN, 'join', /join '([^']+)'/.exec(shown)[1], '--owner', /--owner (\S+)$/.exec(shown)[1], '--agent', 'librarian', '--nick', 'librarian'], { env, timeout: 120000 });
  lib = spawn(process.execPath, [LIB, '--agent', 'librarian', '--every', '8'], { env, stdio: ['ignore', 'inherit', 'inherit'] });

  await O.evaluate((t) => window.__den.say('general', `release ${t}: we deploy on friday after the audit`), tag);
  await O.evaluate(([r, t]) => window.__den.say(r, `ops ${t}: the vault keys rotate in november`), [other.id, tag]);
  await M.evaluate(() => window.__den.say('general', 'lunch at noon?'));
  await new Promise((r) => setTimeout(r, 6000));

  const a1 = await ask(M, 'general', `@librarian deploy ${tag}`);
  ok(a1 && a1.includes('friday after the audit'), `it answers a member in their room (${a1?.slice(0, 90)})`);

  const a2 = await ask(M, 'general', `@librarian everywhere vault ${tag}`);
  ok(a2 && !a2.includes('november') && /Only my owner can search every room/.test(a2), `a member gets nothing from another room (${a2?.replace(/\n/g, ' / ').slice(0, 120)})`);

  const a3 = await ask(O, 'general', `@librarian everywhere vault ${tag} please`);
  ok(a3 && a3.includes('november') && a3.includes(`#${other.id}`), `its owner may search every room (${a3?.slice(0, 90)})`);

  // A deletion reaches the index at the next pass over the rooms.
  const id = await O.evaluate(async (t) => (await window.__den.log('general')).find((m) => m.text?.includes(`release ${t}`))?.id, tag);
  await O.evaluate((x) => window.__den.remove('general', x), id);
  await new Promise((r) => setTimeout(r, 20000));
  const a4 = await ask(M, 'general', `@librarian deploy ${tag} again`);
  ok(a4 && !a4.includes('friday after the audit') && /Nothing I hold matches/.test(a4), `a deleted message is forgotten (${a4?.slice(0, 90)})`);
} catch (e) {
  ok(false, `stopped: ${e.message}`);
} finally {
  lib?.kill();
  try { execFileSync(process.execPath, [AGENT_BIN, 'down', '--agent', 'librarian'], { env, timeout: 30000 }); } catch { /* not up */ }
  await b.close();
  if (process.env.KEEP_HOME) console.log(`kept ${HOME}`); else rmSync(HOME, { recursive: true, force: true });
}
