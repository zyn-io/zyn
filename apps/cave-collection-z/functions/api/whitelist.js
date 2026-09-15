// Receive one whitelist application.
//
// This collects an X handle and a link to the post the applicant engaged with,
// and nothing else. No email, no wallet address, no Zyn account — an
// invitation binds an account on first use (§74.13), so asking for one here
// would collect what the design deliberately avoids, before anyone has decided
// to buy.
//
// Applications are reviewed by hand. Nothing here decides anything, which is
// why it can be this simple: the worst a bad submission does is waste a moment
// of the reviewer's time.

const HANDLE = /^@?[A-Za-z0-9_]{1,15}$/;      // X's own rule
const MAX_NOTE = 280;

export async function onRequestPost({ request, env }) {
  let form;
  try {
    const ct = request.headers.get('content-type') || '';
    form = ct.includes('application/json')
      ? await request.json()
      : Object.fromEntries(await request.formData());
  } catch {
    return json({ ok: false, error: 'could not read the form' }, 400);
  }

  const handle = String(form.handle || '').trim().replace(/^@/, '');
  const post = String(form.post || '').trim();
  const note = String(form.note || '').trim().slice(0, MAX_NOTE);

  if (!HANDLE.test(handle)) {
    return json({ ok: false, error: 'that does not look like an X handle' }, 400);
  }
  // The link is the evidence a reviewer needs. Without it there is nothing to
  // check, so it is required rather than optional.
  if (!/^https?:\/\/(x\.com|twitter\.com)\/[^\s]+$/i.test(post)) {
    return json({ ok: false, error: 'paste the link to your repost or comment on X' }, 400);
  }

  const key = `wl:${handle.toLowerCase()}`;
  const existing = await env.CAVE_WHITELIST.get(key);
  if (existing) {
    // Idempotent: applying twice is not an error and does not create a second
    // entry, but it also does not overwrite what a reviewer may have annotated.
    return json({ ok: true, already: true });
  }

  await env.CAVE_WHITELIST.put(
    key,
    JSON.stringify({
      handle,
      post,
      note,
      at: new Date().toISOString(),
      // Coarse, for spotting a flood from one place. Not an identifier we act
      // on, and not shown to anyone.
      country: request.headers.get('cf-ipcountry') || null,
    }),
  );

  return json({ ok: true });
}

// A GET is how a person arrives at /api/whitelist by accident.
export async function onRequestGet() {
  return json({ ok: false, error: 'POST an application here' }, 405);
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      'content-type': 'application/json',
      'cache-control': 'no-store',
    },
  });
}
