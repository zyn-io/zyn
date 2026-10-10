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

test('the page reaches only the burn tokens\' chains and ZEC\'s price, and only to read', async () => {
  const headers = await readFile(new URL('../dist/_headers', import.meta.url), 'utf8');
  const connect = headers.match(/connect-src ([^;]+);/)[1].split(' ').sort();
  assert.deepEqual(connect, [
    "'self'", 'https://api.coinbase.com', 'https://api.coingecko.com', 'https://api.devnet.solana.com', 'https://api.kraken.com',
    'https://api.mainnet-beta.solana.com', 'https://ethereum-sepolia-rpc.publicnode.com', 'https://rpc.quicknode.mainnet.arc.io', 'https://zyn-rpc.cavebro.win',
  ].sort());
  const zstore = await readFile(new URL('../zstore.js', import.meta.url), 'utf8');
  const asked = [...zstore.matchAll(/solanaRead\(t, '(\w+)'/g)].map((m) => m[1]).sort();
  // The curve's account for the price; the holder's token accounts and a blockhash to burn.
  assert.deepEqual(asked, ['getAccountInfo', 'getLatestBlockhash', 'getTokenAccountsByOwner'], 'the page reads; the wallet sends');
  const evm = await readFile(new URL('../evm-burn.js', import.meta.url), 'utf8');
  assert.deepEqual([...evm.matchAll(/rpc\(cfg\.rpc, '(\w+)'/g)].map((m) => m[1]), ['eth_call', 'eth_call'], 'balance and pool price, read; the wallet sends');
});

test('the tokens: ZCAVE on Solana and REGI on Arc on mainnet; test stand-ins on testnet', async () => {
  const m = JSON.parse(await readFile(new URL('../burn.json', import.meta.url), 'utf8'));
  assert.deepEqual([m.zcave.chain, m.zcave.cluster, m.zcave.mint, m.zcave.curve], ['solana', 'mainnet', 'GQepeKGNpNatDRMPivFYVsJmfWUsJhtzFhBthCtLpump', '2MLraLvpEq5CXTiJS6ETKhLpmZKUHV2eB5AZwxs2r5rg']);
  assert.deepEqual([m.regi.chain, m.regi.chain_id, m.regi.token.toLowerCase()], ['evm', 5042, '0x93d5b8c53ee763c2c4522bf0d958ce51af4360ae']);
  assert.ok(!m.regi.usd && !m.zcave.zec_per_token, 'mainnet prices from markets, never a fixed number');
  const t = JSON.parse(await readFile(new URL('../burn-testnet.json', import.meta.url), 'utf8'));
  assert.equal(t.zcave.cluster, 'devnet', 'the testnet store burns on Solana devnet');
  assert.equal(t.regi.chain_id, 11155111, 'and on Sepolia');
});

test('the REGI pool slot is keccak(pool id ‖ 6), where Uniswap v4 keeps slot0', async () => {
  const { keccak_256 } = await import('@noble/hashes/sha3');
  const m = JSON.parse(await readFile(new URL('../burn.json', import.meta.url), 'utf8'));
  const six = new Uint8Array(32); six[31] = 6;
  const id = Uint8Array.from(Buffer.from('0530f18eb32d732cc8b067bbd0b2ba7e5d807d4f5cf4f7d74429f2a78d3120c8', 'hex'));
  assert.equal(Buffer.from(keccak_256(new Uint8Array([...id, ...six]))).toString('hex'), m.regi.v4.slot);
});

test('a curve-priced burn: ZEC per token at the virtual reserves, then 5% headroom over the credit', async () => {
  const src = await readFile(new URL('../zstore.js', import.meta.url), 'utf8');
  const body = (name) => src.slice(src.indexOf(name), src.indexOf('\n}\n', src.indexOf(name)) + 2);
  // The ZCAVE curve as Solana served it on 10 Oct 2026.
  const data = new Uint8Array(166);
  const put = (at, v) => { for (let i = 0; i < 8; i++) { data[at + i] = Number(v & 0xffn); v >>= 8n; } };
  put(8, 1007685028645276n); put(16, 290109280n);
  const account = { value: { data: [Buffer.from(data).toString('base64')] } };
  const zecPerToken = new Function('solanaRead', 'evm', 'zecUsd', 'atob', `${body('async function zecPerToken')}; return zecPerToken;`)(async () => account, null, null, (x) => Buffer.from(x, 'base64').toString('binary'));
  const price = await zecPerToken({ chain: 'solana', curve: 'C', decimals: 6, zec_decimals: 8 });
  assert.equal(price, 2878967849n, '2.879e-9 ZEC a ZCAVE');
  const BURN_HEADROOM = 5n;
  const at = src.indexOf('const burnFor = ');
  const burnFor = new Function('BURN_HEADROOM', `${src.slice(at, src.indexOf('\n};\n', at) + 3)} return burnFor;`)(BURN_HEADROOM);
  const ticket = 10n ** 16n; // 0.01 ZEC
  const n = burnFor(ticket, price, 6);
  assert.ok(n * price >= ticket * 10n ** 6n * 105n / 100n, 'at least 5% over the credit, at the price');
  assert.equal(n, 3647141000000n, '3,647,141 ZCAVE for a ticket: rounded up to whole tokens');
  assert.equal(burnFor(ticket, 10n ** 18n, 18), 105n * 10n ** 14n, 'under a thousand, four places: 0.0105');
});

test('a REGI burn is the token\'s transfer to 0x…dEaD with the code appended', async () => {
  const evm = await import('../evm-burn.js');
  const c = '5e'.repeat(32);
  const d = evm.burnData(250000n * 10n ** 18n, c);
  assert.equal(d, `0xa9059cbb${'0'.repeat(24)}000000000000000000000000000000000000dead${(250000n * 10n ** 18n).toString(16).padStart(64, '0')}${c}`);
  assert.equal((d.length - 2) / 2, 4 + 32 * 3, 'exactly one word after the call: what the watcher accepts');
  assert.throws(() => evm.burnData(1n, 'zyn:' + c), /commitment/);
  assert.throws(() => evm.burnData(0n, c), /nothing/);
});
