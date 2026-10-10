// REGI holders onto the whitelist (DECISIONS §181): free, nothing burned,
// the REGI stays where it is.
//
// The applicant signs a short message with their Arc (any EVM) wallet. This
// checks the signature, then looks the wallet up in the holder snapshot that
// scripts/regi-snapshot.mjs published — Arc's RPC refuses Cloudflare's
// workers, so nothing here asks Arc. A holder becomes a member at once, with
// an invitation code like an approved handle's, and links it on /mint as
// everyone does; `invite.mjs linked` and `zyn-sale-draw allow` follow.
//
// What is kept never names the wallet: the snapshot and the member row hold
// only HMAC-SHA256(REGI_SALT, address). A plain hash would hide nobody — the
// holder list is public. The code is derived from the wallet the same way, so
// signing again shows the same code (a lost code is not lost) and one wallet
// is one place.
import { secp256k1 } from '@noble/curves/secp256k1';
import { keccak_256 } from '@noble/hashes/sha3';

const ALPHABET = 'ABCDEFGHJKMNPQRSTUVWXYZ23456789'; // as invite.mjs: no 0/O, 1/I/L
const PER_HOUR = 10;
/// How old a signed message may be.
const FRESH_MS = 15 * 60 * 1000;
export const TITLE = 'Cave whitelist: REGI holder sign-up';

/// The message a wallet signs, byte for byte (the page builds the same).
export const messageFor = (wallet, date) => `${TITLE}\nWallet: ${wallet}\nDate: ${date}\nThis signature only proves you hold this wallet. It moves nothing.`;

export async function onRequestPost({ request, env }) {
  if (!env.CAVE_MEMBERS || !env.CAVE_WHITELIST || !env.REGI_SALT) {
    return json({ ok: false, error: 'the whitelist is not reachable — try again shortly' }, 503);
  }
  let body;
  try { body = await request.json(); } catch { return json({ ok: false, error: 'could not read the request' }, 400); }
  const wallet = String(body?.wallet || '').toLowerCase();
  const date = String(body?.date || '');
  const signature = String(body?.signature || '');
  if (!/^0x[0-9a-f]{40}$/.test(wallet)) return json({ ok: false, error: 'connect an Arc wallet first' }, 400);
  const at = Date.parse(date);
  if (!Number.isFinite(at) || Math.abs(Date.now() - at) > FRESH_MS) return json({ ok: false, error: 'that signature is too old — sign again' }, 400);
  if (!/^0x[0-9a-f]{130}$/i.test(signature)) return json({ ok: false, error: 'that is not a wallet signature' }, 400);

  const ip = request.headers.get('cf-connecting-ip') || 'unknown';
  const rl = `rl-regi:${Math.floor(Date.now() / 3_600_000)}:${ip}`;
  const seen = Number((await env.CAVE_WHITELIST.get(rl)) || 0);
  if (seen >= PER_HOUR) return json({ ok: false, error: 'too many tries from here — try again later' }, 429);
  await env.CAVE_WHITELIST.put(rl, String(seen + 1), { expirationTtl: 7200 });

  let signer;
  try { signer = recover(messageFor(wallet, date), signature); } catch { return json({ ok: false, error: 'that signature does not check out' }, 400); }
  if (signer !== wallet) return json({ ok: false, error: 'that signature is from another wallet' }, 400);

  const mac = await hmac(env.REGI_SALT, wallet);
  if (!(await env.CAVE_WHITELIST.get(`regi-ok:${mac}`))) {
    const snap = await env.CAVE_WHITELIST.get('regi-snapshot', 'json').catch(() => null);
    return json({ ok: false, error: snap ? `this wallet is not in the REGI snapshot (block ${snap.block}, at least ${Number(snap.min).toLocaleString('en-US')} REGI)` : 'the REGI snapshot is not published yet' }, 404);
  }

  const code = codeFrom(await hmac(env.REGI_SALT, `code:${wallet}`));
  const codeHash = await sha256(`cave-invite:${code.replace(/-/g, '')}`);
  const handle = `regi:${mac.slice(0, 24)}`;
  try {
    const r = await env.CAVE_MEMBERS.prepare('INSERT INTO members (handle, code_hash, approved_at, note) VALUES (?, ?, ?, ?) ON CONFLICT (handle) DO NOTHING')
      .bind(handle, codeHash, new Date().toISOString(), 'REGI holder').run();
    if (!r.meta?.changes) {
      // Already a member: the same wallet gets the same code back.
      const m = await env.CAVE_MEMBERS.prepare('SELECT code_hash FROM members WHERE handle = ?').bind(handle).first();
      if (m?.code_hash !== codeHash) return json({ ok: false, error: 'this wallet already has a place' }, 409);
      return json({ ok: true, code, already: true });
    }
  } catch {
    return json({ ok: false, error: 'the whitelist did not accept that — try again' }, 503);
  }
  return json({ ok: true, code });
}

export async function onRequestGet() {
  return json({ ok: false, error: 'POST a signed message here' }, 405);
}

/// The address that signed `message` (EIP-191 personal_sign).
export function recover(message, signature) {
  const bytes = new TextEncoder().encode(message);
  const digest = keccak_256(new Uint8Array([...new TextEncoder().encode(`\x19Ethereum Signed Message:\n${bytes.length}`), ...bytes]));
  const raw = hexBytes(signature.slice(2));
  let v = raw[64];
  if (v >= 27) v -= 27;
  if (v !== 0 && v !== 1) throw new Error('bad recovery id');
  const sig = secp256k1.Signature.fromCompact(raw.slice(0, 64)).addRecoveryBit(v);
  const pub = sig.recoverPublicKey(digest).toRawBytes(false);
  return `0x${toHex(keccak_256(pub.slice(1)).slice(-20))}`;
}

/// 12 characters from 31, grouped as invite.mjs prints them.
export function codeFrom(macHex) {
  return [...hexBytes(macHex).slice(0, 12)].map((b) => ALPHABET[b % ALPHABET.length]).join('').replace(/(.{4})(?=.)/g, '$1-');
}

async function hmac(secret, text) {
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  return toHex(new Uint8Array(await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(text))));
}

async function sha256(text) {
  return toHex(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))));
}

const toHex = (b) => [...b].map((x) => x.toString(16).padStart(2, '0')).join('');
const hexBytes = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));

function json(body, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' } });
}
