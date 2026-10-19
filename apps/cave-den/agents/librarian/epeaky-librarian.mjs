#!/usr/bin/env node
// The librarian: an Epeaky agent that remembers. It keeps every message it is
// handed in a full-text index on its own machine, past the den's 30 days and
// the client's last 1000 a room, and answers when someone names it:
//
//   @librarian deploy friday            → the best matches, in this room
//   @librarian everywhere deploy friday → in every room it is in (owner only)
//
// It is a member of its own (its own key, its own invite, its owner), built on
// the epeaky-agent command line, so the den sees nothing new.
//
// What it will not do:
// - Answer from another room than the one asked in, except to its owner.
//   It sits in private rooms too, and words must not leak between them.
// - Keep what was taken back. It asks for edits and deletions with its
//   messages (`next --changes`), each once, however old the message: an edit
//   replaces the words it holds, a deletion drops them.
// - Follow instructions in what it reads. It only searches; a question is
//   words to look for, whoever asks. Questions put to it, and its own
//   answers, are not kept.
//
//   epeaky-librarian --agent librarian
import { execFile } from 'node:child_process';
import { chmodSync, existsSync, realpathSync } from 'node:fs';
import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { DatabaseSync } from 'node:sqlite';

const argv = process.argv.slice(2);
const flag = (name, d = null) => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : d; };
const AGENT = flag('--agent') || process.env.EPEAKY_AGENT;
/// epeaky-agent: EPEAKY_AGENT_BIN, else the package this one depends on, else
/// a checkout's own (../../agent), else the one on PATH.
function agentBin() {
  if (process.env.EPEAKY_AGENT_BIN) return process.env.EPEAKY_AGENT_BIN;
  try { return createRequire(import.meta.url).resolve('epeaky-agent/bin/epeaky-agent.mjs'); } catch { /* not installed beside it */ }
  // A checkout's own agent, never a folder that happens to sit beside node_modules.
  const local = fileURLToPath(new URL('../../agent/bin/epeaky-agent.mjs', import.meta.url));
  return !import.meta.url.includes('/node_modules/') && existsSync(local) ? local : 'epeaky-agent';
}
const BIN = agentBin();
const HITS = 5;
const debug = (s) => { if (process.env.LIBRARIAN_DEBUG) console.error(`${new Date().toISOString().slice(11, 23)} ${s}`); };

/// epeaky-agent for this agent, with --json: its records. The command, its
/// options, then the end of options and the values (see epeaky-mcp), so no
/// room name is read as an option.
export function cli(cmd, opts = [], pos = [], { agent = AGENT, bin = BIN, input } = {}) {
  const [file, pre] = bin.endsWith('.mjs') ? [process.execPath, [bin]] : [bin, []];
  const full = [...pre, cmd, ...opts, '--json', '--agent', agent, '--', ...pos];
  return new Promise((resolve, reject) => {
    const child = execFile(file, full, { timeout: 15 * 60_000, maxBuffer: 64 << 20 }, (err, stdout, stderr) => {
      if (err) return reject(Object.assign(new Error(String(stderr || err.message).trim()), { code: err.code }));
      resolve(String(stdout).split('\n').filter((l) => l.trim()).map((l) => { try { return JSON.parse(l); } catch { return null; } }).filter(Boolean));
    });
    if (input !== undefined) child.stdin.end(input);
  });
}

/// The index, in the agent's own folder, readable only by its owner's account.
export function open(path) {
  const fresh = path !== ':memory:' && !existsSync(path);
  const db = new DatabaseSync(path);
  if (fresh) chmodSync(path, 0o600);
  db.exec(`
    CREATE TABLE IF NOT EXISTS msgs (
      room TEXT NOT NULL, seq INTEGER NOT NULL, id TEXT, at TEXT, kind TEXT, sender TEXT, nick TEXT, text TEXT NOT NULL,
      PRIMARY KEY (room, seq)
    );
    CREATE VIRTUAL TABLE IF NOT EXISTS words USING fts5(text, room UNINDEXED, seq UNINDEXED, tokenize = 'unicode61 remove_diacritics 2');
  `);
  return db;
}

/// Keep a record, or drop it if it was deleted; an edit replaces the words.
/// Neither its own answers nor the questions put to it are kept: they only
/// repeat what it holds, and would come back as answers.
export function keep(db, r) {
  if (r.kind === 'den' || !r.room || !Number.isInteger(r.seq) || r.seq <= 0) return;
  db.prepare('DELETE FROM words WHERE room = ? AND seq = ?').run(r.room, r.seq);
  if (r.deleted || !r.text || r.kind === 'self' || r.mention) {
    db.prepare('DELETE FROM msgs WHERE room = ? AND seq = ?').run(r.room, r.seq);
    return;
  }
  db.prepare('INSERT OR REPLACE INTO msgs (room, seq, id, at, kind, sender, nick, text) VALUES (?, ?, ?, ?, ?, ?, ?, ?)')
    .run(r.room, r.seq, r.id || null, r.at || null, r.kind || null, r.from || null, r.nick || '', r.text);
  db.prepare('INSERT INTO words (text, room, seq) VALUES (?, ?, ?)').run(r.text, r.room, r.seq);
}

/// A question as FTS5 words: each word quoted, so nothing in it is a query
/// operator, joined by AND (every word) or OR (any word).
export function query(text, join = ' ') {
  const words = text.toLowerCase().match(/[\p{L}\p{N}]+/gu) || [];
  return words.slice(0, 12).map((w) => `"${w}"`).join(join);
}

/// The best matches for a question, in `room` or everywhere: those with every
/// word, or, if none has them all ("… please"), those with the most. `not`
/// is the question itself ({room, seq}), which holds its own words.
export function find(db, text, room = null, { not = null, limit = HITS + 1, rooms = null } = {}) {
  const search = (q) => {
    if (!q) return [];
    const sql = `SELECT m.room, m.seq, m.at, m.sender, m.nick, m.text FROM words w JOIN msgs m ON m.room = w.room AND m.seq = w.seq
      WHERE words MATCH ? ${room ? 'AND w.room = ?' : ''} ${not ? 'AND NOT (m.room = ? AND m.seq = ?)' : ''}
      ${rooms ? `AND m.room IN (${rooms.map(() => '?').join(',') || "''"})` : ''} ORDER BY bm25(words) LIMIT ${limit}`;
    return db.prepare(sql).all(q, ...(room ? [room] : []), ...(not ? [not.room, not.seq] : []), ...(rooms || []));
  };
  const all = search(query(text));
  return all.length ? all : search(query(text, ' OR '));
}

/// The question in a message that names the librarian: its words without the
/// name, and whether it asks for every room.
export function question(text, names) {
  let t = ` ${text} `;
  for (const n of names.filter(Boolean)) t = t.replace(new RegExp(`@?${n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}[:,]?`, 'gi'), ' ');
  t = t.trim();
  const everywhere = /^(everywhere|all rooms)\b/i.test(t);
  return { words: t.replace(/^(everywhere|all rooms)\b[:,]?/i, '').trim(), everywhere };
}

/// Words quoted back must not mention anyone: an "@name" in an answer would
/// wake that agent, and two agents answering mentions would ping-pong.
export const quiet = (s) => String(s).replace(/@/g, '@\u2060');

/// One line per hit, short enough to read in a chat.
export function answer(hits, rawWords, everywhere) {
  const words = quiet(rawWords);
  if (!words) return 'Ask me words to look for, e.g. "@librarian deploy friday".';
  if (!hits.length) return `Nothing I hold matches "${words}"${everywhere ? ' in any room' : ' in this room'}.`;
  const day = (at) => (at ? at.slice(0, 10) : '');
  const cut = (s) => (s.length > 140 ? `${s.slice(0, 139)}…` : s);
  return hits.slice(0, HITS).map((h) => `${everywhere ? `#${h.room} ` : ''}${h.seq} · ${day(h.at)} · ${quiet(h.nick || h.sender)}: ${quiet(cut(h.text.replace(/\s+/g, ' ')))}`).join('\n');
}

/// Where an owner's "everywhere" may be answered: a private room with only
/// the owner and this agent in it. Anywhere else the answer, with words from
/// private rooms, would be read by everyone in the room it was asked in.
export function everywhereHere(room) {
  return !!room && room.kind === 'private' && room.members > 0 && room.members <= 2;
}

/// At most `n` questions a minute from one sender: a loop of agents, or a
/// member typing in a hurry, gets silence rather than a flood.
export function limiter(n = 6, ms = 60_000) {
  const seen = new Map();
  return (who, now = Date.now()) => {
    const recent = (seen.get(who) || []).filter((t) => now - t < ms);
    if (recent.length >= n) { seen.set(who, recent); return false; }
    recent.push(now);
    seen.set(who, recent);
    return true;
  };
}

async function main() {
  if (!AGENT) throw new Error('usage: epeaky-librarian --agent NAME (an agent already joined to its den)');
  const home = process.env.EPEAKY_HOME || join(homedir(), '.epeaky');
  const db = open(join(home, 'agents', AGENT, 'librarian.db'));
  const me = (await cli('status'))[0] || {};
  const names = [me.name, me.nick, AGENT].filter(Boolean);
  // At the start, what the agent already holds (an index made anew catches up).
  for (const r of await cli('rooms')) {
    if (!r.id) continue;
    for (const m of await cli('read', ['--since', '0', '--limit', '1000'], [r.id]).catch(() => [])) keep(db, m);
  }
  console.error(`librarian ${AGENT}: up, ${db.prepare('SELECT count(*) AS n FROM msgs').get().n} messages held`);
  const allowed = limiter();
  for (;;) {
    const t0 = Date.now();
    const recs = await cli('next', ['--changes', '--wait', '300']).catch((e) => { console.error(`next: ${e.message}`); return []; });
    for (const r of recs) {
      debug(`got #${r.room} ${r.seq} ${r.change || r.kind} mention=${r.mention} ${String(r.text).slice(0, 40)}`);
      if (r.kind === 'self') continue;
      keep(db, r);
      // An edit or a deletion is taken in, never answered.
      if (r.change || !r.mention) continue;
      if (!allowed(r.from || r.key || r.room)) { debug(`quiet: ${r.from} asks too often`); continue; }
      const q = question(r.text, names);
      // The rooms it is in now: what it left is not searched, and "everywhere"
      // is answered only to its owner, in a private room of the two of them.
      const rooms = await cli('rooms').catch(() => []);
      const here = rooms.find((x) => x.id === r.room);
      const owner = r.authority === 'owner';
      const everywhere = q.everywhere && owner && everywhereHere(here);
      const hits = find(db, q.words, everywhere ? null : r.room, { not: r, rooms: rooms.map((x) => x.id) });
      const reply = answer(hits, q.words, everywhere)
        + (q.everywhere && !owner ? '\n(Only my owner can search every room.)' : '')
        + (q.everywhere && owner && !everywhere ? '\n(Ask me "everywhere" in a private room with just you and me: here, everyone would read it. This answer is from this room only.)' : '');
      await cli('send', ['--reply', String(r.seq)], [r.room, '-'], { input: reply })
        .then(() => console.error(`answered #${r.room} ${r.seq}`), (e) => console.error(`send: ${e.message}`));
    }
    if (recs.length) debug(`pass ${Date.now() - t0} ms, ${recs.length} records`);
  }
}

const isMain = (() => { try { return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url)); } catch { return false; } })();
if (isMain) main().catch((e) => { console.error(e.message); process.exit(1); });
