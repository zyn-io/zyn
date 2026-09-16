// Build cavebro.win, including the content-addressed media it serves.
//
// The site is static and the blobs are hash-named, so this is a staging step
// rather than a build with opinions: whatever `build_manifests.py` produced is
// what ships, and every client verifies the name against the bytes before
// believing them. That is what makes a plain static host an acceptable place
// to keep art for a shielded chain — the host is a byte source, not an
// authority.
import { cp, mkdir, readdir, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const root = new URL('../', import.meta.url);
const output = new URL('dist/', root);

await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });

for (const file of ['index.html', 'app.js', 'styles.css', 'assets']) {
  await cp(new URL(file, root), new URL(file, output), { recursive: true });
}

// `/whitelist` rather than `/whitelist.html`: Pages serves a bare path from a
// directory's index, which keeps the URL clean without a redirect rule.
await mkdir(new URL('whitelist/', output), { recursive: true });
await cp(new URL('whitelist.html', root), new URL('whitelist/index.html', output));

// The deposit door, same shape. Everything in the cave is priced in ZEC, so
// this is the page that has to exist before any of the prices mean anything.
await mkdir(new URL('deposit/', output), { recursive: true });
await cp(new URL('deposit.html', root), new URL('deposit/index.html', output));

// The QR encoder is shared with the wallets rather than copied into each one.
// This site has no bundler, so "shared" means staged here as a plain module.
await cp(new URL('../shared/qr.js', root), new URL('qr.js', output));

// Pages Functions ship from `functions/` beside the static output.
await cp(new URL('functions', root), new URL('functions', output), { recursive: true });

// A `_redirects` rule pointing at a page that does not exist is inert, so the
// 404 has to be a real file before the rule below means anything.
await writeFile(
  new URL('404.html', output),
  '<!doctype html><meta charset="utf-8"><title>Not here</title>\n'
    + '<p>No object with that name.\n',
);

// No `_redirects`. The site is one page with no client-side routes, so Pages
// serving `404.html` for anything it does not have is both correct and the
// thing a content-addressed fetch needs: the previous catch-all answered every
// missing path with 200 and a page of HTML, which is indistinguishable from a
// hit until you hash the body and find it is not the object you asked for.

// Hash-named means immutable, which means a year of caching is honest rather
// than optimistic. Cross-origin is safe for the same reason: bytes that do not
// match the name are discarded by whoever asked for them.
await writeFile(
  new URL('_headers', output),
  [
    '/blobs/*',
    '  Content-Type: application/octet-stream',
    '  Cache-Control: public, max-age=31536000, immutable',
    '  Access-Control-Allow-Origin: *',
    '  X-Content-Type-Options: nosniff',
    '',
  ].join('\n'),
);

const from = new URL('../nap/branding/nap-mole-collection-333/out/blobs/', root);
let names = [];
try {
  names = await readdir(from);
} catch {
  console.warn('no blobs staged — run apps/nap/branding/nap-mole-collection-333/build_manifests.py');
}
if (names.length) {
  const to = new URL('blobs/', output);
  await mkdir(to, { recursive: true });
  let n = 0;
  for (const name of names) {
    // Only a name that *is* a hash. Anything else here would be served as
    // though it were content-addressed, and it would not be.
    if (!/^[0-9a-f]{64}$/.test(name)) continue;
    await cp(new URL(name, from), new URL(name, to));
    n += 1;
  }
  console.log(`staged ${n} content-addressed blob(s)`);
}

console.log(`Cave Collection Z build: ${fileURLToPath(output)}`);
