// The den's page: rooms, messages, files, private rooms, voice.
// Everything here works on what den.js keeps on this device; nothing is
// fetched in the clear but the den's ciphertext and the day keys.

import { REACTIONS as EMOJI } from './den.js';
const GROUP_MS = 5 * 60_000;
// One line of text with nothing hidden in it: no bidirectional or zero-width
// controls (which can make a name or a file name read backwards), and
// whitespace collapsed, for names, titles and notifications.
export const plain = (s) => String(s ?? '').replace(/[\u200B-\u200F\u202A-\u202E\u2060-\u2069\uFEFF\u061C]/g, '').replace(/\s+/g, ' ').trim();
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const time = (t) => new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
const size = (n) => (n > 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);
const dayKey = (t) => new Date(t).toDateString();
function dayName(t) {
  const d = new Date(t);
  const today = new Date();
  const yesterday = new Date(Date.now() - 86_400_000);
  if (d.toDateString() === today.toDateString()) return 'Today';
  if (d.toDateString() === yesterday.toDateString()) return 'Yesterday';
  return d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short', ...(d.getFullYear() !== today.getFullYear() ? { year: 'numeric' } : {}) });
}

// A device's mark: a 5×5 mirrored pixel pattern from its key, so the same
// device looks the same to everyone, without a name or picture to go on.
const INKS = ['#c7f04b', '#f4b728', '#7fd1b9', '#e58a6b', '#b9a7e8', '#e9e6dc'];
// Shown in place: raster images of at most 5 MB (never SVG); den.js already
// turns any other type into a download.
const INLINE = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif']);
const inlineImage = (f) => INLINE.has(String(f?.mime).toLowerCase()) && f.size <= 5 * 1048576;

/// A 64-bit FNV-1a over the whole key, hex: a mark depends on every byte of
/// it, not on the part a name shows.
function spread(key) {
  let h = 0xcbf29ce484222325n;
  for (const c of String(key || '')) h = BigInt.asUintN(64, (h ^ BigInt(c.charCodeAt(0))) * 0x100000001b3n);
  return h.toString(16).padStart(16, '0');
}

export function mark(key, px = 28) {
  const h = spread(key);
  const ink = INKS[parseInt(h.slice(0, 2), 16) % INKS.length];
  let bits = parseInt(h.slice(2, 6), 16) & 0x7fff;
  // Too few cells reads as nothing: take the other half instead.
  if (bits.toString(2).replace(/0/g, '').length < 6) bits ^= 0x7fff;
  let rects = '';
  for (let i = 0; i < 15; i++) {
    if (!((bits >> i) & 1)) continue;
    const x = Math.floor(i / 5), y = i % 5;
    rects += `<rect x="${x}" y="${y}" width="1" height="1"/>`;
    if (x < 2) rects += `<rect x="${4 - x}" y="${y}" width="1" height="1"/>`;
  }
  return `<svg class="mark" width="${px}" height="${px}" viewBox="-1 -1 7 7" aria-hidden="true" shape-rendering="crispEdges"><rect x="-1" y="-1" width="7" height="7" fill="rgba(0,0,0,.55)"/><g fill="${ink}">${rects}</g></svg>`;
}

/// A longer look at a key than its name: 80 bits, in groups, for telling
/// apart two devices whose short names happen to match.
const fingerprint = (key) => (/^[0-9a-f]{64}$/.test(String(key)) ? `Key ${key.match(/.{4}/g).join(' ')}` : '');

const ICON = {
  reply: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3 2 7l4 4M2 7h7a5 5 0 0 1 5 5v1" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>',
  react: '<svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M5.5 9.5c1.3 1.6 3.7 1.6 5 0" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M6 6h.01M10 6h.01" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  edit: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="m3 13 1-4 7-7 3 3-7 7-4 1Z" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>',
  del: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4h10M6 4V2.5h4V4M4.5 4l.7 9.5h5.6L11.5 4" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>',
  file: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 1.5h5l3.5 3.5v9.5H4z M9 1.5V5h3.5" fill="none" stroke="currentColor" stroke-width="1.3"/></svg>',
};

export function mountDen(den, el, me) {
  let room = 'general';
  let reply = null;
  let editing = null;
  let readFrom = 0;
  let mentionAt = -1;
  let mentionPick = 0;
  let notify = (() => { try { return localStorage.getItem('den.notify') || 'mentions'; } catch { return 'mentions'; } })();
  let propsOpen = (() => { try { return localStorage.getItem('den.props') === 'open'; } catch { return false; } })();
  const counts = {};
  const typing = {};
  const urls = new Map();
  // Rooms this device hears nothing from (kept in this browser).
  const muted = new Set((() => { try { return JSON.parse(localStorage.getItem('den.muted') || '[]'); } catch { return []; } })());
  const keepMuted = () => { try { localStorage.setItem('den.muted', JSON.stringify([...muted])); } catch { /* not kept */ } };
  const shown = [];

  const rooms = () => [...den.rooms.values()];
  /// A room's name; a private room of two is named after the other person,
  /// on both sides.
  const title = (r) => {
    if (r?.kind !== 'private') return r?.title || r?.id || '';
    const others = den.inRoom(r.id) ? den.members(r.id).filter((m) => m.key !== me.key) : [];
    if (others.length === 1) return others[0].name;
    return r.title || others.map((m) => m.name).join(', ') || 'Private room';
  };
  const roomTitle = (id) => title(den.rooms.get(id) || { id });
  let statusTimer;
  const status = (t, sticky = false) => {
    el('den-status').textContent = t || '';
    clearTimeout(statusTimer);
    if (t && !sticky) statusTimer = setTimeout(() => { el('den-status').textContent = ''; }, 6000);
  };

  el('me').textContent = me.name;
  el('me-mark').innerHTML = mark(me.key, 32);

  // The den fills the window below the site's bar; only the log scrolls.
  document.querySelector('main.register')?.classList.add('wide');
  const box = el('den');
  const fit = () => {
    const top = box.getBoundingClientRect().top + window.scrollY;
    box.style.height = `${Math.max(480, window.innerHeight - top - 16)}px`;
  };
  fit();
  window.addEventListener('resize', fit);

  // ---- The room list --------------------------------------------------------

  async function drawRooms() {
    const groups = [['open', 'Rooms', 'New room'], ['proposal', 'Proposals', null], ['private', 'Private', 'New private room']];
    const parts = [];
    // Invitations to private rooms wait here until this device joins or declines.
    const invites = await den.invites();
    if (invites.length) {
      parts.push(`<div class="group invites"><p class="group-head"><span>Invitations · ${invites.length}</span></p>
        ${invites.map((i) => `<div class="invite"><span>A private room</span><button type="button" class="mini" data-join="${esc(i.id)}">Join</button><button type="button" class="mini ghost" data-decline="${esc(i.id)}">Decline</button></div>`).join('')}
      </div>`);
    }
    for (const [kind, label, add] of groups) {
      const list = rooms().filter((r) => r.kind === kind);
      if (!list.length && kind === 'proposal') continue;
      // Proposals by number, the newest first.
      if (kind === 'proposal') list.sort((x, y) => Number(y.id.split('-')[1]) - Number(x.id.split('-')[1]));
      const items = [];
      let unreadHere = 0;
      for (const r of list) {
        const n = await den.unread(r.id);
        unreadHere += n;
        const voiceHere = den.voice?.room === r.id;
        items.push(`<button type="button" class="room${n ? ' unread' : ''}" data-room="${esc(r.id)}" aria-current="${r.id === room}"><span class="room-name">${esc(title(r))}</span>${voiceHere ? '<span class="live-dot" title="You are in voice here"></span>' : ''}${n ? `<span class="badge" aria-label="${n} unread">${n > 99 ? '99+' : n}</span>` : ''}</button>`);
      }
      const collapsible = kind === 'proposal';
      const open = !collapsible || propsOpen || list.some((r) => r.id === room);
      parts.push(`<div class="group">
        ${collapsible
          ? `<button type="button" class="group-head" data-toggle="props" aria-expanded="${open}"><span>${label} · ${list.length}</span>${!open && unreadHere ? `<span class="badge">${unreadHere}</span>` : ''}<svg class="caret" viewBox="0 0 10 10" aria-hidden="true"><path d="M3 1.5 6.5 5 3 8.5" fill="none" stroke="currentColor" stroke-width="1.4"/></svg></button>`
          : `<p class="group-head"><span>${label}</span>${add ? `<button type="button" class="add" data-new="${kind}" aria-label="${add}" title="${add}">+</button>` : ''}</p>`}
        ${open ? items.join('') || '<p class="none">None yet.</p>' : ''}
      </div>`);
    }
    el('rooms').innerHTML = parts.join('');
    el('rooms').querySelectorAll('[data-room]').forEach((b) => { b.onclick = () => go(b.dataset.room); });
    el('rooms').querySelectorAll('[data-join]').forEach((b) => { b.onclick = async () => { b.disabled = true; const r = await den.joinInvite(b.dataset.join); drawRooms(); if (r) go(r); }; });
    el('rooms').querySelectorAll('[data-decline]').forEach((b) => { b.onclick = async () => { await den.declineInvite(b.dataset.decline); drawRooms(); }; });
    el('rooms').querySelectorAll('[data-new]').forEach((b) => { b.onclick = () => (b.dataset.new === 'open' ? newRoom() : pickMembers('New private room', 'Make room', true, (keys, name) => den.startPrivate(keys, name).then(go))); });
    const t = el('rooms').querySelector('[data-toggle]');
    if (t) t.onclick = () => { propsOpen = !propsOpen; try { localStorage.setItem('den.props', propsOpen ? 'open' : 'closed'); } catch { /* not kept */ } drawRooms(); };
    drawHead();
  }

  function drawHead() {
    const r = den.rooms.get(room);
    el('room-title').textContent = title(r) || room;
    el('room-title').classList.toggle('plain', r?.kind === 'private');
    el('room-kind').textContent = r?.kind === 'private' ? 'Private' : r?.kind === 'proposal' ? 'Proposal' : '';
    el('room-meta').textContent = counts[room] ? `Members · ${counts[room]}` : 'Members';
    el('voice-go').hidden = !den.inRoom(room) || !!den.voice;
    el('say-text').placeholder = den.inRoom(room) ? `Message ${title(r) || room}` : 'Joining the room…';
    document.title = `${title(r) || 'Chat'} · Members' chat`;
    // Mute (no notifications from the room) and, for a private room, Leave.
    const acts = document.querySelector('.head-acts');
    if (acts && !el('room-mute')) {
      acts.insertAdjacentHTML('beforeend', '<button type="button" class="pill" id="room-mute" aria-pressed="false">Mute</button><button type="button" class="pill" id="room-leave" hidden>Leave</button>');
      el('room-mute').onclick = () => { muted.has(room) ? muted.delete(room) : muted.add(room); keepMuted(); drawHead(); };
      el('room-leave').onclick = async () => {
        if (!(await ask({ heading: 'Leave this room?', text: 'This device forgets the room and its words. A member can invite it again.', yes: 'Leave', danger: true }))) return;
        const left = room;
        await den.leave(left);
        go('general');
      };
    }
    if (el('room-mute')) {
      el('room-mute').textContent = muted.has(room) ? 'Unmute' : 'Mute';
      el('room-mute').setAttribute('aria-pressed', String(muted.has(room)));
    }
    if (el('room-leave')) el('room-leave').hidden = r?.kind !== 'private';
  }

  function go(id, focus = null) {
    if (id !== room) { forgetImages(); drawn = 300; }
    room = id;
    reply = null;
    editing = null;
    el('say-text').value = '';
    el('den').classList.remove('drawer');
    closeMentions();
    drawRooms();
    drawLog(focus);
    drawComposer();
    if (!el('members').hidden) drawMembers();
  }

  // ---- Dialogs ----------------------------------------------------------------

  /// A small form in a dialog: a heading, a line, an optional field; resolves
  /// with the field's value (or true), or null when closed.
  function ask({ heading, text = '', field = null, yes, danger = false }) {
    const d = el('ask');
    d.querySelector('h2').textContent = heading;
    d.querySelector('.ask-text').textContent = text;
    d.querySelector('.ask-text').hidden = !text;
    const label = d.querySelector('.ask-field');
    label.hidden = !field;
    const input = label.querySelector('input');
    if (field) { label.querySelector('span').textContent = field; input.value = ''; }
    const ok = d.querySelector('[data-yes]');
    ok.textContent = yes;
    ok.classList.toggle('danger', danger);
    d.showModal();
    if (field) input.focus(); else ok.focus();
    return new Promise((resolve) => {
      let answer = null;
      d.querySelector('form').onsubmit = (e) => {
        e.preventDefault();
        const v = field ? input.value.trim() : true;
        if (field && !v) return;
        answer = v;
        d.close();
      };
      d.querySelector('[data-close]').onclick = () => d.close();
      d.onclose = () => resolve(answer);
    });
  }

  async function newRoom() {
    const name = await ask({ heading: 'New room', field: 'Name (everyone in the den sees it)', yes: 'Make room' });
    if (!name) return;
    try {
      const r = await den.push(den.lobby, 'open_room', { title: name });
      status(`Opening ${name}…`);
      setTimeout(() => go(r.id), 600);
    } catch (e) { status(e.message); }
  }

  /// Every device this one knows of, from the rooms it is in.
  function known() {
    const all = new Map();
    for (const r of rooms()) for (const m of den.members(r.id)) if (m.key !== me.key) all.set(m.key, m.name);
    return [...all.entries()].map(([key, name]) => ({ key, name })).sort((a, b) => a.name.localeCompare(b.name));
  }
  // A device's name is its key's first ten hex digits (as the core names it).
  const nameOf = (key) => (/^[0-9a-f]{64}$/.test(String(key)) ? `m-${key.slice(0, 20)}` : 'a member');

  function pickMembers(heading, yes, named, done) {
    const already = named ? new Set() : new Set(den.members(room).map((m) => m.key));
    const list = known().filter((m) => !already.has(m.key));
    const d = el('pick');
    d.querySelector('h2').textContent = heading;
    d.querySelector('.who').innerHTML = list.length
      ? list.map((m) => `<label class="pick-row"><input type="checkbox" value="${esc(m.key)}" data-name="${esc(m.name)}" />${mark(m.key, 24)}<span>${esc(m.name)}</span></label>`).join('')
      : '<p class="none">No one to add yet. Members appear here once you share a room.</p>';
    d.querySelector('.pick-name').hidden = !named;
    d.querySelector('[name=title]').value = '';
    d.querySelector('[type=submit]').textContent = yes;
    d.showModal();
    d.querySelector('form').onsubmit = async (e) => {
      e.preventDefault();
      const picked = [...d.querySelectorAll('input[type=checkbox]:checked')];
      if (!picked.length) return;
      // No name given: each member sees the others' names.
      const name = d.querySelector('[name=title]').value.trim();
      d.close();
      status('Sending invitations…', true);
      try { await done(picked.map((x) => x.value), name); status('Invitations sent.'); } catch (err) { status(err.message); }
    };
    d.querySelector('[data-close]').onclick = () => d.close();
  }

  // ---- Messages -------------------------------------------------------------

  // One pass over the words: code, links, mentions and line breaks are each
  // found once, in order, so none can land inside another's markup.
  const TOKENS = /`([^`\n]{1,200})`|(\bhttps?:\/\/[^\s<>"]+[^\s<>".,;:!?)\]'])|@(m-[0-9a-f]{20})|(\n)/g;
  function words(text) {
    let out = '';
    let last = 0;
    for (const t of String(text).matchAll(TOKENS)) {
      out += esc(text.slice(last, t.index));
      if (t[1] !== undefined) out += `<code>${esc(t[1])}</code>`;
      else if (t[2] !== undefined) out += `<a href="${esc(t[2])}" target="_blank" rel="noopener noreferrer nofollow">${esc(t[2])}</a>`;
      else if (t[3] !== undefined) out += `<mark class="${t[3] === me.name ? 'me' : ''}">@${esc(t[3])}</mark>`;
      else out += '<br>';
      last = t.index + t[0].length;
    }
    return out + esc(text.slice(last));
  }

  /// One message as list items; `prev` decides the day line and whether it
  /// continues the previous author's run.
  function line(m, prev, log) {
    const parts = [];
    if (!prev || dayKey(prev.at) !== dayKey(m.at)) parts.push(`<li class="day" role="separator"><span>${dayName(m.at)}</span></li>`);
    if (readFrom && !m.mine && m.seq > readFrom && (!prev || prev.mine || prev.seq <= readFrom)) parts.push('<li class="new-line" role="separator"><span>New</span></li>');
    // What happened to the room itself: who joined, who left, who named it.
    if (m.t === 'note') {
      parts.push(`<li class="note" data-id="${esc(m.id)}"><span>${esc(m.text)}</span> <time datetime="${new Date(m.at).toISOString()}">${time(m.at)}</time></li>`);
      return parts.join('');
    }
    const cont = prev && !m.reply && prev.key === m.key && !prev.deleted && m.at - prev.at < GROUP_MS && !parts.length;
    const own = m.key === me.key;
    const head = `<p class="msg-head"><b class="who" title="${esc(fingerprint(m.key))}">${esc(m.from)}</b>${own ? '<span class="you">you</span>' : ''}<time datetime="${new Date(m.at).toISOString()}" title="${esc(new Date(m.at).toLocaleString())}">${time(m.at)}</time>${m.edited && !m.deleted ? '<span class="edited">edited</span>' : ''}</p>`;
    const gutter = cont ? `<span class="side-time">${time(m.at)}</span>` : mark(m.key);
    if (m.deleted) {
      parts.push(`<li class="msg gone${cont ? ' cont' : ''}" data-id="${esc(m.id)}">${gutter}<div class="msg-main">${cont ? '' : head}<p class="body"><em>Message deleted</em></p></div></li>`);
      return parts.join('');
    }
    const quoted = m.reply && log.find((x) => x.id === m.reply);
    const q = m.reply ? `<button type="button" class="quote" data-jump="${esc(m.reply)}">${quoted ? `<b>${esc(quoted.from)}</b> ${esc(quoted.deleted ? 'Message deleted' : (quoted.text || quoted.file?.name || '').slice(0, 140))}` : 'An earlier message'}</button>` : '';
    let body = `<div class="body">${words(m.text)}${m.edited && cont ? ' <span class="edited">edited</span>' : ''}</div>`;
    if (m.file) {
      const img = inlineImage(m.file);
      body = img
        ? `<button type="button" class="att" data-view="${esc(m.id)}" aria-label="Open ${esc(m.file.name)}"><img data-blob="${esc(m.id)}" alt="${esc(m.file.name)}" /></button>`
        : `<button type="button" class="file" data-file="${esc(m.id)}">${ICON.file}<span><b>${esc(m.file.name)}</b><small>${size(m.file.size)} · Download</small></span></button>`;
    }
    // Reactions are kept by the reactor's key; shown by name.
    const reactions = Object.entries(m.reactions || {}).map(([e, who]) => `<button type="button" class="chip${who.includes(me.key) ? ' on' : ''}" data-react="${esc(m.id)}" data-emoji="${esc(e)}" title="${esc(who.map(nameOf).join(', '))}" aria-pressed="${who.includes(me.key)}">${esc(e)} <span>${who.length}</span></button>`).join('');
    const ping = !own && (m.text || '').includes(`@${me.name}`);
    parts.push(`<li class="msg${own ? ' mine' : ''}${ping ? ' ping' : ''}${cont ? ' cont' : ''}" data-id="${esc(m.id)}">
      ${gutter}
      <div class="msg-main">${cont ? '' : head}${q}${body}${reactions ? `<div class="chips">${reactions}</div>` : ''}</div>
      <div class="tools" role="toolbar" aria-label="Message actions">
        <button type="button" data-act="reply" aria-label="Reply" title="Reply">${ICON.reply}</button>
        <button type="button" data-act="react" aria-label="React" title="React" aria-expanded="false">${ICON.react}</button>
        ${own && !m.file ? `<button type="button" data-act="edit" aria-label="Edit" title="Edit">${ICON.edit}</button>` : ''}
        ${own ? `<button type="button" data-act="del" aria-label="Delete" title="Delete">${ICON.del}</button>` : ''}
      </div>
    </li>`);
    return parts.join('');
  }

  const nearBottom = () => { const b = el('log'); return b.scrollHeight - b.scrollTop - b.clientHeight < 120; };
  const toBottom = () => { const b = el('log'); b.scrollTop = b.scrollHeight; el('jump').hidden = true; };

  let drawn = 300;
  async function drawLog(focus = null) {
    const log = await den.log(room);
    readFrom = den.lastRead(room);
    shown.length = 0;
    shown.push(...log);
    const b = el('log');
    const priv = den.rooms.get(room)?.kind === 'private';
    // The latest messages are drawn, earlier ones on asking (or when a search
    // result or a reply points at one): a long room stays quick to open.
    const want = focus ? log.findIndex((m) => m.id === focus) : -1;
    const from = Math.max(0, Math.min(log.length - drawn, want >= 0 ? want - 20 : Infinity));
    const more = from > 0 ? `<li class="earlier"><button type="button" class="mini ghost" id="earlier">Show earlier messages</button></li>` : '';
    b.innerHTML = log.length
      ? more + log.slice(from).map((m, i) => line(m, log[from + i - 1], log)).join('')
      : `<li class="empty"><b class="${priv ? 'plain' : ''}">${esc(roomTitle(room))}</b><span>${!den.inRoom(room) && priv ? 'Waiting for the invitation to arrive.' : 'No messages yet.'}</span></li>`;
    for (const img of b.querySelectorAll('img[data-blob]')) showImage(img, log.find((m) => m.id === img.dataset.blob));
    const earlier = el('earlier');
    if (earlier) earlier.onclick = () => { drawn += 300; const first = log[from]?.id; drawLog(first); };
    if (focus) flash(focus); else toBottom();
    den.markRead(room);
    drawRooms();
  }

  /// A new message (or a change to one) in the open room, without redrawing
  /// the rest: the reader's place and hover stay put.
  async function onMessage(d) {
    const log = await den.log(room);
    const i = log.findIndex((x) => x.id === d.message.id);
    if (i < 0) return;
    const b = el('log');
    const stay = nearBottom() || d.mine;
    const old = b.querySelector(`li[data-id="${CSS.escape(d.message.id)}"]`);
    if (!old && (d.kind === 'msg' || d.kind === 'file')) {
      b.querySelector('.empty')?.remove();
      b.insertAdjacentHTML('beforeend', line(log[i], log[i - 1], log));
    } else if (old) {
      // Drop the day or "New" line drawn just before it; the redraw brings its own.
      while (old.previousElementSibling && !old.previousElementSibling.dataset.id) old.previousElementSibling.remove();
      old.insertAdjacentHTML('beforebegin', line(log[i], log[i - 1], log));
      old.remove();
    }
    const img = b.querySelector(`li[data-id="${CSS.escape(d.message.id)}"] img[data-blob]`);
    if (img) showImage(img, log[i]);
    shown.length = 0;
    shown.push(...log);
    if (stay) toBottom();
    else if (d.kind === 'msg' || d.kind === 'file') el('jump').hidden = false;
    if (!document.hidden && stay) den.markRead(room);
  }

  function flash(id) {
    const li = el('log').querySelector(`li[data-id="${CSS.escape(id)}"]`);
    if (!li) { status('That message is no longer kept on this device.'); return; }
    li.scrollIntoView({ block: 'center', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
    li.classList.remove('flash');
    void li.offsetWidth;
    li.classList.add('flash');
  }

  // An image is fetched and opened only when it comes into view, checked for
  // its real size before it is drawn (a few kilobytes can claim 40000 × 40000
  // pixels), and its decrypted copy is let go when the room is left.
  const MAX_PIXELS = 40_000_000;
  const seen = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      seen.unobserve(e.target);
      reveal(e.target, e.target._den);
    }
  }, { rootMargin: '400px 0px' });

  function showImage(img, m) {
    if (!m?.file) return;
    img._den = m;
    seen.observe(img);
  }

  async function reveal(img, m) {
    const refuse = (text) => img.closest('.att')?.replaceWith(Object.assign(document.createElement('em'), { className: 'body', textContent: text }));
    try {
      if (!urls.has(m.id)) {
        const blob = await den.openFile(m.file);
        const bmp = await createImageBitmap(blob).catch(() => null);
        if (!bmp) return refuse(`${m.file.name}: this image could not be opened.`);
        const pixels = bmp.width * bmp.height;
        bmp.close();
        if (pixels > MAX_PIXELS) return refuse(`${m.file.name} is too large to show here.`);
        urls.set(m.id, URL.createObjectURL(blob));
      }
      const stay = nearBottom();
      img.onload = () => { if (stay) toBottom(); };
      img.src = urls.get(m.id);
    } catch (e) { refuse(e.message); }
  }

  function forgetImages() {
    for (const u of urls.values()) URL.revokeObjectURL(u);
    urls.clear();
  }

  async function download(m) {
    status(`Opening ${m.file.name}…`, true);
    try {
      const url = URL.createObjectURL(await den.openFile(m.file));
      Object.assign(document.createElement('a'), { href: url, download: m.file.name }).click();
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
      status('');
    } catch (e) { status(e.message); }
  }

  function closeEmoji() {
    el('log').querySelectorAll('.emoji').forEach((x) => { x.closest('li')?.querySelector('[data-act="react"]')?.setAttribute('aria-expanded', 'false'); x.remove(); });
  }

  // One listener for the whole log: messages come and go under it.
  el('log').addEventListener('click', async (e) => {
    const t = e.target.closest('button');
    if (!t) {
      closeEmoji();
      // No hover on a touch screen: a tap on a message shows its actions.
      const tapped = e.target.closest('li.msg');
      if (tapped && !e.target.closest('a') && matchMedia('(hover: none)').matches) {
        el('log').querySelectorAll('.msg.touch').forEach((x) => { if (x !== tapped) x.classList.remove('touch'); });
        tapped.classList.toggle('touch');
      }
      return;
    }
    const li = t.closest('li[data-id]');
    const m = li && shown.find((x) => x.id === li.dataset.id);
    if (t.dataset.jump) return flash(t.dataset.jump);
    if (t.dataset.file && m) return download(m);
    if (t.dataset.view && m) return view(m);
    if (t.dataset.react) { closeEmoji(); return den.react(room, t.dataset.react, t.dataset.emoji).catch((err) => status(err.message)); }
    if (t.dataset.emoji && t.closest('.emoji') && m) { closeEmoji(); return den.react(room, m.id, t.dataset.emoji).catch((err) => status(err.message)); }
    if (!m || !t.dataset.act) return;
    if (t.dataset.act === 'reply') { reply = m; editing = null; drawComposer(); el('say-text').focus(); }
    if (t.dataset.act === 'edit') startEdit(m);
    if (t.dataset.act === 'del') {
      if (await ask({ heading: 'Delete message', text: 'It is removed for everyone in the room. Devices that already read it may keep their copy.', yes: 'Delete', danger: true })) den.remove(room, m.id).catch((err) => status(err.message));
    }
    if (t.dataset.act === 'react') {
      const open = li.querySelector('.emoji');
      closeEmoji();
      if (open) return;
      t.setAttribute('aria-expanded', 'true');
      li.querySelector('.tools').insertAdjacentHTML('beforeend', `<div class="emoji" role="menu">${EMOJI.map((x) => `<button type="button" role="menuitem" data-emoji="${x}">${x}</button>`).join('')}</div>`);
      li.querySelector('.emoji button').focus();
    }
  });
  el('log').addEventListener('scroll', () => { if (!el('jump').hidden && nearBottom()) { el('jump').hidden = true; den.markRead(room); drawRooms(); } });
  el('jump').onclick = () => { toBottom(); den.markRead(room); drawRooms(); };
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeEmoji(); });

  function view(m) {
    const d = el('lightbox');
    d.querySelector('img').src = urls.get(m.id) || '';
    d.querySelector('img').alt = m.file.name;
    d.querySelector('.lb-name').textContent = `${m.file.name} · ${size(m.file.size)}`;
    d.querySelector('[data-save]').onclick = () => download(m);
    d.showModal();
  }
  el('lightbox').addEventListener('click', (e) => { if (e.target === el('lightbox') || e.target.closest('[data-close]')) el('lightbox').close(); });

  // ---- The composer -----------------------------------------------------------

  function grow() {
    const t = el('say-text');
    t.style.height = 'auto';
    t.style.height = `${Math.min(t.scrollHeight, 200)}px`;
  }

  function startEdit(m) {
    editing = m;
    reply = null;
    el('say-text').value = m.text;
    drawComposer();
    grow();
    el('say-text').focus();
  }

  function drawComposer() {
    const banner = el('say-banner');
    banner.hidden = !reply && !editing;
    banner.innerHTML = reply
      ? `<span>Replying to <b>${esc(reply.from)}</b> <q>${esc((reply.text || reply.file?.name || '').slice(0, 90))}</q></span><button type="button" class="x" id="say-cancel" aria-label="Cancel reply">×</button>`
      : editing ? '<span>Editing your message · <kbd>Esc</kbd> to cancel</span><button type="button" class="x" id="say-cancel" aria-label="Cancel editing">×</button>' : '';
    const c = el('say-cancel');
    if (c) c.onclick = cancelCompose;
    const r = den.rooms.get(room);
    el('say').hidden = !den.inRoom(room) && r?.kind === 'private';
    el('say').querySelector('[type=submit]').textContent = editing ? 'Save' : 'Send';
    drawHead();
  }

  function cancelCompose() {
    if (editing) el('say-text').value = '';
    reply = null;
    editing = null;
    drawComposer();
    grow();
  }

  el('say').onsubmit = async (e) => {
    e.preventDefault();
    const text = el('say-text').value.trim();
    if (!text) return;
    el('say-text').value = '';
    grow();
    closeMentions();
    const was = { reply, editing };
    reply = null;
    editing = null;
    drawComposer();
    try {
      if (was.editing) await den.edit(room, was.editing.id, text);
      else await den.say(room, text, was.reply?.id || null);
    } catch (err) {
      status(err.message);
      ({ reply, editing } = was);
      el('say-text').value = text;
      drawComposer();
      grow();
    }
  };

  el('say-text').addEventListener('keydown', (e) => {
    if (!el('mentions').hidden) {
      const items = el('mentions').querySelectorAll('[data-pick]');
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); mentionPick = (mentionPick + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length; drawMentionPick(); return; }
      if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); items[mentionPick]?.click(); return; }
      if (e.key === 'Escape') { e.preventDefault(); closeMentions(); return; }
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); el('say').requestSubmit(); return; }
    if (e.key === 'Escape' && (reply || editing)) { e.preventDefault(); cancelCompose(); return; }
    if (e.key === 'ArrowUp' && !el('say-text').value) {
      const last = [...shown].reverse().find((m) => m.key === me.key && !m.deleted && !m.file);
      if (last) { e.preventDefault(); startEdit(last); }
    }
  });
  el('say-text').addEventListener('input', () => { grow(); den.typing(room); drawMentions(); });

  // @ and a few letters: the members this device knows, to pick from.
  function drawMentions() {
    const t = el('say-text');
    const before = t.value.slice(0, t.selectionStart);
    const m = before.match(/(^|\s)@([\w-]{0,12})$/);
    if (!m) return closeMentions();
    mentionAt = before.length - m[2].length - 1;
    const q = m[2].toLowerCase();
    const list = known().filter((x) => x.name.toLowerCase().includes(q)).slice(0, 6);
    if (!list.length) return closeMentions();
    mentionPick = Math.min(mentionPick, list.length - 1);
    el('mentions').innerHTML = list.map((x, i) => `<li><button type="button" role="option" data-pick="${esc(x.name)}" aria-selected="${i === mentionPick}">${mark(x.key, 20)}<span>${esc(x.name)}</span></button></li>`).join('');
    el('mentions').hidden = false;
    el('mentions').querySelectorAll('[data-pick]').forEach((b) => {
      b.onmousedown = (ev) => ev.preventDefault();
      b.onclick = () => {
        const v = t.value;
        const end = t.selectionStart;
        const ins = `@${b.dataset.pick} `;
        t.value = v.slice(0, mentionAt) + ins + v.slice(end);
        t.selectionStart = t.selectionEnd = mentionAt + ins.length;
        closeMentions();
        t.focus();
      };
    });
  }
  function drawMentionPick() { el('mentions').querySelectorAll('[data-pick]').forEach((b, i) => b.setAttribute('aria-selected', String(i === mentionPick))); }
  function closeMentions() { el('mentions').hidden = true; mentionPick = 0; }
  el('say-text').addEventListener('blur', () => setTimeout(closeMentions, 150));

  // Files: the button, a paste, or a drop anywhere on the conversation.
  async function sendFiles(files) {
    for (const f of [...files].slice(0, 5)) {
      if (f.size > 25 * 1048576) { status(`${f.name} is over 25 MB.`); continue; }
      status(`Sealing and sending ${f.name}…`, true);
      try { await den.sendFile(room, f); status(''); } catch (e) { status(e.message); }
    }
  }
  el('attach').onclick = () => el('attach-file').click();
  el('attach-file').onchange = () => { sendFiles(el('attach-file').files).finally(() => { el('attach-file').value = ''; }); };
  el('say-text').addEventListener('paste', (e) => { if (e.clipboardData?.files?.length) { e.preventDefault(); sendFiles(e.clipboardData.files); } });
  const main = el('den-main');
  let dragDepth = 0;
  const canDrop = (e) => den.inRoom(room) && [...(e.dataTransfer?.types || [])].includes('Files');
  main.addEventListener('dragenter', (e) => { if (!canDrop(e)) return; e.preventDefault(); dragDepth++; el('drop').hidden = false; });
  main.addEventListener('dragover', (e) => { if (canDrop(e)) e.preventDefault(); });
  main.addEventListener('dragleave', () => { if (--dragDepth <= 0) { dragDepth = 0; el('drop').hidden = true; } });
  main.addEventListener('drop', (e) => { if (!canDrop(e)) return; e.preventDefault(); dragDepth = 0; el('drop').hidden = true; sendFiles(e.dataTransfer.files); });

  // ---- Members ----------------------------------------------------------------

  function drawMembers() {
    const list = den.members(room).sort((a, b) => (a.key === me.key ? -1 : b.key === me.key ? 1 : a.name.localeCompare(b.name)));
    const priv = den.rooms.get(room)?.kind === 'private';
    // Two names that start alike are how one member poses as another: say so.
    const starts = new Map();
    for (const m of list) starts.set(m.name.slice(0, 8), (starts.get(m.name.slice(0, 8)) || 0) + 1);
    const alike = [...starts.values()].some((n) => n > 1);
    el('members').innerHTML = `<div class="members-head"><h3>Members · ${list.length}</h3><button type="button" class="x" data-close aria-label="Close members">×</button></div>
      ${alike ? '<p class="alike" role="note">Two members here have names that start alike. Compare whole names before you trust who is who.</p>' : ''}
      <ul>${list.map((m) => `<li>${mark(m.key, 24)}<span class="who" title="${esc(fingerprint(m.key))}">${esc(m.name)}</span>${m.key === me.key ? '<span class="you">you</span>' : `<button type="button" class="mini" data-dm="${esc(m.key)}" data-name="${esc(m.name)}">Message</button>`}</li>`).join('') || '<li class="none">Joining…</li>'}</ul>
      ${priv && den.inRoom(room) ? '<button type="button" class="act ghost small" id="invite">Invite</button>' : ''}`;
    el('members').querySelectorAll('[data-dm]').forEach((b) => {
      b.onclick = async () => {
        // Already a room of just the two of you: that one.
        const pair = rooms().find((r) => r.kind === 'private' && den.inRoom(r.id) && (() => { const ks = den.members(r.id).map((x) => x.key); return ks.length === 2 && ks.includes(b.dataset.dm); })());
        if (pair) return go(pair.id);
        status(`Opening a private room with ${b.dataset.name}…`, true);
        try { go(await den.startPrivate([b.dataset.dm], '')); status(''); } catch (e) { status(e.message); }
      };
    });
    el('members').querySelector('[data-close]').onclick = toggleMembers;
    const inv = el('invite');
    if (inv) inv.onclick = () => pickMembers('Invite to this room', 'Invite', false, (keys) => den.invite(room, keys));
  }
  function toggleMembers() {
    const open = el('members').hidden;
    el('members').hidden = !open;
    el('room-meta').setAttribute('aria-expanded', String(open));
    if (open) drawMembers();
  }
  el('room-meta').onclick = toggleMembers;

  // The room list as a drawer, on a phone.
  el('rooms-open').onclick = () => el('den').classList.toggle('drawer');
  el('den-scrim').onclick = () => el('den').classList.remove('drawer');

  // ---- Search, notifications -----------------------------------------------

  el('search').oninput = async () => {
    const q = el('search').value.trim();
    const found = el('found');
    if (!q) { found.hidden = true; return; }
    const hits = await den.search(q);
    if (el('search').value.trim() !== q) return;
    found.hidden = false;
    const hl = (s) => {
      const i = s.toLowerCase().indexOf(q.toLowerCase());
      return i < 0 ? esc(s.slice(0, 90)) : `${i > 30 ? '…' : ''}${esc(s.slice(Math.max(0, i - 30), i))}<mark>${esc(s.slice(i, i + q.length))}</mark>${esc(s.slice(i + q.length, i + q.length + 60))}`;
    };
    found.innerHTML = hits.length
      ? `<p class="found-count">${hits.length} on this device</p>${hits.slice(-30).reverse().map((h) => `<button type="button" data-room="${esc(h.room)}" data-id="${esc(h.message.id)}"><span class="found-where">${esc(roomTitle(h.room))} · ${esc(h.message.from)}</span><span>${hl(h.message.text || h.message.file?.name || '')}</span></button>`).join('')}`
      : '<p class="none">No matches on this device.</p>';
    found.querySelectorAll('[data-room]').forEach((b) => { b.onclick = () => { found.hidden = true; el('search').value = ''; go(b.dataset.room, b.dataset.id); }; });
  };
  el('search').addEventListener('keydown', (e) => { if (e.key === 'Escape') { el('search').value = ''; el('found').hidden = true; } });

  el('notify').value = notify;
  el('notify').onchange = async () => {
    notify = el('notify').value;
    if (notify !== 'off' && typeof Notification !== 'undefined' && Notification.permission === 'default') await Notification.requestPermission();
    if (notify !== 'off' && typeof Notification !== 'undefined' && Notification.permission === 'denied') status('Notifications are blocked for this site in the browser settings.');
    try { localStorage.setItem('den.notify', notify); } catch { /* not kept */ }
  };

  function ping(room_, m, mention) {
    if (!document.hidden || notify === 'off' || typeof Notification === 'undefined' || Notification.permission !== 'granted') return;
    const priv = den.rooms.get(room_)?.kind === 'private';
    if (muted.has(room_)) return;
    if (notify === 'mentions' && !mention && !priv) return;
    const n = new Notification(plain(roomTitle(room_)), { body: plain(`${m.from}: ${(m.text || m.file?.name || '').slice(0, 120)}`), tag: room_ });
    n.onclick = () => { window.focus(); go(room_); n.close(); };
  }
  document.addEventListener('visibilitychange', () => { if (!document.hidden && nearBottom()) { den.markRead(room); drawRooms(); } });

  // ---- Voice -----------------------------------------------------------------

  let meterTimer;
  function drawVoice() {
    const v = den.voice;
    el('voice').hidden = !v;
    drawHead();
    clearInterval(meterTimer);
    if (!v) return;
    el('voice-room').textContent = roomTitle(v.room);
    el('voice-room').onclick = () => go(v.room);
    const people = [{ mid: 'me', key: me.key }, ...(v.people || [])];
    el('voice-who').innerHTML = people.map((p) => `<li data-mid="${esc(p.mid)}" title="${esc(nameOf(p.key))}">${mark(p.key, 22)}<span>${p.mid === 'me' ? 'You' : esc(nameOf(p.key))}</span></li>`).join('');
    meterTimer = setInterval(() => {
      const lv = den.voice?.levels?.() || {};
      el('voice-who').querySelectorAll('li').forEach((li) => li.classList.toggle('speaking', (lv[li.dataset.mid] || 0) > 0.02));
    }, 150);
  }
  el('voice-go').onclick = async () => {
    el('voice-go').disabled = true;
    status('Joining voice…', true);
    try { await den.joinVoice(room, () => drawVoice()); status(''); drawVoice(); drawRooms(); } catch (e) { status(e.message); } finally { el('voice-go').disabled = false; }
  };
  el('voice-mute').onclick = () => {
    const on = el('voice-mute').getAttribute('aria-pressed') !== 'true';
    den.voice?.mute(on);
    el('voice-mute').setAttribute('aria-pressed', String(on));
    el('voice-mute').textContent = on ? 'Unmute' : 'Mute';
  };
  el('voice-leave').onclick = async () => {
    await den.voice?.leave();
    el('voice-mute').setAttribute('aria-pressed', 'false');
    el('voice-mute').textContent = 'Mute';
    drawVoice();
    drawRooms();
  };

  // ---- Events from den.js ---------------------------------------------------

  function drawTyping() {
    const who = Object.entries(typing).filter(([, t]) => Date.now() - t < 4000).map(([k]) => nameOf(k));
    el('typing').innerHTML = who.length ? `<span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>${esc(who.slice(0, 3).join(', '))}${who.length > 3 ? ` and ${who.length - 3} more` : ''} ${who.length > 1 ? 'are' : 'is'} writing` : '';
  }

  den.on = (event, d) => {
    if (event === 'rooms' || event === 'invites') drawRooms();
    if (event === 'ready' || event === 'members') {
      counts[d.room] = d.members.length;
      if (d.room === room) { drawComposer(); if (!el('members').hidden) drawMembers(); }
      if (event === 'ready' && d.room === room && !shown.length) drawLog();
    }
    if (event === 'message') {
      if (d.room === room) onMessage(d).then(drawRooms);
      else drawRooms();
      if (!d.mine && d.kind !== 'react' && d.kind !== 'note' && d.kind !== 'title') ping(d.room, d.message, d.mention);
      if (d.room === room && !d.mine) { delete typing[d.message.key]; drawTyping(); }
    }
    if (event === 'typing' && d.room === room) {
      typing[d.key] = Date.now();
      drawTyping();
      setTimeout(drawTyping, 4100);
    }
    if (event === 'offline') status('Reconnecting…', true);
    if (event === 'online') status('');
    if (event === 'error') status(`${roomTitle(d.room)}: ${d.error}`);
    if (event === 'notice') status(`${roomTitle(d.room)}: ${d.text}`, true);
    // A roommate was shown a different door key for a day: the den may be
    // telling members apart. Stays on screen until the page is reloaded.
    if (event === 'keywarn') {
      let w = document.getElementById('den-warn');
      if (!w) {
        w = document.createElement('p');
        w.id = 'den-warn';
        w.className = 'den-warn';
        w.setAttribute('role', 'alert');
        el('den-main').prepend(w);
      }
      w.textContent = `${nameOf(d.key)} reports a different door key for ${d.day} than this device was shown. Either the den is telling members apart, or ${nameOf(d.key)}'s device is wrong. Tell the host, and keep private things off this den until it is explained.`;
    }
  };

  // This device's own settings: keeping its pass, and forgetting it all.
  const keep = el('keep-pass');
  if (keep) {
    den.keepsPass().then((on) => { keep.checked = on; });
    keep.onchange = () => den.setKeepPass(keep.checked);
  }
  const forgetButton = el('forget');
  if (forgetButton) {
    forgetButton.onclick = async () => {
      if (!(await ask({ heading: 'Forget this device?', text: 'Its chat name, its pass, its rooms and every word it keeps are deleted from this browser. To come back it needs a new way in.', yes: 'Forget', danger: true }))) return;
      forgetButton.disabled = true;
      await den.forget();
      location.reload();
    };
  }

  drawRooms();
  drawLog();
  drawComposer();
  return { go };
}
