import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { decode, encode } from '../public/shared.mjs';
import { claim, judge, message } from '../lib/check.mjs';
import { jupiterize, P, WRAPPED } from './fixtures/jupiter.mjs';

const MINT = '6dkcEZs83NpCJmWveasCMu3Kmd2XkCzSVmNJJhQBLtx1';
const recorded = JSON.parse(readFileSync(new URL('./fixtures/buy.json', import.meta.url)));
const buy = jupiterize(structuredClone(recorded));
const BUYER = buy.transaction.message.accountKeys[0].pubkey;
const AT = buy.blockTime * 1000 + 60_000;
const opts = { wallet: BUYER, mint: MINT, minLamports: 100_000_000, now: AT };
const edit = (f) => { const t = structuredClone(buy); f(t); return t; };

test('base58 round-trips, leading zeros too', () => {
  for (const s of [BUYER, MINT, buy.transaction.signatures[0], '11Ab']) assert.equal(encode(decode(s)), s);
  assert.equal(decode(BUYER).length, 32);
  assert.throws(() => decode('0OIl'));
});

test('a buy of 0.19 SOL through Jupiter passes, with what went in and came out', () => {
  const r = judge(buy, opts);
  assert.equal(r.ok, true);
  assert.equal(r.lamportsIn, 190_000_000);
  assert.equal(r.tokensIn, 22241383923214n - 18057751453792n);
});
test('a buy that did not go through Jupiter (another app\'s router) is not one from this page', () => assert.equal(judge(recorded, opts).why, 'shape'));
test('SOL sent anywhere but the payer\'s own wrapped-SOL account is refused', () => {
  const t = edit((x) => { x.transaction.message.instructions[3].parsed.info.destination = 'B'.repeat(44); });
  assert.equal(judge(t, opts).why, 'shape');
});
test('tokens slipped in beside the swap (a top-level transfer) are refused', () => {
  for (const type of ['transfer', 'transferChecked']) {
    const t = edit((x) => { x.transaction.message.instructions.splice(6, 0, { program: 'spl-token', programId: P.token, parsed: { type, info: { source: 'S', destination: 'D', amount: '5' } } }); });
    assert.equal(judge(t, opts).why, 'shape', type);
  }
});
test('a program outside the list is refused; the wallet\'s guard and memo are fine', () => {
  assert.equal(judge(edit((x) => { x.transaction.message.instructions.push({ programId: 'Evi1111111111111111111111111111111111111111', accounts: [], data: '' }); }), opts).why, 'shape');
  const guarded = edit((x) => { x.transaction.message.instructions.push({ programId: 'L2TExMFKdjpN9kozasaurPirfHy9P8sbXoAN1qA3S95', accounts: [], data: 'x' }, { program: 'spl-memo', programId: 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr', parsed: 'hi' }); });
  assert.equal(judge(guarded, opts).ok, true);
});
test('no Jupiter route at all is refused', () => assert.equal(judge(edit((x) => { x.transaction.message.instructions.splice(5, 1); }), opts).why, 'shape'));
test('a wrapped-SOL account of someone else does not count', () => {
  const t = edit((x) => { x.transaction.message.instructions[2].parsed.info.wallet = 'C'.repeat(44); });
  assert.equal(judge(t, opts).why, 'shape');
});
test('a closeAccount that sends the SOL elsewhere is refused', () => {
  assert.equal(judge(edit((x) => { x.transaction.message.instructions[6].parsed.info.destination = 'D'.repeat(44); }), opts).why, 'shape');
});
test('no transaction yet', () => assert.equal(judge(null, opts).why, 'missing'));
test('a failed transaction', () => assert.equal(judge(edit((t) => { t.meta.err = { InstructionError: [0, 'x'] }; }), opts).why, 'failed'));
test('an old one', () => assert.equal(judge(buy, { ...opts, now: AT + 20 * 60_000 }).why, 'old'));
test('someone else paid', () => assert.equal(judge(buy, { ...opts, wallet: MINT }).why, 'payer'));
test('under the smallest amount into the swap', () => assert.equal(judge(buy, { ...opts, minLamports: 250_000_000 }).why, 'small'));
test('no EPEAKY came in (a sell)', () => assert.equal(judge(edit((t) => { t.meta.postTokenBalances = t.meta.preTokenBalances; }), opts).why, 'token'));

// A wallet of our own, so the message signature is real.
const key = await crypto.subtle.generateKey({ name: 'Ed25519' }, true, ['sign', 'verify']);
const WALLET = encode(new Uint8Array(await crypto.subtle.exportKey('raw', key.publicKey)));
const mine = edit((t) => {
  t.transaction.message.accountKeys[0].pubkey = WALLET;
  for (const b of [...t.meta.preTokenBalances, ...t.meta.postTokenBalances]) if (b.owner === BUYER) b.owner = WALLET;
  jupiterize(t);
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
    // Jupiter's quote for the SOL that went in: here, about what this buy got.
    quote: async () => 4_000_000_000_000,
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

// ---- Moving a bought access to a new device --------------------------------

import { move } from '../lib/check.mjs';
import { moveMessage } from '../public/shared.mjs';

const DAY = 24 * 3600_000;
// The claims row as D1 holds it: take and restore are conditional updates,
// as their SQL is, so only one of two moves at once can take the row.
function moves(row, log = []) {
  return {
    now: () => AT,
    latest: async (w) => (w === WALLET && row ? { ...row } : null),
    take: async (s, from, to, at, before) => {
      log.push(['take', s, from, to]);
      if (row.signature !== s || row.pass !== from || (row.moved_at !== null && row.moved_at >= before)) return false;
      Object.assign(row, { pass: to, moved_at: at });
      return true;
    },
    restore: async (s, from, to, movedAt) => { log.push(['restore', s, to]); if (row.pass === from) Object.assign(row, { pass: to, moved_at: movedAt }); },
    unregister: async (pass) => { log.push(['unregister', pass]); return { ok: true }; },
    register: async (pass) => { log.push(['register', pass]); await new Promise((r) => setTimeout(r, 5)); return { ok: true }; },
    row, log,
  };
}
const NEW = 'ef'.repeat(32);

test('the wallet that bought moves its access: old pass out, new pass in', async () => {
  const d = moves({ signature: SIG, pass: PASS, moved_at: null });
  const r = await move({ pass: NEW, wallet: WALLET, at: AT, signed: await sign(moveMessage(NEW, WALLET, AT)) }, d);
  assert.deepEqual(r, { ok: true });
  assert.deepEqual(d.log, [['take', SIG, PASS, NEW], ['register', NEW], ['unregister', PASS]]);
});

test('a wallet that never bought has nothing to move', async () => {
  const d = { ...moves(null), latest: async () => null };
  assert.equal((await move({ pass: NEW, wallet: WALLET, at: AT, signed: await sign(moveMessage(NEW, WALLET, AT)) }, d)).why, 'none');
});

test('another wallet cannot move it; an old signature cannot be sent again', async () => {
  const d = moves({ signature: SIG, pass: PASS, moved_at: null });
  assert.equal((await move({ pass: NEW, wallet: WALLET, at: AT, signed: await sign(moveMessage('00'.repeat(32), WALLET, AT)) }, d)).why, 'signed');
  const old = AT - 10 * 60_000;
  assert.equal((await move({ pass: NEW, wallet: WALLET, at: old, signed: await sign(moveMessage(NEW, WALLET, old)) }, d)).why, 'stale');
  assert.deepEqual(d.log, []);
});

test('once a day: a second move within 24 hours waits', async () => {
  const d = moves({ signature: SIG, pass: PASS, moved_at: AT - DAY / 2 });
  const r = await move({ pass: NEW, wallet: WALLET, at: AT, signed: await sign(moveMessage(NEW, WALLET, AT)) }, d);
  assert.equal(r.why, 'soon');
  assert.equal(r.next, AT + DAY / 2);
  assert.deepEqual(d.log, []);
});

test('moving to the device that already has it changes nothing', async () => {
  const d = moves({ signature: SIG, pass: NEW, moved_at: null });
  assert.deepEqual(await move({ pass: NEW, wallet: WALLET, at: AT, signed: await sign(moveMessage(NEW, WALLET, AT)) }, d), { ok: true });
  assert.deepEqual(d.log, []);
});

test('the den failing to take the new pass puts the record back, and the old device keeps its access', async () => {
  const d = { ...moves({ signature: SIG, pass: PASS, moved_at: null }), register: async () => ({ error: 503 }) };
  const r = await move({ pass: NEW, wallet: WALLET, at: AT, signed: await sign(moveMessage(NEW, WALLET, AT)) }, d);
  assert.equal(r.why, 'den');
  assert.deepEqual(d.log, [['take', SIG, PASS, NEW], ['restore', SIG, PASS]]);
  assert.equal(d.row.pass, PASS);
  assert.ok(!d.log.some(([k]) => k === 'unregister'), 'nobody is taken out');
});

test('two moves at once: one wins, the other waits; one buy never lets in two new devices', async () => {
  const d = moves({ signature: SIG, pass: PASS, moved_at: null });
  const B = '12'.repeat(32);
  const [r1, r2] = await Promise.all([
    move({ pass: NEW, wallet: WALLET, at: AT, signed: await sign(moveMessage(NEW, WALLET, AT)) }, d),
    move({ pass: B, wallet: WALLET, at: AT, signed: await sign(moveMessage(B, WALLET, AT)) }, d),
  ]);
  assert.deepEqual([r1.ok, r2.ok].sort(), [false, true]);
  assert.equal((r1.ok ? r2 : r1).why, 'soon');
  assert.equal(d.log.filter(([k]) => k === 'register').length, 1, 'only the winner is registered');
});

test('a failed unregister is tried again, and said, not ignored', async () => {
  let tries = 0;
  const d = { ...moves({ signature: SIG, pass: PASS, moved_at: null }), unregister: async () => { tries++; return { error: 503 }; } };
  const r = await move({ pass: NEW, wallet: WALLET, at: AT, signed: await sign(moveMessage(NEW, WALLET, AT)) }, d);
  assert.equal(r.ok, true, 'the new device is in');
  assert.equal(tries, 2);
  assert.equal(r.left, PASS, 'the pass still to take out, for the log');
});

test('a buy that got far fewer $EPEAKY than the market gives is refused; a failed quote is a retry', async () => {
  const cheap = { ...deps(), quote: async () => 9_000_000_000_000 };
  const r = await claim({ pass: PASS, signature: SIG, wallet: WALLET, signed: await sign(message(PASS, SIG)) }, cheap);
  assert.equal(r.why, 'price');
  assert.deepEqual(cheap.registered, []);
  const down = { ...deps(), quote: async () => { throw new Error('jupiter'); } };
  const q = await claim({ pass: PASS, signature: SIG, wallet: WALLET, signed: await sign(message(PASS, SIG)) }, down);
  assert.equal(q.why, 'quote');
  assert.ok(q.status >= 500, 'the page offers Check again');
});
