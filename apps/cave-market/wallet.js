// The store's wallet connection: one button, a picker, and the session.
//
// Built on Nap Connect (`./nap-connect.js`, copied from apps/nap-connect):
// the picker lists every Nap this browser can reach — the extension with
// desktop Nap behind it, and Nap in the browser, which needs no install.
// Connecting tells the store an account and a Zcash address; everything that
// moves value is approved in Nap, never here.

import { connect, discover, NAP_WEB } from './nap-connect.js';

/// Which Zyn network the store trades on. `?network=testnet` for trying it.
export const NETWORK = new URLSearchParams(location.search).get('network') === 'testnet' ? 'testnet' : 'mainnet';
/// The chain's public RPC, for what anyone may read (the order book).
export const RPC = NETWORK === 'testnet' ? 'https://zyn-rpc.cavebro.win/object' : 'https://zyn-rpc.cavebro.win/';

/// Where Nap in the browser lives. Only a page served from this machine may
/// point it elsewhere (`?nap=http://localhost:4179`, for development): on the
/// real domain a link that swapped the wallet's address would be a phishing
/// kit, so the parameter is not read there at all.
const LOCAL = ['localhost', '127.0.0.1'].includes(location.hostname);
const NAP = (LOCAL && new URLSearchParams(location.search).get('nap')) || NAP_WEB;

const CHOICE_KEY = 'cave.wallet.v1';
const $ = (sel, root = document) => root.querySelector(sel);
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const short = (h) => `${h.slice(0, 6)}…${h.slice(-4)}`;

let wallets = [];
let current = null; // { info, provider, session }
const listeners = new Set();

export const wallet = () => current;
export function onWallet(fn) { listeners.add(fn); return () => listeners.delete(fn); }
const changed = () => { renderButtons(); for (const fn of listeners) { try { fn(current); } catch { /* one listener's fault */ } } };

function remembered() { try { return JSON.parse(localStorage.getItem(CHOICE_KEY) || 'null'); } catch { return null; } }
function rememberChoice(rdns) { try { rdns ? localStorage.setItem(CHOICE_KEY, JSON.stringify({ rdns })) : localStorage.removeItem(CHOICE_KEY); } catch { /* this visit only */ } }

/// Whether a request can go out without a click. Nap in the browser answers
/// through its own window, and only a click may open one.
export function canAskQuietly() {
  if (!current) return false;
  return !current.provider.isNapWeb || current.provider.isOpen();
}

/// Ask the connected Nap. Call it from a click when it may prompt.
export function ask(method, params) {
  if (!current) { openPicker(); return Promise.reject(new Error('Connect a wallet first.')); }
  return current.provider.request({ method, params });
}

// ---------------------------------------------------------------------------

async function choose(entry) {
  const dialog = $('#wallet-dialog');
  const status = $('.wallet-status', dialog);
  status.textContent = entry.provider.isNapWeb ? "Nap's window is open — unlock it there and approve." : 'Approve the connection in Nap.';
  status.dataset.tone = 'ok';
  try {
    current = await connect(entry, { network: NETWORK });
    rememberChoice(entry.info.rdns);
    entry.provider.on?.('disconnect', () => { if (current?.provider === entry.provider) { current = null; changed(); } });
    dialog.close();
    changed();
  } catch (e) {
    status.textContent = e.code === 4001 ? 'Not connected. Nothing was shared.' : e.message;
    status.dataset.tone = 'bad';
  }
}

async function disconnect() {
  const was = current;
  current = null;
  rememberChoice(null);
  changed();
  try { if (!was.provider.isNapWeb || was.provider.isOpen()) await was.provider.request({ method: 'nap_disconnect' }); } catch { /* already gone */ }
  was.provider.close?.();
}

export async function openPicker() {
  const dialog = $('#wallet-dialog');
  if (!dialog) return;
  if (!dialog.open) dialog.showModal();
  if (current) { renderConnected(dialog); return; }
  $('.wallet-body', dialog).innerHTML = '<p class="wallet-status">Looking for Nap…</p>';
  wallets = await discover({ web: NAP });
  const list = wallets.map((w, i) => `<button type="button" class="wallet-option" data-wallet="${i}">
      <img src="${esc(w.info.icon || './assets/nap-mark.png')}" alt="" width="36" height="36" />
      <span><b>${esc(w.info.name)}</b><small>${esc(w.info.detail || '')}</small></span><i aria-hidden="true">→</i></button>`).join('');
  $('.wallet-body', dialog).innerHTML = `${list}
    <p class="wallet-status" role="status"></p>
    <p class="fine-print">Connecting shows the store your Zyn account and a Zcash address — not your balances or keys. Buying, listing and paying are approved in Nap, one at a time.</p>`;
  dialog.querySelectorAll('[data-wallet]').forEach((b) => { b.onclick = () => choose(wallets[Number(b.dataset.wallet)]); });
}

function renderConnected(dialog) {
  const s = current.session;
  $('.wallet-body', dialog).innerHTML = `<dl class="wallet-facts">
      <div><dt>Wallet</dt><dd>${esc(current.info.name)}</dd></div>
      <div><dt>Zyn account</dt><dd><code>${esc(short(s.account))}</code></dd></div>
      ${s.zcash ? `<div><dt>Zcash</dt><dd><code>${esc(short(s.zcash.address))}</code></dd></div>` : ''}
      <div><dt>Network</dt><dd>${esc(s.network)}</dd></div>
    </dl>
    <button type="button" class="dialog-action wallet-disconnect">Disconnect</button>`;
  $('.wallet-disconnect', dialog).onclick = () => { disconnect(); dialog.close(); };
}

function renderButtons() {
  document.querySelectorAll('[data-wallet-button]').forEach((b) => {
    b.disabled = false;
    b.classList.toggle('is-connected', !!current);
    b.innerHTML = current
      ? `<span></span> ${esc(short(current.session.account))}`
      : '<span></span> Connect wallet';
  });
}

// ---------------------------------------------------------------------------

function wire() {
  const dialog = $('#wallet-dialog');
  // The old "not live yet" buttons become the connect button.
  document.querySelectorAll('[data-wallet-dialog], .market-wallet').forEach((b) => {
    b.removeAttribute('data-wallet-dialog');
    b.setAttribute('data-wallet-button', '');
    b.addEventListener('click', (e) => { e.preventDefault(); openPicker(); });
  });
  $('#wallet-dialog-title', dialog).innerHTML = 'Connect<br />a wallet.';
  const body = document.createElement('div');
  body.className = 'wallet-body';
  dialog.querySelectorAll(':scope > p:not(.eyebrow), :scope > .dialog-action').forEach((el) => el.remove());
  dialog.append(body);
  renderButtons();
}

/// An extension the holder already connected answers quietly on return.
/// Nap in the browser cannot: its window opens only from a click.
async function resume() {
  const choice = remembered();
  if (!choice) return;
  const list = await discover({ web: null });
  const entry = list.find((w) => w.info.rdns === choice.rdns);
  if (!entry) return;
  try {
    const session = await entry.provider.request({ method: 'nap_session' });
    if (session) { current = { ...entry, session }; changed(); }
  } catch { /* an older extension without sessions: connect again by hand */ }
}

wire();
resume();
