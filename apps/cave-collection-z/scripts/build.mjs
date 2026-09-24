// Build cavebro.win, including the content-addressed media it serves.
//
// The site is static and the blobs are hash-named, so this is a staging step
// rather than a build with opinions: whatever `build_manifests.py` produced is
// what ships, and every client verifies the name against the bytes before
// believing them. That is what makes a plain static host an acceptable place
// to keep art for a shielded chain — the host is a byte source, not an
// authority.
import { cp, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
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

// The terms, at a stable path. They have to be fetchable by anyone who wants
// to recompute the digest, which is the whole point of publishing them.
await mkdir(new URL('terms/', output), { recursive: true });
await cp(new URL('terms.html', root), new URL('terms/index.html', output));

// The canonical bytes on their own, so a verifier can curl them rather than
// scrape them out of a web page.
await cp(new URL('terms.hex', root), new URL('cave-sale-terms-v1.hex', output));

// The checkout. Same shape again: `/mint` is where an invitation becomes a
// paid entry, and it is the only page on the site that asks for money.
await mkdir(new URL('mint/', output), { recursive: true });
await cp(new URL('mint.html', root), new URL('mint/index.html', output));

// The second half of the journey. Everything above gets a buyer as far as
// owning an item; these are what they do next — see it, price it, sell it,
// buy another, and leave. Same directory shape, so every one is a clean URL.
for (const page of ['items', 'item', 'market', 'exit']) {
  await mkdir(new URL(`${page}/`, output), { recursive: true });
  await cp(new URL(`${page}.html`, root), new URL(`${page}/index.html`, output));
}

// The QR encoder is shared with the wallets rather than copied into each one.
// This site has no bundler, so "shared" means staged here as a plain module.
await cp(new URL('../shared/qr.js', root), new URL('qr.js', output));

// The shelf, item and market pages speak to Nap through Nap Connect — the same
// file Nap ships — and share one module for it.
await cp(new URL('zstore.js', root), new URL('zstore.js', output));
await cp(new URL('solana-burn.js', root), new URL('solana-burn.js', output));
// The burn button's token (§145): one file per network. Mainnet's exists only
// once the token does; without it the page offers no burn.
for (const f of ['burn.json', 'burn-testnet.json']) {
  await cp(new URL(f, root), new URL(f, output)).catch((e) => { if (e.code !== 'ENOENT') throw e; });
}
await cp(new URL('../nap-connect/nap-connect.js', root), new URL('nap-connect.js', output));

// Collection Z's public index: for every serial, its name, the hash of its
// image blob, its rarity and its traits — the frozen metadata, in one file the
// item pages can read without a wallet. Built from the manifest's own
// content.json (the image hashes the root commits to) and the metadata files.
{
  const base = new URL('../../examples/zyn-pfp-concepts/', root);
  try {
    const content = JSON.parse(await readFile(new URL('out/content.json', base), 'utf8'));
    const species = [['mole-collection-2222', 2222], ['owl-collection-1333', 1333], ['moth-collection-889', 889]];
    const skip = new Set(['Species', 'Canvas', 'Canvas Row', 'Canvas Column', 'Interactive Light ID', 'Component Tagging', 'Rarity', 'Rarity Rank', 'Visual Family']);
    const items = [];
    for (const [dir, supply] of species) {
      for (let k = 1; k <= supply; k += 1) {
        const meta = JSON.parse(await readFile(new URL(`${dir}/metadata/${String(k).padStart(4, '0')}.json`, base), 'utf8'));
        const serial = items.length;
        const media = content[String(serial)]?.media?.[0]?.sha256;
        if (!media || content[String(serial)].name !== meta.name) throw new Error(`serial ${serial}: content.json and metadata disagree`);
        const traits = { Species: meta.attributes.find((a) => a.trait_type === 'Species')?.value };
        for (const a of meta.attributes) if (!skip.has(a.trait_type)) traits[a.trait_type] = String(a.value);
        items.push({ n: meta.name, i: media, r: meta.rarity?.class, k: meta.rarity?.rank, t: traits });
      }
    }
    await writeFile(new URL('collection-z.json', output), JSON.stringify({ collection: '024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e', supply: items.length, items }));
    console.log(`collection index: ${items.length} items`);
  } catch (e) {
    console.warn(`no collection index — ${e.message}`);
  }
}

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
//
// Everything else gets the headers a site that talks to a wallet should have
// had from the start. The node status page shipped a full policy; this one --
// the one that takes money -- shipped none, so the checkout could be framed
// and a click on an invisible overlay was a click on "pay".
//
// The CSP names the SHA-256 of every inline script the site serves. That is
// what makes `script-src` meaningful here: the pages have no external scripts
// at all, so anything the browser is asked to run that is not on this list was
// injected. Computed rather than written down, because a hash maintained by
// hand is a hash that goes stale and takes the site down with it.
const pages = ['index.html', 'whitelist.html', 'deposit.html', 'terms.html', 'mint.html',
  'items.html', 'item.html', 'market.html', 'exit.html'];
const scriptHashes = new Set();
for (const page of pages) {
  const html = await readFile(new URL(page, root), 'utf8');
  for (const m of html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)) {
    // Only inline bodies: a <script src=...> has nothing to hash, and this
    // site has none.
    if (!m[1].trim()) continue;
    scriptHashes.add(`'sha256-${createHash('sha256').update(m[1], 'utf8').digest('base64')}'`);
  }
}

const csp = [
  "default-src 'none'",
  // Extensions announce Nap Connect with a script of their own.
  `script-src 'self' ${[...scriptHashes].join(' ')} chrome-extension: moz-extension: safari-web-extension:`,
  // Style attributes are used on a handful of elements. Inline *styles* cannot
  // exfiltrate or execute the way inline scripts can, so this is the one place
  // the policy relaxes.
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data:",
  "font-src 'self'",
  // The pages reach their own Functions and nothing else. The wallet is spoken
  // to through an injected object, not over the network, so no origin needs to
  // be named here for it.
  // Its own Functions, and the chain's public RPC for the order book.
  "connect-src 'self' https://zyn-rpc.cavebro.win",
  "base-uri 'none'",
  "form-action 'none'",
  "frame-ancestors 'none'",
  "object-src 'none'",
].join('; ');

await writeFile(
  new URL('_headers', output),
  [
    '/blobs/*',
    '  Content-Type: application/octet-stream',
    '  Cache-Control: public, max-age=31536000, immutable',
    '  Access-Control-Allow-Origin: *',
    '  X-Content-Type-Options: nosniff',
    '',
    '/*',
    `  Content-Security-Policy: ${csp}`,
    '  X-Frame-Options: DENY',
    '  X-Content-Type-Options: nosniff',
    '  Referrer-Policy: no-referrer',
    '  Permissions-Policy: geolocation=(), microphone=(), camera=(), payment=()',
    // Nap in the browser answers from its own window; same-origin would cut it off.
    '  Cross-Origin-Opener-Policy: same-origin-allow-popups',
    '',
  ].join('\n'),
);

// Content-addressed blobs: the 333's, and Collection Z's 4,444 (images and
// manifests, from examples/zyn-pfp-concepts/build_collection_z_manifests.py).
// Each file is staged only if its bytes hash to its name — it is served as
// content-addressed, so a file that is not would be a lie with a long cache.
const sources = [
  ['../nap/branding/nap-mole-collection-333/out/blobs/', 'apps/nap/branding/nap-mole-collection-333/build_manifests.py'],
  ['../../examples/zyn-pfp-concepts/out/blobs/', 'examples/zyn-pfp-concepts/build_collection_z_manifests.py'],
];
const to = new URL('blobs/', output);
await mkdir(to, { recursive: true });
let n = 0;
for (const [dir, builder] of sources) {
  const from = new URL(dir, root);
  let names = [];
  try {
    names = await readdir(from);
  } catch {
    console.warn(`no blobs staged from ${dir} — run ${builder}`);
  }
  for (const name of names) {
    if (!/^[0-9a-f]{64}$/.test(name)) continue;
    // Media are named by sha256(bytes); manifests by their leaf hash,
    // sha256(0x00 || bytes), as the manifest builders write them.
    const bytes = await readFile(new URL(name, from));
    const plain = createHash('sha256').update(bytes).digest('hex');
    const leaf = createHash('sha256').update(Buffer.from([0])).update(bytes).digest('hex');
    if (plain !== name && leaf !== name) {
      throw new Error(`${dir}${name} does not hash to its name; refusing to publish it`);
    }
    await cp(new URL(name, from), new URL(name, to));
    n += 1;
  }
}
console.log(`staged ${n} content-addressed blob(s)`);

console.log(`Cave Collection Z build: ${fileURLToPath(output)}`);
