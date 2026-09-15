// The register, for the operator.
//
// Behind a bearer token rather than public: the applications are other
// people's handles, and while none of it is secret, a list of who wants in is
// not the applicants' idea of a published document.
//
// Read-only. Nothing here changes a record, so the worst a leaked token does
// is disclose what a reviewer would see — which is why it is a token of its
// own, unrelated to anything that can spend or sign.

export async function onRequestGet({ request, env }) {
  const expected = env.WHITELIST_ADMIN_TOKEN;
  if (!expected) {
    return json({ ok: false, error: 'no admin token configured' }, 503);
  }
  const given = (request.headers.get('authorization') || '').replace(/^Bearer\s+/i, '');
  if (!timingSafeEqual(given, expected)) {
    return json({ ok: false, error: 'no' }, 401);
  }

  const url = new URL(request.url);
  const limit = Math.min(Number(url.searchParams.get('limit') || 200), 1000);

  const listed = await env.CAVE_WHITELIST.list({ prefix: 'wl:', limit });
  const applications = [];
  for (const key of listed.keys) {
    const raw = await env.CAVE_WHITELIST.get(key.name);
    if (!raw) continue;
    try {
      applications.push(JSON.parse(raw));
    } catch {
      // A record that will not parse is worth knowing about, not worth
      // failing the whole listing over.
      applications.push({ handle: key.name.slice(3), broken: true });
    }
  }
  applications.sort((a, b) => String(a.at).localeCompare(String(b.at)));

  return json({
    ok: true,
    count: applications.length,
    truncated: !listed.list_complete,
    applications,
  });
}

/// Compare without leaking where two strings first differ.
function timingSafeEqual(a, b) {
  if (typeof a !== 'string' || typeof b !== 'string' || a.length !== b.length) {
    return false;
  }
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
  });
}
