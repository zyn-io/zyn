// Bringing a coin in (DECISIONS §178): what Jade sends is checked against
// the vaults it pins, the amount covers what the account lacks, and the
// wait collects until the account holds enough.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const store = new Map();
globalThis.localStorage = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) };
const fund = await import('../public/fund.js');

const CODE = 'ab'.repeat(32);
const word = (v) => BigInt(v).toString(16).padStart(64, '0');

test('the page ships the same Solana deposit layout as Nap', async () => {
  const src = await readFile(new URL('../public/solana.js', import.meta.url), 'utf8');
  const nap = await readFile(new URL('../../nap-web/src/solana.js', import.meta.url), 'utf8').catch(() => null);
  if (nap) assert.equal(src, nap, 'apps/jade/public/solana.js is apps/nap-web/src/solana.js');
});

test("the vaults are the ones Zyn Lend's bridge page pins", async () => {
  const lend = await readFile(new URL('../../../../zyn-lend/app/public/bridge.js', import.meta.url), 'utf8').catch(() => null);
  if (!lend) return;
  for (const net of ['testnet', 'mainnet']) {
    const v = fund.VAULTS[net];
    for (const a of [v.sol.vault, v.eth.vault, v.bold.token]) assert.ok(lend.includes(a), `${net}: ${a} is in Lend's bridge page`);
  }
});

test('an amount is brought in whole chain units, never less than lacking', () => {
  assert.equal(fund.chainUnits(1n, 9), 1n);
  assert.equal(fund.chainUnits(550_000_000_000_000_000n, 9), 550_000_000n);
  assert.equal(fund.chainUnits(550_000_000_000_000_001n, 9), 550_000_001n);
  assert.equal(fund.chainUnits(50_000_000_000_000_000n, 8), 5_000_000n);
  assert.equal(fund.chainUnits(123n, 18), 123n);
  assert.equal(fund.unitsText(550_000_000n, 9), '0.55');
  assert.equal(fund.unitsText(2_000_000_000n, 9), '2');
  assert.equal(fund.unitsText(5_000_000n, 8), '0.05');
});

test('what is sent arrives as at least what is lacking, after the bridge fee', () => {
  // 0.02 SOL sent at 50 bps arrived as 0.0199 SOL.zy on testnet (9 Oct): the
  // fee rides on the deposit, so the deposit is grossed up for it.
  const net = (units, bps, decimals) => { const g = units * 10n ** BigInt(18 - decimals); return g - (g * BigInt(bps)) / 10_000n; };
  for (const [short, bps, decimals] of [[2n * 10n ** 16n, 50, 9], [5n * 10n ** 16n, 50, 8], [1n, 50, 18], [10n ** 18n, 0, 9], [123_456_789_012_345_678n, 100, 9], [67n * 10n ** 18n, 50, 18]]) {
    const u = fund.grossUnits(short, bps, decimals);
    assert.ok(net(u, bps, decimals) >= short, `${short} at ${bps} bps arrives whole`);
    assert.ok(u === 1n || net(u - 1n, bps, decimals) < short, `${short} at ${bps} bps sends no more than it must`);
  }
  assert.equal(fund.grossUnits(2n * 10n ** 16n, 50, 9), 20_100_503n);
});

test("Nap's answer must name the pinned vault, or nothing is sent", () => {
  const sol = fund.VAULTS.mainnet.sol.vault;
  assert.deepEqual(fund.checkDeposit('mainnet', 'sol', { vault: sol, memo: `zyn:${CODE}` }, 1n), { memo: `zyn:${CODE}` });
  assert.throws(() => fund.checkDeposit('mainnet', 'sol', { vault: fund.VAULTS.testnet.sol.vault, memo: `zyn:${CODE}` }, 1n), /another vault/);
  assert.throws(() => fund.checkDeposit('mainnet', 'sol', { vault: sol, memo: 'zyn:nope' }, 1n), /No deposit code/);

  const eth = fund.VAULTS.mainnet.eth;
  const data = `0x52d29193${CODE}${word(1)}`;
  assert.deepEqual(fund.checkDeposit('mainnet', 'eth', { to: eth.vault.toLowerCase(), chainId: 1, data }, 10n ** 16n).calls, [{ to: eth.vault, data, value: '0x2386f26fc10000' }]);
  assert.throws(() => fund.checkDeposit('mainnet', 'eth', { to: eth.vault, chainId: 11155111, data }, 1n), /another vault/);
  // Another asset id in the calldata would credit something else.
  assert.throws(() => fund.checkDeposit('mainnet', 'eth', { to: eth.vault, chainId: 1, data: `0x52d29193${CODE}${word(2)}` }, 1n), /No deposit code/);

  const bold = fund.VAULTS.mainnet.bold;
  const [approve, deposit] = fund.checkDeposit('mainnet', 'bold', { to: bold.vault, token: bold.token, chainId: 1, assetId: 2, code: CODE }, 67n * 10n ** 18n).calls;
  assert.equal(approve.to, bold.token);
  assert.equal(approve.data, `0x095ea7b3${word(BigInt(bold.vault))}${word(67n * 10n ** 18n)}`);
  assert.equal(deposit.to, bold.vault);
  assert.equal(deposit.data, `0xdedd8ed5${CODE}${word(2)}${word(67n * 10n ** 18n)}`);
  assert.throws(() => fund.checkDeposit('mainnet', 'bold', { to: bold.vault, token: '0x0000000000000000000000000000000000000001', chainId: 1, assetId: 2, code: CODE }, 1n), /another vault/);

  assert.equal(fund.checkDeposit('mainnet', 'zec', { address: 'u1abc' }, 1n).address, 'u1abc');
  assert.throws(() => fund.checkDeposit('mainnet', 'zec', {}, 1n), /no Zcash deposit address/);
});

test('a closed or full bridge takes nothing', () => {
  assert.throws(() => fund.checkOpen({ bridges: [{ key: 'sol', live: false }] }, 'sol', 1n, 9), /not open/);
  assert.throws(() => fund.checkOpen({ bridges: [{ key: 'bold', live: false, waiting: 'vault' }] }, 'bold', 1n, 18), /does not take BOLD/);
  assert.doesNotThrow(() => fund.checkOpen({ bridges: [{ key: 'sol', live: true }] }, 'sol', 10n ** 12n, 9));
  const capped = { bridges: [{ key: 'eth', live: true, capWei: String(10n ** 16n), issued: String(6n * 10n ** 15n) }] };
  assert.doesNotThrow(() => fund.checkOpen(capped, 'eth', 4n * 10n ** 15n, 18));
  assert.throws(() => fund.checkOpen(capped, 'eth', 4n * 10n ** 15n + 1n, 18), /at most 0\.004 ETH more/);
  assert.doesNotThrow(() => fund.checkOpen({ bridges: [] }, 'zec', 1n, 8));
});

test('the wait collects until the account holds enough, and says when Nap is closed', async () => {
  let t = 0, collected = 0, held = 0n, open = true;
  const said = [];
  const run = fund.arrive({
    collect: async () => { collected++; if (collected === 3) held = 5n; },
    enough: async () => held >= 5n,
    canAsk: () => open,
    say: (x) => said.push(x),
    now: () => t,
    sleep: async (ms) => { t += ms; },
  });
  await run;
  assert.equal(collected, 3);

  t = 0; open = false;
  await assert.rejects(fund.arrive({ collect: async () => {}, enough: async () => true, canAsk: () => open, say: (x) => said.push(x), now: () => t, sleep: async (ms) => { t += ms; }, limit: 20_000 }), /Not arrived yet/);
  assert.ok(said.some((x) => /Open Nap's window/.test(x)));
});

test('a deposit on its way is kept per network and account', () => {
  fund.remember('mainnet', 'aa', { asset: 'x'.repeat(64), target: '5', text: '0.5 SOL', at: 1 });
  assert.equal(fund.recall('mainnet', 'aa').text, '0.5 SOL');
  assert.equal(fund.recall('testnet', 'aa'), null);
  assert.equal(fund.recall('mainnet', 'bb'), null);
  fund.forget('mainnet', 'aa');
  assert.equal(fund.recall('mainnet', 'aa'), null);
});
