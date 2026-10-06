// Link a whitelist invitation to a Zyn account (DECISIONS §74.13, §168).
//
// The code is the secret: whoever holds it chooses the account, once. Nothing
// proves the account is theirs, and nothing needs to — linking someone else's
// account only gives that account the right to buy. A code already linked to
// another account is refused; linking it to the same account again is fine.
// One account backs one member. The operator then puts linked accounts on the
// sale's list on Zyn (`invite.mjs linked` → `zyn-sale-draw allow`).
//
// The members live in their own database (D1 `cave-whitelist`, binding
// CAVE_MEMBERS); the guessing limit uses the applications' KV, as the sign-up
// form's limit does.

const ACCOUNT = /^[0-9a-f]{64}$/;
const PER_HOUR = 20;

export async function onRequestPost({ request, env }) {
  if (!env.CAVE_MEMBERS) return json({ ok: false, error: 'the whitelist is not reachable — try again shortly' }, 503);
  let body;
  try { body = await request.json(); } catch { return json({ ok: false, error: 'could not read the request' }, 400); }
  const code = String(body?.code || '').toUpperCase().replace(/[^A-Z0-9]/g, '');
  const account = String(body?.account || '').toLowerCase();
  if (code.length < 8 || code.length > 32) return json({ ok: false, error: 'that does not look like a whitelist code' }, 400);
  if (!ACCOUNT.test(account)) return json({ ok: false, error: 'connect Nap first' }, 400);

  // Guessing codes is the attack worth slowing: a few tries an hour per address.
  if (env.CAVE_WHITELIST) {
    const ip = request.headers.get('cf-connecting-ip') || 'unknown';
    const rl = `rl-bind:${Math.floor(Date.now() / 3_600_000)}:${ip}`;
    const seen = Number((await env.CAVE_WHITELIST.get(rl)) || 0);
    if (seen >= PER_HOUR) return json({ ok: false, error: 'too many tries from here — try again later' }, 429);
    await env.CAVE_WHITELIST.put(rl, String(seen + 1), { expirationTtl: 7200 });
  }

  const hash = await sha256(`cave-invite:${code}`);
  const member = await env.CAVE_MEMBERS.prepare('SELECT handle, account FROM members WHERE code_hash = ?').bind(hash).first();
  if (!member) return json({ ok: false, error: 'that code is not valid' }, 404);
  if (member.account === account) return json({ ok: true, already: true });
  if (member.account) return json({ ok: false, error: 'that code is already linked to another wallet' }, 409);
  try {
    // Linked only if still unlinked: two tabs racing with one code cannot both win.
    const r = await env.CAVE_MEMBERS.prepare('UPDATE members SET account = ?, linked_at = ? WHERE code_hash = ? AND account IS NULL')
      .bind(account, new Date().toISOString(), hash).run();
    if (!r.meta?.changes) return json({ ok: false, error: 'that code is already linked to another wallet' }, 409);
  } catch (e) {
    if (/UNIQUE/i.test(String(e?.message))) return json({ ok: false, error: 'this wallet is already linked to another invitation' }, 409);
    return json({ ok: false, error: 'the whitelist did not accept that — try again' }, 503);
  }
  return json({ ok: true });
}

export async function onRequestGet() {
  return json({ ok: false, error: 'POST a code and an account here' }, 405);
}

async function sha256(text) {
  const d = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text)));
  return [...d].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' } });
}
