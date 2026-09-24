// A burn of the sale's promotional token on Solana, with the memo that makes
// it the buyer's tickets on Zyn (DECISIONS §145).
//
// One legacy transaction, two instructions: SPL `burnChecked` from the
// holder's token account, and a memo `zyn:<commitment>` from their Nap. The
// bytes are laid out here rather than by a Solana SDK — a few hundred bytes
// a test pins, and nothing else on the page to trust — and handed to the
// holder's wallet through the Wallet Standard, which signs and sends them.

const ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
export const MEMO_PROGRAM = 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr';
export const TOKEN_PROGRAMS = ['TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA', 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb'];
const BURN_CHECKED = 15;

export function b58decode(s) {
  let n = 0n;
  for (const c of s) {
    const i = ALPHABET.indexOf(c);
    if (i < 0) throw new Error('not base58');
    n = n * 58n + BigInt(i);
  }
  const bytes = [];
  while (n > 0n) { bytes.unshift(Number(n % 256n)); n /= 256n; }
  for (const c of s) { if (c !== '1') break; bytes.unshift(0); }
  return Uint8Array.from(bytes);
}

export function b58encode(bytes) {
  let n = 0n;
  for (const b of bytes) n = n * 256n + BigInt(b);
  let out = '';
  while (n > 0n) { out = ALPHABET[Number(n % 58n)] + out; n /= 58n; }
  for (const b of bytes) { if (b !== 0) break; out = `1${out}`; }
  return out;
}

const key = (s) => { const k = b58decode(s); if (k.length !== 32) throw new Error('not a Solana address'); return k; };
const shortVec = (n) => { const out = []; do { let b = n & 0x7f; n >>= 7; if (n) b |= 0x80; out.push(b); } while (n); return out; };
const u64le = (v) => { const b = new Uint8Array(8); let x = BigInt(v); for (let i = 0; i < 8; i++) { b[i] = Number(x & 0xffn); x >>= 8n; } return b; };

/// The message: the owner signs and pays; the token account and the mint are
/// written (the burn lowers both); the token program and the memo program
/// are read. Legacy format, keys in the order the header describes.
export function burnMessage({ owner, tokenAccount, mint, tokenProgram, amount, decimals, memo, blockhash }) {
  if (!TOKEN_PROGRAMS.includes(tokenProgram)) throw new Error('not a token program');
  if (!/^zyn:[0-9a-f]{64}$/.test(memo)) throw new Error('the memo must be the one Nap gave');
  const keys = [owner, tokenAccount, mint, tokenProgram, MEMO_PROGRAM].map(key);
  const burnData = [BURN_CHECKED, ...u64le(amount), decimals];
  const memoData = [...new TextEncoder().encode(memo)];
  const bytes = [
    1, 0, 2, // one signer, none of them read-only; two read-only programs
    ...shortVec(keys.length), ...keys.flatMap((k) => [...k]),
    ...key(blockhash),
    ...shortVec(2),
    3, ...shortVec(3), 1, 2, 0, ...shortVec(burnData.length), ...burnData,
    4, ...shortVec(0), ...shortVec(memoData.length), ...memoData,
  ];
  return Uint8Array.from(bytes);
}

/// A transaction awaiting its one signature: the wallet fills it in.
export function unsigned(message) {
  const out = new Uint8Array(1 + 64 + message.length);
  out[0] = 1;
  out.set(message, 65);
  return out;
}

/// Base units for a whole number of tickets.
export const burnAmount = (tickets, perTicket) => BigInt(tickets) * BigInt(perTicket);

// ---------------------------------------------------------------------------
// Wallets, by the Wallet Standard (DECISIONS 7e.7): every Solana wallet that
// registers is offered, none by name.

export function solanaWallets() {
  const found = [];
  const register = (...ws) => {
    for (const w of ws) {
      if (w?.features?.['solana:signAndSendTransaction'] && w?.features?.['standard:connect'] && !found.includes(w)) found.push(w);
    }
  };
  try {
    window.addEventListener('wallet-standard:register-wallet', (e) => { try { e.detail?.({ register }); } catch { /* one wallet */ } });
    window.dispatchEvent(new CustomEvent('wallet-standard:app-ready', { detail: { register } }));
  } catch { /* no window: tests */ }
  return found;
}

/// Ask the wallet to sign and send; the signature comes back base58.
export async function signAndSend(wallet, account, tx, chain) {
  const [r] = await wallet.features['solana:signAndSendTransaction'].signAndSendTransaction({ account, transaction: tx, chain });
  return b58encode(r.signature);
}
