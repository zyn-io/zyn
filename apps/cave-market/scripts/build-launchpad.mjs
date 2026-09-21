import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const root = new URL('../', import.meta.url);
const output = new URL('../dist/', import.meta.url);
const [html, css, js] = await Promise.all([
  readFile(new URL('jade.html', root), 'utf8'),
  readFile(new URL('jade.css', root), 'utf8'),
  readFile(new URL('jade.js', root), 'utf8'),
]);

await rm(output, { recursive: true, force: true });
await mkdir(new URL('assets/fonts/', output), { recursive: true });
await Promise.all([
  writeFile(new URL('index.html', output), html),
  writeFile(new URL('jade.css', output), css),
  writeFile(new URL('jade.js', output), js),
  writeFile(new URL('_redirects', output), '/* /index.html 200\n'),
  cp(new URL('jade-headers', root), new URL('_headers', output)),
  cp(new URL('assets/fonts/DM-Sans.ttf', root), new URL('assets/fonts/DM-Sans.ttf', output)),
]);

console.log(`Cave Jade standalone build: ${fileURLToPath(new URL('index.html', output))}`);
