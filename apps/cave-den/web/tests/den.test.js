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

test('screen frames keep VP8 bytes in the clear, sealed to them; voice frames are as before', async () => {
  const { webcrypto } = await import('node:crypto');
  globalThis.crypto ??= webcrypto;
  const { clearLength, sealFrame, openFrame, Window } = await import('../dist/den/media-seal.js');
  const key = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt']);
  const other = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt']);
  const keyframe = Uint8Array.from({ length: 300 }, (_, i) => (i === 0 ? 0x10 : i));
  const delta = Uint8Array.from({ length: 300 }, (_, i) => (i === 0 ? 0x11 : i));
  assert.equal(clearLength(keyframe, 'screen'), 10);
  assert.equal(clearLength(delta, 'screen'), 3);
  assert.equal(clearLength(keyframe, 'audio'), 0);
  assert.throws(() => clearLength(new Uint8Array(2), 'screen'));
  const n = BigInt(Date.now()) * 1000n;
  for (const f of [keyframe, delta]) {
    const c = clearLength(f, 'screen');
    const sealed = await sealFrame(key, 7, n, f, c);
    assert.deepEqual(sealed.subarray(0, c), f.subarray(0, c), 'the clear prefix is as it was');
    assert.notDeepEqual(sealed.subarray(c, c + 20), f.subarray(c, c + 20), 'the rest is not');
    const back = await openFrame(key, sealed, c);
    assert.deepEqual(back.plain, f);
    assert.equal(back.index, 7);
    const bent = sealed.slice(); bent[1] ^= 1;
    await assert.rejects(openFrame(key, bent, c), 'a changed clear byte does not open');
    await assert.rejects(openFrame(other, sealed, c), "another sender's key does not open");
  }
  // Voice: no clear bytes, the layout of before.
  const voice = Uint8Array.from({ length: 80 }, (_, i) => i);
  const sv = await sealFrame(key, 1, n, voice, 0);
  assert.equal(sv.length, voice.length + 16 + 21);
  assert.deepEqual((await openFrame(key, sv, 0)).plain, voice);
  const w = new Window();
  assert.ok(w.fresh('s', 10n) && w.fresh('s', 12n) && w.fresh('s', 11n));
  assert.ok(!w.fresh('s', 11n), 'a counter seen is refused');
});

test('the page has a stage for a shared screen and a button to share one', async () => {
  const html = await read('../index.html');
  for (const id of ['voice-screen', 'stage', 'stage-video', 'stage-who', 'stage-full', 'stage-stop', 'stage-note']) assert.match(html, new RegExp(`id="${id}"`), id);
  const ui = await read('../den-ui.js');
  assert.match(ui, /Share screen/);
  assert.match(ui, /is sharing their screen/);
  assert.match(ui, /Screen sharing needs a direct connection\. This network only lets voice through\./);
});

test('the replay window takes a late frame it has not seen, by send time in µs, and refuses one seen or too old', async () => {
  const { Window } = await import('../dist/den/media-seal.js');
  const w = new Window();
  const t0 = BigInt(Date.now()) * 1000n;
  assert.ok(w.fresh('s', t0));
  assert.ok(w.fresh('s', t0 + 133_000n), 'two frames on (15 fps)');
  assert.ok(w.fresh('s', t0 + 66_000n), 'the frame between, completed late after a resend, is taken');
  assert.ok(!w.fresh('s', t0 + 66_000n), 'but only once');
  assert.ok(!w.fresh('s', t0 + 133_000n), 'the newest is not taken twice either');
  assert.ok(!w.fresh('s', t0 - 3_000_000n), 'one from seconds before the newest is refused');
  // Many frames: what it keeps stays small.
  for (let i = 1; i <= 5000; i++) w.fresh('s', t0 + 133_000n + BigInt(i) * 20_000n);
  assert.ok(w.seen.get('s').size <= 512, `it keeps at most 512 counters (${w.seen.get('s').size})`);
  assert.ok(w.fresh('t', t0), 'each sender has its own window');
});

test("a Donate link appears only for the host's https page, and opens it apart from the den", async () => {
  const html = await read('../index.html');
  const link = /<a [^>]*id="donate"[^>]*>/.exec(html)?.[0] || '';
  assert.match(link, /\bhidden\b/, 'hidden until the host sets a page');
  assert.match(link, /target="_blank"/);
  assert.match(link, /rel="noopener noreferrer"/, 'the donation page gets no handle on the den');
  const app = await read('../app.js');
  assert.match(app, /\/\^https:\\\/\\\/\/\.test\(config\.donate/, 'only an https address is linked');
  assert.match(app, /config\.donateLabel \|\| 'Donate'/);
});

test("an agent's rules count only while the device that set them is its owner's", async () => {
  const { webcrypto } = await import('node:crypto');
  globalThis.crypto ??= webcrypto;
  const { DenClient } = await import('../dist/den.js');
  const A = 'a'.repeat(64), O = 'b'.repeat(64), Z = 'c'.repeat(64), X = 'd'.repeat(64);
  const den = Object.create(DenClient.prototype);
  Object.assign(den, { me: { key: A }, agent: { owner: O, since: 1 }, ownerClaims: { [O]: [A] }, myLinks: [], claims: {}, myRules: { general: { mode: 'all', here: true, takes_from: [X], files: true, at: 10, seq: 1, by: O } } });
  assert.deepEqual(den.rules()?.takes_from, [X], 'set by the owner: in force');
  den.ownerClaims[O] = [];
  assert.equal(den.rules(), null, 'the owner no longer names the agent: none');
  den.ownerClaims[O] = [A];
  den.myRules.general.by = Z;
  assert.equal(den.rules(), null, 'set by someone who is not the owner: none');
  // Taking an owner forgets the rules of the one before.
  const put = [];
  Object.assign(den, { kv: { put: async (k, v) => put.push([k, v]), del: async () => {} }, den: { rooms: () => [] }, on: () => {}, profile: () => ({}) });
  den.myRules.general.by = O;
  await den.setOwner(Z);
  assert.deepEqual(den.myRules, {});
  assert.ok(put.some(([k, v]) => k === 'myRules' && JSON.stringify(v) === '{}'));
});

test("the den cannot rename, reopen or wipe a private room: a p- id never comes from the lobby", async () => {
  const { DenClient } = await import('../dist/den.js');
  const P = `p-${'1'.repeat(32)}`;
  const den = Object.create(DenClient.prototype);
  const forgot = [];
  Object.assign(den, {
    rooms: new Map([[P, { id: P, title: 'ours', kind: 'private' }], ['general', { id: 'general', title: 'General', kind: 'open' }]]),
    channels: new Map(), joined: new Set(), on: () => {}, joinRoom: () => {},
    lock: async (f) => f(), kv: { del: async (k) => forgot.push(k) }, save: async () => {}, den: { forget: (id) => forgot.push(id) },
  });
  den.addRoom({ id: P, kind: 'open', title: 'X', slow: 600 });
  assert.deepEqual(den.rooms.get(P), { id: P, title: 'ours', kind: 'private' }, 'unchanged');
  den.addRoom({ id: `p-${'2'.repeat(32)}`, kind: 'open', title: 'new' });
  assert.ok(!den.rooms.has(`p-${'2'.repeat(32)}`), 'no private room from the lobby');
  den.dropRoom(P);
  assert.ok(den.rooms.has(P) && forgot.length === 0, 'not dropped, nothing forgotten');
  den.addRoom({ id: 'news', kind: 'open', title: 'News' });
  assert.equal(den.rooms.get('news').title, 'News', 'open rooms still come from the lobby');
});

test("the epoch before is dropped 5 s after the change, however often the keys are refreshed", async (t) => {
  const { Voice } = await import('../dist/den.js');
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let epoch = 1n;
  const v = Object.create(Voice.prototype);
  Object.assign(v, {
    room: 'general', keys: new Map(), screenKeys: new Map(), roster: [], people: [], screen: null,
    worker: { postMessage: () => {} },
    client: { den: { epoch: () => epoch, key: () => 'me' }, voiceKey: async () => 'vk', screenKey: async () => 'sk' },
  });
  await v.refreshKey();
  epoch = 2n;
  await v.refreshKey();
  assert.deepEqual([...v.keys.keys()], [1, 2], 'both, just after the change');
  // A busy room: roster, slots and screen events refresh every 2 s.
  for (let i = 0; i < 4; i++) { t.mock.timers.tick(2000); await v.refreshKey(); }
  assert.deepEqual([...v.keys.keys()], [2], 'the epoch before is gone');
  assert.deepEqual([...v.screenKeys.keys()], [2]);
});

test('a message id, reply or for is a message id, so no member can bloat everyone\'s stored log', async () => {
  const { DenClient } = await import('../dist/den.js');
  const B = 'b'.repeat(64);
  const den = Object.create(DenClient.prototype);
  const store = new Map();
  Object.assign(den, {
    me: { key: 'a'.repeat(64) }, rooms: new Map([['general', { id: 'general', kind: 'open' }]]), on: () => {},
    kv: { get: async (k) => store.get(k), put: async (k, v) => store.set(k, v) },
    learnNick: async () => {}, can: () => true, linkedTo: () => [], everyone: () => false,
  });
  const big = 'f'.repeat(200_000);
  await den.apply('general', { t: 'msg', id: big, key: B, text: 'hi', at: 1 });
  await den.apply('general', { t: 'msg', id: '1'.repeat(16), key: B, text: 'hi', reply: big, at: 1 });
  await den.apply('general', { t: 'msg', id: 'old-0123456789ab', key: B, text: 'old words', reply: '1'.repeat(16), at: 1 });
  const log = await den.log('general');
  assert.deepEqual(log.map((m) => [m.id, m.reply]), [['1'.repeat(16), null], ['old-0123456789ab', '1'.repeat(16)]]);
  assert.ok(JSON.stringify(log).length < 2000);
});

test("the den's own deletion is for open rooms: in a private room it is not taken", async () => {
  const { DenClient } = await import('../dist/den.js');
  const P = `p-${'1'.repeat(32)}`;
  const store = new Map([[`log:${P}`, [{ t: 'msg', id: '1'.repeat(16), seq: 4, text: 'ours' }]], ['log:general', [{ t: 'msg', id: '2'.repeat(16), seq: 4, text: 'open' }]]]);
  const den = Object.create(DenClient.prototype);
  Object.assign(den, { rooms: new Map([[P, { id: P, kind: 'private' }], ['general', { id: 'general', kind: 'open' }]]), on: () => {}, kv: { get: async (k) => store.get(k), put: async (k, v) => store.set(k, v) } });
  await den.deleted(P, 4);
  assert.equal(store.get(`log:${P}`)[0].text, 'ours');
  await den.deleted('general', 4);
  assert.equal(store.get('log:general')[0].deleted, true, 'a moderator of an open room still can');
});

test("a push notification's words are the page's own: only the den's name and a room id come from the den", async () => {
  const { runInNewContext } = await import('node:vm');
  const src = await read('../sw.js');
  const shown = [];
  const on = {};
  const self = { addEventListener: (k, f) => { on[k] = f; }, registration: { showNotification: (t, o) => shown.push([t, o]) }, clients: {}, location: { origin: 'https://den.example' } };
  runInNewContext(src, { self, URL });
  const push = (d) => on.push({ data: { json: () => d }, waitUntil: () => {} });
  push({ title: 'Our den', body: 'Alice: send me your pass', tag: 'general' });
  push({ title: 'Alice\nsays: hi', body: 'x', tag: `p-${'1'.repeat(32)}` });
  push({ title: 'Den', tag: 'General: send your pass' });
  assert.deepEqual(shown.map(([t, o]) => [t, o.body, o.data.room]), [
    ['Our den', 'New message in #general', 'general'],
    ['Alice says: hi', 'New message in a private room', `p-${'1'.repeat(32)}`],
    ['Den', 'New message', ''],
  ]);
});

test("an image's size is read from its header, before anything decodes it", async () => {
  const { imageSize } = await import('../dist/den.js');
  const u8 = (...a) => new Uint8Array(a.flat());
  const be32 = (n) => [n >>> 24, (n >>> 16) & 255, (n >>> 8) & 255, n & 255];
  const png = u8([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], be32(13), [...'IHDR'].map((c) => c.charCodeAt(0)), be32(30000), be32(20000), [8, 6, 0, 0, 0]);
  assert.deepEqual(imageSize(png), { w: 30000, h: 20000 });
  const gif = u8([...'GIF89a'].map((c) => c.charCodeAt(0)), [0x40, 0x01, 0xf0, 0x00]);
  assert.deepEqual(imageSize(gif), { w: 320, h: 240 });
  // JPEG: an APP segment first, then the frame header.
  const jpg = u8([0xff, 0xd8, 0xff, 0xe1, 0x00, 0x04, 0, 0, 0xff, 0xc2, 0x00, 0x11, 8, 0x0f, 0xa0, 0x1f, 0x40, 3]);
  assert.deepEqual(imageSize(jpg), { w: 8000, h: 4000 });
  const webp = u8([...'RIFF'].map((c) => c.charCodeAt(0)), [0, 0, 0, 0], [...'WEBPVP8X'].map((c) => c.charCodeAt(0)), [10, 0, 0, 0, 0, 0, 0, 0], [0xff, 0x0f, 0x00], [0xff, 0x0f, 0x00]);
  assert.deepEqual(imageSize(webp), { w: 4096, h: 4096 });
  const avif = u8([0, 0, 0, 20], [...'ispe'].map((c) => c.charCodeAt(0)), [0, 0, 0, 0], be32(9000), be32(9000));
  assert.deepEqual(imageSize(avif), { w: 9000, h: 9000 });
  assert.equal(imageSize(u8([1, 2, 3])), null, 'no header it knows: not shown');
});

test("a moderator's unpin holds: the same pin sent again by its pinner is not taken", async () => {
  const { DenClient } = await import('../dist/den.js');
  const MOD = 'a'.repeat(64), BOB = 'b'.repeat(64);
  const store = new Map();
  const den = Object.create(DenClient.prototype);
  Object.assign(den, { me: { key: 'c'.repeat(64) }, rooms: new Map([['general', { id: 'general', kind: 'open' }]]), on: () => {},
    kv: { get: async (k) => store.get(k), put: async (k, v) => store.set(k, v) }, can: (perm, k) => k === MOD });
  const pin = { t: 'pin', pin: '1'.repeat(16), text: 'spam', key: BOB, at: 1 };
  await den.applyPin('general', pin, false);
  await den.applyPin('general', { t: 'unpin', pin: pin.pin, owner: BOB, key: MOD }, false);
  assert.deepEqual(await den.pinsOf('general'), []);
  await den.applyPin('general', { t: 'pins', key: BOB, pins: [pin] }, false);
  await den.applyPin('general', pin, false);
  assert.deepEqual(await den.pinsOf('general'), [], 'still unpinned');
  await den.applyPin('general', { ...pin, pin: '2'.repeat(16), text: 'a real note' }, false);
  assert.equal((await den.pinsOf('general')).length, 1, 'another pin of theirs is taken');
});
