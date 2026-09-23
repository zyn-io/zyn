// A launchpad token's picture and description (DECISIONS §141).
//
// Only the token's creator can attach them, once: the proof is the key that
// made the token (token = tokenOf(accountOf(key), nonce), the ledger's own
// derivation, checked with the wallet's code) signing the content's hash.
// The operator can hide a token's content (`hide:<token>` in the store); it
// is never edited.
//
//   GET  /api/content/<token>   -> { description, image } | 404 | 451 (hidden)
//   POST /api/content/<token>   { content: {description, image}, proof: {publicKey, nonce, signature} }

import { contentHash, hex, unhex, verifyContentProof } from '../../../../../nap-web/src/object.js';

const MAX_IMAGE = 420_000; // a data URL of about 300 KB
const MAX_DESCRIPTION = 400;
const PER_HOUR = 20;
const IMAGE = /^data:image\/(png|jpeg|webp|gif);base64,[A-Za-z0-9+/=]+$/;
const json = (body, status = 200, cache = 'no-store') =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': cache, 'x-content-type-options': 'nosniff' } });

export async function onRequest({ request, env, params }) {
  const store = env.CAVE_JADE_CONTENT;
  if (!store) return json({ error: 'content is not configured here' }, 503);
  const token = (Array.isArray(params.path) ? params.path[0] : params.path || '').toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(token)) return json({ error: 'a token is 64 hex characters' }, 400);

  if (request.method === 'GET') {
    if (await store.get(`hide:${token}`)) return json({ hidden: true }, 451);
    const kept = await store.get(`c:${token}`, 'json');
    return kept ? json({ description: kept.description, image: kept.image }, 200, 'public, max-age=300') : json({ error: 'nothing attached' }, 404);
  }
  if (request.method !== 'POST') return json({ error: 'GET or POST' }, 405);

  const ip = request.headers.get('cf-connecting-ip') || 'unknown';
  const hour = Math.floor(Date.now() / 3_600_000);
  const seen = Number((await store.get(`rl:${hour}:${ip}`)) || 0);
  if (seen >= PER_HOUR) return json({ error: 'too many uploads from here this hour' }, 429);
  await store.put(`rl:${hour}:${ip}`, String(seen + 1), { expirationTtl: 7200 });

  let body;
  try { body = await request.json(); } catch { return json({ error: 'send JSON' }, 400); }
  const description = String(body?.content?.description || '').trim();
  const image = String(body?.content?.image || '');
  if (description.length > MAX_DESCRIPTION) return json({ error: `a description is at most ${MAX_DESCRIPTION} characters` }, 413);
  if (image && (!IMAGE.test(image) || image.length > MAX_IMAGE)) return json({ error: 'a PNG, JPEG, WebP or GIF under 300 KB' }, 413);
  if (!description && !image) return json({ error: 'nothing to attach' }, 400);
  if (await store.get(`c:${token}`)) return json({ error: 'this token already has its content' }, 409);

  const hash = contentHash({ description, image });
  const ok = await verifyContentProof(unhex(token), hash, body?.proof || {}).catch(() => false);
  if (!ok) return json({ error: 'the proof does not show this key made this token' }, 403);
  await store.put(`c:${token}`, JSON.stringify({ description, image, hash: hex(hash), at: Date.now() }));
  return json({ attached: true, hash: hex(hash) }, 201);
}
