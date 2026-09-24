// ZynScan is static: copy the page, its script and styles, and the headers
// that pin where it may connect (only the Zyn gateway).
import { cp, mkdir, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const root = new URL('../', import.meta.url);
const out = new URL('dist/', root);
await rm(out, { recursive: true, force: true });
await mkdir(new URL('assets/fonts/', out), { recursive: true });
for (const f of ['index.html', 'app.js', 'styles.css']) await cp(new URL(f, root), new URL(f, out));
await cp(new URL('assets/fonts/DM-Sans.ttf', root), new URL('assets/fonts/DM-Sans.ttf', out));
await writeFile(new URL('_redirects', out), '/* /index.html 200\n');
await writeFile(new URL('_headers', out), [
  '/*',
  '  X-Content-Type-Options: nosniff',
  '  Referrer-Policy: no-referrer',
  '  X-Frame-Options: DENY',
  '  Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=()',
  "  Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src https://zyn-rpc.cavebro.win; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
  '',
  '/assets/*',
  '  Cache-Control: public, max-age=31536000, immutable',
  '',
].join('\n'));
console.log(`ZynScan build: ${fileURLToPath(out)}`);
