// Nap's connect window: how a site reaches the browser wallet.
//
// A site opens this page in a popup (`nap-connect.js` does it) and talks to it
// with postMessage. The keys never leave this origin: the site sends a
// request, the holder sees who is asking and what it would do — in a window
// the site cannot draw on or script — and the site gets back an answer.
//
// In the extension the same page is the extension's own window, and sites
// reach it through the extension's content script over a runtime port; the
// browser, not the page, names each port's origin.
//
// The rules this page keeps:
// - On the web it answers only the window that opened it, and only at the
//   origin that window had when it first spoke. A message from anywhere else
//   is dropped. In the extension it answers each site on that site's port.
// - It will not run inside a frame, where a site could dress it up.
// - Connecting shows the site the Zyn account and a Zcash address, nothing
//   else. Every write is approved one at a time, and the approval describes
//   what the chain says, not what the site says.
// - What is remembered is which sites are connected, in localStorage. Never a
//   key, never a balance.

import { loadSession, saveSession } from './session.js';
import { deriveIdentity, forgetIdentity, openSealedWallet } from './core.js';
import { deriveZecKeys, stopZec, ZecAccount } from './zec/client.js';
import { ERR, METHODS, ProviderError, session } from './connect-ops.js';
import { endpoint, IN_EXTENSION } from './origin.js';
import './styles.css';
import './connect.css';

const root = document.querySelector('#app');
const KEPT_KEY = 'nap.kept.v1';
const NETWORK_KEY = 'nap.network';
const CONNECTED_KEY = 'nap.connected.v1';

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const read = (k) => { try { return localStorage.getItem(k); } catch { return null; } };
const write = (k, v) => { try { localStorage.setItem(k, v); } catch { /* this session only */ } };
const keptWallet = () => { try { return JSON.parse(read(KEPT_KEY) || 'null'); } catch { return null; } };
const connections = () => { try { return JSON.parse(read(CONNECTED_KEY) || '{}') || {}; } catch { return {}; } };
const isConnected = (origin, net) => connections()[origin]?.network === net;
function remember(origin, net) { const c = connections(); c[origin] = { network: net, at: Date.now() }; write(CONNECTED_KEY, JSON.stringify(c)); }
function forget(origin) { const c = connections(); delete c[origin]; write(CONNECTED_KEY, JSON.stringify(c)); }

let networks = [];
let config = null;
let identity = null;
let site = null; // web: the opener's origin, fixed by its first message; extension: the origin of the request in hand
const queue = [];
let working = false;
let zecAccount = null;
let zecView = null;

// ---------------------------------------------------------------------------

async function loadNetworks() {
  const response = await fetch('/config.json', { cache: 'no-store' });
  if (!response.ok) throw new Error('Deployment is missing config.json');
  const value = await response.json();
  const list = Array.isArray(value.networks) && value.networks.length ? value.networks : [{ id: 'default', ...value }];
  return list.filter((n) => n.kind === 'object').map((n) => {
    for (const key of ['rpcUrl', 'lightdUrl']) {
      if (!n[key]) continue;
      n[key] = endpoint(n[key], key);
    }
    return n;
  });
}

function pickNetwork(wanted) {
  const byId = networks.find((n) => n.id === wanted);
  if (wanted && !byId) throw new ProviderError(ERR.PARAMS, `Nap has no network called ${wanted}`);
  return byId || networks.find((n) => n.id === read(NETWORK_KEY)) || networks[0];
}

const zecNet = () => (config.zcashPrefix === 'u1' ? 'mainnet' : 'testnet');

function context() {
  const hasZec = !!(identity?.zec && config.lightdUrl);
  return {
    config,
    identity,
    // The address, from the keys alone.
    zec: hasZec ? async () => {
      if (!zecAccount) zecAccount = new ZecAccount({ sk: identity.zec[zecNet()], network: zecNet(), url: config.lightdUrl, birthday: config.zecBirthday });
      if (!zecView) zecView = await zecAccount.peek();
      return zecView;
    } : null,
    // A wallet ready to pay: seeded and synced, which needs the block server.
    zecReady: hasZec ? async () => {
      if (!zecAccount) zecAccount = new ZecAccount({ sk: identity.zec[zecNet()], network: zecNet(), url: config.lightdUrl, birthday: config.zecBirthday });
      zecView = await zecAccount.open();
      return zecAccount;
    } : null,
  };
}

// ---------------------------------------------------------------------------
// Messages.

function reply(job, result, error) {
  const { id } = job;
  const msg = error
    ? { nap: 'response', id, error: { code: error.code || -32603, message: error.message || String(error) } }
    : { nap: 'response', id, result: result === undefined ? null : result };
  if (job.port) { try { job.port.postMessage(msg); } catch { /* the site's tab went away */ } return; }
  if (!site || !window.opener) return;
  window.opener.postMessage(msg, site);
}

/// The extension's door: one port per site tab, opened by its content script.
/// Its origin comes from the browser (`port.sender`), so a page cannot speak
/// for another; only http(s) pages in a tab are answered.
function listenForSites() {
  chrome.runtime.onConnect.addListener((port) => {
    if (port.name !== 'nap-site' || !port.sender?.tab) return;
    let origin = port.sender.origin;
    try { origin = origin || new URL(port.sender.url).origin; } catch { origin = null; }
    if (!origin || !/^https?:\/\//.test(origin)) { port.disconnect(); return; }
    port.onMessage.addListener((d) => {
      if (!d || d.nap !== 'request' || typeof d.method !== 'string') return;
      queue.push({ id: d.id, method: d.method, params: d.params && typeof d.params === 'object' ? d.params : {}, origin, port });
      pump();
    });
    // A closed tab's queued requests are nobody's to answer.
    port.onDisconnect.addListener(() => {
      for (let i = queue.length - 1; i > 0; i--) if (queue[i].port === port) queue.splice(i, 1);
    });
    port.postMessage({ nap: 'ready', version: 1 });
  });
}

window.addEventListener('message', (ev) => {
  if (IN_EXTENSION) return;
  if (!window.opener || ev.source !== window.opener) return;
  const d = ev.data;
  if (!d || d.nap !== 'request' || typeof d.method !== 'string') return;
  if (site === null) site = ev.origin;
  if (ev.origin !== site) return; // the opener navigated elsewhere; not ours to answer
  queue.push({ id: d.id, method: d.method, params: d.params && typeof d.params === 'object' ? d.params : {} });
  pump();
});

async function pump() {
  if (working) return;
  const job = queue[0];
  if (!job) { render(); return; }
  working = true;
  if (job.origin) site = job.origin;
  try {
    const result = await handle(job);
    reply(job, result);
  } catch (e) {
    reply(job, undefined, e);
    if (e.code !== ERR.REJECTED) flash(e.message, true);
  } finally {
    queue.shift();
    working = false;
    pump();
  }
}

/// The holder's answer to what is on screen, as a promise.
let answer = null;
const decide = () => new Promise((resolve) => { answer = resolve; });

async function handle({ method, params }) {
  const m = METHODS[method];
  if (!m) throw new ProviderError(ERR.UNSUPPORTED, `Nap does not do ${method}`);

  if (method === 'nap_disconnect') { forget(site); return true; }
  if (method === 'nap_session') return identity && config && isConnected(site, config.id) ? session(context()) : null;

  if (!config) config = pickNetwork(method === 'nap_connect' ? params.network : undefined);
  else if (method === 'nap_connect' && params.network && params.network !== config.id) {
    throw new ProviderError(ERR.PARAMS, `This window is on ${config.id}; close it to connect on ${params.network}`);
  }
  if (m.kind === 'public') return m.run(context(), params);

  if (!identity) await unlock(method);
  if (!isConnected(site, config.id)) {
    show(connectCard());
    if (!(await decide())) throw new ProviderError(ERR.REJECTED, 'The holder did not connect');
    remember(site, config.id);
  }
  if (method === 'nap_connect') { working_('Connected.'); return session(context()); }
  if (m.kind === 'read') { working_('Answering…'); return m.run(context(), params); }
  if (m.kind === 'auto') { working_('Collecting what settled…'); const r = await m.run(context(), params); flash(r?.collected ? `Collected ${r.collected}.` : 'Nothing to collect yet.'); return r; }

  working_('Checking with the chain…');
  const plan = await m.prepare(context(), params);
  show(approvalCard(plan));
  if (!(await decide())) throw new ProviderError(ERR.REJECTED, 'The holder rejected the request');
  working_(plan.title + '…');
  const result = await plan.run((s) => {
    if (s?.proving) working_('Proving the transaction — about half a minute…');
    else if (s?.to) working_(`Syncing ${Number(s.at).toLocaleString('en-US')} of ${Number(s.to).toLocaleString('en-US')}…`);
  });
  flash(`Done: ${plan.title}.`);
  return result;
}

// ---------------------------------------------------------------------------
// Unlocking.

async function unlock(method) {
  // Unlocked already in this browser session: the site's request is still
  // shown and approved below, but the password is not asked again.
  const held = await loadSession();
  if (held) { identity = held.identity; return; }
  return new Promise((resolve, reject) => {
    const kept = keptWallet();
    const why = method === 'nap_connect' ? 'wants to connect to your Nap' : 'is asking your Nap';
    show(`<p class="kicker">UNLOCK</p><h2>${siteName()} ${why}.</h2>
      ${kept ? `<form id="unlock"><label>Password<input id="pw" type="password" autocomplete="current-password" autofocus></label>
        <button class="primary" type="submit">Unlock <span>→</span></button></form>
        <p class="muted"><a href="" id="use-phrase">Use the recovery phrase instead</a></p>`
      : phraseForm()}
      <button class="quiet" id="deny">Not now</button><p class="error" id="err" role="alert"></p>`);
    const done = (id) => { identity = id; resolve(); };
    root.querySelector('#deny').onclick = () => reject(new ProviderError(ERR.REJECTED, 'The holder did not unlock'));
    root.querySelector('#use-phrase')?.addEventListener('click', (e) => { e.preventDefault(); root.querySelector('#unlock').outerHTML = phraseForm(); bindPhrase(done); });
    const form = root.querySelector('#unlock');
    if (kept) {
      form.onsubmit = async (e) => {
        e.preventDefault();
        try { const opened = await openSealedWallet(kept, root.querySelector('#pw').value); await saveSession(opened.identity, opened.receiving); done(opened.identity); }
        catch (err) { root.querySelector('#err').textContent = err.message; }
      };
    } else bindPhrase(done);
  });
}

function phraseForm() {
  return `<form id="unlock"><label>Recovery phrase<textarea id="words" rows="3" autocomplete="off" autocapitalize="none" spellcheck="false"></textarea></label>
    <div class="split"><label>Passphrase<input id="pass" type="password" autocomplete="off" placeholder="if used"></label><label>Account<input id="idx" type="number" min="0" value="0"></label></div>
    <button class="primary" type="submit">Unlock <span>→</span></button></form>`;
}

function bindPhrase(done) {
  root.querySelector('#unlock').onsubmit = async (e) => {
    e.preventDefault();
    try {
      const words = root.querySelector('#words').value;
      const pass = root.querySelector('#pass').value;
      const idx = Number(root.querySelector('#idx').value);
      const id = deriveIdentity(words, pass, idx);
      try { id.zec = await deriveZecKeys(words, pass, idx); } catch { /* Zyn still works */ }
      root.querySelector('#words').value = '';
      done(id);
    } catch (err) { root.querySelector('#err').textContent = err.message; }
  };
}

// ---------------------------------------------------------------------------
// Screens.

const siteName = () => `<b class="site">${esc(site ? new URL(site).host : 'A site')}</b>`;

function connectCard() {
  return `<p class="kicker">CONNECT</p><h2>${siteName()} wants to connect.</h2>
    <dl class="rows"><dt>It will see</dt><dd>Your Zyn account ID${identity.zec ? ' and your Zcash address' : ''}</dd>
    <dt>When it asks</dt><dd>Your ZEC.zy and token balances, your items, what you are owed, your sale tickets and the coins you made</dd>
    <dt>It will not see</dt><dd>Your keys or your recovery phrase</dd>
    <dt>Network</dt><dd>${esc(config.network)} · ${esc(config.zcash)}</dd></dl>
    <p class="muted">It can ask you to approve payments and trades. Nothing moves unless you approve it here.</p>
    <div class="actions"><button class="quiet" id="no">Reject</button><button class="primary" id="yes">Connect <span>→</span></button></div>`;
}

function approvalCard(plan) {
  return `<p class="kicker">${esc(new URL(site).host)} ASKS</p><h2>${esc(plan.title)}.</h2>
    ${plan.image ? `<img class="cn-picture" src="${esc(plan.image)}" alt="The token's picture">` : ''}
    <dl class="rows">${plan.rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join('')}</dl>
    <p class="muted">${esc(plan.note || '')}</p>
    <div class="actions"><button class="quiet" id="no">Reject</button><button class="primary${plan.danger ? ' danger' : ''}" id="yes">Approve <span>→</span></button></div>`;
}

function show(inner) {
  root.innerHTML = frame(inner);
  const yes = root.querySelector('#yes');
  const no = root.querySelector('#no');
  if (yes) yes.onclick = () => { const a = answer; answer = null; a?.(true); };
  if (no) no.onclick = () => { const a = answer; answer = null; a?.(false); };
  root.querySelector('#disconnect')?.addEventListener('click', () => { forget(site); render(); });
}

function frame(inner) {
  return `<div class="grain"></div><header class="cn-head"><span class="brand"><img src="/nap-mark.png" alt=""><span>Nap</span></span>${config ? `<span class="network"><i></i>${esc(config.network)}</span>` : ''}</header>
    <main class="cn">${inner}</main><p class="cn-flash" id="flash" role="status"></p>`;
}

function working_(text) { show(`<p class="kicker">WORKING</p><h2>${esc(text)}</h2><div class="cn-spin" aria-hidden="true"></div>`); }

function flash(text, bad = false) {
  const el = root.querySelector('#flash');
  if (!el) return;
  el.textContent = text;
  el.classList.toggle('bad', bad);
}

function render() {
  if (working || answer) return;
  if (!site) { show(`<p class="kicker">NAP CONNECT</p><h2>Waiting for the site…</h2><p class="muted">${IN_EXTENSION ? 'This window answers sites that ask for Nap.' : 'This window answers the page that opened it.'}</p>`); return; }
  const connected = config && isConnected(site, config.id);
  show(`<p class="kicker">NAP CONNECT</p><h2>${connected ? `Connected to ${siteName()}.` : `${siteName()} is not connected.`}</h2>
    <p class="muted">${connected ? 'Keep this window open while you use the site; it asks you here before anything moves. Closing it locks Nap for that site.' : 'Nothing was shared.'}</p>
    ${connected ? '<button class="quiet" id="disconnect">Disconnect this site</button>' : ''}`);
}

// ---------------------------------------------------------------------------

window.addEventListener('pagehide', () => { forgetIdentity(identity); identity = null; stopZec(); });

(async () => {
  // Inside a frame a site could cover this page with its own and borrow the
  // clicks; without an opener there is nobody to answer. Either way, stop.
  if (window.top !== window.self) { document.body.textContent = 'Nap does not run inside another page.'; return; }
  if (!IN_EXTENSION && !window.opener) {
    root.innerHTML = frame('<p class="kicker">NAP CONNECT</p><h2>Nothing is asking.</h2><p class="muted">This window opens when a site asks to connect to Nap. To use your wallet directly, <a href="/">open Nap</a>.</p>');
    return;
  }
  try {
    networks = await loadNetworks();
    render();
    // Only now can the page answer; saying so earlier would lose requests.
    if (IN_EXTENSION) listenForSites();
    else window.opener.postMessage({ nap: 'ready', version: 1 }, '*');
  } catch (e) {
    root.innerHTML = frame(`<h2>Nap cannot open.</h2><p class="muted">${esc(e.message)}</p>`);
  }
})();
