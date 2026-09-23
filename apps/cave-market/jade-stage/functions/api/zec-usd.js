// ZEC in US dollars, for market caps. Fetched here, not by visitors'
// browsers, so a price provider never sees who is looking; cached 5 minutes.
export async function onRequest({ request }) {
  const cache = caches.default;
  const key = new Request(new URL('/api/zec-usd', request.url).toString());
  const hit = await cache.match(key);
  if (hit) return hit;
  let usd = null;
  try {
    const r = await fetch('https://api.coingecko.com/api/v3/simple/price?ids=zcash&vs_currencies=usd', { headers: { accept: 'application/json', 'user-agent': 'cave-jade/1 (+https://launch.cavebro.win)' } });
    usd = Number((await r.json())?.zcash?.usd) || null;
  } catch { /* no price: the page shows ZEC only */ }
  const response = new Response(JSON.stringify({ usd }), { headers: { 'content-type': 'application/json', 'cache-control': `public, max-age=${usd ? 300 : 30}` } });
  if (usd) await cache.put(key, response.clone());
  return response;
}
