// The whitelist's invitation codes (§74.13, §168), against the real schema in
// SQLite: an approved handle's code links one Zyn account, once; one account
// backs one member; only the code's hash is stored.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { DatabaseSync } from 'node:sqlite';

const { onRequestPost } = await import('../functions/api/whitelist/bind.js');
const hash = (c) => createHash('sha256').update(`cave-invite:${c}`).digest('hex');

/// D1's API over an in-memory SQLite with the migration applied.
function members() {
  const db = new DatabaseSync(':memory:');
  db.exec(readFileSync(new URL('../migrations/0001_members.sql', import.meta.url), 'utf8'));
  const d1 = {
    db,
    prepare: (sql) => ({
      bind: (...args) => ({
        first: async () => db.prepare(sql).get(...args) ?? null,
        run: async () => { const r = db.prepare(sql).run(...args); return { meta: { changes: Number(r.changes) } }; },
      }),
    }),
  };
  return d1;
}
function kv() {
  const m = new Map();
  return { get: async (k) => m.get(k) ?? null, put: async (k, v) => { m.set(k, v); } };
}
const post = (env, body, ip = '1.2.3.4') => onRequestPost({ env, request: new Request('https://cavebro.win/api/whitelist/bind', { method: 'POST', headers: { 'content-type': 'application/json', 'cf-connecting-ip': ip }, body: JSON.stringify(body) }) }).then(async (r) => ({ status: r.status, ...(await r.json()) }));
const approve = (env, handle, code) => env.CAVE_MEMBERS.db.prepare('INSERT INTO members (handle, code_hash, approved_at) VALUES (?, ?, ?)').run(handle, hash(code), '2026-10-06');

test('a code links one account, once, and only its hash is stored', async () => {
  const env = { CAVE_MEMBERS: members(), CAVE_WHITELIST: kv() };
  approve(env, 'mole', 'ABCDEFGHJKMN');
  const a = 'aa'.repeat(32), b = 'bb'.repeat(32);
  assert.equal((await post(env, { code: 'zzzz-zzzz-zzzz', account: a })).status, 404, 'an unknown code');
  assert.equal((await post(env, { code: 'abcd-efgh-jkmn', account: 'nope' })).status, 400, 'no Nap, no link');
  assert.equal((await post(env, { code: 'abcd-efgh-jkmn', account: a })).ok, true, 'typed with dashes and in lower case');
  const row = env.CAVE_MEMBERS.db.prepare('SELECT * FROM members').get();
  assert.equal(row.account, a);
  assert.ok(row.linked_at && !row.listed_at);
  assert.equal((await post(env, { code: 'ABCDEFGHJKMN', account: a })).already, true, 'the same account again is fine');
  assert.equal((await post(env, { code: 'ABCDEFGHJKMN', account: b })).status, 409, 'another account is refused');
  assert.ok(!JSON.stringify(env.CAVE_MEMBERS.db.prepare('SELECT * FROM members').all()).includes('ABCDEFGHJKMN'), 'the code itself is stored nowhere');
});

test('one wallet backs one member', async () => {
  const env = { CAVE_MEMBERS: members(), CAVE_WHITELIST: kv() };
  approve(env, 'mole', 'MOLEMOLEMOLE');
  approve(env, 'owl', 'OWLOWLOWLOWL');
  const a = 'aa'.repeat(32);
  assert.equal((await post(env, { code: 'MOLEMOLEMOLE', account: a })).ok, true);
  const second = await post(env, { code: 'OWLOWLOWLOWL', account: a });
  assert.equal(second.status, 409);
  assert.match(second.error, /already linked to another invitation/);
});

test('guessing codes is slowed per address', async () => {
  const env = { CAVE_MEMBERS: members(), CAVE_WHITELIST: kv() };
  let last;
  for (let i = 0; i < 21; i++) last = await post(env, { code: `GUESS${i}GUESS`, account: 'aa'.repeat(32) }, '9.9.9.9');
  assert.equal(last.status, 429);
});
