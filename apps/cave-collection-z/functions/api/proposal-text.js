// ZCaveDAO proposal texts, by their hash (§172, §173).
//
// The ledger holds a proposal's SHA-256 text hash; this keeps the words so
// members can read them. Only texts the chain has a proposal for are kept or
// served (audit, 7 Oct 2026: an open store on cavebro.win would host anything),
// and every reader checks the hash again, so this store can lose a text but
// cannot change one.
//
//   POST /api/proposal-text  { text, network }  -> { hash } | 409 (no such proposal yet)
//   GET  /api/proposal-text?hash=<64 hex>&network=mainnet|testnet  -> { text } | 404

const MAX = 4000;
const PER_HOUR = 20;
const CAVE_Z = '024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e';
const RPC = { mainnet: 'https://zyn-rpc.cavebro.win/', testnet: 'https://zyn-rpc.cavebro.win/object' };

async function sha256Hex(text) {
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/// Whether the network's DAO has a proposal with this text hash: true,
/// false, or null when the chain did not answer.
export async function proposed(network, hash, fetcher = fetch) {
  try {
    const r = await fetcher(RPC[network], { method: 'POST', headers: { 'content-type': 'text/plain' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'zyn_dao', params: [], collection: CAVE_Z }) });
    const d = (await r.json())?.result;
    if (!d || !Array.isArray(d.proposals)) return d?.set === false ? false : null;
    return d.proposals.some((p) => p?.text === hash);
  } catch { return null; }
}

const net = (v) => (v === 'testnet' ? 'testnet' : 'mainnet');

export async function onRequestPost({ request, env }, fetcher = fetch) {
  if (!env.CAVE_WHITELIST) return json({ ok: false, error: 'the store is not reachable — this is our fault, try again shortly' }, 503);
  let body;
  try { body = await request.json(); } catch { return json({ ok: false, error: 'could not read that' }, 400); }
  // The same trimming Nap applies before hashing, so both hash the same words.
  const text = String(body?.text ?? '').trim();
  if (!text || text.length > MAX) return json({ ok: false, error: `a proposal is 1 to ${MAX} characters` }, 400);
  const network = net(body?.network);
  const hash = await sha256Hex(text);
  const key = `ptext:${network}:${hash}`;
  if (await env.CAVE_WHITELIST.get(key)) return json({ ok: true, hash });
  const ip = request.headers.get('cf-connecting-ip') || 'unknown';
  const rl = `rl-ptext:${Math.floor(Date.now() / 3_600_000)}:${ip}`;
  const seen = Number((await env.CAVE_WHITELIST.get(rl)) || 0);
  if (seen >= PER_HOUR) return json({ ok: false, error: 'too many from here — try again later' }, 429);
  const known = await proposed(network, hash, fetcher);
  if (known === null) return json({ ok: false, error: 'the chain did not answer; try again' }, 503);
  if (!known) return json({ ok: false, error: 'no proposal on the chain has this text yet' }, 409);
  try {
    await env.CAVE_WHITELIST.put(rl, String(seen + 1), { expirationTtl: 7200 });
    await env.CAVE_WHITELIST.put(key, text);
  } catch {
    return json({ ok: false, error: 'the store did not take that — try again' }, 503);
  }
  return json({ ok: true, hash });
}

export async function onRequestGet({ request, env }) {
  const url = new URL(request.url);
  const hash = url.searchParams.get('hash') || '';
  if (!/^[0-9a-f]{64}$/.test(hash)) return json({ ok: false, error: 'hash is 64 hex characters' }, 400);
  const text = env.CAVE_WHITELIST ? await env.CAVE_WHITELIST.get(`ptext:${net(url.searchParams.get('network'))}:${hash}`) : null;
  if (text === null) return json({ ok: false, error: 'no text under that hash' }, 404);
  return json({ ok: true, text }, 200, 'public, max-age=86400, immutable');
}

function json(body, status = 200, cache = 'no-store') {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': cache, 'x-content-type-options': 'nosniff' } });
}
