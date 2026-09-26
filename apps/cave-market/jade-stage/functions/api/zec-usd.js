// ZEC in US dollars, for market caps. Fetched here, not by visitors'
// browsers, so a price provider never sees who is looking; cached 5 minutes.
export async function onRequest({ request }) {
  const cache = caches.default;
  const key = new Request(new URL('/api/zec-usd', request.url).toString());
  const hit = await cache.match(key);
  if (hit) return hit;
  let usd = null;
  // Coinbase's public spot price needs no key (CoinGecko's now does); Kraken
  // if Coinbase does not answer.
  const sources = [
    ['https://api.coinbase.com/v2/prices/ZEC-USD/spot', (j) => j?.data?.amount],
    ['https://api.kraken.com/0/public/Ticker?pair=ZECUSD', (j) => Object.values(j?.result || {})[0]?.c?.[0]],
  ];
  for (const [url, pick] of sources) {
    try {
      const r = await fetch(url, { headers: { accept: 'application/json', 'user-agent': 'cave-jade/1 (+https://launch.cavebro.win)' } });
      const v = Number(pick(await r.json()));
      if (Number.isFinite(v) && v > 0) { usd = v; break; }
    } catch { /* the next source */ }
  }
  const response = new Response(JSON.stringify({ usd }), { headers: { 'content-type': 'application/json', 'cache-control': `public, max-age=${usd ? 300 : 30}` } });
  if (usd) await cache.put(key, response.clone());
  return response;
}
