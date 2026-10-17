import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { test } from 'node:test';
import { DONATE, uri } from '../lib/donate.mjs';
import { decode } from '../public/shared.mjs';

const read = (p) => readFile(new URL(p, import.meta.url), 'utf8');
const require = createRequire('/Users/tobiasd/Downloads/zyn/apps/nap-web/package.json');
const { keccak_256 } = await import('/Users/tobiasd/Downloads/zyn/apps/nap-web/node_modules/@noble/hashes/sha3.js');

test('each address is well formed for its chain, with a valid checksum', () => {
  const byId = Object.fromEntries(DONATE.map((d) => [d.id, d]));
  // ZEC: a unified address, bech32m with hrp "u".
  const CH = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
  const polymod = (v) => { const g = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]; let c = 1; for (const x of v) { const b = c >>> 25; c = (((c & 0x1ffffff) << 5) ^ x) >>> 0; for (let i = 0; i < 5; i++) if ((b >> i) & 1) c = (c ^ g[i]) >>> 0; } return c; };
  const z = byId.zec.address;
  const sep = z.lastIndexOf('1');
  const hrp = z.slice(0, sep);
  assert.equal(hrp, 'u');
  const v = [...hrp].map((c) => c.charCodeAt(0) >> 5).concat([0], [...hrp].map((c) => c.charCodeAt(0) & 31), [...z.slice(sep + 1)].map((c) => CH.indexOf(c)));
  assert.ok(!v.includes(-1) && polymod(v) === 0x2bc830a3, 'ZEC checksum');
  // SOL: 32 bytes of base58.
  assert.equal(decode(byId.sol.address).length, 32, 'SOL is 32 bytes');
  // ETH: EIP-55 mixed-case checksum.
  const a = byId.eth.address.slice(2);
  const h = Buffer.from(keccak_256(new TextEncoder().encode(a.toLowerCase()))).toString('hex');
  assert.equal([...a].map((c, i) => (parseInt(h[i], 16) >= 8 ? c.toUpperCase() : c.toLowerCase())).join(''), a, 'ETH checksum');
});

test('the page shows each address, copies it, and says its minimum', async () => {
  const html = await read('../public/donate/index.html');
  for (const d of DONATE) {
    assert.equal(html.split(d.address).length - 1, 2, `${d.id}: shown once and copied once`);
    assert.match(html, new RegExp(`data-copy="${d.address}"`), `${d.id}: the Copy button copies this address`);
    assert.match(html, new RegExp(`Minimum ${d.min.replace('.', '\\.')} ${d.unit}`), `${d.id}: minimum`);
    assert.match(html, new RegExp(`src="/donate/qr-${d.id}\\.svg"`), `${d.id}: its QR code`);
  }
  assert.doesNotMatch(html, /Zyn|Nap|bridge|vault/i, 'nothing about where it leads');
});

test("each QR code is the one made from its address and nothing else", async () => {
  const QR = require('qrcode');
  for (const d of DONATE) {
    const want = await QR.toString(uri(d), { type: 'svg', errorCorrectionLevel: 'M', margin: 1 });
    assert.equal(await read(`../public/donate/qr-${d.id}.svg`), want, `${d.id}: regenerate with scripts/donate-qr.mjs`);
  }
});
