// The den's page: rooms, messages, files, private rooms, voice.
// Everything here works on what den.js keeps on this device; nothing is
// fetched in the clear but the den's ciphertext and the day keys.

import { REACTIONS as EMOJI } from './den.js';
import { mountAdmin, slowName } from './den-admin.js';
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

// A task's state, as the page says it (A2A's names underneath).
const TASK_LABEL = { submitted: 'Asked', working: 'Working', 'input-required': 'Needs input', completed: 'Done', failed: 'Failed', canceled: 'Canceled', rejected: 'Declined' };
const ICON = {
  reply: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3 2 7l4 4M2 7h7a5 5 0 0 1 5 5v1" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>',
  react: '<svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M5.5 9.5c1.3 1.6 3.7 1.6 5 0" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M6 6h.01M10 6h.01" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  edit: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="m3 13 1-4 7-7 3 3-7 7-4 1Z" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>',
  task: '<svg viewBox="0 0 16 16" aria-hidden="true"><rect x="2.5" y="2.5" width="11" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="m5.5 8 2 2 3-4" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>',
  pin: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 2h4l-.5 4 2.5 2.5H4L6.5 6 6 2ZM8 8.5V14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/></svg>',
  del: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4h10M6 4V2.5h4V4M4.5 4l.7 9.5h5.6L11.5 4" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>',
  file: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 1.5h5l3.5 3.5v9.5H4z M9 1.5V5h3.5" fill="none" stroke="currentColor" stroke-width="1.3"/></svg>',
};

export function mountDen(den, el, me) {
  let room = 'general';
  let reply = null;
  // The next message is a task for this agent's key, when set.
  let taskFor = null;
  // The side panel: 'members' or 'pins'; and each room's pins, as last read.
  let side = 'members';
  const pinCache = {};
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
    const others = den.inRoom(r.id) ? den.members(r.id).filter((m) => !den.isMe(m.key)) : [];
    if (others.length && others.every((m) => m.key === others[0].key || den.linkedTo(others[0].key).includes(m.key))) return display(others[0].key);
    return r.title || others.map((m) => display(m.key)).join(', ') || 'Private room';
  };
  const roomTitle = (id) => title(den.rooms.get(id) || { id });
  /// The start of a device's own name, shown beside the name its member gave
  /// themselves: two members may claim the same name, never the same key.
  const short = (key) => `m-${String(key).slice(0, 6)}`;
  /// A member as the page shows them: their chosen name, or their device's.
  const display = (key) => den.nickFor(key) || nameOf(key);
  const whoHtml = (key, cls = 'who') => {
    const nick = den.nickFor(key);
    const c = den.colourOf(key);
    return `<b class="${cls}"${c ? ` data-c="${esc(c)}"` : ''} title="${esc(`${nameOf(key)}\n${fingerprint(key)}`)}">${esc(nick || nameOf(key))}</b>${nick ? `<span class="id">${esc(short(key))}</span>` : ''}${agentTag(key)}`;
  };
  /// An agent says it is one; its owner's name only when the owner confirms it.
  const agentTag = (key) => {
    if (!den.isAgent(key)) return '';
    const owner = den.agentOwner(key);
    const label = !owner ? 'agent' : owner === me.key || den.agentsOf(me.key).includes(key) ? 'your agent' : `${display(owner)}’s agent`;
    return `<span class="tag-agent" title="${esc(owner ? `Agent of ${display(owner)} (${nameOf(owner)})` : 'Says it is an agent. No member has confirmed it as theirs.')}">${esc(label)}</span>`;
  };
  // Role colours: the page's rules allow no style attributes in markup, so
  // each [data-c] is painted here, once drawn (a colour is only ever #rrggbb).
  const paint = (root) => {
    for (const n of root.querySelectorAll?.('[data-c]') || []) {
      const c = n.dataset.c;
      if (/^#[0-9a-f]{6}$/i.test(c)) { if (n.classList.contains('dot')) n.style.background = c; else n.style.color = c; }
    }
  };
  new MutationObserver((list) => { for (const m of list) for (const n of m.addedNodes) if (n.nodeType === 1) { if (n.dataset?.c) paint(n.parentNode); else paint(n); } }).observe(document.body, { childList: true, subtree: true });
  let statusTimer;
  const status = (t, sticky = false) => {
    el('den-status').textContent = t || '';
    clearTimeout(statusTimer);
    if (t && !sticky) statusTimer = setTimeout(() => { el('den-status').textContent = ''; }, 6000);
  };

  const drawMe = () => {
    el('me').textContent = me.nick || me.name;
    el('me').title = me.nick ? me.name : '';
  };
  drawMe();
  el('me-mark').innerHTML = mark(me.key, 32);
  const setName = el('set-name');
  if (setName) {
    setName.onclick = async () => {
      const name = await ask({ heading: 'Your name', text: 'Members see it beside this device\'s own name. Leave it empty for none.', field: 'Name', yes: 'Save', value: me.nick, empty: true });
      if (name === null) return;
      try { await den.setNick(name); drawMe(); } catch (e) { status(e.message); }
    };
  }

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
        items.push(`<button type="button" class="room${n ? ' unread' : ''}" data-room="${esc(r.id)}" aria-current="${r.id === room}">${kind === 'open' ? '<span class="hash" aria-hidden="true">#</span>' : ''}<span class="room-name">${esc(title(r))}</span>${voiceHere ? '<span class="live-dot" title="You are in voice here"></span>' : ''}${n ? `<span class="badge" aria-label="${n} unread">${n > 99 ? '99+' : n}</span>` : ''}</button>`);
      }
      const collapsible = kind === 'proposal';
      const open = !collapsible || propsOpen || list.some((r) => r.id === room);
      parts.push(`<div class="group">
        ${collapsible
          ? `<button type="button" class="group-head" data-toggle="props" aria-expanded="${open}"><span>${label} · ${list.length}</span>${!open && unreadHere ? `<span class="badge">${unreadHere}</span>` : ''}<svg class="caret" viewBox="0 0 10 10" aria-hidden="true"><path d="M3 1.5 6.5 5 3 8.5" fill="none" stroke="currentColor" stroke-width="1.4"/></svg></button>`
          : `<p class="group-head"><span>${label}</span>${add && (kind !== 'open' || !den.mod.owner || den.can('manage_channels')) ? `<button type="button" class="add" data-new="${kind}" aria-label="${add}" title="${add}">+</button>` : ''}</p>`}
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
    drawPinCount();
    el('room-topic').textContent = r?.kind === 'open' ? plain(r.topic || '') : '';
    const slow = den.slowOf(room);
    el('room-slow').hidden = !slow;
    el('room-slow').textContent = slow ? `Slow mode · ${slowName(slow)}` : '';
    el('room-slow').title = slow ? `Each member may send one message every ${slowName(slow)} here.` : '';
    el('room-settings').hidden = !(r?.kind === 'open' && den.can('manage_channels'));
    drawDenHead();
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
  function ask({ heading, text = '', field = null, yes, danger = false, value = '', empty = false, readonly = false }) {
    const d = el('ask');
    d.querySelector('h2').textContent = heading;
    d.querySelector('.ask-text').textContent = text;
    d.querySelector('.ask-text').hidden = !text;
    const label = d.querySelector('.ask-field');
    label.hidden = !field;
    const input = label.querySelector('input');
    if (field) { label.querySelector('span').textContent = field; input.value = value; }
    input.readOnly = !!readonly;
    const ok = d.querySelector('[data-yes]');
    ok.textContent = yes;
    ok.classList.toggle('danger', danger);
    d.showModal();
    if (field) { input.focus(); if (readonly) input.select(); } else ok.focus();
    return new Promise((resolve) => {
      let answer = null;
      d.querySelector('form').onsubmit = (e) => {
        e.preventDefault();
        const v = field ? input.value.trim() : true;
        if (field && !v && !empty) return;
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
    return [...all.entries()].map(([key, name]) => ({ key, name, label: display(key) })).sort((a, b) => a.label.localeCompare(b.label));
  }
  /// A mention names a device (@m- and 20 hex digits); shown by the name
  /// that device's member chose, when this device knows it.
  const mentioned = (name) => {
    if (name === me.name) return display(me.key);
    for (const r of rooms()) for (const m of den.members(r.id)) if (m.name === name) return display(m.key);
    return name;
  };
  // A device's name is its key's first ten hex digits (as the core names it).
  const nameOf = (key) => (/^[0-9a-f]{64}$/.test(String(key)) ? `m-${key.slice(0, 20)}` : 'a member');

  function pickMembers(heading, yes, named, done, anyone = false) {
    const already = named || anyone ? new Set() : new Set(den.members(room).map((m) => m.key));
    const list = known().filter((m) => !already.has(m.key));
    const d = el('pick');
    d.querySelector('h2').textContent = heading;
    d.querySelector('.who').innerHTML = list.length
      ? list.map((m) => `<label class="pick-row"><input type="checkbox" value="${esc(m.key)}" data-name="${esc(m.name)}" />${mark(m.key, 24)}<span>${whoHtml(m.key, 'pick-who')}</span></label>`).join('')
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
  const TOKENS = /`([^`\n]{1,200})`|(\bhttps?:\/\/[^\s<>"]+[^\s<>".,;:!?)\]'])|@(m-[0-9a-f]{20})|(\n)|(@everyone\b)/g;
  function words(text) {
    let out = '';
    let last = 0;
    for (const t of String(text).matchAll(TOKENS)) {
      out += esc(text.slice(last, t.index));
      if (t[1] !== undefined) out += `<code>${esc(t[1])}</code>`;
      else if (t[2] !== undefined) out += `<a href="${esc(t[2])}" target="_blank" rel="noopener noreferrer nofollow">${esc(t[2])}</a>`;
      else if (t[3] !== undefined) out += `<mark class="${t[3] === me.name ? 'me' : ''}" title="${esc(t[3])}">@${esc(mentioned(t[3]))}</mark>`;
      else if (t[5] !== undefined) out += '<mark class="all">@everyone</mark>';
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
      parts.push(`<li class="note" data-id="${esc(m.id)}"><span>${esc(m.who ? `${display(m.who)} ${m.act}` : m.text)}</span> <time datetime="${new Date(m.at).toISOString()}">${time(m.at)}</time></li>`);
      return parts.join('');
    }
    const cont = prev && !m.reply && prev.key === m.key && !prev.deleted && m.at - prev.at < GROUP_MS && !parts.length;
    const own = m.key === me.key;
    const head = `<p class="msg-head">${whoHtml(m.key)}${den.isMe(m.key) ? '<span class="you">you</span>' : ''}<time datetime="${new Date(m.at).toISOString()}" title="${esc(new Date(m.at).toLocaleString())}">${time(m.at)}</time>${m.edited && !m.deleted ? '<span class="edited">edited</span>' : ''}</p>`;
    const gutter = cont ? `<span class="side-time">${time(m.at)}</span>` : mark(m.key);
    if (m.deleted) {
      parts.push(`<li class="msg gone${cont ? ' cont' : ''}" data-id="${esc(m.id)}">${gutter}<div class="msg-main">${cont ? '' : head}<p class="body"><em>${m.byMod ? 'Message deleted by a moderator' : 'Message deleted'}</em></p></div></li>`);
      return parts.join('');
    }
    // What its sender (an agent) says it sent this for: an order, a task, or
    // just someone's message.
    const c = den.causeOf(m, log);
    const because = c ? `<button type="button" class="because" data-jump="${esc(c.id)}">${c.how === 'order' ? `For ${esc(display(c.key))}` : c.how === 'task' ? `For ${esc(display(c.key))}'s task` : `Re ${esc(display(c.key))}'s message`}</button>` : '';
    const quoted = m.reply && log.find((x) => x.id === m.reply);
    const q = m.reply ? `<button type="button" class="quote" data-jump="${esc(m.reply)}">${quoted ? `<b>${esc(display(quoted.key))}</b> ${esc(quoted.deleted ? 'Message deleted' : (quoted.text || quoted.file?.name || '').slice(0, 140))}` : 'An earlier message'}</button>` : '';
    let body = `<div class="body">${words(m.text)}${m.edited && cont ? ' <span class="edited">edited</span>' : ''}</div>`;
    if (m.file) {
      const img = inlineImage(m.file);
      body = img
        ? `<button type="button" class="att" data-view="${esc(m.id)}" aria-label="Open ${esc(m.file.name)}"><img data-blob="${esc(m.id)}" alt="${esc(m.file.name)}" /></button>`
        : `<button type="button" class="file" data-file="${esc(m.id)}">${ICON.file}<span><b>${esc(m.file.name)}</b><small>${size(m.file.size)} · Download</small></span></button>`;
    }
    // A task: for whom, and where it stands, from that member's own reports.
    if (m.task) {
      const t = m.task;
      const done = ['completed', 'failed', 'canceled', 'rejected'].includes(t.state);
      const results = (t.artifacts || []).map((a, n) => `<button type="button" class="task-result" data-jump="${esc(a)}">Result ${n + 1}</button>`).join('');
      const steps = (t.history || []).length > 1 ? `<details class="task-history"><summary>History</summary><ol>${t.history.map((h) => `<li>${esc(TASK_LABEL[h.state] || h.state)} <span>${esc(display(h.by))}</span> <time datetime="${new Date(h.at).toISOString()}">${time(h.at)}</time></li>`).join('')}</ol></details>` : '';
      body += `<div class="task-card s-${esc(t.state)}"><span class="task-chip">${esc(TASK_LABEL[t.state] || t.state)}</span><span class="task-for">Task for ${whoHtml(t.to)}</span>${t.text ? `<span class="task-text">${esc(t.text)}</span>` : ''}${results}${own && !done ? '<button type="button" class="mini" data-act="task-cancel">Cancel task</button>' : ''}${steps}</div>`;
    }
    // Reactions are kept by the reactor's key; shown by name.
    const reactions = Object.entries(m.reactions || {}).map(([e, who]) => `<button type="button" class="chip${who.includes(me.key) ? ' on' : ''}" data-react="${esc(m.id)}" data-emoji="${esc(e)}" title="${esc(who.map(display).join(', '))}" aria-pressed="${who.includes(me.key)}">${esc(e)} <span>${who.length}</span></button>`).join('');
    const ping = !den.isMe(m.key) && ([me.key, ...den.linkedTo(me.key)].some((k) => (m.text || '').includes(`@m-${k.slice(0, 20)}`)) || den.everyone(room, m));
    parts.push(`<li class="msg${own ? ' mine' : ''}${ping ? ' ping' : ''}${cont ? ' cont' : ''}" data-id="${esc(m.id)}">
      ${gutter}
      <div class="msg-main">${cont ? '' : head}${because}${q}${body}${reactions ? `<div class="chips">${reactions}</div>` : ''}</div>
      <div class="tools" role="toolbar" aria-label="Message actions">
        <button type="button" data-act="reply" aria-label="Reply" title="Reply">${ICON.reply}</button>
        <button type="button" data-act="react" aria-label="React" title="React" aria-expanded="false">${ICON.react}</button>
        ${den.mayPost(room) ? (() => { const on = (pinCache[room] || []).some((p) => p.of === m.id && p.by === me.key); return `<button type="button" data-act="pin" aria-label="${on ? 'Unpin' : 'Pin'}" title="${on ? 'Unpin' : 'Pin for everyone, also who joins later'}" aria-pressed="${on}">${ICON.pin}</button>`; })() : ''}
        ${own && !m.file && !m.task ? `<button type="button" data-act="edit" aria-label="Edit" title="Edit">${ICON.edit}</button>` : ''}
        ${own ? `<button type="button" data-act="del" aria-label="Delete" title="Delete">${ICON.del}</button>` : ''}
        ${!own && den.can('manage_messages') && den.rooms.get(room)?.kind !== 'private' ? `<button type="button" data-act="moddel" aria-label="Delete (moderator)" title="Delete (moderator)">${ICON.del}</button>` : ''}
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
      const url = URL.createObjectURL(await den.openFile(m.file, (x) => { if (m.file.size > 4 * 1048576) status(`Opening ${m.file.name} · ${Math.round(x * 100)}%`, true); }));
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
    if (t.dataset.act === 'task-cancel') den.taskState(room, m.id, 'canceled').then(() => status('Task canceled.'), (err) => status(err.message));
    if (t.dataset.act === 'pin') {
      const had = (pinCache[room] || []).find((p) => p.of === m.id && p.by === me.key);
      try {
        if (had) await den.unpin(room, had.pin); else await den.pin(room, m);
        status(had ? 'Unpinned.' : 'Pinned. Everyone who joins later gets it too.');
      } catch (err) { status(err.message); }
    }
    if (t.dataset.act === 'moddel') {
      if (await ask({ heading: 'Delete message', text: `${display(m.key)}'s message is removed for everyone in the channel. Devices that already read it may keep their copy.`, yes: 'Delete', danger: true })) den.modDelete(room, m.seq).catch((err) => status(err.message));
    }
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
    banner.hidden = !reply && !editing && !taskFor;
    banner.innerHTML = taskFor
      ? `<span>Task for <b>${esc(display(taskFor))}</b> · it reports back here${reply ? `, in reply to <b>${esc(display(reply.key))}</b>` : ''}</span><button type="button" class="x" id="say-cancel" aria-label="Not a task">×</button>`
      : reply
      ? `<span>Replying to <b>${esc(display(reply.key))}</b> <q>${esc((reply.text || reply.file?.name || '').slice(0, 90))}</q></span><button type="button" class="x" id="say-cancel" aria-label="Cancel reply">×</button>`
      : editing ? '<span>Editing your message · <kbd>Esc</kbd> to cancel</span><button type="button" class="x" id="say-cancel" aria-label="Cancel editing">×</button>' : '';
    // Ask an agent: only where the room has one, and not while editing.
    const agents = den.inRoom(room) ? den.members(room).filter((x) => !den.isMe(x.key) && den.isAgent(x.key)) : [];
    askGo.hidden = !agents.length || !!editing;
    askGo.setAttribute('aria-pressed', String(!!taskFor));
    const c = el('say-cancel');
    if (c) c.onclick = cancelCompose;
    const r = den.rooms.get(room);
    // Why this device cannot write here now, in place of the composer.
    const s = den.sanctionOf(me.key);
    const blocked = s ? (s.kind === 'timeout' ? `You are timed out until ${time(s.until * 1000)}. You can read, but not write, react or talk.` : 'This device was removed from the den by its moderators.')
      : !den.mayPost(room) ? 'You do not have permission to send messages in this channel.' : '';
    el('say').hidden = (!den.inRoom(room) && r?.kind === 'private') || !!blocked;
    el('say').querySelector('[type=submit]').textContent = editing ? 'Save' : 'Send';
    drawSlow(blocked);
    drawHead();
  }

  // Slow mode: how long until this device may write again here.
  let slowTimer;
  function drawSlow(blocked = '') {
    clearTimeout(slowTimer);
    const note = el('say-note');
    const left = den.slowLeft(room);
    const send = el('say').querySelector('[type=submit]');
    send.disabled = left > 0 && !editing;
    if (!note) return;
    if (blocked) { note.hidden = false; note.textContent = blocked; note.classList.add('block'); return; }
    note.classList.remove('block');
    note.hidden = !left;
    if (left) {
      note.textContent = `Slow mode · you can send again in ${Math.ceil(left / 1000)} s`;
      slowTimer = setTimeout(() => drawSlow(), Math.min(1000, left));
    }
  }

  function cancelCompose() {
    if (editing) el('say-text').value = '';
    // The banner's ×: a task first stops being one, then the reply goes.
    if (taskFor) taskFor = null;
    else { reply = null; editing = null; }
    drawComposer();
    grow();
  }

  // Ask an agent: the next message goes as a task for an agent in the room.
  const askGo = document.createElement('button');
  askGo.type = 'button';
  askGo.id = 'ask-agent';
  askGo.hidden = true;
  askGo.setAttribute('aria-label', 'Ask an agent');
  askGo.title = 'Give an agent in this room a task: it reports back here';
  askGo.innerHTML = ICON.task;
  el('attach').after(askGo);
  askGo.onclick = async () => {
    if (taskFor) { taskFor = null; return drawComposer(); }
    // Your own agents first; a name two agents share is shown with its device name.
    const mine = (k) => [me.key, ...den.linkedTo(me.key)].includes(den.agentOwner(k));
    const agents = den.members(room).filter((x) => !den.isMe(x.key) && den.isAgent(x.key)).sort((a, b) => Number(mine(b.key)) - Number(mine(a.key)));
    if (agents.length === 1) { taskFor = agents[0].key; drawComposer(); return el('say-text').focus(); }
    const label = (x) => (agents.filter((y) => display(y.key) === display(x.key)).length > 1 ? `${display(x.key)} (${short(x.key)})` : display(x.key));
    const pick = await ask({ heading: 'Ask an agent', text: `Agents here: ${agents.map((x) => `${label(x)}${mine(x.key) ? ', yours' : ''}`).join('; ')}.`, field: 'Which one (its name, or m- and its first digits)', yes: 'Ask' });
    if (!pick) return;
    const want = pick.trim().toLowerCase();
    let hits = agents.filter((x) => display(x.key).toLowerCase() === want || label(x).toLowerCase() === want || (want.startsWith('m-') && x.name.startsWith(want)));
    // The same name for several: yours, if one of them is.
    if (hits.length > 1 && hits.filter((x) => mine(x.key)).length === 1) hits = hits.filter((x) => mine(x.key));
    if (hits.length !== 1) return status(hits.length ? `Several agents are called ${pick}: give its device name (m- and its first digits).` : `No agent ${pick} in this room.`);
    taskFor = hits[0].key;
    drawComposer();
    el('say-text').focus();
  };

  el('say').onsubmit = async (e) => {
    e.preventDefault();
    const text = el('say-text').value.trim();
    if (!text) return;
    el('say-text').value = '';
    grow();
    closeMentions();
    const was = { reply, editing, taskFor };
    reply = null;
    editing = null;
    taskFor = null;
    drawComposer();
    try {
      if (was.editing) await den.edit(room, was.editing.id, text);
      else if (was.taskFor) await den.task(room, was.taskFor, text, was.reply?.id || null);
      else await den.say(room, text, was.reply?.id || null);
      drawSlow();
    } catch (err) {
      if (err.code === 'slow') drawSlow();
      status(err.message);
      ({ reply, editing, taskFor } = was);
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
    const list = known().filter((x) => x.name.toLowerCase().includes(q) || x.label.toLowerCase().includes(q)).slice(0, 6);
    if (den.can('mention_everyone') && den.rooms.get(room)?.kind !== 'private' && 'everyone'.startsWith(q)) list.unshift({ key: 'everyone', name: 'everyone', label: '@everyone' });
    if (!list.length) return closeMentions();
    mentionPick = Math.min(mentionPick, list.length - 1);
    el('mentions').innerHTML = list.map((x, i) => `<li><button type="button" role="option" data-pick="${esc(x.name)}" aria-selected="${i === mentionPick}">${x.key === 'everyone' ? '<span class="all-mark">@</span><span><b class="pick-who">@everyone</b><span class="id">notifies everyone here</span></span>' : `${mark(x.key, 20)}<span>${whoHtml(x.key, 'pick-who')}</span>`}</button></li>`).join('');
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
      const pct = (x) => `${Math.round(x * 100)}%`;
      status(`Sealing ${f.name}…`, true);
      try {
        await den.sendFile(room, f, (step, x) => status(step === 'seal' ? `Sealing ${f.name} · ${pct(x)}` : `Sending ${f.name} · ${pct(x)}`, true));
        status('');
      } catch (e) { status(e.message); }
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
    if (side === 'pins') return drawPins();
    const all = den.members(room);
    // One row per person: devices linked to each other (each says so) are one.
    const seenKeys = new Set();
    const list = [];
    for (const m of all) {
      if (seenKeys.has(m.key)) continue;
      const devices = [m.key, ...den.linkedTo(m.key).filter((k) => all.some((x) => x.key === k))];
      devices.forEach((k) => seenKeys.add(k));
      list.push({ ...m, devices });
    }
    list.sort((a, b) => (den.isMe(a.key) ? -1 : den.isMe(b.key) ? 1 : display(a.key).localeCompare(display(b.key))));
    const priv = den.rooms.get(room)?.kind === 'private';
    // Two names that start alike are how one member poses as another: say so.
    // So are two members who gave themselves the same name.
    const starts = new Map();
    const count = (k) => starts.set(k, (starts.get(k) || 0) + 1);
    for (const m of list) { count(m.name.slice(0, 8)); if (den.nickFor(m.key)) count(`nick:${den.nickFor(m.key).toLowerCase()}`); }
    const alike = [...starts.values()].some((n) => n > 1);
    // Grouped as on Discord: each role shown on its own (highest first), then everyone else.
    const groupOf = (m) => den.rolesOf(m.key).find((r) => r.hoist) || null;
    const groups = new Map();
    for (const m of list) {
      const g = den.isOwner(m.key) && !groupOf(m) ? { id: 'owner', name: 'Owner', position: Infinity } : groupOf(m) || { id: '', name: 'Members', position: -1 };
      if (!groups.has(g.id)) groups.set(g.id, { ...g, people: [] });
      groups.get(g.id).people.push(m);
    }
    const row = (m) => {
      const s = den.sanctionOf(m.key);
      return `<li>${mark(m.key, 24)}<button type="button" class="member-name" data-member="${esc(m.key)}">${whoHtml(m.key)}${m.devices.length > 1 ? `<span class="id">${m.devices.length} devices</span>` : ''}${s ? `<span class="tag-out" title="${s.kind === 'timeout' ? `Timed out until ${time(s.until * 1000)}` : 'Removed'}">${s.kind === 'timeout' ? 'Timed out' : 'Removed'}</span>` : ''}</button>${den.isMe(m.key) ? '<span class="you">you</span>' : `<button type="button" class="mini" data-dm="${esc(m.devices.join(','))}" data-name="${esc(display(m.key))}">Message</button>`}</li>`;
    };
    const drawnGroups = [...groups.values()].sort((a, b) => b.position - a.position);
    el('members').innerHTML = `<div class="members-head"><h3>Members · ${list.length}</h3><button type="button" class="x" data-close aria-label="Close members">×</button></div>
      ${alike ? '<p class="alike" role="note">Two members here have names that look alike. Compare the device names (m- and digits) before you trust who is who.</p>' : ''}
      ${list.length ? drawnGroups.map((g) => `${drawnGroups.length > 1 || g.id ? `<h4 class="member-group">${esc(g.name)} · ${g.people.length}</h4>` : ''}<ul>${g.people.map(row).join('')}</ul>`).join('') : '<ul><li class="none">Joining…</li></ul>'}
      ${priv && den.inRoom(room) ? '<button type="button" class="act ghost small" id="invite">Invite</button>' : ''}`;
    el('members').querySelectorAll('[data-member]').forEach((b) => { b.onclick = () => admin.member(b.dataset.member); });
    el('members').querySelectorAll('[data-dm]').forEach((b) => {
      b.onclick = async () => {
        // Already a room of just the two of you (and your devices): that one.
        const theirs = b.dataset.dm.split(',');
        const pair = rooms().find((r) => r.kind === 'private' && den.inRoom(r.id) && (() => { const ks = den.members(r.id).map((x) => x.key); return ks.includes(theirs[0]) && ks.every((k) => den.isMe(k) || theirs.includes(k)); })());
        if (pair) return go(pair.id);
        status(`Opening a private room with ${b.dataset.name}…`, true);
        // Everyone's devices: theirs, and this member's other ones.
        const keys = [...new Set([...theirs, ...den.linkedTo(me.key)])];
        try { go(await den.startPrivate(keys, '')); status(''); } catch (e) { status(e.message); }
      };
    });
    el('members').querySelector('[data-close]').onclick = toggleMembers;
    const inv = el('invite');
    if (inv) inv.onclick = () => pickMembers('Invite to this room', 'Invite', false, (keys) => den.invite(room, keys));
  }
  function toggleMembers() { toggleSide('members'); }
  /// The side panel shows the room's members or its pins.
  function toggleSide(want) {
    const open = el('members').hidden || side !== want;
    side = want;
    el('members').hidden = !open;
    el('room-meta').setAttribute('aria-expanded', String(open && side === 'members'));
    pinsGo.setAttribute('aria-expanded', String(open && side === 'pins'));
    if (open) drawMembers();
  }
  el('room-meta').onclick = toggleMembers;
  // Pins: a button of their own beside Members, made here so every page has it.
  const pinsGo = document.createElement('button');
  pinsGo.type = 'button';
  pinsGo.className = 'pill';
  pinsGo.id = 'room-pins';
  pinsGo.hidden = true;
  pinsGo.setAttribute('aria-controls', 'members');
  pinsGo.setAttribute('aria-expanded', 'false');
  el('room-meta').before(pinsGo);
  pinsGo.onclick = () => toggleSide('pins');
  async function drawPinCount() {
    const r = room;
    const pins = await den.pinsOf(r);
    if (r !== room) return;
    pinCache[r] = pins;
    pinsGo.hidden = !pins.length;
    pinsGo.textContent = `Pins · ${pins.length}`;
  }
  async function drawPins() {
    const r = room;
    const pins = await den.pinsOf(r);
    if (r !== room || side !== 'pins') return;
    el('members').innerHTML = `<div class="members-head"><h3>Pins · ${pins.length}</h3><button type="button" class="x" data-close aria-label="Close pins">×</button></div>
      ${pins.length ? `<ul class="pins">${pins.slice().reverse().map((p) => `<li><p class="pin-text">${esc(p.text)}</p><p class="pin-by">Pinned by ${whoHtml(p.by)} · ${time(p.at)}</p>${p.by === me.key || (den.can('manage_messages') && den.rooms.get(r)?.kind !== 'private') ? `<button type="button" class="mini" data-unpin="${esc(p.pin)}" data-by="${esc(p.by)}">Unpin</button>` : ''}</li>`).join('')}</ul>` : '<p class="none">Nothing pinned yet. Pin a message from its menu: everyone who joins later gets it too.</p>'}`;
    paint(el('members'));
    el('members').querySelector('[data-close]').onclick = () => toggleSide('pins');
    el('members').querySelectorAll('[data-unpin]').forEach((b) => { b.onclick = () => den.unpin(room, b.dataset.unpin, b.dataset.by).then(() => status('Unpinned.'), (e) => status(e.message)); });
  }

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
      ? `<p class="found-count">${hits.length} on this device</p>${hits.slice(-30).reverse().map((h) => `<button type="button" data-room="${esc(h.room)}" data-id="${esc(h.message.id)}"><span class="found-where">${esc(roomTitle(h.room))} · ${esc(display(h.message.key))}</span><span>${hl(h.message.text || h.message.file?.name || '')}</span></button>`).join('')}`
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
    syncPush();
  };
  // What a push is, said where the choice is made.
  el('notify').closest('label')?.insertAdjacentHTML('afterend', '<p class="push-note">With this page closed, notifications come through your browser’s push service (Apple, Google, Mozilla or Microsoft). It sees when one arrives, from which den, and which of its users get pushes from that den around the same time; a push names the room, never the words. With “Mentions and private”, a closed page hears of private rooms only: the den cannot see mentions.</p>');

  // Installed on the home screen: its own window, notifications on iPhone, and
  // this device's keys kept (Safari clears a site's data after a week unused).
  let sw = null;
  if ('serviceWorker' in navigator) {
    navigator.serviceWorker.register('./sw.js').then((r) => { sw = r; syncPush(); }).catch(() => {});
    navigator.serviceWorker.addEventListener('message', (e) => { if (e.data?.den === 'open' && typeof e.data.room === 'string' && den.rooms.has(e.data.room)) go(e.data.room); });
  }
  navigator.storage?.persist?.().catch(() => {});
  const standalone = matchMedia('(display-mode: standalone)').matches || navigator.standalone === true;
  const foot = el('forget').parentElement;
  let installAsk = null;
  window.addEventListener('beforeinstallprompt', (e) => {
    e.preventDefault();
    installAsk = e;
    if (document.getElementById('install-app')) return;
    foot.querySelector('.link-acts')?.insertAdjacentHTML('beforeend', '<button type="button" class="mini ghost" id="install-app">Install</button>');
    document.getElementById('install-app').onclick = async () => {
      installAsk?.prompt();
      const { outcome } = (await installAsk?.userChoice) || {};
      if (outcome === 'accepted') document.getElementById('install-app')?.remove();
    };
  });
  if (!standalone && /iPhone|iPad|iPod/.test(navigator.userAgent)) {
    foot.insertAdjacentHTML('beforeend', '<p class="push-note">On iPhone: Share, then Add to Home Screen. Installed, this device keeps its keys and gets notifications.</p>');
  }

  /// This device's push address, in step with the setting: signed up when
  /// notifications are on and allowed, taken back when they are off.
  async function syncPush() {
    if (!sw || !('PushManager' in window) || typeof Notification === 'undefined') return;
    try {
      let sub = await sw.pushManager.getSubscription();
      if (notify === 'off' || Notification.permission !== 'granted') {
        if (sub) { await den.pushOff(sub.endpoint).catch(() => {}); await sub.unsubscribe().catch(() => {}); }
        return;
      }
      const key = await den.pushKey();
      const raw = Uint8Array.from(atob(key.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
      // Made for another key (the den's was replaced): sign up again.
      const had = sub?.options?.applicationServerKey ? new Uint8Array(sub.options.applicationServerKey) : null;
      if (sub && had && (had.length !== raw.length || had.some((b, i) => b !== raw[i]))) { await sub.unsubscribe().catch(() => {}); sub = null; }
      if (!sub) sub = await sw.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: raw });
      const j = sub.toJSON();
      await den.pushSub({ endpoint: j.endpoint, p256dh: j.keys.p256dh, auth: j.keys.auth, mode: notify === 'all' ? 'all' : 'private' });
    } catch (e) {
      // A browser without a push service the den sends to: notifications only while open.
      if (/push service/.test(e.message || '')) status(e.message);
    }
  }

  function ping(room_, m, mention) {
    if (!document.hidden || notify === 'off' || typeof Notification === 'undefined' || Notification.permission !== 'granted') return;
    const priv = den.rooms.get(room_)?.kind === 'private';
    if (muted.has(room_)) return;
    if (notify === 'mentions' && !mention && !priv) return;
    const opts = { body: plain(`${display(m.key)}: ${(m.text || m.file?.name || '').slice(0, 120)}`), tag: room_, data: { room: room_ } };
    // Installed, a page cannot make a Notification itself: its worker shows it.
    if (sw) return sw.showNotification(plain(roomTitle(room_)), opts).catch(() => {});
    const n = new Notification(plain(roomTitle(room_)), opts);
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
    // Relayed: this network let no WebRTC through, so voice goes by the den's own connection.
    el('voice').querySelector('.voice-label').lastChild.textContent = v.mode === 'relay' ? 'Voice · relayed' : 'Voice';
    el('voice').querySelector('.voice-label').title = v.mode === 'relay' ? 'This network lets no direct voice connection through, so the sealed audio goes over the den\'s own connection. It may lag a little more.' : '';
    el('voice-room').onclick = () => go(v.room);
    // Everyone in voice; who is in a slot now (heard here) shows who speaks.
    const mids = new Map((v.people || []).map((p) => [p.key, p.mid]));
    const keys = [...new Set([...(v.roster || []), ...mids.keys()])];
    const people = [{ mid: 'me', key: me.key }, ...keys.map((key) => ({ key, mid: mids.get(key) || '' }))];
    el('voice-who').innerHTML = people.map((p) => `<li data-mid="${esc(p.mid)}" title="${esc(nameOf(p.key))}">${mark(p.key, 22)}<span>${p.mid === 'me' ? 'You' : esc(display(p.key))}</span></li>`).join('');
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
    const who = Object.entries(typing).filter(([, t]) => Date.now() - t < 4000).map(([k]) => display(k));
    el('typing').innerHTML = who.length ? `<span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>${esc(who.slice(0, 3).join(', '))}${who.length > 3 ? ` and ${who.length - 3} more` : ''} ${who.length > 1 ? 'are' : 'is'} writing` : '';
  }

  // People, not devices: a member's linked devices count once, as in the members list.
  const people = (members) => {
    const seenKeys = new Set();
    return members.filter((m) => { if (seenKeys.has(m.key)) return false; seenKeys.add(m.key); den.linkedTo(m.key).forEach((k) => seenKeys.add(k)); return true; }).length;
  };
  let nickTimer;
  let modTimer;
  den.on = (event, d) => {
    if (event === 'rooms' || event === 'invites') drawRooms();
    if (event === 'ready' || event === 'members') {
      counts[d.room] = people(d.members);
      if (d.room === room) { drawComposer(); if (!el('members').hidden) drawMembers(); }
      if (event === 'ready' && d.room === room && !shown.length) drawLog();
    }
    if (event === 'message') {
      if (d.room === room) onMessage(d).then(drawRooms);
      else drawRooms();
      if (!d.mine && d.kind !== 'react' && d.kind !== 'note' && d.kind !== 'title' && d.kind !== 'task-state') ping(d.room, d.message, d.mention);
      if (d.room === room && !d.mine) { delete typing[d.message.key]; drawTyping(); }
    }
    if (event === 'typing' && d.room === room) {
      typing[d.key] = Date.now();
      drawTyping();
      setTimeout(drawTyping, 4100);
    }
    if (event === 'nicks') {
      clearTimeout(nickTimer);
      nickTimer = setTimeout(() => { for (const r of Object.keys(counts)) if (den.inRoom(r)) counts[r] = people(den.members(r)); drawMe(); drawRooms(); drawLog(); drawComposer(); if (!el('members').hidden) drawMembers(); if (den.voice) drawVoice(); }, 300);
    }
    if (event === 'mod') {
      clearTimeout(modTimer);
      modTimer = setTimeout(() => { drawRooms(); drawComposer(); drawLog(); if (!el('members').hidden) drawMembers(); den.onMod?.(); }, 100);
    }
    if (event === 'removed') {
      status('This device was removed from the den by its moderators.', true);
      el('say').hidden = true;
    }
    if (event === 'pins' && d.room === room) { drawPinCount().then(() => { drawLog(); if (!el('members').hidden && side === 'pins') drawPins(); }); }
    if (event === 'linked') status('Linked: members now see these devices as one person.');
    if (event === 'owned') {
      const said = `${display(d.key)} is now your agent. It takes orders from you only.`;
      agentInvite = null;
      status(said);
      // The dialog that showed the code says so too, where the member is looking.
      const a = el('ask');
      if (a.open && a.dataset.kind === 'own') {
        a.querySelector('h2').textContent = 'Agent added';
        a.querySelector('.ask-text').textContent = said;
        a.querySelector('.ask-text').hidden = false;
        a.querySelector('.ask-field').hidden = true;
        a.querySelector('[data-yes]').textContent = 'Done';
      }
    }
    if (event === 'offline') status('Reconnecting…', true);
    if (event === 'online') {
      status('');
      syncPush();
      // Opened from a notification: that room.
      const want = new URLSearchParams(location.hash.slice(1)).get('room');
      if (want) { history.replaceState(null, '', location.pathname); setTimeout(() => { if (den.rooms.has(want)) go(want); }, 500); }
    }
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
      w.textContent = `${display(d.key)} (${nameOf(d.key)}) reports a different door key for ${d.day} than this device was shown. Either the den is telling members apart, or that member's device is wrong. Tell the host, and keep private things off this den until it is explained.`;
    }
  };

  // Linking another device of this member's: a code shown here, typed there.
  const linkGo = el('link-device');
  if (linkGo) {
    linkGo.onclick = async () => {
      const code = den.startLink();
      await ask({ heading: 'Link another device', text: `On the other device, enter this den with its own invite, choose “Enter a link code” and type ${code} within ten minutes. Members then see both as you.`, yes: 'Done' });
    };
  }
  let agentInvite = null;
  // An agent of this member's (epeaky-agent): a code shown here, given to it there.
  const agentGo = el('add-agent');
  if (agentGo) {
    agentGo.onclick = async () => {
      const code = den.startOwn();
      // A member who may make invites gets one command that brings the agent
      // in and makes it theirs; anyone else, the code for an agent already in.
      // One invite until an agent takes it: closing the dialog and opening it
      // again within the hour shows the same one, not a new one each time.
      let link = agentInvite && Date.now() - agentInvite.at < 3_600_000 ? agentInvite.link : null;
      // Still unused? The den lists invites by the SHA-256 of their code; an
      // agent may have used this one before it said so here.
      if (link) {
        try {
          const code = new URLSearchParams(new URL(link).hash.slice(1)).get('invite') || '';
          const bytes = Uint8Array.from(code.match(/../g) || [], (h) => parseInt(h, 16));
          const id = [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map((b) => b.toString(16).padStart(2, '0')).join('').slice(0, 12);
          const { invites } = await den.modDo('invites');
          const i = invites.find((x) => x.id === id);
          if (!i || i.used_at || i.revoked_at || (i.expires_at && i.expires_at * 1000 < Date.now())) link = null;
        } catch { link = null; }
        if (!link) agentInvite = null;
      }
      if (!link && den.can('manage_invites')) {
        try { link = (await den.modDo('invite_make', { n: 1, days: 1 })).links?.[0] || null; } catch { link = null; }
        if (link) agentInvite = { link, at: Date.now() };
      }
      el('ask').dataset.kind = 'own';
      await ask(link
        ? { heading: 'Add an agent', text: 'Run this on the agent’s machine within ten minutes. The agent joins the den with a new invite and takes orders from you and your linked devices only. Whoever runs it first becomes your agent: give it to your agent alone. Add --agent NAME to run more than one there.', field: 'Command', value: `epeaky-agent join '${link}' --owner ${code}`, readonly: true, yes: 'Done' }
        : { heading: 'Add an agent', text: 'For an agent already in this den: run this on its machine within ten minutes. It then takes orders from you and your linked devices only. A new agent first needs an invite from the den’s host or a moderator.', field: 'Command', value: `epeaky-agent owner ${code}`, readonly: true, yes: 'Done' });
      delete el('ask').dataset.kind;
    };
  }
  const linkUse = el('link-code');
  if (linkUse) {
    linkUse.onclick = async () => {
      const code = await ask({ heading: 'Enter a link code', text: 'The code your other device shows under “Link another device”.', field: 'Code', yes: 'Link' });
      if (!code) return;
      try { await den.useLink(code); status('Asked your other device…', true); } catch (e) { status(e.message); }
    };
  }

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

  // ---- The den itself: its name and icon, and its settings ------------------------------

  const admin = mountAdmin(den, el, { display, whoHtml, mark, ask, status, known, pickMembers });
  function drawDenHead() {
    const d = den.mod.den || {};
    const may = admin.sections().length > 0;
    el('den-head').hidden = !d.name && !d.icon && !may;
    el('den-name').textContent = plain(d.name) || 'Epeaky';
    el('den-icon').innerHTML = d.icon ? `<img src="/door/icon?v=${encodeURIComponent(d.name || '')}" alt="" />` : esc((plain(d.name) || 'E').slice(0, 1));
    el('den-settings').hidden = !may;
  }
  el('den-settings').onclick = () => admin.open();
  el('room-settings').onclick = () => admin.open('channels', room);

  drawRooms();
  drawLog();
  drawComposer();
  return { go };
}
