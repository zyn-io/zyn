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
  assert.match(page, /end-to-end encrypted/);
  assert.match(page, /can link your chat name to your pass/);
  assert.match(page, /operator can link a pass to the item/);
  assert.match(page, /takes you out within a few minutes/);
  assert.match(page, /<title>Members' chat<\/title>/);
});

test('the chat client is the pinned Converse.js, served from this site', async () => {
  const pkg = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(pkg.dependencies['converse.js'], '14.0.0');
  for (const f of ['converse.min.js', 'converse.min.css', 'libomemo.esm.min.js', 'curve25519_compiled.wasm', 'LICENSE']) {
    await readFile(new URL(`../dist/converse/${f}`, import.meta.url));
  }
  const zstore = await readFile(new URL('../zstore.js', import.meta.url), 'utf8');
  assert.match(zstore, /omemo_default: true/, 'encryption is on unless a member turns it off — and the server refuses plaintext');
  assert.match(zstore, /allow_registration: false/);
});
