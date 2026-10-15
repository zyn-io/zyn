// Base58 (Bitcoin alphabet), as Solana writes addresses and signatures.
const ABC = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
const AT = Object.fromEntries([...ABC].map((c, i) => [c, BigInt(i)]));

export function decode(s) {
  if (typeof s !== 'string' || !s) throw new Error('not base58');
  let n = 0n;
  for (const c of s) {
    if (!(c in AT)) throw new Error('not base58');
    n = n * 58n + AT[c];
  }
  const out = [];
  while (n > 0n) { out.unshift(Number(n & 255n)); n >>= 8n; }
  for (const c of s) { if (c !== '1') break; out.unshift(0); }
  return Uint8Array.from(out);
}

export function encode(bytes) {
  let n = 0n;
  for (const b of bytes) n = (n << 8n) | BigInt(b);
  let s = '';
  while (n > 0n) { s = ABC[Number(n % 58n)] + s; n /= 58n; }
  for (const b of bytes) { if (b !== 0) break; s = '1' + s; }
  return s;
}

/// What the buyer's wallet signs: it ties this buy to this device's pass, so a
/// buy seen on chain cannot be claimed by anyone but the wallet that made it.
export const message = (pass, signature) => `Epeaky access\npass: ${pass}\ntx: ${signature}`;

/// What a wallet that bought before signs to move its access to this device.
/// `at` (ms) keeps an old signature from being sent again later.
export const moveMessage = (pass, wallet, at) => `Epeaky access\nmove to: ${pass}\nwallet: ${wallet}\nat: ${at}`;
