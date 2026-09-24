// The object ledger, as the wallet's pages talk to it: one JSON-RPC call,
// the holder's own view of what they hold, and signing an intent.
//
// Shared by Nap Market and the connect window. Two copies of the claim-id
// derivation or the nonce counter would drift, and a drift here is money the
// wallet can no longer find — so there is one.

import { checkedEpoch } from './core.js';
import { hex, scan, signedFrame, submitSigned, unhex } from './object.js';
import * as mk from './market.js';

/// Epochs a signature stays valid. Long enough to survive a slow submit,
/// short enough that a frame found later is useless.
export const VALIDITY = 100n;
/// How far past the last nonce this device used to look for claims — another
/// device, or a restore, may have used some.
export const LOOKAHEAD = 24;
/// Claims per redemption: the VM's I/O limit.
export const PER_CLAIM = 16;

const read = (key) => { try { return localStorage.getItem(key); } catch { return null; } };
const write = (key, v) => { try { localStorage.setItem(key, v); } catch { /* this session only */ } };

export async function call(config, method, fields = {}) {
  const id = crypto.randomUUID();
  const response = await fetch(config.rpcUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id, method, params: [], ...fields }),
    cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer',
    signal: AbortSignal.timeout(15000),
  });
  if (!response.ok) throw new Error(`The chain answered HTTP ${response.status}`);
  const body = await response.json();
  if (body.id !== id) throw new Error('The chain answered a different question');
  if (body.error) throw new Error(body.error.message || 'The chain refused');
  return body.result;
}

// Nonces are receipts, so they are counted per chain and account and never
// handed out twice by this device — whichever page hands them out.
const counterKey = (config, identity) => `nap.market.n.${config.chain}.${identity.accountHex}`;
export const nonceCount = (config, identity) => Number(read(counterKey(config, identity)) || 0);
export function nextNonce(config, identity) {
  const n = nonceCount(config, identity);
  write(counterKey(config, identity), String(n + 1));
  return mk.marketNonce(identity.secretKey, n);
}

/// Every claim this wallet could be owed, from every nonce it may have used.
export function claimSources(identity, limit) {
  const out = [];
  const me = identity.account;
  for (let n = 0; n < limit; n++) {
    const nonce = mk.marketNonce(identity.secretKey, n);
    for (const source of [mk.orderId(me, nonce), mk.liquidityId(me, nonce), mk.commitId(me, nonce), mk.launchSource(me, nonce), mk.listingSource(me, nonce), mk.curveOrderId(me, nonce), mk.curveSource(me, nonce)]) {
      out.push(...mk.claimIds(source).map(hex));
    }
  }
  return out;
}

export async function loadOwed(config, identity) {
  const ids = claimSources(identity, nonceCount(config, identity) + LOOKAHEAD);
  const rows = [];
  for (let i = 0; i < ids.length; i += 240) {
    const part = await call(config, 'zyn_claims', { ids: ids.slice(i, i + 240).join(',') });
    if (Array.isArray(part)) rows.push(...part);
  }
  return rows.filter((r) => /^[0-9a-f]{64}$/.test(r.id) && /^[0-9a-f]{64}$/.test(r.asset) && /^\d+$/.test(r.amount));
}

/// Wait until notes a write spent are gone from this holder's view, so the
/// next write funds from what the chain now publishes (its change) rather
/// than from notes already spent. The node may seal the write before this
/// is even called, so this watches the notes, not the epoch.
export async function waitSpent(config, identity, commitments, timeoutMs = 90_000) {
  const gone = new Set(commitments);
  for (const until = Date.now() + timeoutMs; Date.now() < until;) {
    const { held } = await loadHoldings(config, identity, []);
    if (!held.some((h) => gone.has(h.commitment))) return;
    await new Promise((r) => setTimeout(r, 1500));
  }
  throw new Error('The chain has not taken the last step in yet; try the rest again shortly');
}

/// Tickets this wallet bought in a sale (§144). A ticket id derives from a
/// nonce only this wallet holds, so it asks by id: the first ticket of every
/// nonce it may have used, then the rest of each buy it finds.
export async function loadTickets(config, identity, collection) {
  const me = identity.account;
  const limit = nonceCount(config, identity) + LOOKAHEAD;
  const nonces = Array.from({ length: limit }, (_, n) => mk.marketNonce(identity.secretKey, n));
  const ask = async (ids) => {
    const rows = [];
    for (let i = 0; i < ids.length; i += 500) {
      const part = await call(config, 'zyn_tickets', { ids: ids.slice(i, i + 500).map((x) => x.id).join(',') });
      if (Array.isArray(part)) rows.push(...part);
    }
    const byId = new Map(ids.map((x) => [x.id, x]));
    return rows
      .filter((r) => isHex(r.ticket) && r.collection === collection && isSmall(r.index) && typeof r.claimed === 'boolean' && byId.has(r.ticket))
      .map((r) => ({ ...byId.get(r.ticket), index: r.index, claimed: r.claimed }));
  };
  const firsts = await ask(nonces.map((nonce) => ({ id: hex(mk.ticketId(me, nonce, 0)), nonce, k: 0 })));
  const rest = await ask(firsts.flatMap((t) => Array.from({ length: 19 }, (_, j) => ({ id: hex(mk.ticketId(me, t.nonce, j + 1)), nonce: t.nonce, k: j + 1 }))));
  return [...firsts, ...rest].map((t) => ({ ticket: t.id, nonce: t.nonce, k: t.k, index: t.index, claimed: t.claimed })).sort((a, b) => a.index - b.index);
}

/// A sale as the chain holds it, and the posted draw and open-mint order —
/// each checked against the root the chain holds, so the node serving them
/// cannot change who won or what sells next.
export async function loadSale(config, collection) {
  const market = await call(config, 'zyn_market');
  const sale = (market.sales || []).find((s) => s.collection === collection);
  if (!sale) return null;
  const terms = (market.collections || []).find((c) => c.asset === collection);
  const out = { sale, quote: terms && isHex(terms.quote) ? terms.quote : config.zecAsset, draw: null, open: null };
  if (sale.phase === 'drawn' || sale.phase === 'openmint') {
    const d = await call(config, 'zyn_saleDraw', { collection, kind: 'draw' });
    if (d && Array.isArray(d.won)) {
      const won = d.won.filter((w) => isHex(w.ticket) && Array.isArray(w.serials) && w.serials.length && w.serials.every(isSmall));
      const leaves = won.map((w) => mk.drawLeaf(unhex(w.ticket), w.serials));
      if (won.length === d.won.length && hex(mk.merkleRoot(leaves)) === sale.draw_root) out.draw = { won, leaves };
    }
  }
  if (sale.phase === 'openmint') {
    const o = await call(config, 'zyn_saleDraw', { collection, kind: 'open' });
    if (o && Array.isArray(o.order) && o.order.every(isSmall) && o.order.length === sale.open_count) {
      const leaves = o.order.map((s, i) => mk.openLeaf(i, s));
      if (hex(mk.merkleRoot(leaves)) === sale.open_root) out.open = { order: o.order, leaves };
    }
  }
  return out;
}

// What the node says goes into markup, ids and arithmetic, so its shape is
// checked first: a row that is not exactly hex ids and integer strings is
// dropped rather than rendered.
export const isHex = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
export const isInt = (v) => typeof v === 'string' && /^\d{1,40}$/.test(v);
export const isSmall = (v) => Number.isSafeInteger(v) && v >= 0;
export const validOffer = (o) => ['item', 'asset', 'price_asset'].every((k) => isHex(o[k]))
  && ['price_raw', 'fee_raw', 'pay_raw'].every((k) => isInt(o[k])) && isSmall(o.expires_at_epoch)
  && (o.collection === '' || isHex(o.collection)) && (o.nullifier === '' || isHex(o.nullifier)) && Number.isSafeInteger(o.serial);

/// The public order book, validated.
export async function loadOffers(config) {
  const book = await call(config, 'zyn_offers').catch(() => ({ offers: [] }));
  return (book?.offers || []).filter(validOffer);
}

/// What this holder holds, opened with their own keys, and which of it are
/// items (asked by asset, which says nothing a listing does not).
export async function loadHoldings(config, identity, offers) {
  const published = await call(config, 'zyn_published');
  const { held, balances } = scan(published, { secret: identity.viewing, nkey: identity.nullifier, owner: identity.account });
  const items = new Map();
  for (const o of offers) if (o.serial >= 0) items.set(o.asset, { collection: o.collection, serial: o.serial });
  const unknown = [...new Set(held.map((h) => h.asset))].filter((a) => a !== config.zecAsset && !items.has(a));
  if (unknown.length) {
    const found = await call(config, 'zyn_items', { assets: unknown.slice(0, 500).join(',') }).catch(() => []);
    for (const i of Array.isArray(found) ? found : []) {
      if (isHex(i.asset) && isHex(i.collection) && isSmall(i.serial)) items.set(i.asset, { collection: i.collection, serial: i.serial });
    }
  }
  return { held, balances, items };
}

/// Submit an intent that carries a nonce, taking the next nonce and trying
/// again whenever the chain says one was already used. A nonce is counted on
/// this device, so a restored phrase, a second device or a cleared browser
/// starts the count where another left off — and the chain, which refuses a
/// reused source, is the only place that knows. Returns the nonce that took.
/// Every way the chain says "this nonce is already taken" — by a claim or
/// order, by a resting listing, or by a coin (whose address is a hash of its
/// creator and nonce). tests/nonce.test.mjs checks these against the VM.
export const NONCE_TAKEN = /that nonce was already used|that nonce is in use|that token exists/;

export async function submitFresh(config, identity, build, tries = 64) {
  for (let i = 0; i < tries; i++) {
    const nonce = nextNonce(config, identity);
    try {
      await submitIntent(config, identity, build(nonce));
      return nonce;
    } catch (e) {
      if (!NONCE_TAKEN.test(String(e.message))) throw e;
    }
  }
  throw new Error('Every nonce tried was already used; this wallet has traded from somewhere else a lot. Try again.');
}

/// Sign an intent and hand it to the holders' door. A refusal inside the
/// receipts is an error too — the frame was accepted, the intent was not.
export async function submitIntent(config, identity, intent) {
  const status = await call(config, 'zyn_status');
  const epoch = checkedEpoch(status, config.chain);
  const frame = await signedFrame(identity.secretKey, config.chain, epoch + VALIDITY, intent);
  const result = await submitSigned(config.rpcUrl, frame);
  const refused = /Refused\("([^"]*)"\)/.exec(String(result?.receipts || ''));
  if (refused) throw new Error(`The chain refused: ${refused[1]}`);
  return result;
}
