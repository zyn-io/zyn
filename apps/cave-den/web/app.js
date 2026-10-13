// The den's page: starts this device, enters through the host's gate
// (gate.js), then draws the rooms (den-ui.js) over the client (den.js).
import { DenClient } from './den.js';
import { mountDen } from './den-ui.js';
import { enter } from './gate.js';

const el = (id) => document.getElementById(id);
const config = await fetch('./config.json', { credentials: 'omit' }).then((r) => r.json());
const den = new DenClient({ base: location.origin, keysUrl: config.keysUrl || '/den-keys.json', prefix: config.prefix || 'den' });
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

// A session or tomorrow's token kept on this device: in without the gate.
const kept = await den.session().catch(() => null);
if (kept) open(kept);
else {
  el('enter').onclick = async () => {
    el('enter').disabled = true;
    try {
      open(await enter({ den, say, config }));
      say('');
    } catch (e) {
      say(e?.message || String(e), !e?.quiet);
    } finally { el('enter').disabled = false; }
  };
}
