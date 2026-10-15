// Whether a finished buy lets a device in. Pure: the chain, the claims table
// and the den come in as `deps`, so tests run on recorded transactions.
import { decode, message, moveMessage } from '../public/shared.mjs';

export { message, moveMessage };

const PASS = /^[0-9a-f]{64}$/;

export async function verifySigned(wallet, text, signed) {
  try {
    const key = await crypto.subtle.importKey('raw', decode(wallet), { name: 'Ed25519' }, false, ['verify']);
    return await crypto.subtle.verify('Ed25519', key, decode(signed), new TextEncoder().encode(text));
  } catch { return false; }
}

const tokens = (list, mint, owner) =>
  (list || []).filter((b) => b.mint === mint && b.owner === owner).reduce((n, b) => n + BigInt(b.uiTokenAmount?.amount || '0'), 0n);

/// Checks one transaction (getTransaction, jsonParsed): it succeeded, is
/// recent, `wallet` paid for it, spent at least `minLamports` and got the mint.
export function judge(tx, { wallet, mint, minLamports, now = Date.now(), maxAge = 15 * 60_000 }) {
  if (!tx?.meta || !tx.transaction) return { ok: false, why: 'missing' };
  if (tx.meta.err) return { ok: false, why: 'failed' };
  if (!tx.blockTime || now - tx.blockTime * 1000 > maxAge) return { ok: false, why: 'old' };
  const payer = tx.transaction.message?.accountKeys?.[0];
  if ((payer?.pubkey ?? payer) !== wallet || payer?.signer === false) return { ok: false, why: 'payer' };
  if (BigInt(tx.meta.preBalances[0]) - BigInt(tx.meta.postBalances[0]) < BigInt(minLamports)) return { ok: false, why: 'small' };
  if (tokens(tx.meta.postTokenBalances, mint, wallet) <= tokens(tx.meta.preTokenBalances, mint, wallet)) return { ok: false, why: 'token' };
  return { ok: true };
}

/// The whole claim. deps: getTx(signature), record(signature, pass, wallet) →
/// 'new' | 'same' | 'other', register(pass) → {ok} | {error}, mint, minLamports, now.
export async function claim({ pass, signature, wallet, signed }, deps) {
  if (!PASS.test(pass || '')) return { ok: false, why: 'pass', status: 400 };
  if (typeof signature !== 'string' || typeof wallet !== 'string' || typeof signed !== 'string') return { ok: false, why: 'signed', status: 400 };
  if (!(await verifySigned(wallet, message(pass, signature), signed))) return { ok: false, why: 'signed', status: 403 };
  const seen = judge(await deps.getTx(signature), { wallet, mint: deps.mint, minLamports: deps.minLamports, now: deps.now?.() });
  if (!seen.ok) return { ...seen, status: seen.why === 'missing' ? 404 : 403 };
  if ((await deps.record(signature, pass, wallet)) === 'other') return { ok: false, why: 'used', status: 409 };
  const den = await deps.register(pass);
  return den?.ok ? { ok: true } : { ok: false, why: 'den', status: 502 };
}

/// Moves a wallet's bought access to a new device, once a day: the new pass
/// goes in first and the old one out last, so a den that fails halfway locks
/// nobody out. deps: latest(wallet) → {signature, pass, moved_at} | null,
/// register(pass), swap(signature, pass, at), unregister(pass), now.
export const MOVE_EVERY = 24 * 3600_000;
export async function move({ pass, wallet, at, signed }, deps) {
  if (!PASS.test(pass || '')) return { ok: false, why: 'pass', status: 400 };
  if (typeof wallet !== 'string' || typeof signed !== 'string' || !Number.isSafeInteger(at)) return { ok: false, why: 'signed', status: 400 };
  const now = deps.now?.() ?? Date.now();
  if (Math.abs(now - at) > 5 * 60_000) return { ok: false, why: 'stale', status: 403 };
  if (!(await verifySigned(wallet, moveMessage(pass, wallet, at), signed))) return { ok: false, why: 'signed', status: 403 };
  const row = await deps.latest(wallet);
  if (!row) return { ok: false, why: 'none', status: 404 };
  if (row.pass === pass) return { ok: true };
  if (row.moved_at && now - row.moved_at < MOVE_EVERY) return { ok: false, why: 'soon', next: row.moved_at + MOVE_EVERY, status: 429 };
  const old = row.pass;
  if (!(await deps.register(pass))?.ok) return { ok: false, why: 'den', status: 502 };
  await deps.swap(row.signature, pass, now);
  await deps.unregister(old);
  return { ok: true };
}
