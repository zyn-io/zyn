// Differential test: our encoder against the `qrcode` package.
//
// A QR encoder is the kind of code that looks right and is wrong — a mask
// scored slightly differently, one misplaced format bit, and the picture still
// looks like a QR code but scans as nothing, or worse as something else. So
// this does not check properties. It checks that every module of every matrix
// matches a library that has been scanned billions of times.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

import { encode, toSvg } from './qr.js';

const require = createRequire(new URL('../nap-web/package.json', import.meta.url));
const qrcode = require('qrcode');

// `mode: 'byte'` is not a convenience: left to itself the library picks the
// tightest mode for the input, so an all-uppercase string like "ZEC" encodes as
// alphanumeric and produces a legitimately different symbol. This encoder is
// byte-mode only and says so, and the comparison has to hold that fixed or it
// is measuring the wrong difference.
const reference = (text, level) => {
  // The mode has to be pinned on the *segment*; the top-level `mode` option is
  // silently ignored, which is its own small trap.
  const qr = qrcode.create([{ data: text, mode: 'byte' }], { errorCorrectionLevel: level });
  const { size, data } = qr.modules;
  return { size, get: (r, c) => (data[r * size + c] ? 1 : 0), version: qr.version };
};

// Real subjects, plus lengths that straddle a version boundary. The unified
// address is the case this exists for and the longest thing any of these pages
// will ever draw.
const UNIFIED = 'utest1vergg5jkp4xy8sqfasw6s5eqrkdkhxnrxhuqvv3u3nkhpzf0pqtnlnfdqnkftpzvzhvqx9wtn0u3rtnlg2qx4v6p3zsnqqfynw4yhjr8yzn8z3tdgvmhf4x6fw4h9kc6cvq9sx6rpc0mkjgnk9kczur7h3qpvhkdz4p4rz7rjx2q9v0e6tmr2z6ztv3mzn9pjx6kszq8k2p9hlq6w';
const SUBJECTS = [
  'ZYN1:b85db260ec3a7c0a22c19c1f3380bfc75599c0ea4eeeeda69177ab12f9da56ea',
  UNIFIED,
  'zs1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq',
  'a',
  'x'.repeat(14),   // version 1 at L
  'x'.repeat(15),   // ...and the step to version 2
  'x'.repeat(271),  // across the 8/16-bit length-field boundary at version 10
  'x'.repeat(272),
  'ZEC',           // all-uppercase, the case that exposed a mode mismatch
  'ZYN 4444',      // uppercase and a space: alphanumeric if the mode were free
];

test('every module matches the reference encoder', () => {
  for (const level of ['L', 'M', 'Q', 'H']) {
    for (const text of SUBJECTS) {
      const mine = encode(text, { level });
      const theirs = reference(text, level);
      assert.equal(
        mine.size,
        theirs.size,
        `size differs for level ${level}, ${text.length} bytes`,
      );
      assert.equal(mine.version, theirs.version, `version differs for level ${level}`);
      for (let r = 0; r < mine.size; r++) {
        for (let c = 0; c < mine.size; c++) {
          assert.equal(
            mine.modules[r][c],
            theirs.get(r, c),
            `module ${r},${c} differs at level ${level} for ${text.length} bytes`,
          );
        }
      }
    }
  }
});

test('the SVG carries one rect per dark module and the quiet zone', () => {
  const { size, modules } = encode(UNIFIED, { level: 'M' });
  let dark = 0;
  for (let r = 0; r < size; r++) for (let c = 0; c < size; c++) dark += modules[r][c];
  const svg = toSvg(UNIFIED, { level: 'M', margin: 4 });
  assert.equal((svg.match(/h1v1h-1z/g) || []).length, dark);
  assert.match(svg, new RegExp(`viewBox="0 0 ${size + 8} ${size + 8}"`));
});

test('an unknown level is refused rather than silently downgraded', () => {
  assert.throws(() => encode('x', { level: 'Z' }), /unknown level/);
});
