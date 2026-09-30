import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const app = await readFile(new URL('../app.js', import.meta.url), 'utf8');
const html = await readFile(new URL('../index.html', import.meta.url), 'utf8');
const build = await readFile(new URL('../scripts/build.mjs', import.meta.url), 'utf8');

test('reads only the zynscan API, over the Zyn gateway, with no credentials', () => {
  assert.match(app, /mainnet: 'https:\/\/zyn-rpc\.cavebro\.win\/scan'/);
  assert.match(app, /testnet: 'https:\/\/zyn-rpc\.cavebro\.win\/scan-testnet'/);
  assert.equal((app.match(/fetch\(/g) || []).length, 1);
  assert.match(app, /credentials: 'omit'/);
  assert.match(app, /LOCAL && params\.get\('api'\)/, 'an API override is for localhost only');
  assert.match(build, /connect-src https:\/\/zyn-rpc\.cavebro\.win;/);
  assert.match(build, /script-src 'self';/);
});

test('says what it does not show, and never asks for an account', () => {
  assert.match(html, /never an account, an amount or an owner/);
  assert.doesNotMatch(app, /\/api\/account|trader|owner:/);
});

test('every value from the API is escaped or formatted before it reaches the page', () => {
  // The only unescaped interpolations allowed are the helpers that escape.
  assert.match(app, /const esc = /);
  assert.match(app, /const hash = \(h\) => `<span class="hash" title="\$\{esc\(h\)\}">/);
});
