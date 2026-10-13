// X sign-in for the whitelist (DECISIONS §187.8), step one: send the applicant
// to X with a fresh state and PKCE challenge. The wallet to link comes from the
// page (the Zyn account Nap is connected to) and waits here, keyed by the
// state, for ten minutes. Nothing is asked of X but the account's id and handle
// (scope users.read, which X requires tweet.read beside).

const ACCOUNT = /^[0-9a-f]{64}$/;
const AUTHORIZE = 'https://x.com/i/oauth2/authorize';

export async function onRequestGet({ request, env }) {
  if (!env.X_CLIENT_ID || !env.CAVE_WHITELIST) return done(request, 'unavailable');
  const url = new URL(request.url);
  const account = String(url.searchParams.get('account') || '').toLowerCase();
  if (!ACCOUNT.test(account)) return done(request, 'no-wallet');
  const state = b64url(crypto.getRandomValues(new Uint8Array(24)));
  const verifier = b64url(crypto.getRandomValues(new Uint8Array(48)));
  const challenge = b64url(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier))));
  await env.CAVE_WHITELIST.put(`xauth:${state}`, JSON.stringify({ verifier, account }), { expirationTtl: 600 });
  const to = new URL(AUTHORIZE);
  to.search = new URLSearchParams({
    response_type: 'code',
    client_id: env.X_CLIENT_ID,
    redirect_uri: `${url.origin}/api/whitelist/x/callback`,
    scope: 'users.read tweet.read',
    state,
    code_challenge: challenge,
    code_challenge_method: 'S256',
  }).toString();
  return Response.redirect(to.toString(), 302);
}

/// The popup's last page, which tells /mint how it went and closes.
export function done(request, result) {
  return Response.redirect(`${new URL(request.url).origin}/x-done/?r=${encodeURIComponent(result)}`, 302);
}

export function b64url(bytes) {
  let s = '';
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
