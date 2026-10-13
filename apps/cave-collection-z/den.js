// The den: ZCaveDAO's members' chat (DECISIONS §184), in the page.
//
// This device holds an MLS signature key, a pseudonym made from it, and its
// rooms' state; the den (chat.cavebro.win, Elixir) relays and orders
// ciphertext it cannot read. Entry is a day token the door signed blind: the
// door checks a member pass and signs a token it cannot see; the den later
// admits the device the token names, and cannot tell which pass it came
// from. Day keys come from cavebro.win/den-keys.json — one file for
// everyone — never from the den.
//
// Inside a room every message is a small JSON document sealed by MLS: words
// (with a reply), reactions, edits and deletions by their author, files
// (encrypted here with a one-time key, the ciphertext stored by the den), a
// private room's title. Private rooms grow by invitation (MLS Add and
// Welcome). Voice rooms send audio sealed per frame under a key every member
// derives from the room's group, so the den forwards what it cannot hear.
//
// Everything that must survive a reload stays in this browser (IndexedDB):
// the device, its session, tomorrow's token, its private rooms, and the
// words it has read — which forward secrecy means nobody can decrypt later.

import { Socket } from './den/phoenix.mjs';
import init, { Den, token_blind as tokenBlind, token_finalize as tokenFinalize } from './den/den_mls.js';

const KEEP = 1000;
const INVITATIONS = 5;
export const VOICE_LABEL = 'cave-den voice';

// ---------------------------------------------------------------------------
// Bytes and storage.

export const b64 = (u8) => { let s = ''; for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode(...u8.subarray(i, i + 0x8000)); return btoa(s); };
export const unb64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
const unhex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));
const hex = (u8) => [...u8].map((x) => x.toString(16).padStart(2, '0')).join('');
const rand = (n) => crypto.getRandomValues(new Uint8Array(n));
const day = (offset = 0) => new Date(Date.now() + offset * 86_400_000).toISOString().slice(0, 10);

function store(prefix) {
  const db = new Promise((resolve, reject) => {
    const r = indexedDB.open('cave-den', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('kv');
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
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

/// A room message's body, decoded: older devices sent bare words.
function decode(text) {
  try {
    const m = JSON.parse(text);
    if (m && m.v === 1 && typeof m.t === 'string') return m;
  } catch { /* words */ }
  return { v: 1, t: 'msg', id: `old-${hex(rand(6))}`, text };
}

/// When a message was sent, by its sender's clock; one from the future (or
/// none) counts as now.
function sent(at) {
  const now = Date.now();
  return Number.isFinite(at) && at > 0 && at <= now + 60_000 ? Math.min(at, now) : now;
}

// ---------------------------------------------------------------------------

export class DenClient {
  /// base: the den's https origin; keysUrl: the published day keys; prefix:
  /// what this browser keeps it under (one per network); on(event, data).
  constructor({ base, keysUrl, prefix = 'mainnet', on = () => {} }) {
    Object.assign(this, { base, keysUrl, on, kv: store(prefix) });
    this.queue = Promise.resolve();
    this.channels = new Map();
    this.joined = new Map(); // id → a promise of the channel's join
    this.rooms = new Map(); // id → { id, title, kind }
    this.seq = {};
    this.read = {};
    this.typingAt = 0;
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
    this.read = (await this.kv.get('read')) || {};
    for (const r of (await this.kv.get('private')) || []) this.rooms.set(r.id, r);
    this.me = { name: this.den.name(), key: this.den.key() };
    return this.me;
  }

  async save() {
    await this.kv.put('device', this.den.save());
    await this.kv.put('seq', this.seq);
  }

  async keepPrivate() {
    await this.kv.put('private', [...this.rooms.values()].filter((r) => r.kind === 'private'));
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

  // ---- Entry ---------------------------------------------------------------

  /// A session without asking for the pass: the one kept, or one from the
  /// token kept since yesterday. Null when the pass is needed.
  async session() {
    const s = await this.kv.get('session');
    if (s && s.until * 1000 > Date.now() + 60_000) return (this.sessionToken = s);
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
    // The token names this device's key; the proof shows the device holds it.
    const s = await this.post('/door/enter', { ...token, proof: this.den.prove(token.msg) });
    await this.kv.put('session', s);
    this.sessionToken = s;
    return s;
  }

  // ---- The socket and the rooms ---------------------------------------------

  /// Connect with a session: the lobby, this device's inbox, every room.
  connect(session) {
    this.sessionToken = session;
    this.socket = new Socket(`${this.base.replace(/^http/, 'ws')}/socket`, { params: { session: session.session } });
    this.socket.onError(() => this.on('offline'));
    this.socket.onOpen(() => this.on('online'));
    this.socket.connect();

    this.lobby = this.socket.channel('lobby');
    this.lobby.on('room', (r) => this.addRoom(r));
    this.lobby.join().receive('ok', ({ rooms }) => rooms.forEach((r) => this.addRoom(r)));
    for (const r of this.rooms.values()) if (r.kind === 'private') this.joinRoom(r.id);

    const inbox = this.socket.channel(`inbox:${this.den.key()}`);
    this.inbox = inbox;
    inbox.on('welcome', (w) => this.lock(() => this.accept([{ id: null, room: w.room, welcome: w.welcome }])));
    inbox.join().receive('ok', ({ welcomes, key_packages: left }) => this.lock(async () => {
      await this.accept(welcomes);
      if (left < INVITATIONS) await this.push(inbox, 'publish', { key_packages: JSON.parse(this.den.key_packages(INVITATIONS - left)) }).catch(() => {});
      await this.save();
    }));
  }

  addRoom(r) {
    if (this.rooms.has(r.id) && this.channels.has(r.id)) return;
    this.rooms.set(r.id, { ...this.rooms.get(r.id), ...r });
    this.on('rooms', [...this.rooms.values()]);
    this.joinRoom(r.id);
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
    if (this.channels.has(room)) return;
    const ch = this.socket.channel(`room:${room}`);
    this.channels.set(room, ch);
    ch.on('msg', (m) => this.lock(() => this.take(room, m)));
    ch.on('expired', ({ keys }) => this.lock(() => this.removeStale(room, keys, true)));
    ch.on('typing', ({ key }) => this.on('typing', { room, key }));
    this.joined.set(room, new Promise((resolve) => {
      ch.join()
        .receive('ok', (state) => { resolve(true); this.lock(() => this.sync(room, state)); })
        .receive('error', (e) => { resolve(false); this.on('error', { room, error: e?.reason || 'could not open the room' }); });
    }));
  }

  /// In the room's group, caught up, and with members whose day ended taken out.
  async sync(room, state, tries = 0) {
    const ch = this.channels.get(room);
    const kind = state.kind || this.rooms.get(room)?.kind;
    // An open room the den has no group for (started afresh): this device's
    // old group is gone with it; start the room again.
    if (kind !== 'private' && !state.group_info && this.den.rooms().includes(room)) {
      try { this.den.forget(room); } catch { /* not in it */ }
      this.seq[room] = 0;
    }
    if (!this.den.rooms().includes(room)) {
      // A private room is entered by invitation only: wait for the Welcome.
      if (kind === 'private') return;
      try {
        if (!state.group_info) {
          const info = this.den.create(room);
          await this.push(ch, 'create', { group_info: b64(info) });
        } else {
          const commit = this.den.join(room, unb64(state.group_info));
          const r = await this.push(ch, 'relay', { bytes: b64(commit) });
          this.seq[room] = r.seq;
          await this.publishInfo(room);
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
      await this.catchUp(room);
    }
    this.on('ready', { room, members: this.members(room) });
    const present = (await this.push(ch, 'state', {})).present || [];
    const stale = this.members(room).map((m) => m.key).filter((k) => k !== this.den.key() && !present.includes(k));
    if (stale.length) await this.removeStale(room, stale, false);
  }

  async publishInfo(room) {
    if (this.rooms.get(room)?.kind === 'private') return;
    await this.push(this.channels.get(room), 'info', { epoch: Number(this.den.epoch(room)), group_info: b64(this.den.group_info(room)) }).catch(() => {});
  }

  members(room) {
    try { return JSON.parse(this.den.members(room)); } catch { return []; }
  }

  inRoom(room) {
    return this.den.rooms().includes(room);
  }

  /// Everything the room holds after what this device has read, a page at a
  /// time (the den caps a reply's size and says when there is more).
  async catchUp(room) {
    const ch = this.channels.get(room);
    for (let page = 0; page < 1000; page++) {
      const { messages, more } = await this.push(ch, 'history', { after: this.seq[room] || 0 });
      for (const m of messages) await this.take(room, m, false);
      if (!more || !messages.length) return;
    }
  }

  /// The den ordered something of this device's at `seq`: read what came
  /// before it first (a broadcast can still be on its way, or be overtaken
  /// by the den's answer), then stand at `seq`.
  async advance(room, seq) {
    if (seq > (this.seq[room] || 0) + 1) await this.catchUp(room);
    this.seq[room] = Math.max(this.seq[room] || 0, seq);
  }

  /// One relayed message: read it, keep it. A live message that skips a
  /// number means one was not seen: those are fetched first, so none is lost.
  async take(room, m, live = true) {
    if ((this.seq[room] || 0) >= m.seq) return;
    if (live && m.seq > (this.seq[room] || 0) + 1) {
      await this.catchUp(room);
      if ((this.seq[room] || 0) >= m.seq) return;
    }
    this.seq[room] = m.seq;
    if (!this.inRoom(room)) return;
    let ev;
    try { ev = JSON.parse(this.den.receive(room, unb64(m.bytes))); } catch { await this.save(); return; } // before this device joined, or its own
    if (ev.kind === 'text') {
      const d = decode(ev.text);
      await this.apply(room, { ...d, from: ev.from, key: ev.key, seq: m.seq, at: sent(d.at) });
    } else if (ev.kind === 'commit') {
      if (!ev.members.some((x) => x.key === this.den.key())) {
        // Taken out (a day that ended without renewal): join again if admitted.
        this.den.forget(room);
        await this.save();
        if (this.rooms.get(room)?.kind !== 'private') return this.sync(room, await this.push(this.channels.get(room), 'state', {}));
        return;
      }
      this.on('members', { room, members: ev.members });
    }
    await this.save();
  }

  // ---- Messages --------------------------------------------------------------

  async log(room) {
    return (await this.kv.get(`log:${room}`)) || [];
  }

  /// Apply one sealed document to the room's log: words added; a reaction,
  /// edit or deletion applied to the message it names (edits and deletions
  /// only from that message's own author).
  async apply(room, m, mine = false) {
    const log = await this.log(room);
    const at = (id) => log.findIndex((x) => x.id === id);
    let shown = null;
    if (m.t === 'msg' || m.t === 'file') {
      if (at(m.id) >= 0) return;
      shown = { id: m.id, t: m.t, from: m.from, key: m.key, text: m.text || '', reply: m.reply || null, file: m.t === 'file' ? m.file : null, seq: m.seq || 0, at: m.at || Date.now(), mine, reactions: {} };
      log.push(shown);
    } else if (m.t === 'react') {
      const i = at(m.to);
      if (i < 0 || typeof m.emoji !== 'string' || !m.emoji || [...m.emoji].length > 4) return;
      // Counted by the reactor's key (the MLS sender's), not by its name.
      if (!m.key) return;
      const r = (log[i].reactions[m.emoji] ||= []);
      const k = r.indexOf(m.key);
      if (k >= 0) r.splice(k, 1); else r.push(m.key);
      if (!r.length) delete log[i].reactions[m.emoji];
      shown = log[i];
    } else if (m.t === 'edit' || m.t === 'del') {
      const i = at(m.to);
      if (i < 0 || !m.key || log[i].key !== m.key) return;
      if (m.t === 'edit') Object.assign(log[i], { text: String(m.text || '').slice(0, 4000), edited: true });
      else Object.assign(log[i], { text: '', file: null, deleted: true });
      shown = log[i];
    } else if (m.t === 'title') {
      const r = this.rooms.get(room);
      if (r && r.kind === 'private') { r.title = String(m.text || '').slice(0, 60); await this.keepPrivate(); this.on('rooms', [...this.rooms.values()]); }
      return;
    } else return;
    await this.kv.put(`log:${room}`, log.slice(-KEEP));
    this.on('message', { room, message: shown, kind: m.t, mine, mention: !mine && (m.text || '').includes(`@${this.me.name}`) });
  }

  /// Send one sealed document to a room, and keep it here.
  send(room, body) {
    return this.lock(async () => {
      const ch = this.channels.get(room);
      const m = { v: 1, id: hex(rand(8)), at: Date.now(), ...body };
      for (let tries = 0; tries < 2; tries++) {
        try {
          const r = await this.push(ch, 'relay', { bytes: b64(this.den.send(room, JSON.stringify(m))) });
          await this.advance(room, r.seq);
          await this.apply(room, { ...m, from: this.me.name, key: this.me.key, seq: r.seq }, true);
          this.markRead(room);
          await this.save();
          return m.id;
        } catch (e) {
          if (e.reason !== 'epoch') throw e;
          // Behind: catch up, then send again.
          await this.catchUp(room);
        }
      }
      throw new Error('The room moved while sending; try again.');
    });
  }

  say(room, text, reply = null) {
    // A new message after this one: others see the writing again at once.
    this.typingAt = 0;
    return this.send(room, { t: 'msg', text: String(text).slice(0, 4000), ...(reply ? { reply } : {}) });
  }
  react(room, to, emoji) { return this.send(room, { t: 'react', to, emoji }); }
  edit(room, to, text) { return this.send(room, { t: 'edit', to, text: String(text).slice(0, 4000) }); }
  remove(room, to) { return this.send(room, { t: 'del', to }); }

  typing(room) {
    if (Date.now() - this.typingAt < 3000) return;
    this.typingAt = Date.now();
    this.channels.get(room)?.push('typing', {});
  }

  // ---- Files -----------------------------------------------------------------

  /// A file: sealed here with a one-time key, its ciphertext to the den, the
  /// key only in the room's message.
  async sendFile(room, file) {
    if (file.size > 25 * 1024 * 1024) throw new Error('A file is at most 25 MB.');
    const raw = rand(32);
    const iv = rand(12);
    const key = await crypto.subtle.importKey('raw', raw, 'AES-GCM', false, ['encrypt']);
    const sealed = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, await file.arrayBuffer()));
    const r = await fetch(`${this.base}/blob/`, { method: 'POST', headers: { 'content-type': 'application/octet-stream', authorization: `Bearer ${this.sessionToken.session}` }, body: sealed, credentials: 'omit' });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || `The den answered ${r.status}`);
    return this.send(room, { t: 'file', file: { blob: j.id, name: String(file.name).slice(0, 120), mime: String(file.type || 'application/octet-stream').slice(0, 80), size: file.size, key: b64(raw), iv: b64(iv) } });
  }

  /// A file from the den, opened here.
  async openFile(file) {
    const r = await fetch(`${this.base}/blob/${file.blob}`, { headers: { authorization: `Bearer ${this.sessionToken.session}` }, credentials: 'omit' });
    if (!r.ok) throw new Error('That file is gone (files are kept 30 days).');
    const key = await crypto.subtle.importKey('raw', unb64(file.key), 'AES-GCM', false, ['decrypt']);
    const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: unb64(file.iv) }, key, await r.arrayBuffer());
    return new Blob([plain], { type: file.mime });
  }

  // ---- Private rooms ---------------------------------------------------------

  /// A private room with these devices: made here, each invited by one of
  /// its own one-time invitations; the den keeps no GroupInfo for it.
  startPrivate(keys, title) {
    return this.lock(async () => {
      const id = `p-${hex(rand(16))}`;
      await this.push(this.lobby, 'private_room', { id });
      const info = this.den.create(id);
      this.rooms.set(id, { id, title: String(title || '').slice(0, 60), kind: 'private' });
      await this.keepPrivate();
      this.joinRoom(id);
      if (!(await this.joined.get(id))) throw new Error('The den would not open the room.');
      // The den tracks the room's public state from its first GroupInfo
      // (it keeps none for joiners: a private room is joined by invitation).
      await this.push(this.channels.get(id), 'create', { group_info: b64(info) });
      await this.invite(id, keys);
      if (title) await this.lockless_send(id, { t: 'title', text: title });
      this.on('rooms', [...this.rooms.values()]);
      await this.save();
      return id;
    });
  }

  /// Add devices to a private room this device is in.
  invite(room, keys) {
    const run = async () => {
      const kps = [];
      for (const k of keys) kps.push((await this.push(this.lobby, 'claim', { key: k })).key_package);
      // Each invitation must be the device asked for (checked in the core).
      const { commit, welcome } = JSON.parse(this.den.add(room, JSON.stringify(kps), JSON.stringify(keys)));
      try {
        const r = await this.push(this.channels.get(room), 'relay', { bytes: commit });
        this.den.confirm(room);
        await this.advance(room, r.seq);
      } catch (e) {
        this.den.discard(room);
        throw e;
      }
      await this.push(this.channels.get(room), 'invite', { to: keys, welcome });
      this.on('members', { room, members: this.members(room) });
      await this.save();
    };
    return run();
  }

  /// A send inside a lock already held.
  async lockless_send(room, body) {
    const m = { v: 1, id: hex(rand(8)), at: Date.now(), ...body };
    const r = await this.push(this.channels.get(room), 'relay', { bytes: b64(this.den.send(room, JSON.stringify(m))) });
    await this.advance(room, r.seq);
    await this.apply(room, { ...m, from: this.me.name, key: this.me.key, seq: r.seq }, true);
  }

  /// Welcomes in this device's inbox: into those private rooms.
  async accept(welcomes) {
    const taken = [];
    for (const w of welcomes) {
      try {
        const room = this.den.accept(unb64(w.welcome));
        if (!this.rooms.has(room)) this.rooms.set(room, { id: room, title: '', kind: 'private' });
        await this.keepPrivate();
        this.joinRoom(room);
        this.on('rooms', [...this.rooms.values()]);
      } catch { /* already in it, or not for this device */ }
      if (w.id) taken.push(w.id);
    }
    if (taken.length) await this.push(this.inbox, 'taken', { ids: taken }).catch(() => {});
    await this.save();
  }

  // ---- Leaving ---------------------------------------------------------------

  /// Take out devices whose day ended. Every member's device tries; the den
  /// keeps the first commit, the others step back.
  async removeStale(room, keys, wait) {
    if (!this.inRoom(room)) return;
    if (wait) await new Promise((r) => setTimeout(r, Math.random() * 3000));
    const ours = keys.filter((k) => k !== this.den.key());
    const commit = this.den.remove(room, JSON.stringify(ours));
    if (!commit) return;
    try {
      const r = await this.push(this.channels.get(room), 'relay', { bytes: b64(commit) });
      this.den.confirm(room);
      await this.advance(room, r.seq);
      await this.publishInfo(room);
      this.on('members', { room, members: this.members(room) });
    } catch {
      this.den.discard(room);
    }
    await this.save();
  }

  // ---- Unread ----------------------------------------------------------------

  async unread(room) {
    const last = this.read[room] || 0;
    return (await this.log(room)).filter((m) => !m.mine && m.seq > last).length;
  }

  lastRead(room) {
    return this.read[room] || 0;
  }

  markRead(room) {
    this.read[room] = this.seq[room] || 0;
    this.kv.put('read', this.read);
  }

  /// Search the words this device keeps, in every room.
  async search(q) {
    const needle = q.trim().toLowerCase();
    if (!needle) return [];
    const out = [];
    for (const r of this.rooms.values()) {
      for (const m of await this.log(r.id)) {
        if (!m.deleted && ((m.text || '').toLowerCase().includes(needle) || (m.file?.name || '').toLowerCase().includes(needle))) out.push({ room: r.id, message: m });
      }
    }
    return out.slice(-200);
  }

  // ---- Voice -----------------------------------------------------------------

  /// The room's voice key for this epoch: every member derives it alike.
  async voiceKey(room) {
    const raw = this.den.export(room, VOICE_LABEL, 32);
    return { index: Number(this.den.epoch(room)) % 256, key: await crypto.subtle.importKey('raw', raw, 'AES-GCM', false, ['encrypt', 'decrypt']) };
  }

  /// Join a room's voice: one connection to the den, our microphone up,
  /// eight slots down, every frame sealed here and opened here.
  async joinVoice(room, onSlots) {
    if (!this.inRoom(room)) throw new Error('Enter the room first.');
    const voice = new Voice(this, room, onSlots);
    await voice.start();
    this.voice = voice;
    return voice;
  }

  /// Forget this device: its key, rooms, session, tokens and words.
  async forget() {
    for (const k of ['device', 'seq', 'read', 'session', 'private', `token:${day()}`, `token:${day(1)}`, ...[...this.rooms.keys()].map((r) => `log:${r}`)]) await this.kv.del(k);
  }
}

// ---------------------------------------------------------------------------
// Voice: WebRTC to the den's forwarding unit; each audio frame sealed with
// AES-GCM under the room's voice key: [ciphertext ‖ tag][iv 12][key index 1].

const supportsScript = typeof RTCRtpScriptTransform !== 'undefined';

class Voice {
  constructor(client, room, onSlots) {
    Object.assign(this, { client, room, onSlots: onSlots || (() => {}), keys: new Map(), people: [], sealed: 0, opened: 0, failed: 0 });
  }

  async refreshKey() {
    const { index, key } = await this.client.voiceKey(this.room);
    this.keys.set(index, key);
    this.current = index;
    if (this.keys.size > 3) this.keys.delete([...this.keys.keys()][0]);
    this.worker?.postMessage({ keys: [...this.keys.entries()], current: index });
  }

  async seal(frame) {
    const key = this.keys.get(this.current);
    const iv = rand(12);
    const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, frame.data));
    const out = new Uint8Array(ct.length + 13);
    out.set(ct); out.set(iv, ct.length); out[ct.length + 12] = this.current;
    frame.data = out.buffer;
    this.sealed++;
    return frame;
  }

  async open(frame) {
    const d = new Uint8Array(frame.data);
    if (d.length < 29) throw new Error('short');
    const key = this.keys.get(d[d.length - 1]);
    if (!key) throw new Error('no key for that epoch');
    const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: d.subarray(d.length - 13, d.length - 1) }, key, d.subarray(0, d.length - 13));
    frame.data = plain;
    this.opened++;
    return frame;
  }

  transform(endpoint, mode) {
    if (supportsScript) {
      this.worker ??= new Worker(new URL('./den/voice-worker.js', import.meta.url));
      this.worker.postMessage({ keys: [...this.keys.entries()], current: this.current });
      this.worker.onmessage = (e) => { if (e.data.count) { this.sealed = e.data.sealed; this.opened = e.data.opened; } };
      endpoint.transform = new RTCRtpScriptTransform(this.worker, { mode });
      return;
    }
    const { readable, writable } = endpoint.createEncodedStreams();
    const step = mode === 'seal' ? (f) => this.seal(f) : (f) => this.open(f);
    readable.pipeThrough(new TransformStream({
      transform: async (frame, ctl) => { try { ctl.enqueue(await step(frame)); } catch { this.failed++; } },
    })).pipeTo(writable);
  }

  async start() {
    await this.refreshKey();
    this.mic = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true }, video: false })
      .catch((e) => { throw new Error(e?.name === 'NotAllowedError' ? 'The microphone is blocked for this page.' : 'No microphone found.'); });
    const pc = new RTCPeerConnection({ encodedInsertableStreams: !supportsScript, bundlePolicy: 'max-bundle' });
    this.pc = pc;
    const up = pc.addTransceiver(this.mic.getAudioTracks()[0], { direction: 'sendonly' });
    this.transform(up.sender, 'seal');
    this.downs = [];
    for (let i = 0; i < 8; i++) this.downs.push(pc.addTransceiver('audio', { direction: 'recvonly' }));
    for (const t of this.downs) this.transform(t.receiver, 'open');
    pc.ontrack = (e) => {
      const stream = new MediaStream([e.track]);
      const audio = new Audio();
      audio.srcObject = stream;
      audio.autoplay = true;
      audio.play().catch(() => {});
      (this.audios ||= []).push(audio);
      this.meter(e.transceiver.mid, stream);
    };
    this.meter('me', this.mic);
    const ch = this.client.socket.channel(`voice:${this.room}`);
    this.ch = ch;
    ch.on('ice', (c) => pc.addIceCandidate(c).catch(() => {}));
    ch.on('slots', (slots) => {
      this.people = Object.entries(slots).map(([mid, key]) => ({ mid, key }));
      this.onSlots(this.people);
    });
    pc.onicecandidate = (e) => { if (e.candidate) ch.push('ice', e.candidate.toJSON()); };
    await new Promise((resolve, reject) => ch.join().receive('ok', resolve).receive('error', (e) => reject(new Error(e?.reason || 'could not join the voice room'))));
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    const answer = await this.client.push(ch, 'offer', { type: 'offer', sdp: offer.sdp });
    await pc.setRemoteDescription(answer);
    // A new epoch (someone joined or left the room): a new voice key.
    this.offMembers = setInterval(() => {
      const e = Number(this.client.den.epoch(this.room)) % 256;
      if (e !== this.current) this.refreshKey();
    }, 1000);
  }

  /// A level meter on one stream (a slot's mid, or 'me'), read after the
  /// frames are opened: who is speaking, for the page.
  meter(id, stream) {
    try {
      this.ac ||= new AudioContext();
      const a = this.ac.createAnalyser();
      a.fftSize = 512;
      this.ac.createMediaStreamSource(stream).connect(a);
      (this.meters ||= new Map()).set(id, a);
    } catch { /* no levels; voice still works */ }
  }

  /// Each stream's loudness now, 0..1, by slot mid (and 'me').
  levels() {
    const out = {};
    const buf = new Uint8Array(512);
    for (const [id, a] of this.meters || []) {
      a.getByteTimeDomainData(buf);
      let sum = 0;
      for (const v of buf) sum += ((v - 128) / 128) ** 2;
      out[id] = Math.sqrt(sum / buf.length);
    }
    if (this.muted) out.me = 0;
    return out;
  }

  mute(on) {
    this.muted = on;
    this.mic.getAudioTracks().forEach((t) => { t.enabled = !on; });
  }

  async leave() {
    clearInterval(this.offMembers);
    try { await this.client.push(this.ch, 'leave', {}); } catch { /* gone */ }
    this.ch.leave();
    this.pc.close();
    this.mic.getTracks().forEach((t) => t.stop());
    (this.audios || []).forEach((a) => { a.srcObject = null; });
    this.worker?.terminate();
    this.ac?.close().catch(() => {});
    this.client.voice = null;
  }
}
