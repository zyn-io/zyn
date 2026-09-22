// Reading an object ledger.
//
// The AMM chains answer `zyn_account` and hand a wallet its balance. An object
// ledger cannot: it holds commitments and nullifiers, and *nothing on it says
// who owns what*. That is the privacy property, and the cost is that the
// balance is something the holder computes rather than asks for.
//
// So: fetch every published note, try to open each one, keep the ones that
// open, and drop the ones whose nullifier has already appeared. What is left
// is what this account holds.
//
// Every constant here is a wire format shared with the Rust side. Change one
// and a restored wallet looks at the right chain and finds nothing — which is
// indistinguishable from having been robbed.

import { x25519 } from '@noble/curves/ed25519.js';
import { chacha20poly1305 } from '@noble/ciphers/chacha.js';
import { sha256 } from '@noble/hashes/sha2.js';
import { hkdf } from '@noble/hashes/hkdf.js';

const te = new TextEncoder();

/// `hash_leaf` on the Rust side: SHA-256 over a domain-separating 0x00 byte
/// and the encoded body. The prefix is what keeps a leaf from colliding with
/// an interior node of the same bytes.
const LEAF_PREFIX = 0x00;

function leaf(...parts) {
  let n = 1;
  for (const p of parts) n += p.length;
  const buf = new Uint8Array(n);
  buf[0] = LEAF_PREFIX;
  let at = 1;
  for (const p of parts) { buf.set(p, at); at += p.length; }
  return sha256(buf);
}

/// Derivations, pinned to `zynzapd/src/object_keys.rs`.
const VIEWING_SALT = 'nap.zyn.viewing.v1';
const VIEWING_INFO = 'viewing';
const NULLIFIER_SALT = 'nap.zyn.nullifier.v1';
const NULLIFIER_INFO = 'nullifier';

function derive(seed, salt, info, account) {
  const labelled = new Uint8Array(info.length + 4);
  labelled.set(te.encode(info), 0);
  new DataView(labelled.buffer).setUint32(info.length, account, false);
  return hkdf(sha256, seed, te.encode(salt), labelled, 32);
}

/// The key that opens notes sent to this account. Never spends one.
export const viewingSecret = (seed, account = 0) =>
  derive(seed, VIEWING_SALT, VIEWING_INFO, account);

/// What to publish so others can pay this account.
export const viewingKey = (seed, account = 0) =>
  x25519.getPublicKey(viewingSecret(seed, account));

/// The key that recognises this account's own spends. Also never spends.
export const nullifierKey = (seed, account = 0) =>
  derive(seed, NULLIFIER_SALT, NULLIFIER_INFO, account);

/// `zyn.note.kdf.v1` — the symmetric key for one note.
const noteKey = (shared, epk) => leaf(te.encode('zyn.note.kdf.v1'), shared, epk);

/// A single fixed nonce is safe because the key never repeats: it comes from a
/// fresh ECDH against a per-note ephemeral key, so no two notes share one.
const NOTE_NONCE = te.encode('zyn.note.v1\0');

const PLAINTEXT_LEN = 128;

/// Open one published note, or return null.
///
/// Null covers both "not ours" and "damaged" deliberately, exactly as the Rust
/// side does: telling a scanner *why* a trial decryption failed tells an
/// attacker which notes are close.
export function openNote(secret, epk, ciphertext) {
  let padded;
  try {
    const shared = x25519.getSharedSecret(secret, epk);
    padded = chacha20poly1305(noteKey(shared, epk), NOTE_NONCE).decrypt(ciphertext);
  } catch {
    return null;
  }
  if (padded.length !== PLAINTEXT_LEN) return null;
  const len = new DataView(padded.buffer, padded.byteOffset).getUint32(0, false);
  if (len > PLAINTEXT_LEN - 4) return null;
  const body = padded.subarray(4, 4 + len);
  // asset(32) ‖ amount(i128 big-endian) ‖ nonce(32)
  if (body.length !== 80) return null;
  return {
    asset: body.subarray(0, 32),
    amount: readI128(body.subarray(32, 48)),
    nonce: body.subarray(48, 80),
  };
}

/// A signed 128-bit big-endian integer, as `BigInt`.
///
/// `BigInt` and not `Number`: amounts are in WAD units, so a balance of a few
/// ZEC already exceeds what a double represents exactly, and a wallet that
/// rounds someone's balance is a wallet that lies about it.
function readI128(b) {
  let v = 0n;
  for (const byte of b) v = (v << 8n) | BigInt(byte);
  // Two's complement.
  return v >= 1n << 127n ? v - (1n << 128n) : v;
}

function i128Bytes(v) {
  let u = v < 0n ? (1n << 128n) + v : v;
  const out = new Uint8Array(16);
  for (let i = 15; i >= 0; i--) { out[i] = Number(u & 0xffn); u >>= 8n; }
  return out;
}

/// `commitment(note)` — what the chain published for this note.
///
/// Recomputed rather than trusted: a mirror could hand back a ciphertext
/// paired with somebody else's commitment, and a wallet that believed the
/// pairing would show a balance it cannot spend.
export function commitment(note, owner) {
  return leaf(
    te.encode('zyn.object.v1'),
    note.asset,
    i128Bytes(note.amount),
    owner,
    note.nonce,
  );
}

/// The nullifier that retires a commitment.
export const nullifierOf = (nkey, commit) =>
  leaf(te.encode('zyn.nullifier.v1'), nkey, commit);

const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');

/// What this account holds, from a `zyn_published` feed.
///
/// `published.outputs` is `{commitment, epk, ciphertext}` and
/// `published.nullifiers` is every nullifier the chain has seen. A note is
/// held when it opens, when the commitment it recomputes to is the one the
/// chain published, and when its nullifier has not appeared.
export function scan(published, { secret, nkey, owner }) {
  const spent = new Set(published.nullifiers || []);
  const held = [];
  const byAsset = new Map();

  for (const o of published.outputs || []) {
    const note = openNote(secret, unhex(o.epk), unhex(o.ciphertext));
    if (!note) continue;

    // The pairing is checked, not assumed.
    const commit = commitment(note, owner);
    if (hex(commit) !== o.commitment) continue;
    if (spent.has(hex(nullifierOf(nkey, commit)))) continue;

    const asset = hex(note.asset);
    held.push({ commitment: hex(commit), asset, amount: note.amount });
    byAsset.set(asset, (byAsset.get(asset) || 0n) + note.amount);
  }
  return { held, balances: byAsset };
}

export function unhex(s) {
  if (typeof s !== 'string' || s.length % 2) return new Uint8Array(0);
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export { hex };
