// Jade keeps its own copy of the curve and pool arithmetic (it is a page with
// no build step that could share Nap's module). A copy that drifts quotes one
// thing and Nap signs another, so every function is run here against Nap's
// own, over many inputs. Also pins what the 27 Sep review found in Jade.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const jade = await readFile(new URL('../public/jade.js', import.meta.url), 'utf8');
const mk = await import('../../nap-web/src/market.js').catch(() => null);
const SKIP = mk ? false : 'apps/nap-web is not checked out beside this app';

/// Jade's own functions, taken from its source, run in isolation.
function jadeFns() {
  const grab = (re) => { const m = jade.match(re); assert.ok(m, `jade.js has ${re}`); return m[0]; };
  const src = [
    grab(/const CURVE = Object\.freeze\([^;]+\);/),
    'const WAD = 10n ** 18n; const ZAT = 10_000_000_000n; const LOT = CURVE.lotTokens * WAD; const BPS = 10_000n;',
    grab(/const cost = \(q, d\) => \{[^\n]+\n/),
    grab(/function lotsFor\(q, budget, cap\) \{[\s\S]*?\n\}/),
    grab(/function previewBuy\(sold, amountIn, feeBps\) \{[\s\S]*?\n\}/),
    grab(/function previewSell\(sold, lots, feeBps\) \{[\s\S]*?\n\}/),
    grab(/function poolQuote\(pool, sellA, amountIn\) \{[\s\S]*?\n\}/),
    grab(/function decodeMeta\(hexMeta\) \{[\s\S]*?\n\}/),
    'return { cost, lotsFor, previewBuy, previewSell, poolQuote, decodeMeta, CURVE };',
  ].join('\n');
  return new Function(src)(); // eslint-disable-line no-new-func -- Jade's own source, checked in
}

test('Jade\'s curve and pool arithmetic is Nap\'s, to the unit', { skip: SKIP }, () => {
  const j = jadeFns();
  assert.deepEqual({ ...j.CURVE }, { lots: mk.CURVE.lots, lotTokens: mk.CURVE.lotTokens, baseZat: mk.CURVE.baseZat, slopeZat: mk.CURVE.slopeZat, graduateAt: mk.CURVE.graduateAt });
  let seed = 7;
  const rnd = (n) => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed % n; };
  for (let i = 0; i < 400; i++) {
    const sold = rnd(51_098);
    const amountIn = BigInt(1 + rnd(1e9)) * 10n ** BigInt(8 + rnd(9));
    const fee = [0, 30, 100][rnd(3)];
    const a = j.previewBuy(sold, amountIn, fee);
    const b = mk.previewCurveBuy(sold, amountIn, fee);
    assert.deepEqual([a.lots, a.tokens, a.paid, a.fee], [b.lots, b.tokens, b.paid, b.fee], `buy sold=${sold} in=${amountIn} fee=${fee}`);
    const lots = 1 + rnd(Math.max(1, sold));
    const c = j.previewSell(sold, lots, fee);
    const d = mk.previewCurveSell(sold, lots, fee);
    assert.deepEqual([c.gross, c.fee, c.net], [d.gross, d.fee, d.net], `sell sold=${sold} lots=${lots}`);
    const pool = { reserve_a: String(BigInt(1 + rnd(1e9)) * 10n ** 12n), reserve_b: String(BigInt(1 + rnd(1e9)) * 10n ** 9n), fee_bps: [5, 30, 100][rnd(3)] };
    const sellA = rnd(2) === 0;
    assert.equal(j.poolQuote(pool, sellA, amountIn), mk.quoteAlone(pool, sellA, amountIn), 'pool quote');
  }
});

test('Jade reads a coin\'s name as Nap does: padding must be zeros', { skip: SKIP }, () => {
  const j = jadeFns();
  const meta = (bytes) => Array.from(bytes, (x) => x.toString(16).padStart(2, '0')).join('');
  const good = new Uint8Array(32); good.set(new TextEncoder().encode('ZTEST'), 0); good.set(new TextEncoder().encode('Zyn Test Stone'), 8);
  assert.deepEqual(j.decodeMeta(meta(good)), mk.decodeMeta(meta(good)));
  const dirty = good.slice(); dirty[8 + 14 + 1] = 0x41; // garbage after the name's end
  assert.equal(j.decodeMeta(meta(dirty)).symbol, '?', 'Jade refuses what Nap refuses');
  assert.equal(mk.decodeMeta(meta(dirty)), null);
});

test('the fixes from the 27 Sep review stay in', () => {
  // A one-lot buy used to send a floor of none, which Nap refuses.
  assert.match(jade, /const minTokens = \(floorLots > 0n \? floorLots : 1n\) \* LOT;/);
  // The last order's status has its own line that a refresh never wipes.
  assert.match(jade, /hint\.textContent = e\.message;/);
  assert.doesNotMatch(jade, /\$\('#trade-status'\)\.textContent = '';\n  if \(!c\) return null;/);
  // A buy over the wallet's balance is refused on the page.
  assert.match(jade, /value > mine\.zec\) throw new Error/);
  // Creating is one at a time, and the coin is reported even if its picture fails.
  assert.match(jade, /if \(creating\) return;/);
  assert.match(jade, /The coin exists from here on, whatever happens to the picture\./);
  // The bond is not promised back.
  assert.doesNotMatch(jade, /returned at graduation/);
  // Every request names its network.
  assert.match(jade, /params: \{ \.\.\.\(p \|\| \{\}\), network: NETWORK \}/);
});

test('the page ships the same Nap Connect as the source', async () => {
  const src = await readFile(new URL('../public/nap-connect.js', import.meta.url), 'utf8');
  const shared = await readFile(new URL('../../nap-connect/nap-connect.js', import.meta.url), 'utf8').catch(() => null);
  if (shared) assert.equal(src, shared, 'apps/jade/public/nap-connect.js is the shared nap-connect.js');
});
