// The checkout's only way to reach the sale service.
//
// `zyn-sale` listens on a private address and stays there: it holds the order
// book and the whitelist registry, and it talks to a node whose RPC is not
// public (§53.6). This relays exactly three calls to it and refuses everything
// else, so putting the checkout on the internet does not put the service there.
//
// The upstream is `ZYN_SALE_URL`, a Pages environment variable, so the service's
// address is not in this repo, not in a built asset, and not in any reply —
// same rule as the node status page (§107.3).

/// What the checkout may ask for. Anything not on this list does not exist as
/// far as the internet is concerned — including any route `zyn-sale` grows
/// later, which is the point of naming them rather than forwarding a prefix.
const ALLOWED = {
  'POST /quote': true,
  'POST /settle': true,
  'GET /order': true,
};

export async function onRequest({ request, env, params }) {
  const upstream = env.ZYN_SALE_URL;
  if (!upstream) {
    // Not an error state to hide: before the sale opens there is no service,
    // and the page says so plainly rather than looking broken.
    return json({ error: 'the sale is not open yet' }, 503);
  }

  const path = '/' + (Array.isArray(params.path) ? params.path.join('/') : params.path || '');
  const key = `${request.method} ${path}`;
  if (!ALLOWED[key]) {
    return json({ error: 'no such call' }, 404);
  }

  const url = new URL(request.url);
  const target = upstream.replace(/\/$/, '') + path + url.search;

  let response;
  try {
    response = await fetch(target, {
      method: request.method,
      headers: { 'content-type': 'application/json' },
      body: request.method === 'GET' ? undefined : await request.text(),
      signal: AbortSignal.timeout(10_000),
    });
  } catch {
    // Never the upstream's own words: they would name it.
    return json({ error: 'the sale service is not answering' }, 502);
  }

  // The service's replies are already the checkout's vocabulary — a refusal
  // says why in terms a buyer can act on — so they pass through as they are.
  const body = await response.text();
  return new Response(body, {
    status: response.status,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
  });
}

function json(body, status) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
  });
}
