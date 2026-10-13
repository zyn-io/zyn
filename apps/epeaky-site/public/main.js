// Epeaky's site: the peephole's ciphertext, the two views of one message,
// the "i" bubbles and the copy button. Nothing is sent anywhere.

const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const noise = (n) => {
  const r = crypto.getRandomValues(new Uint8Array(n));
  let s = '';
  for (const b of r) s += B64[b & 63];
  return s;
};

// The peephole: what passes the door.
const peep = document.getElementById('peep-cipher');
if (peep) peep.textContent = Array.from({ length: 6 }, () => noise(44)).join('   ');

// One message, two views: typed on a member's device, a blob on the server.
const lines = [
  ['m-4e7827479f', 'general', 'Dinner at ours on Friday? Bring the good bread.'],
  ['m-72646d5724', 'general', 'Draft is in the shared folder, comments welcome.'],
  ['m-a31f0c9be2', 'p-7f3a…00aa', 'Can we talk tonight? Just the three of us.'],
  ['m-548d17cc29', 'lookouts', 'Landed. Phone at 4%, will call from the hotel.'],
];
const $ = (id) => document.getElementById(id);
const pad = (n) => String(n).padStart(2, '0');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function views() {
  const text = $('v-text');
  const blob = $('v-blob');
  if (!text || !blob) return;
  for (let i = 0; ; i = (i + 1) % lines.length) {
    const [who, room, said] = lines[i];
    $('v-who').textContent = who;
    $('v-dev').textContent = who;
    $('v-room').textContent = room;
    // An MLS PrivateMessage carries the words plus framing, padding and a tag.
    const bytes = new TextEncoder().encode(said).length + 96 + Math.floor(Math.random() * 16);
    const sealed = noise(Math.ceil(bytes / 3) * 4);
    if (still) {
      text.textContent = said;
      blob.textContent = sealed;
      $('v-size').textContent = `${bytes} bytes`;
      $('v-at').textContent = new Date().toTimeString().slice(0, 8);
      return;
    }
    text.textContent = '';
    blob.textContent = '';
    $('v-size').textContent = '…';
    $('v-at').textContent = '--:--:--';
    for (const ch of said) {
      text.textContent += ch;
      await sleep(34 + Math.random() * 60);
    }
    await sleep(420);
    const d = new Date();
    $('v-at').textContent = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
    $('v-size').textContent = `${bytes} bytes`;
    for (let k = 0; k < sealed.length; k += 6) {
      blob.replaceChildren(sealed.slice(0, k));
      const fresh = document.createElement('span');
      fresh.className = 'fresh';
      fresh.textContent = sealed.slice(k, k + 6);
      blob.append(fresh);
      await sleep(14);
    }
    blob.textContent = sealed;
    await sleep(3200);
  }
}

// Start the two views when they come into sight.
const split = document.querySelector('.split');
if (split && 'IntersectionObserver' in window) {
  const io = new IntersectionObserver((es) => {
    if (es.some((e) => e.isIntersecting)) { io.disconnect(); views(); }
  }, { threshold: 0.3 });
  io.observe(split);
} else views();

// The "i" bubbles: one open at a time; Escape or a click elsewhere closes.
let open = null;
const close = () => {
  if (!open) return;
  open.btn.setAttribute('aria-expanded', 'false');
  open.tip.hidden = true;
  open = null;
};
for (const btn of document.querySelectorAll('.info[data-tip]')) {
  const tip = document.getElementById(btn.dataset.tip);
  btn.setAttribute('aria-expanded', 'false');
  btn.setAttribute('aria-controls', tip.id);
  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    const was = open?.btn === btn;
    close();
    if (was) return;
    tip.hidden = false;
    // Point the bubble's arrow at its button.
    const x = btn.getBoundingClientRect().left - tip.getBoundingClientRect().left + btn.offsetWidth / 2 - 6;
    tip.style.setProperty('--ax', `${Math.max(12, Math.min(x, tip.offsetWidth - 24))}px`);
    btn.setAttribute('aria-expanded', 'true');
    open = { btn, tip };
  });
  tip.addEventListener('click', (e) => e.stopPropagation());
}
document.addEventListener('click', close);
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') { const b = open?.btn; close(); b?.focus(); } });

// Copy the install command.
for (const btn of document.querySelectorAll('.copy[data-copy]')) {
  btn.addEventListener('click', async () => {
    const text = document.getElementById(btn.dataset.copy).textContent.trim();
    try {
      await navigator.clipboard.writeText(text);
      btn.textContent = 'Copied';
    } catch {
      const r = document.createRange();
      r.selectNodeContents(document.getElementById(btn.dataset.copy));
      getSelection().removeAllRanges();
      getSelection().addRange(r);
      btn.textContent = 'Selected';
    }
    setTimeout(() => { btn.textContent = 'Copy'; }, 1600);
  });
}

// Sections rise in as they arrive.
if (!still && 'IntersectionObserver' in window) {
  const items = document.querySelectorAll('main > section:not(.door) > *, .coda');
  const io = new IntersectionObserver((es) => {
    for (const e of es) if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); }
  }, { threshold: 0.12, rootMargin: '0px 0px -40px 0px' });
  for (const el of items) { el.classList.add('reveal'); io.observe(el); }
}
