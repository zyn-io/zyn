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
import { hex, nullifierOf, unhex } from './object.js';
import * as mk from './market.js';
import * as ledger from './ledger.js';
import { parsePaymentRequest, zatoshiOf } from './zip321.js';

/// EIP-1193's error codes, so a site handles Nap the way it handles any wallet.
export const ERR = Object.freeze({ REJECTED: 4001, UNAUTHORIZED: 4100, UNSUPPORTED: 4200, PARAMS: -32602 });
export class ProviderError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}
const bad = (m) => new ProviderError(ERR.PARAMS, m);

const ZAT = 100_000_000n;
const zecText = (zat) => { const v = BigInt(zat); const f = (v % ZAT).toString().padStart(8, '0').replace(/0+$/, ''); return `${v / ZAT}${f ? `.${f}` : ''}`; };
const wadText = (raw) => { const v = BigInt(raw); const f = (v % WAD).toString().padStart(18, '0').replace(/0+$/, '').slice(0, 8); return `${v / WAD}${f ? `.${f}` : ''}`; };
const isHex = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);

/// Collection Z's species share one index space (§86.1).
const CAVE_Z = '024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e';
export function itemName(collection, serial) {
  if (collection !== CAVE_Z) return `${collection.slice(0, 6)}… #${serial}`;
  const [species, start] = serial < 2222 ? ['Mole', 0] : serial < 3555 ? ['Owl', 2222] : ['Moth', 3555];
  return `Zyn ${species} #${String(serial - start + 1).padStart(4, '0')}`;
}

/// What the site learns on connecting: the account, and a Zcash address. Not
/// a balance, not a note, not a viewing key.
export async function session(ctx) {
  let zcash = null;
  if (ctx.zec) {
    const view = await ctx.zec();
    zcash = { network: view.network, address: view.address };
  }
  return { account: ctx.identity.accountHex, network: ctx.config.id, chain: ctx.config.chain, zcash };
}

async function holdings(ctx) {
  const offers = await ledger.loadOffers(ctx.config);
  const mine = await ledger.loadHoldings(ctx.config, ctx.identity, offers);
  return { offers, ...mine };
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

const tokenText = (raw) => { const v = BigInt(raw) / WAD; return v.toLocaleString('en-US'); };

export const METHODS = {
  nap_connect: { kind: 'connect' },
  nap_session: { kind: 'session' },
  nap_disconnect: { kind: 'session' },

  zyn_account: { kind: 'read', run: (ctx) => session(ctx) },
  zec_getAddress: {
    kind: 'read',
    run: async (ctx) => {
      if (!ctx.zec) throw new ProviderError(ERR.UNSUPPORTED, 'This wallet has no Zcash keys on this device yet');
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
      return {
        holdings: curves.map((c) => ({ token: c.token, symbol: c.symbol, name: c.name, amount: String(h.balances.get(c.token) || 0n) }))
          .filter((x) => x.amount !== '0'),
      };
    },
  },

  // --- writes ---------------------------------------------------------------

  zyn_curveCreate: {
    kind: 'write',
    async prepare(ctx, p) {
      const meta = mk.encodeMeta({ symbol: p.symbol, name: p.name });
      const { terms } = await curveBook(ctx);
      const bond = BigInt(terms.bond || '0');
      return {
        title: `Create ${String(p.symbol).toUpperCase()}`,
        rows: [
          ['Name', String(p.name).trim()],
          ['Supply', '1,000,000,000 — all sold on the curve or put in its pool; none to you'],
          ['Graduates', `at ${mk.CURVE.graduateAt.toLocaleString('en-US')} of 80,000 lots (about 17 ZEC.zy raised), into a locked pool`],
          ['Bond', bond ? `${wadText(bond)} ZEC.zy, returned when it graduates` : 'none'],
          ['Your share', terms.fee_bps ? `10% of its trading fees, paid when it graduates` : 'none (no fee is set)'],
        ],
        note: 'Anyone can buy it from the next epoch. You own none unless you buy, at the same price as everybody.',
        run: async () => {
          let inputs = [], change = [];
          if (bond > 0n) {
            const h = await holdings(ctx);
            ({ inputs, change } = mk.fund(ctx.identity, h.held, unhex(ctx.config.zecAsset), bond));
          }
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.createCurveIntent({
            creator: ctx.identity.account, nonce: n, quote: unhex(ctx.config.zecAsset), meta, inputs, change,
          }));
          return { token: hex(mk.tokenOf(ctx.identity.account, nonce)) };
        },
      };
    },
  },

  zyn_curveBuy: {
    kind: 'write',
    async prepare(ctx, p) {
      const { c } = await oneCurve(ctx, p.token);
      if (c.graduated) throw bad('That token has graduated; it trades in its pool (zyn_swap)');
      const amount = parseAmount(String(p.amount ?? ''));
      if (amount < mk.MIN_CURVE_BUY) throw bad('A buy is at least 0.001 ZEC.zy');
      const q = mk.previewCurveBuy(c.sold, amount, c.fee_bps);
      if (q.lots === 0) throw bad('That buys less than one lot at the current price');
      const minLots = p.minTokens != null ? BigInt(parseAmount(String(p.minTokens))) / mk.LOT : 0n;
      return {
        title: `Buy ${c.symbol} with ${wadText(amount)} ZEC.zy`,
        rows: [
          ['Token', `${c.name} (${c.symbol})`],
          ['If nobody else buys this epoch', `${tokenText(q.tokens)} tokens`],
          ['Fee', c.fee_bps ? `${wadText(q.fee)} ZEC.zy (${c.fee_bps / 100}%)` : 'none'],
          ['Unspent', `${wadText(q.refund)} ZEC.zy comes back`],
          ...(minLots ? [['At least', `${tokenText(minLots * mk.LOT)} tokens, or the whole buy comes back`]] : []),
        ],
        note: 'Everyone buying this epoch pays one price, set when it closes; nobody can buy ahead of you. Collect the tokens once it settles.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(ctx.config.zecAsset), amount);
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
      const { c } = await oneCurve(ctx, p.token);
      if (c.graduated) throw bad('That token has graduated; it trades in its pool (zyn_swap)');
      const tokens = parseAmount(String(p.tokens ?? ''));
      if (tokens % mk.LOT !== 0n) throw bad(`Sell whole lots of ${mk.CURVE.lotTokens.toLocaleString('en-US')} tokens`);
      const lots = Number(tokens / mk.LOT);
      const q = mk.previewCurveSell(c.sold, lots, c.fee_bps);
      return {
        title: `Sell ${tokenText(tokens)} ${c.symbol}`,
        rows: [
          ['If nobody else sells this epoch', `${wadText(q.net)} ZEC.zy`],
          ['Fee', c.fee_bps ? `${wadText(q.fee)} ZEC.zy (${c.fee_bps / 100}%)` : 'none'],
        ],
        note: 'Sellers this epoch share one price, set when it closes. Collect the ZEC.zy once it settles.',
        run: async () => {
          const h = await holdings(ctx);
          const { inputs, change } = mk.fund(ctx.identity, h.held, unhex(c.token), tokens);
          const nonce = await ledger.submitFresh(ctx.config, ctx.identity, (n) => mk.curveTradeIntent({
            trader: ctx.identity.account, token: unhex(c.token), buy: false, amountIn: tokens, minOut: 0n, inputs, change, nonce: n,
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
      if (!pool) throw bad('No such pool');
      const sellA = !!p.sellA;
      const amountIn = parseAmount(String(p.amount ?? ''));
      const quote = mk.quoteAlone(pool, sellA, amountIn);
      const slippage = Number.isInteger(p.slippageBps) && p.slippageBps >= 0 && p.slippageBps <= 1000 ? p.slippageBps : 100;
      const minOut = mk.withSlippage(quote, slippage);
      const sold = sellA ? pool.a : pool.b;
      return {
        title: `Swap in pool ${p.pool.slice(0, 8)}…`,
        rows: [['You put in', wadText(amountIn)], ['If nobody else trades this epoch', wadText(quote)], ['At least', `${wadText(minOut)} (${slippage / 100}% slippage)`]],
        note: 'Swaps in one epoch clear together at one price.',
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

  zec_pay: {
    kind: 'write',
    async prepare(ctx, p) {
      if (!ctx.zec) throw new ProviderError(ERR.UNSUPPORTED, 'This wallet has no Zcash keys on this device yet');
      let req;
      if (p.uri) req = parsePaymentRequest(p.uri);
      else {
        if (typeof p.address !== 'string' || !p.address) throw bad('address is required');
        req = { address: p.address.trim(), zatoshi: zatoshiOf(String(p.amount ?? '')), memo: String(p.memo || ''), message: '' };
      }
      if (req.zatoshi == null || req.zatoshi <= 0) throw bad('The request names no amount');
      const net = ctx.config.zcashPrefix === 'u1' ? 'mainnet' : 'testnet';
      const shielded = req.address.startsWith(ctx.config.zcashPrefix);
      if (!shielded && !/^t[a-z0-9]{20,}$/i.test(req.address)) throw bad(`That is not a ${net} Zcash address`);
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
      if (p.pay != null && parseAmount(String(p.pay)) !== pay) throw bad('The price changed since the page loaded; reload and try again');
      return {
        title: `Buy ${itemName(o.collection, o.serial)}`,
        rows: [['Price', `${wadText(o.price_raw)} ZEC.zy`], ['Collection fee', `${wadText(o.fee_raw)} ZEC.zy`], ['You pay', `${wadText(pay)} ZEC.zy`]],
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
      const price = parseAmount(String(p.price ?? ''));
      const h = await holdings(ctx);
      const { note, item } = heldItem(h, p.asset);
      if (h.offers.some((o) => o.item === note.commitment)) throw bad('That item is already listed');
      const market = await ledger.call(ctx.config, 'zyn_market');
      const terms = (market.collections || []).find((c) => c.asset === item.collection);
      const quote = terms && isHex(terms.quote) ? terms.quote : ctx.config.zecAsset;
      const feeBps = terms ? Number(terms.fee_bps) || 0 : 0;
      return {
        title: `List ${itemName(item.collection, item.serial)}`,
        rows: [['Price', `${wadText(price)} ZEC.zy`], ['Fee', feeBps ? `${feeBps / 100}% from each side` : 'none'], ['You receive', `${wadText(price - (price * BigInt(feeBps)) / 10_000n)} ZEC.zy when it sells`]],
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
      const share = terms && BigInt(terms.outstanding || '0') > 0n ? BigInt(terms.floor) / BigInt(terms.outstanding) : 0n;
      return {
        title: `Redeem ${itemName(item.collection, item.serial)} for the floor`,
        rows: [['Floor share', `${wadText(share)} ZEC.zy`], ['The item', 'is burned — this cannot be undone']],
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

  zyn_collect: {
    kind: 'write',
    async prepare(ctx) {
      const owed = await ledger.loadOwed(ctx.config, ctx.identity);
      if (!owed.length) throw bad('Nothing is owed to this wallet right now');
      const byAsset = new Map();
      for (const c of owed) byAsset.set(c.asset, (byAsset.get(c.asset) || 0n) + BigInt(c.amount));
      return {
        title: `Collect ${owed.length} payment${owed.length === 1 ? '' : 's'}`,
        rows: [...byAsset].map(([a, v]) => [a === ctx.config.zecAsset ? 'ZEC.zy' : `${a.slice(0, 8)}…`, wadText(v)]),
        note: 'Sale proceeds, refunds and floor shares the ledger is holding for you.',
        run: async () => {
          for (let i = 0; i < owed.length; i += ledger.PER_CLAIM) {
            await ledger.submitIntent(ctx.config, ctx.identity, mk.claimIntent(mk.redeem(ctx.identity, owed.slice(i, i + ledger.PER_CLAIM))));
          }
          return { collected: owed.length };
        },
      };
    },
  },
};

export { forgetIdentity, hex };
