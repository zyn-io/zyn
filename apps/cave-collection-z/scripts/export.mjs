// Take the register out of Cloudflare and put it in a file.
//
// KV is durable, but it is one company's copy of a list you will act on. This
// writes a timestamped snapshot you hold, so losing access to the account —
// or deleting the wrong namespace — costs a snapshot's age rather than
// everything. Run it before you start reviewing, and after.
//
// The Telegram messages are a second copy already, arriving one at a time as
// applications do. This is the one you can grep.
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';

const NS = '8181407749be40ff89d7691132723ae7';
const OUT = new URL('../register/', import.meta.url);

const wrangler = (args) =>
  execFileSync('wrangler', args, { encoding: 'utf8', maxBuffer: 64 << 20 });

const keys = JSON.parse(wrangler(['kv', 'key', 'list', '--namespace-id', NS, '--remote']))
  .map((k) => k.name)
  .filter((n) => n.startsWith('wl:'))
  .sort();

const applications = [];
for (const key of keys) {
  try {
    applications.push(JSON.parse(wrangler(['kv', 'key', 'get', key, '--namespace-id', NS, '--remote'])));
  } catch {
    console.error(`could not read ${key} — recorded as unreadable`);
    applications.push({ handle: key.slice(3), unreadable: true });
  }
}
applications.sort((a, b) => String(a.at).localeCompare(String(b.at)));

mkdirSync(OUT, { recursive: true });
const stamp = new Date().toISOString().replace(/[:.]/g, '-');
const path = new URL(`whitelist-${stamp}.json`, OUT);
writeFileSync(path, JSON.stringify({ exported: new Date().toISOString(), count: applications.length, applications }, null, 2));

// A flat list too: one handle per line is what you paste into a spreadsheet or
// hand to whatever issues the invitations.
const flat = new URL(`whitelist-${stamp}.txt`, OUT);
writeFileSync(flat, applications.map((a) => `@${a.handle}\t${a.post || ''}`).join('\n') + '\n');

console.log(`${applications.length} application(s)`);
console.log(`  ${path.pathname}`);
console.log(`  ${flat.pathname}`);
