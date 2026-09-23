// The market, from the browser: pools, batch swaps, liquidity, launches.
//
// Every intent here is encoded exactly as `ObjectVm::encode_intent` reads it,
// and every id exactly as `objectvm::market` derives it. The shared fixture
// (`tests/object-vectors.json`, section `market`) holds both sides to the same
// bytes; a drift is an intent the VM refuses, or runs under the wrong rule.
//
// What the VM owes a trader — swap proceeds, refunds, pool shares, a sale's
// allocation — is a *claim*, and a claim's id is derived from a nonce only the
// trader holds. So the nonce is the receipt: lose it and the claim still
// exists, but nothing points this wallet at it. Nonces are therefore derived
// from the identity key and a counter, not drawn at random, so a restored
// wallet can walk the counter and find everything it is owed again.

import { sha256 } from '@noble/hashes/sha2.js';
import { x25519 } from '@noble/curves/ed25519.js';
import {
  commitment,
  concatBytes,
  hex,
  i128Bytes,
  leaf,
  nullifierOf,
  sealNote,
  u32be,
  u64be,
  unhex,
} from './object.js';

const te = new TextEncoder();
export const WAD = 10n ** 18n;
export const BPS = 10_000n;
export const MINIMUM_LIQUIDITY = 1000n;

/// Program ids, derived here exactly as `zyn-vm/src/program.rs` names them:
/// `leaf("zyn.program.named.v1" ‖ u32 len ‖ name)`. Derived rather than
/// pasted, and checked against the fixture, so neither can drift alone.
const named = (name) => leaf(te.encode('zyn.program.named.v1'), u32be(name.length), te.encode(name));
export const MARKET_PROGRAM = Object.freeze(Object.fromEntries([
  ['ammSwap', 'amm-swap'],
  ['addLiquidity', 'add-liquidity'],
  ['removeLiquidity', 'remove-liquidity'],
  ['claim', 'claim'],
  ['launchCreate', 'launch-create'],
  ['launchCommit', 'launch-commit'],
  ['curveCreate', 'curve-create'],
  ['curveTrade', 'curve-trade'],
].map(([k, name]) => [k, hex(named(name))])));

const TAG = { createPool: 10, addLiquidity: 11, removeLiquidity: 12, swap: 13, claim: 14, createLaunch: 15, commit: 16, createCurve: 23, curveTrade: 24 };

const id = (domain, ...parts) => leaf(te.encode(domain), ...parts);

/// Byte order, as Rust compares `[u8; 32]`.
export function compareBytes(a, b) {
  for (let i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) return a[i] - b[i];
  return a.length - b.length;
}

export const poolId = (a, b) => (compareBytes(a, b) <= 0 ? id('objectvm.pool.v1', a, b) : id('objectvm.pool.v1', b, a));
export const lpAsset = (pool) => id('objectvm.lp.v1', pool);
export const tokenOf = (creator, nonce) => id('objectvm.token.v1', creator, nonce);
export const launchSource = (creator, nonce) => id('objectvm.launch.v1', creator, nonce);
export const orderId = (trader, nonce) => id('objectvm.order.v1', trader, nonce);
export const commitId = (buyer, nonce) => id('objectvm.commit.v1', buyer, nonce);
export const liquidityId = (provider, nonce) => id('objectvm.liquidity.v1', provider, nonce);
export const claimId = (source, k) => id('objectvm.claim.v1', source, new Uint8Array([k]));

function inputs(list) {
  const parts = [u32be(list.length)];
  for (const i of list) parts.push(i.commitment, i.nullifier);
  return concatBytes(...parts);
}

function outputs(list) {
  const parts = [u32be(list.length)];
  for (const o of list) {
    parts.push(o.note.asset, i128Bytes(o.note.amount), o.note.owner, o.note.nonce, o.enc.epk, u32be(o.enc.ciphertext.length), o.enc.ciphertext);
  }
  return concatBytes(...parts);
}

const u32bytes = (n) => u32be(Number(n));
const bool = (b) => new Uint8Array([b ? 1 : 0]);
const prog = (k) => unhex(MARKET_PROGRAM[k]);

export const createPoolIntent = (o) => concatBytes(
  new Uint8Array([TAG.createPool]), o.provider, prog('addLiquidity'), o.assetA, o.assetB,
  i128Bytes(o.amountA), i128Bytes(o.amountB), u32bytes(o.feeBps), o.nonce, inputs(o.inputs), outputs(o.change),
);

export const addLiquidityIntent = (o) => concatBytes(
  new Uint8Array([TAG.addLiquidity]), o.provider, prog('addLiquidity'), o.pool,
  i128Bytes(o.amountA), i128Bytes(o.amountB), i128Bytes(o.minShares), o.nonce, inputs(o.inputs), outputs(o.change),
);

export const removeLiquidityIntent = (o) => concatBytes(
  new Uint8Array([TAG.removeLiquidity]), o.provider, prog('removeLiquidity'), o.pool,
  i128Bytes(o.shares), i128Bytes(o.minA), i128Bytes(o.minB), o.nonce, inputs(o.inputs), outputs(o.change),
);

export const swapIntent = (o) => concatBytes(
  new Uint8Array([TAG.swap]), o.trader, prog('ammSwap'), o.pool, bool(o.sellA),
  i128Bytes(o.amountIn), i128Bytes(o.minOut), o.nonce, inputs(o.inputs), outputs(o.change),
);

export function claimIntent(o) {
  const ids = [u32be(o.claims.length), ...o.claims];
  return concatBytes(new Uint8Array([TAG.claim]), o.owner, prog('claim'), ...ids, outputs(o.outputs));
}

export const createLaunchIntent = (o) => concatBytes(
  new Uint8Array([TAG.createLaunch]), o.creator, prog('launchCreate'), o.nonce, o.quote,
  i128Bytes(o.totalSupply), i128Bytes(o.saleSupply), i128Bytes(o.poolSupply),
  i128Bytes(o.softCap), i128Bytes(o.hardCap), u32bytes(o.poolBps), u32bytes(o.feeBps),
  u64be(o.closesAtEpoch), o.meta,
);

export const commitIntent = (o) => concatBytes(
  new Uint8Array([TAG.commit]), o.buyer, prog('launchCommit'), o.launch,
  i128Bytes(o.amount), o.nonce, inputs(o.inputs), outputs(o.change),
);

// ---------------------------------------------------------------------------
// Bonding curves (objectvm::curve, DECISIONS §141). Every token has the same
// curve: 80,000 lots of 12,500 tokens, lot i costs 7,721 + i zatoshi, and it
// graduates at lot 51,098 into a pool whose shares nobody holds.
// ---------------------------------------------------------------------------

export const CURVE = Object.freeze({
  lots: 80_000,
  lotTokens: 12_500n,
  baseZat: 7_721n,
  slopeZat: 1n,
  graduateAt: 51_098,
  poolFeeBps: 30,
});
const ZAT = 10_000_000_000n; // one zatoshi, in the ledger's units
/// One lot, in the ledger's units (1e18 per token).
export const LOT = CURVE.lotTokens * WAD;
/// The smallest buy: 0.001 of the quote asset.
export const MIN_CURVE_BUY = 100_000n * ZAT;

export const curveSource = (creator, nonce) => id('objectvm.curve.v1', creator, nonce);
export const curveOrderId = (trader, nonce) => id('objectvm.curve-order.v1', trader, nonce);

/// What lots `[q, q+d)` cost, in the ledger's units — `objectvm::curve::cost`.
export function curveCost(q, d) {
  const Q = BigInt(q), D = BigInt(d);
  return (CURVE.baseZat * D + CURVE.slopeZat * ((D * (2n * Q + D - 1n)) / 2n)) * ZAT;
}

/// The most lots `budget` buys from lot `q`, at most `cap`.
export function lotsFor(q, budget, cap) {
  let lo = 0, hi = cap;
  while (lo < hi) {
    const mid = lo + Math.ceil((hi - lo) / 2);
    if (curveCost(q, mid) <= budget) lo = mid; else hi = mid - 1;
  }
  return lo;
}

/// A buy of `amountIn` alone in its batch: lots, what they cost, the fee.
/// Other buyers in the same epoch share the price, so this is what a buy
/// gets at best when nobody else buys — a preview, never a promise.
export function previewCurveBuy(sold, amountIn, feeBps) {
  const budget = (BigInt(amountIn) * BPS) / (BPS + BigInt(feeBps));
  const lots = lotsFor(sold, budget, Math.max(0, CURVE.graduateAt - sold));
  const paid = curveCost(sold, lots);
  const fee = (paid * BigInt(feeBps)) / BPS;
  return { lots, tokens: BigInt(lots) * LOT, paid, fee, refund: BigInt(amountIn) - paid - fee };
}

/// Selling `lots` alone: gross from the curve, the fee, what arrives.
export function previewCurveSell(sold, lots, feeBps) {
  const n = Math.min(lots, sold);
  const gross = n ? curveCost(sold - n, n) : 0n;
  const fee = (gross * BigInt(feeBps)) / BPS;
  return { lots: n, gross, fee, net: gross - fee };
}

/// Price of the next lot, per token, in the ledger's units.
export const curvePricePerToken = (sold) => curveCost(sold, 1) / CURVE.lotTokens;

export const createCurveIntent = (o) => concatBytes(
  new Uint8Array([TAG.createCurve]), o.creator, prog('curveCreate'), o.nonce, o.quote, o.meta,
  inputs(o.inputs || []), outputs(o.change || []),
);

export const curveTradeIntent = (o) => concatBytes(
  new Uint8Array([TAG.curveTrade]), o.trader, prog('curveTrade'), o.token, bool(o.buy),
  i128Bytes(o.amountIn), i128Bytes(o.minOut), o.nonce, inputs(o.inputs), outputs(o.change),
);

// ---------------------------------------------------------------------------
// Pricing, as the VM clears it. Previews only — the batch price is set at the
// seal by everybody's orders together — but computed with the same floors so
// a quote never promises more than the ledger could pay.
// ---------------------------------------------------------------------------

export const netOfFee = (amount, feeBps) => (amount * (BPS - BigInt(feeBps))) / BPS;

/// What `amountIn` would buy if it were the only order in the batch:
/// `net · (y) / (x + 2·net)` for a seller of A. Other orders on the same side
/// lower it and orders on the other side raise it, which is why the trade
/// carries a minimum rather than a promise.
export function quoteAlone(pool, sellA, amountIn) {
  const x = BigInt(pool.reserve_a);
  const y = BigInt(pool.reserve_b);
  const net = netOfFee(amountIn, pool.fee_bps);
  if (net <= 0n) return 0n;
  return sellA ? (net * y) / (x + 2n * net) : (net * x) / (y + 2n * net);
}

/// The minimum to sign for, `slippageBps` below the quote.
export const withSlippage = (quote, slippageBps) => (quote * (BPS - BigInt(slippageBps))) / BPS;

/// Shares `amountA`/`amountB` would earn in an existing pool, and what of each
/// it would actually take (the rest comes back as a claim).
export function liquidityPreview(pool, amountA, amountB) {
  const x = BigInt(pool.reserve_a), y = BigInt(pool.reserve_b), s = BigInt(pool.shares);
  const sa = (amountA * s) / x, sb = (amountB * s) / y;
  const shares = sa < sb ? sa : sb;
  const ceil = (n, d) => (n + d - 1n) / d;
  return { shares, usedA: ceil(shares * x, s), usedB: ceil(shares * y, s) };
}

/// `floor(√(a·b))` in WAD — the shares a new pool opens with.
export function geometricMean(a, b) {
  const n = a * b;
  if (n <= 0n) return 0n;
  let x = BigInt(Math.floor(Math.sqrt(Number(n))));
  // Newton from the float estimate; exact on BigInt.
  for (;;) {
    const y = (x + n / x) >> 1n;
    if (y >= x) break;
    x = y;
  }
  while (x * x > n) x -= 1n;
  while ((x + 1n) * (x + 1n) <= n) x += 1n;
  return x;
}

// ---------------------------------------------------------------------------
// The wallet's side: funding an intent from held notes, change to self,
// nonces that can be found again.
// ---------------------------------------------------------------------------

const NONCE_DOMAIN = te.encode('nap.market.nonce.v1');

/// The `counter`th market nonce for this identity. Derived rather than random
/// so a wallet restored from its phrase can find its claims again.
export const marketNonce = (secretKey, counter) => sha256(concatBytes(NONCE_DOMAIN, secretKey, u64be(counter)));

const random32 = () => crypto.getRandomValues(new Uint8Array(32));

/// A note to this wallet, sealed so it can find it. Fresh ephemeral key and
/// fresh nonce every time: reusing either links notes.
export function selfNote(identity, asset, amount) {
  const note = { asset, amount, owner: identity.account, nonce: random32() };
  const enc = sealNote(x25519.getPublicKey(identity.viewing), random32(), note);
  return { note, enc };
}

/// Pick held notes of `asset` covering `amount`, largest first, and the
/// change note for the rest. `held` is `scan(...).held`.
export function fund(identity, held, asset, amount, taken = new Set()) {
  const a = hex(asset);
  const mine = held
    .filter((h) => h.asset === a && !taken.has(h.commitment))
    .sort((p, q) => (q.amount > p.amount ? 1 : q.amount < p.amount ? -1 : 0));
  const picked = [];
  let sum = 0n;
  for (const h of mine) {
    if (sum >= amount) break;
    picked.push(h);
    taken.add(h.commitment);
    sum += h.amount;
  }
  if (sum < amount) throw new Error('Not enough to cover that');
  if (picked.length > 16) throw new Error('That needs too many notes at once; merge some first');
  const ins = picked.map((h) => {
    const c = unhex(h.commitment);
    return { commitment: c, nullifier: nullifierOf(identity.nullifier, c) };
  });
  const change = sum > amount ? [selfNote(identity, asset, sum - amount)] : [];
  return { inputs: ins, change };
}

/// Every claim id a source can produce (see the table in `objectvm::market`).
export const claimIds = (source) => [0, 1, 2].map((k) => claimId(source, k));

/// Redemption outputs: one note to self per claim, same asset and amount.
export function redeem(identity, claims) {
  return {
    owner: identity.account,
    claims: claims.map((c) => unhex(c.id)),
    outputs: claims.map((c) => selfNote(identity, unhex(c.asset), BigInt(c.amount))),
  };
}

export { commitment };

// ---------------------------------------------------------------------------
// A launch's name. The VM commits 32 opaque bytes of `meta`; this wallet reads
// them as an 8-byte symbol and a 24-byte name, printable ASCII, zero padded —
// on chain and public, so every wallet shows the same name without a server
// to ask. Anything that does not decode as that is shown as a bare id.
// ---------------------------------------------------------------------------

const PRINTABLE = /^[\x20-\x7e]*$/;

export function encodeMeta({ symbol, name }) {
  const s = String(symbol || '').trim().toUpperCase();
  const n = String(name || '').trim();
  if (!/^[A-Z0-9]{1,8}$/.test(s)) throw new Error('A symbol is 1–8 letters or digits');
  if (!n || n.length > 24 || !PRINTABLE.test(n)) throw new Error('A name is 1–24 plain characters');
  const out = new Uint8Array(32);
  out.set(te.encode(s), 0);
  out.set(te.encode(n), 8);
  return out;
}

export function decodeMeta(meta) {
  const bytes = typeof meta === 'string' ? unhex(meta) : meta;
  if (!(bytes instanceof Uint8Array) || bytes.length !== 32) return null;
  const text = (b) => {
    const end = b.indexOf(0);
    return new TextDecoder().decode(end === -1 ? b : b.subarray(0, end));
  };
  const symbol = text(bytes.subarray(0, 8));
  const name = text(bytes.subarray(8));
  if (!/^[A-Z0-9]{1,8}$/.test(symbol) || !name || !PRINTABLE.test(name)) return null;
  // Padding must be zeros all the way: otherwise two different metas would
  // show the same name, and a copycat could wear someone else's.
  const tail = (from, to) => bytes.subarray(from, to).every((b) => b === 0);
  if (!tail(symbol.length, 8) || !tail(8 + name.length, 32)) return null;
  return { symbol, name };
}

// ---------------------------------------------------------------------------
// NFTs: every item is its own asset, `item_address(collection, serial)`, so
// its identity survives every sale and send. A listing names no seller: the
// proceeds wait as a claim only the seller's nonce finds.
// ---------------------------------------------------------------------------

const DERIVE_DOMAIN = te.encode('zyn.derive.v1');
const ADDRESS_SCOPE_V1 = new Uint8Array([...te.encode('zynzap'), 0x00, 0x02]);

/// `zyn_vm::derive`: sha256 over the domain and length-prefixed parts.
function deriveAddress(program, namespace, seeds) {
  const parts = [DERIVE_DOMAIN, u32be(program.length), program, u32be(namespace.length), namespace, u32be(seeds.length)];
  for (const s of seeds) parts.push(u32be(s.length), s);
  return sha256(concatBytes(...parts));
}

export const itemAsset = (collection, serial) => deriveAddress(ADDRESS_SCOPE_V1, te.encode('item'), [collection, u32be(serial)]);
export const listingSource = (seller, nonce) => id('objectvm.listing.v1', seller, nonce);
export const saleFeeSource = (item) => id('objectvm.sale-fee.v1', item);
export const redemptionSource = (firstItem) => id('objectvm.redeem.v1', firstItem);
/// One item is one unit of its own asset.
export const ITEM_UNIT = WAD;

const PLACE_OFFER = hex(leaf(te.encode('zyn.program.named.v1'), u32be(11), te.encode('place-offer')));
const CANCEL_OFFER = hex(leaf(te.encode('zyn.program.named.v1'), u32be(12), te.encode('cancel-offer')));
const TAKE_OFFER = hex(leaf(te.encode('zyn.program.named.v1'), u32be(10), te.encode('take-offer')));
const REDEEM_ITEM = hex(leaf(te.encode('zyn.program.named.v1'), u32be(11), te.encode('redeem-item')));

export const listIntent = (o) => concatBytes(
  new Uint8Array([21]), o.seller, unhex(PLACE_OFFER), o.item, o.itemNullifier, o.priceAsset,
  i128Bytes(o.priceAmount), u64be(o.expiresAtEpoch), o.nonce,
);

export const cancelOfferIntent = (o) => concatBytes(new Uint8Array([4]), o.seller, unhex(CANCEL_OFFER), o.item);

/// `Op::TakeOffer`: the listed item's nullifier (from `zyn_offers`), the
/// buyer's payment, and outputs that are the item and the buyer's change.
export function takeOfferIntent(o) {
  const payment = [];
  for (const p of o.payment) payment.push(p.commitment, p.nullifier);
  const outs = [];
  for (const x of o.outputs) outs.push(x.note.asset, i128Bytes(x.note.amount), x.note.owner, x.note.nonce, x.enc.epk, u32be(x.enc.ciphertext.length), x.enc.ciphertext);
  return concatBytes(
    new Uint8Array([5]), o.buyer, unhex(TAKE_OFFER), o.item, o.itemNullifier,
    u32be(o.payment.length), u32be(o.outputs.length), ...payment, ...outs,
  );
}

export const redeemIntent = (o) => concatBytes(new Uint8Array([20]), o.holder, unhex(REDEEM_ITEM), o.collection, inputs(o.inputs));
