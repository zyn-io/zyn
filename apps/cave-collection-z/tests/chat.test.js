import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

// The members' chat (§183): /chat alone talks to the chat server and runs the
// OMEMO library's wasm; the rest of the site keeps the strict policy.
const headers = await readFile(new URL('../dist/_headers', import.meta.url), 'utf8');
const rule = (path) => headers.split('\n\n').find((b) => b.split('\n')[0] === path) || '';

test('only /chat reaches the chat server, and it detaches the site-wide policy first', () => {
  for (const path of ['/chat', '/chat/*']) {
    const r = rule(path);
    assert.match(r, /^  ! Content-Security-Policy$/m, `${path} detaches the site-wide policy`);
    assert.match(r, /connect-src 'self' https:\/\/chat\.cavebro\.win wss:\/\/chat\.cavebro\.win/);
    assert.match(r, /script-src 'self' 'wasm-unsafe-eval'/);
  }
  assert.doesNotMatch(rule('/*'), /chat\.cavebro\.win|wasm-unsafe-eval/, 'no other page talks to the chat or runs wasm');
});

test('the chat page says what the team can see, inline, before anyone enters', async () => {
  const page = await readFile(new URL('../chat.html', import.meta.url), 'utf8');
  assert.match(page, /end-to-end encrypted \(MLS\)/);
  assert.match(page, /sees which device names are in which room, when they write or talk, file sizes, the names of open rooms, and who joins or leaves which room/);
  // The pass and the device can be linked by the server that runs both: said, not hidden (§184.13).
  assert.match(page, /could link your pass to this device by your network address and timing; a VPN or Tor keeps them apart/);
  assert.match(page, /ends your entry at the end of the day/);
  assert.match(page, /<title>Members' chat<\/title>/);
});

test("the den's client is served from this site: its wasm, the pinned socket client, the day keys from the static file", async () => {
  const pkg = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(pkg.dependencies.phoenix, '1.8.15', 'the den runs Phoenix 1.8.15');
  assert.equal(pkg.dependencies['converse.js'], undefined);
  for (const f of ['den.js', 'den/den_mls.js', 'den/den_mls_bg.wasm', 'den/phoenix.mjs']) await readFile(new URL(`../dist/${f}`, import.meta.url));
  const den = await readFile(new URL('../den.js', import.meta.url), 'utf8');
  // Day keys come from the site's own static file, never from the den.
  assert.match(den, /keysUrl: '\/den-keys\.json'|this\.keysUrl/);
  const zstore = await readFile(new URL('../zstore.js', import.meta.url), 'utf8');
  assert.match(zstore, /keysUrl: '\/den-keys\.json'/);
  // The door gets the pass code and a blinded token; the den gets the token, later.
  assert.match(den, /this\.post\('\/door\/sign', \{ code, day: d, blinded: JSON\.parse\(pending\)\.blinded \}\)/);
  assert.match(den, /kv\.put\(`token:\$\{d\}`, token\)/, "tomorrow's token is kept for tomorrow");
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
