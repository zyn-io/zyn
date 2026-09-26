// Nap in the toolbar — the extension's popup, a wallet in 360 × 600 — and
// the phone app's home screen, the same wallet full screen.
//
// It opens on what matters — the balance, four actions, what is held — and
// does the everyday things in place: receive, send, switch network, lock.
// Setting a wallet up (24 words want room) and the market and bridge open in
// a tab, the way other toolbar wallets do it. The unlocked keys live in the
// browser's memory-only session store (session.js), so reopening the popup
// does not ask for the password again until it has sat idle. On the phone
// there is no tab to hand setup to, so it happens here; Face ID or a
// fingerprint can open the wallet (§151), and the Nap browser is one tap away.
import QRCode from 'qrcode';
import { deriveCollectionAccount, deriveIdentity, forgetIdentity, newMnemonic, openSealedWallet, parseAmount, sealWallet, unhex, WAD } from './core.js';
import { deriveZecKeys, stopZec, ZecAccount } from './zec/client.js';
import { IN_APP } from './origin.js';
import { parsePaymentRequest } from './zip321.js';
import { activity, chooseNetwork, collectOwed, depositAddress, loadConfig, loadPocket, logActivity, payOnZyn, placeSwap, quoteSwap, resolveRecipient, revealExits, withdrawZec } from './pocket.js';
import { endSession, loadSession, saveSession } from './session.js';
import './popup.css';

const pop = document.querySelector('#pop');
const KEPT_KEY = 'nap.kept.v1';
const keptWallet = () => { try { return JSON.parse(localStorage.getItem(KEPT_KEY) || 'null'); } catch { return null; } };
const keepWallet = (blob) => { try { localStorage.setItem(KEPT_KEY, JSON.stringify(blob)); return true; } catch { return false; } };

// The phone app's native side: biometric unlock (the `napvault` plugin keeps
// the password behind Face ID / a fingerprint) and the Nap browser.
const vault = IN_APP && window.__TAURI__?.core?.invoke
  ? { call: (cmd, args = {}) => window.__TAURI__.core.invoke(`plugin:napvault|${cmd}`, args) }
  : null;
const vaultStatus = () => (vault ? vault.call('status').catch(() => null) : Promise.resolve(null));
const bioName = (kind) => ({ faceId: 'Face ID', touchId: 'Touch ID', opticId: 'Optic ID' }[kind] || 'fingerprint');
let napBrowser = null;
if (IN_APP && window.__TAURI__?.core?.invoke) {
  import('./nap-browser.js').then((m) => {
    napBrowser = m;
    return m.startBrowser(() => (identity && config ? { config, identity, zec } : null));
  }).catch(() => { napBrowser = null; });
}

let config = null;
let identity = null;
let receiving = [];
let pocket = null; // { epoch, coins, items, held }
let pocketError = '';
let zec = null; // ZecAccount
let zecView = null;
let zecState = ''; // a short status line
let tab = 'tokens';
let zecFresh = false; // a wallet made just now has nothing to scan back for
let generation = 0;
let timer = 0;

// ---------------------------------------------------------------------------
// Small helpers.

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const short = (s, a = 6, b = 4) => (s.length > a + b + 1 ? `${s.slice(0, a)}…${s.slice(-b)}` : s);
const $ = (sel, el = pop) => el.querySelector(sel);
const ZAT = 100_000_000n;
function zecText(zat) {
  const v = BigInt(zat);
  const frac = (v % ZAT).toString().padStart(8, '0').replace(/0+$/, '');
  return `${(v / ZAT).toLocaleString('en-US')}${frac ? `.${frac}` : ''}`;
}
function wadText(v, places = 6) {
  const whole = (v / WAD).toLocaleString('en-US');
  const frac = (v % WAD).toString().padStart(18, '0').slice(0, places).replace(/0+$/, '');
  return frac ? `${whole}.${frac}` : whole;
}
function zatOf(text) {
  if (!/^\d+(\.\d{1,8})?$/.test(text)) throw new Error('Enter an amount in ZEC, up to eight decimals');
  const [w, f = ''] = text.split('.');
  const zat = BigInt(w) * ZAT + BigInt(f.padEnd(8, '0'));
  if (zat <= 0n) throw new Error('Enter an amount above zero');
  return Number(zat);
}
const isTest = () => config.zcashPrefix !== 'u1';
const netLabel = (n) => (n.zcashPrefix === 'u1' ? 'Mainnet' : 'Testnet');

function toast(text, bad = false) {
  pop.querySelector('.toast')?.remove();
  const t = document.createElement('div');
  t.className = `toast${bad ? ' bad' : ''}`;
  t.textContent = text;
  pop.append(t);
  setTimeout(() => t.remove(), bad ? 4200 : 2200);
}
async function copy(text, what) {
  try { await navigator.clipboard.writeText(text); toast(`${what} copied`); } catch { toast('Could not copy', true); }
}

/// Open one of Nap's pages in a tab and close the popup behind it — or, in
/// the phone app, go to it.
function openTab(path) {
  if (IN_APP) { location.href = `/${path}`; return; }
  const url = globalThis.chrome?.runtime?.getURL ? chrome.runtime.getURL(path) : path;
  if (globalThis.chrome?.tabs?.create) chrome.tabs.create({ url }); else window.open(url, '_blank');
  window.close();
}

/// A face for the account, drawn from its id: a mirrored 5×5 of two tones,
/// so two accounts are easy to tell apart at a glance. Not a security check.
function drawAvatar(canvas, accountHex) {
  const b = accountHex.match(/../g).map((h) => parseInt(h, 16));
  const ctx = canvas.getContext('2d');
  canvas.width = 60; canvas.height = 60;
  const hue = (b[0] * 360) / 256;
  ctx.fillStyle = `hsl(${hue} 42% 22%)`; ctx.fillRect(0, 0, 60, 60);
  ctx.fillStyle = `hsl(${(hue + 40) % 360} 78% 62%)`;
  for (let y = 0; y < 5; y++) for (let x = 0; x < 3; x++) {
    if (b[1 + y * 3 + x] & 1) { ctx.fillRect(6 + x * 10, 6 + y * 10, 10, 10); ctx.fillRect(6 + (4 - x) * 10, 6 + y * 10, 10, 10); }
  }
}

/// A coin's disc: amber for ZEC, a steady colour per symbol otherwise.
function tokenDisc(el, symbol) {
  const s = String(symbol);
  if (/^ZEC/.test(s)) { el.style.background = 'linear-gradient(145deg,#f4c24a,#c98a06)'; return; }
  let h = 0; for (const c of s) h = (h * 31 + c.charCodeAt(0)) % 360;
  el.style.background = `linear-gradient(145deg,hsl(${h} 60% 66%),hsl(${(h + 30) % 360} 55% 44%))`;
}

const ICON = {
  receive: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 4v13M6 11l6 6 6-6M5 20h14"/></svg>',
  send: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20V7M6 13l6-6 6 6M5 4h14"/></svg>',
  swap: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M7 4v14M3 14l4 4 4-4M17 20V6M13 10l4-4 4 4"/></svg>',
  bridge: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 17h18M5 17v-4a7 7 0 0 1 14 0v4M9 17v-3M15 17v-3"/></svg>',
  back: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 5l-7 7 7 7"/></svg>',
  close: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M6 6l12 12M18 6L6 18"/></svg>',
  menu: '<svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor"><circle cx="5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="19" cy="12" r="1.8"/></svg>',
  browse: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/></svg>',
  expand: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 4h6v6M20 4l-7 7M10 20H4v-6M4 20l7-7"/></svg>',
};

// ---------------------------------------------------------------------------
// Gates: no wallet on this device, or a locked one.

function welcome() {
  pop.innerHTML = `<div class="gate">
    <img class="mark" src="/nap-mark.png" alt="">
    <h1>Nap <em>Wallet</em></h1>
    <p class="lead">Shielded ZEC, and Zyn tokens and items, in one private wallet.</p>
    <ul class="facts"><li>Your keys are made and kept on this ${IN_APP ? 'phone' : 'browser'}, sealed under your password.</li><li>No sign-up, no email. Your keys never leave this ${IN_APP ? 'phone' : 'browser'}.</li><li>The same 24 words open Nap ${IN_APP ? 'on the web and in the browser extension' : 'on the web and on your phone'}.</li></ul>
    <div class="grow"></div>
    <div class="gate-actions" role="group">
      <button class="btn" id="create">Create a new wallet</button>
      <div class="foot"><button class="link" id="restore">I already have a recovery phrase</button></div>
    </div>
  </div>`;
  $('#create').onclick = () => (IN_APP ? createWallet() : openTab('index.html?setup=create'));
  $('#restore').onclick = () => (IN_APP ? restoreWallet() : openTab('index.html?setup=restore'));
}

// Setting up in place — the phone app only; the extension does it in a tab.

const knownCollections = () => (config.collections || [])
  .filter((c) => /^[0-9a-f]{64}$/i.test(String(c.id)))
  .map((c) => ({ name: String(c.name || 'Collection'), id: unhex(String(c.id).toLowerCase(), 32) }));

/// From words to a kept, open wallet: the Zyn keys, the Zcash keys, one
/// receiving account per collection, sealed under the password. The phrase
/// goes no further than this function.
async function setUp(words, passphrase, index, password, fresh) {
  if (String(password).length < 8) throw new Error('Choose a password of at least 8 characters');
  const nextIdentity = deriveIdentity(words, passphrase, index);
  try { nextIdentity.zec = await deriveZecKeys(words, passphrase, index); } catch { nextIdentity.zec = undefined; }
  const nextReceiving = knownCollections().map((c) => ({ ...deriveCollectionAccount(words, passphrase, index, c.id), name: c.name }));
  const sealed = await sealWallet(nextIdentity, nextReceiving, password, index);
  if (!keepWallet(sealed)) throw new Error('This device would not store the wallet');
  await saveSession(nextIdentity, nextReceiving);
  zecFresh = fresh;
  begin({ identity: nextIdentity, receiving: nextReceiving });
  offerBiometrics(password);
}

function createWallet() {
  const words = newMnemonic();
  const s = screen('New wallet', `
    <div class="warn"><b>Write these 24 words down</b>, in order, on paper. They are the wallet: anyone with them can take everything, and without them a lost phone loses it all. Nap never sees them again.</div>
    <div class="words">${words.split(' ').map((w, i) => `<span><i>${i + 1}</i>${esc(w)}</span>`).join('')}</div>
    <label class="check"><input type="checkbox" id="c-ack"><span>I wrote all 24 words down offline — not a screenshot, not a cloud note.</span></label>
    <label class="field"><span>Password for this phone (8+ characters)</span><input id="c-pw" type="password" autocomplete="new-password" minlength="8"></label>
    <p class="hint" id="c-err" role="alert">The password opens Nap on this phone. It is not the phrase, and it cannot restore the wallet anywhere else.</p>`,
    '<button class="btn" id="c-go" disabled>Create wallet</button>');
  const go = $('#c-go', s);
  $('#c-ack', s).onchange = (e) => { go.disabled = !e.target.checked; };
  go.onclick = async () => {
    go.disabled = true; go.textContent = 'Creating…';
    try { await setUp(words, '', 0, $('#c-pw', s).value, true); }
    catch (e) { $('#c-err', s).textContent = e.message; $('#c-err', s).classList.add('bad'); go.disabled = false; go.textContent = 'Create wallet'; }
  };
}

function restoreWallet() {
  const s = screen('Restore', `
    <label class="field"><span>Recovery phrase (24 words)</span><textarea id="r-words" rows="5" autocomplete="off" autocapitalize="none" autocorrect="off" spellcheck="false"></textarea></label>
    <details class="more"><summary>Passphrase or another account? ›</summary><div class="two">
      <label class="field"><span>Passphrase</span><input id="r-pass" type="password" autocomplete="off" placeholder="if you used one"></label>
      <label class="field"><span>Account</span><input id="r-index" type="number" min="0" value="0" inputmode="numeric"></label>
    </div></details>
    <label class="field"><span>Password for this phone (8+ characters)</span><input id="r-pw" type="password" autocomplete="new-password" minlength="8"></label>
    <p class="hint" id="r-err" role="alert">Your phrase is used here to derive the keys, then dropped; only the keys are kept, sealed under the password.</p>`,
    '<button class="btn" id="r-go">Restore wallet</button>');
  const go = $('#r-go', s);
  go.onclick = async () => {
    go.disabled = true; go.textContent = 'Restoring…';
    try {
      const words = $('#r-words', s).value;
      await setUp(words, $('#r-pass', s).value, Number($('#r-index', s).value) || 0, $('#r-pw', s).value, false);
    } catch (e) { $('#r-err', s).textContent = e.message; $('#r-err', s).classList.add('bad'); go.disabled = false; go.textContent = 'Restore wallet'; }
  };
}

/// After a password opened (or set up) the wallet: offer Face ID, once.
async function offerBiometrics(password) {
  const st = await vaultStatus();
  if (!st?.available || st.stored || !password) return;
  const name = bioName(st.kind);
  setTimeout(() => {
    if (!identity) return;
    sheet(`<button id="b-yes">Open Nap with ${esc(name)}<span>next time</span></button><button id="b-no">Not now</button>`, (el) => {
      $('#b-no', el).onclick = () => { closeSheet(); password = ''; };
      $('#b-yes', el).onclick = async () => {
        closeSheet();
        try { await vault.call('store', { secret: password }); toast(`${name} opens Nap from now on`); }
        catch (e) { if (!/cancel/i.test(String(e))) toast(String(e).slice(0, 120), true); }
        finally { password = ''; }
      };
    });
  }, 900);
}

function lockScreen(error = '') {
  pop.innerHTML = `<div class="gate">
    <img class="mark" src="/nap-mark.png" alt="">
    <h1>Welcome back</h1>
    <p class="lead">Your wallet is sealed on this device.</p>
    <div class="grow"></div>
    <form id="unlock">
      <label class="field"><span>Password</span><input id="pw" type="password" autocomplete="current-password" autofocus></label>
      <p class="hint bad" id="err" role="alert">${esc(error)}</p>
      <button class="btn" type="submit">Unlock</button>
    </form>
    <div class="foot"><button class="link" id="phrase">Forgot it? Restore with your phrase</button></div>
  </div>`;
  if (!IN_APP) $('#pw').focus();
  $('#phrase').onclick = () => (IN_APP ? restoreWallet() : openTab('index.html?setup=restore'));
  void biometricUnlock();
  $('#unlock').onsubmit = async (e) => {
    e.preventDefault();
    const button = $('#unlock .btn');
    button.disabled = true; button.textContent = 'Unlocking…';
    try {
      const password = $('#pw').value;
      const opened = await openSealedWallet(keptWallet(), password);
      await saveSession(opened.identity, opened.receiving);
      begin(opened);
      offerBiometrics(password);
    } catch (err) {
      $('#err').textContent = err.message;
      button.disabled = false; button.textContent = 'Unlock';
      $('#pw').select();
    }
  };
}

/// On the lock screen: Face ID / a fingerprint, asked for once, when this
/// phone keeps the password behind it.
async function biometricUnlock() {
  const st = await vaultStatus();
  const form = $('#unlock');
  if (!st?.available || !st.stored || !form) return;
  const name = bioName(st.kind);
  form.insertAdjacentHTML('afterbegin', `<button type="button" class="btn" id="bio">Unlock with ${esc(name)}</button>`);
  const go = async () => {
    try {
      const { secret } = await vault.call('unlock', { reason: 'Open your Nap wallet' });
      try {
        const opened = await openSealedWallet(keptWallet(), secret);
        await saveSession(opened.identity, opened.receiving);
        begin(opened);
      } catch {
        await vault.call('forget').catch(() => {});
        $('#err').textContent = `${name} no longer matches this wallet; use your password.`;
        $('#bio')?.remove();
      }
    } catch (e) { if (!/cancel/i.test(String(e))) $('#err').textContent = String(e).slice(0, 160); }
  };
  $('#bio').onclick = go;
  go();
}

function lock() {
  napBrowser?.closeBrowser();
  generation++;
  clearTimeout(timer);
  stopZec(); zec = null; zecView = null;
  forgetIdentity(identity); identity = null; receiving = []; pocket = null;
  void endSession();
  lockScreen();
}

// ---------------------------------------------------------------------------
// The wallet.

function begin(opened) {
  identity = opened.identity;
  receiving = opened.receiving || [];
  pocket = null; pocketError = ''; zecView = null; zecState = '';
  generation++;
  home();
  refresh();
  startZec();
}

function home() {
  const acct = identity.accountHex;
  pop.innerHTML = `
    <header class="bar">
      <button class="acct" id="acct" title="Copy your Zyn account id"><canvas></canvas><div><b>Account ${identity.accountIndex + 1}</b><span>${esc(short(acct, 6, 4))}</span></div></button>
      <button class="net${isTest() ? ' test' : ''}" id="net" aria-haspopup="menu"><i></i>${esc(netLabel(config))}</button>
      <button class="icon-btn" id="menu" aria-label="Menu" aria-haspopup="menu">${ICON.menu}</button>
    </header>
    <div class="body" id="body">
      <section class="hero">
        <span class="label"><i class="shield"></i>Shielded ZEC</span>
        <div class="amount" id="zec-amount">—</div>
        <div class="sub" id="zec-sub">Opening your Zcash wallet…</div>
      </section>
      <nav class="actions">
        <button class="act" id="a-receive"><span>${ICON.receive}</span>Receive</button>
        <button class="act" id="a-send"><span>${ICON.send}</span>Send</button>
        <button class="act" id="a-swap"><span>${ICON.swap}</span>Swap</button>
        <button class="act" id="a-bridge"><span>${ICON.bridge}</span>Bridge</button>
        ${IN_APP ? `<button class="act" id="a-browse"><span>${ICON.browse}</span>Browser</button>` : ''}
      </nav>
      <div class="tabs" role="tablist">
        <button role="tab" data-tab="tokens" aria-selected="${tab === 'tokens'}">Tokens</button>
        <button role="tab" data-tab="items" aria-selected="${tab === 'items'}">Items<span class="count" id="item-count"></span></button>
        <button role="tab" data-tab="activity" aria-selected="${tab === 'activity'}">Activity</button>
      </div>
      <div id="panel"></div>
    </div>`;
  drawAvatar($('#acct canvas'), acct);
  $('#acct').onclick = () => copy(acct, 'Account id');
  $('#net').onclick = networkMenu;
  $('#menu').onclick = mainMenu;
  $('#a-receive').onclick = () => receive();
  $('#a-send').onclick = () => sendPick();
  $('#a-swap').onclick = () => swap();
  $('#a-bridge').onclick = () => openTab('bridge.html');
  $('#a-browse')?.addEventListener('click', () => (napBrowser ? napBrowser.openBrowser('').catch((e) => toast(String(e).slice(0, 120), true)) : toast('The browser is not available here', true)));
  pop.querySelectorAll('[data-tab]').forEach((b) => { b.onclick = () => { tab = b.dataset.tab; pop.querySelectorAll('[data-tab]').forEach((x) => x.setAttribute('aria-selected', String(x === b))); renderPanel(); }; });
  renderZec();
  renderPanel();
}

function renderZec() {
  const amount = $('#zec-amount');
  const sub = $('#zec-sub');
  if (!amount) return;
  if (!identity.zec) {
    amount.innerHTML = '—';
    sub.textContent = 'Zcash keys not on this device — add them in the full view.';
    return;
  }
  if (zecView) {
    const held = zecView.balance.orchard + zecView.balance.ironwood;
    amount.innerHTML = `${esc(zecText(held))}<small>ZEC</small>`;
  }
  const pending = zecView?.balance.inFlight ? ` · ${zecText(zecView.balance.inFlight)} on its way` : '';
  sub.textContent = zecState ? `${zecState}${pending}` : pending.slice(3);
  sub.classList.toggle('bad', /unreachable|failed|error|not /i.test(zecState));
}

function renderPanel() {
  const panel = $('#panel');
  if (!panel) return;
  const count = $('#item-count');
  if (count) count.textContent = pocket?.items.length ? String(pocket.items.length) : '';
  if (tab === 'tokens') panel.innerHTML = tokensHtml();
  else if (tab === 'items') panel.innerHTML = itemsHtml();
  else panel.innerHTML = activityHtml();
  panel.querySelectorAll('.tok[data-sym]').forEach((el) => tokenDisc(el, el.dataset.sym));
  panel.querySelectorAll('[data-coin]').forEach((el) => { el.onclick = () => sendForm(assetOptions().find((o) => o.key === el.dataset.coin)); });
  panel.querySelectorAll('[data-item]').forEach((el) => { el.onclick = () => sendForm(assetOptions().find((o) => o.key === `item:${el.dataset.item}`)); });
  $('#reveal-old')?.addEventListener('click', () => openTab('index.html'));
}

function tokensHtml() {
  const rows = [];
  if (identity.zec) {
    const held = zecView ? zecView.balance.orchard + zecView.balance.ironwood : null;
    rows.push(`<li><button class="row" data-coin="zec"><span class="tok" data-sym="ZEC">Z</span><span class="who"><b>ZEC</b><span>Zcash · shielded</span></span><span class="val"><b>${held === null ? '…' : esc(zecText(held))}</b><span class="teal">private</span></span></button></li>`);
  }
  if (pocketError) rows.push(`<li class="empty"><b>Zyn is not answering.</b>${esc(pocketError)}</li>`);
  else if (!pocket) rows.push('<li class="skeleton"></li><li class="skeleton"></li>');
  else {
    for (const c of pocket.coins) {
      rows.push(`<li><button class="row" data-coin="${esc(c.asset)}"><span class="tok" data-sym="${esc(c.symbol)}">${esc(c.symbol.replace(/^LP /, '').slice(0, 1))}</span><span class="who"><b>${esc(c.symbol)}</b><span>${esc(c.origin === 'Zyn' ? 'on Zyn' : `${c.origin} · on Zyn`)}</span></span><span class="val"><b>${esc(wadText(c.amount))}</b><span>${esc(config.network)}</span></span></button></li>`);
    }
    if (pocket.old) rows.push('<li class="empty"><b>Balances need your phrase once.</b>This wallet was kept before the object ledger. <button class="link" id="reveal-old">Open the full view</button></li>');
  }
  return `<ul class="rows">${rows.join('')}</ul>`;
}

function itemsHtml() {
  if (pocketError) return `<div class="empty"><b>Zyn is not answering.</b>${esc(pocketError)}</div>`;
  if (!pocket) return '<div class="grid"><div class="skeleton"></div><div class="skeleton"></div></div>';
  if (!pocket.items.length) return '<div class="empty"><img src="/nap-mark.png" alt=""><b>No items yet.</b>What you buy in the Cave store, or collect at an event, lands here.</div>';
  return `<div class="grid">${pocket.items.map((i) => `<button class="card" data-item="${esc(i.asset)}"><div class="art"><img src="/nap-mark.png" alt=""></div><div><b>${esc(i.name)}</b><span>send →</span></div></button>`).join('')}</div>`;
}

function activityHtml() {
  const mine = activity(config, identity).map((a) => `<li class="row"><span class="tok" data-sym="${esc(a.symbol || '?')}">${a.kind === 'in' ? '↓' : '↑'}</span><span class="who"><b>${esc(a.title)}</b><span>${esc(a.to ? `to ${short(a.to, 8, 6)}` : '')}</span></span><span class="val"><b>${esc(a.amount || '')}</b><span>${esc(new Date(a.at).toLocaleDateString('en-US', { month: 'short', day: 'numeric' }))}</span></span></li>`);
  const received = (zecView?.notes || []).slice(0, 20).map((n) => `<li class="row"><span class="tok" data-sym="ZEC">↓</span><span class="who"><b>Received ZEC</b><span>block ${Number(n.height).toLocaleString('en-US')}</span></span><span class="val"><b>+${esc(zecText(n.zatoshi))}</b><span class="teal">shielded</span></span></li>`);
  const all = [...mine, ...received];
  return all.length ? `<ul class="rows">${all.join('')}</ul>` : '<div class="empty"><b>Nothing yet.</b>Sends from this wallet and ZEC it receives show up here. Zyn keeps no public history of your account — that is the point.</div>';
}

// ---------------------------------------------------------------------------
// Reading the chains.

async function refresh() {
  const g = generation;
  clearTimeout(timer);
  try {
    const next = await loadPocket(config, identity);
    if (g !== generation) return;
    pocket = next; pocketError = '';
    void revealExits(config, identity);
    if (next.owed?.length && !collecting) void collect(next);
  } catch (e) {
    if (g !== generation) return;
    pocketError = e.message === 'Failed to fetch' ? 'Check your connection; your keys are safe.' : e.message;
  }
  renderPanel();
  timer = setTimeout(refresh, 20_000);
}

// Finished swaps pay into claims; the wallet collects them itself.
let collecting = false;
async function collect(p) {
  const g = generation;
  collecting = true;
  try {
    await collectOwed(config, identity, p.owed);
    if (g !== generation) return;
    const byAsset = new Map();
    for (const o of p.owed) byAsset.set(o.asset, (byAsset.get(o.asset) || 0n) + BigInt(o.amount));
    toast(`Received ${[...byAsset].map(([a, v]) => `${wadText(v)} ${p.nameOf(a)}`).join(', ')}`);
    setTimeout(refresh, 1500);
  } catch { /* tried again on the next pass */ } finally { collecting = false; }
}

async function startZec() {
  if (!identity.zec || !config.lightdUrl) { renderZec(); return; }
  const g = generation;
  const net = config.zcashPrefix === 'u1' ? 'mainnet' : 'testnet';
  const account = new ZecAccount({ sk: identity.zec[net], network: net, url: config.lightdUrl, birthday: config.zecBirthday });
  zec = account;
  try {
    zecView = await account.peek(); // what this device already knows: instant
    if (g !== generation) return;
    zecState = 'catching up…'; renderZec(); if (tab !== 'items') renderPanel();
    zecView = await account.open({ fresh: zecFresh });
    zecFresh = false;
    zecView = await account.sync((s) => {
      if (g !== generation || !s.to) return;
      zecState = `scanning ${Number(s.at).toLocaleString('en-US')} / ${Number(s.to).toLocaleString('en-US')}`;
      renderZec();
    });
    if (g !== generation) return;
    zecState = zecView.syncedTo >= 0 ? `synced · block ${Number(zecView.syncedTo).toLocaleString('en-US')}` : '';
  } catch (e) {
    if (g !== generation) return;
    zecState = e.message === 'Failed to fetch' ? 'block server unreachable' : e.message;
  }
  renderZec();
  if (tab !== 'items') renderPanel();
}

// ---------------------------------------------------------------------------
// Menus.

function sheet(html, bind) {
  closeSheet();
  pop.insertAdjacentHTML('beforeend', `<div class="scrim" id="scrim"></div><div class="sheet" role="menu">${html}</div>`);
  $('#scrim').onclick = closeSheet;
  bind($('.sheet'));
}
function closeSheet() { $('#scrim')?.remove(); $('.sheet')?.remove(); }

function networkMenu() {
  sheet(config.networks.map((n) => `<button role="menuitemradio" data-net="${esc(n.id)}" class="${n.id === config.id ? 'on' : ''}">${esc(n.network)}<span>${n.id === config.id ? '● current' : esc(n.zcash)}</span></button>`).join(''), (el) => {
    el.querySelectorAll('[data-net]').forEach((b) => {
      b.onclick = () => {
        closeSheet();
        if (b.dataset.net === config.id) return;
        config = chooseNetwork(config, b.dataset.net);
        generation++; clearTimeout(timer); stopZec(); zec = null; zecView = null; zecState = ''; pocket = null; pocketError = '';
        home(); refresh(); startZec();
        toast(`Switched to ${config.network}`);
      };
    });
  });
}

function mainMenu() {
  sheet(`<button data-go="${IN_APP ? 'index.html?full=1' : 'index.html'}">Open full view ${ICON.expand}</button>
    <button data-go="market.html">Market<span>swap · launch</span></button>
    <button data-go="bridge.html">Bridge<span>SOL · ETH · BTC</span></button>
    <hr>
    <button id="m-copy">Copy Zyn account id<span>${esc(short(identity.accountHex, 6, 4))}</span></button>
    <button id="m-recovery">Recovery<span>phrase</span></button>
    <hr>
    <button id="m-lock">Lock<span>${IN_APP ? 'this phone' : 'auto after 15 min'}</span></button>`, (el) => {
    el.querySelectorAll('[data-go]').forEach((b) => { b.onclick = () => openTab(b.dataset.go); });
    $('#m-copy', el).onclick = () => { closeSheet(); copy(identity.accountHex, 'Account id'); };
    $('#m-recovery', el).onclick = () => { closeSheet(); recovery(); };
    $('#m-lock', el).onclick = () => { closeSheet(); lock(); };
  });
}

function screen(title, body, foot = '') {
  pop.querySelector('.screen')?.remove();
  pop.insertAdjacentHTML('beforeend', `<section class="screen"><header class="screen-head"><button class="icon-btn" id="s-back" aria-label="Back">${ICON.back}</button><h1>${esc(title)}</h1><span></span></header><div class="screen-body">${body}</div>${foot ? `<div class="screen-foot">${foot}</div>` : ''}</section>`);
  const s = pop.querySelector('.screen');
  $('#s-back', s).onclick = () => s.remove();
  return s;
}

function recovery() {
  screen('Recovery', `<div class="big-amount"><b>24 words</b><span>are this wallet</span></div>
    <p class="hint">Nap never stores your recovery phrase — this browser holds only the keys, sealed under your password. Restore the same phrase, passphrase and account ${identity.accountIndex} in any Nap to recover this wallet.</p>
    <div class="review"><span>Zyn account</span><b class="mono">${esc(short(identity.accountHex, 10, 8))}</b><span>Derivation</span><b class="mono">nap.zyn.ed25519.v1</b><span>Account index</span><b class="mono">${identity.accountIndex}</b></div>
    <p class="hint">Lost the words? Anything still here can be moved to a new wallet while this one is unlocked.</p>`);
}

// ---------------------------------------------------------------------------
// Receive.

function receive(which = identity.zec ? 'zcash' : 'zyn') {
  const s = screen('Receive', `<div class="seg" role="group">
      ${identity.zec ? '<button data-r="zcash">Zcash</button>' : ''}<button data-r="deposit">ZEC → Zyn</button><button data-r="zyn">Zyn account</button>
    </div>
    <div class="qr"><canvas></canvas></div>
    <div class="address" id="r-address">…</div>
    <p class="hint" id="r-hint"></p>`, '<button class="btn" id="r-copy" disabled>Copy</button><button class="btn ghost hidden" id="r-fund">Pay it from my ZEC</button>');
  const show = async (kind) => {
    s.querySelectorAll('[data-r]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.r === kind)));
    const addr = $('#r-address', s), hint = $('#r-hint', s), btn = $('#r-copy', s), fundBtn = $('#r-fund', s);
    const canvas = $('canvas', s);
    btn.disabled = true; fundBtn.classList.add('hidden'); addr.textContent = '…'; canvas.getContext('2d').clearRect(0, 0, canvas.width, canvas.height);
    let text = '', what = '';
    try {
      if (kind === 'zcash') {
        if (!zecView) throw new Error('Your Zcash wallet is still opening.');
        text = zecView.address; what = 'Zcash address';
        hint.textContent = `A shielded ${config.zcash} address. Pay it from any Zcash wallet; only you can see what arrives.`;
      } else if (kind === 'deposit') {
        hint.textContent = 'Asking Zyn for your deposit address…';
        text = await depositAddress(config, identity); what = 'Deposit address';
        hint.textContent = `Send shielded ZEC here and it arrives on ${config.network} as ZEC.zy. No memo needed; this address is yours alone.`;
        if (identity.zec && zecView && zecView.balance.orchard + zecView.balance.ironwood > 0) fundBtn.classList.remove('hidden');
      } else {
        text = identity.accountHex; what = 'Account id';
        hint.textContent = 'For Zyn tokens and items from another Zyn wallet. Not a Zcash address — ZEC sent here is lost.';
      }
    } catch (e) { hint.textContent = e.message; hint.classList.add('bad'); addr.textContent = 'Unavailable'; return; }
    hint.classList.remove('bad');
    addr.textContent = text;
    await QRCode.toCanvas(canvas, text, { width: 336, margin: 1, color: { dark: '#15181b', light: '#f8f2e4' } });
    btn.disabled = false; btn.onclick = () => copy(text, what);
    fundBtn.onclick = () => sendForm(assetOptions().find((o) => o.key === 'zec'), { to: text });
  };
  s.querySelectorAll('[data-r]').forEach((b) => { b.onclick = () => show(b.dataset.r); });
  show(which);
}

// ---------------------------------------------------------------------------
// Swap: pay, receive, one tap. The full market (pools, launchpad) is in the
// menu; this is the everyday trade.

let swapPrefs = { pay: null, get: null, slip: 100 };

function swap() {
  if (!pocket) { toast('Still reading your balances…'); return; }
  const pools = pocket.pools || [];
  if (!pools.length) {
    screen('Swap', `<div class="empty"><img src="/nap-mark.png" alt=""><b>No pools on ${esc(config.network)} yet.</b>Pools and launches are in the full market.</div>`, '<button class="btn ghost" id="w-market">Open the market</button>');
    $('#w-market').onclick = () => openTab('market.html');
    return;
  }
  const name = pocket.nameOf;
  const bal = (a) => (pocket.coins.find((c) => c.asset === a)?.amount || 0n);
  const tokens = [...new Set(pools.flatMap((p) => [p.a, p.b]))];
  const partners = (a) => [...new Set(pools.filter((p) => p.a === a || p.b === a).map((p) => (p.a === a ? p.b : p.a)))];
  const poolOf = (a, b) => pools.find((p) => (p.a === a && p.b === b) || (p.a === b && p.b === a));
  let pay = tokens.includes(swapPrefs.pay) ? swapPrefs.pay : (tokens.find((t) => t === config.zecAsset) || tokens[0]);
  let get = poolOf(pay, swapPrefs.get) ? swapPrefs.get : partners(pay)[0];

  const s = screen('Swap', `
    <div class="leg"><div class="leg-top"><span>You pay</span><button type="button" class="bal" id="w-max"></button></div>
      <div class="leg-row"><input id="w-amt" inputmode="decimal" autocomplete="off" placeholder="0"><button type="button" class="chip" id="w-pay"></button></div></div>
    <button type="button" class="flip" id="w-flip" aria-label="Flip">${ICON.swap}</button>
    <div class="leg"><div class="leg-top"><span>You receive</span><span id="w-impact"></span></div>
      <div class="leg-row"><output id="w-out">0</output><button type="button" class="chip" id="w-get"></button></div></div>
    <div class="facts" id="w-facts"></div>
    <p class="hint" id="w-err" role="alert"></p>`,
    '<button class="btn" id="w-go" disabled>Swap</button>');
  const amt = $('#w-amt', s);
  const chip = (el, a) => { el.innerHTML = `<span class="tok" data-sym="${esc(name(a))}">${esc(name(a).slice(0, 1))}</span>${esc(name(a))}<i>▾</i>`; tokenDisc($('.tok', el), name(a)); };
  const readAmount = () => { try { return amt.value.trim() ? parseAmount(amt.value.trim().replace(/,/g, '')) : 0n; } catch { return -1n; } };
  const draw = () => {
    chip($('#w-pay', s), pay); chip($('#w-get', s), get);
    $('#w-max', s).textContent = `Balance ${wadText(bal(pay))}`;
    const pool = poolOf(pay, get);
    const raw = readAmount();
    const go = $('#w-go', s), err = $('#w-err', s);
    err.textContent = ''; err.classList.remove('bad');
    const q = pool && raw > 0n ? quoteSwap(pool, pool.a === pay, raw, swapPrefs.slip) : null;
    $('#w-out', s).textContent = q ? wadText(q.out) : '0';
    $('#w-impact', s).textContent = q && q.impactBps >= 50 ? `${(q.impactBps / 100).toFixed(2)}% impact` : '';
    $('#w-impact', s).classList.toggle('bad', !!q && q.impactBps >= 500);
    const x = BigInt(pool.reserve_a), y = BigInt(pool.reserve_b);
    const rate = pool.a === pay ? (y * WAD) / x : (x * WAD) / y;
    $('#w-facts', s).innerHTML = `<span>Rate</span><b>1 ${esc(name(pay))} ≈ ${esc(wadText(rate))} ${esc(name(get))}</b>
      ${q ? `<span>Minimum</span><b>${esc(wadText(q.min))} ${esc(name(get))}</b>` : ''}
      <span>Slippage</span><b class="slips">${[50, 100, 300].map((b) => `<button type="button" data-slip="${b}" class="${swapPrefs.slip === b ? 'on' : ''}">${b / 100}%</button>`).join('')}</b>
      <span>Pool fee</span><b>${(pool.fee_bps / 100).toFixed(2)}%</b>`;
    s.querySelectorAll('[data-slip]').forEach((b) => { b.onclick = () => { swapPrefs.slip = Number(b.dataset.slip); draw(); }; });
    if (raw < 0n) { err.textContent = 'Enter a number'; err.classList.add('bad'); }
    else if (raw > bal(pay)) { err.textContent = `You have ${wadText(bal(pay))} ${name(pay)}`; err.classList.add('bad'); }
    go.disabled = !(q && q.min > 0n && raw <= bal(pay));
    go.textContent = raw > 0n ? `Swap ${name(pay)} for ${name(get)}` : 'Enter an amount';
  };
  const pick = (which) => {
    const list = which === 'pay' ? tokens : partners(pay);
    sheet(list.map((a) => `<button data-t="${esc(a)}" class="${a === (which === 'pay' ? pay : get) ? 'on' : ''}">${esc(name(a))}<span>${esc(wadText(bal(a)))}</span></button>`).join(''), (el) => {
      el.querySelectorAll('[data-t]').forEach((b) => {
        b.onclick = () => {
          closeSheet();
          if (which === 'pay') { pay = b.dataset.t; if (!poolOf(pay, get) || get === pay) get = partners(pay)[0]; } else get = b.dataset.t;
          swapPrefs = { ...swapPrefs, pay, get };
          draw();
        };
      });
    });
  };
  $('#w-pay', s).onclick = () => pick('pay');
  $('#w-get', s).onclick = () => pick('get');
  $('#w-flip', s).onclick = () => { [pay, get] = [get, pay]; swapPrefs = { ...swapPrefs, pay, get }; amt.value = ''; draw(); };
  $('#w-max', s).onclick = () => { amt.value = wadText(bal(pay), 18).replace(/,/g, ''); draw(); };
  amt.oninput = draw;
  $('#w-go', s).onclick = async () => {
    const go = $('#w-go', s), err = $('#w-err', s);
    const pool = poolOf(pay, get), raw = readAmount();
    go.disabled = true; go.textContent = 'Signing…';
    const g = generation;
    try {
      const q = quoteSwap(pool, pool.a === pay, raw, swapPrefs.slip);
      await placeSwap(config, identity, pocket.held, pool, pool.a === pay, raw, swapPrefs.slip);
      if (g !== generation) return;
      logActivity(config, identity, { kind: 'out', title: `Swapped ${name(pay)} → ${name(get)}`, symbol: name(get), amount: `≈ ${wadText(q.out)} ${name(get)}` });
      s.remove();
      const secs = pocket.epochSeconds || 0;
      toast(secs ? `Swap placed — fills in about ${secs}s` : 'Swap placed — fills at the end of this batch');
      setTimeout(refresh, (secs + 4) * 1000);
      refresh();
    } catch (x) {
      if (g !== generation) return;
      err.textContent = x.message; err.classList.add('bad'); go.disabled = false; go.textContent = 'Try again';
    }
  };
  draw();
  amt.focus();
}

// ---------------------------------------------------------------------------
// Send: pick → fill → review → done.

/// Everything this wallet can send, in the order a person reaches for it.
function assetOptions() {
  const out = [];
  if (identity.zec && zecView) {
    const held = zecView.balance.orchard + zecView.balance.ironwood;
    if (held > 0) out.push({ key: 'zec', kind: 'zcash', symbol: 'ZEC', label: 'Zcash · shielded', have: BigInt(held), shown: zecText(held) });
  }
  for (const c of pocket?.coins || []) {
    if (c.amount > 0n) out.push({ key: c.asset, kind: 'coin', asset: c.asset, symbol: c.symbol, label: `on ${config.network}`, have: c.amount, shown: wadText(c.amount), isZec: c.asset === config.zecAsset });
  }
  for (const i of pocket?.items || []) out.push({ key: `item:${i.asset}`, kind: 'item', asset: i.asset, symbol: i.name, label: 'item', have: WAD, shown: '1' });
  return out;
}

function sendPick() {
  const options = assetOptions();
  const s = screen('Send', options.length
    ? `<p class="hint">What are you sending?</p><div class="pick">${options.map((o) => `<button class="row" data-pick="${esc(o.key)}"><span class="tok" data-sym="${esc(o.symbol)}">${esc(o.symbol.slice(0, 1))}</span><span class="who"><b>${esc(o.symbol)}</b><span>${esc(o.label)}</span></span><span class="val"><b>${esc(o.shown)}</b></span></button>`).join('')}</div>`
    : `<div class="empty"><img src="/nap-mark.png" alt=""><b>Nothing to send yet.</b>${pocket || zecView ? 'Receive ZEC first.' : 'Still reading your balances…'}</div>`);
  s.querySelectorAll('.tok[data-sym]').forEach((el) => tokenDisc(el, el.dataset.sym));
  s.querySelectorAll('[data-pick]').forEach((b) => { b.onclick = () => sendForm(options.find((o) => o.key === b.dataset.pick)); });
}

function sendForm(opt, prefill = {}) {
  if (!opt) { toast('That is not in this wallet any more', true); return; }
  const zcash = opt.kind === 'zcash';
  const item = opt.kind === 'item';
  const s = screen(`Send ${opt.symbol}`, `
    <div class="big-amount"><b>${esc(opt.shown)}</b><span>${esc(opt.symbol)} available</span></div>
    <label class="field"><span>To</span><textarea id="f-to" rows="2" autocomplete="off" autocapitalize="none" spellcheck="false" placeholder="${zcash ? `${esc(config.zcashPrefix)}… Zcash address or zcash: link` : `Zyn account, or ${esc(config.zcashPrefix)}… address`}">${esc(prefill.to || '')}</textarea></label>
    ${item ? '' : `<label class="field"><span>Amount</span><span class="inline"><input id="f-amount" inputmode="decimal" autocomplete="off" placeholder="0.0"><button type="button" id="f-max">MAX</button></span></label>`}
    ${zcash ? '<label class="field"><span>Memo (optional, shielded only)</span><input id="f-memo" maxlength="512" autocomplete="off"></label>' : ''}
    <p class="hint" id="f-err" role="alert">${zcash ? 'A Zcash transaction; the network fee is added on top.' : opt.isZec ? 'To a Zyn account, or out to any Zcash address as a withdrawal.' : item ? 'Items stay on Zyn: send to a Zyn account.' : 'Paid privately on Zyn.'}</p>`,
    '<button class="btn" id="f-next">Review</button>');
  const err = $('#f-err', s);
  $('#f-max', s)?.addEventListener('click', () => {
    if (zcash) { err.textContent = 'The fee depends on the transaction; enter a little under your balance.'; return; }
    $('#f-amount', s).value = wadText(opt.have, 18).replace(/,/g, '');
  });
  $('#f-to', s).addEventListener('input', (e) => {
    const v = e.target.value.trim();
    if (!zcash || !/^zcash:/i.test(v)) return;
    try {
      const r = parsePaymentRequest(v);
      e.target.value = r.address;
      if (r.zatoshi != null) $('#f-amount', s).value = zecText(r.zatoshi).replace(/,/g, '');
      if ($('#f-memo', s)) $('#f-memo', s).value = r.memo;
      err.textContent = r.message ? `Payment request: “${r.message}”` : 'Payment request filled in.';
    } catch (x) { err.textContent = x.message; }
  });
  $('#f-next', s).onclick = async () => {
    const button = $('#f-next', s);
    button.disabled = true; err.classList.remove('bad');
    try {
      const to = $('#f-to', s).value.trim();
      const amountText = item ? '1' : $('#f-amount', s).value.trim();
      if (!to) throw new Error('Enter who it goes to');
      if (zcash) {
        if (/^[0-9a-f]{64}$/i.test(to)) throw new Error('That is a Zyn account. Send ZEC.zy to it instead — or use its deposit address.');
        const transparent = /^t[a-z0-9]{20,}$/i.test(to);
        if (!to.startsWith(config.zcashPrefix) && !transparent) throw new Error(`That is not a ${config.zcash} address`);
        const memo = $('#f-memo', s).value;
        if (transparent && memo) throw new Error('A memo only reaches a shielded address');
        const zat = zatOf(amountText.replace(/,/g, ''));
        if (BigInt(zat) > opt.have) throw new Error('More than this wallet holds');
        review(opt, { to, zat, memo, amountText: zecText(zat), route: 'zcash' });
      } else {
        const amount = item ? WAD : parseAmount(amountText.replace(/,/g, ''));
        if (amount > opt.have) throw new Error('More than this wallet holds');
        const r = await resolveRecipient(config, identity, to, { isZec: opt.isZec, isItem: item });
        review(opt, { to: r.to || r.external, amount, amountText: item ? '1 item' : wadText(amount, 18), route: r.external ? 'withdraw' : 'zyn', viaAddress: !!(r.to && !/^[0-9a-f]{64}$/i.test(to)), typed: to });
      }
    } catch (x) { err.textContent = x.message; err.classList.add('bad'); }
    button.disabled = false;
  };
}

function review(opt, p) {
  const where = p.route === 'zcash' ? `${config.zcash}` : p.route === 'withdraw' ? `${config.network} → ${config.zcash}` : config.network;
  const note = p.route === 'zcash'
    ? 'Proving takes about half a minute. Keep this window open until it is sent — closing it cancels, and nothing leaves.'
    : p.route === 'withdraw'
      ? 'Leaves Zyn as a withdrawal: the bridge fee is added, the Zcash fee comes out of the amount, and the vault pays once two custodians sign.'
      : `Paid privately on Zyn${p.viaAddress ? ' — that Zcash address belongs to a Zyn account' : ''}. Nothing on chain names either of you.`;
  const s = screen('Review', `
    <div class="big-amount"><b>${esc(p.amountText)}</b><span>${esc(opt.kind === 'item' ? opt.symbol : opt.symbol)}</span></div>
    <div class="review">
      <span>To</span><b class="mono">${esc(short(p.to, 12, 10))}</b>
      <span>Network</span><b>${esc(where)}</b>
      ${p.memo ? `<span>Memo</span><b>${esc(p.memo.slice(0, 80))}</b>` : ''}
      <span>From</span><b>Account ${identity.accountIndex + 1}</b>
    </div>
    <p class="hint">${esc(note)} It cannot be undone.</p>
    <p class="hint" id="v-state" role="status"></p>`, '<button class="btn" id="v-go">Confirm and send</button><button class="btn ghost" id="v-cancel">Cancel</button>');
  $('#v-cancel', s).onclick = () => pop.querySelectorAll('.screen').forEach((x) => x.remove());
  $('#v-go', s).onclick = async () => {
    const go = $('#v-go', s), state = $('#v-state', s);
    go.disabled = true; $('#v-cancel', s).disabled = true; $('#s-back', s).disabled = true;
    const g = generation;
    try {
      if (p.route === 'zcash') {
        go.textContent = 'Sending…';
        zecView = await zec.send(p.to, p.zat, p.memo, (st) => {
          if (st.proving) state.textContent = 'Proving the transaction… about half a minute';
          else if (st.to) state.textContent = `Catching up: ${Number(st.at).toLocaleString('en-US')} / ${Number(st.to).toLocaleString('en-US')}`;
        });
        logActivity(config, identity, { kind: 'out', title: 'Sent ZEC', symbol: 'ZEC', amount: `−${p.amountText}`, to: p.to, tx: zecView.txid });
      } else if (p.route === 'withdraw') {
        go.textContent = 'Signing…';
        await withdrawZec(config, identity, pocket.held, p.to, p.amountText.replace(/,/g, ''));
        logActivity(config, identity, { kind: 'out', title: 'Withdrew to Zcash', symbol: 'ZEC', amount: `−${p.amountText}`, to: p.to });
      } else {
        go.textContent = 'Signing…';
        await payOnZyn(config, identity, pocket.held, opt.asset, p.amount, p.to);
        logActivity(config, identity, { kind: 'out', title: `Sent ${opt.symbol}`, symbol: opt.symbol, amount: opt.kind === 'item' ? '1' : `−${p.amountText}`, to: p.typed || p.to });
      }
      if (g !== generation) return;
      pop.querySelectorAll('.screen').forEach((x) => x.remove());
      toast(p.route === 'withdraw' ? 'Withdrawal recorded' : 'Sent');
      renderZec(); refresh();
    } catch (x) {
      if (g !== generation) return;
      state.textContent = x.message; state.classList.add('bad');
      go.disabled = false; go.textContent = 'Try again'; $('#v-cancel', s).disabled = false; $('#s-back', s).disabled = false;
    }
  };
}

// ---------------------------------------------------------------------------

(async () => {
  try {
    config = await loadConfig();
  } catch (e) {
    pop.innerHTML = `<div class="gate"><img class="mark" src="/nap-mark.png" alt=""><h1>Nap is not set up</h1><p class="lead">${esc(e.message)}</p></div>`;
    return;
  }
  const session = await loadSession();
  if (session) begin(session);
  else if (keptWallet()) lockScreen();
  else welcome();
})();
