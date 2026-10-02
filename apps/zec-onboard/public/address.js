// What kind of Zcash address a string is, checked by its checksum.
//
// Transparent and Sprout addresses are Base58Check (double SHA-256); Sapling
// addresses are Bech32; unified and TEX addresses are Bech32m. A string that
// has the right prefix but the wrong checksum is reported as a typo, not as an
// address. Runs in the page and under Node (tests) — both have crypto.subtle.

const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
const B32 = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';

/// Base58Check versions (two bytes) of the transparent and Sprout encodings.
const BASE58 = {
  '1cb8': { kind: 'transparent', net: 'mainnet', name: 'Transparent address (P2PKH)' },
  '1cbd': { kind: 'transparent', net: 'mainnet', name: 'Transparent address (P2SH)' },
  '1d25': { kind: 'transparent', net: 'testnet', name: 'Transparent address (P2PKH)' },
  '1cba': { kind: 'transparent', net: 'testnet', name: 'Transparent address (P2SH)' },
  '169a': { kind: 'sprout', net: 'mainnet', name: 'Sprout address' },
  '16b6': { kind: 'sprout', net: 'testnet', name: 'Sprout address' },
};

/// Bech32 / Bech32m human-readable parts.
const BECH = {
  u: { kind: 'unified', net: 'mainnet', name: 'Unified address', m: true },
  utest: { kind: 'unified', net: 'testnet', name: 'Unified address', m: true },
  zs: { kind: 'sapling', net: 'mainnet', name: 'Sapling address', m: false },
  ztestsapling: { kind: 'sapling', net: 'testnet', name: 'Sapling address', m: false },
  tex: { kind: 'tex', net: 'mainnet', name: 'TEX address', m: true },
  textest: { kind: 'tex', net: 'testnet', name: 'TEX address', m: true },
  uview: { kind: 'viewing', net: 'mainnet', name: 'Unified viewing key', m: true },
  uviewtest: { kind: 'viewing', net: 'testnet', name: 'Unified viewing key', m: true },
};

async function sha256(bytes) {
  return new Uint8Array(await globalThis.crypto.subtle.digest('SHA-256', bytes));
}

function base58(s) {
  let n = 0n;
  for (const c of s) {
    const v = B58.indexOf(c);
    if (v < 0) return null;
    n = n * 58n + BigInt(v);
  }
  const out = [];
  while (n > 0n) { out.unshift(Number(n & 0xffn)); n >>= 8n; }
  for (const c of s) { if (c === '1') out.unshift(0); else break; }
  return new Uint8Array(out);
}

function polymod(values) {
  const G = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
  let chk = 1;
  for (const v of values) {
    const top = chk >>> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i++) if ((top >>> i) & 1) chk ^= G[i];
  }
  return chk >>> 0;
}

/// `{hrp, m}` when `s` is a valid Bech32 (m = false) or Bech32m (m = true)
/// string, else null. No 90-character limit: unified addresses are longer.
function bech32(s) {
  if (s !== s.toLowerCase() && s !== s.toUpperCase()) return null;
  s = s.toLowerCase();
  const at = s.lastIndexOf('1');
  if (at < 1 || at + 7 > s.length) return null;
  const hrp = s.slice(0, at);
  const data = [];
  for (const c of s.slice(at + 1)) {
    const v = B32.indexOf(c);
    if (v < 0) return null;
    data.push(v);
  }
  const expand = [...hrp].map((c) => c.charCodeAt(0) >> 5).concat(0, [...hrp].map((c) => c.charCodeAt(0) & 31));
  const check = polymod(expand.concat(data));
  if (check === 1) return { hrp, m: false };
  if (check === 0x2bc830a3) return { hrp, m: true };
  return null;
}

/// What `input` is. Always returns an object; `kind` is null when it is not
/// a Zcash address, with `reason` saying why in a sentence.
export async function identify(input) {
  const s = String(input || '').trim();
  if (!s) return { kind: null, reason: '' };
  if (/\s/.test(s)) return { kind: null, reason: 'An address has no spaces.' };

  if (/^(t1|t3|tm|t2|zc|zt)/.test(s)) {
    const raw = base58(s);
    if (!raw || raw.length !== (s.startsWith('z') ? 70 : 26)) {
      return { kind: null, reason: 'This starts like a Zcash address but has the wrong length or characters.' };
    }
    const body = raw.slice(0, -4);
    const sum = (await sha256(await sha256(body))).slice(0, 4);
    if (sum.some((b, i) => b !== raw[raw.length - 4 + i])) {
      return { kind: null, reason: 'The checksum does not match. Check for a typo, or copy the address again.' };
    }
    const version = [...raw.slice(0, 2)].map((b) => b.toString(16).padStart(2, '0')).join('');
    const hit = BASE58[version];
    if (!hit) return { kind: null, reason: 'Not a Zcash address.' };
    return { ...hit, address: s };
  }

  const b = bech32(s);
  if (b) {
    const hit = BECH[b.hrp];
    if (!hit) return { kind: null, reason: `Valid Bech32, but "${b.hrp}" is not a Zcash prefix.` };
    if (hit.m !== b.m) return { kind: null, reason: 'The checksum does not match. Check for a typo, or copy the address again.' };
    if (hit.kind === 'sapling' && s.length !== b.hrp.length + 76) return { kind: null, reason: 'A Sapling address of the wrong length.' };
    return { kind: hit.kind, net: hit.net, name: hit.name, address: s };
  }
  if (/^(u1|utest1|zs1|ztestsapling1|tex1|textest1)/i.test(s)) {
    return { kind: null, reason: 'The checksum does not match. Check for a typo, or copy the address again.' };
  }
  return { kind: null, reason: 'Not a Zcash address.' };
}

/// What sending to an address of this kind reveals, and whether Nap can pay it.
export const DESCRIBE = {
  transparent: {
    pool: 'Transparent',
    seen: 'Public. Anyone can see what this address receives, from where, and its balance.',
    nap: 'Nap can pay it. The payment leaves the shielded pool (unshielding), and a memo cannot be attached.',
  },
  unified: {
    pool: 'Shielded (usually)',
    seen: 'Payments are private when the address includes a shielded receiver, which every wallet in this guide adds.',
    nap: 'Nap can pay it, with a memo.',
  },
  sapling: {
    pool: 'Shielded (Sapling)',
    seen: 'Private. An older shielded address type.',
    nap: 'Nap cannot pay a Sapling address. Ask for a unified (u1…) address instead.',
  },
  tex: {
    pool: 'Transparent-source only',
    seen: 'Public. It only accepts ZEC that comes straight from a transparent address.',
    nap: 'Nap cannot pay a TEX address yet. Some wallets can; or ask for a regular t-address.',
  },
  sprout: {
    pool: 'Sprout (retired)',
    seen: 'The original shielded pool. It no longer accepts payments.',
    nap: 'No wallet can pay it.',
  },
  viewing: {
    pool: 'Not an address',
    seen: 'A viewing key shows everything its wallet receives and sends. Share it only with someone who should see all of it.',
    nap: 'Do not send funds to it.',
  },
};
