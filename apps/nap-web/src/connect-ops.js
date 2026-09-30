// What a connected site may ask Nap for, and what each costs.
//
// The same names and shapes the extension serves (`apps/extension`), so a
// site written against one works against the other. Four kinds:
//
//   public   the market's own public data; no connection needed
//   read     this holder's data, for a site the holder connected
//   write    moves something; the holder approves each one
//   connect  nap_connect itself
//
// Every write is *prepared* before it is shown: the approval describes what
// the chain says — the listed price, the item, the recipient — not what the
// site claims, and the site's claim is checked against it.

import { forgetIdentity, parseAmount, WAD } from './core.js';
import { contentHash, contentProof, hex, nullifierOf, unhex } from './object.js';
import * as mk from './market.js';
import * as ledger from './ledger.js';
import * as bridges from './bridges.js';
import { keccak_256 } from '@noble/hashes/sha3.js';
import { parsePaymentRequest, zatoshiOf } from './zip321.js';

/// EIP-1193's error codes, so a site handles Nap the way it handles any wallet.
export const ERR = Object.freeze({ REJECTED: 4001, UNAUTHORIZED: 4100, UNSUPPORTED: 4200, PARAMS: -32602 });
export class ProviderError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}
const bad = (m) => new ProviderError(ERR.PARAMS, m);

const ZAT = 100_000_000n;
const zecText = (zat) => { const v = BigInt(zat); const f = (v % ZAT).toString().padStart(8, '0').replace(/0+$/, ''); return `${v / ZAT}${f ? `.${f}` : ''}`; };
/// A ledger amount in full: an approval shows exactly what is signed, never
/// a rounded figure.
const wadText = (raw) => { const v = BigInt(raw); const f = (v % WAD).toString().padStart(18, '0').replace(/0+$/, ''); return `${v / WAD}${f ? `.${f}` : ''}`; };
/// An amount a site sent, or a parameter error that says which.
const amountOf = (v, what) => { try { return parseAmount(String(v ?? '')); } catch (e) { throw bad(`${what}: ${e.message}`); } };
const NO_ZEC = 'This wallet has no Zcash keys on this device yet. Open Nap, choose Zcash, add your Zcash wallet, then try again.';
const isHex = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);

/// Collection Z's species share one index space (§86.1).
const CAVE_Z = '024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e';
export function itemName(collection, serial) {
  if (collection !== CAVE_Z) return `${collection.slice(0, 6)}… #${serial}`;
  const [species, start] = serial < 2222 ? ['Mole', 0] : serial < 3555 ? ['Owl', 2222] : ['Moth', 3555];
  return `Zyn ${species} #${String(serial - start + 1).padStart(4, '0')}`;
}

/// The sale a site asks about: Collection Z unless it names another.
function saleCollection(p) {
  const c = p?.collection ?? CAVE_Z;
  if (!isHex(c)) throw bad('collection must be 64 hex characters');
  return c;
}

/// What the site learns on connecting: the account, and a Zcash address. Not
/// a balance, not a note, not a viewing key.
export async function session(ctx) {
  let zcash = null;
  // The Zcash address is a courtesy: if its wallet cannot open here, the
  // site still gets the account rather than a connection that never works.
  if (ctx.zec) {
    try { const view = await ctx.zec(); zcash = { network: view.network, address: view.address }; } catch { zcash = null; }
  }
  return { account: ctx.identity.accountHex, network: ctx.config.id, chain: ctx.config.chain, zcash };
}

async function holdings(ctx) {
  const offers = await ledger.loadOffers(ctx.config);
  const mine = await ledger.loadHoldings(ctx.config, ctx.identity, offers);
  return { offers, ...mine };
}

/// Refuse before asking the holder to approve something they cannot pay for.
async function mustHold(ctx, asset, amount, name) {
  const h = await holdings(ctx);
  const have = h.held.filter((x) => x.asset === asset).reduce((a, x) => a + BigInt(x.amount), 0n);
  // The refusal reaches the site, so it says what was asked, never what this
  // wallet holds: the connect card promises the site the account ID only.
  if (have < amount) throw bad(`This wallet does not hold enough ${name} (that needs ${wadText(amount)})`);
}

/// What an approval calls the coin a payment is made in.
async function quoteName(ctx, asset) {
  return (await namer(ctx, await ledger.call(ctx.config, 'zyn_market')))(asset);
}

function heldItem(h, asset) {
  if (!isHex(asset)) throw bad('asset must be 64 hex characters');
  const note = h.held.find((x) => x.asset === asset);
  const item = h.items.get(asset);
  if (!note || !item) throw bad('This wallet does not hold that item');
  return { note, item };
}

// ---------------------------------------------------------------------------
// Bonding curves (objectvm::curve, DECISIONS §141).
// ---------------------------------------------------------------------------

const isInt = (v) => typeof v === 'string' && /^\d{1,40}$/.test(v);
const validCurve = (c) => isHex(c.token) && isHex(c.quote) && isHex(c.meta)
  && ['reserve', 'graduation', 'creator_fees', 'bond', 'price_per_token'].every((k) => isInt(c[k]))
  && Number.isSafeInteger(c.sold) && c.sold >= 0 && typeof c.graduated === 'boolean';

async function curveBook(ctx) {
  const market = await ledger.call(ctx.config, 'zyn_market');
  const curves = (market.curves || []).filter(validCurve).map((c) => ({ ...c, ...(mk.decodeMeta(c.meta) || { symbol: '?', name: 'Unnamed' }) }));
  return { market, curves, terms: market.curve_terms || { set: false, fee_bps: 0, bond: '0' } };
}

async function oneCurve(ctx, token) {
  if (!isHex(token)) throw bad('token must be 64 hex characters');
  const { market, curves, terms } = await curveBook(ctx);
  const c = curves.find((x) => x.token === token);
  if (!c) throw bad('No token on a curve has that address');
  return { market, c, terms };
}

/// What an approval calls an asset: ZEC.zy, a bridged coin, a curve token by
/// its symbol, else its short address.
async function namer(ctx, market) {
  const bridged = await ledger.loadBridged(ctx.config);
  // Lending receipts, named from the asset they are a share of.
  const receipts = new Map([[ctx.config.zecAsset, 'ZEC.zy'], ...bridged.map((x) => [x.asset, ledger.bridgedName(ctx.config, x)])]
    .map(([u, n]) => [hex(mk.lendAsset(unhex(u))), `l${n}`]));
  return (a) => {
    if (a === ctx.config.zecAsset) return 'ZEC.zy';
    const b = bridged.find((x) => x.asset === a);
    if (b) return ledger.bridgedName(ctx.config, b);
    if (receipts.has(a)) return receipts.get(a);
    const c = (market.curves || []).find((x) => x.token === a);
    const meta = c && isHex(c.meta) ? mk.decodeMeta(c.meta) : null;
    return meta?.symbol || `${a.slice(0, 8)}…`;
  };
}

const tokenText = (raw) => { const v = BigInt(raw) / WAD; return v.toLocaleString('en-US'); };

// ---------------------------------------------------------------------------
// Zyn Lend (objectvm/src/lend.rs; zyn-io/zyn-lend).
// ---------------------------------------------------------------------------

const pctText = (wadRate) => `${(Number(BigInt(wadRate) * 10_000n / WAD) / 100).toFixed(2)}%`;

/// The lending markets, checked: a pool that does not parse is not offered.
async function lendBook(ctx) {
  const m = await ledger.call(ctx.config, 'zyn_lendMarkets');
  const pools = (m.pools || []).filter((x) => isHex(x.asset) && isHex(x.receipt)
    && ['cash', 'borrowed', 'supplied', 'share_price', 'borrow_rate', 'supply_rate', 'utilization'].every((k) => isInt(x[k]))
    && [x.max_ltv_bps, x.liquidation_bps, x.penalty_bps].every(Number.isSafeInteger));
  const prices = Object.fromEntries(Object.entries(m.prices || {}).filter(([a, v]) => isHex(a) && isInt(v)).map(([a, v]) => [a, BigInt(v)]));
  const name = await namer(ctx, await ledger.call(ctx.config, 'zyn_market'));
  return { m, pools, prices, name };
}

function lendPool(book, asset, what) {
  if (!isHex(asset)) throw bad(`${what} must be 64 hex characters`);
  const pool = book.pools.find((x) => x.asset === asset);
  if (!pool) throw bad(`${what}: that asset is not lent on this chain`);
  return pool;
}

/// Dollars for an amount at a posted price, both WAD.
const usdOf = (amount, price) => (price ? (amount * price) / WAD : null);
const usdText = (v) => (v == null ? 'no price posted' : `$${(Number(v / (WAD / 100n)) / 100).toLocaleString('en-US', { maximumFractionDigits: 2 })}`);

/// Health and the collateral price that liquidates, as the ledger will see it.
function lendPreview(book, coll, collAmount, borrow, owed) {
  const pc = book.prices[coll.asset], pb = book.prices[borrow.asset];
  const cv = usdOf(collAmount, pc), dv = usdOf(owed, pb);
  if (cv == null || dv == null) return { health: null, liquidation: null, allowed: false, cv, dv };
  const allowed = dv * 10_000n <= cv * BigInt(coll.max_ltv_bps);
  const health = dv > 0n ? Number((cv * BigInt(coll.liquidation_bps) * 100n) / (dv * 10_000n)) / 100 : null;
  const liquidation = dv > 0n && collAmount > 0n ? (dv * 10_000n * WAD) / (collAmount * BigInt(coll.liquidation_bps)) : null;
  return { health, liquidation, allowed, cv, dv };
}

async function ownPosition(ctx, id) {
  if (!isHex(id)) throw bad('position must be 64 hex characters');
  const p = (await ledger.loadPositions(ctx.config, ctx.identity)).find((x) => x.id === id);
  if (!p) throw bad('This wallet has no position with that id');
  if (p.state !== 'open') throw bad('That position is in a liquidation auction');
  return p;
}

export const METHODS = {
  nap_connect: { kind: 'connect' },
  nap_session: { kind: 'session' },
  nap_disconnect: { kind: 'session' },

  zyn_account: { kind: 'read', run: (ctx) => session(ctx) },
  zec_getAddress: {
    kind: 'read',
    run: async (ctx) => {
      if (!ctx.zec) throw new ProviderError(ERR.UNSUPPORTED, NO_ZEC);
      const v = await ctx.zec();
      return { network: v.network, address: v.address };
    },
  },

  zyn_nftOffers: {
    kind: 'public',
    run: async (ctx) => ({ offers: (await ledger.loadOffers(ctx.config)).filter((o) => o.serial >= 0) }),
  },
  zyn_nftItems: {
    kind: 'read',
    run: async (ctx) => {
      const h = await holdings(ctx);
      const listed = new Set(h.offers.map((o) => o.item));
      return {
        items: h.held.filter((n) => h.items.has(n.asset)).map((n) => ({
          asset: n.asset, ...h.items.get(n.asset), listed: listed.has(n.commitment),
        })),
      };
    },
  },
  zyn_owed: {
    kind: 'read',
    run: async (ctx) => ({ owed: (await ledger.loadOwed(ctx.config, ctx.identity)).map((r) => ({ id: r.id, asset: r.asset, amount: r.amount })) }),
  },

  zyn_curves: {
    kind: 'public',
    run: async (ctx) => { const { curves, terms } = await curveBook(ctx); return { curves, terms, curve: mk.CURVE }; },
  },
  zyn_curveHoldings: {
    kind: 'read',
    run: async (ctx) => {
      const { curves } = await curveBook(ctx);
      const h = await holdings(ctx);
      // The tokens this wallet made: its own nonces, turned into token ids.
      const mine = new Set();
      const count = (await ledger.ensureNonces(ctx.config, ctx.identity)) + ledger.LOOKAHEAD;
      for (let n = 0; n < count; n++) mine.add(hex(mk.tokenOf(ctx.identity.account, mk.marketNonce(ctx.identity.secretKey, n))));
      return {
        zec: String(h.balances.get(ctx.config.zecAsset) || 0n),
        holdings: curves.map((c) => ({ token: c.token, symbol: c.symbol, name: c.name, amount: String(h.balances.get(c.token) || 0n) }))
          .filter((x) => x.amount !== '0'),
        created: curves.filter((c) => mine.has(c.token)).map((c) => ({ token: c.token, symbol: c.symbol, name: c.name })),
      };
    },
  },

  // --- writes ---------------------------------------------------------------

  zyn_curveCreate: {
    kind: 'write',
    async prepare(ctx, p) {
      const meta = mk.encodeMeta({ symbol: p.symbol, name: p.name });
      // Optional picture and description, attached by proof after creation.
      const description = String(p.description || '').trim();
      const image = String(p.image || '');
      if (description.length > 400) throw bad('A description is at most 400 characters');
      if (image && !/^data:image\/(png|jpeg|webp|gif);base64,[A-Za-z0-9+/=]+$/.test(image)) throw bad('The picture must be a PNG, JPEG, WebP or GIF');
      if (image.length > 420_000) throw bad('The picture must be under 300 KB');
      const content = description || image ? { description, image } : null;
      const { terms } = await curveBook(ctx);
      const bond = BigInt(terms.bond || '0');
      if (bond > 0n) await mustHold(ctx, ctx.config.zecAsset, bond, 'ZEC.zy');
      return {
        title: `Create ${String(p.symbol).toUpperCase()}`,
        rows: [
          ['Name', String(p.name).trim()],
          ['Supply', '1,000,000,000 — all sold on the curve or put in its pool; none to you'],
          ['Graduates', `at ${mk.CURVE.graduateAt.toLocaleString('en-US')} of 80,000 lots (about 17 ZEC.zy raised), into a locked pool`],
          ['Bond', bond ? `${wadText(bond)} ZEC.zy — returned only if it graduates; otherwise it stays in the curve` : 'none'],
          ['Your share', terms.fee_bps ? `10% of its trading fees, paid when it graduates` : 'none (no fee is set)'],
          ...(description ? [['Description', description]] : []),
          ...(image ? [['Picture', 'shown above, stored by the site that asked; it proves you made the token']] : []),
        ],
        image: image || null,
        note: 'Anyone can buy it right away. You own none unless you buy, at the same price as everybody buying in that epoch.',
        run: async () => {
          let inputs = [], change = [];
          if (bond > 0n) {
            const h = await holdings(ctx);
            ({ inputs, change } = mk.fund(ctx.identity, h.held, unhex(ctx.config.zecAsset), bond));
          }
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.createCurveIntent({
            creator: ctx.identity.account, nonce: n, quote: unhex(ctx.config.zecAsset), meta, inputs, change,
          }));
          const token = mk.tokenOf(ctx.identity.account, nonce);
          if (!content) return { token: hex(token) };
          const hash = contentHash(content);
          return { token: hex(token), content: { hash: hex(hash), proof: await contentProof(ctx.identity.secretKey, nonce, token, hash) } };
        },
      };
    },
  },

  zyn_curveBuy: {
    kind: 'write',
    async prepare(ctx, p) {
      const { market, c } = await oneCurve(ctx, p.token);
      const q$ = (await namer(ctx, market))(c.quote);
      if (c.graduated) throw bad('That token has graduated; it trades in its pool (zyn_swap)');
      const amount = amountOf(p.amount, 'amount');
      if (amount < mk.MIN_CURVE_BUY) throw bad(`A buy is at least 0.001 ${q$}`);
      const q = mk.previewCurveBuy(c.sold, amount, c.fee_bps);
      if (q.lots === 0) throw bad('That buys less than one lot at the current price');
      const minLots = p.minTokens != null ? BigInt(amountOf(p.minTokens, 'minTokens')) / mk.LOT : 0n;
      await mustHold(ctx, c.quote, amount, q$);
      return {
        title: `Buy ${c.symbol} with ${wadText(amount)} ${q$}`,
        // No floor means filling at whatever price the batch clears: said, in red.
        danger: !minLots,
        rows: [
          ['Token', `${c.name} (${c.symbol})`],
          ['If nobody else buys this epoch', `${tokenText(q.tokens)} tokens`],
          ['Fee', c.fee_bps ? `${wadText(q.fee)} ${q$} (${c.fee_bps / 100}%)` : 'none'],
          ['Unspent', `${wadText(q.refund)} ${q$} comes back`],
          ['At least', minLots ? `${tokenText(minLots * mk.LOT)} tokens, or the whole buy comes back` : 'no floor — it fills at whatever price the batch clears'],
        ],
        note: 'Everyone buying this epoch pays one price, set when it closes; nobody can buy ahead of you. Collect the tokens once it settles.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(c.quote), amount);
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.curveTradeIntent({
            trader: ctx.identity.account, token: unhex(c.token), buy: true, amountIn: amount, minOut: minLots * mk.LOT, inputs, change, nonce: n,
          }));
          return { order: hex(mk.curveOrderId(ctx.identity.account, nonce)) };
        },
      };
    },
  },

  zyn_curveSell: {
    kind: 'write',
    async prepare(ctx, p) {
      const { market, c } = await oneCurve(ctx, p.token);
      const q$ = (await namer(ctx, market))(c.quote);
      if (c.graduated) throw bad('That token has graduated; it trades in its pool (zyn_swap)');
      const tokens = amountOf(p.tokens, 'tokens');
      if (tokens % mk.LOT !== 0n) throw bad(`Sell whole lots of ${mk.CURVE.lotTokens.toLocaleString('en-US')} tokens`);
      const lots = Number(tokens / mk.LOT);
      const q = mk.previewCurveSell(c.sold, lots, c.fee_bps);
      const minZec = p.minZec != null ? amountOf(p.minZec, 'minZec') : 0n;
      await mustHold(ctx, c.token, tokens, c.symbol);
      return {
        title: `Sell ${tokenText(tokens)} ${c.symbol}`,
        danger: !minZec,
        rows: [
          ['If nobody else sells this epoch', `${wadText(q.net)} ${q$}`],
          ['Fee', c.fee_bps ? `${wadText(q.fee)} ${q$} (${c.fee_bps / 100}%)` : 'none'],
          ['At least', minZec ? `${wadText(minZec)} ${q$}, or the tokens come back` : 'no floor — it sells at whatever price the batch clears'],
        ],
        note: `Sellers this epoch share one price, set when it closes. Collect the ${q$} once it settles.`,
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(c.token), tokens);
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.curveTradeIntent({
            trader: ctx.identity.account, token: unhex(c.token), buy: false, amountIn: tokens, minOut: minZec, inputs, change, nonce: n,
          }));
          return { order: hex(mk.curveOrderId(ctx.identity.account, nonce)) };
        },
      };
    },
  },

  zyn_swap: {
    kind: 'write',
    async prepare(ctx, p) {
      if (!isHex(p.pool)) throw bad('pool must be 64 hex characters');
      const market = await ledger.call(ctx.config, 'zyn_market');
      const pool = (market.pools || []).find((x) => x.id === p.pool);
      if (!pool || !['a', 'b', 'lp'].every((k) => isHex(pool[k])) || !isInt(pool.reserve_a) || !isInt(pool.reserve_b) || !Number.isSafeInteger(pool.fee_bps)
        || BigInt(pool.reserve_a) === 0n || BigInt(pool.reserve_b) === 0n) throw bad('No such pool');
      // A string "false" is true to JavaScript: the direction is a boolean or nothing.
      if (typeof p.sellA !== 'boolean') throw bad('sellA must be true or false');
      const sellA = p.sellA;
      const amountIn = amountOf(p.amount, 'amount');
      const quote = mk.quoteAlone(pool, sellA, amountIn);
      if (quote <= 0n) throw bad('Too small to trade in this pool');
      const slippage = Number.isInteger(p.slippageBps) && p.slippageBps >= 0 && p.slippageBps <= 1000 ? p.slippageBps : 100;
      const minOut = mk.withSlippage(quote, slippage);
      const sold = sellA ? pool.a : pool.b;
      const bought = sellA ? pool.b : pool.a;
      const name = await namer(ctx, market);
      const [inName, outName] = [name(sold), name(bought)];
      await mustHold(ctx, sold, amountIn, inName);
      // How far below the pool's spot price this fills, if alone.
      const x = BigInt(sellA ? pool.reserve_a : pool.reserve_b), y = BigInt(sellA ? pool.reserve_b : pool.reserve_a);
      const spot = (amountIn * y) / x;
      const impact = spot > 0n ? Number(((spot - quote) * 10_000n) / spot) / 100 : 0;
      return {
        title: `Swap ${inName} for ${outName}`,
        danger: impact >= 5,
        rows: [
          ['You put in', `${wadText(amountIn)} ${inName}`],
          ['If nobody else trades this epoch', `${wadText(quote)} ${outName}`],
          ['At least', `${wadText(minOut)} ${outName} (${slippage / 100}% slippage)`],
          ['Below the pool price', `${impact.toFixed(2)}%${impact >= 5 ? ' — a large share of this pool' : ''}`],
          ['Pool fee', `${pool.fee_bps / 100}%, kept by the pool`],
          // Symbols can repeat; the addresses cannot.
          ['Selling', `${inName} · ${sold.slice(0, 8)}…${sold.slice(-4)}`],
          ['Buying', `${outName} · ${bought.slice(0, 8)}…${bought.slice(-4)}`],
        ],
        note: 'Swaps in one epoch clear together at one price. If that price is worse than your floor, everything you put in comes back.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(sold), amountIn);
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.swapIntent({
            trader: ctx.identity.account, pool: unhex(p.pool), sellA, amountIn, minOut, nonce: n, inputs, change,
          }));
          return { order: hex(mk.orderId(ctx.identity.account, nonce)) };
        },
      };
    },
  },

  // --- Zyn Lend ---------------------------------------------------------------

  zyn_lendMarkets: { kind: 'public', run: async (ctx) => ledger.call(ctx.config, 'zyn_lendMarkets') },
  zyn_lendPositions: { kind: 'read', run: async (ctx) => ({ positions: await ledger.loadPositions(ctx.config, ctx.identity) }) },

  zyn_lendSupply: {
    kind: 'write',
    async prepare(ctx, p) {
      const book = await lendBook(ctx);
      const pool = lendPool(book, p.asset, 'asset');
      const amount = amountOf(p.amount, 'amount');
      const name = book.name(pool.asset);
      await mustHold(ctx, pool.asset, amount, name);
      return {
        title: `Lend ${name}`,
        danger: false,
        rows: [
          ['You lend', `${wadText(amount)} ${name}`],
          ['Earning now', `${pctText(pool.supply_rate)} a year, changing as the pool is used`],
          ['You receive', `l${name} shares, collected into this wallet`],
          ['Pool lent out', pctText(pool.utilization)],
        ],
        note: 'If a borrower\'s collateral sells for less than their debt, the loss lowers every share in this pool. Withdrawing needs the coins to be free in the pool.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(pool.asset), amount);
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.lendSupplyIntent({ lender: ctx.identity.account, asset: unhex(pool.asset), amount, nonce: n, inputs, change }));
          return { source: hex(mk.lendSource(ctx.identity.account, nonce)) };
        },
      };
    },
  },

  zyn_lendWithdraw: {
    kind: 'write',
    async prepare(ctx, p) {
      const book = await lendBook(ctx);
      const pool = lendPool(book, p.asset, 'asset');
      const shares = amountOf(p.shares, 'shares');
      const name = book.name(pool.asset);
      await mustHold(ctx, pool.receipt, shares, `l${name}`);
      // The chain publishes a share's price, not how many shares exist.
      const worth = (shares * BigInt(pool.share_price)) / WAD;
      if (worth > BigInt(pool.cash)) throw bad(`About ${wadText(pool.cash)} ${name} is free in the pool right now; the rest is lent out`);
      return {
        title: `Withdraw ${name}`,
        danger: false,
        rows: [['You return', `${wadText(shares)} l${name}`], ['About', `${wadText(worth)} ${name}, collected into this wallet`]],
        note: 'The exact amount is what the shares are worth when this is applied.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(pool.receipt), shares);
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.lendWithdrawIntent({ lender: ctx.identity.account, asset: unhex(pool.asset), shares, nonce: n, inputs, change }));
          return { source: hex(mk.lendSource(ctx.identity.account, nonce)) };
        },
      };
    },
  },

  zyn_lendOpen: {
    kind: 'write',
    async prepare(ctx, p) {
      const book = await lendBook(ctx);
      const coll = lendPool(book, p.collateral, 'collateral');
      const borrow = lendPool(book, p.borrow, 'borrow');
      if (coll.asset === borrow.asset) throw bad('Collateral and the borrowed asset must differ');
      const collAmount = amountOf(p.collateralAmount, 'collateralAmount');
      const amount = amountOf(p.amount, 'amount');
      const [cn, bn] = [book.name(coll.asset), book.name(borrow.asset)];
      await mustHold(ctx, coll.asset, collAmount, cn);
      if (amount > BigInt(borrow.cash)) throw bad(`The ${bn} pool has about ${wadText(borrow.cash)} free to borrow`);
      if (coll.price_held || borrow.price_held || book.m.prices_fresh === false) throw bad('Prices are stale or still moving; borrowing waits for the next confirmed price');
      if (borrow.min_borrow_usd != null && isInt(borrow.min_borrow_usd) && book.prices[borrow.asset] && usdOf(amount, book.prices[borrow.asset]) < BigInt(borrow.min_borrow_usd)) {
        throw bad(`The smallest ${bn} loan is ${usdText(BigInt(borrow.min_borrow_usd))}`);
      }
      const v = lendPreview(book, coll, collAmount, borrow, amount);
      if (v.health == null) throw bad('No price is posted for this pair yet; borrowing opens once one is');
      if (!v.allowed) throw bad(`That is over the ${coll.max_ltv_bps / 100}% most ${cn} collateral may borrow`);
      return {
        title: `Borrow ${bn} against ${cn}`,
        danger: v.health < 1.5,
        rows: [
          ['You lock', `${wadText(collAmount)} ${cn} (${usdText(v.cv)})`],
          ['You borrow', `${wadText(amount)} ${bn} (${usdText(v.dv)})`],
          ['Health', `${v.health.toFixed(2)} — liquidated below 1.00`],
          ['Liquidated if', `${cn} falls to ${usdText(v.liquidation)}`],
          ['Borrow rate', `${pctText(borrow.borrow_rate)} a year, changing as the pool is used`],
          ['If liquidated', `your ${cn} is auctioned to cover the debt plus ${coll.penalty_bps / 100}%; what is left comes back to you`],
        ],
        note: 'The position is private: only this wallet can find it. Prices are posted by the sequencer; if Zyn\'s own pool for this pair disagrees with them, the chain refuses the borrow.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(coll.asset), collAmount);
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.lendOpenIntent({
            borrower: ctx.identity.account, collateralAsset: unhex(coll.asset), collateral: collAmount, borrowAsset: unhex(borrow.asset), amount, nonce: n, inputs, change,
          }));
          return { position: hex(mk.lendSource(ctx.identity.account, nonce)) };
        },
      };
    },
  },

  zyn_lendRepay: {
    kind: 'write',
    async prepare(ctx, p) {
      const pos = await ownPosition(ctx, p.position);
      const book = await lendBook(ctx);
      const name = book.name(pos.borrow_asset);
      const owed = BigInt(pos.owed);
      // "all" pays what is owed plus a little for interest until it lands;
      // the chain returns anything over.
      const all = p.amount === 'all';
      const amount = all ? owed + owed / 1000n + 1n : amountOf(p.amount, 'amount');
      await mustHold(ctx, pos.borrow_asset, amount, name);
      const cn = book.name(pos.collateral_asset);
      const collateral = BigInt(pos.collateral);
      // Repaying everything closes the position: the collateral comes back in
      // the same approval, as a second intent the chain applies right after.
      const closing = all && collateral > 0n;
      return {
        title: closing ? `Repay ${name} and close` : `Repay ${name}`,
        danger: false,
        rows: [
          ['You pay', `${wadText(amount)} ${name}`],
          ['Owed now', `${wadText(owed)} ${name}`],
          ...(closing ? [['You get back', `${wadText(collateral)} ${cn}, collected into this wallet`]] : []),
        ],
        note: 'Anything paid over what is owed comes back to this wallet.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(pos.borrow_asset), amount);
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.lendRepayIntent({ borrower: ctx.identity.account, position: unhex(pos.id), amount, nonce: n, inputs, change }));
          const source = hex(mk.lendSource(ctx.identity.account, nonce));
          if (!closing) return { source };
          try {
            const back = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.lendRemoveCollateralIntent({ borrower: ctx.identity.account, position: unhex(pos.id), amount: collateral, nonce: n }));
            return { source, closed: true, collateral: hex(mk.lendSource(ctx.identity.account, back)) };
          } catch (e) {
            // The loan is repaid either way; say where the collateral is.
            return { source, closed: false, note: `Repaid. The collateral is still in the position (${e.message}); take it out under Positions.` };
          }
        },
      };
    },
  },

  zyn_lendAddCollateral: {
    kind: 'write',
    async prepare(ctx, p) {
      const pos = await ownPosition(ctx, p.position);
      const book = await lendBook(ctx);
      const name = book.name(pos.collateral_asset);
      const amount = amountOf(p.amount, 'amount');
      await mustHold(ctx, pos.collateral_asset, amount, name);
      return {
        title: `Add ${name} collateral`,
        danger: false,
        rows: [['You add', `${wadText(amount)} ${name}`], ['Collateral after', `${wadText(BigInt(pos.collateral) + amount)} ${name}`]],
        note: 'More collateral moves the liquidation price further away.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(pos.collateral_asset), amount);
          await ledger.submitIntent(ctx.config, ctx.identity, mk.lendAddCollateralIntent({ borrower: ctx.identity.account, position: unhex(pos.id), amount, inputs, change }));
          return { position: pos.id };
        },
      };
    },
  },

  zyn_lendRemoveCollateral: {
    kind: 'write',
    async prepare(ctx, p) {
      const pos = await ownPosition(ctx, p.position);
      const book = await lendBook(ctx);
      const name = book.name(pos.collateral_asset);
      const amount = p.amount === 'all' ? BigInt(pos.collateral) : amountOf(p.amount, 'amount');
      if (amount > BigInt(pos.collateral)) throw bad('More than the position holds');
      return {
        title: `Take out ${name} collateral`,
        danger: BigInt(pos.owed) > 0n,
        rows: [['You take out', `${wadText(amount)} ${name}`], ['Debt', `${wadText(pos.owed)} ${book.name(pos.borrow_asset)}`]],
        note: 'The chain refuses this if it would leave the position over its maximum.',
        run: async () => {
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.lendRemoveCollateralIntent({ borrower: ctx.identity.account, position: unhex(pos.id), amount, nonce: n }));
          return { source: hex(mk.lendSource(ctx.identity.account, nonce)) };
        },
      };
    },
  },

  zyn_lendBid: {
    kind: 'write',
    async prepare(ctx, p) {
      if (!isHex(p.lot)) throw bad('lot must be 64 hex characters');
      const book = await lendBook(ctx);
      const lot = (book.m.lots || []).find((x) => x.lot === p.lot);
      if (!lot || !isHex(lot.borrow_asset) || !isHex(lot.collateral_asset) || !isInt(lot.price) || !isInt(lot.collateral)) throw bad('No such auction lot');
      const pay = amountOf(p.pay, 'pay');
      const [cn, bn] = [book.name(lot.collateral_asset), book.name(lot.borrow_asset)];
      await mustHold(ctx, lot.borrow_asset, pay, bn);
      const price = BigInt(lot.price);
      const most = price > 0n ? (pay * WAD) / price : 0n;
      return {
        title: `Bid for ${cn} in a liquidation auction`,
        danger: false,
        rows: [
          ['You pay up to', `${wadText(pay)} ${bn}`],
          ['Price this epoch', `${wadText(price)} ${bn} per ${cn}`],
          ['You get up to', `${wadText(most)} ${cn}`],
        ],
        note: 'Every bid this epoch pays the same price. If bids want more than the lot has, each gets a share and the rest of the payment comes back.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(lot.borrow_asset), pay);
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.lendBidIntent({ bidder: ctx.identity.account, lot: unhex(lot.lot), pay, nonce: n, inputs, change }));
          return { source: hex(mk.lendSource(ctx.identity.account, nonce)) };
        },
      };
    },
  },

  zec_pay: {
    kind: 'write',
    async prepare(ctx, p) {
      if (!ctx.zec) throw new ProviderError(ERR.UNSUPPORTED, NO_ZEC);
      let req;
      if (p.uri) req = parsePaymentRequest(p.uri);
      else {
        if (typeof p.address !== 'string' || !p.address) throw bad('address is required');
        req = { address: p.address.trim(), zatoshi: zatoshiOf(String(p.amount ?? '')), memo: String(p.memo || ''), message: '' };
      }
      if (req.zatoshi == null || req.zatoshi <= 0) throw bad('The request names no amount');
      const net = ctx.config.zcashPrefix === 'u1' ? 'mainnet' : 'testnet';
      const shielded = req.address.startsWith(ctx.config.zcashPrefix);
      // Transparent: t1/t3 on mainnet, tm/t2 on testnet. Sapling (zs…) is not
      // something this wallet sends to.
      const transparent = (net === 'mainnet' ? /^t[13][a-km-zA-HJ-NP-Z1-9]{33}$/ : /^t[m2][a-km-zA-HJ-NP-Z1-9]{33}$/).test(req.address);
      if (/^z(s|testsapling)1/.test(req.address)) throw bad('That is a Sapling address; ask for a unified (u1…) or transparent address');
      if (!shielded && !transparent) throw bad(`That is not a ${net} Zcash address`);
      if (!shielded && req.memo) throw bad('A memo only reaches a shielded address');
      return {
        title: `Pay ${zecText(req.zatoshi)} ZEC`,
        rows: [
          ['To', req.address],
          ['Amount', `${zecText(req.zatoshi)} ZEC, plus the network fee`],
          ...(req.memo ? [['Memo', req.memo]] : []),
          ...(req.message ? [['The site says', req.message]] : []),
          ['Network', `Zcash ${net}`],
        ],
        note: 'A Zcash transaction from this wallet. Proving it takes about half a minute; keep this window open.',
        run: async (onStep) => {
          const account = await ctx.zecReady();
          const v = await account.send(req.address, req.zatoshi, req.memo, onStep);
          return { txid: v.txid };
        },
      };
    },
  },

  // -------------------------------------------------------------- bridges
  // A site's bridge page (Zyn Lend's) talks to Phantom or MetaMask itself;
  // Nap makes what only this wallet can — the one-time deposit code, and the
  // withdrawal — and says plainly that the other chain sees both.

  zyn_bridges: {
    kind: 'public',
    async run(ctx) {
      const st = await bridges.bridgeState(ctx.config);
      return {
        bps: Number(st.bps),
        zec: { asset: ctx.config.zecAsset, zcash: ctx.config.zcash },
        bridges: st.bridges.map((b) => ({
          key: b.key, symbol: b.symbol, zy: b.zy, chainName: b.chainName, decimals: b.decimals, network: b.network, asset: b.asset,
          live: b.live, issued: b.issued === null ? null : String(b.issued),
          vault: b.vault || null, rpc: b.rpc || null, cluster: b.cluster || null, chainId: b.chainId || null, explorer: b.explorer || null,
          capWei: b.capWei || null, custody: b.custody || null, safe: b.safe || null, chain: b.chain || null,
        })),
      };
    },
  },

  /// What to send, for a deposit into this wallet: SOL a memo, ETH the
  /// calldata, BTC an address (watched from now), ZEC this account's
  /// deposit address. The code names nobody; the other chain sees the sender.
  zyn_bridgeDeposit: {
    kind: 'read',
    async run(ctx, p) {
      const key = String(p?.key || '');
      if (key === 'zec') {
        const r = await ledger.call(ctx.config, 'zyn_depositAddress', { account: ctx.identity.accountHex });
        if (typeof r?.address !== 'string') throw bad('The node has no deposit address for this account yet');
        return { key, address: r.address };
      }
      const b = bridges.bridgesOf(ctx.config).find((x) => x.key === key);
      if (!b) throw bad(`No ${key.toUpperCase() || 'such'} bridge on this network`);
      if (key === 'btc') return { key, address: await bridges.btcDepositAddress(ctx.config, ctx.identity, b) };
      const code = await bridges.depositCode(ctx.config, ctx.identity);
      if (key === 'sol') return { key, memo: `zyn:${code}`, vault: b.vault, rpc: b.rpc, cluster: b.cluster };
      const asset = new Uint8Array(32);
      new DataView(asset.buffer).setUint32(28, Number(b.assetId || 1));
      const selector = hex(keccak_256(new TextEncoder().encode('depositNative(bytes32,uint32)')).slice(0, 4));
      return { key, to: b.vault, chainId: b.chainId, data: `0x${selector}${code}${hex(asset)}`, capWei: b.capWei || null };
    },
  },

  /// SOL.zy / ETH.zy / BTC.zy / ZEC.zy back to their chain.
  zyn_bridgeWithdraw: {
    kind: 'write',
    async prepare(ctx, p) {
      const key = String(p?.key || '');
      const to = String(p?.to || '').trim();
      const amount = amountOf(p?.amount, 'amount');
      const st = await bridges.bridgeState(ctx.config);
      const fee = (amount * st.bps) / 10_000n;
      if (key === 'zec') {
        if (!to.startsWith(ctx.config.zcashPrefix) && !/^t[mu]?[1-9A-HJ-NP-Za-km-z]{30,40}$/.test(to)) throw bad('That is not a Zcash address for this network');
        await mustHold(ctx, ctx.config.zecAsset, amount + fee, 'ZEC.zy');
        return {
          title: 'Withdraw ZEC',
          danger: true,
          rows: [['You send', `${wadText(amount)} ZEC.zy`], ['Bridge fee', `${wadText(fee)} ZEC.zy`], ['To', to]],
          note: 'Paid from the vault once two custodians sign; the Zcash network fee comes out of the amount. A shielded address keeps the payout private.',
          run: async () => {
            const h = await holdings(ctx);
            await bridges.withdrawZec(ctx.config, ctx.identity, h.held, to, wadText(amount));
            return { ok: true };
          },
        };
      }
      const b = st.bridges.find((x) => x.key === key);
      if (!b || !b.live) throw bad(`No ${key.toUpperCase() || 'such'} bridge on this network`);
      try { bridges.checkBridgeOut(b, to, amount); } catch (e) { throw bad(e.message); }
      await mustHold(ctx, b.asset, amount + fee, b.zy);
      return {
        title: `Withdraw ${b.symbol}`,
        danger: true,
        rows: [['You send', `${wadText(amount)} ${b.zy}`], ['Bridge fee', `${wadText(fee)} ${b.zy}`], ['To', to], ['On', b.chainName]],
        note: `${b.chainName} will show this address receiving the amount from the vault${b.safe ? '; the Safe owners approve each payout, which can take hours' : ''}.`,
        run: async () => {
          const h = await holdings(ctx);
          await bridges.bridgeOut(ctx.config, ctx.identity, h.held, b, to, amount, st.bps);
          return { ok: true };
        },
      };
    },
  },

  zyn_nftBuy: {
    kind: 'write',
    async prepare(ctx, p) {
      if (!isHex(p.item)) throw bad('item must be the listing\'s 64-hex item');
      const offers = await ledger.loadOffers(ctx.config);
      const o = offers.find((x) => x.item === p.item && x.serial >= 0);
      if (!o) throw bad('That listing is gone');
      const pay = BigInt(o.pay_raw);
      // The site shows a price; the chain has one. If they differ the holder
      // would be approving a number they were not shown.
      if (p.pay != null && amountOf(p.pay, 'pay') !== pay) throw bad('The price changed since the page loaded; reload and try again');
      const q = await quoteName(ctx, o.price_asset);
      await mustHold(ctx, o.price_asset, pay, q);
      return {
        title: `Buy ${itemName(o.collection, o.serial)}`,
        rows: [['Price', `${wadText(o.price_raw)} ${q}`], ['Collection fee', `${wadText(o.fee_raw)} ${q}`], ['You pay', `${wadText(pay)} ${q}`]],
        note: 'The item arrives in this wallet. The seller is paid by the ledger; you never learn who they are.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(o.price_asset), pay);
          const item = mk.selfNote(ctx.identity, unhex(o.asset), mk.ITEM_UNIT);
          await ledger.submitIntent(ctx.config, ctx.identity, mk.takeOfferIntent({
            buyer: ctx.identity.account, item: unhex(o.item), itemNullifier: unhex(o.nullifier), payment: inputs, outputs: [item, ...change],
          }));
          return { bought: o.asset };
        },
      };
    },
  },

  zyn_nftList: {
    kind: 'write',
    async prepare(ctx, p) {
      const price = amountOf(p.price, 'price');
      if (price <= 0n) throw bad('price must be above zero');
      const h = await holdings(ctx);
      const { note, item } = heldItem(h, p.asset);
      if (h.offers.some((o) => o.item === note.commitment)) throw bad('That item is already listed');
      const market = await ledger.call(ctx.config, 'zyn_market');
      const terms = (market.collections || []).find((c) => c.asset === item.collection);
      const quote = terms && isHex(terms.quote) ? terms.quote : ctx.config.zecAsset;
      const feeBps = terms ? Number(terms.fee_bps) || 0 : 0;
      const q = (await namer(ctx, market))(quote);
      return {
        title: `List ${itemName(item.collection, item.serial)}`,
        rows: [['Price', `${wadText(price)} ${q}`], ['Fee', feeBps ? `${feeBps / 100}% from each side` : 'none'], ['You receive', `${wadText(price - (price * BigInt(feeBps)) / 10_000n)} ${q} when it sells`]],
        note: 'The listing names no seller. When it sells, your proceeds wait as a claim only this wallet can find.',
        run: async () => {
          const c = unhex(note.commitment);
          await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.listIntent({
            seller: ctx.identity.account, item: c, itemNullifier: nullifierOf(ctx.identity.nullifier, c),
            priceAsset: unhex(quote), priceAmount: price, expiresAtEpoch: 0xffffffffn, nonce: n,
          }));
          return { listed: note.asset };
        },
      };
    },
  },

  zyn_nftCancel: {
    kind: 'write',
    async prepare(ctx, p) {
      const h = await holdings(ctx);
      const { note, item } = heldItem(h, p.asset);
      if (!h.offers.some((o) => o.item === note.commitment)) throw bad('That item is not listed');
      return {
        title: `Cancel the listing of ${itemName(item.collection, item.serial)}`,
        rows: [['Item', itemName(item.collection, item.serial)]],
        note: 'The item stays in your wallet and can be moved again.',
        run: async () => {
          await ledger.submitIntent(ctx.config, ctx.identity, mk.cancelOfferIntent({ seller: ctx.identity.account, item: unhex(note.commitment) }));
          return { cancelled: note.asset };
        },
      };
    },
  },

  zyn_nftRedeem: {
    kind: 'write',
    async prepare(ctx, p) {
      const h = await holdings(ctx);
      const { note, item } = heldItem(h, p.asset);
      const market = await ledger.call(ctx.config, 'zyn_market');
      const terms = (market.collections || []).find((c) => c.asset === item.collection);
      if (!terms || !(BigInt(terms.outstanding || '0') > 0n)) throw bad('This collection has no floor to redeem from yet');
      const share = BigInt(terms.floor) / BigInt(terms.outstanding);
      if (share <= 0n) throw bad('The floor is empty; redeeming now would burn the item for nothing');
      const q = (await namer(ctx, market))(terms.quote);
      return {
        title: `Redeem ${itemName(item.collection, item.serial)} for the floor`,
        rows: [['Floor share', `${wadText(share)} ${q}`], ['The item', 'is burned — this cannot be undone']],
        note: 'Your share of the floor waits as a claim; collect it afterwards.',
        danger: true,
        run: async () => {
          const c = unhex(note.commitment);
          await ledger.submitIntent(ctx.config, ctx.identity, mk.redeemIntent({
            holder: ctx.identity.account, collection: unhex(item.collection), inputs: [{ commitment: c, nullifier: nullifierOf(ctx.identity.nullifier, c) }],
          }));
          return { redeemed: note.asset };
        },
      };
    },
  },

  // Collection Z's sale (§144): tickets in ZEC.zy, a lottery drawn after the
  // close, winners claim with a proof against the posted root, and whatever
  // is left sells in the posted order.
  zyn_saleInfo: {
    kind: 'public',
    run: async (ctx, p) => {
      const s = await ledger.loadSale(ctx.config, saleCollection(p));
      if (!s) return { sale: null };
      return { sale: s.sale, drawPosted: !!s.draw, openPosted: !!s.open, remaining: s.open ? s.sale.open_count - s.sale.open_next : null };
    },
  },
  zyn_saleTickets: {
    kind: 'read',
    run: async (ctx, p) => {
      const collection = saleCollection(p);
      const [s, mine] = await Promise.all([ledger.loadSale(ctx.config, collection), ledger.loadTickets(ctx.config, ctx.identity, collection)]);
      const wins = new Map((s?.draw?.won || []).map((w) => [w.ticket, w.serials]));
      const drawn = !!s?.draw;
      return {
        tickets: mine.map((t) => ({
          ticket: t.ticket, index: t.index, claimed: t.claimed, burned: !!t.voucher,
          result: !drawn ? 'waiting' : t.claimed ? 'claimed' : wins.has(t.ticket) ? 'won' : 'no win',
          serials: wins.get(t.ticket) || [],
          names: (wins.get(t.ticket) || []).map((x) => itemName(collection, x)),
        })),
      };
    },
  },

  // The memo that makes a burn of the sale's promotional token on Solana
  // into this wallet's tickets (§145). It names no account: the commitment
  // binds this one without saying which. Nothing moves, so no approval.
  zyn_burnMemo: {
    kind: 'read',
    run: async (ctx, p) => ledger.burnMemo(ctx.config, ctx.identity, { fresh: p?.fresh === true }),
  },

  zyn_buyTickets: {
    kind: 'write',
    async prepare(ctx, p) {
      const collection = saleCollection(p);
      const count = Number(p.count ?? 1);
      if (!Number.isInteger(count) || count < 1 || count > 20) throw bad('Buy between 1 and 20 tickets at a time');
      const s = await ledger.loadSale(ctx.config, collection);
      if (!s) throw bad('There is no sale for that collection');
      if (s.sale.phase !== 'open') throw bad('The sale is closed');
      const total = BigInt(s.sale.ticket_price) * BigInt(count);
      if (p.pay != null && amountOf(p.pay, 'pay') !== total) throw bad('The price changed since the page loaded; reload and try again');
      const q = await quoteName(ctx, s.quote);
      await mustHold(ctx, s.quote, total, q);
      return {
        title: `Buy ${count} ticket${count > 1 ? 's' : ''} in the ${collection === CAVE_Z ? 'Collection Z' : `${collection.slice(0, 6)}…`} sale`,
        rows: [['Per ticket', `${wadText(s.sale.ticket_price)} ${q}`], ['Tickets', String(count)], ['You pay', `${wadText(total)} ${q}`]],
        note: 'Each ticket is drawn after the sale closes. A winning ticket claims its items at the claim price; the ticket price is not refunded.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(s.quote), total);
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.buyTicketsIntent({
            buyer: ctx.identity.account, collection: unhex(collection), count, nonce: n, inputs, change,
          }));
          return { tickets: Array.from({ length: count }, (_, k) => hex(mk.ticketId(ctx.identity.account, nonce, k))) };
        },
      };
    },
  },

  zyn_claimItems: {
    kind: 'write',
    async prepare(ctx, p) {
      const collection = saleCollection(p);
      const s = await ledger.loadSale(ctx.config, collection);
      if (!s || s.sale.phase !== 'drawn') throw bad('Claims open after the draw is posted and close when the open mint starts');
      if (!s.draw) throw bad('The posted draw does not match the chain; nothing can be claimed until it does');
      const index = new Map(s.draw.won.map((w, i) => [w.ticket, i]));
      const mine = (await ledger.loadTickets(ctx.config, ctx.identity, collection)).filter((t) => !t.claimed && index.has(t.ticket));
      if (!mine.length) throw bad('This wallet has no unclaimed winning tickets');
      const serials = mine.flatMap((t) => s.draw.won[index.get(t.ticket)].serials);
      const total = BigInt(s.sale.claim_price) * BigInt(serials.length);
      const q = await quoteName(ctx, s.quote);
      await mustHold(ctx, s.quote, total, q);
      return {
        title: `Claim ${serials.length} item${serials.length > 1 ? 's' : ''}`,
        rows: [...serials.slice(0, 6).map((x) => ['Item', itemName(collection, x)]), ...(serials.length > 6 ? [['…', `and ${serials.length - 6} more`]] : []),
          ['Per item', `${wadText(s.sale.claim_price)} ${q}`], ['You pay', `${wadText(total)} ${q}`]],
        note: 'Each item is minted straight into this wallet. Unclaimed wins go to the open mint when claims close.',
        run: async () => {
          const claimed = [];
          let spent = [];
          for (const t of mine) {
            if (spent.length) await ledger.waitSpent(ctx.config, ctx.identity, spent);
            const i = index.get(t.ticket);
            const won = s.draw.won[i].serials;
            const h = await holdings(ctx);
            const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(s.quote), BigInt(s.sale.claim_price) * BigInt(won.length));
            const claim = {
              buyer: ctx.identity.account, collection: unhex(collection), k: t.k, serials: won,
              proof: mk.merkleProof(s.draw.leaves, i),
              items: won.map((x) => mk.selfNote(ctx.identity, mk.itemAsset(unhex(collection), x), mk.ITEM_UNIT)), inputs, change,
            };
            // A burn ticket is claimed by revealing the secret its memo committed to.
            await ledger.submitIntent(ctx.config, ctx.identity, t.voucher
              ? mk.claimVoucherItemsIntent({ ...claim, secret: t.nonce })
              : mk.claimItemsIntent({ ...claim, nonce: t.nonce }));
            spent = inputs.map((x) => hex(x.commitment));
            claimed.push(...won);
          }
          return { claimed };
        },
      };
    },
  },

  zyn_mintOpen: {
    kind: 'write',
    async prepare(ctx, p) {
      const collection = saleCollection(p);
      const count = Number(p.count ?? 1);
      if (!Number.isInteger(count) || count < 1 || count > 10) throw bad('Mint between 1 and 10 at a time');
      const s = await ledger.loadSale(ctx.config, collection);
      if (!s || s.sale.phase !== 'openmint') throw bad('The open mint has not started');
      if (!s.open) throw bad('The posted order does not match the chain; nothing can be minted until it does');
      const left = s.sale.open_count - s.sale.open_next;
      if (left < count) throw bad(left ? `Only ${left} left` : 'The open mint is sold out');
      const total = BigInt(s.sale.open_price) * BigInt(count);
      const q = await quoteName(ctx, s.quote);
      await mustHold(ctx, s.quote, total, q);
      return {
        title: `Mint ${count} from the open mint`,
        rows: [['Per item', `${wadText(s.sale.open_price)} ${q}`], ['Items', String(count)], ['You pay', `${wadText(total)} ${q}`], ['Left', String(left)]],
        note: 'Items come in the order posted after the draw — the next ones in line, whichever they are.',
        run: async () => {
          const minted = [];
          let spent = [];
          for (let got = 0, tries = 0; got < count && tries < count + 8; tries++) {
            if (spent.length) await ledger.waitSpent(ctx.config, ctx.identity, spent);
            const now = await ledger.loadSale(ctx.config, collection);
            const pos = now.sale.open_next;
            if (pos >= now.sale.open_count) break;
            const serial = s.open.order[pos];
            const h = await holdings(ctx);
            const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(s.quote), BigInt(s.sale.open_price));
            const item = mk.selfNote(ctx.identity, mk.itemAsset(unhex(collection), serial), mk.ITEM_UNIT);
            try {
              await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.mintOpenIntent({
                buyer: ctx.identity.account, collection: unhex(collection), serial, proof: mk.merkleProof(s.open.leaves, pos), item, nonce: n, inputs, change,
              }));
            } catch (e) {
              // Someone else minted that place first; take the next one.
              if (/not the next item|already minted/.test(String(e.message))) continue;
              throw e;
            }
            spent = inputs.map((x) => hex(x.commitment));
            minted.push(serial);
            got++;
          }
          if (!minted.length) throw new Error('The open mint sold out first');
          return { minted, names: minted.map((x) => itemName(collection, x)) };
        },
      };
    },
  },

  // Collecting only moves what the ledger owes this wallet into it, so a
  // connected site may ask without an approval each time — trades then
  // simply arrive, as they do on any launchpad.
  zyn_collect: {
    kind: 'auto',
    run: async (ctx) => {
      const owed = await ledger.loadOwed(ctx.config, ctx.identity);
      for (let i = 0; i < owed.length; i += ledger.PER_CLAIM) {
        await ledger.submitIntent(ctx.config, ctx.identity, mk.claimIntent(mk.redeem(ctx.identity, owed.slice(i, i + ledger.PER_CLAIM))));
      }
      // Deposits from other chains waiting for this wallet (§146).
      const arrived = await ledger.claimWaiting(ctx.config, ctx.identity).catch(() => 0);
      return { collected: owed.length + arrived };
    },
  },

};

export { forgetIdentity, hex };
