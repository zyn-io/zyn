// A load test of a running den, from one machine: TALKERS full clients (the
// page's own den.js and MLS) join General and write in it; LISTENERS are
// devices that enter and only receive. Measures what the den delivers: every
// message reaches every client in the room, how many a second, how late.
//
//   node tests/load.mjs https://den.example.org invites.txt [talkers] [listeners] [rates…]
//
// invites.txt: one invite code a line, one per client (epeaky invite N 1).
// The den must allow that many door requests from one address
// (DEN_DOOR_PER_MINUTE). Rates are messages a second across all talkers, each
// held for 20 seconds.
import { readFileSync } from 'node:fs';

// ---- An IndexedDB of the den's own shape (one store, kv), in memory. --------
const dbs = new Map();
globalThis.indexedDB = {
  open(name) {
    const r = {};
    setTimeout(() => {
      const fresh = !dbs.has(name);
      if (fresh) dbs.set(name, new Map());
      const data = dbs.get(name);
      const db = {
        createObjectStore() {},
        close() {},
        transaction() {
          const t = {};
          const store = {
            get: (k) => { const q = { result: data.get(k) }; return q; },
            put: (v, k) => { data.set(k, v); return {}; },
            delete: (k) => { data.delete(k); return {}; },
          };
          t.objectStore = () => store;
          setTimeout(() => t.oncomplete?.(), 0);
          return t;
        },
      };
      r.result = db;
      if (fresh) r.onupgradeneeded?.();
      r.onsuccess?.();
    }, 0);
    return r;
  },
  deleteDatabase(name) { dbs.delete(name); const r = {}; setTimeout(() => r.onsuccess?.(), 0); return r; },
};

const [base, invitesFile, talkersArg = '10', listenersArg = '50', ...rateArgs] = process.argv.slice(2);
if (!base || !invitesFile) { console.error('usage: node tests/load.mjs DEN invites.txt [talkers] [listeners] [rates…]'); process.exit(2); }
const invites = readFileSync(invitesFile, 'utf8').split(/\s+/).filter((x) => /^[0-9a-f]{64}$/.test(x));
const TALKERS = Number(talkersArg);
const LISTENERS = Number(listenersArg);
const RATES = rateArgs.length ? rateArgs.map(Number) : [5, 10, 20, 50];
if (invites.length < TALKERS + LISTENERS) { console.error(`need ${TALKERS + LISTENERS} invites, have ${invites.length}`); process.exit(2); }

const mls = await import('../dist/den/den_mls.js');
await mls.default({ module_or_path: readFileSync(new URL('../dist/den/den_mls_bg.wasm', import.meta.url)) });
const { DenClient } = await import('../dist/den.js');
const { Socket } = await import('../dist/den/phoenix.mjs');

const sentAt = new Map(); // the end of a message's ciphertext → when it went out
const lat = [];
let delivered = 0;
const errors = [];
const now = () => performance.now();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const keysUrl = `${base}/door/keys`;

async function enter(i) {
  const c = new DenClient({ base, keysUrl, db: `load-${i}`, prefix: 'load', on: () => {} });
  await c.start();
  const s = await c.join(invites[i]);
  return { c, s };
}

// Talkers: one by one into General (each joins by an external commit).
const talkers = [];
const t0 = now();
for (let i = 0; i < TALKERS; i++) {
  const { c, s } = await enter(i);
  c.on = (e, d) => { if (e === 'error') errors.push(d.error); };
  c.connect(s);
  for (let w = 0; w < 600 && !c.inRoom('general'); w++) await sleep(100);
  if (!c.inRoom('general')) { console.error(`talker ${i} did not get into General`); process.exit(1); }
  // When each of its messages leaves for the den.
  const push = c.push.bind(c);
  c.push = (ch, event, payload) => { if (event === 'relay') sentAt.set(payload.bytes.slice(-32), now()); return push(ch, event, payload); };
  talkers.push(c);
  process.stdout.write(`\rtalkers in: ${i + 1}/${TALKERS}`);
}
console.log(`  (${((now() - t0) / 1000).toFixed(1)} s)`);
// Wait until every talker is at the room's newest epoch.
for (let w = 0; w < 300; w++) {
  const e = talkers.map((c) => Number(c.den.epoch('general')));
  if (e.every((x) => x === e[0])) break;
  await sleep(100);
}

// Listeners: entered, on the room's channel, counting what arrives.
const listeners = [];
const t1 = now();
for (let i = 0; i < LISTENERS; i++) {
  const { s } = await enter(TALKERS + i);
  const socket = new Socket(`${base.replace(/^http/, 'ws')}/socket`, { authToken: s.session });
  socket.connect();
  const ch = socket.channel('room:general');
  ch.on('msg', (m) => { delivered++; const at = sentAt.get(m.bytes.slice(-32)); if (at) lat.push(now() - at); });
  await new Promise((resolve, reject) => ch.join().receive('ok', resolve).receive('error', (e) => reject(new Error(e?.reason))));
  listeners.push(socket);
  process.stdout.write(`\rlisteners in: ${i + 1}/${LISTENERS}`);
}
console.log(`  (${((now() - t1) / 1000).toFixed(1)} s)`);

// Talkers also count what reaches them (sealed for them, opened by MLS).
let opened = 0;
for (const c of talkers) c.on = (e, d) => { if (e === 'message' && !d.mine && d.kind === 'msg') opened++; if (e === 'error') errors.push(d.error); };

const pct = (xs, p) => { if (!xs.length) return null; const s = [...xs].sort((a, b) => a - b); return Math.round(s[Math.min(s.length - 1, Math.floor(p * s.length))]); };
const rows = [];
for (const rate of RATES) {
  lat.length = 0;
  delivered = 0;
  opened = 0;
  let sent = 0;
  let refused = 0;
  const end = now() + 20_000;
  const each = 1000 / rate;
  let k = 0;
  while (now() < end) {
    const c = talkers[k++ % talkers.length];
    const at = now();
    c.say('general', `load ${rate}/s #${k} ${'x'.repeat(80)}`).then(() => { sent++; }, () => { refused++; });
    await sleep(Math.max(0, each - (now() - at)));
  }
  await sleep(3000);
  const expected = sent * LISTENERS;
  rows.push({ rate, sent, refused, to_listeners: delivered, expected, per_s: Math.round(delivered / 20), p50_ms: pct(lat, 0.5), p95_ms: pct(lat, 0.95), p99_ms: pct(lat, 0.99), talkers_opened: opened, talkers_expected: sent * (TALKERS - 1) });
  console.log(JSON.stringify(rows.at(-1)));
}
console.table(rows);
if (errors.length) console.log('errors:', [...new Set(errors)].slice(0, 5));
for (const s of listeners) s.disconnect();
for (const c of talkers) c.socket.disconnect();
process.exit(0);
