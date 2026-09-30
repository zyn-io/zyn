// What the toolbar wallet (popup-app.js) reads and does, without a page
// around it: the deployment's networks, what this wallet holds on the object
// ledger and what each thing is called, where ZEC comes in, and the two ways
// value leaves — to another Zyn account, or out to a Zcash address. The same
// rules as the full wallet (app.js), which stays the reference for each.
import { x25519 } from '@noble/curves/ed25519.js';
import { checkedEpoch, hex, parseAmount, unhex } from './core.js';
import { bridgedAsset, claimIntent, decodeMeta, exitCommitment, fund, quoteAlone, redeem, swapIntent, withSlippage } from './market.js';
import { bridgeMemo, bridgedName, call, claimWaiting, loadBridged, loadOwed, loadWaiting, PER_CLAIM, submitFresh, submitIntent } from './ledger.js';
import * as btc from './bitcoin.js';
import { solanaKey } from './solana.js';
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
  let pools = [], epochSeconds = 0;
  try {
    const market = await objectCall(config.rpcUrl, 'zyn_market');
    epochSeconds = Number(market.epoch_seconds) || 0;
    pools = (market.pools || []).filter((p) => ['id', 'a', 'b'].every((k) => /^[0-9a-f]{64}$/.test(p[k]))
      && /^\d+$/.test(String(p.reserve_a)) && /^\d+$/.test(String(p.reserve_b)) && BigInt(p.reserve_a) > 0n && BigInt(p.reserve_b) > 0n
      && Number.isSafeInteger(p.fee_bps));
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

  const sums = new Map();
  for (const h of held) sums.set(h.asset, (sums.get(h.asset) || 0n) + h.amount);
  const collectionName = (id) => (config.collections.find((c) => String(c.id).toLowerCase() === id) || {}).name || `Collection ${id.slice(0, 6)}`;
  const coins = [...sums].filter(([a, v]) => !items.has(a) && v > 0n)
    .map(([asset, amount]) => ({ asset, amount, symbol: names.get(asset) || `${asset.slice(0, 8)}…`, origin: origin.get(asset) || 'Zyn' }));
  if (zec && !coins.some((c) => c.asset === zec)) coins.push({ asset: zec, amount: 0n, symbol: 'ZEC.zy', origin: 'Zcash' });
  coins.sort((a, b) => (a.asset === zec ? -1 : b.asset === zec ? 1 : a.symbol.localeCompare(b.symbol)));
  const owned = [...sums].filter(([a, v]) => items.has(a) && v > 0n)
    .map(([asset]) => ({ asset, collection: items.get(asset).collection, serial: items.get(asset).serial, name: `${collectionName(items.get(asset).collection)} #${items.get(asset).serial}` }));
  // What finished swaps owe this wallet: collected on the next pass.
  const owed = await loadOwed(config, identity).catch(() => []);
  const nameOf = (id) => names.get(id) || `${id.slice(0, 6)}…`;
  return { epoch, coins, items: owned, held, pools, epochSeconds, owed, nameOf };
}

// ---------------------------------------------------------------------------
// Swapping. A swap is an order in the pool's current batch: every order in
// an epoch clears at one price when it seals, and what it pays (the coin
// bought, or the whole input back if the price moved past the minimum)
// waits as a claim. The compact wallet collects those claims itself, so to
// the holder a swap is placed, then arrives.

/// The quote if this were the only order in the batch, and the minimum the
/// order is signed for.
export function quoteSwap(pool, sellA, amountIn, slippageBps) {
  const out = quoteAlone(pool, sellA, amountIn);
  const x = BigInt(pool.reserve_a), y = BigInt(pool.reserve_b);
  const spot = sellA ? (amountIn * y) / x : (amountIn * x) / y;
  const impactBps = spot > 0n ? Number(((spot - out) * 10_000n) / spot) : 0;
  return { out, min: withSlippage(out, slippageBps), impactBps };
}

export async function placeSwap(config, identity, held, pool, sellA, amountIn, slippageBps) {
  const sell = unhex(sellA ? pool.a : pool.b, 32);
  const { min } = quoteSwap(pool, sellA, amountIn, slippageBps);
  if (min <= 0n) throw new Error('Too small to trade');
  const { inputs, change } = fund(identity, held, sell, amountIn);
  await submitFresh(config, identity, (nonce) => swapIntent({ trader: identity.account, pool: unhex(pool.id, 32), sellA, amountIn, minOut: min, inputs, change, nonce }));
}

/// Collect everything the market owes this wallet. Returns how many.
export async function collectOwed(config, identity, owed) {
  let done = 0;
  for (let i = 0; i < owed.length; i += PER_CLAIM) {
    const batch = owed.slice(i, i + PER_CLAIM);
    await submitIntent(config, identity, claimIntent(redeem(identity, batch)));
    done += batch.length;
  }
  return done;
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

// ---------------------------------------------------------------------------
// The bridges (§146, §154, §155): SOL, ETH and BTC in and out as SOL.zy,
// ETH.zy and BTC.zy — the same rules as bridge.html.

/// The bridges a network's config declares, as bridge.html reads them.
export function bridgesOf(net) {
  const b = net.bridges || {};
  const out = [];
  if (b.sol && /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(b.sol.vault || '')) {
    out.push({ ...b.sol, key: 'sol', symbol: 'SOL', zy: 'SOL.zy', chainName: /devnet/.test(b.sol.network) ? 'Solana devnet' : /testnet/.test(b.sol.network) ? 'Solana testnet' : 'Solana', decimals: 9, network: b.sol.network, asset: hex(bridgedAsset(b.sol.network)) });
  }
  if (b.eth && /^0x[0-9a-fA-F]{40}$/.test(b.eth.vault || '') && Number.isSafeInteger(b.eth.chainId)) {
    out.push({ ...b.eth, key: 'eth', symbol: 'ETH', zy: 'ETH.zy', chainName: b.eth.chainId === 1 ? 'Ethereum' : 'Sepolia', decimals: 18, network: `eip155:${b.eth.chainId}`, asset: hex(bridgedAsset(`eip155:${b.eth.chainId}`)) });
  }
  if (b.btc && /^[0-9a-f]{64}$/.test(b.btc.internal || '')) {
    const chain = ['bitcoin', 'testnet4'].includes(b.btc.network) ? b.btc.network : 'signet';
    const network = { bitcoin: 'bitcoin', testnet4: 'bitcoin-test', signet: 'bitcoin-signet' }[chain];
    out.push({ ...b.btc, key: 'btc', symbol: 'BTC', zy: 'BTC.zy', chainName: chain === 'bitcoin' ? 'Bitcoin' : `Bitcoin ${chain}`, decimals: 8, chain, network, asset: hex(bridgedAsset(network)) });
  }
  return out;
}

/// Which bridges this chain has registered, what each has issued, and the fee.
export async function bridgeState(config) {
  const [bridged, market] = await Promise.all([loadBridged(config), objectCall(config.rpcUrl, 'zyn_market').catch(() => ({}))]);
  const bridges = bridgesOf(config).map((b) => {
    const row = bridged.find((x) => x.asset === b.asset);
    return { ...b, live: !!row, issued: /^\d+$/.test(String(row?.issued ?? '')) ? BigInt(row.issued) : null };
  });
  return { bridges, bps: BigInt(Number(market?.fees?.bridge_bps) || 0) };
}

/// Deposits that arrived for this wallet's codes, claimed into it. Returns them.
export async function claimDeposits(config, identity) {
  const waiting = await loadWaiting(config, identity).catch(() => []);
  if (waiting.length) await claimWaiting(config, identity);
  return waiting;
}

/// This wallet's one-time deposit code, for a deposit made elsewhere.
export const depositCode = async (config, identity) => (await bridgeMemo(config, identity)).commitment;

/// A Bitcoin deposit address: the vault key tweaked by this wallet's code,
/// derived here, with the bridge asked to watch it.
export async function btcDepositAddress(config, identity, b) {
  const commitment = await depositCode(config, identity);
  await call(config, 'zyn_depositWatch', { network: b.network, commitment });
  return btc.depositAddress(b.internal, unhex(commitment, 32), b.chain);
}

const LAMPORT = 1_000_000_000n; // one lamport in WAD
const SAT = 10_000_000_000n; // one satoshi in WAD

function destinationBytes(b, text) {
  const t = String(text).trim();
  if (b.key === 'sol') return solanaKey(t);
  if (b.key === 'btc') {
    const script = btc.scriptOf(t, b.chain);
    if (!script) throw new Error(`That is not a ${b.chain === 'bitcoin' ? 'bc1' : 'tb1'}… address (segwit or Taproot)`);
    return script;
  }
  if (!/^0x[0-9a-fA-F]{40}$/.test(t)) throw new Error('That is not an Ethereum address');
  return unhex(t.slice(2).toLowerCase(), 20);
}

// Exits waiting to be revealed — the record bridge.html keeps, shared.
const bridgeExitsKey = (config) => `nap.bridge.exits.${config.chain}`;
const bridgeExits = (config) => { try { return JSON.parse(read(bridgeExitsKey(config)) || '[]'); } catch { return []; } };

export async function revealBridgeExits(config) {
  const left = [];
  for (const x of bridgeExits(config)) {
    try { await call(config, 'zyn_bridgeReveal', { asset: x.asset, address: x.address, salt: x.salt }); }
    catch (e) { if (!/no unpaid exit/.test(e.message)) left.push(x); }
  }
  write(bridgeExitsKey(config), JSON.stringify(left));
}

/// Check an exit before anything is signed: the address and the amount's
/// precision and floor for that chain.
export function checkBridgeOut(b, to, raw) {
  if (b.key === 'sol' && raw % LAMPORT !== 0n) throw new Error('SOL has nine decimal places');
  if (b.key === 'sol' && raw <= 5_000n * LAMPORT) throw new Error('Send more than the 0.000005 SOL network fee');
  if (b.key === 'btc' && raw % SAT !== 0n) throw new Error('BTC has eight decimal places');
  if (b.key === 'btc' && raw < 10_000n * SAT) throw new Error('Send at least 0.0001 BTC; the Bitcoin fee comes out of it');
  return destinationBytes(b, to);
}

/// SOL.zy / ETH.zy / BTC.zy back to their chain: commit to the address,
/// burn, then show the address to the bridge that pays it.
export async function bridgeOut(config, identity, held, b, to, raw, bps) {
  const address = checkBridgeOut(b, to, raw);
  const salt = crypto.getRandomValues(new Uint8Array(32));
  const fee = (raw * bps) / 10_000n;
  const { inputs, change } = fund(identity, held, unhex(b.asset, 32), raw + fee);
  write(bridgeExitsKey(config), JSON.stringify([...bridgeExits(config), { asset: b.asset, address: hex(address), salt: hex(salt) }]));
  await submitIntent(config, identity, withdrawIntent({ holder: identity.account, inputs, asset: unhex(b.asset, 32), amount: raw, destination: exitCommitment(address, salt), outputs: change }));
  await revealBridgeExits(config);
}
