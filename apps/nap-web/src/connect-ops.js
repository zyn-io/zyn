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

  // --- writes ---------------------------------------------------------------

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
          await ctx.zec();
          const v = await ctx.zecAccount().send(req.address, req.zatoshi, req.memo, onStep);
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
          await ledger.submitIntent(ctx.config, ctx.identity, mk.listIntent({
            seller: ctx.identity.account, item: c, itemNullifier: nullifierOf(ctx.identity.nullifier, c),
            priceAsset: unhex(quote), priceAmount: price, expiresAtEpoch: 0xffffffffn, nonce: ledger.nextNonce(ctx.config, ctx.identity),
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
