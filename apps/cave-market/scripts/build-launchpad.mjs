import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const root = new URL('../', import.meta.url);
// Into the deploy stage, next to Jade's Pages Functions and wrangler.toml.
const output = new URL('../jade-stage/dist/', import.meta.url);
const [html, css, js] = await Promise.all([
  readFile(new URL('jade.html', root), 'utf8'),
  readFile(new URL('jade.css', root), 'utf8'),
  readFile(new URL('jade.js', root), 'utf8'),
]);

await rm(output, { recursive: true, force: true });
await mkdir(new URL('assets/fonts/', output), { recursive: true });
await mkdir(new URL('vendor/', output), { recursive: true });
await Promise.all([
  writeFile(new URL('index.html', output), html),
  writeFile(new URL('jade.css', output), css),
  writeFile(new URL('jade.js', output), js),
  cp(new URL('nap-connect.js', root), new URL('nap-connect.js', output)),
  cp(new URL('vendor/lightweight-charts.js', root), new URL('vendor/lightweight-charts.js', output)),
  cp(new URL('vendor/LIGHTWEIGHT-CHARTS-LICENSE', root), new URL('vendor/LIGHTWEIGHT-CHARTS-LICENSE', output)),
  writeFile(new URL('_redirects', output), '/* /index.html 200\n'),
  cp(new URL('jade-headers', root), new URL('_headers', output)),
  cp(new URL('assets/fonts/DM-Sans.ttf', root), new URL('assets/fonts/DM-Sans.ttf', output)),
]);

console.log(`Cave Jade standalone build: ${fileURLToPath(new URL('index.html', output))}`);
