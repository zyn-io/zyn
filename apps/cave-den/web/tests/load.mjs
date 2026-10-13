// A load test of a running den, from one machine: TALKERS full clients (the
// page's own den.js and MLS) join General and write in it; LISTENERS are
// devices that enter and only receive. Measures what the den delivers: every
// message reaches every client in the room, how many a second, how late.
//
//   node tests/load.mjs https://den.example.org invites.txt [talkers] [listeners] [rates…]
//
// Listeners run in LOAD_WORKERS processes (4), so this machine's own work
// (opening every message for every talker) is not what is measured.
// invites.txt: one invite code a line, one per client (epeaky invite N 1).
// The den must allow that many door requests from one address
// (DEN_DOOR_PER_MINUTE). Rates are messages a second across all talkers, each
// held for 20 seconds.
import { readFileSync } from 'node:fs';
import { fork } from 'node:child_process';

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
// One clock for every process on this machine.
const now = () => performance.timeOrigin + performance.now();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const keysUrl = `${base}/door/keys`;

async function enter(i) {
  const c = new DenClient({ base, keysUrl, db: `load-${i}`, prefix: 'load', on: () => {} });
  await c.start();
  const s = await c.join(invites[i]);
  return { c, s };
}

// ---- A listener process: its share of the listeners, reporting on asking. ----
if (process.env.DEN_LOAD_ROLE === 'listen') {
  const [from, to] = process.env.DEN_LOAD_RANGE.split('-').map(Number);
  const sockets = [];
  for (let i = from; i < to; i++) {
    const { s } = await enter(i);
    const socket = new Socket(`${base.replace(/^http/, 'ws')}/socket`, { authToken: s.session });
    socket.connect();
    const ch = socket.channel('room:general');
    ch.on('msg', (m) => { delivered++; const at = sentAt.get(m.bytes.slice(-32)); if (at) lat.push(now() - at); });
    await new Promise((resolve, reject) => ch.join().receive('ok', resolve).receive('error', (e) => reject(new Error(e?.reason))));
    sockets.push(socket);
  }
  process.on('message', (m) => {
    if (m.sent) sentAt.set(m.sent, m.at);
    if (m.report) { process.send({ delivered, lat }); delivered = 0; lat.length = 0; sentAt.clear(); }
    if (m.stop) { sockets.forEach((x) => x.disconnect()); process.exit(0); }
  });
  process.send({ ready: to - from });
  await new Promise(() => {});
}

// Talkers: one by one into General (each joins by an external commit).
const talkers = [];
const workers = [];
const t0 = now();
for (let i = 0; i < TALKERS; i++) {
  const { c, s } = await enter(i);
  c.on = (e, d) => { if (e === 'error') errors.push(d.error); };
  // No key refreshes during the test: the room's epoch stays put.
  c.soonRefresh = () => {};
  c.connect(s);
  for (let w = 0; w < 600 && !c.inRoom('general'); w++) await sleep(100);
  if (!c.inRoom('general')) { console.error(`talker ${i} did not get into General`); process.exit(1); }
  // When each of its messages leaves for the den.
  const push = c.push.bind(c);
  c.push = (ch, event, payload) => {
    if (event === 'relay') { const sent = payload.bytes.slice(-32); const at = now(); sentAt.set(sent, at); for (const w of workers) w.send({ sent, at }); }
    return push(ch, event, payload);
  };
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

// Listeners: entered, on the room's channel, counting what arrives, in
// their own processes.
const t1 = now();
const W = Math.max(1, Math.min(Number(process.env.LOAD_WORKERS || 4), LISTENERS || 1));
const share = Math.ceil(LISTENERS / W);
await Promise.all(Array.from({ length: LISTENERS ? W : 0 }, (_, w) => new Promise((resolve, reject) => {
  const from = TALKERS + w * share;
  const to = Math.min(TALKERS + LISTENERS, from + share);
  const child = fork(new URL(import.meta.url).pathname, process.argv.slice(2), { env: { ...process.env, DEN_LOAD_ROLE: 'listen', DEN_LOAD_RANGE: `${from}-${to}` } });
  child.once('message', (m) => (m.ready !== undefined ? resolve() : reject(new Error('a listener process failed'))));
  child.once('exit', (code) => code && reject(new Error(`a listener process exited with ${code}`)));
  workers.push(child);
})));
console.log(`listeners in: ${LISTENERS}, in ${workers.length} processes (${((now() - t1) / 1000).toFixed(1)} s)`);
const reports = () => Promise.all(workers.map((w) => new Promise((resolve) => { w.once('message', resolve); w.send({ report: true }); })));

// The first talker opens everything it is sent (sealed for it, by MLS) and
// counts it; the others only keep their place, so this machine's MLS work
// stays small.
let opened = 0;
talkers[0].on = (e, d) => { if (e === 'message' && !d.mine && d.kind === 'msg') opened++; if (e === 'error') errors.push(d.error); };
for (const c of talkers.slice(1)) c.take = async (room, m) => { c.seq[room] = Math.max(c.seq[room] || 0, m.seq); };
let sentBy0 = 0;

const pct = (xs, p) => { if (!xs.length) return null; const s = [...xs].sort((a, b) => a - b); return Math.round(s[Math.min(s.length - 1, Math.floor(p * s.length))]); };
const rows = [];
await reports();
for (const rate of RATES) {
  opened = 0;
  sentBy0 = 0;
  let sent = 0;
  let refused = 0;
  const end = now() + 20_000;
  const each = 1000 / rate;
  let k = 0;
  while (now() < end) {
    const c = talkers[k++ % talkers.length];
    const at = now();
    c.say('general', `load ${rate}/s #${k} ${'x'.repeat(80)}`).then(() => { sent++; if (c === talkers[0]) sentBy0++; }, (e) => { refused++; errors.push(e.message); });
    await sleep(Math.max(0, each - (now() - at)));
  }
  await sleep(3000);
  const got = await reports();
  delivered = got.reduce((n, r) => n + r.delivered, 0);
  lat.length = 0;
  for (const r of got) lat.push(...r.lat);
  const expected = sent * LISTENERS;
  rows.push({ rate, sent, refused, to_listeners: delivered, expected, per_s: Math.round(delivered / 20), p50_ms: pct(lat, 0.5), p95_ms: pct(lat, 0.95), p99_ms: pct(lat, 0.99), opened_by_one: opened, expected_by_one: sent - sentBy0 });
  console.log(JSON.stringify(rows.at(-1)));
}
console.table(rows);
if (errors.length) console.log('errors:', [...new Set(errors)].slice(0, 5));
for (const w of workers) w.send({ stop: true });
for (const c of talkers) c.socket.disconnect();
process.exit(0);
