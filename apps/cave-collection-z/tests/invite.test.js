// The whitelist's invitation codes (§74.13, §168): an approved handle's code
// links one Zyn account, once; the store keeps only the code's hash.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';

const { onRequestPost } = await import('../functions/api/whitelist/bind.js');
const hash = (c) => createHash('sha256').update(`cave-invite:${c}`).digest('hex');

function kv() {
  const m = new Map();
  return { m, get: async (k) => m.get(k) ?? null, put: async (k, v) => { m.set(k, v); } };
}
const post = (env, body, ip = '1.2.3.4') => onRequestPost({ env, request: new Request('https://cavebro.win/api/whitelist/bind', { method: 'POST', headers: { 'content-type': 'application/json', 'cf-connecting-ip': ip }, body: JSON.stringify(body) }) }).then(async (r) => ({ status: r.status, ...(await r.json()) }));

test('a code links one account, once, and is stored only as a hash', async () => {
  const env = { CAVE_WHITELIST: kv() };
  env.CAVE_WHITELIST.m.set(`invite:${hash('ABCDEFGHJKMN')}`, JSON.stringify({ handle: 'mole' }));
  const a = 'aa'.repeat(32), b = 'bb'.repeat(32);
  assert.equal((await post(env, { code: 'zzzz-zzzz-zzzz', account: a })).status, 404, 'an unknown code');
  assert.equal((await post(env, { code: 'abcd-efgh-jkmn', account: 'nope' })).status, 400, 'no Nap, no link');
  const ok = await post(env, { code: 'abcd-efgh-jkmn', account: a });
  assert.equal(ok.ok, true, 'typed with dashes and in lower case');
  assert.equal(JSON.parse(env.CAVE_WHITELIST.m.get(`invite:${hash('ABCDEFGHJKMN')}`)).account, a);
  assert.equal((await post(env, { code: 'ABCDEFGHJKMN', account: a })).already, true, 'the same account again is fine');
  assert.equal((await post(env, { code: 'ABCDEFGHJKMN', account: b })).status, 409, 'another account is refused');
  assert.ok(![...env.CAVE_WHITELIST.m.keys(), ...env.CAVE_WHITELIST.m.values()].some((x) => String(x).includes('ABCDEFGHJKMN')), 'the code itself is stored nowhere');
});

test('guessing codes is slowed per address', async () => {
  const env = { CAVE_WHITELIST: kv() };
  let last;
  for (let i = 0; i < 21; i++) last = await post(env, { code: `GUESS${i}GUESS`, account: 'aa'.repeat(32) }, '9.9.9.9');
  assert.equal(last.status, 429);
});
