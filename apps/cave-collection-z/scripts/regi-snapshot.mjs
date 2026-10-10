// REGI holders onto the Cave whitelist (DECISIONS §181): the snapshot.
//
// Arc's RPC refuses Cloudflare's workers, so the site never asks Arc. This
// runs here instead: it replays REGI's Transfer logs from the token's first
// block to the snapshot block, leaves out the contracts that are not holders
// (pool, hook, fee vault, reward tracker, the burn address), keeps holders at
// or above the minimum, and publishes each as a keyed hash — HMAC-SHA256 under
// a secret the site alone holds (REGI_SALT). A plain hash would not hide
// anyone: the holder list is public, so anyone could hash it and match.
//
//   node scripts/regi-snapshot.mjs stats [--block N]                 holders at each minimum
//   node scripts/regi-snapshot.mjs publish --min 1000 [--block N]     write the keyed hashes to KV
//
// The salt is read from ~/.zyn/cave/regi-salt (made on first use); the same
// value must be the site's REGI_SALT secret:
//   npx wrangler pages secret put REGI_SALT --project-name cave-collection-z < ~/.zyn/cave/regi-salt
import { execFileSync } from 'node:child_process';
import { createHmac, randomBytes } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';

const RPC = process.env.ARC_RPC || 'https://rpc.mainnet.arc.io';
const TOKEN = '0x93d5b8c53ee763c2c4522bf0d958ce51af4360ae';
const TRANSFER = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';
const WAD = 10n ** 18n;
/// Not holders: where tokens sit in contracts, or were sent to be gone.
export const EXCLUDED = new Set([
  '0x0000000000000000000000000000000000000000',
  '0x000000000000000000000000000000000000dead',
  '0x8366a39cc670b4001a1121b8f6a443a643e40951', // Uniswap v4 PoolManager
  '0x779a7f22480db20ed3ed2bb7950b207ce71ae044', // Argus hook
  '0x15b27781d5236c0fff232803597ba36ab9d96464', // fee vault
  '0xe6275eccfc47d88e2bacfc21e0e2606678d73de8', // reward tracker
]);
const STEP = 100_000; // the RPC's widest log range

async function rpc(method, params) {
  // Paced: Arc's public RPC limits how fast one client may ask.
  await new Promise((z) => setTimeout(z, 150));
  for (let i = 0; ; i++) {
    try {
      const r = await fetch(RPC, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) });
      const b = await r.json();
      if (b.error) throw new Error(b.error.message);
      return b.result;
    } catch (e) {
      if (i >= 8 || /range|too large/i.test(e.message)) throw e;
      // A rate limit: back off and ask again.
      await new Promise((z) => setTimeout(z, 1000 * 2 ** Math.min(i, 5)));
    }
  }
}
const hex = (n) => `0x${n.toString(16)}`;

/// The first block with REGI's code, found by halving.
async function deployBlock(to) {
  let lo = 0, hi = to;
  while (lo < hi) {
    const mid = Math.floor((lo + hi) / 2);
    const code = await rpc('eth_getCode', [TOKEN, hex(mid)]);
    if (code && code !== '0x') hi = mid; else lo = mid + 1;
  }
  return lo;
}

/// Balances at `block`, from every Transfer up to it.
export async function balancesAt(block) {
  const from = await deployBlock(block);
  const bal = new Map();
  // The RPC also caps how many logs one answer holds: a busy range is
  // halved until it fits.
  let step = STEP;
  for (let a = from; a <= block;) {
    const b = Math.min(block, a + step - 1);
    let logs;
    try {
      logs = await rpc('eth_getLogs', [{ address: TOKEN, fromBlock: hex(a), toBlock: hex(b), topics: [TRANSFER] }]);
    } catch (e) {
      if (!/range|too large/i.test(e.message) || step <= 1) throw e;
      step = Math.max(1, Math.floor(step / 2));
      continue;
    }
    a = b + 1;
    step = Math.min(STEP, step * 2);
    for (const l of logs) {
      const f = `0x${l.topics[1].slice(26)}`, t = `0x${l.topics[2].slice(26)}`, v = BigInt(l.data);
      bal.set(f, (bal.get(f) || 0n) - v);
      bal.set(t, (bal.get(t) || 0n) + v);
    }
  }
  return { from, bal };
}

const args = process.argv.slice(2);
const opt = (name) => { const i = args.indexOf(`--${name}`); return i >= 0 ? args[i + 1] : null; };
const block = opt('block') ? Number(opt('block')) : parseInt(await rpc('eth_blockNumber', []), 16);
const { from, bal } = await balancesAt(block);
const holders = [...bal].filter(([a, v]) => v > 0n && !EXCLUDED.has(a)).sort((x, y) => (y[1] > x[1] ? 1 : -1));
const supply = BigInt(await rpc('eth_call', [{ to: TOKEN, data: '0x18160ddd' }, hex(block)]));

if (args[0] === 'stats') {
  const held = holders.reduce((t, [, v]) => t + v, 0n);
  console.log(`REGI at block ${block} (first ${from}): ${holders.length} holders outside the excluded contracts, holding ${(held * 10000n / supply) / 100n}% of ${supply / WAD} supply`);
  for (const min of [1n, 100n, 1_000n, 10_000n, 100_000n, 1_000_000n]) {
    console.log(`  ≥ ${min.toLocaleString('en-US').padStart(9)} REGI: ${holders.filter(([, v]) => v >= min * WAD).length}`);
  }
  console.log(`  largest: ${holders.slice(0, 5).map(([, v]) => (v / WAD).toLocaleString('en-US')).join(', ')} REGI`);
} else if (args[0] === 'publish') {
  if (!opt('min')) throw new Error('publish --min <REGI> [--block N]');
  const min = BigInt(opt('min')) * WAD;
  const dir = join(homedir(), '.zyn', 'cave');
  const saltFile = join(dir, 'regi-salt');
  if (!existsSync(saltFile)) { mkdirSync(dir, { recursive: true }); writeFileSync(saltFile, randomBytes(32).toString('hex'), { mode: 0o600 }); console.log(`made ${saltFile}; set it as the site's REGI_SALT secret`); }
  const salt = readFileSync(saltFile, 'utf8').trim();
  const eligible = holders.filter(([, v]) => v >= min).map(([a]) => createHmac('sha256', salt).update(a).digest('hex'));
  // The snapshot itself, so the site can say which one it is.
  const entries = [...eligible.map((m) => ({ key: `regi-ok:${m}`, value: '1' })), { key: 'regi-snapshot', value: JSON.stringify({ block, min: opt('min'), holders: eligible.length, at: new Date().toISOString() }) }];
  const file = join(tmpdir(), `regi-${block}.json`);
  writeFileSync(file, JSON.stringify(entries));
  execFileSync('npx', ['wrangler@4.40.0', 'kv', 'bulk', 'put', file, '--binding', 'CAVE_WHITELIST', '--remote'], { stdio: 'inherit', cwd: new URL('..', import.meta.url) });
  console.log(`published ${eligible.length} keyed holders (≥ ${opt('min')} REGI at block ${block})`);
} else {
  console.log('usage: node scripts/regi-snapshot.mjs stats [--block N] | publish --min <REGI> [--block N]');
}
