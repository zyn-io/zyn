import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { decode, encode } from '../public/shared.mjs';
import { claim, judge, message } from '../lib/check.mjs';

const MINT = '6dkcEZs83NpCJmWveasCMu3Kmd2XkCzSVmNJJhQBLtx1';
const buy = JSON.parse(readFileSync(new URL('./fixtures/buy.json', import.meta.url)));
const BUYER = buy.transaction.message.accountKeys[0].pubkey;
const AT = buy.blockTime * 1000 + 60_000;
const opts = { wallet: BUYER, mint: MINT, minLamports: 100_000_000, now: AT };
const edit = (f) => { const t = structuredClone(buy); f(t); return t; };

test('base58 round-trips, leading zeros too', () => {
  for (const s of [BUYER, MINT, buy.transaction.signatures[0], '11Ab']) assert.equal(encode(decode(s)), s);
  assert.equal(decode(BUYER).length, 32);
  assert.throws(() => decode('0OIl'));
});

test('a real buy of 0.19 SOL passes', () => assert.deepEqual(judge(buy, opts), { ok: true }));
test('no transaction yet', () => assert.equal(judge(null, opts).why, 'missing'));
test('a failed transaction', () => assert.equal(judge(edit((t) => { t.meta.err = { InstructionError: [0, 'x'] }; }), opts).why, 'failed'));
test('an old one', () => assert.equal(judge(buy, { ...opts, now: AT + 20 * 60_000 }).why, 'old'));
test('someone else paid', () => assert.equal(judge(buy, { ...opts, wallet: MINT }).why, 'payer'));
test('under the smallest amount', () => assert.equal(judge(buy, { ...opts, minLamports: 250_000_000 }).why, 'small'));
test('no EPEAKY came in (a sell)', () => assert.equal(judge(edit((t) => { t.meta.postTokenBalances = t.meta.preTokenBalances; }), opts).why, 'token'));

// A wallet of our own, so the message signature is real.
const key = await crypto.subtle.generateKey({ name: 'Ed25519' }, true, ['sign', 'verify']);
const WALLET = encode(new Uint8Array(await crypto.subtle.exportKey('raw', key.publicKey)));
const mine = edit((t) => {
  t.transaction.message.accountKeys[0].pubkey = WALLET;
  for (const b of [...t.meta.preTokenBalances, ...t.meta.postTokenBalances]) if (b.owner === BUYER) b.owner = WALLET;
});
const SIG = buy.transaction.signatures[0];
const PASS = 'ab'.repeat(32);
const sign = async (text) => encode(new Uint8Array(await crypto.subtle.sign('Ed25519', key.privateKey, new TextEncoder().encode(text))));

function deps(rows = new Map(), registered = []) {
  return {
    mint: MINT, minLamports: 100_000_000, now: () => AT,
    getTx: async (s) => (s === SIG ? mine : null),
    record: async (s, pass) => { if (!rows.has(s)) { rows.set(s, pass); return 'new'; } return rows.get(s) === pass ? 'same' : 'other'; },
    register: async (pass) => { registered.push(pass); return { ok: true }; },
    rows, registered,
  };
}

test('the buyer claims with its own signature: the pass is registered', async () => {
  const d = deps();
  assert.deepEqual(await claim({ pass: PASS, signature: SIG, wallet: WALLET, signed: await sign(message(PASS, SIG)) }, d), { ok: true });
  assert.deepEqual(d.registered, [PASS]);
});

test('a signature over another pass is refused, nothing registered', async () => {
  const d = deps();
  const r = await claim({ pass: PASS, signature: SIG, wallet: WALLET, signed: await sign(message('cd'.repeat(32), SIG)) }, d);
  assert.equal(r.why, 'signed');
  assert.deepEqual(d.registered, []);
});

test('someone who saw the buy on chain cannot claim it with their own wallet', async () => {
  const other = await crypto.subtle.generateKey({ name: 'Ed25519' }, true, ['sign', 'verify']);
  const w = encode(new Uint8Array(await crypto.subtle.exportKey('raw', other.publicKey)));
  const s = encode(new Uint8Array(await crypto.subtle.sign('Ed25519', other.privateKey, new TextEncoder().encode(message(PASS, SIG)))));
  assert.equal((await claim({ pass: PASS, signature: SIG, wallet: w, signed: s }, deps())).why, 'payer');
});

test('one buy lets in one device; the same pass may retry', async () => {
  const d = deps();
  const p2 = 'cd'.repeat(32);
  assert.ok((await claim({ pass: PASS, signature: SIG, wallet: WALLET, signed: await sign(message(PASS, SIG)) }, d)).ok);
  assert.ok((await claim({ pass: PASS, signature: SIG, wallet: WALLET, signed: await sign(message(PASS, SIG)) }, d)).ok);
  assert.equal((await claim({ pass: p2, signature: SIG, wallet: WALLET, signed: await sign(message(p2, SIG)) }, d)).why, 'used');
  assert.deepEqual(d.registered, [PASS, PASS]);
});

test('a buy not on chain yet answers 404', async () => {
  const s = '5'.repeat(88);
  const r = await claim({ pass: PASS, signature: s, wallet: WALLET, signed: await sign(message(PASS, s)) }, deps());
  assert.deepEqual([r.why, r.status], ['missing', 404]);
});

test('a malformed pass or signature is refused', async () => {
  assert.equal((await claim({ pass: 'xyz', signature: SIG, wallet: WALLET, signed: 'x' }, deps())).why, 'pass');
  assert.equal((await claim({ pass: PASS, signature: SIG, wallet: WALLET, signed: 'not-b58!' }, deps())).why, 'signed');
});

test('the den refusing is passed on', async () => {
  const d = { ...deps(), register: async () => ({ error: 'closed' }) };
  assert.equal((await claim({ pass: PASS, signature: SIG, wallet: WALLET, signed: await sign(message(PASS, SIG)) }, d)).why, 'den');
});
