// The den: ZCaveDAO's members' chat (DECISIONS §184), in the page.
//
// This device holds an MLS signature key and a pseudonym made from it, and
// its rooms' state; the den (chat.cavebro.win, Elixir) relays and orders
// ciphertext it cannot read. Entry is a day token the door signed blind:
// the door checks a member pass and signs a token it cannot see; the den
// later admits the device the token names, and cannot tell which pass it
// came from. Day keys come from cavebro.win/den-keys.json — one file for
// everyone — never from the den.
//
// Everything that must survive a reload stays in this browser (IndexedDB):
// the device, its session, tomorrow's token, and the words it has read —
// which forward secrecy means nobody can decrypt again later.

import { Socket } from './den/phoenix.mjs';
import init, { Den, token_blind as tokenBlind, token_finalize as tokenFinalize } from './den/den_mls.js';

export const ROOMS = [['general', 'General'], ['diggers', 'Diggers'], ['lookouts', 'Lookouts'], ['lanterns', 'Lanterns']];
const KEEP = 500;

// ---------------------------------------------------------------------------
// Bytes and storage.

const b64 = (u8) => { let s = ''; for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode(...u8.subarray(i, i + 0x8000)); return btoa(s); };
const unb64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
const unhex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));
const day = (offset = 0) => new Date(Date.now() + offset * 86_400_000).toISOString().slice(0, 10);

function store(prefix) {
  const open = () => new Promise((resolve, reject) => {
    const r = indexedDB.open('cave-den', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('kv');
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
  const db = open();
  const tx = async (mode, f) => {
    const d = await db;
    return new Promise((resolve, reject) => {
      const t = d.transaction('kv', mode);
      const r = f(t.objectStore('kv'));
      t.oncomplete = () => resolve(r?.result);
      t.onerror = () => reject(t.error);
    });
  };
  return {
    get: (k) => tx('readonly', (s) => s.get(`${prefix}:${k}`)),
    put: (k, v) => tx('readwrite', (s) => s.put(v, `${prefix}:${k}`)),
    del: (k) => tx('readwrite', (s) => s.delete(`${prefix}:${k}`)),
  };
}

// ---------------------------------------------------------------------------

export class DenClient {
  /// base: the den's https origin; keysUrl: the published day keys; prefix:
  /// what this browser keeps it under (one per network); on(event, data).
  constructor({ base, keysUrl, prefix = 'mainnet', on = () => {} }) {
    Object.assign(this, { base, keysUrl, on, kv: store(prefix) });
    this.queue = Promise.resolve();
    this.channels = new Map();
    this.seq = {};
  }

  /// Run MLS steps one at a time: the group state is one thing.
  lock(f) {
    const run = this.queue.then(f, f);
    this.queue = run.catch(() => {});
    return run;
  }

  async start() {
    await init(new URL('./den/den_mls_bg.wasm', import.meta.url));
    const saved = await this.kv.get('device');
    this.den = saved ? Den.restore(saved) : new Den();
    if (!saved) await this.save();
    this.seq = (await this.kv.get('seq')) || {};
    return { name: this.den.name(), key: this.den.key() };
  }

  async save() {
    await this.kv.put('device', this.den.save());
    await this.kv.put('seq', this.seq);
  }

  async dayKey(d) {
    this.keys ??= fetch(this.keysUrl, { credentials: 'omit' }).then((r) => r.json()).then((j) => j.keys || {});
    const k = (await this.keys)[d];
    return k ? unb64(k) : null;
  }

  async post(path, body) {
    const r = await fetch(`${this.base}${path}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), credentials: 'omit' });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || `The den answered ${r.status}`);
    return j;
  }

  /// A session without asking for the pass: the one kept, or one from the
  /// token kept since yesterday. Null when the pass is needed.
  async session() {
    const s = await this.kv.get('session');
    if (s && s.until * 1000 > Date.now() + 60_000) return s;
    const held = await this.kv.get(`token:${day()}`);
    if (held) {
      await this.kv.del(`token:${day()}`);
      try { return await this.enter(held); } catch { /* spent or stale: ask for the pass */ }
    }
    return null;
  }

  /// With a pass code from Nap: tokens for today and tomorrow, blinded; today's
  /// used now, tomorrow's kept for tomorrow — when the door cannot see it.
  async fromPass(code) {
    const key = this.den.key();
    let today = null;
    for (const [i, d] of [day(), day(1)].entries()) {
      const der = await this.dayKey(d);
      if (!der) { if (i === 0) throw new Error('The door has published no key for today yet.'); continue; }
      const pending = tokenBlind(der, d, key);
      const { blind_sig: sig } = await this.post('/door/sign', { code, day: d, blinded: JSON.parse(pending).blinded });
      const p = JSON.parse(pending);
      const token = { msg: p.msg, randomizer: p.randomizer, sig: tokenFinalize(der, pending, unhex(sig)) };
      if (i === 0) today = token;
      else await this.kv.put(`token:${d}`, token);
    }
    return this.enter(today);
  }

  async enter(token) {
    const s = await this.post('/door/enter', token);
    await this.kv.put('session', s);
    return s;
  }

  /// Connect with a session and take this device into every room.
  connect(session) {
    this.socket = new Socket(`${this.base.replace(/^http/, 'ws')}/socket`, { params: { session: session.session } });
    this.socket.onError(() => this.on('offline'));
    this.socket.onOpen(() => this.on('online'));
    this.socket.connect();
    for (const [room] of ROOMS) this.joinRoom(room);
  }

  push(ch, event, payload) {
    return new Promise((resolve, reject) => {
      ch.push(event, payload, 15_000)
        .receive('ok', resolve)
        .receive('error', (e) => reject(Object.assign(new Error(e?.reason || 'refused'), e || {})))
        .receive('timeout', () => reject(new Error('The den did not answer.')));
    });
  }

  joinRoom(room) {
    const ch = this.socket.channel(`room:${room}`);
    this.channels.set(room, ch);
    ch.on('msg', (m) => this.lock(() => this.take(room, m)));
    ch.on('expired', ({ keys }) => this.lock(() => this.removeStale(room, keys, true)));
    ch.join()
      .receive('ok', (state) => this.lock(() => this.sync(room, state)))
      .receive('error', (e) => this.on('error', { room, error: e?.reason || 'could not open the room' }));
  }

  /// In the room's group, caught up, and with members whose day ended taken out.
  async sync(room, state, tries = 0) {
    const ch = this.channels.get(room);
    if (!this.den.rooms().includes(room)) {
      try {
        if (!state.group_info) {
          const info = this.den.create(room);
          await this.push(ch, 'create', { group_info: b64(info) });
        } else {
          const commit = this.den.join(room, unb64(state.group_info));
          const r = await this.push(ch, 'relay', { bytes: b64(commit) });
          this.seq[room] = r.seq;
          await this.push(ch, 'info', { epoch: Number(this.den.epoch(room)), group_info: b64(this.den.group_info(room)) }).catch(() => {});
        }
        await this.save();
      } catch (e) {
        // Someone moved the room first: start again from where it is now.
        try { this.den.forget(room); } catch { /* not in it */ }
        if (tries < 4) return this.sync(room, await this.push(ch, 'state', {}), tries + 1);
        this.on('error', { room, error: e.message });
        return;
      }
    } else {
      const { messages } = await this.push(ch, 'history', { after: this.seq[room] || 0 });
      for (const m of messages) await this.take(room, m);
    }
    this.on('ready', { room, members: JSON.parse(this.den.members(room)) });
    const present = (await this.push(ch, 'state', {})).present || [];
    const stale = JSON.parse(this.den.members(room)).map((m) => m.key).filter((k) => k !== this.den.key() && !present.includes(k));
    if (stale.length) await this.removeStale(room, stale, false);
  }

  /// One relayed message: read it, keep the words.
  async take(room, m) {
    if ((this.seq[room] || 0) >= m.seq) return;
    this.seq[room] = m.seq;
    if (!this.den.rooms().includes(room)) return;
    let ev;
    try { ev = JSON.parse(this.den.receive(room, unb64(m.bytes))); } catch { await this.save(); return; } // before this device joined, or its own
    if (ev.kind === 'text') {
      await this.keep(room, { seq: m.seq, from: ev.from, text: ev.text, at: Date.now() });
      this.on('text', { room, from: ev.from, text: ev.text, mine: false });
    } else if (ev.kind === 'commit') {
      if (!ev.members.some((x) => x.key === this.den.key())) {
        // Taken out (a day that ended without renewal): join again if admitted.
        this.den.forget(room);
        await this.save();
        return this.sync(room, await this.push(this.channels.get(room), 'state', {}));
      }
      this.on('members', { room, members: ev.members });
    }
    await this.save();
  }

  /// Take out devices whose day ended. Every member's device tries; the den
  /// keeps the first commit, the others step back.
  async removeStale(room, keys, wait) {
    if (!this.den.rooms().includes(room)) return;
    if (wait) await new Promise((r) => setTimeout(r, Math.random() * 3000));
    const ours = keys.filter((k) => k !== this.den.key());
    const commit = this.den.remove(room, JSON.stringify(ours));
    if (!commit) return;
    try {
      const r = await this.push(this.channels.get(room), 'relay', { bytes: b64(commit) });
      this.seq[room] = r.seq;
      this.den.confirm(room);
      await this.push(this.channels.get(room), 'info', { epoch: Number(this.den.epoch(room)), group_info: b64(this.den.group_info(room)) }).catch(() => {});
      this.on('members', { room, members: JSON.parse(this.den.members(room)) });
    } catch {
      this.den.discard(room);
    }
    await this.save();
  }

  send(room, text) {
    return this.lock(async () => {
      const ch = this.channels.get(room);
      for (let tries = 0; tries < 2; tries++) {
        try {
          const r = await this.push(ch, 'relay', { bytes: b64(this.den.send(room, text)) });
          this.seq[room] = Math.max(this.seq[room] || 0, r.seq);
          await this.keep(room, { seq: r.seq, from: this.den.name(), text, at: Date.now(), mine: true });
          await this.save();
          return;
        } catch (e) {
          if (e.reason !== 'epoch') throw e;
          // Behind: catch up, then send again.
          const { messages } = await this.push(ch, 'history', { after: this.seq[room] || 0 });
          for (const m of messages) await this.take(room, m);
        }
      }
      throw new Error('The room moved while sending; try again.');
    });
  }

  async keep(room, line) {
    const log = (await this.kv.get(`log:${room}`)) || [];
    log.push(line);
    await this.kv.put(`log:${room}`, log.slice(-KEEP));
  }

  log(room) {
    return this.kv.get(`log:${room}`).then((l) => l || []);
  }

  /// Forget this device: its key, rooms, session, tokens and words.
  async forget() {
    for (const k of ['device', 'seq', 'session', `token:${day()}`, `token:${day(1)}`, ...ROOMS.map(([r]) => `log:${r}`)]) await this.kv.del(k);
  }
}
