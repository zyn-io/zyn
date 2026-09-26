// What the toolbar wallet (popup-app.js) reads and does, without a page
// around it: the deployment's networks, what this wallet holds on the object
// ledger and what each thing is called, where ZEC comes in, and the two ways
// value leaves — to another Zyn account, or out to a Zcash address. The same
// rules as the full wallet (app.js), which stays the reference for each.
import { x25519 } from '@noble/curves/ed25519.js';
import { checkedEpoch, hex, parseAmount, unhex } from './core.js';
import { decodeMeta, fund } from './market.js';
import { bridgedName, call, loadBridged, submitIntent } from './ledger.js';
import { objectCall, PROGRAM, registerIntent, scan, sealNote, spendIntent, withdrawIntent } from './object.js';
import { endpoint } from './origin.js';

// ---------------------------------------------------------------------------
// Networks.

const NETWORK_KEY = 'nap.network';
const read = (k) => { try { return localStorage.getItem(k); } catch { return null; } };
const write = (k, v) => { try { localStorage.setItem(k, v); } catch { /* this session only */ } };

/// One network, checked as app.js checks it: every endpoint a path on Nap's
/// origin, and the Zcash network named by an address prefix something checks.
function checkedNetwork(value) {
  if (!Number.isInteger(value.chain) || typeof value.network !== 'string') throw new Error('Deployment configuration is incomplete');
  if (typeof value.zcash !== 'string' || !/^u(test)?1$/.test(String(value.zcashPrefix || ''))) throw new Error('Each network must name its Zcash network');
  const out = { ...value, collections: Array.isArray(value.collections) ? value.collections : [] };
  for (const key of ['rpcUrl', 'claimUrl', 'mediaGateway', 'lightdUrl', 'attendUrl']) {
    if ((key === 'lightdUrl' || key === 'attendUrl') && !value[key]) continue;
    out[key] = endpoint(value[key], key);
  }
  return out;
}

export async function loadConfig() {
  const response = await fetch('/config.json', { cache: 'no-store' });
  if (!response.ok) throw new Error('config.json is missing');
  const value = await response.json();
  const networks = (Array.isArray(value.networks) && value.networks.length ? value.networks : [value])
    .map((n) => ({ id: String(n.id || n.chain), ...checkedNetwork(n) }));
  const chosen = networks.find((n) => n.id === read(NETWORK_KEY)) || networks[0];
  return { ...chosen, networks };
}

/// The same choice the full wallet remembers, so both open on one network.
export function chooseNetwork(config, id) {
  const next = config.networks.find((n) => n.id === id);
  if (!next) return config;
  write(NETWORK_KEY, next.id);
  return { ...next, networks: config.networks };
}

// ---------------------------------------------------------------------------
// Holdings.

/// Everything this wallet holds on the ledger, summed per asset and named.
/// The ledger says nothing about who owns what: the notes are opened here,
/// from the wallet's own keys (see app.js `refreshObject`).
export async function loadPocket(config, identity) {
  const epoch = checkedEpoch(await call(config, 'zyn_status'), config.chain);
  if (!identity.viewing || !identity.nullifier) return { epoch, coins: [], items: [], held: [], old: true };
  const published = await call(config, 'zyn_published');
  const { held } = scan(published, { secret: identity.viewing, nkey: identity.nullifier, owner: identity.account });
  const zec = config.zecAsset;
  const others = [...new Set(held.map((h) => h.asset))].filter((a) => a !== zec);

  const items = new Map();
  if (others.length) {
    const found = await objectCall(config.rpcUrl, 'zyn_items', { assets: others.slice(0, 500).join(',') }).catch(() => []);
    for (const i of Array.isArray(found) ? found : []) {
      if (/^[0-9a-f]{64}$/.test(i.asset) && /^[0-9a-f]{64}$/.test(i.collection) && Number.isSafeInteger(i.serial)) items.set(i.asset, i);
    }
  }
  // Names nobody signed are not invented: a launchpad token by its ticker, a
  // bridged coin by its origin, a pool share by its pair, anything else by id.
  const names = new Map([[zec, 'ZEC.zy']]);
  const origin = new Map([[zec, 'Zcash']]);
  if (others.some((a) => !items.has(a))) {
    try {
      const market = await objectCall(config.rpcUrl, 'zyn_market');
      for (const c of market.curves || []) {
        const m = /^[0-9a-f]{64}$/.test(c.token) && typeof c.meta === 'string' ? decodeMeta(c.meta) : null;
        if (m) { names.set(c.token, m.symbol); origin.set(c.token, 'Launchpad'); }
      }
      for (const b of await loadBridged(config)) {
        names.set(b.asset, bridgedName(config, b));
        origin.set(b.asset, ({ 'eip155:1': 'Ethereum', 'eip155:11155111': 'Sepolia', 'solana-devnet': 'Solana devnet', 'bitcoin-signet': 'Bitcoin signet', bitcoin: 'Bitcoin' })[b.network] || b.network);
      }
      const nameOf = (id) => names.get(id) || `${id.slice(0, 6)}…`;
      for (const p of market.pools || []) {
        if (/^[0-9a-f]{64}$/.test(p.lp)) { names.set(p.lp, `LP ${nameOf(p.a)}/${nameOf(p.b)}`); origin.set(p.lp, 'Pool share'); }
      }
    } catch { /* names only */ }
  }

  const sums = new Map();
  for (const h of held) sums.set(h.asset, (sums.get(h.asset) || 0n) + h.amount);
  const collectionName = (id) => (config.collections.find((c) => String(c.id).toLowerCase() === id) || {}).name || `Collection ${id.slice(0, 6)}`;
  const coins = [...sums].filter(([a, v]) => !items.has(a) && v > 0n)
    .map(([asset, amount]) => ({ asset, amount, symbol: names.get(asset) || `${asset.slice(0, 8)}…`, origin: origin.get(asset) || 'Zyn' }));
  if (zec && !coins.some((c) => c.asset === zec)) coins.push({ asset: zec, amount: 0n, symbol: 'ZEC.zy', origin: 'Zcash' });
  coins.sort((a, b) => (a.asset === zec ? -1 : b.asset === zec ? 1 : a.symbol.localeCompare(b.symbol)));
  const owned = [...sums].filter(([a, v]) => items.has(a) && v > 0n)
    .map(([asset]) => ({ asset, collection: items.get(asset).collection, serial: items.get(asset).serial, name: `${collectionName(items.get(asset).collection)} #${items.get(asset).serial}` }));
  return { epoch, coins, items: owned, held };
}

// ---------------------------------------------------------------------------
// Coming in.

/// This wallet's ZEC.zy deposit address, publishing where it is paid first if
/// it never has. An address from the wrong Zcash network is refused outright.
export async function depositAddress(config, identity) {
  let r = await objectCall(config.rpcUrl, 'zyn_depositAddress', { account: identity.accountHex });
  if (r?.registered === false && identity.viewing) {
    await submitIntent(config, identity, registerIntent(identity.account, x25519.getPublicKey(identity.viewing)));
    r = await objectCall(config.rpcUrl, 'zyn_depositAddress', { account: identity.accountHex });
  }
  const address = String(r?.address || '');
  if (!address.startsWith(config.zcashPrefix)) throw new Error(`The node answered with an address that is not ${config.zcash}`);
  return address;
}

// ---------------------------------------------------------------------------
// Going out.

/// Who a recipient is, decided before anything is signed: a Zyn account, a
/// Zcash address that belongs to one (paid on Zyn), or an outside Zcash
/// address (only ZEC.zy may go there, as a withdrawal).
export async function resolveRecipient(config, identity, raw, { isZec, isItem }) {
  const text = String(raw).trim();
  let to = null, external = null;
  if (/^[0-9a-f]{64}$/i.test(text)) to = text.toLowerCase();
  else if (text.startsWith(config.zcashPrefix) || /^t[a-z0-9]{20,}$/i.test(text)) {
    const r = await objectCall(config.rpcUrl, 'zyn_resolveAddress', { address: text });
    if (r && /^[0-9a-f]{64}$/.test(String(r.account))) to = r.account;
    else external = text;
  } else throw new Error(`That is neither a Zyn account nor a ${config.zcash} address`);
  if (external && !isZec) throw new Error(isItem ? 'An item cannot leave Zyn. Send it to a Zyn account.' : 'Only ZEC.zy can be paid to an outside Zcash address.');
  if (to && (/^0{64}$/.test(to) || to === identity.accountHex)) throw new Error('Choose a different recipient');
  return { to, external };
}

/// Pay another Zyn account: retire notes this wallet holds and make one
/// sealed to the key the recipient published, with change back to this one.
export async function payOnZyn(config, identity, held, assetHex, amount, toHex) {
  const { viewing } = await objectCall(config.rpcUrl, 'zyn_viewingKey', { account: toHex });
  if (!/^[0-9a-f]{64}$/.test(String(viewing || ''))) throw new Error('That account has not published where it is paid yet');
  const asset = unhex(assetHex, 32);
  const { inputs, change } = fund(identity, held, asset, amount);
  const note = { asset, amount, owner: unhex(toHex, 32), nonce: crypto.getRandomValues(new Uint8Array(32)) };
  const enc = sealNote(unhex(viewing, 32), crypto.getRandomValues(new Uint8Array(32)), note);
  return submitIntent(config, identity, spendIntent(identity.account, PROGRAM.privateTransfer, inputs, [{ note, enc }, ...change]));
}

// Exits waiting to be told where they go — the same record app.js keeps.
const exitsKey = (config, identity) => `nap.exits.${config.chain}.${identity.accountHex}`;
const pendingExits = (config, identity) => { try { return JSON.parse(read(exitsKey(config, identity)) || '[]'); } catch { return []; } };

export async function revealExits(config, identity) {
  const left = [];
  for (const x of pendingExits(config, identity)) {
    try { await objectCall(config.rpcUrl, 'zyn_exitReveal', { address: x.address, salt: x.salt }); }
    catch (e) { if (!/no unpaid exit/.test(e.message)) left.push(x); }
  }
  write(exitsKey(config, identity), JSON.stringify(left));
}

/// ZEC.zy out to a Zcash address: commit to the address, burn, then reveal
/// it to the vault. The bridge fee rides on top; the Zcash fee comes out.
export async function withdrawZec(config, identity, held, address, amountText) {
  const raw = parseAmount(amountText);
  if (raw % 10_000_000_000n !== 0n) throw new Error('ZEC has eight decimal places');
  if (raw < 100_000n * 10_000_000_000n) throw new Error('Withdraw at least 0.001 ZEC — the Zcash fee comes out of it');
  const salt = hex(crypto.getRandomValues(new Uint8Array(32)));
  const { destination } = await objectCall(config.rpcUrl, 'zyn_exitCommitment', { address, salt });
  if (!/^[0-9a-f]{64}$/.test(String(destination))) throw new Error('The node did not return a commitment');
  const market = await objectCall(config.rpcUrl, 'zyn_market');
  const fee = (raw * BigInt(Number(market?.fees?.bridge_bps) || 0)) / 10_000n;
  const zec = unhex(config.zecAsset, 32);
  const { inputs, change } = fund(identity, held, zec, raw + fee);
  write(exitsKey(config, identity), JSON.stringify([...pendingExits(config, identity), { address, salt }]));
  await submitIntent(config, identity, withdrawIntent({ holder: identity.account, inputs, asset: zec, amount: raw, destination: unhex(destination, 32), outputs: change }));
  await revealExits(config, identity);
}

// ---------------------------------------------------------------------------
// What this wallet did, as this device saw it. The ledger publishes no
// history an account can read back, so the popup keeps its own short log.

const logKey = (config, identity) => `nap.activity.${config.chain}.${identity.accountHex}`;
export const activity = (config, identity) => { try { return JSON.parse(read(logKey(config, identity)) || '[]'); } catch { return []; } };
export function logActivity(config, identity, entry) {
  write(logKey(config, identity), JSON.stringify([{ at: Date.now(), ...entry }, ...activity(config, identity)].slice(0, 50)));
}
