// A QR encoder, byte mode, small enough to vendor and self-contained enough to
// ship to a static host with no bundler.
//
// It exists because a deposit address is the one thing on any of these pages
// that must be transferred without a typo, and the wallet holding the ZEC is
// usually not on the machine showing the address. Every surface that displays
// an address — the desktop wallet, the browser wallet, the extension, the web
// pages — needs the same picture, so this is one file they all load rather
// than a dependency three of them cannot take.
//
// Scope is deliberately narrow: 8-bit byte mode, one of the four error
// correction levels, automatic version selection, all eight masks scored per
// the specification. No kanji, no numeric or alphanumeric packing, no ECI.
// A Zcash unified address is long and case-mixed, so byte mode is the mode it
// would choose anyway.
//
// Verified against the `qrcode` npm package in `apps/shared/qr.test.mjs`: the
// module matrices must agree exactly, at every level, across lengths that span
// a version boundary.

const GF_EXP = new Uint8Array(512);
const GF_LOG = new Uint8Array(256);
(() => {
  let x = 1;
  for (let i = 0; i < 255; i++) {
    GF_EXP[i] = x;
    GF_LOG[x] = i;
    x <<= 1;
    if (x & 0x100) x ^= 0x11d; // the QR field polynomial
  }
  for (let i = 255; i < 512; i++) GF_EXP[i] = GF_EXP[i - 255];
})();

const gfMul = (a, b) => (a === 0 || b === 0 ? 0 : GF_EXP[GF_LOG[a] + GF_LOG[b]]);

/** The generator polynomial for `degree` error-correction codewords. */
function generator(degree) {
  let poly = [1];
  for (let i = 0; i < degree; i++) {
    const next = new Array(poly.length + 1).fill(0);
    for (let j = 0; j < poly.length; j++) {
      next[j] ^= poly[j];
      next[j + 1] ^= gfMul(poly[j], GF_EXP[i]);
    }
    poly = next;
  }
  return poly;
}

function remainder(data, degree) {
  const out = new Uint8Array(degree);
  const gen = generator(degree);
  for (const byte of data) {
    const factor = byte ^ out[0];
    out.copyWithin(0, 1);
    out[degree - 1] = 0;
    for (let i = 0; i < degree; i++) out[i] ^= gfMul(gen[i + 1], factor);
  }
  return out;
}

// Per version 1..40: total codewords, then for each level L/M/Q/H the count of
// error-correction codewords per block and the block layout (group1, group2).
// Transcribed from the tables in ISO/IEC 18004.
const EC_LEVELS = { L: 0, M: 1, Q: 2, H: 3 };
const TOTAL_CODEWORDS = [
  26, 44, 70, 100, 134, 172, 196, 242, 292, 346, 404, 466, 532, 581, 655, 733, 815, 901, 991, 1085,
  1156, 1258, 1364, 1474, 1588, 1706, 1828, 1921, 2051, 2185, 2323, 2465, 2611, 2761, 2876, 3034,
  3196, 3362, 3532, 3706,
];
// [ecPerBlock, blocksInGroup1, blocksInGroup2] for each version and level.
const EC_TABLE = [
  [[7,1,0],[10,1,0],[13,1,0],[17,1,0]],[[10,1,0],[16,1,0],[22,1,0],[28,1,0]],
  [[15,1,0],[26,1,0],[18,2,0],[22,2,0]],[[20,1,0],[18,2,0],[26,2,0],[16,4,0]],
  [[26,1,0],[24,2,0],[18,2,2],[22,2,2]],[[18,2,0],[16,4,0],[24,4,0],[28,4,0]],
  [[20,2,0],[18,4,0],[18,2,4],[26,4,1]],[[24,2,0],[22,2,2],[22,4,2],[26,4,2]],
  [[30,2,0],[22,3,2],[20,4,4],[24,4,4]],[[18,2,2],[26,4,1],[24,6,2],[28,6,2]],
  [[20,4,0],[30,1,4],[28,4,4],[24,3,8]],[[24,2,2],[22,6,2],[26,4,6],[28,7,4]],
  [[26,4,0],[22,8,1],[24,8,4],[22,12,4]],[[30,3,1],[24,4,5],[20,11,5],[24,11,5]],
  [[22,5,1],[24,5,5],[30,5,7],[24,11,7]],[[24,5,1],[28,7,3],[24,15,2],[30,3,13]],
  [[28,1,5],[28,10,1],[28,1,15],[28,2,17]],[[30,5,1],[26,9,4],[28,17,1],[28,2,19]],
  [[28,3,4],[26,3,11],[26,17,4],[26,9,16]],[[28,3,5],[26,3,13],[30,15,5],[28,15,10]],
  [[28,4,4],[26,17,0],[28,17,6],[30,19,6]],[[28,2,7],[28,17,0],[30,7,16],[24,34,0]],
  [[30,4,5],[28,4,14],[30,11,14],[30,16,14]],[[30,6,4],[28,6,14],[30,11,16],[30,30,2]],
  [[26,8,4],[28,8,13],[30,7,22],[30,22,13]],[[28,10,2],[28,19,4],[28,28,6],[30,33,4]],
  [[30,8,4],[28,22,3],[30,8,26],[30,12,28]],[[30,3,10],[28,3,23],[30,4,31],[30,11,31]],
  [[30,7,7],[28,21,7],[30,1,37],[30,19,26]],[[30,5,10],[28,19,10],[30,15,25],[30,23,25]],
  [[30,13,3],[28,2,29],[30,42,1],[30,23,28]],[[30,17,0],[28,10,23],[30,10,35],[30,19,35]],
  [[30,17,1],[28,14,21],[30,29,19],[30,11,46]],[[30,13,6],[28,14,23],[30,44,7],[30,59,1]],
  [[30,12,7],[28,12,26],[30,39,14],[30,22,41]],[[30,6,14],[28,6,34],[30,46,10],[30,2,64]],
  [[30,17,4],[28,29,14],[30,49,10],[30,24,46]],[[30,4,18],[28,13,32],[30,48,14],[30,42,32]],
  [[30,20,4],[28,40,7],[30,43,22],[30,10,67]],[[30,19,6],[28,18,31],[30,34,34],[30,20,61]],
];

const ALIGNMENT = [
  [], [6,18], [6,22], [6,26], [6,30], [6,34], [6,22,38], [6,24,42], [6,26,46], [6,28,50],
  [6,30,54], [6,32,58], [6,34,62], [6,26,46,66], [6,26,48,70], [6,26,50,74], [6,30,54,78],
  [6,30,56,82], [6,30,58,86], [6,34,62,90], [6,28,50,72,94], [6,26,50,74,98], [6,30,54,78,102],
  [6,28,54,80,106], [6,32,58,84,110], [6,30,58,86,114], [6,34,62,90,118], [6,26,50,74,98,122],
  [6,30,54,78,102,126], [6,26,52,78,104,130], [6,30,56,82,108,134], [6,34,60,86,112,138],
  [6,30,58,86,114,142], [6,34,62,90,118,146], [6,30,54,78,102,126,150], [6,24,50,76,102,128,154],
  [6,28,54,80,106,132,158], [6,32,58,84,110,136,162], [6,26,54,82,110,138,166],
  [6,30,58,86,114,142,170],
];

function blocks(version, level) {
  const [ecLen, g1, g2] = EC_TABLE[version - 1][EC_LEVELS[level]];
  const total = TOTAL_CODEWORDS[version - 1];
  const count = g1 + g2;
  const dataTotal = total - ecLen * count;
  const short = Math.floor(dataTotal / count);
  return { ecLen, count, short, longCount: dataTotal - short * count, dataTotal };
}

/** The smallest version that holds `byteLength` bytes at this level. */
function pickVersion(byteLength, level) {
  for (let version = 1; version <= 40; version++) {
    const { dataTotal } = blocks(version, level);
    const lengthBits = version < 10 ? 8 : 16;
    if (4 + lengthBits + byteLength * 8 <= dataTotal * 8) return version;
  }
  throw new Error('too much data for a QR code');
}

function codewords(bytes, version, level) {
  const { dataTotal, ecLen, count, short, longCount } = blocks(version, level);
  const lengthBits = version < 10 ? 8 : 16;

  // Bit buffer: mode indicator 0100, length, payload, terminator, pad.
  const bits = [];
  const push = (value, width) => {
    for (let i = width - 1; i >= 0; i--) bits.push((value >> i) & 1);
  };
  push(0b0100, 4);
  push(bytes.length, lengthBits);
  for (const b of bytes) push(b, 8);
  const capacity = dataTotal * 8;
  for (let i = 0; i < 4 && bits.length < capacity; i++) bits.push(0);
  while (bits.length % 8) bits.push(0);
  const pad = [0xec, 0x11];
  for (let i = 0; bits.length < capacity; i++) push(pad[i % 2], 8);

  const data = new Uint8Array(dataTotal);
  for (let i = 0; i < dataTotal; i++) {
    let byte = 0;
    for (let j = 0; j < 8; j++) byte = (byte << 1) | bits[i * 8 + j];
    data[i] = byte;
  }

  // Split into blocks, compute EC per block, then interleave both.
  const dataBlocks = [];
  const ecBlocks = [];
  let at = 0;
  for (let i = 0; i < count; i++) {
    const size = i < count - longCount ? short : short + 1;
    const block = data.subarray(at, at + size);
    at += size;
    dataBlocks.push(block);
    ecBlocks.push(remainder(block, ecLen));
  }

  const out = [];
  const longest = Math.max(...dataBlocks.map((b) => b.length));
  for (let i = 0; i < longest; i++) {
    for (const block of dataBlocks) if (i < block.length) out.push(block[i]);
  }
  for (let i = 0; i < ecLen; i++) {
    for (const block of ecBlocks) out.push(block[i]);
  }
  return Uint8Array.from(out);
}

// 15-bit format information, BCH(15,5) with the 0x5412 mask.
function formatBits(level, mask) {
  const levelBits = { L: 0b01, M: 0b00, Q: 0b11, H: 0b10 }[level];
  const data = (levelBits << 3) | mask;
  let rem = data << 10;
  for (let i = 4; i >= 0; i--) if (rem & (1 << (i + 10))) rem ^= 0x537 << i;
  return ((data << 10) | rem) ^ 0x5412;
}

// 18-bit version information, BCH(18,6), versions 7 and up only.
function versionBits(version) {
  let rem = version << 12;
  for (let i = 5; i >= 0; i--) if (rem & (1 << (i + 12))) rem ^= 0x1f25 << i;
  return (version << 12) | rem;
}

const MASKS = [
  (r, c) => (r + c) % 2 === 0,
  (r) => r % 2 === 0,
  (r, c) => c % 3 === 0,
  (r, c) => (r + c) % 3 === 0,
  (r, c) => (Math.floor(r / 2) + Math.floor(c / 3)) % 2 === 0,
  (r, c) => ((r * c) % 2) + ((r * c) % 3) === 0,
  (r, c) => (((r * c) % 2) + ((r * c) % 3)) % 2 === 0,
  (r, c) => (((r + c) % 2) + ((r * c) % 3)) % 2 === 0,
];

/** Penalty score for a finished matrix — lower is the mask the spec picks. */
function penalty(m, size) {
  let score = 0;
  // Rule 1: runs of five or more.
  for (let i = 0; i < size; i++) {
    for (const read of [(k) => m[i][k], (k) => m[k][i]]) {
      let run = 1;
      for (let k = 1; k < size; k++) {
        if (read(k) === read(k - 1)) run++;
        else {
          if (run >= 5) score += run - 2;
          run = 1;
        }
      }
      if (run >= 5) score += run - 2;
    }
  }
  // Rule 2: 2x2 blocks of one colour.
  for (let r = 0; r < size - 1; r++) {
    for (let c = 0; c < size - 1; c++) {
      const v = m[r][c];
      if (v === m[r][c + 1] && v === m[r + 1][c] && v === m[r + 1][c + 1]) score += 3;
    }
  }
  // Rule 3: the finder-like 1:1:3:1:1 pattern with four light modules beside it.
  const a = [1, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0];
  const b = [0, 0, 0, 0, 1, 0, 1, 1, 1, 0, 1];
  for (let i = 0; i < size; i++) {
    for (let k = 0; k + 11 <= size; k++) {
      let rowA = true, rowB = true, colA = true, colB = true;
      for (let j = 0; j < 11; j++) {
        if (m[i][k + j] !== a[j]) rowA = false;
        if (m[i][k + j] !== b[j]) rowB = false;
        if (m[k + j][i] !== a[j]) colA = false;
        if (m[k + j][i] !== b[j]) colB = false;
      }
      if (rowA) score += 40;
      if (rowB) score += 40;
      if (colA) score += 40;
      if (colB) score += 40;
    }
  }
  // Rule 4: deviation from an even split of dark and light.
  //
  // The spec describes this as "the smallest k such that the proportion
  // deviates by 5k%", which admits more than one reading just above 50%: a
  // symbol that is 50.5% dark scores 0 under a floor-of-the-distance reading
  // and 10 under this ceiling one. Encoders in the wild differ here, and a
  // disagreement changes which mask is chosen and therefore every module of
  // the output. This follows the `qrcode` package, because agreeing exactly
  // with a scanner-tested encoder is worth more than either reading of an
  // ambiguous sentence — and `qr.test.mjs` holds the two together.
  let dark = 0;
  for (let r = 0; r < size; r++) for (let c = 0; c < size; c++) dark += m[r][c];
  const modules = size * size;
  score += Math.abs(Math.ceil(((dark * 100) / modules) / 5) - 10) * 10;
  return score;
}

/**
 * Encode `text` as a QR matrix.
 *
 * Returns `{ size, modules }` where `modules[row][col]` is 1 for dark. The
 * quiet zone is not included — a renderer adds it, because how much margin a
 * given surface wants is a layout question.
 */
export function encode(text, { level = 'M' } = {}) {
  if (!EC_LEVELS.hasOwnProperty(level)) throw new Error(`unknown level ${level}`);
  const bytes = new TextEncoder().encode(text);
  const version = pickVersion(bytes.length, level);
  const size = version * 4 + 17;
  const data = codewords(bytes, version, level);

  const m = Array.from({ length: size }, () => new Uint8Array(size));
  const fixed = Array.from({ length: size }, () => new Uint8Array(size));
  const put = (r, c, v) => {
    m[r][c] = v;
    fixed[r][c] = 1;
  };

  // Finders and their separators.
  for (const [br, bc] of [[0, 0], [0, size - 7], [size - 7, 0]]) {
    for (let r = -1; r <= 7; r++) {
      for (let c = -1; c <= 7; c++) {
        const rr = br + r, cc = bc + c;
        if (rr < 0 || cc < 0 || rr >= size || cc >= size) continue;
        // Outside the 7x7 finder is the separator, which is always light.
        // Treating it as part of the ring is the classic way to produce a
        // picture that still looks like a QR code and scans as nothing.
        const outside = r < 0 || r > 6 || c < 0 || c > 6;
        const edge = r === 0 || r === 6 || c === 0 || c === 6;
        const core = r >= 2 && r <= 4 && c >= 2 && c <= 4;
        put(rr, cc, !outside && (edge || core) ? 1 : 0);
      }
    }
  }
  // Timing patterns.
  for (let i = 8; i < size - 8; i++) {
    put(6, i, i % 2 === 0 ? 1 : 0);
    put(i, 6, i % 2 === 0 ? 1 : 0);
  }
  // Alignment patterns, skipping the three that would sit on a finder.
  const centres = ALIGNMENT[version - 1];
  for (const r of centres) {
    for (const c of centres) {
      if ((r <= 8 && c <= 8) || (r <= 8 && c >= size - 9) || (r >= size - 9 && c <= 8)) continue;
      for (let dr = -2; dr <= 2; dr++) {
        for (let dc = -2; dc <= 2; dc++) {
          const ring = Math.max(Math.abs(dr), Math.abs(dc));
          put(r + dr, c + dc, ring === 1 ? 0 : 1);
        }
      }
    }
  }
  // The dark module, and the format areas reserved before they are written.
  put(size - 8, 8, 1);
  for (let i = 0; i < 9; i++) {
    if (!fixed[8][i]) put(8, i, 0);
    if (!fixed[i][8]) put(i, 8, 0);
  }
  for (let i = 0; i < 8; i++) {
    if (!fixed[8][size - 1 - i]) put(8, size - 1 - i, 0);
    if (!fixed[size - 1 - i][8]) put(size - 1 - i, 8, 0);
  }
  if (version >= 7) {
    const bits = versionBits(version);
    for (let i = 0; i < 18; i++) {
      const bit = (bits >> i) & 1;
      put(Math.floor(i / 3), size - 11 + (i % 3), bit);
      put(size - 11 + (i % 3), Math.floor(i / 3), bit);
    }
  }

  // Lay the codewords in the two-module-wide serpentine, skipping column 6.
  let bit = 0;
  let upward = true;
  for (let right = size - 1; right > 0; right -= 2) {
    if (right === 6) right = 5;
    for (let step = 0; step < size; step++) {
      const row = upward ? size - 1 - step : step;
      for (const col of [right, right - 1]) {
        if (fixed[row][col]) continue;
        const byte = data[bit >> 3];
        m[row][col] = byte === undefined ? 0 : (byte >> (7 - (bit & 7))) & 1;
        bit++;
      }
    }
    upward = !upward;
  }

  // Try every mask, keep the best-scoring one.
  let best = null;
  for (let mask = 0; mask < 8; mask++) {
    const candidate = m.map((row) => Uint8Array.from(row));
    for (let r = 0; r < size; r++) {
      for (let c = 0; c < size; c++) {
        if (!fixed[r][c] && MASKS[mask](r, c)) candidate[r][c] ^= 1;
      }
    }
    const bits = formatBits(level, mask);
    for (let i = 0; i < 15; i++) {
      // Most significant bit first: (8,0) carries bit 14. Placing it the other
      // way round produces a scannable-looking code that decodes at the wrong
      // error-correction level, or not at all.
      const v = (bits >> (14 - i)) & 1;
      // The two copies, laid out as the specification places them.
      if (i < 6) candidate[8][i] = v;
      else if (i < 8) candidate[8][i + 1] = v;
      else if (i === 8) candidate[7][8] = v;
      else candidate[14 - i][8] = v;
      // Bit 7 appears in both halves of the second copy: once at the square
      // the dark module then claims, and once at the start of the row. These
      // are two conditions rather than a branch for exactly that reason.
      if (i < 8) candidate[size - 1 - i][8] = v;
      if (i >= 7) candidate[8][size - 15 + i] = v;
    }
    // The dark module is not part of the format string and is written last,
    // because the second copy's eighth bit lands on the same square.
    candidate[size - 8][8] = 1;
    const score = penalty(candidate, size);
    if (!best || score < best.score) best = { score, modules: candidate };
  }

  return { size, modules: best.modules, version, level };
}

/**
 * Draw into a canvas element. `dark`/`light` are any CSS colour; `margin` is in
 * modules and defaults to the four the specification asks for.
 */
export function toCanvas(canvas, text, opts = {}) {
  const { level = 'M', margin = 4, scale, width, dark = '#000000', light = '#ffffff' } = opts;
  const { size, modules } = encode(text, { level });
  const total = size + margin * 2;
  const px = scale || Math.max(1, Math.floor((width || 256) / total));
  canvas.width = total * px;
  canvas.height = total * px;
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = light;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = dark;
  for (let r = 0; r < size; r++) {
    for (let c = 0; c < size; c++) {
      if (modules[r][c]) ctx.fillRect((c + margin) * px, (r + margin) * px, px, px);
    }
  }
  return canvas;
}

/** An SVG string, for surfaces that would rather not own a canvas. */
export function toSvg(text, opts = {}) {
  const { level = 'M', margin = 4, dark = '#000000', light = '#ffffff' } = opts;
  const { size, modules } = encode(text, { level });
  const total = size + margin * 2;
  let path = '';
  for (let r = 0; r < size; r++) {
    for (let c = 0; c < size; c++) {
      if (modules[r][c]) path += `M${c + margin} ${r + margin}h1v1h-1z`;
    }
  }
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${total} ${total}" shape-rendering="crispEdges">`
    + `<rect width="${total}" height="${total}" fill="${light}"/>`
    + `<path d="${path}" fill="${dark}"/></svg>`
  );
}
