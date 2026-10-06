// A launchpad token's picture and description (DECISIONS §141).
//
// Only the token's creator can attach them, once: the proof is the key that
// made the token (token = tokenOf(accountOf(key), nonce), the ledger's own
// derivation, checked with the wallet's code) signing the content's hash.
// The operator can hide a token's content (`hide:<token>` in the store); it
// is never edited.
//
//   GET  /api/content/<token>?network=mainnet|testnet  -> { description, image } | {} | 451 (hidden)
//   POST /api/content/<token>?network=…  { content: {description, image}, proof: {publicKey, nonce, signature, v?, chain?} }
//
// Per network (audit, 6 Oct 2026): one creator's coin can have the same
// address on testnet and mainnet, so content is kept per network, a coin must
// exist on that network's chain to get any, and the proof names the chain
// (v2). A v1 proof (an older Nap) is taken only for a coin with no twin on
// the other chain — the only case where it cannot be replayed across.

import { contentHash, hex, unhex, verifyContentProof } from '../../../../nap-web/src/object.js';

const MAX_IMAGE = 420_000; // a data URL of about 300 KB
const MAX_BODY = 450_000;
const MAX_DESCRIPTION = 400;
const MAX_SIDE = 1024;
const PER_HOUR = 20;
const IMAGE = /^data:image\/(png|jpeg|webp|gif);base64,[A-Za-z0-9+/=]+$/;
const NETWORKS = {
  mainnet: { chain: 26460, rpc: 'https://zyn-rpc.cavebro.win/' },
  testnet: { chain: 212, rpc: 'https://zyn-rpc.cavebro.win/object' },
};
const json = (body, status = 200, cache = 'no-store') =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': cache, 'x-content-type-options': 'nosniff' } });

export async function onRequest({ request, env, params }) {
  const store = env.JADE_CONTENT;
  if (!store) return json({ error: 'content is not configured here' }, 503);
  const token = (Array.isArray(params.path) ? params.path[0] : params.path || '').toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(token)) return json({ error: 'a token is 64 hex characters' }, 400);
  const url = new URL(request.url);
  const network = url.searchParams.get('network') === 'testnet' ? 'testnet' : 'mainnet';
  const key = `c:${network}:${token}`;

  if (request.method === 'GET') {
    // Served from the edge for five minutes; nothing attached for half a
    // minute, so a picture attached a moment later shows soon.
    const cache = caches.default;
    const cacheKey = new Request(`${url.origin}/api/content/${token}?network=${network}`);
    const hit = await cache.match(cacheKey);
    if (hit) return hit;
    let r;
    if (await store.get(`hide:${token}`)) r = json({ hidden: true }, 451, 'public, max-age=300');
    else {
      // Content attached before it was kept per network was all testnet's.
      const kept = (await store.get(key, 'json')) ?? (network === 'testnet' ? await store.get(`c:${token}`, 'json') : null);
      r = kept ? json({ description: kept.description, image: kept.image }, 200, 'public, max-age=300') : json({}, 200, 'public, max-age=30');
    }
    await cache.put(cacheKey, r.clone());
    return r;
  }
  if (request.method !== 'POST') return json({ error: 'GET or POST' }, 405);

  // The size, before anything reads the body.
  if (Number(request.headers.get('content-length') || 0) > MAX_BODY) return json({ error: 'too large' }, 413);
  let body;
  try {
    const text = await request.text();
    if (text.length > MAX_BODY) return json({ error: 'too large' }, 413);
    body = JSON.parse(text);
  } catch { return json({ error: 'send JSON' }, 400); }
  // Control and direction-override characters can make text say something
  // other than it reads as; they have no place in a description.
  const description = String(body?.content?.description || '').replace(/[\p{Cc}\p{Cf}]/gu, (c) => (c === '\n' ? c : '')).trim();
  const image = String(body?.content?.image || '');
  if (description.length > MAX_DESCRIPTION) return json({ error: `a description is at most ${MAX_DESCRIPTION} characters` }, 413);
  if (image && (!IMAGE.test(image) || image.length > MAX_IMAGE)) return json({ error: 'a PNG, JPEG, WebP or GIF under 300 KB' }, 413);
  if (image) {
    const size = imageSize(image);
    if (!size) return json({ error: 'that picture does not read as the type it says it is' }, 415);
    if (size.w > MAX_SIDE || size.h > MAX_SIDE) return json({ error: `a picture is at most ${MAX_SIDE}×${MAX_SIDE}` }, 413);
  }
  if (!description && !image) return json({ error: 'nothing to attach' }, 400);
  if (await store.get(key)) return json({ error: 'this token already has its content' }, 409);

  // The proof first: it costs CPU only, where every rate-limit count costs a
  // write, so a malformed or forged request costs the store nothing.
  const hash = contentHash({ description, image });
  const net = NETWORKS[network];
  const proof = body?.proof || {};
  const ok = await verifyContentProof(unhex(token), hash, proof, net.chain).catch(() => false);
  if (!ok) return json({ error: 'the proof does not show this key made this token' }, 403);
  const here = await coinExists(net.rpc, token);
  if (here === null) return json({ error: 'the chain did not answer; try again' }, 503);
  if (!here) return json({ error: `no coin has this address on ${network}` }, 404);
  if (proof.v !== 2) {
    const other = await coinExists(NETWORKS[network === 'mainnet' ? 'testnet' : 'mainnet'].rpc, token);
    if (other !== false) return json({ error: 'this coin has a twin on the other network; update Nap to attach content to it' }, 409);
  }

  const ip = request.headers.get('cf-connecting-ip') || 'unknown';
  const hour = Math.floor(Date.now() / 3_600_000);
  try {
    const seen = Number((await store.get(`rl:${hour}:${ip}`)) || 0);
    if (seen >= PER_HOUR) return json({ error: 'too many uploads from here this hour' }, 429);
    await store.put(`rl:${hour}:${ip}`, String(seen + 1), { expirationTtl: 7200 });
    await store.put(key, JSON.stringify({ description, image, hash: hex(hash), at: Date.now() }));
  } catch {
    return json({ error: 'the store did not take that; try again in a moment' }, 503);
  }
  return json({ attached: true, hash: hex(hash) }, 201);
}

/// Whether a coin with this address exists on the chain: true, false, or
/// null when the chain did not answer.
async function coinExists(rpc, token) {
  try {
    const r = await fetch(rpc, { method: 'POST', headers: { 'content-type': 'text/plain' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'zyn_market', params: [] }) });
    const m = (await r.json())?.result;
    if (!m || !Array.isArray(m.curves)) return null;
    return m.curves.some((c) => c.token === token);
  } catch { return null; }
}

/// A picture's width and height from its own header, or null when the bytes
/// are not the type the data URL names.
export function imageSize(dataUrl) {
  const [, type, b64] = dataUrl.match(/^data:image\/(png|jpeg|webp|gif);base64,(.+)$/) || [];
  if (!type) return null;
  let b;
  try { b = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)); } catch { return null; }
  const u16 = (i, le) => (le ? b[i] | (b[i + 1] << 8) : (b[i] << 8) | b[i + 1]);
  const u32 = (i) => ((b[i] << 24) | (b[i + 1] << 16) | (b[i + 2] << 8) | b[i + 3]) >>> 0;
  if (type === 'png') return b[0] === 0x89 && b[1] === 0x50 && b.length > 24 ? { w: u32(16), h: u32(20) } : null;
  if (type === 'gif') return b[0] === 0x47 && b[1] === 0x49 && b.length > 10 ? { w: u16(6, true), h: u16(8, true) } : null;
  if (type === 'webp') {
    if (b.length < 30 || String.fromCharCode(...b.slice(8, 12)) !== 'WEBP') return null;
    const chunk = String.fromCharCode(...b.slice(12, 16));
    if (chunk === 'VP8 ') return { w: u16(26, true) & 0x3fff, h: u16(28, true) & 0x3fff };
    if (chunk === 'VP8L') return { w: 1 + (((b[22] & 0x3f) << 8) | b[21]), h: 1 + (((b[24] & 0x0f) << 10) | (b[23] << 2) | ((b[22] & 0xc0) >> 6)) };
    if (chunk === 'VP8X') return { w: 1 + (b[24] | (b[25] << 8) | (b[26] << 16)), h: 1 + (b[27] | (b[28] << 8) | (b[29] << 16)) };
    return null;
  }
  // JPEG: walk the segments to the frame header.
  if (b[0] !== 0xff || b[1] !== 0xd8) return null;
  for (let i = 2; i + 9 < b.length;) {
    if (b[i] !== 0xff) return null;
    const marker = b[i + 1];
    const len = u16(i + 2);
    if (marker >= 0xc0 && marker <= 0xcf && ![0xc4, 0xc8, 0xcc].includes(marker)) return { h: u16(i + 5), w: u16(i + 7) };
    i += 2 + len;
  }
  return null;
}
