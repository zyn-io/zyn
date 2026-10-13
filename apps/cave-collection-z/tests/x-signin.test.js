import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { DatabaseSync } from 'node:sqlite';
import { test } from 'node:test';
import { link } from '../functions/api/whitelist/x/callback.js';
import { onRequestGet as callback } from '../functions/api/whitelist/x/callback.js';
import { onRequestGet as start } from '../functions/api/whitelist/x/start.js';

// X sign-in for the whitelist (§187.8), against the members table as the
// migrations make it, in a real SQLite, with X itself stubbed.
async function members() {
  const db = new DatabaseSync(':memory:');
  for (const f of ['0001_members.sql', '0002_members_x.sql']) db.exec(await readFile(new URL(`../migrations/${f}`, import.meta.url), 'utf8'));
  // The D1 surface the functions use: prepare().bind().first() / .run().
  return {
    raw: db,
    prepare: (sql) => ({
      bind: (...args) => ({
        first: async () => db.prepare(sql).get(...args) ?? null,
        run: async () => ({ meta: { changes: db.prepare(sql).run(...args).changes } }),
      }),
    }),
  };
}
const kv = (entries = {}) => {
  const m = new Map(Object.entries(entries));
  return { get: async (k) => m.get(k) ?? null, put: async (k, v) => { m.set(k, v); }, delete: async (k) => { m.delete(k); }, m };
};
const A = 'a'.repeat(64);
const B = 'b'.repeat(64);

test('an approved handle links the wallet and pins the X account', async () => {
  const db = await members();
  db.raw.prepare("INSERT INTO members (handle, code_hash, approved_at) VALUES ('letbdu', 'h1', 'now')").run();
  const env = { CAVE_MEMBERS: db, CAVE_WHITELIST: kv() };
  assert.equal(await link(env, { id: '111', handle: 'LetBDU' }, A), 'linked', 'handles match without case');
  assert.deepEqual({ ...db.raw.prepare("SELECT account, x_id FROM members WHERE handle = 'letbdu'").get() }, { account: A, x_id: '111' });
  assert.equal(await link(env, { id: '111', handle: 'letbdu' }, A), 'already');
  assert.equal(await link(env, { id: '111', handle: 'letbdu' }, B), 'other-wallet', 'one member, one wallet');
});

test('a handle given up and taken by someone else links nothing', async () => {
  const db = await members();
  db.raw.prepare("INSERT INTO members (handle, code_hash, approved_at, x_id) VALUES ('pjmazzz', 'h2', 'now', '222')").run();
  const env = { CAVE_MEMBERS: db, CAVE_WHITELIST: kv() };
  assert.equal(await link(env, { id: '999', handle: 'pjmazzz' }, A), 'not-approved');
  // The member who renamed still links, by their id.
  assert.equal(await link(env, { id: '222', handle: 'new_name' }, A), 'linked');
});

test('every applicant is accepted; someone who never applied is not', async () => {
  const db = await members();
  const env = { CAVE_MEMBERS: db, CAVE_WHITELIST: kv({ 'wl:youshang888888': '{"handle":"youshang888888"}' }) };
  assert.equal(await link(env, { id: '333', handle: 'YouShang888888' }, A), 'linked');
  const row = db.raw.prepare("SELECT handle, account, x_id, code_hash FROM members").get();
  assert.equal(row.account, A);
  assert.equal(row.x_id, '333');
  assert.match(row.code_hash, /^x:[0-9a-f]{64}$/, 'no code exists for a sign-in approval, and none can match one');
  assert.equal(await link(env, { id: '444', handle: 'stranger' }, B), 'not-approved');
});

test('a wallet backs one member', async () => {
  const db = await members();
  db.raw.prepare("INSERT INTO members (handle, code_hash, approved_at, account) VALUES ('0xjayn3', 'h3', 'now', ?)").run(A);
  db.raw.prepare("INSERT INTO members (handle, code_hash, approved_at) VALUES ('1guma_', 'h4', 'now')").run();
  const env = { CAVE_MEMBERS: db, CAVE_WHITELIST: kv() };
  assert.equal(await link(env, { id: '555', handle: '1guma_' }, A), 'wallet-used');
});

test('the round trip: state and PKCE, a token, the account, the token revoked', async () => {
  const db = await members();
  db.raw.prepare("INSERT INTO members (handle, code_hash, approved_at) VALUES ('GhezaliEl', 'h5', 'now')").run();
  const store = kv();
  const env = { CAVE_MEMBERS: db, CAVE_WHITELIST: store, X_CLIENT_ID: 'cid', X_CLIENT_SECRET: 'secret' };
  const go = await start({ request: new Request(`https://cavebro.win/api/whitelist/x/start?account=${A}`), env });
  const to = new URL(go.headers.get('location'));
  assert.equal(to.origin + to.pathname, 'https://x.com/i/oauth2/authorize');
  assert.equal(to.searchParams.get('code_challenge_method'), 'S256');
  assert.equal(to.searchParams.get('redirect_uri'), 'https://cavebro.win/api/whitelist/x/callback');
  const state = to.searchParams.get('state');
  assert.match(go.headers.get('set-cookie'), new RegExp(`^cave-xauth=${state}; Path=/api/whitelist/x; Max-Age=600; HttpOnly; Secure; SameSite=Lax$`), 'the state is bound to this browser');
  const cookie = { cookie: `other=1; cave-xauth=${state}` };
  const seen = [];
  const real = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    seen.push(String(url));
    if (String(url).endsWith('/oauth2/token')) {
      const body = new URLSearchParams(init.body);
      assert.equal(init.headers.authorization, `Basic ${btoa('cid:secret')}`);
      assert.ok(body.get('code_verifier'), 'the verifier goes with the code');
      return Response.json({ access_token: 'tok' });
    }
    if (String(url).endsWith('/users/me')) return Response.json({ data: { id: '666', username: 'GhezaliEl' } });
    return Response.json({ revoked: true });
  };
  try {
    // Another browser (an applicant sent this link by someone else) cannot finish it.
    const elsewhere = await callback({ request: new Request(`https://cavebro.win/api/whitelist/x/callback?state=${state}&code=abc`), env });
    assert.equal(elsewhere.headers.get('location'), 'https://cavebro.win/x-done/?r=expired');
    assert.equal(seen.length, 0, 'nothing was asked of X for it');
    const back = await callback({ request: new Request(`https://cavebro.win/api/whitelist/x/callback?state=${state}&code=abc`, { headers: cookie }), env });
    assert.equal(back.headers.get('location'), 'https://cavebro.win/x-done/?r=linked');
    assert.ok(seen.some((u) => u.endsWith('/oauth2/revoke')), 'the token is revoked');
    // The state is spent: the same callback again finds nothing waiting.
    assert.match(back.headers.get('set-cookie'), /^cave-xauth=; .*Max-Age=0/, 'the sign-in is over');
    const again = await callback({ request: new Request(`https://cavebro.win/api/whitelist/x/callback?state=${state}&code=abc`, { headers: cookie }), env });
    assert.equal(again.headers.get('location'), 'https://cavebro.win/x-done/?r=expired');
  } finally {
    globalThis.fetch = real;
  }
  assert.equal(db.raw.prepare("SELECT account FROM members WHERE handle = 'GhezaliEl'").get().account, A);
});

test('without the X app set up, it says so instead of failing', async () => {
  const r = await start({ request: new Request(`https://cavebro.win/api/whitelist/x/start?account=${A}`), env: {} });
  assert.equal(r.headers.get('location'), 'https://cavebro.win/x-done/?r=unavailable');
});
