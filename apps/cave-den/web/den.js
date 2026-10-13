// The den's client, in the page.
//
// This device holds an MLS signature key, a pseudonym made from it, and its
// rooms' state; the den (Elixir) relays and orders ciphertext it cannot read.
// Entry is a day token the door signed blind: the door checks a member pass
// and signs a token it cannot see; the den later admits the device the token
// names, and cannot tell which pass it came from. Day keys come from one
// published file, the same for everyone (keysUrl).
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
export const VOICE_LABEL = 'den voice';

// ---------------------------------------------------------------------------
// Bytes and storage.

export const b64 = (u8) => { let s = ''; for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode(...u8.subarray(i, i + 0x8000)); return btoa(s); };
export const unb64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

/// Files are sealed in pieces of 1 MiB, each its own AES-GCM message under
/// the file's one-time key: its IV is the file's with the piece's number mixed
/// in, and its authenticated data names that number and whether it is the
/// last, so pieces can be neither reordered nor dropped from the end.
export const CHUNK = 1 << 20;
/// The largest file any message may describe; each den sets its own lower
/// limit (/door/ways file_max), which is what this device checks before sealing.
const FILE_HARD = 2 ** 31;
const chunkIv = (iv, i) => { const v = Uint8Array.from(iv); const d = new DataView(v.buffer); d.setUint32(8, d.getUint32(8) ^ i); return v; };
const chunkAad = (i, last) => { const a = new Uint8Array(16); a.set(new TextEncoder().encode('den-file-v2')); new DataView(a.buffer).setUint32(11, i); a[15] = last ? 1 : 0; return a; };
/// Bytes a file of `size` takes sealed: a 16-byte tag per piece.
export const sealedSize = (size) => size + 16 * Math.max(1, Math.ceil(size / CHUNK));
/// The reactions the page offers (den-ui.js), and the only ones taken.
export const REACTIONS = ['👍', '❤️', '😂', '🎉', '👀', '🙏', '🔥', '✅'];
/// One line with nothing hidden in it (see den-ui.js plain()).
const plainText = (s) => String(s ?? '').replace(/[\t\n\r]/g, ' ').replace(/[\u0000-\u001f\u007f\u200B-\u200F\u202A-\u202E\u2060-\u2069\uFEFF\u061C]/g, '').replace(/\s+/g, ' ').trim();
// Shown in place only these; anything else (SVG, HTML, …) is a download.
const INLINE = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif']);

/// A file message's fields as another device sent them, checked: a den blob
/// id, a 32-byte key, a 12-byte iv, a size in range, a plain name. Null if
/// any is wrong.
export function fileMeta(f) {
  if (!f || typeof f !== 'object') return null;
  const bytes = (v, n) => { try { return typeof v === 'string' && unb64(v).length === n; } catch { return false; } };
  if (typeof f.blob !== 'string' || !/^[0-9a-f]{64}$/.test(f.blob)) return null;
  if (!bytes(f.key, 32) || !bytes(f.iv, 12)) return null;
  if (!Number.isSafeInteger(f.size) || f.size < 0 || f.size > FILE_HARD) return null;
  // Sealed in pieces (CHUNK), or, from older devices, whole.
  if (f.chunk !== undefined && f.chunk !== CHUNK) return null;
  const name = (typeof f.name === 'string' ? f.name : '').replace(/[\u0000-\u001f\u007f/\\\u200B-\u200F\u202A-\u202E\u2060-\u2069\uFEFF\u061C]/g, '').replace(/\s+/g, ' ').trim().slice(0, 120) || 'file';
  const mime = typeof f.mime === 'string' && INLINE.has(f.mime.toLowerCase()) ? f.mime.toLowerCase() : 'application/octet-stream';
  return { blob: f.blob, name, mime, size: f.size, key: f.key, iv: f.iv, ...(f.chunk ? { chunk: f.chunk } : {}) };
}
/// A name a member gives themselves, as others take it: one plain line of 1
/// to 32 characters that cannot pass for a device name (m- and hex). Empty
/// when it is not one.
export function nickOf(s) {
  const n = plainText(s).slice(0, 32);
  return n && !/^m-[0-9a-f]/i.test(n) ? n : '';
}
const sha = async (s) => hex(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s))));
const unhex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));
const hex = (u8) => [...u8].map((x) => x.toString(16).padStart(2, '0')).join('');
const rand = (n) => crypto.getRandomValues(new Uint8Array(n));
const day = (offset = 0) => new Date(Date.now() + offset * 86_400_000).toISOString().slice(0, 10);

function store(name, prefix) {
  const db = new Promise((resolve, reject) => {
    const r = indexedDB.open(name, 1);
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
    /// Everything this device keeps, gone: the database itself is deleted.
    wipe: async () => {
      (await db).close();
      await new Promise((resolve, reject) => { const r = indexedDB.deleteDatabase(name); r.onsuccess = resolve; r.onerror = () => reject(r.error); r.onblocked = resolve; });
    },
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
  /// base: the den's https origin; keysUrl: the published day keys; db and
  /// prefix: the browser database and key prefix it keeps this device under;
  /// on(event, data).
  constructor({ base, keysUrl, db = 'den', prefix = 'den', on = () => {} }) {
    Object.assign(this, { base, keysUrl, on, kv: store(db, prefix) });
    this.queue = Promise.resolve();
    this.channels = new Map();
    this.joined = new Map(); // id → a promise of the channel's join
    this.rooms = new Map(); // id → { id, title, kind }
    this.seq = {};
    this.read = {};
    this.nicks = {}; // device key → { name, at }: the names members gave themselves
    this.claims = {}; // device key → the device keys it says are the same person's
    this.typingAt = 0;
  }

  /// Run MLS steps one at a time: the group state is one thing.
  lock(f) {
    const run = this.queue.then(f, f);
    this.queue = run.catch(() => {});
    return run;
  }

  async start() {
    await init({ module_or_path: new URL('./den/den_mls_bg.wasm', import.meta.url) });
    const saved = await this.kv.get('device');
    this.den = saved ? Den.restore(saved) : new Den();
    if (!saved) await this.save();
    this.seq = (await this.kv.get('seq')) || {};
    this.read = (await this.kv.get('read')) || {};
    for (const r of (await this.kv.get('private')) || []) this.rooms.set(r.id, r);
    this.nicks = (await this.kv.get('nicks')) || {};
    this.claims = (await this.kv.get('claims')) || {};
    this.myLinks = (await this.kv.get('links')) || [];
    this.me = { name: this.den.name(), key: this.den.key(), nick: (await this.kv.get('nick')) || '' };
    // Leaving the page: what was read since the last write, written.
    if (typeof addEventListener === 'function') addEventListener('pagehide', () => { if (this.saveTimer) this.save().catch(() => {}); });
    return this.me;
  }

  /// What this device keeps of its rooms, written now: after anything it
  /// sends and every change of a room's members, so a reload can never reuse
  /// a key it already used.
  async save() {
    clearTimeout(this.saveTimer);
    this.saveTimer = null;
    await this.kv.put('device', this.den.save());
    await this.kv.put('seq', this.seq);
  }

  /// After words it reads, within a second: the state grows with the room
  /// (several MB at a thousand members), and a busy room should not write it
  /// for every message. Lost on a crash, it is read again from the den.
  saveSoon() {
    this.saveTimer ??= setTimeout(() => { this.saveTimer = null; this.save().catch(() => {}); }, 1000);
  }

  async keepPrivate() {
    await this.kv.put('private', [...this.rooms.values()].filter((r) => r.kind === 'private'));
  }

  /// The door's key for day `d`. Each day's key is pinned on this device the
  /// first time it is seen: a den that later shows a different key for a day
  /// (to tell this device apart from the other members) is refused.
  async dayKey(d) {
    this.keys ??= fetch(this.keysUrl, { credentials: 'omit' }).then((r) => r.json()).then(async (j) => {
      const keys = j.keys || {};
      for (const [day, k] of Object.entries(keys)) {
        if (!/^\d{4}-\d{2}-\d{2}$/.test(day) || typeof k !== 'string') continue;
        const pinned = await this.kv.get(`pin:${day}`);
        if (pinned && pinned !== k) throw new Error(`The den showed this device a different door key for ${day} than it showed before. Entry is refused: the den may be trying to tell this device apart.`);
        if (!pinned) await this.kv.put(`pin:${day}`, k);
      }
      return keys;
    });
    const k = (await this.keys)[d];
    return k ? unb64(k) : null;
  }

  async post(path, body) {
    return this.answer(await fetch(`${this.base}${path}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), credentials: 'omit' }));
  }

  async get(path) {
    return this.answer(await fetch(`${this.base}${path}`, { credentials: 'omit' }));
  }

  async answer(r) {
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw Object.assign(new Error(j.error || `The den answered ${r.status}`), { status: r.status });
    return j;
  }

  // ---- Entry ---------------------------------------------------------------

  /// A session without the gate: the one kept, one from the token kept since
  /// yesterday, or one from this device's own pass code. Null when the way in
  /// is needed.
  async session() {
    const s = await this.kv.get('session');
    if (s && s.until * 1000 > Date.now() + 60_000) return (this.sessionToken = s);
    const held = await this.kv.get(`token:${day()}`);
    if (held) {
      await this.kv.del(`token:${day()}`);
      try { return await this.enter(held); } catch { /* spent or stale: the pass, then the gate */ }
    }
    const code = await this.kv.get('pass');
    if (code) {
      try { return await this.fromPass(code); } catch (e) { if (e.status === 403 && /not a member/.test(e.message)) await this.kv.del('pass'); }
    }
    return null;
  }

  /// A day token for `d`, blinded: the door sees the pass, never the token.
  async tokenFor(code, d) {
    const der = await this.dayKey(d);
    if (!der) return null;
    const pending = tokenBlind(der, d, this.den.key());
    const { blind_sig: sig } = await this.post('/door/sign', { code, day: d, blinded: JSON.parse(pending).blinded });
    const p = JSON.parse(pending);
    return { msg: p.msg, randomizer: p.randomizer, sig: tokenFinalize(der, pending, unhex(sig)) };
  }

  /// With a pass code: today's token, used now, and in the day's last hours
  /// tomorrow's too, kept for tomorrow, when the door cannot see it. The
  /// code is kept on this device, so it can get the next days' tokens itself.
  async fromPass(code) {
    const today = await this.tokenFor(code, day());
    if (!today) throw new Error('The door has published no key for today yet.');
    if (await this.lateInDay()) {
      const tomorrow = await this.tokenFor(code, day(1)).catch(() => null);
      if (tomorrow) await this.kv.put(`token:${day(1)}`, tomorrow);
    }
    if (await this.keepsPass()) await this.kv.put('pass', code);
    return this.enter(today);
  }

  /// The UTC hour from which the door signs tomorrow's tokens (it signs them
  /// only late in the day, so a pass the host removes stops at midnight).
  async tomorrowFrom() {
    this.waysSeen ??= this.ways().catch(() => ({}));
    const h = (await this.waysSeen).tomorrow_from;
    return Number.isInteger(h) && h >= 0 && h <= 23 ? h : 0;
  }

  async lateInDay() {
    return new Date().getUTCHours() >= await this.tomorrowFrom();
  }

  /// When to fetch tomorrow's token: a random moment in the day's last hours,
  /// while connected, rather than at entry, so the door's and the den's
  /// records of this device do not line up in time.
  async refillAt() {
    const from = await this.tomorrowFrom();
    const now = Date.now();
    const d = new Date(now);
    const start = Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), from);
    const end = Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate() + 1) - 10 * 60_000;
    const lo = Math.max(start, now + 60_000);
    return lo >= end ? null : lo + Math.random() * (end - lo);
  }

  scheduleRefill() {
    clearTimeout(this.refillTimer);
    this.refillAt().then((at) => {
      // Past the window today: try again tomorrow.
      const wait = at ? at - Date.now() : Date.UTC(new Date().getUTCFullYear(), new Date().getUTCMonth(), new Date().getUTCDate() + 1) + 60_000 - Date.now();
      this.refillTimer = setTimeout(() => (at ? this.refill() : Promise.resolve()).finally(() => this.scheduleRefill()), Math.min(wait, 2 ** 31 - 1));
    });
  }

  async refill() {
    const d = day(1);
    const code = await this.kv.get('pass');
    if (!code || await this.kv.get(`token:${d}`)) return;
    const t = await this.tokenFor(code, d).catch(() => null);
    if (t) await this.kv.put(`token:${d}`, t);
  }

  // ---- The den's own member list (Den.Gate.Members) ---------------------------

  /// The ways in the den opens: { invite, approval }.
  ways() { return this.get('/door/ways'); }

  /// A pass code made on this device, and the hash the den keeps of it.
  static async newPass() {
    const code = rand(32);
    const msg = new Uint8Array([0, ...new TextEncoder().encode('den.pass.v1'), ...code]);
    return { code: hex(code), pass: hex(new Uint8Array(await crypto.subtle.digest('SHA-256', msg))) };
  }

  /// In with an invite link's code: this device's pass joins the member list.
  async join(invite) {
    const { code, pass } = await DenClient.newPass();
    await this.post('/door/join', { invite, pass });
    return this.fromPass(code);
  }

  /// Asks the host to let this device in, with a note.
  async ask(note) {
    const { code, pass } = await DenClient.newPass();
    const { id } = await this.post('/door/ask', { pass, note });
    await this.kv.put('asking', { id, code });
    return id;
  }

  /// Where this device's request stands: { status }, and a session once approved.
  async asked() {
    const a = await this.kv.get('asking');
    if (!a) return null;
    const { status } = await this.get(`/door/ask/${a.id}`);
    if (status === 'pending') return { status };
    await this.kv.del('asking');
    return status === 'approved' ? { status, session: await this.fromPass(a.code) } : { status };
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
    // The session goes in the WebSocket's protocol header, never in its URL.
    this.socket = new Socket(`${this.base.replace(/^http/, 'ws')}/socket`, { authToken: session.session });
    this.socket.onError(() => this.on('offline'));
    this.socket.onOpen(() => this.on('online'));
    this.socket.connect();
    this.scheduleRefill();

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
    ch.on('rolled_back', (r) => this.lock(() => this.rolledBack(room, r)));
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
    // A private room's id says so (p- and 32 hex digits): whatever the den
    // answers, this device never joins it by GroupInfo nor forgets it because
    // the den has no group for it.
    const kind = /^p-[0-9a-f]{32}$/.test(room) ? 'private' : state.kind || this.rooms.get(room)?.kind;
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
        if (this.me.nick || this.myLinks.length) this.soonProfile(room);
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
    (this.seenMembers ||= {})[room] = new Set(this.members(room).map((m) => m.key));
    this.on('ready', { room, members: this.members(room) });
    this.tellSeen(room);
    this.soonRefresh(room);
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
    const at = (() => { try { return Number(this.den.epoch(room)); } catch { return -1; } })();
    try { ev = JSON.parse(this.den.receive(room, unb64(m.bytes))); } catch {
      // A commit at this device's own epoch that it cannot apply: never
      // skipped quietly, or the room would go on without this device.
      if (m.commit && m.epoch === at) return this.cannotApply(room, m);
      this.saveSoon();
      return; // before this device joined, or its own
    }
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
      await this.noteMembers(room, ev.members);
      this.on('members', { room, members: ev.members });
      await this.save();
      return;
    }
    this.saveSoon();
  }

  /// A line in the room's log for each device a commit added or took out.
  async noteMembers(room, members) {
    const now = new Set(members.map((m) => m.key));
    const before = this.seenMembers?.[room];
    (this.seenMembers ||= {})[room] = now;
    if (!before) return;
    const lines = [];
    for (const k of now) if (!before.has(k) && k !== this.den.key()) lines.push([k, 'joined']);
    for (const k of before) if (!now.has(k)) lines.push([k, 'left']);
    if (!lines.length) return;
    // Someone new does not know the names members gave themselves: tell them.
    if ((this.me.nick || this.myLinks.length) && lines.some(([, act]) => act === 'joined') && now.size <= 100) this.soonProfile(room);
    const log = await this.log(room);
    for (const [who, act] of lines) log.push({ id: `note-${hex(rand(6))}`, t: 'note', who, act, text: `m-${who.slice(0, 20)} ${act}`, at: Date.now(), seq: this.seq[room] || 0, reactions: {} });
    await this.kv.put(`log:${room}`, log.slice(-KEEP));
    this.on('message', { room, message: log.at(-1), kind: 'note', mine: false, mention: false });
  }

  /// This device cannot apply the commit the den ordered at `m.seq`. It says
  /// so, signed, to the den, which undoes the commit when most members say
  /// the same; otherwise an open room is joined again where it now stands,
  /// and a private room waits for a member to invite this device again.
  async cannotApply(room, m) {
    const ch = this.channels.get(room);
    this.on('notice', { room, text: 'A member sent a change to this room that this device could not apply. It has told the den.' });
    let undone = false;
    try {
      const r = await this.push(ch, 'report', { seq: m.seq, sig: this.den.report(room, BigInt(m.seq)) });
      undone = !!r.rolled_back;
    } catch { /* stale, or already undone */ }
    if (undone) { this.seq[room] = m.seq - 1; await this.save(); return; }
    await this.save();
    if (this.rooms.get(room)?.kind === 'private') {
      this.on('notice', { room, text: 'This device is out of step with this private room. Ask a member to invite it again.' });
      return;
    }
    // An open room: join again where it now stands, once someone has given
    // the den its GroupInfo there; if nobody does, the den undoes the commit
    // and this device is in step again. Waited for outside the lock.
    setTimeout(() => this.recover(room, m.epoch), 3000);
  }

  async recover(room, stuckAt, tries = 0) {
    const ch = this.channels.get(room);
    const st = await this.push(ch, 'state', {}).catch(() => null);
    if (st && st.epoch <= stuckAt) return; // undone: the room is back where this device is
    if (st && st.info_epoch === st.epoch && st.group_info) {
      return this.lock(async () => {
        try { this.den.forget(room); } catch { /* not in it */ }
        await this.save();
        return this.sync(room, st);
      });
    }
    if (tries < 30) setTimeout(() => this.recover(room, stuckAt, tries + 1), 5000);
  }

  /// The den undid a commit most members could not apply. A device that had
  /// applied it is now ahead of the room: it joins again (an open room) or
  /// waits to be invited again (a private one).
  async rolledBack(room, { epoch, seq }) {
    if ((this.seq[room] || 0) > seq) this.seq[room] = seq;
    let mine = -1;
    try { mine = Number(this.den.epoch(room)); } catch { /* not in it */ }
    if (mine <= epoch) { await this.save(); return; }
    try { this.den.forget(room); } catch { /* not in it */ }
    await this.save();
    if (this.rooms.get(room)?.kind === 'private') {
      this.on('notice', { room, text: 'A change in this private room was undone and this device had already applied it. Ask a member to invite it again.' });
      return;
    }
    return this.sync(room, await this.push(this.channels.get(room), 'state', {}));
  }

  // ---- Messages --------------------------------------------------------------

  async log(room) {
    return (await this.kv.get(`log:${room}`)) || [];
  }

  /// Apply one sealed document to the room's log: words added; a reaction,
  /// edit or deletion applied to the message it names (edits and deletions
  /// only from that message's own author).
  async apply(room, m, mine = false) {
    // Any document can carry the name its sender gave themselves. It is a
    // claim, sealed by MLS to the sender's key: shown beside that key's own
    // name, never instead of it where it matters.
    if (!mine && typeof m.key === 'string' && 'nick' in m) await this.learnNick(m.key, m.nick, m.at);
    if (!mine && typeof m.key === 'string' && Array.isArray(m.links)) await this.learnClaims(m.key, m.links);
    if (m.t === 'link-ask' || m.t === 'link') { if (!mine) await this.onLink(room, m); return; }
    if (m.t === 'profile') return;
    const log = await this.log(room);
    const at = (id) => log.findIndex((x) => x.id === id);
    let shown = null;
    if (m.t === 'msg' || m.t === 'file') {
      if (typeof m.id !== 'string' || at(m.id) >= 0) return;
      // Every message carries its sender's door-key fingerprint for its day,
      // so the den cannot drop the evidence without dropping the words.
      if (!mine && m.kh) await this.compareKey(room, new Date(m.at || Date.now()).toISOString().slice(0, 10), m.kh, m);
      const file = m.t === 'file' ? fileMeta(m.file) : null;
      if (m.t === 'file' && !file) return;
      // As the sender's page caps it: a longer message from a modified client is cut.
      shown = { id: m.id, t: m.t, from: m.from, key: m.key, text: typeof m.text === 'string' ? m.text.slice(0, 4000) : '', reply: typeof m.reply === 'string' ? m.reply : null, file, seq: m.seq || 0, at: m.at || Date.now(), mine, reactions: {} };
      log.push(shown);
    } else if (m.t === 'react') {
      const i = at(m.to);
      // Only the reactions the page offers, and at most a dozen kinds a message.
      if (i < 0 || !REACTIONS.includes(m.emoji)) return;
      if (!log[i].reactions[m.emoji] && Object.keys(log[i].reactions).length >= 12) return;
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
    } else if (m.t === 'seen') {
      // A roommate's fingerprint of the door key the den showed it for a day,
      // for a member who has not written that day (words carry it too).
      if (!mine) await this.compareKey(room, m.day, m.key_hash, m);
      return;
    } else if (m.t === 'title') {
      // A private room's name: one line, never a member's name (which would
      // make it look like a one-to-one room with someone else), and the room
      // is told who gave it.
      const r = this.rooms.get(room);
      const title = plainText(m.text).slice(0, 60);
      if (!r || r.kind !== 'private' || !title || /^m-[0-9a-f]/i.test(title)) return;
      r.title = title;
      await this.keepPrivate();
      this.on('rooms', [...this.rooms.values()]);
      shown = { id: `note-${m.id}`, t: 'note', who: m.key, act: `named the room “${title}”`, text: `${m.from} named the room “${title}”`, at: m.at || Date.now(), seq: m.seq || 0, reactions: {} };
      log.push(shown);
    } else return;
    await this.kv.put(`log:${room}`, log.slice(-KEEP));
    // A mention of this device, or of one linked to it, is a mention of its member.
    const mention = !mine && [this.me.key, ...this.linkedTo(this.me.key)].some((k) => (m.text || '').includes(`@m-${k.slice(0, 20)}`));
    this.on('message', { room, message: shown, kind: m.t, mine, mention });
  }

  /// A roommate's fingerprint of the door key it was shown for `day`: one
  /// different from ours is said loudly. The message is sealed, so the den
  /// cannot hide or change it (though any member can send a wrong one).
  async compareKey(room, d, hash, m) {
    if (typeof d !== 'string' || typeof hash !== 'string') return;
    const own = await this.seenHash(d);
    if (own && own !== hash) this.on('keywarn', { room, day: d, from: m.from, key: m.key });
  }

  /// SHA-256 of the door key this device pinned for `day`, hex; null if none.
  async seenHash(day) {
    const k = await this.kv.get(`pin:${day}`);
    if (typeof k !== 'string') return null;
    return hex(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(k))));
  }

  /// Once a day in each room: tell the members, sealed, which door key the
  /// den showed this device today, so each can compare with its own.
  async tellSeen(room) {
    const d = day();
    if ((await this.kv.get(`seen:${room}`)) === d) return;
    const key_hash = await this.seenHash(d);
    if (!key_hash) return;
    try {
      await this.send(room, { t: 'seen', day: d, key_hash });
      await this.kv.put(`seen:${room}`, d);
    } catch { /* next time */ }
  }

  /// Send one sealed document to a room, and keep it here.
  send(room, body) {
    return this.lock(async () => {
      const ch = this.channels.get(room);
      const m = { v: 1, id: hex(rand(8)), at: Date.now(), ...(this.me.nick ? { nick: this.me.nick } : {}), ...body };
      if (body.t === 'msg' || body.t === 'file') {
        m.kh = await this.seenHash(day()).catch(() => null);
        if (!m.kh) delete m.kh;
        await this.kv.put(`seen:${room}`, day());
      }
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

  /// The most one file may be on this den, sealed (older dens: 25 MiB).
  async fileMax() {
    this.waysSeen ??= this.ways().catch(() => ({}));
    const m = (await this.waysSeen).file_max;
    return Number.isSafeInteger(m) && m > 0 ? m : 25 * 1024 * 1024;
  }

  /// A file: sealed here, piece by piece, with a one-time key; its
  /// ciphertext to the den, the key only in the room's message. `progress`
  /// hears ('seal' | 'send', 0..1).
  async sendFile(room, file, progress = () => {}) {
    const max = await this.fileMax();
    if (sealedSize(file.size) > max) throw new Error(`A file is at most ${Math.floor(max / 1e6)} MB on this den.`);
    const raw = rand(32);
    const iv = rand(12);
    const key = await crypto.subtle.importKey('raw', raw, 'AES-GCM', false, ['encrypt']);
    const n = Math.max(1, Math.ceil(file.size / CHUNK));
    // A Blob of Blobs: the browser may keep a large one on disk, not in memory.
    let sealed = new Blob([]);
    for (let i = 0; i < n; i++) {
      const piece = await file.slice(i * CHUNK, (i + 1) * CHUNK).arrayBuffer();
      sealed = new Blob([sealed, await crypto.subtle.encrypt({ name: 'AES-GCM', iv: chunkIv(iv, i), additionalData: chunkAad(i, i === n - 1) }, key, piece)]);
      progress('seal', (i + 1) / n);
    }
    const j = await this.upload(sealed, (f) => progress('send', f));
    return this.send(room, { t: 'file', file: { blob: j.id, name: String(file.name).slice(0, 120), mime: String(file.type || 'application/octet-stream').slice(0, 80), size: file.size, key: b64(raw), iv: b64(iv), chunk: CHUNK } });
  }

  /// Sealed bytes to the den, with progress (an XMLHttpRequest says how far
  /// an upload is; fetch does not).
  upload(body, progress = () => {}) {
    return new Promise((resolve, reject) => {
      const x = new XMLHttpRequest();
      x.open('POST', `${this.base}/blob/`);
      x.setRequestHeader('content-type', 'application/octet-stream');
      x.setRequestHeader('authorization', `Bearer ${this.sessionToken.session}`);
      x.upload.onprogress = (e) => { if (e.lengthComputable) progress(e.loaded / e.total); };
      x.onload = () => {
        let j = {};
        try { j = JSON.parse(x.responseText); } catch { /* not JSON */ }
        if (x.status >= 200 && x.status < 300 && typeof j.id === 'string') resolve(j); else reject(new Error(j.error || `The den answered ${x.status}`));
      };
      x.onerror = () => reject(new Error('The file did not reach the den.'));
      x.send(body);
    });
  }

  /// A file from the den, opened here, piece by piece as it arrives.
  /// `progress` hears 0..1.
  async openFile(given, progress = () => {}) {
    const file = fileMeta(given);
    if (!file) throw new Error('That file cannot be opened.');
    const r = await fetch(`${this.base}/blob/${file.blob}`, { headers: { authorization: `Bearer ${this.sessionToken.session}` }, credentials: 'omit' });
    if (!r.ok) throw new Error('That file is gone (files are kept 30 days).');
    const key = await crypto.subtle.importKey('raw', unb64(file.key), 'AES-GCM', false, ['decrypt']);
    const iv = unb64(file.iv);
    if (!file.chunk) {
      const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, key, await r.arrayBuffer());
      return new Blob([plain], { type: file.mime });
    }
    const n = Math.max(1, Math.ceil(file.size / file.chunk));
    const whole = file.chunk + 16;
    const parts = [];
    let pending = [];
    let have = 0;
    let i = 0;
    let out = 0;
    const flat = () => { const b = new Uint8Array(have); let o = 0; for (const p of pending) { b.set(p, o); o += p.length; } return b; };
    const open = async (bytes, last) => {
      const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: chunkIv(iv, i), additionalData: chunkAad(i, last) }, key, bytes);
      parts.push(new Blob([plain]));
      out += plain.byteLength;
      i++;
      progress(i / n);
    };
    const reader = r.body.getReader();
    for (;;) {
      const { value, done } = await reader.read();
      if (value?.length) { pending.push(value); have += value.length; }
      while (i < n - 1 && have >= whole) {
        const b = flat();
        await open(b.subarray(0, whole), false);
        pending = [b.subarray(whole)];
        have = pending[0].length;
      }
      if (done) break;
    }
    if (i !== n - 1 || have < 16 || have > whole) throw new Error('That file is not whole.');
    await open(flat(), true);
    if (out !== file.size) throw new Error('That file is not whole.');
    return new Blob(parts, { type: file.mime });
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
    const m = { v: 1, id: hex(rand(8)), at: Date.now(), ...(this.me.nick ? { nick: this.me.nick } : {}), ...body };
    const r = await this.push(this.channels.get(room), 'relay', { bytes: b64(this.den.send(room, JSON.stringify(m))) });
    await this.advance(room, r.seq);
    await this.apply(room, { ...m, from: this.me.name, key: this.me.key, seq: r.seq }, true);
  }

  /// Welcomes in this device's inbox: into those private rooms.
  /// Welcomes in this device's inbox wait, kept on this device, until its
  /// member chooses to join or decline: nobody can pull a device into a room.
  async accept(welcomes) {
    const taken = [];
    const waiting = (await this.kv.get('invites')) || [];
    for (const w of welcomes) {
      if (typeof w.welcome === 'string' && !waiting.some((x) => x.welcome === w.welcome) && waiting.length < 50) {
        waiting.push({ id: hex(rand(6)), welcome: w.welcome, at: Date.now() });
      }
      if (w.id) taken.push(w.id);
    }
    await this.kv.put('invites', waiting);
    if (taken.length) await this.push(this.inbox, 'taken', { ids: taken }).catch(() => {});
    this.on('invites', waiting);
  }

  async invites() {
    return (await this.kv.get('invites')) || [];
  }

  /// Join the private room an invitation is for.
  joinInvite(id) {
    return this.lock(async () => {
      const waiting = await this.invites();
      const w = waiting.find((x) => x.id === id);
      if (!w) return null;
      await this.kv.put('invites', waiting.filter((x) => x.id !== id));
      let room = null;
      try {
        room = this.den.accept(unb64(w.welcome));
        if (!this.rooms.has(room)) this.rooms.set(room, { id: room, title: '', kind: 'private' });
        await this.keepPrivate();
        this.joinRoom(room);
        if (this.me.nick || this.myLinks.length) this.soonProfile(room);
        this.on('rooms', [...this.rooms.values()]);
      } catch { /* already in it, or not for this device */ }
      await this.save();
      this.on('invites', await this.invites());
      return room;
    });
  }

  async declineInvite(id) {
    await this.kv.put('invites', (await this.invites()).filter((x) => x.id !== id));
    this.on('invites', await this.invites());
  }

  /// Leave a private room: this device forgets it, and the members' devices
  /// take it out once they see it gone.
  leave(room) {
    return this.lock(async () => {
      this.channels.get(room)?.leave();
      this.channels.delete(room);
      this.joined.delete(room);
      try { this.den.forget(room); } catch { /* not in it */ }
      this.rooms.delete(room);
      await this.keepPrivate();
      await this.kv.del(`log:${room}`);
      await this.save();
      this.on('rooms', [...this.rooms.values()]);
    });
  }

  // ---- Fresh keys ----------------------------------------------------------------

  /// Once a day in each room, at a random moment, this device replaces its
  /// own MLS keys (a self-update commit): whoever once copied its state reads
  /// nothing sent after that.
  async soonRefresh(room) {
    (this.refreshSoon ||= {});
    if (this.refreshSoon[room]) return;
    const last = (await this.kv.get(`refresh:${room}`)) || 0;
    const due = Math.max(last + 86_400_000, Date.now()) + Math.random() * 30 * 60_000;
    this.refreshSoon[room] = setTimeout(() => {
      delete this.refreshSoon[room];
      this.refreshKeys(room).catch(() => {}).finally(() => this.soonRefresh(room));
    }, Math.min(due - Date.now(), 2 ** 31 - 1));
  }

  refreshKeys(room) {
    return this.lock(async () => {
      if (!this.inRoom(room)) return;
      const commit = this.den.refresh(room);
      try {
        const r = await this.push(this.channels.get(room), 'relay', { bytes: b64(commit) });
        this.den.confirm(room);
        await this.advance(room, r.seq);
        await this.publishInfo(room);
        await this.kv.put(`refresh:${room}`, Date.now());
      } catch (e) {
        // Someone changed the room first: next time.
        this.den.discard(room);
      }
      await this.save();
    });
  }

  // ---- Leaving ---------------------------------------------------------------

  /// Take out devices whose day ended. Every member's device tries; the den
  /// keeps the first commit, the others step back.
  async removeStale(room, keys, wait) {
    if (!this.inRoom(room)) return;
    // In a private room the den's word that someone's day ended is not enough
    // to take them out: there, only a member decides. (A device whose day
    // ended cannot reach the room's messages without a new token anyway.)
    if (this.rooms.get(room)?.kind === 'private' || /^p-[0-9a-f]{32}$/.test(room)) return;
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

  // ---- Names members give themselves ---------------------------------------------

  /// The name a device's member gave themselves, or ''; for a device linked
  /// to others (one person's), the newest name any of them gave.
  nickFor(key) {
    if (key === this.me?.key && this.me.nick) return this.me.nick;
    let best = key === this.me?.key ? null : this.nicks[key] || null;
    for (const k of this.linkedTo(key)) {
      if (k === this.me?.key && this.me.nick) return this.me.nick;
      const n = this.nicks[k];
      if (n && (!best || n.at > best.at)) best = n;
    }
    return best?.name || '';
  }

  // ---- One person, several devices ---------------------------------------------

  /// The devices `key` is linked to: each says the other is the same
  /// person's (a claim one side alone makes links nothing).
  linkedTo(key) {
    const says = (k) => (k === this.me?.key ? this.myLinks : this.claims[k] || []);
    return says(key).filter((k) => k !== key && says(k).includes(key));
  }

  /// This device's own, and those linked to it: "you".
  isMe(key) {
    return key === this.me?.key || this.linkedTo(this.me?.key).includes(key);
  }

  async learnClaims(key, links) {
    const keys = [...new Set(links.filter((k) => typeof k === 'string' && /^[0-9a-f]{64}$/.test(k) && k !== key))].slice(0, 10);
    const had = this.claims[key] || [];
    if (had.length === keys.length && had.every((k) => keys.includes(k))) return;
    if (keys.length) this.claims[key] = keys; else delete this.claims[key];
    await this.kv.put('claims', this.claims);
    this.on('nicks', { key });
    // A device of ours now says the same of us: bring it into our private rooms.
    if (this.myLinks.includes(key) && keys.includes(this.me.key)) this.bringIn(key);
  }

  /// A code for linking another device to this one: shown here, typed there
  /// within ten minutes. It never crosses the den in the clear.
  startLink() {
    const abc = 'abcdefghjkmnpqrstuvwxyz23456789';
    const code = [...rand(10)].map((x) => abc[x % abc.length]).join('');
    this.linkCode = { code, until: Date.now() + 10 * 60_000 };
    return `${code.slice(0, 5)}-${code.slice(5)}`;
  }

  /// On the other device: the code from the first. The rooms hear a hash of
  /// it with this device's key, which only the first device can check.
  async useLink(typed) {
    const code = String(typed || '').toLowerCase().replace(/[^a-z0-9]/g, '');
    if (code.length !== 10) throw new Error('A link code is ten letters and digits, as the other device shows it.');
    this.linkAsk = { code, until: Date.now() + 10 * 60_000 };
    const tag = await sha(`den-link-ask|${code}|${this.me.key}`);
    const rooms = this.den.rooms().filter((r) => this.rooms.get(r)?.kind !== 'private');
    if (!rooms.length) throw new Error('Enter a room first.');
    for (const room of rooms) await this.send(room, { t: 'link-ask', tag }).catch(() => {});
  }

  /// A link code's hash from another device (on the first), or the first
  /// device's answer (on the other). Checked here, inside the room's lock;
  /// what it leads to is sent after it, outside.
  async onLink(room, m) {
    const later = (f) => setTimeout(() => f().catch(() => {}), 0);
    if (m.t === 'link-ask' && this.linkCode && Date.now() < this.linkCode.until && typeof m.tag === 'string') {
      const { code } = this.linkCode;
      if (m.tag !== await sha(`den-link-ask|${code}|${m.key}`)) return;
      this.linkCode = null;
      const tag = await sha(`den-link-ok|${code}|${this.me.key}|${m.key}`);
      later(async () => {
        await this.send(room, { t: 'link', to: m.key, tag });
        await this.claim(m.key);
        this.on('linked', { key: m.key });
      });
    }
    if (m.t === 'link' && m.to === this.me.key && this.linkAsk && Date.now() < this.linkAsk.until && typeof m.tag === 'string') {
      const { code } = this.linkAsk;
      if (m.tag !== await sha(`den-link-ok|${code}|${m.key}|${this.me.key}`)) return;
      this.linkAsk = null;
      later(async () => {
        await this.claim(m.key);
        // One person, one name: the first device's.
        if (!this.me.nick && this.nickFor(m.key)) await this.setNick(this.nickFor(m.key));
        this.on('linked', { key: m.key });
      });
    }
  }

  /// Say, in every room, that `key` is this member's too.
  async claim(key) {
    if (!this.myLinks.includes(key)) this.myLinks = [...this.myLinks, key].slice(-10);
    await this.kv.put('links', this.myLinks);
    this.on('nicks', { key });
    for (const room of this.den.rooms()) await this.send(room, this.profile()).catch(() => {});
    if ((this.claims[key] || []).includes(this.me.key)) this.bringIn(key);
  }

  /// No longer this member's: said in every room.
  async unlink(key) {
    this.myLinks = this.myLinks.filter((k) => k !== key);
    await this.kv.put('links', this.myLinks);
    this.on('nicks', { key });
    for (const room of this.den.rooms()) await this.send(room, this.profile()).catch(() => {});
  }

  /// Invite a device linked to this one into this device's private rooms it
  /// is not in yet (its member then joins or declines each, there).
  async bringIn(key) {
    for (const r of [...this.rooms.values()]) {
      if (r.kind !== 'private' || !this.inRoom(r.id) || this.members(r.id).some((m) => m.key === key)) continue;
      await this.lock(() => this.invite(r.id, [key])).catch(() => {});
    }
  }

  /// A member's name as sent from their key: the newest by their clock wins.
  async learnNick(key, nick, at) {
    const name = nickOf(nick);
    const when = sent(at);
    const had = this.nicks[key];
    if (had && had.at >= when) return;
    if ((had?.name || '') === name) { if (had) had.at = when; return; }
    if (name) this.nicks[key] = { name, at: when }; else delete this.nicks[key];
    await this.kv.put('nicks', this.nicks);
    this.on('nicks', { key, name });
  }

  /// Give this device's member a name (or none), and tell every room it is in.
  async setNick(nick) {
    const name = nickOf(nick);
    if (String(nick || '').trim() && !name) throw new Error('A name is one line of at most 32 characters, and not one that starts like a device name (m- and digits).');
    this.me.nick = name;
    await this.kv.put('nick', name);
    this.on('nicks', { key: this.me.key, name });
    for (const room of this.den.rooms()) await this.send(room, this.profile()).catch(() => {});
  }

  /// What this device says of itself in a room: its member's name, and the
  /// devices it says are the same member's.
  profile() {
    return { t: 'profile', nick: this.me.nick, links: this.myLinks };
  }

  /// A room's newcomers hear this device's name a little after they arrive,
  /// once however many arrive together.
  soonProfile(room) {
    (this.profileSoon ||= {});
    if (this.profileSoon[room]) return;
    this.profileSoon[room] = setTimeout(() => {
      delete this.profileSoon[room];
      if (this.inRoom(room) && (this.me.nick || this.myLinks.length)) this.send(room, this.profile()).catch(() => {});
    }, 2000 + Math.random() * 8000);
  }

  // ---- Unread ----------------------------------------------------------------

  async unread(room) {
    const last = this.read[room] || 0;
    // Words and files only: a line saying someone joined is not a message.
    return (await this.log(room)).filter((m) => !m.mine && m.t !== 'note' && m.seq > last).length;
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

  /// One member's voice key for this epoch: every member derives it alike,
  /// and it opens only frames sealed by that member.
  async voiceKey(room, sender) {
    const raw = this.den.export(room, `${VOICE_LABEL}|${sender}`, 32);
    return crypto.subtle.importKey('raw', raw, 'AES-GCM', false, ['encrypt', 'decrypt']);
  }

  /// Join a room's voice: one connection to the den, our microphone up,
  /// eight slots down, every frame sealed here and opened here. By WebRTC;
  /// where the network lets none through, the same sealed frames go over the
  /// den's own connection (`mode`: 'auto', 'rtc' or 'relay').
  async joinVoice(room, onSlots, { mode = 'auto' } = {}) {
    if (!this.inRoom(room)) throw new Error('Enter the room first.');
    const voice = new Voice(this, room, onSlots);
    await voice.start(mode);
    this.voice = voice;
    return voice;
  }

  /// Forget this device: its key, pass, rooms, invitations, session,
  /// tokens, pinned door keys and words, by deleting its database.
  async forget() {
    try { this.socket?.disconnect(); } catch { /* not connected */ }
    await this.kv.wipe();
  }

  /// Whether this device keeps its pass code, so it can come back by itself
  /// (on unless its member turned it off).
  async keepsPass() {
    return (await this.kv.get('keep_pass')) !== false;
  }

  async setKeepPass(on) {
    await this.kv.put('keep_pass', !!on);
    if (!on) await this.kv.del('pass');
  }
}

// ---------------------------------------------------------------------------
// Voice: WebRTC to the den's forwarding unit; each audio frame sealed in a
// worker (den-voice-worker.js) under its sender's key for the epoch.

const supportsScript = typeof RTCRtpScriptTransform !== 'undefined';
const supportsStreams = typeof RTCRtpSender !== 'undefined' && 'createEncodedStreams' in RTCRtpSender.prototype;
const VOICE_BITRATE = 64_000;

class Voice {
  constructor(client, room, onSlots) {
    Object.assign(this, { client, room, onSlots: onSlots || (() => {}), keys: new Map(), people: [], roster: [], sealed: 0, opened: 0, failed: 0, waiting: new Map(), asked: 0 });
  }

  /// Keys for us and everyone in the voice room (not only those in a slot
  /// now: the den moves speakers between slots), for this epoch. The previous
  /// epoch's are kept a few seconds only, for frames still in flight: a member
  /// the new epoch took out is not heard for long after.
  async refreshKey() {
    const index = Number(this.client.den.epoch(this.room)) % 256;
    const me = this.client.den.key();
    const ks = index === this.current ? this.keys.get(index) : new Map();
    for (const k of new Set([me, ...this.roster, ...this.people.map((p) => p.key)])) {
      if (!ks.has(k)) ks.set(k, await this.client.voiceKey(this.room, k));
    }
    this.keys.delete(index);
    this.keys.set(index, ks);
    this.current = index;
    while (this.keys.size > 2) this.keys.delete(this.keys.keys().next().value);
    this.worker.postMessage({ keys: [...this.keys].map(([i, m]) => [i, [...m]]), current: index, me });
    clearTimeout(this.dropOld);
    this.dropOld = setTimeout(() => {
      for (const i of [...this.keys.keys()]) if (i !== this.current) this.keys.delete(i);
      this.worker.postMessage({ keys: [...this.keys].map(([i, m]) => [i, [...m]]), current: this.current, me });
    }, 5000);
  }

  /// Which member speaks in which down slot, as the den names them.
  postSlots() {
    // Only the room's members, as this device's own MLS group has them: a
    // slot the den names for anyone else stays silent.
    const members = new Set(this.client.members(this.room).map((m) => m.key));
    const byMid = new Map(this.people.filter((p) => members.has(p.key)).map((p) => [p.mid, p.key]));
    if (this.mode === 'relay') {
      // Relayed slots are w0…w7; a slot that changed speaker starts afresh.
      const slots = [...byMid].map(([mid, key]) => [Number(mid.slice(1)), key]).filter(([i]) => Number.isInteger(i) && i >= 0 && i < 8);
      for (const [i, key] of slots) if (this.carried?.get(i) !== key) this.resetSlot(i);
      this.carried = new Map(slots);
      this.worker.postMessage({ slots });
      return;
    }
    if (!this.downs) return;
    this.worker.postMessage({ slots: this.downs.map((t, i) => [i, byMid.get(t.mid)]).filter(([, k]) => k) });
  }

  transform(endpoint, mode, slot) {
    if (supportsScript) {
      endpoint.transform = new RTCRtpScriptTransform(this.worker, { mode, slot });
      return;
    }
    const { readable, writable } = endpoint.createEncodedStreams();
    this.worker.postMessage({ readable, writable, mode, slot }, [readable, writable]);
  }

  /// One seal or open in the worker, by message (relayed voice).
  ask(op, data, slot) {
    const id = ++this.asked;
    return new Promise((resolve) => {
      this.waiting.set(id, resolve);
      this.worker.postMessage({ op, id, data, slot }, [data]);
    });
  }

  async start(mode = 'auto') {
    const rtc = supportsScript || supportsStreams;
    const relay = typeof AudioEncoder !== 'undefined' && typeof AudioDecoder !== 'undefined' && typeof AudioWorkletNode !== 'undefined';
    if (!rtc && !relay) throw new Error('This browser cannot encrypt voice.');
    if (!rtc) mode = 'relay';
    if (!relay && mode === 'relay') throw new Error('This browser cannot send voice through the den; try another browser or network.');
    this.worker = new Worker(new URL('./den/voice-worker.js', import.meta.url));
    this.worker.onmessage = (e) => {
      if (e.data.count) Object.assign(this, { sealed: e.data.sealed, opened: e.data.opened, failed: e.data.failed });
      if (e.data.id) { this.waiting.get(e.data.id)?.(e.data.data); this.waiting.delete(e.data.id); }
    };
    await this.refreshKey();
    this.mic = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true }, video: false })
      .catch((e) => { throw new Error(e?.name === 'NotAllowedError' ? 'The microphone is blocked for this page.' : 'No microphone found.'); });
    const ch = this.client.socket.channel(`voice:${this.room}`);
    this.ch = ch;
    ch.on('ice', (c) => this.pc?.addIceCandidate(c).catch(() => {}));
    // Everyone in the voice room; the slots carry the eight heard most recently.
    ch.on('roster', ({ keys }) => {
      const members = new Set(this.client.members(this.room).map((m) => m.key));
      this.roster = (Array.isArray(keys) ? keys : []).filter((k) => typeof k === 'string' && members.has(k) && k !== this.client.den.key()).slice(0, 100);
      this.refreshKey().catch(() => {});
      this.onSlots(this.people);
    });
    ch.on('slots', (slots) => {
      this.people = Object.entries(slots).map(([mid, key]) => ({ mid, key }));
      this.refreshKey().then(() => this.postSlots()).catch(() => {});
      this.onSlots(this.people);
    });
    ch.on('f', (buf) => this.heard(buf));
    await new Promise((resolve, reject) => ch.join().receive('ok', resolve).receive('error', (e) => reject(new Error(e?.reason || 'could not join the voice room'))));
    if (mode !== 'relay') {
      await this.startRtc();
      // WebRTC needs UDP to the den. Where none gets through (a strict
      // office or hotel network), the same sealed frames go by the den's own
      // connection instead.
      if (mode === 'rtc' || !relay || await this.connected(8000)) return this.watchEpochs();
      await this.stopRtc();
    }
    await this.startRelay();
    this.watchEpochs();
  }

  // A new epoch (someone joined or left the room): a new voice key.
  watchEpochs() {
    this.offMembers = setInterval(() => {
      const e = Number(this.client.den.epoch(this.room)) % 256;
      if (e !== this.current) this.refreshKey().catch(() => {});
    }, 1000);
  }

  connected(ms) {
    return new Promise((resolve) => {
      const done = (v) => { clearTimeout(t); this.pc.removeEventListener('connectionstatechange', check); resolve(v); };
      const check = () => {
        if (this.pc.connectionState === 'connected') done(true);
        if (['failed', 'closed'].includes(this.pc.connectionState)) done(false);
      };
      const t = setTimeout(() => done(this.pc.connectionState === 'connected'), ms);
      this.pc.addEventListener('connectionstatechange', check);
      check();
    });
  }

  async startRtc() {
    this.mode = 'rtc';
    const pc = new RTCPeerConnection({ encodedInsertableStreams: !supportsScript, bundlePolicy: 'max-bundle' });
    this.pc = pc;
    const up = pc.addTransceiver(this.mic.getAudioTracks()[0], { direction: 'sendonly' });
    this.transform(up.sender, 'seal');
    this.downs = [];
    for (let i = 0; i < 8; i++) this.downs.push(pc.addTransceiver('audio', { direction: 'recvonly' }));
    this.downs.forEach((t, i) => this.transform(t.receiver, 'open', i));
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
    pc.onicecandidate = (e) => { if (e.candidate) this.ch.push('ice', e.candidate.toJSON()); };
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    const answer = await this.client.push(this.ch, 'offer', { type: 'offer', sdp: offer.sdp });
    await pc.setRemoteDescription(answer);
    this.postSlots();
    const p = up.sender.getParameters();
    p.encodings = p.encodings?.length ? p.encodings : [{}];
    p.encodings[0].maxBitrate = VOICE_BITRATE;
    await up.sender.setParameters(p).catch(() => {});
  }

  async stopRtc() {
    try { await this.client.push(this.ch, 'leave', {}); } catch { /* gone */ }
    this.pc.close();
    this.pc = null;
    this.downs = null;
    (this.audios || []).forEach((a) => { a.srcObject = null; });
    this.audios = [];
    this.meters?.clear();
  }

  /// Voice through the den's own connection: the microphone encoded here
  /// (Opus, WebCodecs), each 20 ms frame sealed as WebRTC's would be, up the
  /// channel; each slot's frames down the channel, opened, decoded, played.
  async startRelay() {
    this.mode = 'relay';
    // The worklet frames 20 ms at 48 kHz: a context at another rate (one made
    // for WebRTC's meters) is replaced.
    if (this.ac?.sampleRate !== 48000) { this.ac?.close().catch(() => {}); this.ac = new AudioContext({ sampleRate: 48000 }); }
    await this.ac.audioWorklet.addModule(new URL('./den/voice-io.js', import.meta.url));
    this.io = new AudioWorkletNode(this.ac, 'den-voice-io', { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1] });
    this.ac.createMediaStreamSource(this.mic).connect(this.io);
    this.io.connect(this.ac.destination);
    this.ac.resume?.().catch(() => {});
    this.relayLevels = {};
    this.decoders = new Map();
    this.enc = new AudioEncoder({ output: (chunk) => this.say(chunk), error: () => { this.failed++; } });
    this.enc.configure({ codec: 'opus', sampleRate: 48000, numberOfChannels: 1, bitrate: VOICE_BITRATE, opus: { frameDuration: 20000 } });
    let at = 0;
    this.io.port.onmessage = (e) => {
      const pcm = e.data;
      this.level = rms(pcm);
      if (this.muted) return;
      this.enc.encode(new AudioData({ format: 'f32-planar', sampleRate: 48000, numberOfFrames: pcm.length, numberOfChannels: 1, timestamp: at, data: pcm }));
      at += 20_000;
    };
    await this.client.push(this.ch, 'relay_join', {});
    this.postSlots();
  }

  /// One encoded frame of ours: sealed, then up the channel after its level
  /// (as -dBov, what WebRTC tells the den in each packet's header).
  async say(chunk) {
    const bytes = new Uint8Array(chunk.byteLength);
    chunk.copyTo(bytes);
    const sealed = await this.ask('seal', bytes.buffer);
    if (!sealed || !this.ch) return;
    const out = new Uint8Array(sealed.byteLength + 1);
    out[0] = Math.max(0, Math.min(127, Math.round(-20 * Math.log10(this.level || 1e-7))));
    out.set(new Uint8Array(sealed), 1);
    this.ch.push('frame', out.buffer);
  }

  /// One frame down the channel: its slot's number, then the sealed frame.
  async heard(buf) {
    if (this.mode !== 'relay' || !(buf instanceof ArrayBuffer) || buf.byteLength < 2) return;
    const slot = new Uint8Array(buf)[0];
    const plain = await this.ask('open', buf.slice(1), slot);
    if (!plain) return;
    let dec = this.decoders.get(slot);
    if (!dec) {
      dec = new AudioDecoder({
        output: (data) => {
          const pcm = new Float32Array(data.numberOfFrames);
          data.copyTo(pcm, { planeIndex: 0 });
          data.close();
          this.relayLevels[`w${slot}`] = rms(pcm);
          this.io.port.postMessage({ slot, pcm }, [pcm.buffer]);
        },
        error: () => { this.decoders.delete(slot); },
      });
      dec.configure({ codec: 'opus', sampleRate: 48000, numberOfChannels: 1 });
      dec.at = 0;
      this.decoders.set(slot, dec);
    }
    dec.decode(new EncodedAudioChunk({ type: 'key', timestamp: (dec.at += 20_000), data: plain }));
  }

  resetSlot(slot) {
    const dec = this.decoders?.get(slot);
    if (dec) { try { dec.close(); } catch { /* closed */ } this.decoders.delete(slot); }
    this.io?.port.postMessage({ slot, reset: true });
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
    if (this.mode === 'relay') {
      const out = { ...this.relayLevels, me: this.muted ? 0 : this.level || 0 };
      for (const k of Object.keys(this.relayLevels)) this.relayLevels[k] *= 0.5;
      return out;
    }
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
    clearTimeout(this.dropOld);
    try { await this.client.push(this.ch, 'leave', {}); } catch { /* gone */ }
    this.ch.leave();
    this.pc?.close();
    try { this.enc?.close(); } catch { /* closed */ }
    for (const d of this.decoders?.values() || []) { try { d.close(); } catch { /* closed */ } }
    this.mic.getTracks().forEach((t) => t.stop());
    (this.audios || []).forEach((a) => { a.srcObject = null; });
    this.worker?.terminate();
    this.ac?.close().catch(() => {});
    this.client.voice = null;
  }
}

/// Root mean square of some samples: 0 (silence) to 1.
function rms(pcm) {
  let sum = 0;
  for (let i = 0; i < pcm.length; i++) sum += pcm[i] * pcm[i];
  return Math.sqrt(sum / (pcm.length || 1));
}
