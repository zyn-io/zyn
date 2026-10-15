// POST {sol, wallet}: the unsigned swap for one of the fixed amounts, quoted
// here so the page cannot choose another amount.
import { isWallet, json, quote, swap } from '../../lib/env.mjs';

export async function onRequestPost({ env, request }) {
  const { sol, wallet } = await request.json().catch(() => ({}));
  if (!isWallet(wallet)) return json({ error: 'wallet' }, 400);
  try {
    const q = await quote(env, sol);
    if (!q) return json({ error: 'amount' }, 400);
    return json({ tx: await swap(q, wallet), out: q.outAmount });
  } catch {
    return json({ error: 'swap' }, 502);
  }
}
