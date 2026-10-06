// Invitations for the gated sale (DECISIONS §74.13, §168).
//
// An approved handle gets a code; its holder links the code to their own Zyn
// account on cavebro.win/mint (functions/api/whitelist/bind.js), and the
// accounts linked so far go on the sale's list with `zyn-sale-draw allow`.
// Nobody's address is collected before they choose to buy.
//
//   node scripts/invite.mjs approve <handle> [handle …]   print one code each
//   node scripts/invite.mjs linked > accounts.txt         the accounts to allow
//   node scripts/invite.mjs status                         codes issued / linked
//
// KV keeps only a code's SHA-256 (`invite:<hash>`), so the store holds no
// usable code: a code exists in the operator's DM and the holder's hands.
// Run with the Cloudflare account wrangler is logged in to, like review.mjs.
import { execFileSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';

const NS = '8181407749be40ff89d7691132723ae7';
const kv = (args) => execFileSync('wrangler', ['kv', 'key', ...args, '--namespace-id', NS, '--remote'], { encoding: 'utf8', maxBuffer: 64 << 20 });
const ALPHABET = 'ABCDEFGHJKMNPQRSTUVWXYZ23456789'; // no 0/O, 1/I/L
/// 12 characters from 31: ~59 bits, typed once, never guessed.
const code = () => [...randomBytes(12)].map((b) => ALPHABET[b % ALPHABET.length]).join('').replace(/(.{4})(?=.)/g, '$1-');
export const codeHash = (c) => createHash('sha256').update(`cave-invite:${String(c).toUpperCase().replace(/[^A-Z0-9]/g, '')}`).digest('hex');

const [cmd, ...args] = process.argv.slice(2);
const invites = () => JSON.parse(kv(['list'])).map((k) => k.name).filter((n) => n.startsWith('invite:'));
const read = (key) => { try { return JSON.parse(kv(['get', key])); } catch { return null; } };

if (cmd === 'approve') {
  if (!args.length) throw new Error('approve <handle> [handle …]');
  for (const raw of args) {
    const handle = raw.replace(/^@/, '');
    if (!/^[A-Za-z0-9_]{1,15}$/.test(handle)) { console.error(`skip ${raw}: not an X handle`); continue; }
    const c = code();
    kv(['put', `invite:${codeHash(c)}`, JSON.stringify({ handle, at: new Date().toISOString() })]);
    console.log(`@${handle}  ${c}`);
  }
} else if (cmd === 'linked') {
  for (const key of invites()) {
    const r = read(key);
    if (r?.account) console.log(r.account);
  }
} else if (cmd === 'status') {
  const rows = invites().map(read).filter(Boolean);
  console.log(`${rows.length} code(s) issued, ${rows.filter((r) => r.account).length} linked`);
  for (const r of rows) console.log(`@${r.handle}  ${r.account ? `linked ${r.linked_at}` : 'not linked yet'}`);
} else {
  console.log('usage: node scripts/invite.mjs approve <handle…> | linked | status');
}
