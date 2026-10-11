// The den's page: rooms, messages, files, private rooms, voice (§184).
// Everything here works on what den.js keeps on this device; nothing is
// fetched in the clear but the den's ciphertext and the day keys.

const EMOJI = ['👍', '❤️', '😂', '🎉', '👀', '🙏'];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const time = (t) => new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
const size = (n) => (n > 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);

export function mountDen(den, el, me) {
  let room = 'general';
  let reply = null;
  let editing = null;
  let notify = (() => { try { return localStorage.getItem('cave.den.notify') || 'mentions'; } catch { return 'mentions'; } })();
  const counts = {};
  const typing = {};
  const urls = new Map();

  const rooms = () => [...den.rooms.values()];
  const title = (r) => r.title || (r.kind === 'private' ? 'Private room' : r.id);
  const status = (t) => { el('den-status').textContent = t || ''; };

  // ---- The room list --------------------------------------------------------

  async function drawRooms() {
    const groups = [['open', 'Rooms'], ['proposal', 'Proposals'], ['private', 'Private']];
    const parts = [];
    for (const [kind, label] of groups) {
      const list = rooms().filter((r) => r.kind === kind);
      if (!list.length && kind === 'proposal') continue;
      parts.push(`<p class="group">${label}${kind === 'open' ? ' <button type="button" class="mini" data-new="open">New</button>' : kind === 'private' ? ' <button type="button" class="mini" data-new="private">New</button>' : ''}</p>`);
      for (const r of list) {
        const n = await den.unread(r.id);
        parts.push(`<button type="button" data-room="${esc(r.id)}" aria-current="${r.id === room}">${esc(title(r))}${n ? ` <small class="badge">${n}</small>` : ''}</button>`);
      }
    }
    el('rooms').innerHTML = parts.join('');
    el('rooms').querySelectorAll('[data-room]').forEach((b) => { b.onclick = () => go(b.dataset.room); });
    el('rooms').querySelectorAll('[data-new]').forEach((b) => { b.onclick = () => (b.dataset.new === 'open' ? newRoom() : pickMembers('New private room', (keys, name) => den.startPrivate(keys, name).then(go))); });
    const r = den.rooms.get(room);
    el('room-title').textContent = r ? title(r) : room;
    el('room-meta').textContent = counts[room] ? `${counts[room]} in the room` : '';
    el('voice-go').hidden = !den.inRoom(room) || !!den.voice;
  }

  function go(id) {
    room = id;
    reply = null; editing = null;
    drawRooms();
    drawLog();
    drawComposer();
  }

  async function newRoom() {
    const name = prompt('Name the room (others see it).');
    if (!name) return;
    try {
      const r = await den.push(den.lobby, 'open_room', { title: name });
      setTimeout(() => go(r.id), 600);
    } catch (e) { status(e.message); }
  }

  // ---- Messages -------------------------------------------------------------

  function words(text) {
    return esc(text).replace(/@(m-[0-9a-f]{10})/g, (m, n) => `<mark class="${n === me.name ? 'me' : ''}">${m}</mark>`).replace(/\n/g, '<br>');
  }

  function line(m, log) {
    if (m.deleted) return `<li class="gone" data-id="${esc(m.id)}"><b>${esc(m.from)}</b><span class="t">${time(m.at)}</span><em>deleted</em></li>`;
    const quoted = m.reply && log.find((x) => x.id === m.reply);
    const q = quoted ? `<blockquote>${esc(quoted.from)}: ${esc((quoted.text || quoted.file?.name || '').slice(0, 120))}</blockquote>` : '';
    let body = words(m.text);
    if (m.file) {
      const img = m.file.mime.startsWith('image/') && m.file.size <= 5 * 1048576;
      body = img ? `<img class="att" data-blob="${esc(m.id)}" alt="${esc(m.file.name)}" />` : `<button type="button" class="file" data-file="${esc(m.id)}">${esc(m.file.name)} · ${size(m.file.size)}</button>`;
    }
    const reactions = Object.entries(m.reactions || {}).map(([e, who]) => `<button type="button" class="chip${who.includes(me.name) ? ' on' : ''}" data-react="${esc(m.id)}" data-emoji="${esc(e)}" title="${esc(who.join(', '))}">${esc(e)} ${who.length}</button>`).join('');
    const own = m.key === me.key;
    return `<li class="${own ? 'mine' : ''}${m.text.includes(`@${me.name}`) ? ' ping' : ''}" data-id="${esc(m.id)}">
      <b>${esc(m.from)}</b><span class="t">${time(m.at)}${m.edited ? ' · edited' : ''}</span>${q}<div class="body">${body}</div>
      ${reactions ? `<div class="chips">${reactions}</div>` : ''}
      <div class="acts"><button type="button" data-act="reply">Reply</button><button type="button" data-act="react">React</button>${own && !m.file ? '<button type="button" data-act="edit">Edit</button>' : ''}${own ? '<button type="button" data-act="del">Delete</button>' : ''}</div>
    </li>`;
  }

  async function drawLog() {
    const log = await den.log(room);
    const box = el('log');
    box.innerHTML = log.length ? log.map((m) => line(m, log)).join('') : `<li class="note">${den.inRoom(room) || den.rooms.get(room)?.kind !== 'private' ? 'Nothing here yet. What you read is kept on this device only.' : 'Waiting for an invitation to this room.'}</li>`;
    box.scrollTop = box.scrollHeight;
    den.markRead(room);
    wireLog(log);
    for (const img of box.querySelectorAll('img[data-blob]')) showImage(img, log.find((m) => m.id === img.dataset.blob));
    drawRooms();
  }

  async function showImage(img, m) {
    try {
      if (!urls.has(m.id)) urls.set(m.id, URL.createObjectURL(await den.openFile(m.file)));
      img.src = urls.get(m.id);
    } catch (e) { img.replaceWith(Object.assign(document.createElement('em'), { textContent: e.message })); }
  }

  function wireLog(log) {
    const box = el('log');
    box.querySelectorAll('[data-file]').forEach((b) => {
      b.onclick = async () => {
        const m = log.find((x) => x.id === b.dataset.file);
        try {
          const url = URL.createObjectURL(await den.openFile(m.file));
          Object.assign(document.createElement('a'), { href: url, download: m.file.name }).click();
          setTimeout(() => URL.revokeObjectURL(url), 60_000);
        } catch (e) { status(e.message); }
      };
    });
    box.querySelectorAll('[data-react]').forEach((b) => { b.onclick = () => den.react(room, b.dataset.react, b.dataset.emoji).catch((e) => status(e.message)); });
    box.querySelectorAll('[data-act]').forEach((b) => {
      const id = b.closest('li').dataset.id;
      const m = log.find((x) => x.id === id);
      b.onclick = () => {
        if (b.dataset.act === 'reply') { reply = m; editing = null; drawComposer(); el('say-text').focus(); }
        if (b.dataset.act === 'edit') { editing = m; reply = null; el('say-text').value = m.text; drawComposer(); el('say-text').focus(); }
        if (b.dataset.act === 'del' && confirm('Delete this message for everyone in the room?')) den.remove(room, id).catch((e) => status(e.message));
        if (b.dataset.act === 'react') {
          const bar = document.createElement('div');
          bar.className = 'emoji';
          bar.innerHTML = EMOJI.map((e) => `<button type="button">${e}</button>`).join('');
          bar.querySelectorAll('button').forEach((x) => { x.onclick = () => { bar.remove(); den.react(room, id, x.textContent).catch((e) => status(e.message)); }; });
          b.closest('li').append(bar);
        }
      };
    });
  }

  function drawComposer() {
    const banner = el('say-banner');
    banner.hidden = !reply && !editing;
    banner.innerHTML = reply ? `Replying to ${esc(reply.from)}: ${esc((reply.text || reply.file?.name || '').slice(0, 80))} <button type="button" id="say-cancel">Cancel</button>`
      : editing ? `Editing your message <button type="button" id="say-cancel">Cancel</button>` : '';
    const c = el('say-cancel');
    if (c) c.onclick = () => { reply = null; if (editing) el('say-text').value = ''; editing = null; drawComposer(); };
    const r = den.rooms.get(room);
    el('say').hidden = !den.inRoom(room) && r?.kind === 'private';
  }

  el('say').onsubmit = async (e) => {
    e.preventDefault();
    const text = el('say-text').value.trim();
    if (!text) return;
    el('say-text').value = '';
    try {
      if (editing) await den.edit(room, editing.id, text);
      else await den.say(room, text, reply?.id || null);
      reply = null; editing = null; drawComposer();
    } catch (err) { status(err.message); el('say-text').value = text; }
  };
  el('say-text').onkeydown = (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); el('say').requestSubmit(); } };
  el('say-text').oninput = () => den.typing(room);
  el('attach').onclick = () => el('attach-file').click();
  el('attach-file').onchange = async () => {
    const f = el('attach-file').files[0];
    el('attach-file').value = '';
    if (!f) return;
    status(`Sealing and sending ${f.name}…`);
    try { await den.sendFile(room, f); status(''); } catch (e) { status(e.message); }
  };

  // ---- Members, private rooms ------------------------------------------------

  /// Every device this one knows of, from the rooms it is in.
  function known() {
    const all = new Map();
    for (const r of rooms()) for (const m of den.members(r.id)) if (m.key !== me.key) all.set(m.key, m.name);
    return [...all.entries()].map(([key, name]) => ({ key, name }));
  }

  function pickMembers(heading, done) {
    const list = known();
    const d = el('pick');
    d.querySelector('h2').textContent = heading;
    d.querySelector('.who').innerHTML = list.length
      ? list.map((m) => `<label><input type="checkbox" value="${esc(m.key)}" data-name="${esc(m.name)}" /> ${esc(m.name)}</label>`).join('')
      : '<p class="note">No one yet: members appear here once they are in a room with you.</p>';
    d.querySelector('[name=title]').value = '';
    d.showModal();
    d.querySelector('form').onsubmit = async (e) => {
      e.preventDefault();
      const picked = [...d.querySelectorAll('input[type=checkbox]:checked')];
      if (!picked.length) return;
      const name = d.querySelector('[name=title]').value.trim() || picked.map((x) => x.dataset.name).join(', ');
      d.close();
      status('Making the room and sending invitations…');
      try { await done(picked.map((x) => x.value), name); status(''); } catch (err) { status(err.message); }
    };
    d.querySelector('[data-close]').onclick = () => d.close();
  }

  function drawMembers() {
    const list = den.members(room);
    const priv = den.rooms.get(room)?.kind === 'private';
    el('members').innerHTML = `<p class="group">In this room</p>${list.map((m) => `<p class="who">${esc(m.name)}${m.key === me.key ? ' (you)' : ` <button type="button" class="mini" data-dm="${esc(m.key)}" data-name="${esc(m.name)}">Message privately</button>`}</p>`).join('')}
      ${priv && den.inRoom(room) ? '<button type="button" class="act ghost small" id="invite">Invite</button>' : ''}`;
    el('members').querySelectorAll('[data-dm]').forEach((b) => {
      b.onclick = async () => {
        status(`Opening a private room with ${b.dataset.name}…`);
        try { go(await den.startPrivate([b.dataset.dm], b.dataset.name)); status(''); } catch (e) { status(e.message); }
      };
    });
    const inv = el('invite');
    if (inv) inv.onclick = () => pickMembers('Invite to this room', (keys) => den.invite(room, keys));
  }
  el('room-meta').onclick = () => { el('members').hidden = !el('members').hidden; drawMembers(); };

  // ---- Search, notifications -----------------------------------------------

  el('search').oninput = async () => {
    const q = el('search').value;
    const box = el('found');
    if (!q.trim()) { box.hidden = true; return; }
    const hits = await den.search(q);
    box.hidden = false;
    box.innerHTML = hits.length ? hits.slice(-30).reverse().map((h) => `<button type="button" data-room="${esc(h.room)}"><b>${esc(title(den.rooms.get(h.room) || { id: h.room }))}</b> ${esc(h.message.from)}: ${esc((h.message.text || h.message.file?.name || '').slice(0, 80))}</button>`).join('') : '<p class="note">Nothing on this device matches.</p>';
    box.querySelectorAll('[data-room]').forEach((b) => { b.onclick = () => { box.hidden = true; el('search').value = ''; go(b.dataset.room); }; });
  };

  const drawNotify = () => { el('notify').textContent = { all: 'Notify: every message', mentions: 'Notify: mentions and private', off: 'Notify: off' }[notify]; };
  el('notify').onclick = async () => {
    if (typeof Notification !== 'undefined' && Notification.permission === 'default') await Notification.requestPermission();
    notify = { mentions: 'all', all: 'off', off: 'mentions' }[notify];
    try { localStorage.setItem('cave.den.notify', notify); } catch { /* not kept */ }
    drawNotify();
  };
  drawNotify();

  function ping(room_, m, mention) {
    if (!document.hidden || notify === 'off' || typeof Notification === 'undefined' || Notification.permission !== 'granted') return;
    const priv = den.rooms.get(room_)?.kind === 'private';
    if (notify === 'mentions' && !mention && !priv) return;
    const n = new Notification(title(den.rooms.get(room_) || { id: room_ }), { body: `${m.from}: ${(m.text || m.file?.name || '').slice(0, 120)}`, tag: room_ });
    n.onclick = () => { window.focus(); go(room_); n.close(); };
  }

  // ---- Voice -----------------------------------------------------------------

  async function drawVoice() {
    const v = den.voice;
    const people = v?.people;
    el('voice').hidden = !v;
    el('voice-go').hidden = !!v || !den.inRoom(room);
    if (!v) return;
    const names = new Map(known().map((m) => [m.key, m.name]));
    el('voice-who').textContent = `Voice in ${title(den.rooms.get(v.room) || { id: v.room })}: you${people?.length ? `, ${people.map((p) => names.get(p.key) || 'a member').join(', ')}` : ''}`;
  }
  el('voice-go').onclick = async () => {
    status('Joining voice…');
    try { await den.joinVoice(room, () => drawVoice()); status(''); drawVoice(); } catch (e) { status(e.message); }
  };
  el('voice-mute').onclick = () => {
    const on = el('voice-mute').getAttribute('aria-pressed') !== 'true';
    den.voice?.mute(on);
    el('voice-mute').setAttribute('aria-pressed', String(on));
    el('voice-mute').textContent = on ? 'Unmute' : 'Mute';
  };
  el('voice-leave').onclick = async () => { await den.voice?.leave(); drawVoice(); drawRooms(); };

  // ---- Events from den.js ---------------------------------------------------

  den.on = (event, d) => {
    if (event === 'rooms') drawRooms();
    if (event === 'ready' || event === 'members') { counts[d.room] = d.members.length; if (d.room === room) { drawRooms(); drawComposer(); if (!el('members').hidden) drawMembers(); } }
    if (event === 'message') {
      if (d.room === room) drawLog();
      else drawRooms();
      if (!d.mine && d.kind !== 'react') ping(d.room, d.message, d.mention);
    }
    if (event === 'typing' && d.room === room) {
      typing[d.key] = Date.now();
      const names = new Map(known().map((m) => [m.key, m.name]));
      const who = Object.entries(typing).filter(([, t]) => Date.now() - t < 4000).map(([k]) => names.get(k) || 'someone');
      el('typing').textContent = who.length ? `${who.join(', ')} ${who.length > 1 ? 'are' : 'is'} writing…` : '';
      setTimeout(() => { if (Date.now() - typing[d.key] >= 4000) el('typing').textContent = ''; }, 4100);
    }
    if (event === 'offline') status('Reconnecting…');
    if (event === 'online') status('');
    if (event === 'error') status(`${title(den.rooms.get(d.room) || { id: d.room })}: ${d.error}`);
  };

  drawRooms();
  drawLog();
  drawComposer();
  return { go };
}
