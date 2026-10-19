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

// What may appear in a buy made on this page, at the top level: the compute
// budget, a wallet's own guard (Lighthouse) and memos, creating token
// accounts, the SOL into the payer's own wrapped-SOL account, wrapping and
// closing it, and Jupiter's route. Nothing else moves SOL or tokens outside
// the route, so what the payer got came from the market.
const BUDGET = 'ComputeBudget111111111111111111111111111111';
const ATA = 'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL';
const SYSTEM = '11111111111111111111111111111111';
const TOKEN = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
const TOKEN22 = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb';
const JUPITER = 'JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4';
const HARMLESS = new Set([BUDGET, 'L2TExMFKdjpN9kozasaurPirfHy9P8sbXoAN1qA3S95', 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr', 'Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo']);
const WSOL = 'So11111111111111111111111111111111111111112';

/// The SOL a transaction put into Jupiter's route for `payer`, if it is shaped
/// as a buy from this page; null otherwise.
function routeIn(ixs, payer) {
  // Wrapped-SOL accounts the transaction itself made for the payer: the
  // token-account program checks the address, so these are the payer's own.
  const wrapped = new Set(ixs.filter((i) => i.programId === ATA && ['create', 'createIdempotent'].includes(i.parsed?.type) && i.parsed.info?.mint === WSOL && i.parsed.info?.wallet === payer).map((i) => i.parsed.info.account));
  let lamports = 0;
  let route = false;
  for (const i of ixs) {
    const t = i.parsed?.type;
    const info = i.parsed?.info || {};
    if (HARMLESS.has(i.programId)) continue;
    if (i.programId === JUPITER) { route = true; continue; }
    if (i.programId === ATA && (t === 'create' || t === 'createIdempotent')) continue;
    if (i.programId === SYSTEM && t === 'transfer' && info.source === payer && wrapped.has(info.destination)) { lamports += Number(info.lamports) || 0; continue; }
    if ((i.programId === TOKEN || i.programId === TOKEN22) && t === 'syncNative' && wrapped.has(info.account)) continue;
    if ((i.programId === TOKEN || i.programId === TOKEN22) && t === 'closeAccount' && wrapped.has(info.account) && info.destination === payer) continue;
    return null;
  }
  return route ? lamports : null;
}

/// Checks one transaction (getTransaction, jsonParsed): it succeeded, is
/// recent, `wallet` paid for it, it is a buy shaped as this page makes one,
/// at least `minLamports` went into the route, and the mint came in.
export function judge(tx, { wallet, mint, minLamports, now = Date.now(), maxAge = 15 * 60_000 }) {
  if (!tx?.meta || !tx.transaction) return { ok: false, why: 'missing' };
  if (tx.meta.err) return { ok: false, why: 'failed' };
  if (!tx.blockTime || now - tx.blockTime * 1000 > maxAge) return { ok: false, why: 'old' };
  const payer = tx.transaction.message?.accountKeys?.[0];
  if ((payer?.pubkey ?? payer) !== wallet || payer?.signer === false) return { ok: false, why: 'payer' };
  const lamportsIn = routeIn(tx.transaction.message.instructions || [], wallet);
  if (lamportsIn === null) return { ok: false, why: 'shape' };
  if (lamportsIn < minLamports) return { ok: false, why: 'small' };
  const tokensIn = tokens(tx.meta.postTokenBalances, mint, wallet) - tokens(tx.meta.preTokenBalances, mint, wallet);
  if (tokensIn <= 0n) return { ok: false, why: 'token' };
  return { ok: true, lamportsIn, tokensIn };
}

/// The whole claim. deps: getTx(signature), record(signature, pass, wallet) →
/// 'new' | 'same' | 'other', register(pass) → {ok} | {error}, quote(lamports)
/// → the mint's base units Jupiter gives for them now, mint, minLamports, now.
export async function claim({ pass, signature, wallet, signed }, deps) {
  if (!PASS.test(pass || '')) return { ok: false, why: 'pass', status: 400 };
  if (typeof signature !== 'string' || typeof wallet !== 'string' || typeof signed !== 'string') return { ok: false, why: 'signed', status: 400 };
  if (!(await verifySigned(wallet, message(pass, signature), signed))) return { ok: false, why: 'signed', status: 403 };
  const seen = judge(await deps.getTx(signature), { wallet, mint: deps.mint, minLamports: deps.minLamports, now: deps.now?.() });
  if (!seen.ok) return { ...seen, status: seen.why === 'missing' ? 404 : 403 };
  // What the market would give for that SOL now: a buy that got under half of
  // it did not come from the market (fees and the price moving are within).
  let market;
  try { market = BigInt(await deps.quote(seen.lamportsIn)); } catch { return { ok: false, why: 'quote', status: 503 }; }
  if (seen.tokensIn * 2n < market) return { ok: false, why: 'price', status: 403 };
  if ((await deps.record(signature, pass, wallet)) === 'other') return { ok: false, why: 'used', status: 409 };
  const den = await deps.register(pass);
  return den?.ok ? { ok: true } : { ok: false, why: 'den', status: 502 };
}

/// Moves a wallet's bought access to a new device, once a day. The move is
/// taken in the claims table first, by a conditional update only one request
/// can win, so two at once never let in two devices; then the new pass goes
/// in, and the old one out last, so a den that fails halfway locks nobody out
/// (the record is put back). deps: latest(wallet) → {signature, pass,
/// moved_at} | null, take(signature, fromPass, toPass, at, before) → true if
/// it took the row, restore(signature, toPass, fromPass, movedAt),
/// register(pass), unregister(pass), now.
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
  const soon = () => ({ ok: false, why: 'soon', next: (row.moved_at ?? now) + MOVE_EVERY, status: 429 });
  if (row.moved_at && now - row.moved_at < MOVE_EVERY) return soon();
  const old = row.pass;
  if (!(await deps.take(row.signature, old, pass, now, now - MOVE_EVERY))) return { ...soon(), next: now + MOVE_EVERY };
  if (!(await deps.register(pass))?.ok) {
    await deps.restore(row.signature, pass, old, row.moved_at ?? null);
    return { ok: false, why: 'den', status: 502 };
  }
  for (let i = 0; i < 2; i++) if ((await deps.unregister(old))?.ok) return { ok: true };
  return { ok: true, left: old };
}
