// Epeaky access: connect a Solana wallet (Wallet Standard), buy $EPEAKY with one
// of the fixed amounts, and have the buy let this device into the den. The den's
// page opened this window with the device's pass hash after "#"; this page
// never sees the pass code.
import { encode, message, moveMessage } from './shared.mjs';

const el = (id) => document.getElementById(id);
const CHAIN = 'solana:mainnet';
const NEED = ['standard:connect', 'solana:signAndSendTransaction', 'solana:signMessage'];
const WHY = {
  missing: 'The buy is not on chain yet.',
  failed: 'The buy failed on chain. Nothing was spent but the network fee.',
  old: 'That buy is more than 15 minutes old.',
  payer: 'This wallet did not make that buy.',
  small: 'The buy was under the smallest amount.',
  token: 'That transaction did not buy $EPEAKY.',
  signed: 'The wallet did not sign the message for this device.',
  used: 'This buy already let a device in.',
  den: 'The buy is fine, but the den did not answer. Press Check again.',
  pass: 'Open this from the den\'s page: press Get access with $EPEAKY there.',
  none: 'This wallet has no earlier buy here.',
  stale: 'The signature took too long. Try again.',
};

// The pass hash rides after "#"; kept for this tab, so a reload of the window keeps it.
let pass = new URLSearchParams(location.hash.slice(1)).get('den-pass');
try { if (pass) sessionStorage.setItem('epeaky-access-pass', pass); else pass = sessionStorage.getItem('epeaky-access-pass'); } catch { /* private window */ }
history.replaceState(null, '', location.pathname);
let config = null;
let wallet = null;
let account = null;
let busy = false;

function say(text, how = '') {
  el('say').textContent = text;
  el('say').className = `say ${how}`;
}

const tell = (data) => { try { window.opener?.postMessage({ den: 'pass', ...data }, config.den); } catch { /* the den's page is gone */ } };
const short = (a) => `${a.slice(0, 4)}…${a.slice(-4)}`;
const tokens = (raw) => new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 2 }).format(Number(raw) / 1e6);

// A buy that was sent but not yet let in survives a reload of this window.
const KEPT = 'epeaky-access-claim';
const keep = (c) => { try { c ? localStorage.setItem(KEPT, JSON.stringify(c)) : localStorage.removeItem(KEPT); } catch { /* private window */ } };
const kept = () => { try { const c = JSON.parse(localStorage.getItem(KEPT)); return c?.pass === pass ? c : null; } catch { return null; } };

// ---- Wallets -------------------------------------------------------------

const found = [];
const api = Object.freeze({
  register: (...ws) => {
    for (const w of ws) if (NEED.every((f) => w.features?.[f]) && w.chains?.includes(CHAIN) && !found.includes(w)) found.push(w);
    return () => {};
  },
});
addEventListener('wallet-standard:register-wallet', (e) => { try { e.detail(api); } catch { /* a broken wallet */ } });
dispatchEvent(new CustomEvent('wallet-standard:app-ready', { detail: api }));

async function use(w) {
  const { accounts } = await w.features['standard:connect'].connect();
  const a = accounts.find((x) => x.chains?.includes(CHAIN)) || accounts[0];
  if (!a) throw new Error('The wallet gave no account.');
  wallet = w;
  account = a;
  el('wallets').hidden = true;
  el('connect').hidden = true;
  el('account').hidden = false;
  el('account').innerHTML = '';
  el('account').append('Connected: ', Object.assign(document.createElement('b'), { textContent: short(a.address) }), ` (${w.name})`);
  for (const b of document.querySelectorAll('.amount')) b.disabled = busy;
  el('moving').hidden = false;
  say('');
}

el('connect').onclick = () => {
  if (!found.length) return say('No Solana wallet found in this browser. Install one (Phantom, Solflare or Backpack) and reload this window.', 'bad');
  if (found.length === 1) return use(found[0]).catch((e) => say(e?.message || 'The wallet did not connect.', 'bad'));
  const list = el('wallets');
  list.innerHTML = '';
  for (const w of found) {
    const b = document.createElement('button');
    b.type = 'button';
    if (w.icon) b.append(Object.assign(document.createElement('img'), { src: w.icon, alt: '' }));
    b.append(w.name);
    b.onclick = () => use(w).catch((e) => say(e?.message || 'The wallet did not connect.', 'bad'));
    list.append(b);
  }
  list.hidden = false;
};

// ---- Amounts ---------------------------------------------------------------

async function quotes() {
  await Promise.all([...document.querySelectorAll('.amount')].map(async (b) => {
    const q = await fetch(`/api/quote?sol=${b.dataset.sol}`).then((r) => r.json()).catch(() => null);
    b.querySelector('span').textContent = q?.out
      ? `About ${tokens(q.out)} $EPEAKY · price impact ${(q.impact * 100).toFixed(1)}%`
      : 'No quote right now';
  }));
}

function amounts() {
  const box = el('amounts');
  for (const sol of config.amounts) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'amount';
    b.dataset.sol = sol;
    b.disabled = true;
    b.innerHTML = '<strong></strong><span>Getting a quote…</span>';
    b.querySelector('strong').textContent = `Buy with ${sol} SOL`;
    b.onclick = () => buy(sol);
    box.append(b);
  }
}

function lock(on) {
  busy = on;
  for (const b of document.querySelectorAll('.amount')) b.disabled = on || !account;
  el('move').disabled = on;
}

// ---- Buying and getting in ------------------------------------------------

async function buy(sol) {
  if (busy || !account) return;
  lock(true);
  try {
    say('Preparing the buy…');
    const s = await fetch('/api/swap', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ sol, wallet: account.address }) }).then((r) => r.json());
    if (!s.tx) throw new Error('Jupiter gave no buy right now. Try again in a moment.');
    say('Confirm the buy in your wallet.');
    const transaction = Uint8Array.from(atob(s.tx), (c) => c.charCodeAt(0));
    const [sent] = await wallet.features['solana:signAndSendTransaction'].signAndSendTransaction({ account, chain: CHAIN, transaction });
    const signature = encode(sent.signature);
    keep({ pass, signature, wallet: account.address });
    say('Now sign the message that ties the buy to this device.');
    const [m] = await wallet.features['solana:signMessage'].signMessage({ account, message: new TextEncoder().encode(message(pass, signature)) });
    const c = { pass, signature, wallet: account.address, signed: encode(m.signature) };
    keep(c);
    await claim(c);
  } catch (e) {
    const no = /reject|declin|cancel|denied/i.test(e?.message || '');
    say(no ? 'Not confirmed in the wallet.' : e?.message || 'The buy did not go through.', 'bad');
    if (kept()) offerAgain();
  } finally { lock(false); }
}

async function claim(c) {
  el('again').hidden = true;
  for (let i = 0; i < 20; i++) {
    say(i ? 'Waiting for the buy to land…' : 'Checking the buy…');
    const r = await fetch('/api/claim', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(c) })
      .then(async (x) => ({ status: x.status, ...(await x.json().catch(() => ({}))) }))
      .catch(() => ({ error: 'den' }));
    if (r.ok) {
      keep(null);
      say('You\'re in. This window closes now.', 'good');
      tell({ registered: true });
      setTimeout(() => close(), 1500);
      return;
    }
    if (r.error !== 'missing') {
      // The den or this page's server failing keeps the buy, to check again.
      const retry = r.error === 'den' || !WHY[r.error] || r.status >= 500;
      say(WHY[r.error] || 'The check did not finish. Press Check again.', 'bad');
      if (retry) offerAgain();
      else keep(null);
      return;
    }
    await new Promise((ok) => setTimeout(ok, 3000));
  }
  say('The buy has not landed yet. If your wallet shows it, press Check again.', 'bad');
  offerAgain();
}

function offerAgain() {
  const c = kept();
  if (!c?.signed) return;
  el('again').hidden = false;
  el('again').onclick = () => claim(c);
}

// A wallet that bought before: its access moves here, with no new buy.
el('move').onclick = async () => {
  if (busy || !account) return;
  lock(true);
  try {
    const at = Date.now();
    say('Sign the message in your wallet to move access to this device.');
    const [m] = await wallet.features['solana:signMessage'].signMessage({ account, message: new TextEncoder().encode(moveMessage(pass, account.address, at)) });
    say('Moving access…');
    const r = await fetch('/api/move', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ pass, wallet: account.address, at, signed: encode(m.signature) }) })
      .then(async (x) => ({ status: x.status, ...(await x.json().catch(() => ({}))) }))
      .catch(() => ({}));
    if (r.ok) {
      say('You\'re in. This window closes now.', 'good');
      tell({ registered: true });
      setTimeout(() => close(), 1500);
    } else if (r.error === 'soon') {
      say(`This wallet moved its access less than a day ago. Try again after ${new Date(r.next).toLocaleString()}.`, 'bad');
    } else {
      say(WHY[r.error] || 'The move did not finish. Try again.', 'bad');
    }
  } catch (e) {
    say(/reject|declin|cancel|denied/i.test(e?.message || '') ? 'Not signed in the wallet.' : e?.message || 'The move did not finish.', 'bad');
  } finally { lock(false); }
};

el('cancel').onclick = () => { tell({ error: true, cancelled: true }); close(); };

// ---- Start -----------------------------------------------------------------

config = await fetch('/api/config').then((r) => r.json()).catch(() => null);
if (!config) {
  say('This page could not load its settings. Reload the window.', 'bad');
} else if (!/^[0-9a-f]{64}$/.test(pass || '')) {
  el('wallet').hidden = true;
  el('buy').hidden = true;
  say(WHY.pass, 'bad');
} else {
  if (config.feeBps != null) el('fee-note').textContent = `$EPEAKY takes a ${config.feeBps / 100}% fee on every transfer, so you receive less than the quote.`;
  amounts();
  quotes();
  setInterval(quotes, 30_000);
  const c = kept();
  if (c?.signed) { say('A buy from earlier has not let this device in yet.'); offerAgain(); }
}
