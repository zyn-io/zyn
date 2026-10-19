// POST {pass, signature, wallet, signed}: checks the buy, then registers the
// device's pass with the den.
import { claim } from '../../lib/check.mjs';
import { isSignature, isWallet, json, marketOut, minLamports, rpc } from '../../lib/env.mjs';

export async function onRequestPost({ env, request }) {
  const body = await request.json().catch(() => ({}));
  // Nothing that is not a signature and a wallet goes on to the Solana node.
  if (!isSignature(body?.signature) || !isWallet(body?.wallet)) return json({ error: 'signed' }, 400);
  const db = env.CLAIMS;
  const r = await claim(body, {
    mint: env.MINT,
    minLamports: minLamports(env),
    quote: (lamports) => marketOut(env, lamports),
    getTx: (s) => rpc(env, 'getTransaction', [s, { encoding: 'jsonParsed', maxSupportedTransactionVersion: 1, commitment: 'confirmed' }]).catch(() => null),
    record: async (s, pass, wallet) => {
      const ins = await db.prepare('INSERT INTO claims (signature, pass, wallet, at) VALUES (?, ?, ?, ?) ON CONFLICT (signature) DO NOTHING')
        .bind(s, pass, wallet, Date.now()).run();
      if (ins.meta.changes === 1) return 'new';
      const row = await db.prepare('SELECT pass FROM claims WHERE signature = ?').bind(s).first();
      return row?.pass === pass ? 'same' : 'other';
    },
    register: async (pass) => {
      const d = await fetch(new URL('/door/passes', env.DEN_URL), {
        method: 'POST',
        headers: { authorization: `Bearer ${env.DEN_ISSUER_TOKEN}`, 'content-type': 'application/json' },
        body: JSON.stringify({ pass }),
      }).catch(() => null);
      return d?.ok ? { ok: true } : { error: d ? d.status : 'unreachable' };
    },
  });
  return r.ok ? json({ ok: true }) : json({ error: r.why }, r.status);
}
