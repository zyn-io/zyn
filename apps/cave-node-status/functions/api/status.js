// The one-way window into the node.
//
// The page is public; the way to reach the node is not published with it. A
// visitor gets six fields describing a tip and nothing they can call. The
// upstream lives in `ZEC_RPC_URL`, a Pages environment variable, so the name
// never appears in an asset the browser can read, in a redirect, or in an
// error body — the last one is the easy mistake, and it is why nothing from
// upstream is ever echoed back.
//
// This is not a claim that the RPC is secret. DECISIONS §53 made it public
// deliberately, default-deny by method. What this endpoint withholds is a
// working endpoint handed to every visitor of a proof-of-build page, and the
// traffic that comes with one.
//
// Deliberately absent: CORS. Same-origin only, matching the page's
// `connect-src 'self'`. An open header here would turn the status page into
// an RPC proxy for anyone who wanted one.

/// How long a reading may be reused before the node is asked again.
///
/// The page samples every 15s per visitor, so without this every reader costs
/// an upstream request and a crowd would spend the node's 8 r/s (§53.4) on
/// answering the same question. 5s is well inside the 120s the page treats as
/// fresh, so a shared reading is never a stale one.
const COALESCE_MS = 5_000;

/// Give up before the page does. Its fetch aborts at 5s; an answer that
/// arrives after that is one nobody is waiting for.
const UPSTREAM_TIMEOUT_MS = 4_000;

/// Best-effort, per-isolate. Cloudflare may run this in many isolates at
/// once, so this thins the upstream load rather than capping it.
let cached = null;

// One handler rather than `onRequestGet` plus a catch-all: when both are
// exported, which of them answers a GET is a detail of the Pages router, and
// the status endpoint is not the place to find out.
export async function onRequest({ request, env }) {
  if (request.method !== 'GET') {
    return new Response('method not allowed', { status: 405, headers: { allow: 'GET' } });
  }

  const upstream = env.ZEC_RPC_URL;
  if (!upstream) {
    return json({ ok: false, error: 'no upstream configured' }, 503);
  }

  const now = Date.now();
  if (cached && now - cached.at < COALESCE_MS) {
    return json(cached.body);
  }

  let info;
  const startedAt = Date.now();
  try {
    const response = await fetch(upstream, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'getblockchaininfo' }),
      signal: AbortSignal.timeout(UPSTREAM_TIMEOUT_MS),
    });
    if (!response.ok) throw new Error('upstream refused');
    const envelope = await response.json();
    if (envelope?.error) throw new Error('upstream returned an error');
    info = envelope?.result;
  } catch {
    // Whatever went wrong, the reader learns only that the node is out of
    // reach. The upstream's own words could name it.
    return json({ ok: false, error: 'node unreachable' }, 502);
  }
  const latencyMs = Date.now() - startedAt;

  let body;
  try {
    body = shapeStatus(info, new Date(), latencyMs);
  } catch {
    return json({ ok: false, error: 'node returned an unreadable tip' }, 502);
  }

  cached = { at: Date.now(), body };
  return json(body);
}

/// Reduce `getblockchaininfo` to the six fields the page reads.
///
/// Everything else Zebra reports — chainwork, size_on_disk, the value pools,
/// the peer-facing detail — is dropped here rather than filtered later. The
/// shape is the allowlist.
///
/// `network` is passed through as the node reports it. If the node were ever
/// not on mainnet this returns that honestly and the page refuses it; a
/// server-side assertion would only turn a wrong node into a dead endpoint.
export function shapeStatus(info, sampledAt, latencyMs) {
  if (!info || typeof info !== 'object') throw new TypeError('no result object');

  const network = String(info.chain ?? '');
  const height = Number(info.blocks);
  const tip = String(info.bestblockhash ?? '').toLowerCase();
  const estimated = Number(info.estimatedheight);

  if (!network) throw new TypeError('no chain');
  if (!Number.isSafeInteger(height) || height < 1) throw new TypeError('no height');
  if (!/^[0-9a-f]{64}$/.test(tip)) throw new TypeError('no tip hash');

  // A node caught up can report an estimate below its own tip — it knows the
  // chain better than the estimate does. Reporting a gap of zero is the
  // truthful reading of that, and the page requires estimate >= height.
  const estimatedHeight = Number.isSafeInteger(estimated)
    ? Math.max(height, estimated)
    : height;

  return {
    network,
    height,
    estimatedHeight,
    tip,
    sampledAt: sampledAt.toISOString(),
    latencyMs: Math.max(0, Math.round(latencyMs)),
  };
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
  });
}
