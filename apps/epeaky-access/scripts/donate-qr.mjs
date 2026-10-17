// The donation page's QR codes, made once from lib/donate.mjs (static SVGs:
// the page runs no code to draw them). Run after changing an address.
//   node scripts/donate-qr.mjs
import { writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { DONATE, uri } from '../lib/donate.mjs';
const require = createRequire('/Users/tobiasd/Downloads/zyn/apps/nap-web/package.json');
const QR = require('qrcode');
for (const d of DONATE) {
  await writeFile(new URL(`../public/donate/qr-${d.id}.svg`, import.meta.url), await QR.toString(uri(d), { type: 'svg', errorCorrectionLevel: 'M', margin: 1 }));
  console.log(d.id, uri(d));
}
