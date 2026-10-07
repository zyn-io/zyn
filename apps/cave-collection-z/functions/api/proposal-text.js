// CaveDAO proposal texts, by their hash (§172).
//
// The ledger holds a proposal's SHA-256 text hash; this keeps the words so
// members can read them. It is content-addressed: a text is stored only under
// its own hash, and every reader checks the hash again, so this store can lose
// a text but cannot change one.
//
//   POST /api/proposal-text  { text }  -> { hash }
//   GET  /api/proposal-text?hash=<64 hex>  -> { text } | 404

const MAX = 4000;
const PER_HOUR = 10;

async function sha256Hex(text) {
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

export async function onRequestPost({ request, env }) {
  if (!env.CAVE_WHITELIST) return json({ ok: false, error: 'the store is not reachable — this is our fault, try again shortly' }, 503);
  let body;
  try { body = await request.json(); } catch { return json({ ok: false, error: 'could not read that' }, 400); }
  // The same trimming Nap applies before hashing, so both hash the same words.
  const text = String(body?.text ?? '').trim();
  if (!text || text.length > MAX) return json({ ok: false, error: `a proposal is 1 to ${MAX} characters` }, 400);
  const ip = request.headers.get('cf-connecting-ip') || 'unknown';
  const rl = `rl-ptext:${Math.floor(Date.now() / 3_600_000)}:${ip}`;
  const seen = Number((await env.CAVE_WHITELIST.get(rl)) || 0);
  if (seen >= PER_HOUR) return json({ ok: false, error: 'too many from here — try again later' }, 429);
  const hash = await sha256Hex(text);
  try {
    await env.CAVE_WHITELIST.put(rl, String(seen + 1), { expirationTtl: 7200 });
    if (!(await env.CAVE_WHITELIST.get(`ptext:${hash}`))) await env.CAVE_WHITELIST.put(`ptext:${hash}`, text);
  } catch {
    return json({ ok: false, error: 'the store did not take that — try again' }, 503);
  }
  return json({ ok: true, hash });
}

export async function onRequestGet({ request, env }) {
  const hash = new URL(request.url).searchParams.get('hash') || '';
  if (!/^[0-9a-f]{64}$/.test(hash)) return json({ ok: false, error: 'hash is 64 hex characters' }, 400);
  const text = env.CAVE_WHITELIST ? await env.CAVE_WHITELIST.get(`ptext:${hash}`) : null;
  if (text === null) return json({ ok: false, error: 'no text under that hash' }, 404);
  return json({ ok: true, text }, 200, 'public, max-age=86400, immutable');
}

function json(body, status = 200, cache = 'no-store') {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': cache } });
}
