import { cp, mkdir, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const root = new URL('../', import.meta.url);
const output = new URL('dist/', root);

await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });
await mkdir(new URL('assets/fonts/', output), { recursive: true });

for (const file of ['index.html', 'styles.css', 'app.js', 'status.js']) {
  await cp(new URL(file, root), new URL(file, output));
}

const collectionAssets = new URL('../cave-collection-z/assets/', root);
const marketFonts = new URL('../cave-market/assets/fonts/', root);
await cp(new URL('cave-mark.svg', collectionAssets), new URL('assets/cave-mark.svg', output));
await cp(new URL('assets/cave-node-chamber.webp', root), new URL('assets/cave-node-chamber.webp', output));
await cp(new URL('DM-Sans.ttf', marketFonts), new URL('assets/fonts/DM-Sans.ttf', output));
await cp(new URL('Tiny5-Regular.ttf', marketFonts), new URL('assets/fonts/Tiny5-Regular.ttf', output));
await cp(new URL('Silkscreen-Regular.ttf', marketFonts), new URL('assets/fonts/Silkscreen-Regular.ttf', output));

await writeFile(
  new URL('_headers', output),
  [
    '/*',
    "  Content-Security-Policy: default-src 'self'; connect-src 'self'; font-src 'self'; img-src 'self' data:; script-src 'self'; style-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
    '  Referrer-Policy: no-referrer',
    '  X-Content-Type-Options: nosniff',
    '  X-Frame-Options: DENY',
    '  Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=()',
    '',
    '/api/status',
    '  Cache-Control: no-store',
    '',
  ].join('\n'),
);

console.log(`Cave node status build: ${fileURLToPath(output)}`);
