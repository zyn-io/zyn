import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as sol from '../solana-burn.js';

// Burn tickets (§145). The layout below was accepted and finalized by Solana
// devnet on 24 Sep 2026 (burn pezJdGmt…6BXUxSD, 3 tickets of the test token),
// so these bytes are pinned rather than re-derived.
const INPUT = {
  owner: 'HBvV7YqSRSPW4YEBsDvpvF2PrUWFubqVbTNYafkddTsy',
  tokenAccount: 'AyaytbBqKXb1EinuiB3aPnBPY6vFs85eJqgYuncinz7h',
  mint: 'FvCnGJ1BGghAbQoMJ4tLwKudpcTVdo45ws7GFxNxRrU9',
  tokenProgram: sol.TOKEN_PROGRAMS[0],
  amount: 3_000_000n,
  decimals: 6,
  memo: `zyn:${'b3'.repeat(32)}`,
  blockhash: 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG',
};

test('base58 round-trips, leading zeros included', () => {
  for (const a of [INPUT.owner, INPUT.mint, sol.MEMO_PROGRAM, '11111111111111111111111111111111']) {
    assert.equal(sol.b58encode(sol.b58decode(a)), a);
  }
  assert.equal(sol.b58decode('11111111111111111111111111111111').length, 32);
});

test('the burn message: one signer, burnChecked from the holder, then the memo', () => {
  const m = sol.burnMessage(INPUT);
  assert.deepEqual([...m.slice(0, 4)], [1, 0, 2, 5], 'one signer, two read-only programs, five keys');
  const keys = Array.from({ length: 5 }, (_, i) => sol.b58encode(m.slice(4 + i * 32, 36 + i * 32)));
  assert.deepEqual(keys, [INPUT.owner, INPUT.tokenAccount, INPUT.mint, INPUT.tokenProgram, sol.MEMO_PROGRAM]);
  const ix = m.slice(4 + 5 * 32 + 32);
  assert.equal(ix[0], 2, 'two instructions');
  assert.deepEqual([...ix.slice(1, 6)], [3, 3, 1, 2, 0], 'token program burns: account, mint, owner');
  assert.deepEqual([...ix.slice(6, 17)], [10, 15, 0xc0, 0xc6, 0x2d, 0, 0, 0, 0, 0, 6], 'burnChecked 3,000,000 at 6 decimals');
  assert.equal(new TextDecoder().decode(ix.slice(20)), INPUT.memo);
  assert.equal(sol.unsigned(m).length, 65 + m.length);
});

test("it burns only with Nap's memo and a real token program", () => {
  assert.throws(() => sol.burnMessage({ ...INPUT, memo: 'hello' }), /memo/);
  assert.throws(() => sol.burnMessage({ ...INPUT, tokenProgram: sol.MEMO_PROGRAM }), /token program/);
});

test('the page may reach Solana only where a burn config says, and only to read', async () => {
  const headers = await readFile(new URL('../dist/_headers', import.meta.url), 'utf8');
  assert.match(headers, /connect-src 'self' https:\/\/zyn-rpc\.cavebro\.win https:\/\/api\.devnet\.solana\.com/);
  const zstore = await readFile(new URL('../zstore.js', import.meta.url), 'utf8');
  const asked = [...zstore.matchAll(/solanaRead\(cfg, '(\w+)'/g)].map((m) => m[1]).sort();
  assert.deepEqual(asked, ['getLatestBlockhash', 'getTokenAccountsByOwner'], 'the page reads; the wallet sends');
});

test('no burn is offered on mainnet until the token exists', async () => {
  const mainnet = await readFile(new URL('../burn.json', import.meta.url), 'utf8').catch(() => null);
  if (mainnet) {
    const c = JSON.parse(mainnet);
    assert.equal(c.cluster, 'mainnet');
    assert.match(c.per_ticket, /^\d+$/);
  }
  const t = JSON.parse(await readFile(new URL('../burn-testnet.json', import.meta.url), 'utf8'));
  assert.equal(t.cluster, 'devnet', 'the testnet store burns on Solana devnet');
});
