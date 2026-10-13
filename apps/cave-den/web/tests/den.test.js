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
  for (const bad of [{ blob: '../door/keys' }, { blob: 'A'.repeat(64) }, { key: b64(16) }, { iv: 'not base64!' }, { size: -1 }, { size: 26 * 1024 * 1024 }, { size: '9' }]) {
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
