// REGI holders onto the whitelist (DECISIONS §181): the signature check finds
// the wallet that signed, rejects another's, and a wallet's code is its own,
// stable, in the invitation's format.
import test from 'node:test';
import assert from 'node:assert/strict';
import { secp256k1 } from '@noble/curves/secp256k1';
import { keccak_256 } from '@noble/hashes/sha3';
import { codeFrom, messageFor, recover, TITLE } from '../functions/api/whitelist/regi.js';

const hex = (b) => [...b].map((x) => x.toString(16).padStart(2, '0')).join('');
const addressOf = (priv) => `0x${hex(keccak_256(secp256k1.getPublicKey(priv, false).slice(1)).slice(-20))}`;
/// What a wallet's personal_sign returns: r ‖ s ‖ v (27 or 28).
function personalSign(priv, message) {
  const bytes = new TextEncoder().encode(message);
  const digest = keccak_256(new Uint8Array([...new TextEncoder().encode(`\x19Ethereum Signed Message:\n${bytes.length}`), ...bytes]));
  const sig = secp256k1.sign(digest, priv);
  return `0x${hex(sig.toCompactRawBytes())}${(27 + sig.recovery).toString(16)}`;
}

test('the signer is recovered from a wallet signature', () => {
  const priv = secp256k1.utils.randomPrivateKey();
  const wallet = addressOf(priv);
  const msg = messageFor(wallet, new Date().toISOString());
  assert.ok(msg.startsWith(TITLE));
  assert.equal(recover(msg, personalSign(priv, msg)), wallet);
});

test('a signature over another message or by another wallet does not pass', () => {
  const a = secp256k1.utils.randomPrivateKey();
  const b = secp256k1.utils.randomPrivateKey();
  const date = new Date().toISOString();
  const forA = messageFor(addressOf(a), date);
  assert.notEqual(recover(forA, personalSign(b, forA)), addressOf(a), "B's signature is not A's");
  assert.notEqual(recover(messageFor(addressOf(a), '2020-01-01T00:00:00.000Z'), personalSign(a, forA)), addressOf(a), 'another date is another message');
});

test("a wallet's code is stable and shaped like an invitation's", () => {
  const mac = 'ab'.repeat(32);
  assert.equal(codeFrom(mac), codeFrom(mac));
  assert.match(codeFrom(mac), /^[A-HJ-NP-Z2-9]{4}-[A-HJ-NP-Z2-9]{4}-[A-HJ-NP-Z2-9]{4}$/);
  assert.notEqual(codeFrom('cd'.repeat(32)), codeFrom(mac));
});
