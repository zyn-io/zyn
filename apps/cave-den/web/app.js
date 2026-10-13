// The den's page: starts this device, lets it in (an invite link, a request
// to the host, or the host's issuer window in gate.js), then draws the rooms
// (den-ui.js) over the client (den.js).
import { DenClient } from './den.js';
import { mountDen } from './den-ui.js';
import { enter as gate } from './gate.js';

const el = (id) => document.getElementById(id);
const config = await fetch('./config.json', { credentials: 'omit' }).then((r) => r.json());
const den = new DenClient({ base: location.origin, keysUrl: config.keysUrl || '/door/keys', db: config.db || 'den', prefix: config.prefix || 'den' });
// Local builds only: the tests reach the client.
if (['localhost', '127.0.0.1'].includes(location.hostname)) window.__den = den;

/// The "i" bubbles: what needs explaining opens beside the words it explains.
for (const b of document.querySelectorAll('.tip[data-tip]')) {
  b.onclick = () => {
    const open = b.getAttribute('aria-expanded') === 'true';
    b.setAttribute('aria-expanded', String(!open));
    const host = b.closest('p, div');
    if (host.nextElementSibling?.classList.contains('bubble')) host.nextElementSibling.remove();
    if (!open) {
      const bubble = document.createElement('p');
      bubble.className = 'bubble';
      bubble.textContent = b.dataset.tip;
      host.after(bubble);
    }
  };
}

const out = el('chat-out');
const say = (t, bad = false) => { out.textContent = t; out.classList.toggle('bad', bad); };
const me = await den.start();
const open = (session) => {
  el('me').textContent = me.name;
  document.querySelector('.ledger').hidden = true;
  document.querySelector('.register h1').hidden = true;
  el('den').hidden = false;
  mountDen(den, el, me);
  den.connect(session);
};

// The ways in. An invite link carries its code after "#", so it never reaches
// a server log; it is taken off the address at once.
const ways = await den.ways().catch(() => ({}));
const invite = new URLSearchParams(location.hash.slice(1)).get('invite');
if (invite) history.replaceState(null, '', location.pathname + location.search);

const show = (id) => { const e = el(id); if (e) e.hidden = false; };
let waiting = null;
function wait() {
  say("Asked. This page lets you in once the host approves; you can close it and come back.");
  for (const id of ['way-ask', 'way-invite']) if (el(id)) el(id).hidden = true;
  waiting ??= setInterval(async () => {
    const a = await den.asked().catch(() => null);
    if (!a || a.status === 'pending') return;
    clearInterval(waiting);
    if (a.session) { say(''); open(a.session); } else { say('The host declined the request.', true); ways.approval && show('way-ask'); }
  }, 15_000);
}

function door(asking) {
  if (config.issuer) show('enter');
  if (ways.invite) show('way-invite');
  if (ways.approval) show('way-ask');
  if (asking?.status === 'pending') return wait();
  if (asking?.status === 'declined') say('The host declined the request.', true);
  else if (!config.issuer && !ways.invite && !ways.approval) say('This den has no way in set up yet.', true);
}

const enter = el('enter');
if (enter) {
  enter.onclick = async () => {
    enter.disabled = true;
    try {
      open(await gate({ den, say, config }));
      say('');
    } catch (e) {
      say(e?.message || String(e), !e?.quiet);
    } finally { enter.disabled = false; }
  };
}
const askForm = el('way-ask');
if (askForm) {
  askForm.onsubmit = async (ev) => {
    ev.preventDefault();
    const note = el('ask-note').value.trim();
    if (!note) return say('Write a short note for the host.', true);
    try { await den.ask(note); wait(); } catch (e) { say(e.message, true); }
  };
}

// A session, tomorrow's token or this device's pass kept here: in without the door.
const kept = await den.session().catch(() => null);
if (kept) open(kept);
else if (invite) {
  say('Joining with the invite…');
  try { open(await den.join(invite)); say(''); } catch (e) { say(e.message, true); door(); }
} else door(await den.asked().catch(() => null));
