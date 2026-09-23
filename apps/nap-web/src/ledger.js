// The object ledger, as the wallet's pages talk to it: one JSON-RPC call,
// the holder's own view of what they hold, and signing an intent.
//
// Shared by Nap Market and the connect window. Two copies of the claim-id
// derivation or the nonce counter would drift, and a drift here is money the
// wallet can no longer find — so there is one.

import { checkedEpoch } from './core.js';
import { hex, scan, signedFrame, submitSigned } from './object.js';
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
