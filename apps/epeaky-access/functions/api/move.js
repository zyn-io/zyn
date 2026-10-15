// POST {pass, wallet, at, signed}: a wallet that bought before moves its
// access to this device (once a day); the device that had it is taken out.
import { move } from '../../lib/check.mjs';
import { json } from '../../lib/env.mjs';

export async function onRequestPost({ env, request }) {
  const body = await request.json().catch(() => ({}));
  const db = env.CLAIMS;
  const door = (method, path, body) =>
    fetch(new URL(path, env.DEN_URL), {
      method,
      headers: { authorization: `Bearer ${env.DEN_ISSUER_TOKEN}`, 'content-type': 'application/json' },
      body: body && JSON.stringify(body),
    }).catch(() => null);
  const r = await move(body, {
    latest: (w) => db.prepare('SELECT signature, pass, moved_at FROM claims WHERE wallet = ? ORDER BY at DESC LIMIT 1').bind(w).first(),
    register: async (pass) => ({ ok: (await door('POST', '/door/passes', { pass }))?.ok === true }),
    swap: (s, pass, at) => db.prepare('UPDATE claims SET pass = ?, moved_at = ? WHERE signature = ?').bind(pass, at, s).run(),
    unregister: (pass) => door('DELETE', `/door/passes/${pass}`),
  });
  return r.ok ? json({ ok: true }) : json({ error: r.why, next: r.next }, r.status);
}
