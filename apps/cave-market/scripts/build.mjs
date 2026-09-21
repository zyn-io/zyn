import { mkdir, rm, cp } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
const root = new URL('../', import.meta.url);
const output = new URL('dist/', root);
await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });
for (const file of ['index.html', 'app.js', 'styles.css', 'assets']) {
  await cp(new URL(file, root), new URL(file, output), { recursive: true });
}
console.log(`Cave Market static build: ${fileURLToPath(output)}`);
