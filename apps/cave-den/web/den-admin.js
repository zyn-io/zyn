// Den settings and member actions: what the den's owner and the roles they
// make may do (Den.Mod on the den checks every one of these again). Drawn
// over den.js's moderation state (den.mod).

import { PERMS, SLOW } from './den.js';

export const PERM_TEXT = {
  administrator: ['Administrator', 'Every permission below, over members and roles below this one.'],
  manage_server: ['Manage den', "Change the den's name, description, icon and limits."],
  manage_channels: ['Manage channels', 'Create, rename and delete channels; set slow mode and who may post.'],
  manage_roles: ['Manage roles', 'Create and edit roles below their own, and give or take them.'],
  manage_messages: ['Manage messages', "Delete anyone's messages."],
  timeout: ['Timeout members', 'Stop a member from writing, reacting and talking for a while.'],
  kick: ['Kick members', 'Take a member out of every channel until tomorrow.'],
  ban: ['Ban members', "Take a member's device out of the den for good."],
  manage_invites: ['Manage invites', 'Make and withdraw invite links; let in people who ask.'],
  bypass_slow: ['Bypass slow mode', 'Write without waiting in channels with slow mode.'],
  mention_everyone: ['Mention @everyone', 'Notify everyone in a channel.'],
};

export const slowName = (s) => (!s ? 'Off' : s < 60 ? `${s} s` : s < 3600 ? `${s / 60} min` : `${s / 3600} h`);
const TIMEOUTS = [[60, '60 seconds'], [300, '5 minutes'], [600, '10 minutes'], [3600, '1 hour'], [86400, '1 day'], [604800, '1 week']];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const when = (unix) => (unix ? new Date(unix * 1000).toLocaleString([], { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }) : '');
const mb = (n) => Math.round(n / 1e6);

export function mountAdmin(den, el, ui) {
  const { display, whoHtml, mark, ask, status, known } = ui;
  const dlg = el('settings');
  let section = 'overview';
  let picked = null; // the role or channel open in its editor

  const sections = () => [
    ['overview', 'Overview', den.can('manage_server')],
    ['roles', 'Roles', den.can('manage_roles')],
    ['channels', 'Channels', den.can('manage_channels')],
    ['moderation', 'Timeouts and bans', den.can('timeout') || den.can('kick') || den.can('ban')],
    ['invites', 'Invites', den.can('manage_invites')],
    ['audit', 'Audit log', den.moderates()],
    ['owner', 'Ownership', den.isOwner()],
  ].filter((s) => s[2]);

  function open(which = null, pick = null) {
    const list = sections();
    if (!list.length) return;
    section = list.some((s) => s[0] === which) ? which : list[0][0];
    picked = pick;
    draw();
    if (!dlg.open) dlg.showModal();
  }

  function draw() {
    el('set-tabs').innerHTML = sections().map(([id, label]) => `<button type="button" data-tab="${id}" aria-current="${id === section}">${label}</button>`).join('');
    el('set-tabs').querySelectorAll('[data-tab]').forEach((b) => { b.onclick = () => { section = b.dataset.tab; picked = null; draw(); }; });
    ({ overview, roles, channels, moderation, invites, audit, owner })[section]();
  }

  const body = () => el('set-body');
  const fail = (e) => { const o = body().querySelector('.set-out'); if (o) { o.textContent = e.message; o.classList.add('bad'); } else status(e.message); };
  const done = (t) => { const o = body().querySelector('.set-out'); if (o) { o.textContent = t; o.classList.remove('bad'); } };
  const run = async (f, ok = 'Saved.') => { try { await f(); done(ok); } catch (e) { fail(e); } };

  // ---- Overview --------------------------------------------------------------

  function overview() {
    const d = den.mod.den || {};
    const lim = { ...den.mod.ceilings, ...den.mod.limits };
    body().innerHTML = `<h3>Overview</h3>
      <form class="set-form" id="ov">
        <div class="icon-row">
          <span class="den-icon big">${d.icon ? `<img src="/door/icon?v=${Date.now()}" alt="" />` : esc((d.name || 'E').slice(0, 1))}</span>
          <div class="icon-acts">
            <label class="mini file-pick">Upload image<input type="file" accept="image/png,image/jpeg,image/webp" id="ov-icon" hidden /></label>
            ${d.icon ? '<button type="button" class="mini ghost" id="ov-noicon">Remove</button>' : ''}
            <small>PNG, JPEG or WebP, up to 256 KB. Square works best.</small>
          </div>
        </div>
        <label class="field">Name<input name="name" maxlength="40" value="${esc(d.name)}" placeholder="Epeaky" /></label>
        <label class="field">Description<textarea name="description" maxlength="300" rows="3">${esc(d.description)}</textarea></label>
        <div class="two">
          <label class="field">Largest file (MB)<input name="file_max" type="number" min="1" max="${mb(den.mod.ceilings?.file_max || 1e8)}" value="${mb(lim.file_max || 1e8)}" /></label>
          <label class="field">Files a member may send a day (MB)<input name="file_bytes" type="number" min="1" max="${mb(den.mod.ceilings?.file_bytes || 5e8)}" value="${mb(lim.file_bytes || 5e8)}" /></label>
        </div>
        <p class="set-note">The server's host allows at most ${mb(den.mod.ceilings?.file_max || 1e8)} MB a file and ${mb(den.mod.ceilings?.file_bytes || 5e8)} MB a day.</p>
        <div class="row"><button class="act small" type="submit">Save</button></div>
        <p class="out set-out" role="status"></p>
      </form>`;
    const f = el('ov');
    f.onsubmit = (e) => {
      e.preventDefault();
      const v = (n) => f.elements[n].value;
      const limits = { file_max: Math.max(1, Number(v('file_max'))) * 1e6, file_bytes: Math.max(1, Number(v('file_bytes'))) * 1e6 };
      run(() => den.modDo('settings_save', { name: v('name'), description: v('description'), limits }));
    };
    el('ov-icon').onchange = async () => {
      const file = el('ov-icon').files[0];
      if (!file) return;
      if (file.size > 262144) return fail(new Error('That image is over 256 KB.'));
      const url = await new Promise((resolve) => { const r = new FileReader(); r.onload = () => resolve(r.result); r.readAsDataURL(file); });
      run(() => den.modDo('settings_save', { icon: url }), 'Icon saved.');
    };
    const no = el('ov-noicon');
    if (no) no.onclick = () => run(() => den.modDo('settings_save', { icon: '' }), 'Icon removed.');
  }

  // ---- Roles -------------------------------------------------------------------

  const myTop = () => den.top(den.me.key);
  const editable = (r) => den.isOwner() || r.id === 'everyone' || r.position < myTop();
  const holdersOf = (id) => Object.entries(den.mod.holders).filter(([, ids]) => ids.includes(id)).map(([k]) => k);

  function roles() {
    const list = [...den.mod.roles].sort((a, b) => b.position - a.position);
    const sel = list.find((r) => r.id === picked) || null;
    body().innerHTML = `<h3>Roles</h3>
      <p class="set-note">Members get the permissions of all their roles. A role's place in the list sets who can act on whom: members act only on those whose highest role is below theirs.</p>
      <div class="split">
        <div class="pick-list">
          ${list.map((r) => `<button type="button" data-role="${esc(r.id)}" aria-current="${r.id === picked}"><i class="dot" data-c="${esc(r.colour || '')}"></i><span>${esc(r.name)}</span><small>${r.id === 'everyone' ? 'all' : holdersOf(r.id).length}</small></button>`).join('')}
          <button type="button" class="mini" id="role-new">Create role</button>
        </div>
        <div class="editor" id="role-editor">${sel ? '' : '<p class="none">Pick a role to edit it.</p>'}</div>
      </div>
      <p class="out set-out" role="status"></p>`;
    body().querySelectorAll('[data-role]').forEach((b) => { b.onclick = () => { picked = b.dataset.role; roles(); }; });
    el('role-new').onclick = () => run(async () => {
      const top = den.isOwner() ? Math.max(1, ...den.mod.roles.map((r) => r.position)) + 1 : Math.max(1, myTop() - 1);
      await den.modDo('role_save', { name: 'New role', position: top, perms: [] });
      await new Promise((r) => setTimeout(r, 150));
      picked = [...den.mod.roles].filter((r) => r.name === 'New role').sort((a, b) => b.position - a.position)[0]?.id || null;
      roles();
    }, 'Role made.');
    if (sel) roleEditor(sel, list);
  }

  function roleEditor(r, list) {
    const every = r.id === 'everyone';
    const can = editable(r);
    const ed = el('role-editor');
    const members = holdersOf(r.id);
    ed.innerHTML = `<form class="set-form" id="role-form">
      ${every ? '<p class="set-note">@everyone is every member of the den. Its permissions are everyone\'s.</p>' : `
      <label class="field">Name<input name="name" maxlength="32" value="${esc(r.name)}" ${can ? '' : 'disabled'} /></label>
      <div class="two">
        <label class="field">Colour<input name="colour" type="color" value="${esc(r.colour || '#c7f04b')}" ${can ? '' : 'disabled'} /></label>
        <label class="check"><input type="checkbox" name="nocolour" ${r.colour ? '' : 'checked'} ${can ? '' : 'disabled'} /> No colour</label>
      </div>
      <label class="check"><input type="checkbox" name="hoist" ${r.hoist ? 'checked' : ''} ${can ? '' : 'disabled'} /> List its members on their own in the member list</label>`}
      <fieldset class="perms"><legend>Permissions</legend>
        ${PERMS.map((p) => { const mine = den.can(p) || den.isOwner(); return `<label class="perm${mine ? '' : ' off'}"><input type="checkbox" name="perm" value="${p}" ${r.perms.includes(p) ? 'checked' : ''} ${can && mine ? '' : 'disabled'} /><span><b>${PERM_TEXT[p][0]}</b><small>${PERM_TEXT[p][1]}</small></span></label>`; }).join('')}
      </fieldset>
      ${can ? `<div class="row">${every ? '' : '<button type="button" class="mini" data-move="1">Move up</button><button type="button" class="mini" data-move="-1">Move down</button><button type="button" class="mini danger" id="role-del">Delete role</button>'}<button class="act small" type="submit">Save</button></div>` : '<p class="set-note">This role is at or above your own highest role, so you cannot change it.</p>'}
      </form>
      ${every ? '' : `<div class="holders"><h4>Members · ${members.length}</h4>
        <ul>${members.map((k) => `<li>${mark(k, 20)}${whoHtml(k)}${can ? `<button type="button" class="mini ghost" data-take="${esc(k)}">Remove</button>` : ''}</li>`).join('') || '<li class="none">Nobody has this role yet.</li>'}</ul>
        ${can ? '<button type="button" class="mini" id="role-add">Add members</button>' : ''}</div>`}`;
    const f = el('role-form');
    const perms = () => [...f.querySelectorAll('[name=perm]:checked')].map((x) => x.value);
    f.onsubmit = (e) => {
      e.preventDefault();
      const attrs = every ? { id: r.id, perms: perms() } : { id: r.id, name: f.elements.name.value, colour: f.elements.nocolour.checked ? null : f.elements.colour.value, hoist: f.elements.hoist.checked, position: r.position, perms: perms() };
      run(() => den.modDo('role_save', attrs));
    };
    ed.querySelectorAll('[data-move]').forEach((b) => {
      b.onclick = () => run(async () => {
        const others = list.filter((x) => x.id !== 'everyone');
        const i = others.findIndex((x) => x.id === r.id);
        const next = others[i - Number(b.dataset.move)];
        if (!next) return;
        const save = (x, position) => den.modDo('role_save', { id: x.id, name: x.name, colour: x.colour, hoist: x.hoist, perms: x.perms, position });
        if (next.position === r.position) await save(r, Math.max(1, r.position + Number(b.dataset.move)));
        else { await save(r, next.position); await save(next, r.position); }
      }, 'Moved.');
    });
    const del = el('role-del');
    if (del) del.onclick = async () => { if (await ask({ heading: `Delete ${r.name}?`, text: 'Members who have it lose its permissions.', yes: 'Delete role', danger: true })) run(async () => { await den.modDo('role_delete', { id: r.id }); picked = null; }, 'Role deleted.'); };
    ed.querySelectorAll('[data-take]').forEach((b) => { b.onclick = () => run(() => den.onPerson('role_take', b.dataset.take, { id: r.id }), 'Removed.'); });
    const add = el('role-add');
    if (add) add.onclick = () => ui.pickMembers(`Add to ${r.name}`, 'Add', false, (keys) => Promise.all(keys.map((k) => den.onPerson('role_give', k, { id: r.id }))), true);
  }

  // ---- Channels ----------------------------------------------------------------

  function channels() {
    const list = [...den.rooms.values()].filter((r) => r.kind === 'open');
    const sel = list.find((r) => r.id === picked) || null;
    body().innerHTML = `<h3>Channels</h3>
      <div class="split">
        <div class="pick-list">
          ${list.map((r) => `<button type="button" data-chan="${esc(r.id)}" aria-current="${r.id === picked}"><span># ${esc(r.title || r.id)}</span><small>${r.slow ? slowName(r.slow) : ''}</small></button>`).join('')}
          <button type="button" class="mini" id="chan-new">Create channel</button>
        </div>
        <div class="editor" id="chan-editor">${sel ? '' : '<p class="none">Pick a channel to set it up.</p>'}</div>
      </div>
      <p class="out set-out" role="status"></p>`;
    body().querySelectorAll('[data-chan]').forEach((b) => { b.onclick = () => { picked = b.dataset.chan; channels(); }; });
    el('chan-new').onclick = async () => {
      const name = await ask({ heading: 'Create channel', field: 'Name', yes: 'Create' });
      if (!name) return;
      run(async () => { const r = await den.push(den.lobby, 'open_room', { title: name }); picked = r.id; await new Promise((x) => setTimeout(x, 150)); channels(); }, 'Channel made.');
    };
    if (sel) chanEditor(sel);
  }

  function chanEditor(r) {
    const roleList = den.mod.roles.filter((x) => x.id !== 'everyone').sort((a, b) => b.position - a.position);
    const some = (r.post_roles || []).length > 0;
    el('chan-editor').innerHTML = `<form class="set-form" id="chan-form">
      <label class="field">Name<input name="title" maxlength="40" value="${esc(r.title)}" /></label>
      <label class="field">Topic<input name="topic" maxlength="200" value="${esc(r.topic || '')}" placeholder="What this channel is for" /></label>
      <label class="field">Slow mode<select name="slow">${SLOW.map((s) => `<option value="${s}" ${s === (r.slow || 0) ? 'selected' : ''}>${slowName(s)}</option>`).join('')}</select></label>
      <p class="set-note">Each member may send one message per this interval. Bypass slow mode skips it.</p>
      <fieldset class="perms"><legend>Who may post</legend>
        <label class="check"><input type="radio" name="who" value="all" ${some ? '' : 'checked'} /> Everyone</label>
        <label class="check"><input type="radio" name="who" value="some" ${some ? 'checked' : ''} /> Only these roles (and administrators)</label>
        <div class="role-checks">${roleList.map((x) => `<label class="check"><input type="checkbox" name="role" value="${esc(x.id)}" ${(r.post_roles || []).includes(x.id) ? 'checked' : ''} /> <i class="dot" data-c="${esc(x.colour || '')}"></i>${esc(x.name)}</label>`).join('') || '<p class="none">Make a role first.</p>'}</div>
      </fieldset>
      <div class="row">${r.fixed ? '' : '<button type="button" class="mini danger" id="chan-del">Delete channel</button>'}<button class="act small" type="submit">Save</button></div>
    </form>`;
    const f = el('chan-form');
    f.onsubmit = (e) => {
      e.preventDefault();
      const roles = f.elements.who.value === 'some' ? [...f.querySelectorAll('[name=role]:checked')].map((x) => x.value) : [];
      if (f.elements.who.value === 'some' && !roles.length) return fail(new Error('Pick at least one role, or let everyone post.'));
      run(() => den.modDo('room_save', { id: r.id, title: f.elements.title.value, topic: f.elements.topic.value, slow: Number(f.elements.slow.value), post_roles: roles }));
    };
    if (el('chan-del')) el('chan-del').onclick = async () => {
      if (!(await ask({ heading: `Delete # ${r.title}?`, text: 'The channel and its messages go for everyone.', yes: 'Delete channel', danger: true }))) return;
      run(async () => { await den.modDo('room_delete', { id: r.id }); picked = null; channels(); }, 'Channel deleted.');
    };
  }

  // ---- Timeouts and bans -----------------------------------------------------

  async function moderation() {
    const out = den.mod.sanctions.filter((s) => s.kind !== 'ban' && den.sanctionOf(s.key));
    body().innerHTML = `<h3>Timeouts and bans</h3>
      <h4>Timed out and kicked</h4>
      <ul class="rows">${out.map((s) => `<li>${mark(s.key, 20)}${whoHtml(s.key)}<small>${s.kind === 'kick' ? 'Kicked' : 'Timed out'} until ${when(s.until)}</small>${den.can(s.kind === 'kick' ? 'ban' : 'timeout') && den.above(s.key) ? `<button type="button" class="mini ghost" data-lift="${esc(s.key)}" data-kind="${s.kind}">${s.kind === 'kick' ? 'Let back in' : 'Remove timeout'}</button>` : ''}</li>`).join('') || '<li class="none">Nobody.</li>'}</ul>
      ${den.can('ban') ? '<h4>Bans</h4><ul class="rows" id="ban-list"><li class="none">Loading…</li></ul>' : ''}
      <p class="out set-out" role="status"></p>`;
    body().querySelectorAll('[data-lift]').forEach((b) => { b.onclick = () => run(async () => { await den.modDo(b.dataset.kind === 'kick' ? 'unban' : 'untimeout', { key: b.dataset.lift }); moderation(); }, 'Done.'); });
    if (!den.can('ban')) return;
    try {
      const { bans } = await den.modDo('bans');
      el('ban-list').innerHTML = bans.map((b) => `<li>${mark(b.key, 20)}${whoHtml(b.key)}<small>${esc(b.reason || 'No reason given')} · ${when(b.at)}</small><button type="button" class="mini ghost" data-unban="${esc(b.key)}">Unban</button></li>`).join('') || '<li class="none">Nobody is banned.</li>';
      el('ban-list').querySelectorAll('[data-unban]').forEach((b) => { b.onclick = () => run(async () => { await den.modDo('unban', { key: b.dataset.unban }); moderation(); }, 'Unbanned.'); });
    } catch (e) { fail(e); }
  }

  // ---- Invites --------------------------------------------------------------------

  async function invites() {
    body().innerHTML = `<h3>Invites</h3>
      <form class="set-form inline" id="inv-form">
        <label class="field">Links<input name="n" type="number" min="1" max="20" value="1" /></label>
        <label class="field">Good for<select name="days"><option value="1">1 day</option><option value="7" selected>7 days</option><option value="30">30 days</option><option value="90">90 days</option></select></label>
        <button class="act small" type="submit">Make links</button>
      </form>
      <p class="set-note">Each link lets one person in, on the device that opens it. A link is shown only now: copy it before closing.</p>
      <ul class="rows links" id="inv-links"></ul>
      <h4>Requests to join</h4><ul class="rows" id="req-list"><li class="none">Loading…</li></ul>
      <h4>Made</h4><ul class="rows" id="inv-list"><li class="none">Loading…</li></ul>
      <p class="out set-out" role="status"></p>`;
    el('inv-form').onsubmit = (e) => {
      e.preventDefault();
      const f = e.target;
      run(async () => {
        const { links } = await den.modDo('invite_make', { n: Number(f.elements.n.value), days: Number(f.elements.days.value) });
        el('inv-links').innerHTML = links.map((l) => `<li><code>${esc(l)}</code><button type="button" class="mini" data-copy="${esc(l)}">Copy</button></li>`).join('');
        el('inv-links').querySelectorAll('[data-copy]').forEach((b) => { b.onclick = () => navigator.clipboard?.writeText(b.dataset.copy).then(() => { b.textContent = 'Copied'; }); });
        listInvites();
      }, 'Links made.');
    };
    listInvites();
  }

  async function listInvites() {
    try {
      const [{ requests }, { invites: made }] = await Promise.all([den.modDo('requests'), den.modDo('invites')]);
      el('req-list').innerHTML = requests.map((r) => `<li><span class="note-text">${esc(r.note || 'No note')}</span><small>${when(r.asked_at)}</small><button type="button" class="mini" data-yes="${esc(r.id)}">Let in</button><button type="button" class="mini ghost" data-no="${esc(r.id)}">Decline</button></li>`).join('') || '<li class="none">Nobody is waiting.</li>';
      el('req-list').querySelectorAll('[data-yes],[data-no]').forEach((b) => { b.onclick = () => run(async () => { await den.modDo(b.dataset.yes ? 'approve' : 'decline', { id: b.dataset.yes || b.dataset.no }); listInvites(); }, 'Done.'); });
      const state = (i) => (i.revoked_at ? 'Withdrawn' : i.used_at ? `Used ${when(i.used_at)}` : i.expires_at * 1000 < Date.now() ? 'Expired' : `Open until ${when(i.expires_at)}`);
      el('inv-list').innerHTML = made.slice(-50).reverse().map((i) => `<li><code>${esc(i.id)}</code><small>${state(i)}</small>${!i.revoked_at ? `<button type="button" class="mini ghost" data-withdraw="${esc(i.id)}">Withdraw</button>` : ''}</li>`).join('') || '<li class="none">None yet.</li>';
      el('inv-list').querySelectorAll('[data-withdraw]').forEach((b) => {
        b.onclick = async () => {
          if (!(await ask({ heading: 'Withdraw this invite?', text: 'If someone joined with it, their pass stops working too.', yes: 'Withdraw', danger: true }))) return;
          run(async () => { await den.modDo('invite_withdraw', { id: b.dataset.withdraw }); listInvites(); }, 'Withdrawn.');
        };
      });
    } catch (e) { fail(e); }
  }

  // ---- Audit log -------------------------------------------------------------------

  async function audit() {
    body().innerHTML = '<h3>Audit log</h3><p class="set-note">What moderators did, for 90 days. Never what anyone wrote.</p><ul class="rows audit" id="audit-list"><li class="none">Loading…</li></ul><p class="out set-out" role="status"></p>';
    try {
      const { entries } = await den.modDo('audit', { n: 200 });
      const who = (k) => (k && /^[0-9a-f]{64}$/.test(k) ? `${mark(k, 18)}${whoHtml(k)}` : esc(k ? (den.rooms.get(k)?.title ? `# ${den.rooms.get(k).title}` : k) : ''));
      el('audit-list').innerHTML = entries.map((a) => `<li><small>${when(a.at)}</small>${a.actor ? who(a.actor) : '<b class="who">Host</b>'}<span>${esc(a.action)}</span>${who(a.target)}${a.reason ? `<q>${esc(a.reason)}</q>` : ''}</li>`).join('') || '<li class="none">Nothing yet.</li>';
    } catch (e) { fail(e); }
  }

  // ---- Ownership -----------------------------------------------------------------

  function owner() {
    body().innerHTML = `<h3>Ownership</h3>
      <p class="set-note">You own this den: you hold every permission and nobody can act on you. You can hand that to another member's device. The server's host can always choose the owner again.</p>
      <div class="row start"><button type="button" class="mini danger" id="own-pass">Transfer ownership</button></div>
      <p class="out set-out" role="status"></p>`;
    el('own-pass').onclick = () => ui.pickMembers('Transfer ownership to', 'Transfer', false, async (keys) => {
      if (keys.length !== 1) throw new Error('Pick one member.');
      if (!(await ask({ heading: `Make ${display(keys[0])} the owner?`, text: 'You keep your roles, but lose the owner\'s powers.', yes: 'Transfer', danger: true }))) return;
      await den.modDo('owner_pass', { key: keys[0] });
      dlg.close();
    }, true);
  }

  dlg.addEventListener('click', (e) => { if (e.target === dlg || e.target.closest('[data-close]')) dlg.close(); });
  den.onMod = () => { if (dlg.open && section !== 'invites' && section !== 'audit' && !dlg.querySelector(':focus-within input:focus, textarea:focus')) draw(); };

  // ---- One member: their roles, and what a moderator may do to them ---------------

  function member(key) {
    const d = el('member');
    const roles = den.rolesOf(key);
    const s = den.sanctionOf(key);
    const may = (p) => den.can(p) && den.above(key) && !den.isMe(key);
    const givable = den.mod.roles.filter((r) => r.id !== 'everyone' && (den.isOwner() || r.position < den.top(den.me.key))).sort((a, b) => b.position - a.position);
    const manage = den.can('manage_roles') && (den.isMe(key) || den.above(key));
    el('member-card').innerHTML = `<div class="card-head">${mark(key, 40)}<div><p>${whoHtml(key)}</p><small>m-${esc(key.slice(0, 20))}${den.isOwner(key) ? ' · Owner' : ''}</small></div><button class="x" type="button" data-close aria-label="Close">×</button></div>
      ${roles.length ? `<div class="chips">${roles.map((r) => `<span class="role-chip"><i class="dot" data-c="${esc(r.colour || '')}"></i>${esc(r.name)}</span>`).join('')}</div>` : ''}
      ${s ? `<p class="sanction">${s.kind === 'ban' ? 'Banned' : s.kind === 'kick' ? `Kicked until ${when(s.until)}` : `Timed out until ${when(s.until)}`}</p>` : ''}
      ${manage && givable.length ? `<fieldset class="perms"><legend>Roles</legend>${givable.map((r) => `<label class="check"><input type="checkbox" data-give="${esc(r.id)}" ${roles.some((x) => x.id === r.id) ? 'checked' : ''} /> <i class="dot" data-c="${esc(r.colour || '')}"></i>${esc(r.name)}</label>`).join('')}</fieldset>` : ''}
      ${may('timeout') ? `<form class="set-form inline" id="to-form"><label class="field">Timeout<select name="s">${TIMEOUTS.map(([v, t]) => `<option value="${v}">${t}</option>`).join('')}</select></label><label class="field grow">Reason<input name="reason" maxlength="300" placeholder="Optional; moderators see it" /></label><button class="act small" type="submit">Time out</button></form>${s?.kind === 'timeout' ? '<button type="button" class="mini" id="to-lift">Remove timeout</button>' : ''}` : ''}
      <div class="row start">${may('kick') ? '<button type="button" class="mini danger" id="m-kick">Kick</button>' : ''}${may('ban') ? '<button type="button" class="mini danger" id="m-ban">Ban</button>' : ''}</div>
      <p class="out set-out" role="status"></p>`;
    const out = (t, bad = false) => { const o = el('member-card').querySelector('.set-out'); o.textContent = t; o.classList.toggle('bad', bad); };
    const act = async (f, ok) => { try { await f(); out(ok); } catch (e) { out(e.message, true); } };
    el('member-card').querySelectorAll('[data-give]').forEach((c) => { c.onchange = () => act(() => den.onPerson(c.checked ? 'role_give' : 'role_take', key, { id: c.dataset.give }), c.checked ? 'Role given.' : 'Role taken.'); });
    const to = el('to-form');
    if (to) to.onsubmit = (e) => { e.preventDefault(); act(() => den.onPerson('timeout', key, { seconds: Number(to.elements.s.value), reason: to.elements.reason.value }), 'Timed out.'); };
    const lift = el('to-lift');
    if (lift) lift.onclick = () => act(() => den.onPerson('untimeout', key), 'Timeout removed.');
    const reasoned = async (heading, text, yes) => ask({ heading, text, field: 'Reason (moderators see it)', yes, danger: true, empty: true });
    const kick = el('m-kick');
    if (kick) kick.onclick = async () => { const r = await reasoned(`Kick ${display(key)}?`, 'They are taken out of every channel and can come back tomorrow.', 'Kick'); if (r !== null) act(() => den.onPerson('kick', key, { reason: r }), 'Kicked.'); };
    const ban = el('m-ban');
    if (ban) ban.onclick = async () => { const r = await reasoned(`Ban ${display(key)}?`, 'Their device is taken out of the den and cannot come back. They could return on a new device, as a new name.', 'Ban'); if (r !== null) act(() => den.onPerson('ban', key, { reason: r }), 'Banned.'); };
    if (!d.open) d.showModal();
  }
  el('member').addEventListener('click', (e) => { if (e.target === el('member') || e.target.closest('[data-close]')) el('member').close(); });

  return { open, member, sections };
}
