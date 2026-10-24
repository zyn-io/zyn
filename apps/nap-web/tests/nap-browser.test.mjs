// The phone app's browser (nap-browser.js), driven as the platform drives it:
// requests arrive as plugin events, sheets are answered, responses go back.
// What is checked here is the connection record: it is the same one the
// connect window keeps (connect-grants.js), with the account in it.
import { test } from 'node:test';
import assert from 'node:assert/strict';

const store = new Map();
globalThis.localStorage = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) };

let listener = null;
const sheets = []; // titles of the sheets shown
let answered = null; // resolves with the next response
globalThis.window = {
  __TAURI__: {
    core: {
      addPluginListener: async (_plugin, _event, cb) => { listener = cb; },
      invoke: async (cmd, args) => {
        if (cmd === 'plugin:napbrowser|approve') { sheets.push(args.title); return { approved: true }; }
        if (cmd === 'plugin:napbrowser|respond') { answered(JSON.parse(args.json)); return null; }
        throw new Error(`unexpected ${cmd}`);
      },
    },
  },
};

const { startBrowser } = await import('../src/nap-browser.js');
const { paySchedulesOf, withPaySchedule } = await import('../src/connect-grants.js');
const { deriveIdentity } = await import('../src/core.js');

const KEY = 'nap.connected.v1';
const SITE = 'https://shop.example';
const A = deriveIdentity('abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art');
const B = deriveIdentity('zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote');
const net = (id) => ({ id, network: id, zcash: id, chain: id === 'testnet' ? 212 : 26460 });
let open = { config: net('testnet'), identity: A, zec: null };
await startBrowser(() => open);

let next = 0;
const ask = (method, params = {}) => new Promise((resolve) => {
  answered = resolve;
  listener({ id: ++next, origin: SITE, method, params: JSON.stringify(params) });
});
const record = () => JSON.parse(store.get(KEY) || '{}');
const ID = 'ab'.repeat(32);

test('reconnecting by the same wallet keeps the pay list; another wallet does not inherit it', async () => {
  store.clear();
  // Wallet A connects on testnet; a schedule is kept for the site (as paySite.add does).
  assert.equal((await ask('nap_connect')).result.account, A.accountHex);
  assert.deepEqual(sheets.splice(0), [`Connect ${new URL(SITE).host} to Nap`]);
  assert.equal(record()[SITE].account, A.accountHex, 'the record names the wallet');
  store.set(KEY, JSON.stringify(withPaySchedule(record(), SITE, 'testnet', A.accountHex, ID)));

  // Already connected: no sheet, the list stays.
  assert.equal((await ask('nap_session')).result.account, A.accountHex);
  await ask('nap_connect');
  assert.deepEqual(sheets.splice(0), []);
  assert.deepEqual(paySchedulesOf(record(), SITE, 'testnet', A.accountHex), [ID]);

  // To mainnet and back: each connect asks again, and testnet's list survives.
  open = { ...open, config: net('mainnet') };
  await ask('nap_connect');
  open = { ...open, config: net('testnet') };
  await ask('nap_connect');
  assert.equal(sheets.splice(0).length, 2);
  assert.deepEqual(paySchedulesOf(record(), SITE, 'testnet', A.accountHex), [ID]);

  // Wallet B, same site and network: not connected until it says so, and it
  // starts with nothing of A's.
  open = { ...open, identity: B };
  assert.equal((await ask('nap_session')).result, null, "another wallet is not connected by A's record");
  assert.equal((await ask('nap_connect')).result.account, B.accountHex);
  assert.equal(sheets.splice(0).length, 1, 'B is asked to connect');
  assert.deepEqual(paySchedulesOf(record(), SITE, 'testnet', B.accountHex), []);
  assert.equal(record()[SITE].pay, undefined, "A's list is not B's");
});
