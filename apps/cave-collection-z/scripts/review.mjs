// Read the whitelist applications, for review by a person.
//
// Reads KV directly through wrangler rather than adding an admin endpoint: an
// endpoint that lists applicants is a thing to protect, and there is no reason
// to have one when the reviewer already holds the Cloudflare account.
import { execFileSync } from 'node:child_process';

const NS = '8181407749be40ff89d7691132723ae7';

function wrangler(args) {
  return execFileSync('wrangler', args, { encoding: 'utf8', maxBuffer: 64 << 20 });
}

const keys = JSON.parse(wrangler(['kv', 'key', 'list', '--namespace-id', NS, '--remote']))
  .map((k) => k.name)
  .filter((n) => n.startsWith('wl:'))
  .sort();

if (!keys.length) {
  console.log('no applications yet');
  process.exit(0);
}

const rows = [];
for (const key of keys) {
  try {
    rows.push(JSON.parse(wrangler(['kv', 'key', 'get', key, '--namespace-id', NS, '--remote'])));
  } catch {
    console.error(`could not read ${key}`);
  }
}

rows.sort((a, b) => String(a.at).localeCompare(String(b.at)));
for (const r of rows) {
  console.log(`@${r.handle}`);
  console.log(`  applied ${r.at}${r.country ? `  (${r.country})` : ''}`);
  console.log(`  post    ${r.post}`);
  if (r.note) console.log(`  note    ${r.note.replace(/\s+/g, ' ').slice(0, 200)}`);
  console.log('');
}
console.log(`${rows.length} application(s)`);
