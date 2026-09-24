// The two Solana reads a burn needs (DECISIONS §145), relayed so the page's
// policy names only this site: the latest blockhash, and the buyer's token
// accounts for the sale's one mint. Nothing else is forwarded, and nothing is
// sent — the buyer's own wallet signs and sends the burn.
//
// `SOLANA_RPC` / `SOLANA_RPC_DEVNET` (optional) point it at a paid endpoint;
// the public ones answer otherwise.

const PUBLIC = { mainnet: 'https://api.mainnet-beta.solana.com', devnet: 'https://api.devnet.solana.com' };
const ADDRESS = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/;

function allowed(method, params) {
  if (method === 'getLatestBlockhash') return Array.isArray(params) && params.length <= 1;
  if (method === 'getTokenAccountsByOwner') {
    return Array.isArray(params) && params.length === 3 && ADDRESS.test(params[0])
      && params[1] && Object.keys(params[1]).length === 1 && ADDRESS.test(params[1].mint || '');
  }
  return false;
}

export async function onRequestPost({ request, env }) {
  const body = await request.json().catch(() => null);
  const cluster = new URL(request.url).searchParams.get('cluster') === 'devnet' ? 'devnet' : 'mainnet';
  if (!body || !allowed(body.method, body.params)) {
    return Response.json({ error: 'only a blockhash and token accounts by mint are relayed' }, { status: 400 });
  }
  const upstream = (cluster === 'devnet' ? env.SOLANA_RPC_DEVNET : env.SOLANA_RPC) || PUBLIC[cluster];
  try {
    const r = await fetch(upstream, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: body.method, params: body.params }),
    });
    const j = await r.json();
    return Response.json(j.error ? { error: 'Solana refused that read' } : { result: j.result });
  } catch {
    return Response.json({ error: 'Solana is not answering; try again shortly' }, { status: 502 });
  }
}
