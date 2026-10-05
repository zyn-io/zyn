import { test } from 'node:test';
import assert from 'node:assert/strict';
import { identify } from '../public/address.js';

const kind = async (s) => (await identify(s)).kind;

test('transparent addresses, both networks', async () => {
  assert.deepEqual(
    [await kind('t1PKtYdJJHhc3Pxowmznkg7vdTwnhEsCvR4'), (await identify('t1PKtYdJJHhc3Pxowmznkg7vdTwnhEsCvR4')).net],
    ['transparent', 'mainnet'],
  );
  const tm = await identify('tm9iMLAuYMzJ6jtFLcA7rzUmfreGuKvr7Ma');
  assert.equal(tm.kind, 'transparent');
  assert.equal(tm.net, 'testnet');
});

test('a transparent address with a typo fails its checksum', async () => {
  const r = await identify('t1PKtYdJJHhc3Pxowmznkg7vdTwnhEsCvR5');
  assert.equal(r.kind, null);
  assert.match(r.reason, /checksum/);
  assert.equal(await kind('t1nkjqphuc4nxdj3aaaaaaaaaaaaaaaaaaa'), null);
});

test('unified addresses, both networks', async () => {
  const u = await identify('u1kc38pww6c8ahz0z20g9kv9s7afrc2npmr8s6nngw00qufveq43q5m2vh8u7wxpaltl0vjs7etr7y7d0pcm5dtc24pqhu5qfe5cwtyc2u');
  assert.equal(u.kind, 'unified');
  assert.equal(u.net, 'mainnet');
  assert.equal(await kind('u1l8xunezsvhq8fgzfl7404m450nwnd76zshscn6nfys7vyz2ywyh4cc5daaq0c7q2su5lqfh23sp7fkf3kt27ve5948mzpfdvckzaect2jtte308mkwlycj2u0eac077wu70vqcetkxf'), 'unified');
  const t = await identify('utest1h6sfz7alnztp0vst6u0s8sxj9zhzrv5s7qjy5897qcdse75epp43x4uus90naxaea973q22nshuggyywujdj5zulckj2vppz85x2t407');
  assert.equal(t.kind, 'unified');
  assert.equal(t.net, 'testnet');
});

test('a unified address with one changed character is a typo', async () => {
  const r = await identify('u1kc38pww6c8ahz0z20g9kv9s7afrc2npmr8s6nngw00qufveq43q5m2vh8u7wxpaltl0vjs7etr7y7d0pcm5dtc24pqhu5qfe5cwtyc2v');
  assert.equal(r.kind, null);
  assert.match(r.reason, /checksum/);
});

test('upper-case bech32 is accepted, mixed case is not', async () => {
  const up = 'utest1h6sfz7alnztp0vst6u0s8sxj9zhzrv5s7qjy5897qcdse75epp43x4uus90naxaea973q22nshuggyywujdj5zulckj2vppz85x2t407'.toUpperCase();
  assert.equal(await kind(up), 'unified');
  assert.equal(await kind('Utest1h6sfz7alnztp0vst6u0s8sxj9zhzrv5s7qjy5897qcdse75epp43x4uus90naxaea973q22nshuggyywujdj5zulckj2vppz85x2t407'), null);
});

test('not an address', async () => {
  assert.equal(await kind(''), null);
  assert.equal(await kind('hello'), null);
  assert.equal(await kind('0x52908400098527886E0F7030069857D2E4169EE7'), null);
  assert.match((await identify('t1Kk TCS5')).reason, /spaces/);
});

// A minimal Bech32 encoder, to build well-formed Sapling and viewing-key strings.
const B32 = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
function polymod(v) { const G = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]; let c = 1; for (const x of v) { const t = c >>> 25; c = ((c & 0x1ffffff) << 5) ^ x; for (let i = 0; i < 5; i++) if ((t >>> i) & 1) c ^= G[i]; } return c >>> 0; }
function encode(hrp, bytes, m) {
  const d = []; let acc = 0, bits = 0;
  for (const b of bytes) { acc = (acc << 8) | b; bits += 8; while (bits >= 5) { bits -= 5; d.push((acc >> bits) & 31); } }
  if (bits) d.push((acc << (5 - bits)) & 31);
  const ex = [...hrp].map((c) => c.charCodeAt(0) >> 5).concat(0, [...hrp].map((c) => c.charCodeAt(0) & 31));
  const pm = polymod(ex.concat(d, [0, 0, 0, 0, 0, 0])) ^ (m ? 0x2bc830a3 : 1);
  const cs = []; for (let i = 0; i < 6; i++) cs.push((pm >> (5 * (5 - i))) & 31);
  return hrp + '1' + d.concat(cs).map((x) => B32[x]).join('');
}

test('testnet Sapling addresses are read as Sapling, not Sprout', async () => {
  const a = encode('ztestsapling', new Uint8Array(43).fill(7), false);
  assert.equal(a.length, 88);
  const r = await identify(a);
  assert.equal(r.kind, 'sapling');
  assert.equal(r.net, 'testnet');
  assert.equal((await identify(encode('zs', new Uint8Array(43).fill(9), false))).kind, 'sapling');
});

test('incoming viewing keys are recognised', async () => {
  assert.equal((await identify(encode('uivk', new Uint8Array(80).fill(3), true))).kind, 'viewing');
});
