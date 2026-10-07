// Receive one "take it on" for a CaveDAO role (Digger, Lookout, Lantern).
//
// Until Cave names carry roles, a role starts as a note to the team: which
// role, a contact, and what the member will do. Nothing else — no wallet, no
// account, no item — so taking on a role says nothing about what anyone holds.
//
// The guards are the whitelist's cheap ones (a honeypot, a dwell check, a
// per-address hourly limit, one entry per contact and role): a person reads
// every entry, so these keep the queue readable rather than decide anything.

const ROLES = new Set(['digger', 'lookout', 'lantern']);
const MAX_CONTACT = 80;
const MAX_NOTE = 280;
const PER_HOUR = 6;

export async function onRequestPost({ request, env }) {
  if (!env.CAVE_WHITELIST) return json({ ok: false, error: 'the register is not reachable — this is our fault, try again shortly' }, 503);
  let form;
  try {
    form = await request.json();
  } catch {
    return json({ ok: false, error: 'could not read the form' }, 400);
  }
  if (!form || typeof form !== 'object' || Array.isArray(form)) return json({ ok: false, error: 'could not read the form' }, 400);
  if (String(form.burrow || '').trim() !== '') return json({ ok: true });

  const role = String(form.role || '');
  // Control and direction characters can make a contact read as something else.
  const clean = (v, max) => String(v || '').replace(/[\p{Cc}\p{Cf}]/gu, ' ').replace(/\s+/g, ' ').trim().slice(0, max + 1);
  const contact = clean(form.contact, MAX_CONTACT);
  const note = clean(form.note, MAX_NOTE);
  if (!ROLES.has(role)) return json({ ok: false, error: 'pick Digger, Lookout or Lantern' }, 400);
  if (!contact || contact.length > MAX_CONTACT) return json({ ok: false, error: `a contact of up to ${MAX_CONTACT} characters` }, 400);
  if (!note || note.length > MAX_NOTE) return json({ ok: false, error: `say what you will do, in up to ${MAX_NOTE} characters` }, 400);
  const dwell = Number(form.dwell || 0);
  if (!Number.isFinite(dwell) || dwell < 3000) return json({ ok: false, error: 'that was too quick — have another look' }, 400);

  const ip = request.headers.get('cf-connecting-ip') || 'unknown';
  const hour = Math.floor(Date.now() / 3_600_000);
  const rl = `rl-role:${hour}:${ip}`;
  const seen = Number((await env.CAVE_WHITELIST.get(rl)) || 0);
  if (seen >= PER_HOUR) return json({ ok: false, error: 'too many from here — try again later' }, 429);

  const key = `role:${role}:${contact.toLowerCase()}`;
  if (await env.CAVE_WHITELIST.get(key)) return json({ ok: true, already: true });
  try {
    await env.CAVE_WHITELIST.put(rl, String(seen + 1), { expirationTtl: 7200 });
    await env.CAVE_WHITELIST.put(key, JSON.stringify({ role, contact, note, at: new Date().toISOString() }));
  } catch {
    return json({ ok: false, error: 'the register did not take that — nothing was saved, try again' }, 503);
  }
  return json({ ok: true });
}

export async function onRequestGet() {
  return json({ ok: false, error: 'POST a role here' }, 405);
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' } });
}
