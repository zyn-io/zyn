import test from 'node:test';
import assert from 'node:assert/strict';
import { deriveIdentity, hex, parseAmount, signedRead, transferFrame, unhex, vmId, WAD } from '../src/core.js';

const WORDS = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art';

test('browser derivation matches the normative Rust vectors', () => {
  const zero = deriveIdentity(WORDS, '', 0);
  assert.equal(hex(zero.secretKey), '57d47cefdba062bb9669a7a64e9072e49d2b5bc66892952429240e4c91b16183');
  assert.equal(hex(zero.publicKey), '308ab8b209813f5912287682b50950d62782abc61507f0a80abafd0f7a33a7a6');
  assert.equal(zero.accountHex, 'b85db260ec3a7c0a22c19c1f3380bfc75599c0ea4eeeeda69177ab12f9da56ea');
  const seven = deriveIdentity(WORDS, 'nap passphrase', 7);
  assert.equal(hex(seven.secretKey), 'f4e1b20f8a0cd2e19ae9d85ce3057cbb13863be3630e87972afce0b14c513c2e');
  assert.equal(hex(seven.publicKey), '276237e6804911ecd6d44c3d170ac67ff8dc8abf87cf423489a71dddf68857eb');
  assert.equal(seven.accountHex, '4c976ef0d248340b910e246439c3139911f1752e6ba1d3c198f41071e4503604');
});

test('signed reads and transfers have canonical wire lengths', () => {
  const id = deriveIdentity(WORDS, '', 0);
  const read = signedRead(id, 11, 42);
  const frame = transferFrame(id, 11, 42, unhex('11'.repeat(32), 32), '0'.repeat(63) + '7', WAD);
  assert.equal(hex(vmId()), '5cf4557b297390b6c6d08f752fc04a0ac8496349d6fbd0f68657328812d4e5ee');
  assert.equal(hex(read), 'b85db260ec3a7c0a22c19c1f3380bfc75599c0ea4eeeeda69177ab12f9da56ea000000000000002a01308ab8b209813f5912287682b50950d62782abc61507f0a80abafd0f7a33a7a612d919d7b6805ae80ff169cbbf7be20b8548456aaf38d6c60a892dc25be1c0fa28d8e5af2a0bb8411aec809507942a8e1b58219c97f151141b172e16b18d210e');
  assert.equal(hex(frame), '5cf4557b297390b6c6d08f752fc04a0ac8496349d6fbd0f68657328812d4e5ee000000000000008e01308ab8b209813f5912287682b50950d62782abc61507f0a80abafd0f7a33a7a653e89fba1119f7490c8b9c0e58f89db87d4b1e1402b1e9335d05a72b5440f4f4b492289fab01e5a1b05dd80879d1b80ecef529c4c3ff3d7e433a6a7b4305360602b85db260ec3a7c0a22c19c1f3380bfc75599c0ea4eeeeda69177ab12f9da56ea1111111111111111111111111111111111111111111111111111111111111111000000000000000000000000000000000000000000000000000000000000000700000000000000000de0b6b3a7640000');
});

test('decimal parser is exact and rejects fractions beyond WAD precision', () => {
  assert.equal(parseAmount('1'), WAD);
  assert.equal(parseAmount('0.000000000000000001'), 1n);
  assert.throws(() => parseAmount('0'));
  assert.throws(() => parseAmount('1.0000000000000000001'));
});
