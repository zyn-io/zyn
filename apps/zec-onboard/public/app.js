import { identify, DESCRIBE } from './address.js';

const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

// Progress and network live in this browser only; the page works without them.
const KEY = 'zec-onboard';
function load() {
  try { return JSON.parse(localStorage.getItem(KEY)) || {}; } catch { return {}; }
}
function save(state) {
  try { localStorage.setItem(KEY, JSON.stringify(state)); } catch { /* private window: this visit only */ }
}
const state = { net: 'main', done: [], ...load() };
if (!Array.isArray(state.done)) state.done = [];

const EXPLORER = {
  main: (tx) => `https://mainnet.zcashexplorer.app/transactions/${tx}`,
  test: (tx) => `https://testnet.zcashexplorer.app/transactions/${tx}`,
};

// --- network ---------------------------------------------------------------

function setNet(net) {
  state.net = net === 'test' ? 'test' : 'main';
  document.documentElement.dataset.net = state.net;
  for (const b of $$('.net > button')) b.setAttribute('aria-pressed', String(b.dataset.net === state.net));
  save(state);
  if ($('#addr').value.trim()) checkAddress();
  if ($('#txid').value.trim()) lookUp();
}
for (const b of $$('.net > button')) b.addEventListener('click', () => setNet(b.dataset.net));

// --- progress --------------------------------------------------------------

function paint() {
  const done = new Set(state.done);
  for (const s of $$('.step')) s.classList.toggle('is-done', done.has(Number(s.dataset.step)));
  for (const a of $$('.rail a')) a.classList.toggle('is-done', done.has(Number(a.dataset.step)));
  for (const b of $$('button.done')) {
    const on = done.has(Number(b.dataset.step));
    b.setAttribute('aria-pressed', String(on));
    b.textContent = on ? `Step ${b.dataset.step} done` : `Mark step ${b.dataset.step} done`;
  }
  $('#count').textContent = `${done.size} of 6 done`;
}
for (const b of $$('button.done')) {
  b.addEventListener('click', () => {
    const n = Number(b.dataset.step);
    const done = new Set(state.done);
    if (done.has(n)) done.delete(n); else done.add(n);
    state.done = [...done].sort();
    save(state);
    paint();
    if (!done.has(n)) return;
    const next = $(`.step[data-step="${n + 1}"]`) || $('#finish');
    next.scrollIntoView({ block: 'start' });
  });
}
$('#reset').addEventListener('click', () => { state.done = []; save(state); paint(); });

// The rail marks the step in view.
const io = new IntersectionObserver((entries) => {
  for (const e of entries) {
    if (!e.isIntersecting) continue;
    for (const a of $$('.rail a')) a.classList.toggle('here', a.dataset.step === e.target.dataset.step);
  }
}, { rootMargin: '-40% 0px -55% 0px' });
for (const s of $$('.step')) io.observe(s);

// --- info bubbles ----------------------------------------------------------

function closeTips(except) {
  for (const b of $$('.i[aria-expanded="true"]')) {
    if (b === except) continue;
    b.setAttribute('aria-expanded', 'false');
    b.nextElementSibling.hidden = true;
  }
}
for (const b of $$('.i')) {
  b.addEventListener('click', (ev) => {
    ev.stopPropagation();
    const bubble = b.nextElementSibling;
    const open = b.getAttribute('aria-expanded') !== 'true';
    closeTips(b);
    b.setAttribute('aria-expanded', String(open));
    bubble.hidden = !open;
    if (open) {
      bubble.classList.remove('flip');
      if (bubble.getBoundingClientRect().right > document.documentElement.clientWidth - 8) bubble.classList.add('flip');
    }
  });
}
document.addEventListener('click', (ev) => { if (!ev.target.closest('.bubble')) closeTips(); });
document.addEventListener('keydown', (ev) => {
  if (ev.key !== 'Escape') return;
  const open = $('.i[aria-expanded="true"]');
  closeTips();
  open?.focus();
});

// --- address checker -------------------------------------------------------

const NET = { main: 'mainnet', test: 'testnet' };

async function checkAddress() {
  const out = $('#addr-out');
  const r = await identify($('#addr').value);
  if (!r.kind) {
    out.innerHTML = r.reason ? `<dt class="bad">${esc(r.reason)}</dt>` : '';
    return;
  }
  const d = DESCRIBE[r.kind];
  const other = r.net !== NET[state.net]
    ? `<dd class="bad">This is a ${esc(r.net)} address; this page is set to ${esc(NET[state.net])}. Coins on one network cannot be sent to the other.</dd>`
    : '';
  const publicKinds = new Set(['transparent', 'tex']);
  out.innerHTML = `
    <dt>${esc(r.name)}, ${esc(r.net)}</dt>${other}
    <dt>Pool</dt><dd>${esc(d.pool)}</dd>
    <dt>What the chain shows</dt><dd${publicKinds.has(r.kind) ? ' class="pub"' : ''}>${esc(d.seen)}</dd>
    <dt>From Nap</dt><dd>${esc(d.nap)}</dd>`;
}
$('#addr-form').addEventListener('submit', (ev) => { ev.preventDefault(); checkAddress(); });
$('#addr').addEventListener('input', () => { if (!$('#addr').value.trim()) $('#addr-out').innerHTML = ''; });
$('#addr').addEventListener('paste', () => setTimeout(checkAddress));

// --- transaction lookup ----------------------------------------------------

function lookUp() {
  const out = $('#tx-out');
  const tx = $('#txid').value.trim().toLowerCase();
  if (!tx) { out.textContent = ''; return; }
  if (!/^[0-9a-f]{64}$/.test(tx)) {
    out.innerHTML = '<span class="bad">A transaction id is 64 characters of 0–9 and a–f.</span>';
    return;
  }
  const url = EXPLORER[state.net](tx);
  out.innerHTML = `<a href="${esc(url)}" target="_blank" rel="noopener">Open in the ${state.net === 'test' ? 'testnet' : 'mainnet'} explorer</a>. A payment between two shielded addresses shows its time, fee and size, and nothing about the sender, the receiver or the amount.`;
}
$('#tx-form').addEventListener('submit', (ev) => { ev.preventDefault(); lookUp(); });
$('#txid').addEventListener('paste', () => setTimeout(lookUp));

// --- screenshots that are not there yet are left out -----------------------

for (const img of $$('.shot img')) {
  const hide = () => img.closest('.shot').classList.add('missing');
  if (img.complete && img.naturalWidth === 0) hide();
  img.addEventListener('error', hide);
}

setNet(state.net);
paint();
