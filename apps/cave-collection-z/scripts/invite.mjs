// The whitelist for the gated sale (DECISIONS §74.13, §168), kept in its own
// database: D1 `cave-whitelist`, table `members` (migrations/0001_members.sql).
//
// An approved handle gets a code; its holder links the code to their own Zyn
// account on cavebro.win/mint (functions/api/whitelist/bind.js); linked
// accounts go on the sale's list on Zyn with `zyn-sale-draw allow`, and are
// then marked listed here. Nobody's address is collected before they buy.
//
//   node scripts/invite.mjs approve <handle> [handle …]  print one code each
//   node scripts/invite.mjs linked > accounts.txt        linked, not yet listed
//   node scripts/invite.mjs listed accounts.txt          mark them listed, after `allow`
//   node scripts/invite.mjs status                       every member and where they stand
//
// Only a code's SHA-256 is stored: a code exists in the operator's DM and the
// holder's hands. Runs with the Cloudflare account wrangler is logged in to.
import { execFileSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';

const DB = 'cave-whitelist';
const sql = (command) => {
  const out = execFileSync('npx', ['wrangler@4.40.0', 'd1', 'execute', DB, '--remote', '--json', '--command', command], { encoding: 'utf8', maxBuffer: 64 << 20, cwd: new URL('..', import.meta.url) });
  return JSON.parse(out)[0]?.results || [];
};
const q = (s) => `'${String(s).replace(/'/g, "''")}'`;
const ALPHABET = 'ABCDEFGHJKMNPQRSTUVWXYZ23456789'; // no 0/O, 1/I/L
/// 12 characters from 31: ~59 bits, typed once, never guessed.
const code = () => [...randomBytes(12)].map((b) => ALPHABET[b % ALPHABET.length]).join('').replace(/(.{4})(?=.)/g, '$1-');
export const codeHash = (c) => createHash('sha256').update(`cave-invite:${String(c).toUpperCase().replace(/[^A-Z0-9]/g, '')}`).digest('hex');

const [cmd, ...args] = process.argv.slice(2);
if (cmd === 'approve') {
  if (!args.length) throw new Error('approve <handle> [handle …]');
  for (const raw of args) {
    const handle = raw.replace(/^@/, '');
    if (!/^[A-Za-z0-9_]{1,15}$/.test(handle)) { console.error(`skip ${raw}: not an X handle`); continue; }
    if (sql(`SELECT handle FROM members WHERE handle = ${q(handle)}`).length) { console.error(`skip @${handle}: already approved`); continue; }
    const c = code();
    sql(`INSERT INTO members (handle, code_hash, approved_at) VALUES (${q(handle)}, ${q(codeHash(c))}, ${q(new Date().toISOString())})`);
    console.log(`@${handle}  ${c}`);
  }
} else if (cmd === 'linked') {
  for (const r of sql('SELECT account FROM members WHERE account IS NOT NULL AND listed_at IS NULL ORDER BY linked_at')) console.log(r.account);
} else if (cmd === 'listed') {
  const accounts = readFileSync(args[0], 'utf8').split('\n').map((l) => l.trim()).filter((l) => /^[0-9a-f]{64}$/.test(l));
  if (!accounts.length) throw new Error('listed <file of accounts already allowed on Zyn>');
  sql(`UPDATE members SET listed_at = ${q(new Date().toISOString())} WHERE listed_at IS NULL AND account IN (${accounts.map(q).join(',')})`);
  console.log(`${accounts.length} marked listed`);
} else if (cmd === 'status') {
  const rows = sql('SELECT handle, approved_at, account, linked_at, listed_at FROM members ORDER BY approved_at');
  console.log(`${rows.length} approved, ${rows.filter((r) => r.account).length} linked, ${rows.filter((r) => r.listed_at).length} listed`);
  for (const r of rows) console.log(`@${r.handle}  ${r.listed_at ? 'listed' : r.account ? 'linked — run allow' : 'code sent, not linked'}`);
} else {
  console.log('usage: node scripts/invite.mjs approve <handle…> | linked | listed <file> | status');
}
