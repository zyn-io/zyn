import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

// The den's page (web/, built by build.sh into dist/ for these tests).
const read = (p) => readFile(new URL(p, import.meta.url), 'utf8');

test('the page says what the server can see, inline, before anyone enters', async () => {
  for (const page of ['../index.html']) {
    const html = await read(page);
    assert.match(html, /end-to-end encrypted \(MLS\)/, page);
    assert.match(html, /sees which device names are in which room, when they write or talk, file sizes, the names of open rooms, and who joins or leaves which room/, page);
    // The pass and the device can be linked by the server that runs both: said, not hidden.
    assert.match(html, /could link your pass to this device by your network address and timing; a VPN or Tor keeps them apart/, page);
    assert.match(html, /<script type="module" src="\.\/app\.js"><\/script>/, page);
    assert.doesNotMatch(html, /<script(?![^>]*src=)[^>]*>/, `${page}: no inline script`);
    assert.doesNotMatch(html, /style=/, `${page}: no style attributes`);
  }
});

test('the core page has nothing of the Cave or Zyn in it', async () => {
  for (const f of ['../index.html', '../app.js', '../gate.js', '../base.css', '../den.css', '../config.json']) {
    assert.doesNotMatch(await read(f), /cavebro|Cave|Zyn|Nap\b|Collection Z|Diggers|Lookouts|Lanterns/, f);
  }
});

test("the client takes day keys from the host's one file, and keeps tomorrow's token", async () => {
  const den = await read('../den.js');
  const app = await read('../app.js');
  assert.match(app, /keysUrl: config\.keysUrl \|\| '\/door\/keys'/, "by default the den's own published keys");
  assert.match(den, /fetch\(this\.keysUrl, \{ credentials: 'omit' \}\)/);
  // The door gets the pass code and a blinded token; the den gets the token, later.
  assert.match(den, /this\.post\('\/door\/sign', \{ code, day: d, blinded: JSON\.parse\(pending\)\.blinded \}\)/);
  assert.match(den, /kv\.put\(`token:\$\{day\(1\)\}`, tomorrow\)/, "tomorrow's token is kept for tomorrow");
});

test("the gate takes a pass code only from the issuer's own window and origin", async () => {
  const gate = await read('../gate.js');
  assert.match(gate, /e\.origin !== issuer\.origin \|\| e\.source !== win \|\| e\.data\?\.den !== 'pass'/);
  assert.match(gate, /const PASS = \/\^\[0-9a-f\]\{64\}\$\//);
});

test('a file message from another device is checked before it is shown or fetched', async () => {
  const { fileMeta } = await import('../dist/den.js');
  const b64 = (n) => Buffer.alloc(n, 7).toString('base64');
  const good = { blob: 'a'.repeat(64), name: 'cave.png', mime: 'image/png', size: 1000, key: b64(32), iv: b64(12) };
  assert.deepEqual(fileMeta(good), good);
  for (const bad of [{ blob: '../door/keys' }, { blob: 'A'.repeat(64) }, { key: b64(16) }, { iv: 'not base64!' }, { size: -1 }, { size: 2 ** 31 + 1 }, { size: '9' }, { chunk: 4096 }]) {
    assert.equal(fileMeta({ ...good, ...bad }), null, JSON.stringify(bad));
  }
  assert.equal(fileMeta({ ...good, mime: 'image/svg+xml' }).mime, 'application/octet-stream', 'SVG is a download, never shown in place');
  assert.equal(fileMeta({ ...good, mime: 'text/html' }).mime, 'application/octet-stream');
  assert.equal(fileMeta({ ...good, name: '../../etc/pass\u0000wd' }).name, '....etcpasswd');
  assert.equal(fileMeta({ ...good, name: 7 }).name, 'file');
});

test("a device's pass hash is the one the den's member list keeps", async () => {
  const { webcrypto } = await import('node:crypto');
  globalThis.crypto ??= webcrypto;
  const { DenClient } = await import('../dist/den.js');
  const realRandom = crypto.getRandomValues.bind(crypto);
  crypto.getRandomValues = (a) => a.fill(0x11);
  try {
    const { code, pass } = await DenClient.newPass();
    assert.equal(code, '11'.repeat(32));
    // Den.Gate.Members.hash/1 of 32 × 0x11 (server/test/den/members_test.exs checks the same vector).
    assert.equal(pass, '89fbbb8554656bd7bdd0f200b552b67d6cc3c8faabd86261531e1d49ce822833');
  } finally { crypto.getRandomValues = realRandom; }
});

test('a name a member gives themselves is one plain line that cannot pass for a device name', async () => {
  const { nickOf } = await import('../dist/den.js');
  assert.equal(nickOf('  Ada  Lovelace '), 'Ada Lovelace');
  assert.equal(nickOf('Ada‮ecalevol'), 'Adaecalevol', 'no bidirectional controls');
  assert.equal(nickOf('a\nb'), 'a b');
  assert.equal(nickOf('x'.repeat(40)).length, 32);
  assert.equal(nickOf('m-e0458d253595addd653e'), '', 'not a device name');
  assert.equal(nickOf('M-0abc'), '');
  assert.equal(nickOf('m-zed'), 'm-zed', 'only m- and hex is a device name');
  assert.equal(nickOf(''), '');
  assert.equal(nickOf(7), '7');
});

test('a file is sealed in pieces that open only whole and in order', async () => {
  const { webcrypto } = await import('node:crypto');
  globalThis.crypto ??= webcrypto;
  const { DenClient, CHUNK, sealedSize } = await import('../dist/den.js');
  const den = Object.create(DenClient.prototype);
  den.base = 'https://den.test';
  den.sessionToken = { session: 's' };
  den.waysSeen = Promise.resolve({ file_max: 100e6 });
  let sealed = null;
  let sentMsg = null;
  den.upload = async (blob) => { sealed = new Uint8Array(await blob.arrayBuffer()); return { id: 'a'.repeat(64) }; };
  den.send = async (_room, body) => { sentMsg = body; return 'id'; };
  const realFetch = globalThis.fetch;
  let served = null;
  globalThis.fetch = async () => new Response(served);
  try {
    for (const size of [0, 1, CHUNK, CHUNK + 1, 2.5 * CHUNK]) {
      const data = new Uint8Array(size);
      for (let o = 0; o < size; o += 65536) webcrypto.getRandomValues(data.subarray(o, o + 65536));
      await den.sendFile('general', new File([data], 'f.bin'));
      assert.equal(sealed.length, sealedSize(size), `sealed size for ${size}`);
      assert.equal(sentMsg.file.chunk, CHUNK);
      served = sealed;
      const back = new Uint8Array(await (await den.openFile(sentMsg.file)).arrayBuffer());
      assert.deepEqual(back, data, `round trip of ${size} bytes`);
    }
    // 2.5 pieces: the last one dropped, or two swapped, does not open.
    const whole = CHUNK + 16;
    served = sealed.slice(0, 2 * whole);
    await assert.rejects(den.openFile(sentMsg.file), /not whole|operation/i, 'cut short');
    served = new Uint8Array([...sealed.slice(whole, 2 * whole), ...sealed.slice(0, whole), ...sealed.slice(2 * whole)]);
    await assert.rejects(den.openFile(sentMsg.file), 'reordered');
    served = sealed;
    await assert.rejects(den.openFile({ ...sentMsg.file, size: sentMsg.file.size - 1 }), 'a size that does not match');
    den.waysSeen = Promise.resolve({ file_max: 1000 });
    await assert.rejects(den.sendFile('general', new File([new Uint8Array(2000)], 'big.bin')), /at most/);
  } finally { globalThis.fetch = realFetch; }
});

test('two devices are one person only when each says so of the other', async () => {
  const { DenClient } = await import('../dist/den.js');
  const den = Object.create(DenClient.prototype);
  const [a, b, c] = ['a', 'b', 'c'].map((x) => x.repeat(64));
  den.me = { key: a, nick: '' };
  den.myLinks = [b];
  den.claims = { [b]: [a], [c]: [a] };
  den.nicks = { [b]: { name: 'Ada', at: 1 } };
  assert.deepEqual(den.linkedTo(a), [b]);
  assert.deepEqual(den.linkedTo(b), [a]);
  assert.deepEqual(den.linkedTo(c), [], 'c says it is a, a never said so');
  assert.equal(den.isMe(b), true);
  assert.equal(den.isMe(c), false);
  assert.equal(den.nickFor(a), 'Ada', "a device takes its linked device's name");
  assert.equal(den.nickFor(c), '');
});

test('permissions and the order of roles, as the den works them out', async () => {
  const { DenClient, PERMS } = await import('../dist/den.js');
  const den = Object.create(DenClient.prototype);
  const [owner, mod, helper, member] = ['a', 'b', 'c', 'd'].map((x) => x.repeat(64));
  den.me = { key: mod, nick: '' };
  den.rooms = new Map([['general', { id: 'general', kind: 'open', slow: 30, post_roles: [] }], ['news', { id: 'news', kind: 'open', post_roles: ['r-team'] }]]);
  den.said = {};
  den.mod = {
    owner,
    roles: [
      { id: 'r-mod', name: 'Moderator', colour: '#e0a030', position: 10, perms: ['timeout', 'kick'] },
      { id: 'r-team', name: 'Team', colour: null, position: 5, perms: ['bypass_slow'] },
      { id: 'everyone', name: '@everyone', colour: null, position: 0, perms: ['mention_everyone'] },
    ],
    holders: { [mod]: ['r-mod'], [helper]: ['r-team'] },
    sanctions: [{ key: member, kind: 'timeout', until: Math.floor(Date.now() / 1000) + 60 }, { key: helper, kind: 'timeout', until: 1 }],
  };
  assert.deepEqual([...den.perms(owner)].sort(), [...PERMS].sort());
  assert.ok(den.can('kick') && !den.can('ban'));
  assert.ok(den.can('mention_everyone', member), "@everyone's permissions are everyone's");
  assert.ok(den.above(helper) && den.above(member) && !den.above(owner));
  assert.equal(den.colourOf(mod), '#e0a030');
  assert.equal(den.colourOf(helper), null, 'a role without a colour gives none');
  assert.equal(den.sanctionOf(member)?.kind, 'timeout');
  assert.equal(den.sanctionOf(helper), null, 'a timeout that ended');
  assert.ok(den.mayPost('general') && !den.mayPost('news'));
  assert.equal(den.slowLeft('general'), 0);
  den.said.general = Date.now();
  assert.ok(den.slowLeft('general') > 29_000);
  den.me.key = helper;
  assert.equal(den.slowLeft('general'), 0, 'Bypass slow mode');
  assert.ok(den.mayPost('news'));
});

test('a second message from one device inside the slow mode window is not shown', async () => {
  const { tooSoon } = await import('../dist/den.js');
  const k = 'a'.repeat(64);
  const log = [{ t: 'msg', key: k, srv: 1000 }, { t: 'react', key: k }, { t: 'msg', key: 'b'.repeat(64), srv: 1020 }];
  assert.equal(tooSoon(log, { t: 'msg', key: k, srv: 1010 }, 30), true);
  assert.equal(tooSoon(log, { t: 'msg', key: k, srv: 1030 }, 30), false);
  assert.equal(tooSoon(log, { t: 'file', key: k, srv: 1010 }, 0), false, 'no slow mode');
  assert.equal(tooSoon([], { t: 'msg', key: k, srv: 1010 }, 30), false);
  assert.equal(tooSoon(log, { t: 'msg', key: k }, 30), false, 'from a den that gives no time');
});
