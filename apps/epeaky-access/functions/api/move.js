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
    // Only if the row is still as it was read: one of two moves at once wins.
    take: async (s, from, to, at, before) => (await db.prepare('UPDATE claims SET pass = ?, moved_at = ? WHERE signature = ? AND pass = ? AND (moved_at IS NULL OR moved_at < ?)').bind(to, at, s, from, before).run()).meta.changes === 1,
    restore: (s, from, to, movedAt) => db.prepare('UPDATE claims SET pass = ?, moved_at = ? WHERE signature = ? AND pass = ?').bind(to, movedAt, s, from).run(),
    register: async (pass) => ({ ok: (await door('POST', '/door/passes', { pass }))?.ok === true }),
    unregister: async (pass) => ({ ok: (await door('DELETE', `/door/passes/${pass}`))?.ok === true }),
  });
  // A pass the den would not take out: logged, for the host to take out by hand.
  if (r.left) console.error(`epeaky-access: move: the den did not take out pass ${r.left}`);
  return r.ok ? json({ ok: true }) : json({ error: r.why, next: r.next }, r.status);
}
