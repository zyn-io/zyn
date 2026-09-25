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

/// What an approval calls an asset: ZEC.zy, a bridged coin, a curve token by
/// its symbol, else its short address.
async function namer(ctx, market) {
  const bridged = await ledger.loadBridged(ctx.config);
  return (a) => {
    if (a === ctx.config.zecAsset) return 'ZEC.zy';
    const b = bridged.find((x) => x.asset === a);
    if (b) return ledger.bridgedName(ctx.config, b);
    const c = (market.curves || []).find((x) => x.token === a);
    const meta = c && isHex(c.meta) ? mk.decodeMeta(c.meta) : null;
    return meta?.symbol || `${a.slice(0, 8)}…`;
  };
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
      // The tokens this wallet made: its own nonces, turned into token ids.
      const mine = new Set();
      const count = ledger.nonceCount(ctx.config, ctx.identity) + ledger.LOOKAHEAD;
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
      return {
        title: `Create ${String(p.symbol).toUpperCase()}`,
        rows: [
          ['Name', String(p.name).trim()],
          ['Supply', '1,000,000,000 — all sold on the curve or put in its pool; none to you'],
          ['Graduates', `at ${mk.CURVE.graduateAt.toLocaleString('en-US')} of 80,000 lots (about 17 ZEC.zy raised), into a locked pool`],
          ['Bond', bond ? `${wadText(bond)} ZEC.zy, returned when it graduates` : 'none'],
          ['Your share', terms.fee_bps ? `10% of its trading fees, paid when it graduates` : 'none (no fee is set)'],
          ...(description ? [['Description', description]] : []),
          ...(image ? [['Picture', 'shown above, stored by the site that asked; it proves you made the token']] : []),
        ],
        image: image || null,
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
      const amount = parseAmount(String(p.amount ?? ''));
      if (amount < mk.MIN_CURVE_BUY) throw bad(`A buy is at least 0.001 ${q$}`);
      const q = mk.previewCurveBuy(c.sold, amount, c.fee_bps);
      if (q.lots === 0) throw bad('That buys less than one lot at the current price');
      const minLots = p.minTokens != null ? BigInt(parseAmount(String(p.minTokens))) / mk.LOT : 0n;
      return {
        title: `Buy ${c.symbol} with ${wadText(amount)} ${q$}`,
        rows: [
          ['Token', `${c.name} (${c.symbol})`],
          ['If nobody else buys this epoch', `${tokenText(q.tokens)} tokens`],
          ['Fee', c.fee_bps ? `${wadText(q.fee)} ${q$} (${c.fee_bps / 100}%)` : 'none'],
          ['Unspent', `${wadText(q.refund)} ${q$} comes back`],
          ...(minLots ? [['At least', `${tokenText(minLots * mk.LOT)} tokens, or the whole buy comes back`]] : []),
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
      const tokens = parseAmount(String(p.tokens ?? ''));
      if (tokens % mk.LOT !== 0n) throw bad(`Sell whole lots of ${mk.CURVE.lotTokens.toLocaleString('en-US')} tokens`);
      const lots = Number(tokens / mk.LOT);
      const q = mk.previewCurveSell(c.sold, lots, c.fee_bps);
      const minZec = p.minZec != null ? parseAmount(String(p.minZec)) : 0n;
      return {
        title: `Sell ${tokenText(tokens)} ${c.symbol}`,
        rows: [
          ['If nobody else sells this epoch', `${wadText(q.net)} ${q$}`],
          ['Fee', c.fee_bps ? `${wadText(q.fee)} ${q$} (${c.fee_bps / 100}%)` : 'none'],
          ...(minZec ? [['At least', `${wadText(minZec)} ${q$}, or the tokens come back`]] : []),
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
      if (!pool) throw bad('No such pool');
      const sellA = !!p.sellA;
      const amountIn = parseAmount(String(p.amount ?? ''));
      const quote = mk.quoteAlone(pool, sellA, amountIn);
      const slippage = Number.isInteger(p.slippageBps) && p.slippageBps >= 0 && p.slippageBps <= 1000 ? p.slippageBps : 100;
      const minOut = mk.withSlippage(quote, slippage);
      const sold = sellA ? pool.a : pool.b;
      const name = await namer(ctx, market);
      const [inName, outName] = [name(sold), name(sellA ? pool.b : pool.a)];
      return {
        title: `Swap ${inName} for ${outName}`,
        rows: [['You put in', `${wadText(amountIn)} ${inName}`], ['If nobody else trades this epoch', `${wadText(quote)} ${outName}`], ['At least', `${wadText(minOut)} ${outName} (${slippage / 100}% slippage)`], ['Pool', `${p.pool.slice(0, 8)}…`]],
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
    run: async (ctx) => ledger.burnMemo(ctx.config, ctx.identity),
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
      if (p.pay != null && parseAmount(String(p.pay)) !== total) throw bad('The price changed since the page loaded; reload and try again');
      return {
        title: `Buy ${count} ticket${count > 1 ? 's' : ''} in the ${collection === CAVE_Z ? 'Collection Z' : `${collection.slice(0, 6)}…`} sale`,
        rows: [['Per ticket', `${wadText(s.sale.ticket_price)} ZEC.zy`], ['Tickets', String(count)], ['You pay', `${wadText(total)} ZEC.zy`]],
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
      return {
        title: `Claim ${serials.length} item${serials.length > 1 ? 's' : ''}`,
        rows: [...serials.slice(0, 6).map((x) => ['Item', itemName(collection, x)]), ...(serials.length > 6 ? [['…', `and ${serials.length - 6} more`]] : []),
          ['Per item', `${wadText(s.sale.claim_price)} ZEC.zy`], ['You pay', `${wadText(total)} ZEC.zy`]],
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
      return {
        title: `Mint ${count} from the open mint`,
        rows: [['Per item', `${wadText(s.sale.open_price)} ZEC.zy`], ['Items', String(count)], ['You pay', `${wadText(total)} ZEC.zy`], ['Left', String(left)]],
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
