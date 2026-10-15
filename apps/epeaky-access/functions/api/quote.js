// GET ?sol=0.1: what that amount buys now.
import { json, quote } from '../../lib/env.mjs';

export async function onRequestGet({ env, request }) {
  const sol = new URL(request.url).searchParams.get('sol');
  try {
    const q = await quote(env, sol);
    if (!q) return json({ error: 'amount' }, 400);
    return json({ sol, out: q.outAmount, impact: Number(q.priceImpactPct) });
  } catch {
    return json({ error: 'quote' }, 502);
  }
}
