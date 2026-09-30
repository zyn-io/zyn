// Receive one whitelist application.
//
// Collects an X handle and a link to the post the applicant engaged with, and
// nothing else. No email, no wallet address, no Zyn account — an invitation
// binds an account on first use (§74.13), so asking for one here would collect
// exactly what that design avoids, before anyone has decided to buy.
//
// # Spam
//
// Every application costs a reviewer a moment of attention, so the defences
// are aimed at volume rather than at identity. In rising order of what they
// cost an attacker:
//
//   a honeypot field   free to add, catches form-fillers that read the DOM
//   a dwell check      a person takes seconds; a script takes none
//   a rate limit       bounded submissions per address per hour
//   two dedupes        one handle applies once; one post backs one handle
//   proof of work      the only one whose cost scales with volume
//
// None of them identify anybody and none of them decide anything. The last
// word is a person reading the application, which is why these can be modest:
// they exist to keep the queue readable, not to be a security boundary.

const HANDLE = /^@?[A-Za-z0-9_]{1,15}$/;      // X's own rule
const MAX_NOTE = 280;

// Leading zero bits required of the proof. ~18 bits is around a quarter of a million
// hashes: unnoticeable once on a phone, expensive across thousands of forged
// applications, which is the asymmetry worth buying.
const POW_BITS = 18;
const WINDOW_SECONDS = 600;

/// Submissions allowed from one address per hour. Generous for a household
/// behind one address, useless as a flood.
const PER_HOUR = 6;

export async function onRequestPost({ request, env }) {
  // Without the store there is nowhere to put an application, and answering
  // anything but an error would lose it. This is a deployment fault, not the
  // applicant's, so it says so.
  if (!env.CAVE_WHITELIST) {
    return json(
      { ok: false, error: 'the register is not reachable — this is our fault, try again shortly' },
      503,
    );
  }

  let form;
  try {
    const ct = request.headers.get('content-type') || '';
    form = ct.includes('application/json')
      ? await request.json()
      : Object.fromEntries(await request.formData());
  } catch {
    return json({ ok: false, error: 'could not read the form' }, 400);
  }
  if (!form || typeof form !== 'object' || Array.isArray(form)) {
    return json({ ok: false, error: 'could not read the form' }, 400);
  }

  // A field positioned off-screen and left empty by anyone who can see. Checked
  // first because it costs nothing and a filled one means nothing else matters.
  if (String(form.burrow || '').trim() !== '') {
    return json({ ok: true });            // answer as though accepted
  }

  const handle = String(form.handle || '').trim().replace(/^@/, '');
  const post = String(form.post || '').trim().slice(0, 300);
  const note = String(form.note || '').trim().slice(0, MAX_NOTE);

  if (!HANDLE.test(handle)) {
    return json({ ok: false, error: 'that does not look like an X handle' }, 400);
  }
  // The link is the evidence a reviewer opens. Without it the engagement
  // requirement cannot be checked at all, so it is required, not optional.
  const postMatch = post.match(
    /^https?:\/\/(?:www\.|mobile\.)?(?:x|twitter)\.com\/([A-Za-z0-9_]{1,15})\/status\/(\d{5,25})(?:[/?#][^\s]*)?$/i,
  );
  if (!postMatch) {
    return json(
      { ok: false, error: 'paste the link to your repost or comment — an x.com status link' },
      400,
    );
  }

  // A person reads the steps, follows, reposts, then types. A script does not.
  const dwell = Number(form.dwell || 0);
  if (!Number.isFinite(dwell) || dwell < 3000) {
    return json({ ok: false, error: 'that was too quick — have another look at the steps' }, 400);
  }

  const proof = await checkProof(handle, form.nonce);
  if (!proof) {
    return json({ ok: false, error: 'the dig did not check out — press Apply again' }, 400);
  }

  const ip = request.headers.get('cf-connecting-ip') || 'unknown';
  if (!(await underRateLimit(env, ip))) {
    return json({ ok: false, error: 'too many applications from here — try again later' }, 429);
  }

  const key = `wl:${handle.toLowerCase()}`;
  if (await env.CAVE_WHITELIST.get(key)) {
    // Applying twice is not an error and does not make a second entry, nor
    // overwrite anything a reviewer may have annotated.
    return json({ ok: true, already: true });
  }

  // One post backs one handle. Without this a single real post is a template
  // for as many applications as someone cares to type.
  const postKey = `post:${postMatch[2]}`;
  const claimedBy = await env.CAVE_WHITELIST.get(postKey);
  if (claimedBy && claimedBy !== handle.toLowerCase()) {
    return json({ ok: false, error: 'that post has already been used by another applicant' }, 409);
  }

  const record = {
    handle,
    post,
    note,
    at: new Date().toISOString(),
    // Coarse, for spotting a flood from one place. Never shown, never acted on.
    country: request.headers.get('cf-ipcountry') || null,
  };
  // A resolved `put` is committed — KV's eventual consistency governs when a
  // *read* elsewhere sees it, not whether it was stored. So a write that
  // throws is the only way to lose an application, and it must reach the
  // applicant as an error rather than as a cheerful ok.
  //
  // The application is written before the post claim, deliberately. If the
  // second write fails, a post is left unclaimed and could be reused — a
  // reviewer's problem. The other order would claim a post for an application
  // that was never stored, which locks out the person it belonged to.
  try {
    await env.CAVE_WHITELIST.put(key, JSON.stringify(record));
  } catch (e) {
    return json(
      { ok: false, error: 'the register did not accept that — nothing was saved, please try again' },
      503,
    );
  }
  try {
    await env.CAVE_WHITELIST.put(postKey, handle.toLowerCase());
  } catch {
    // The application is safe; only the post claim is missing. Not worth
    // failing a stored application over.
  }

  // Tell the operator, after the application is safely stored and never in a
  // way that can lose one: a Telegram outage must not turn a good application
  // into an error the applicant sees.
  await notify(env, record).catch(() => {});

  return json({ ok: true });
}

// A GET is how a person arrives here by accident.
export async function onRequestGet() {
  return json({ ok: false, error: 'POST an application here' }, 405);
}

/// Push one application to Telegram.
///
/// Best-effort by construction. The register is the record; this is a tap on
/// the shoulder, and a tap that does not arrive is not worth failing a
/// submission over.
async function notify(env, record) {
  const token = env.TG_TOKEN;
  const chat = env.TG_CHAT;
  if (!token || !chat) return;                 // not configured, nothing to do
  const text =
    `new whitelist application\n\n` +
    `@${record.handle}\n` +
    `${record.post}\n` +
    (record.note ? `\n"${record.note.slice(0, 200)}"\n` : '') +
    `\n${record.at}${record.country ? ` · ${record.country}` : ''}`;
  await fetch(`https://api.telegram.org/bot${token}/sendMessage`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      chat_id: chat,
      text,
      disable_web_page_preview: true,
    }),
  });
}

/// Verify the proof of work.
///
/// The challenge is the applicant's own handle plus a coarse time window, so
/// no secret and no round trip are needed: work done for one handle is worth
/// nothing for another, and work done an hour ago is worth nothing now. Both
/// the current window and the previous one are accepted, so a form filled in
/// slowly across a boundary is not punished for it.
async function checkProof(handle, nonce) {
  if (typeof nonce !== 'string' || nonce.length > 64) return false;
  const now = Math.floor(Date.now() / 1000 / WINDOW_SECONDS);
  for (const window of [now, now - 1]) {
    if (await hasLeadingZeroBits(`cave:${handle.toLowerCase()}:${window}:${nonce}`, POW_BITS)) {
      return true;
    }
  }
  return false;
}

async function hasLeadingZeroBits(text, bits) {
  const digest = new Uint8Array(
    await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text)),
  );
  let left = bits;
  for (const byte of digest) {
    if (left >= 8) {
      if (byte !== 0) return false;
      left -= 8;
    } else {
      return (byte >> (8 - left)) === 0;
    }
    if (left === 0) return true;
  }
  return true;
}

/// A counter per address per hour. Approximate on purpose — KV is eventually
/// consistent, so this bounds a flood rather than metering exactly, which is
/// all a manually reviewed queue needs.
async function underRateLimit(env, ip) {
  const hour = Math.floor(Date.now() / 3_600_000);
  const key = `rl:${hour}:${ip}`;
  const seen = Number((await env.CAVE_WHITELIST.get(key)) || 0);
  if (seen >= PER_HOUR) return false;
  await env.CAVE_WHITELIST.put(key, String(seen + 1), { expirationTtl: 7200 });
  return true;
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
  });
}
