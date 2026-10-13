// X sign-in for the whitelist (DECISIONS §187.8), step two: X sends the
// applicant back with a code; the code and the PKCE verifier become a token,
// the token names the X account, and the token is revoked at once. An
// approved member, or anyone with an application on file (the user, 13 Oct:
// "whitelist applicants we accept all of them"), has the waiting wallet linked
// to them — the same row a code links. Only the X account's id and handle are
// read; the token is never kept.

import { COOKIE, done } from './start.js';

const TOKEN = 'https://api.x.com/2/oauth2/token';
const REVOKE = 'https://api.x.com/2/oauth2/revoke';
const ME = 'https://api.x.com/2/users/me';

export async function onRequestGet({ request, env }) {
  if (!env.X_CLIENT_ID || !env.X_CLIENT_SECRET || !env.CAVE_WHITELIST || !env.CAVE_MEMBERS) return done(request, 'unavailable');
  const url = new URL(request.url);
  if (url.searchParams.get('error')) return done(request, 'cancelled');
  const state = String(url.searchParams.get('state') || '');
  const code = String(url.searchParams.get('code') || '');
  if (!/^[A-Za-z0-9_-]{20,64}$/.test(state) || !code) return done(request, 'failed');
  // Only the browser that started this sign-in may finish it.
  const mine = (request.headers.get('cookie') || '').split(/;\s*/).find((c) => c.startsWith(`${COOKIE}=`))?.slice(COOKIE.length + 1) || '';
  if (!same(mine, state)) return done(request, 'expired');
  const waiting = await env.CAVE_WHITELIST.get(`xauth:${state}`);
  if (!waiting) return done(request, 'expired');
  await env.CAVE_WHITELIST.delete(`xauth:${state}`);
  const { verifier, account } = JSON.parse(waiting);

  let me;
  try {
    me = await whoIsThis(env, code, verifier, `${url.origin}/api/whitelist/x/callback`);
  } catch {
    return done(request, 'failed');
  }
  return done(request, await link(env, me, account));
}

/// Equal strings, compared without stopping at the first difference.
function same(a, b) {
  if (a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return d === 0;
}

/// The X account behind a sign-in: its id and handle. The token is revoked
/// as soon as it has answered.
async function whoIsThis(env, code, verifier, redirect) {
  const basic = `Basic ${btoa(`${env.X_CLIENT_ID}:${env.X_CLIENT_SECRET}`)}`;
  const form = (o) => ({ method: 'POST', headers: { authorization: basic, 'content-type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams(o).toString() });
  const t = await fetch(TOKEN, form({ grant_type: 'authorization_code', code, redirect_uri: redirect, code_verifier: verifier, client_id: env.X_CLIENT_ID }));
  const tok = await t.json();
  if (!t.ok || !tok.access_token) throw new Error('no token');
  try {
    const r = await fetch(ME, { headers: { authorization: `Bearer ${tok.access_token}` } });
    const j = await r.json();
    if (!r.ok || !/^\d+$/.test(j?.data?.id || '') || !/^[A-Za-z0-9_]{1,15}$/.test(j?.data?.username || '')) throw new Error('no account');
    return { id: j.data.id, handle: j.data.username };
  } finally {
    await fetch(REVOKE, form({ token: tok.access_token, token_type_hint: 'access_token', client_id: env.X_CLIENT_ID })).catch(() => {});
  }
}

/// Link the wallet to the member this X account is. Returns the result the
/// page shows: linked, already, not-approved, other-wallet, wallet-used.
export async function link(env, me, account) {
  const db = env.CAVE_MEMBERS;
  const now = new Date().toISOString();
  // The member pinned to this X account, else the member with this handle
  // that no X account has pinned yet.
  let member = await db.prepare('SELECT handle, account, x_id FROM members WHERE x_id = ?').bind(me.id).first()
    || await db.prepare('SELECT handle, account, x_id FROM members WHERE handle = ? AND x_id IS NULL').bind(me.handle).first();
  if (!member) {
    // Every applicant is accepted: an application on file makes a member now.
    const applied = await env.CAVE_WHITELIST.get(`wl:${me.handle.toLowerCase()}`);
    if (!applied) return 'not-approved';
    const placeholder = [...crypto.getRandomValues(new Uint8Array(32))].map((b) => b.toString(16).padStart(2, '0')).join('');
    try {
      await db.prepare('INSERT INTO members (handle, code_hash, approved_at, note) VALUES (?, ?, ?, ?)').bind(me.handle, `x:${placeholder}`, now, 'approved by X sign-in (§187.8)').run();
    } catch { /* approved a moment ago by another tab: read it below */ }
    member = await db.prepare('SELECT handle, account, x_id FROM members WHERE handle = ? AND (x_id IS NULL OR x_id = ?)').bind(me.handle, me.id).first();
    if (!member) return 'not-approved';
  }
  if (member.account === account) {
    if (!member.x_id) await db.prepare('UPDATE members SET x_id = ? WHERE handle = ? AND x_id IS NULL').bind(me.id, member.handle).run().catch(() => {});
    return 'already';
  }
  if (member.account) return 'other-wallet';
  try {
    const r = await db.prepare('UPDATE members SET account = ?, linked_at = ?, x_id = ? WHERE handle = ? AND account IS NULL AND (x_id IS NULL OR x_id = ?)')
      .bind(account, now, me.id, member.handle, me.id).run();
    if (!r.meta?.changes) return 'other-wallet';
  } catch (e) {
    return /UNIQUE/i.test(String(e?.message)) ? 'wallet-used' : 'failed';
  }
  return 'linked';
}
