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
  assert.match(page, /sees which device names are in which room, when they write or talk, file sizes and the names of open rooms, not whose they are/);
  assert.match(page, /first day, entering right after the pass check can be timed together/);
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
