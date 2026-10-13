import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

// The Cave's add-on on top of the den's page: what it says, where it gets
// its keys and its members' passes.
const read = (p) => readFile(new URL(p, import.meta.url), 'utf8');

test("the Cave's page says what the team can see, and runs no wallet code", async () => {
  const html = await read('../index.html');
  assert.match(html, /end-to-end encrypted \(MLS\)/);
  assert.match(html, /could link your pass to this device by your network address and timing; a VPN or Tor keeps them apart/);
  assert.match(html, /ends your entry at the end of the day/);
  assert.match(html, /<title>Members' chat<\/title>/);
  assert.match(html, /<script type="module" src="\.\/app\.js"><\/script>/);
  assert.doesNotMatch(html, /zstore\.js|nap-connect|data-wallet-button|<script(?![^>]*src=)[^>]*>|style=/);
});

test('day keys from cavebro.win; the pass from the store window, handed to this origin alone', async () => {
  const config = JSON.parse(await read('../config.json'));
  assert.equal(config.keysUrl, 'https://cavebro.win/den-keys.json');
  assert.equal(config.issuer, 'https://cavebro.win/pass');
  const store = await read('../../../cave-collection-z/zstore.js');
  assert.match(store, /window\.opener\?\.postMessage\(\{ den: 'pass', code: r\.token \}, new URL\(CHAT\)\.origin\)/);
});
