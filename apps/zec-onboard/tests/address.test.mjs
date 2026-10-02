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
