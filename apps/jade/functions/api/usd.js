// Dollars per coin a Jade coin can be priced in (§166): ZEC, ETH, SOL, BOLD,
// BTC. Fetched here, not by visitors' browsers, so a price provider never
// sees who is looking; cached 5 minutes. DefiLlama answers all of them in
// one request and needs no key; Coinbase's spot price fills any it missed
// (it does not list BOLD).
const LLAMA = {
  ZEC: 'coingecko:zcash',
  ETH: 'coingecko:ethereum',
  SOL: 'coingecko:solana',
  BTC: 'coingecko:bitcoin',
  BOLD: 'ethereum:0x6440f144b7e50D6a8439336510312d2F54beB01D',
};
const HEADERS = { accept: 'application/json', 'user-agent': 'zyn-jade/1 (+https://jade.zyn-network.com)' };

export async function onRequest({ request }) {
  const cache = caches.default;
  const key = new Request(new URL('/api/usd', request.url).toString());
  const hit = await cache.match(key);
  if (hit) return hit;
  const usd = {};
  try {
    const r = await fetch(`https://coins.llama.fi/prices/current/${Object.values(LLAMA).join(',')}`, { headers: HEADERS });
    const coins = (await r.json())?.coins || {};
    for (const [sym, id] of Object.entries(LLAMA)) {
      const v = Number(coins[id]?.price ?? coins[id.toLowerCase()]?.price);
      if (Number.isFinite(v) && v > 0) usd[sym] = v;
    }
  } catch { /* Coinbase below */ }
  for (const sym of ['ZEC', 'ETH', 'SOL', 'BTC'].filter((s) => !usd[s])) {
    try {
      const r = await fetch(`https://api.coinbase.com/v2/prices/${sym}-USD/spot`, { headers: HEADERS });
      const v = Number((await r.json())?.data?.amount);
      if (Number.isFinite(v) && v > 0) usd[sym] = v;
    } catch { /* unpriced: shown in its own coin */ }
  }
  const ok = Object.keys(usd).length > 0;
  const response = new Response(JSON.stringify({ usd }), { headers: { 'content-type': 'application/json', 'cache-control': `public, max-age=${ok ? 300 : 30}` } });
  if (ok) await cache.put(key, response.clone());
  return response;
}
