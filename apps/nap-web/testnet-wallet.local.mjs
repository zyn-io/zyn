// Testnet wallets for demos and rehearsals, run through Nap's own code.
// Not shipped. Phrases live in ~/.zyn/test-wallets (0600) and are never
// printed; each wallet keeps Nap's nonce counters in its own storage file, so
// no nonce is ever used twice.
//
//   node testnet-wallet.local.mjs new <name>              account id (registers it)
//   node testnet-wallet.local.mjs balances <name>
//   node testnet-wallet.local.mjs owed <name>             what the chain owes it
//   node testnet-wallet.local.mjs collect <name>          collect all of that
//   node testnet-wallet.local.mjs code <name>             a one-time bridge deposit code
//   node testnet-wallet.local.mjs claim <name>            claim bridge deposits that arrived
//   node testnet-wallet.local.mjs call <name> <method> '<json params>'
//        any Nap Connect method (zyn_lendSupply, zyn_lendOpen, zyn_swap, …):
//        prints the approval Nap would show, then runs it.

import { readFileSync, writeFileSync, existsSync, mkdirSync, chmodSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

const DIR = join(homedir(), '.zyn', 'test-wallets');
const [,, cmd, name, ...rest] = process.argv;
if (!cmd || !name || !/^[a-z0-9-]{1,32}$/.test(name)) {
  console.error('usage: node testnet-wallet.local.mjs <new|balances|owed|collect|code|claim|call> <name> …');
  process.exit(2);
}
mkdirSync(DIR, { recursive: true, mode: 0o700 });

// Nap keeps its nonce counters in localStorage; here, a file per wallet.
const storeFile = join(DIR, `${name}.storage.json`);
const store = existsSync(storeFile) ? JSON.parse(readFileSync(storeFile, 'utf8')) : {};
globalThis.localStorage = {
  getItem: (k) => (k in store ? store[k] : null),
  setItem: (k, v) => { store[k] = String(v); writeFileSync(storeFile, JSON.stringify(store, null, 1), { mode: 0o600 }); },
  removeItem: (k) => { delete store[k]; writeFileSync(storeFile, JSON.stringify(store, null, 1), { mode: 0o600 }); },
};

const core = await import('./src/core.js');
const obj = await import('./src/object.js');
const ledger = await import('./src/ledger.js');
const mk = await import('./src/market.js');
const { METHODS } = await import('./src/connect-ops.js');
const { x25519 } = await import('@noble/curves/ed25519.js');

const net = JSON.parse(readFileSync(new URL('./public/config.json', import.meta.url), 'utf8')).networks.find((n) => n.id === 'testnet');
const config = { ...net, id: 'testnet', rpcUrl: 'https://zyn-rpc.cavebro.win/object' };

const phraseFile = join(DIR, `${name}.phrase`);
if (!existsSync(phraseFile)) {
  if (cmd !== 'new') { console.error(`no wallet named ${name}; make it with "new"`); process.exit(1); }
  writeFileSync(phraseFile, core.newMnemonic(), { mode: 0o600 });
  chmodSync(phraseFile, 0o600);
}
const identity = core.deriveIdentity(readFileSync(phraseFile, 'utf8').trim());
const W = 10n ** 18n;
const show = (raw) => { const v = BigInt(raw); const f = (v % W).toString().padStart(18, '0').replace(/0+$/, ''); return `${v / W}${f ? `.${f}` : ''}`; };
const names = { [config.zecAsset]: 'ZEC.zy' };
for (const b of await ledger.loadBridged(config).catch(() => [])) names[b.asset] = ledger.bridgedName(config, b);
{ const mk0 = await import('./src/market.js'); for (const [a, n] of Object.entries({ ...names })) names[obj.hex(mk0.lendAsset(obj.unhex(a)))] = `l${n}`; }
const nameOf = (a) => names[a] || `${a.slice(0, 8)}…`;

switch (cmd) {
  case 'new': {
    const r = await obj.objectCall(config.rpcUrl, 'zyn_depositAddress', { account: identity.accountHex });
    if (!r.registered) await ledger.submitIntent(config, identity, obj.registerIntent(identity.account, x25519.getPublicKey(identity.viewing)));
    console.log(`${name}: account ${identity.accountHex}`);
    break;
  }
  case 'balances': {
    const { balances } = await ledger.loadHoldings(config, identity, []);
    for (const [a, v] of balances) console.log(`${nameOf(a).padEnd(14)} ${show(v)}`);
    if (!balances.size) console.log('(nothing)');
    break;
  }
  case 'owed': {
    for (const o of await ledger.loadOwed(config, identity)) console.log(`${o.kind}: ${show(o.amount)} ${nameOf(o.asset)}`);
    break;
  }
  case 'collect': {
    const owed = await ledger.loadOwed(config, identity);
    if (!owed.length) { console.log('nothing owed'); break; }
    // As Nap's collect does: claims in batches the VM's I/O limit allows.
    for (let i = 0; i < owed.length; i += 16) {
      await ledger.submitIntent(config, identity, mk.claimIntent(mk.redeem(identity, owed.slice(i, i + 16))));
    }
    console.log(`collected ${owed.length} claim(s)`);
    break;
  }
  case 'code': {
    const { memo, commitment } = await ledger.bridgeMemo(config, identity);
    console.log(JSON.stringify({ memo, commitment }));
    break;
  }
  case 'claim': {
    const got = await ledger.loadWaiting(config, identity).catch(() => []);
    if (got.length) await ledger.claimWaiting(config, identity);
    console.log(got.length ? got.map((w) => `claimed ${show(w.amount)} ${nameOf(w.asset)}`).join('\n') : 'no deposits waiting');
    break;
  }
  case 'call': {
    const [method, json] = rest;
    const m = METHODS[method];
    if (!m) { console.error(`no Nap Connect method ${method}`); process.exit(2); }
    const params = json ? JSON.parse(json) : {};
    const ctx = { config, identity, origin: 'testnet-wallet.local' };
    if (m.kind === 'write') {
      const a = await m.prepare(ctx, params);
      console.log(`— ${a.title}${a.danger ? '  [danger]' : ''}`);
      for (const [k, v] of a.rows) console.log(`  ${k}: ${v}`);
      if (a.note) console.log(`  (${a.note})`);
      console.log(JSON.stringify(await a.run()));
    } else {
      console.log(JSON.stringify(await m.run(ctx, params), null, 1));
    }
    break;
  }
  default:
    console.error(`unknown command ${cmd}`);
    process.exit(2);
}
