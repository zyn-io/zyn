import QRCode from 'qrcode';
import { x25519 } from '@noble/curves/ed25519.js';
import { PROGRAM as OBJECT_PROGRAM, scan as scanObjects, objectCall, registerIntent, sealNote, signedFrame as objectFrame, spendIntent, submitSigned, withdrawIntent } from './object.js';
import { fund as fundFrom } from './market.js';
import { checkedEpoch, cancelOfferIntent, deriveCollectionAccount, XZEC, deriveIdentity, forgetIdentity, hex, newMnemonic, openSealedWallet, parseAmount, placeOfferIntent, readableAmount, rpc, sealWallet, signedFrame, signedRead, transferFrame, unhex, WAD } from './core.js';
import { boundedBytes, MAX_IMAGE, MAX_MANIFEST, parseManifest, safeRaster } from './media.js';
import './styles.css';

const root = document.querySelector('#app');
let config;
let identity = null;
/// Where bought items land — one per collection the deployment declares.
let receiving = [];
/// Our own resting offers — items escrowed in the order book, not in an account.
let listings = [];
let assets = [];
let holdings = [];
/// The object ledger's notes this wallet holds, with commitments — what a
/// withdrawal spends.
let objectHeld = [];
let generatedWords = '';
const giftUrl = new URL(location.href);
const claimCode = new URLSearchParams(giftUrl.hash.slice(1)).get('claim') || giftUrl.searchParams.get('claim') || '';
// Fragment links keep the card secret out of HTTP access logs. Scrub legacy
// query links immediately; deployments must also redact them from access logs.
giftUrl.searchParams.delete('claim'); giftUrl.hash = '';
history.replaceState(null, '', giftUrl.pathname + giftUrl.search);
let generation = 0;
let refreshId = 0;
const artUrls = new Set();
function clearSession() {
  generation++; refreshId++;
  forgetIdentity(identity); identity = null;
  for (const r of receiving) forgetIdentity(r);
  receiving = []; holdings = []; assets = []; listings = []; generatedWords = '';
  for (const url of artUrls) URL.revokeObjectURL(url);
  artUrls.clear();
}

const escapeHtml = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const short = (s) => `${s.slice(0, 8)}…${s.slice(-6)}`;
// One network: a label, as before. More than one: a control, because a wallet
// that can reach two chains must let the holder see which one they are on and
// say so out loud — the chain id stays visible either way.
// The name only. The chain id still appears where it decides something — on
// the send review, beside the destination — but in the corner it was noise on
// every screen rather than a check anyone was making.
// Both chains, because the wallet spans both: ZEC goes in at a Zcash address
// and arrives as ZEC.zy on Zyn. The chain id is not here — it appears on the
// send review, where it decides something.
const networkNames = (n) => `${n.zcash} · ${n.network}`;
const networkPill = () => (config.networks && config.networks.length > 1)
  ? `<label class="network network-pick"><i></i><span class="visually-hidden">Network</span><select id="network-pick">${config.networks.map((n) => `<option value="${escapeHtml(n.id)}"${n.id === config.id ? ' selected' : ''}>${escapeHtml(networkNames(n))}</option>`).join('')}</select></label>`
  : `<span class="network"><i></i>${escapeHtml(networkNames(config))}</span>`;

/// Validate one network's shape. Every URL must be a same-origin path: the
/// shipped CSP permits nothing else, and a cross-origin endpoint here would be
/// a wallet quietly talking to a host the policy never admitted.
function checkedNetwork(value) {
  if (!Number.isInteger(value.chain) || value.chain < 0 || value.chain > 0xffffffff || typeof value.network !== 'string') throw new Error('Deployment configuration is incomplete');
  // A Zyn chain sits on exactly one Zcash network and the node refuses to mix
  // them. Naming it lets the header say both — and `zcashPrefix` is what keeps
  // that name honest, because a label nothing checks is a label that can lie.
  if (typeof value.zcash !== 'string' || !/^u(test)?1$/.test(String(value.zcashPrefix || ''))) {
    throw new Error('Each network must name its Zcash network and address prefix');
  }
  const out = { ...value, collections: Array.isArray(value.collections) ? value.collections : [] };
  for (const key of ['rpcUrl', 'claimUrl', 'mediaGateway']) {
    const url = new URL(value[key], location.origin);
    if (!value[key] || url.origin !== location.origin || url.username || url.password || url.search || url.hash) throw new Error(`${key} must be a same-origin path`);
    out[key] = url.pathname;
  }
  return out;
}

/// Which network the holder last chose. Not a secret — it is the name of a
/// public chain — so `localStorage` is the right place, unlike anything
/// derived from the phrase.
const NETWORK_KEY = 'nap.network';

/// A wallet kept on this device: the derived key, sealed under a password the
/// holder chose. Never the phrase — see `sealIdentity`.
const KEPT_KEY = 'nap.kept.v1';
const keptWallet = () => { try { return JSON.parse(localStorage.getItem(KEPT_KEY) || 'null'); } catch { return null; } };
const keepWallet = (blob) => { try { localStorage.setItem(KEPT_KEY, JSON.stringify(blob)); return true; } catch { return false; } };
const forgetKept = () => { try { localStorage.removeItem(KEPT_KEY); } catch { /* nothing to forget */ } };
const rememberedNetwork = () => { try { return localStorage.getItem(NETWORK_KEY); } catch { return null; } };
const rememberNetwork = (id) => { try { localStorage.setItem(NETWORK_KEY, id); } catch { /* private window: this session only */ } };

async function loadConfig() {
  const response = await fetch('/config.json', { cache: 'no-store' });
  if (!response.ok) throw new Error('Deployment is missing config.json');
  const value = await response.json();

  // The top level is the default network, so a deployment that never heard of
  // `networks` still validates exactly as it did before.
  const fallback = { id: 'default', ...checkedNetwork(value) };
  const networks = Array.isArray(value.networks) && value.networks.length
    ? value.networks.map((n) => ({ id: String(n.id || n.chain), ...checkedNetwork(n) }))
    : [fallback];

  const wanted = rememberedNetwork();
  const chosen = networks.find((n) => n.id === wanted)
    // An unknown or removed id falls back to the deployment's default rather
    // than failing: a stale preference must not lock someone out of a wallet.
    || networks.find((n) => n.chain === fallback.chain)
    || networks[0];
  return { ...chosen, networks };
}

/// Move to another chain without forgetting the phrase.
///
/// The seed is chain-independent — the same words derive the same account on
/// either — so there is no reason to make switching cost a restore. What must
/// go is every cached answer from the old chain, which `reset` already does;
/// `checkedEpoch` then fails closed if an endpoint ever answers for the wrong
/// one.
function switchNetwork(id) {
  const next = config.networks.find((n) => n.id === id);
  if (!next || next.id === config.id) return;
  rememberNetwork(next.id);
  config = { ...next, networks: config.networks };
  // Deliberately NOT clearSession(): that forgets the phrase, and the phrase is
  // not what changed. Only the chain's answers are stale.
  holdings = []; assets = [];
  generation++; refreshId++;
  if (identity) { wallet(); refresh(); } else onboarding();
}

// Asking a gateway for the picture of an item you own tells that host your IP,
// which item you hold, and when — on a shielded chain, the one thing the chain
// itself will not reveal. The desktop wallet has always kept this behind an
// explicit toggle; this one fetched art for every holding on every render, so a
// single page load handed over the complete list. Off until the holder says
// otherwise, and the reason is on the button rather than in a help page.
const ART_KEY = 'nap_web_art';
function artEnabled() {
  try {
    return localStorage.getItem(ART_KEY) === '1';
  } catch {
    // A browser that refuses storage is a browser that cannot remember consent,
    // and unremembered consent is not consent. Stay off.
    return false;
  }
}
function setArtEnabled(on) {
  try {
    localStorage.setItem(ART_KEY, on ? '1' : '0');
  } catch {
    /* refused storage: the setting lasts for this render only */
  }
}

function shell(content) {
  root.innerHTML = `<div class="grain"></div><header><a class="brand" href="/" aria-label="Nap Wallet"><img src="/nap-mark.png" alt=""><span>Nap</span></a>${networkPill()}</header>${content}<footer><span>Keys stay in this tab.</span><span>Nap Wallet · Zyn</span></footer>`;
  const pick = root.querySelector('#network-pick');
  if (pick) pick.addEventListener('change', (e) => switchNetwork(e.target.value));
}

function unlockForm() {
  return `<form id="kept-form"><p class="kicker">THIS DEVICE</p><h2>Open your wallet.</h2><p class="muted">Your key is held here, sealed. It opens with the password you chose — not your recovery phrase.</p><label>Password<input id="kept-password" type="password" autocomplete="current-password" autofocus></label><button class="primary" type="submit">Open <span>→</span></button><p class="error" id="form-error" role="alert"></p><p class="muted"><a href="" id="use-phrase">Use a recovery phrase instead</a> · <a href="" id="forget-kept">Forget this device</a></p></form>`;
}

function bindKept() {
  root.querySelector('#kept-form').onsubmit = async (event) => {
    event.preventDefault();
    const button = root.querySelector('.primary');
    button.disabled = true;
    try { await openKept(root.querySelector('#kept-password').value); }
    catch (e) { root.querySelector('#form-error').textContent = e.message; button.disabled = false; }
  };
  root.querySelector('#use-phrase').onclick = (e) => { e.preventDefault(); onboarding('restore'); };
  root.querySelector('#forget-kept').onclick = (e) => {
    e.preventDefault();
    forgetKept();
    onboarding('restore');
  };
}

function onboarding(mode = keptWallet() ? 'kept' : (claimCode ? 'create' : 'restore')) {
  if (mode !== 'create') generatedWords = '';
  shell(`<main class="onboard">
    <section class="intro">
      <p class="eyebrow">BROWSER WALLET / NO INSTALL</p>
      <h1>Your pocket,<br><em>between blocks.</em></h1>
      <p class="lede">Receive the Nap Mole in person, then carry the same account into desktop Nap with one recovery phrase.</p>
      <div class="promise"><span>01</span><p><b>No account server.</b><br>Your key is derived and used on this device.</p></div>
      <div class="promise"><span>02</span><p><b>Nothing kept unless you say so.</b><br>Closing this tab forgets the key. Keep it here and only the sealed key is stored — never your phrase.</p></div>
    </section>
    <section class="entry panel">
      ${mode === 'kept' ? '' : `<div class="tabs"><button data-mode="restore" class="${mode === 'restore' ? 'active' : ''}">Restore</button><button data-mode="create" class="${mode === 'create' ? 'active' : ''}">Create new</button></div>`}
      ${mode === 'kept' ? unlockForm() : (mode === 'restore' ? restoreForm() : createForm())}
      <div class="scope"><b>Phase 1</b><span>Zyn NFTs and account transfers. Shielded Zcash stays in desktop Nap.</span></div>
    </section>
  </main>`);
  root.querySelectorAll('[data-mode]').forEach((b) => b.onclick = () => onboarding(b.dataset.mode));
  if (mode === 'kept') bindKept(); else if (mode === 'create') bindCreate(); else bindRestore();
}

function restoreForm() {
  return `<form id="restore-form"><p class="kicker">WELCOME BACK</p><h2>Wake your wallet.</h2><label>24-word recovery phrase<textarea id="words" rows="5" autocomplete="off" autocapitalize="none" spellcheck="false" placeholder="word 1  word 2  word 3 …"></textarea></label><div class="split"><label>Optional passphrase<input id="passphrase" type="password" autocomplete="off" placeholder="Leave blank if unused"></label><label>Account<input id="account-index" type="number" min="0" max="2147483647" value="0"></label></div><label>Keep on this device <span class="muted">(optional password, 8+ characters)</span><input id="keep-password" type="password" autocomplete="new-password" placeholder="Leave blank to forget on close"></label><button class="primary" type="submit">Restore on this device <span>→</span></button><p class="error" id="form-error" role="alert"></p></form>`;
}
function createForm() {
  if (!generatedWords) generatedWords = newMnemonic();
  const numbered = generatedWords.split(' ').map((w, i) => `<span><i>${String(i + 1).padStart(2, '0')}</i>${w}</span>`).join('');
  return `<form id="create-form"><p class="kicker">NEW POCKET</p><h2>Write this down first.</h2><p class="muted">These words restore both your Nap Zcash wallet and your Zyn items. They are shown once.</p><div class="phrase">${numbered}</div><label class="check"><input id="ack" type="checkbox"><span>I saved all 24 words offline. I understand a screenshot or cloud note can expose my wallet.</span></label><label>Keep on this device <span class="muted">(optional password, 8+ characters)</span><input id="keep-password" type="password" autocomplete="new-password" placeholder="Leave blank to forget on close"></label><button class="primary" type="submit" disabled>Open my wallet <span>→</span></button><p class="error" id="form-error" role="alert"></p></form>`;
}
function bindRestore() {
  root.querySelector('#restore-form').onsubmit = async (event) => {
    event.preventDefault();
    try {
      await unlock(
        root.querySelector('#words').value,
        root.querySelector('#passphrase').value,
        Number(root.querySelector('#account-index').value),
        root.querySelector('#keep-password')?.value || '',
      );
    } catch (e) { root.querySelector('#form-error').textContent = e.message; }
  };
}
function bindCreate() {
  const ack = root.querySelector('#ack');
  const button = root.querySelector('.primary');
  ack.onchange = () => { button.disabled = !ack.checked; };
  root.querySelector('#create-form').onsubmit = async (event) => {
    event.preventDefault();
    if (!ack.checked) return;
    try {
      await unlock(generatedWords, '', 0, root.querySelector('#keep-password')?.value || '');
      generatedWords = '';
    } catch (e) { root.querySelector('#form-error').textContent = e.message; }
  };
}
/// The collections this deployment knows about, from `config.json`.
const knownCollections = () => (config.collections || [])
  .filter((c) => /^[0-9a-f]{64}$/i.test(String(c.id)))
  .map((c) => ({ name: String(c.name || 'Collection'), id: unhex(c.id, 32) }));

async function unlock(words, passphrase, index, keepPassword = '') {
  const nextIdentity = deriveIdentity(words, passphrase, index);
  // Derived here because this is the only moment the phrase exists. A kept
  // wallet stores these too — without them, a reopened wallet cannot see what
  // its owner bought.
  const nextReceiving = knownCollections().map((c) => ({
    ...deriveCollectionAccount(words, passphrase, index, c.id),
    name: c.name,
  }));
  // Sealed before the phrase is wiped from the form, and only if asked. A
  // failure here must not cost the unlock — the wallet still works, it just
  // will not be here next time, and the holder is told which.
  let kept = null;
  if (keepPassword) {
    try {
      kept = await sealWallet(nextIdentity, nextReceiving, keepPassword, index);
      if (!keepWallet(kept)) kept = null;
    } catch (e) { kept = null; throw e; }
  }
  clearSession(); identity = nextIdentity; receiving = nextReceiving;
  root.querySelectorAll('textarea,input').forEach((el) => { if (el.type !== 'checkbox') el.value = ''; });
  wallet(kept ? 'Kept on this device. Your password opens it next time; your phrase still restores it anywhere.' : '');
  refresh();
}

/// Open a wallet this device already holds.
async function openKept(password) {
  const opened = await openSealedWallet(keptWallet(), password);
  clearSession();
  identity = opened.identity;
  // Names are not sealed — they are deployment labels, not secrets — so they
  // are matched back from the current config.
  const named = knownCollections();
  receiving = opened.receiving.map((r) => ({
    ...r,
    name: (named.find((c) => hex(c.id) === hex(r.collection)) || {}).name || 'Collection',
  }));
  root.querySelectorAll('input').forEach((el) => { if (el.type !== 'checkbox') el.value = ''; });
  wallet();
  refresh();
}

function wallet(message = '') {
  const account = identity.accountHex;
  shell(`<main class="wallet">
    <aside class="rail">
      <div><p class="kicker">YOUR ZYN ACCOUNT</p><button class="account-copy" id="copy-account"><b>${short(account)}</b><span>Copy full ID</span></button></div>
      <nav><button class="active">Pocket</button><button id="receive-link">Receive</button><button id="market-link">Market</button><button id="recovery-link">Recovery</button></nav>
      <button class="quiet" id="forget">${keptWallet() ? 'Lock' : 'Lock &amp; forget key'}</button>
    </aside>
    <section class="content">
      <div class="topline"><div><p class="eyebrow">POCKET / ITEMS</p><h1>Small things,<br>held properly.</h1></div><button class="refresh" id="refresh">↻ Refresh</button></div>
      <div class="notice ${message ? 'show' : ''}" id="notice">${escapeHtml(message)}</div>
      ${claimCode ? `<section class="claim"><div><p class="kicker">GIFT CARD DETECTED</p><h2>Your mole is waiting.</h2><p>The coordinator sees this public account ID—not your phrase or key.</p></div><button id="claim">Claim gift <span>→</span></button></section>` : ''}
      <section><div class="section-title"><h2>Coins</h2><span id="coin-count">—</span></div><div class="coins" id="coins"><div class="skeleton"></div></div></section>
      <section><div class="section-title"><h2>Items</h2><span id="item-count">—</span></div><label class="art-consent"><input type="checkbox" id="art-toggle"><span>Show artwork — fetches each item's picture from the gateway, which tells that host your IP, which items you hold, and when. Off by default.</span></label><div class="items" id="items"><div class="skeleton"></div><div class="skeleton"></div></div></section>
      <section class="activity"><div class="section-title"><h2>Account</h2></div><div class="account-card"><div><span>Public account</span><code>${account}</code></div><canvas id="qr" width="164" height="164"></canvas></div></section>
      <section class="activity"><div class="section-title"><h2>Bring ZEC in</h2></div><div class="account-card" id="deposit-card"><div><span>Your Zcash deposit address</span><code id="deposit-address">Asking the node…</code><p class="muted" id="deposit-note">Send shielded ZEC here and it arrives as ZEC.zy. No memo needed — this address is yours alone.</p><button class="quiet" id="copy-deposit" hidden>Copy address</button></div><canvas id="deposit-qr" width="164" height="164"></canvas></div></section>
      ${config.kind === 'object' ? `<section class="activity"><div class="section-title"><h2>Take ZEC out</h2></div><form class="account-card" id="withdraw-form"><div><span>Send ZEC.zy back to a Zcash address</span><label class="visually-hidden" for="withdraw-to">Zcash address</label><input id="withdraw-to" class="withdraw-input" autocomplete="off" spellcheck="false" placeholder="${escapeHtml(config.zcashPrefix)}… or t…"><label class="visually-hidden" for="withdraw-amount">Amount</label><input id="withdraw-amount" class="withdraw-input" inputmode="decimal" autocomplete="off" placeholder="0.0 ZEC"><p class="muted">The chain records only a commitment to the address. The vault pays it once two custodians sign; the Zcash network fee comes out of the amount.</p></div><div><button class="primary" type="submit" id="withdraw-go">Withdraw <span>→</span></button></div></form></section>` : ''}
    </section>
  </main><dialog id="send-dialog"></dialog><dialog id="recovery-dialog"><button class="x">×</button><p class="kicker">RECOVERY</p><h2>The phrase is the wallet.</h2><p>Nap does not store it in this browser. Restore this same phrase, passphrase and account index in browser or desktop Nap to recover <code>${account}</code>.</p><div class="descriptor"><span>Derivation</span><b>nap.zyn.ed25519.v1</b><span>Account index</span><b>${identity.accountIndex}</b><span>Network scoped</span><b>No</b></div></dialog>`);
  QRCode.toCanvas(root.querySelector('#qr'), account, { width: 164, margin: 1, color: { dark: '#1f2428', light: '#f8f2e4' } });
  void loadDepositAddress(identity);
  root.querySelector('#withdraw-form')?.addEventListener('submit', async (e) => {
    e.preventDefault();
    const button = root.querySelector('#withdraw-go');
    button.disabled = true;
    try {
      const to = root.querySelector('#withdraw-to').value.trim();
      if (!to.startsWith(config.zcashPrefix) && !/^t[a-z0-9]/i.test(to)) throw new Error(`That is not a ${config.zcash} address`);
      await withdrawToZcash(to, root.querySelector('#withdraw-amount').value.trim());
      note('Withdrawal recorded. The vault pays it once two custodians sign — usually within minutes.');
      root.querySelector('#withdraw-amount').value = '';
      refresh(false);
    } catch (err) { note(err.message, true); } finally { button.disabled = false; }
  });
  root.querySelector('#copy-account').onclick = async () => { await navigator.clipboard.writeText(account); note('Account ID copied.'); };
  root.querySelector('#receive-link').onclick = () => root.querySelector('.account-card').scrollIntoView({ behavior: 'smooth', block: 'center' });
  // Same origin, same kept key: the market opens it with the same password.
  root.querySelector('#market-link').onclick = () => { location.href = '/market.html'; };
  root.querySelector('#recovery-link').onclick = () => root.querySelector('#recovery-dialog').showModal();
  root.querySelector('#recovery-dialog .x').onclick = () => root.querySelector('#recovery-dialog').close();
  // Locking returns to whatever this device can actually offer: a password if
  // the wallet is kept here, the phrase if it is not.
  root.querySelector('#forget').onclick = () => { clearSession(); onboarding(keptWallet() ? 'kept' : 'restore'); };
  root.querySelector('#refresh').onclick = refresh;
  root.querySelector('#claim')?.addEventListener('click', claimGift);
}
function note(text, bad = false) {
  const el = root.querySelector('#notice');
  if (!el) return;
  el.textContent = text; el.classList.add('show'); el.classList.toggle('bad', bad);
}

async function refresh(announce = true) {
  if (!identity) return;
  const owner = identity, current = generation, request = ++refreshId;
  const active = () => generation === current && request === refreshId;
  const button = root.querySelector('#refresh');
  if (button) button.disabled = true;
  try {
    const status = await rpc(config.rpcUrl, 'zyn_status');
    if (!active()) return;
    const epoch = checkedEpoch(status, config.chain);

    // An object ledger answers none of the questions below. It publishes
    // commitments and nullifiers and says nothing about who owns what, so the
    // balance is computed here from the holder's own keys rather than asked
    // for. Everything after this point is the AMM chains' shape.
    if (config.kind === 'object') {
      await refreshObject(epoch, announce, active);
      return;
    }
    // The identity *and* every receiving account. An item bought through a
    // store lands in the latter (§97.5), so reading only the identity shows an
    // empty pocket to somebody who has just paid.
    const owners = [{ key: owner, name: null }, ...receiving.map((r) => ({ key: r, name: r.name }))];
    const [allAssets, allOffers, ...records] = await Promise.all([
      rpc(config.rpcUrl, 'zyn_assets'),
      // A listed item is escrowed, so it is gone from `spendable` — without
      // this the wallet would show it as vanished the moment it was listed.
      rpc(config.rpcUrl, 'zyn_offers').catch(() => []),
      ...owners.map((o) => rpc(config.rpcUrl, 'zyn_account', [hex(signedRead(o.key, config.chain, epoch))])),
    ]);
    if (!active()) return;
    if (!Array.isArray(allAssets)) throw new Error('Malformed holdings response');
    const validAsset = (id) => typeof id === 'string' && /^[a-f0-9]{64}$/.test(id);
    if (!allAssets.every((a) => validAsset(a.id))) throw new Error('Invalid asset identifier');

    const merged = [];
    for (const [i, record] of records.entries()) {
      if (record !== null && !Array.isArray(record?.spendable)) throw new Error('Malformed holdings response');
      for (const h of record?.spendable || []) {
        if (!validAsset(h.asset)) throw new Error('Invalid asset identifier');
        // Tagged with the account it was found in, so a send signs with the
        // key that actually holds the item rather than the identity's.
        merged.push({ ...h, owner: owners[i].key, via: owners[i].name });
      }
    }
    const mine = new Map(owners.map((o) => [hex(o.key.account), o]));
    listings = (Array.isArray(allOffers) ? allOffers : [])
      .filter((o) => mine.has(String(o.maker)))
      .map((o) => ({ ...o, owner: mine.get(String(o.maker)).key, via: mine.get(String(o.maker)).name }));
    assets = allAssets;
    holdings = merged;
    renderCoins();
    renderItems();
    if (announce) note(`Synced at epoch ${status.epoch}.`);
  } catch (e) {
    if (!active()) return;
    holdings = []; assets = []; listings = [];
    const coinCount = root.querySelector('#coin-count');
    if (coinCount) coinCount.textContent = 'Unavailable';
    const coinList = root.querySelector('#coins');
    if (coinList) coinList.innerHTML = '';
    root.querySelector('#item-count').textContent = 'Unavailable';
    root.querySelector('#items').innerHTML = '<div class="empty"><h3>Holdings unavailable.</h3><p>Reconnect and refresh before sending.</p></div>';
    note(e.message === 'Failed to fetch' ? 'RPC unavailable. Your key is safe; try again when the connection returns.' : e.message, true);
  } finally { if (button) button.disabled = false; }
}

// The deposit address is not the account. The account receives Zyn items from
// other Zyn users; this receives *Zcash*, and sending ZEC to the account id
// would be sending it to a string that is not an address at all. They sit in
// separate cards, labelled differently, for that reason.
//
// Asking requires a signature, which the browser wallet can produce and a
// stranger cannot: issuing an address writes it into the node's scan register,
// so the public method is gated rather than open (DECISIONS 87.6).
/// Sign and send an intent through the object ledger's holders' door. A
/// refusal inside the receipts is an error here too.
async function submitObject(intent) {
  const status = await rpc(config.rpcUrl, 'zyn_status');
  const epoch = checkedEpoch(status, config.chain);
  const frame = await objectFrame(identity.secretKey, config.chain, epoch + 100n, intent);
  const result = await submitSigned(config.rpcUrl, frame);
  const refused = /Refused\("([^"]*)"\)/.exec(String(result?.receipts || ''));
  if (refused) throw new Error(`The chain refused: ${refused[1]}`);
  return result;
}

/// Publish where this account is paid: its viewing key. Public by design —
/// a payer who cannot read it cannot pay — and it cannot spend anything.
async function registerObjectAccount(owner) {
  await submitObject(registerIntent(owner.account, x25519.getPublicKey(owner.viewing)));
  note('Registered this account to receive ZEC. Deposits to the address below are credited to it.');
}

const EXITS_KEY = () => `nap.exits.${config.chain}.${identity.accountHex}`;
const pendingExits = () => { try { return JSON.parse(localStorage.getItem(EXITS_KEY()) || '[]'); } catch { return []; } };
const savePendingExits = (list) => { try { localStorage.setItem(EXITS_KEY(), JSON.stringify(list)); } catch { /* this session only */ } };

/// Tell the vault where a withdrawal goes, now or on a later visit. The chain
/// holds only a commitment; until the address and salt are revealed the exit
/// is owed and cannot be paid. Kept on this device until the node files it.
async function revealExits() {
  const left = [];
  for (const x of pendingExits()) {
    try {
      await objectCall(config.rpcUrl, 'zyn_exitReveal', { address: x.address, salt: x.salt });
    } catch (e) {
      if (!/no unpaid exit/.test(e.message)) left.push(x);
    }
  }
  savePendingExits(left);
}

/// ZEC.zy back to Zcash: commit to the address, sign the withdrawal, reveal.
async function withdrawToZcash(address, amountText) {
  const zec = config.zecAsset;
  const raw = parseAmount(amountText);
  // The vault pays whole zatoshi: at most eight decimals.
  if (raw % 10_000_000_000n !== 0n) throw new Error('ZEC has eight decimal places');
  if (raw < 100_000n * 10_000_000_000n) throw new Error('Withdraw at least 0.001 ZEC — the Zcash fee comes out of it');
  const salt = hex(crypto.getRandomValues(new Uint8Array(32)));
  const { destination } = await objectCall(config.rpcUrl, 'zyn_exitCommitment', { address, salt });
  if (!/^[0-9a-f]{64}$/.test(String(destination))) throw new Error('The node did not return a commitment');
  const { inputs, change } = fundFrom(identity, objectHeld, unhex(zec, 32), raw);
  // Kept before submitting: if the page closes between the two steps, the
  // next visit still knows where this exit was meant to go.
  savePendingExits([...pendingExits(), { address, salt }]);
  await submitObject(withdrawIntent({ holder: identity.account, inputs, asset: unhex(zec, 32), amount: raw, destination: unhex(destination, 32), outputs: change }));
  await revealExits();
}

async function loadDepositAddress(owner) {
  const current = generation;
  const field = root.querySelector('#deposit-address');
  const note_ = root.querySelector('#deposit-note');
  const copy = root.querySelector('#copy-deposit');
  if (!field) return;
  try {
    let result;
    if (config.kind === 'object') {
      result = await objectCall(config.rpcUrl, 'zyn_depositAddress', { account: owner.accountHex });
      if (generation !== current) return;
      // An address only credits an account that has published where to be
      // paid. The node now holds such a payment as a debt rather than losing
      // it, but registering first means it is credited the moment it lands.
      if (result && result.registered === false && owner.viewing) {
        await registerObjectAccount(owner);
        if (generation !== current) return;
      }
    } else {
      const status = await rpc(config.rpcUrl, 'zyn_status');
      if (generation !== current) return;
      const payload = signedRead(owner, config.chain, checkedEpoch(status, config.chain));
      result = await rpc(config.rpcUrl, 'zyn_depositAddress', [hex(payload)]);
    }
    if (generation !== current) return;
    const address = result?.address;
    if (typeof address !== 'string' || !address) throw new Error('No address returned');
    // The header says which Zcash network this is. If the node hands back an
    // address from the other one, the header is wrong and the money would go
    // to a chain the buyer did not mean — so nothing is shown at all.
    if (!address.startsWith(config.zcashPrefix)) {
      throw new Error(`This node answered with an address that is not ${config.zcash}`);
    }
    field.textContent = address;
    copy.hidden = false;
    copy.onclick = async () => { await navigator.clipboard.writeText(address); note('Deposit address copied.'); };
    QRCode.toCanvas(root.querySelector('#deposit-qr'), address, { width: 164, margin: 1, color: { dark: '#1f2428', light: '#f8f2e4' } });
  } catch (e) {
    if (generation !== current) return;
    // Failing closed matters here: an address that is wrong, stale or someone
    // else's is worse than no address, so nothing is shown at all.
    field.textContent = 'Unavailable';
    note_.textContent = e.message === 'Failed to fetch'
      ? 'The node is unreachable, so no deposit address can be shown. Do not send ZEC anywhere until it returns.'
      : `No deposit address: ${e.message}`;
    root.querySelector('#deposit-qr')?.replaceChildren();
  }
}

/// Fungible holdings: what the account can spend, as opposed to what it owns
/// one of. ZEC.zy is always listed even at zero — somebody who has just sent a
/// deposit needs to watch it arrive, and an empty list would look like the
/// wallet had not noticed.

/// Hold-and-compute, for a chain that will not tell you what you own.
///
/// `zyn_published` carries every note the chain has ever published and every
/// nullifier it has seen. Opening the ones that belong to this wallet, and
/// dropping the ones already spent, is the balance. Nothing here asks the
/// operator a question whose answer they could tailor.
async function refreshObject(epoch, announce, active) {
  if (!identity?.viewing || !identity?.nullifier) {
    holdings = []; assets = []; listings = [];
    renderCoins(); renderItems();
    note('This wallet was stored before the object ledger existed. Restore it from the recovery phrase to read balances here.', true);
    return;
  }

  const published = await rpc(config.rpcUrl, 'zyn_published');
  if (!active()) return;
  const { held } = scanObjects(published, {
    secret: identity.viewing,
    nkey: identity.nullifier,
    owner: identity.account,
  });

  // An object ledger has no asset registry, so the only name this wallet can
  // put to an id is the one the deployment pinned. Anything else is shown by
  // its id rather than given a name nobody assigned.
  const zec = config.zecAsset;
  const seen = [...new Set(held.map((h) => h.asset))];
  assets = seen.map((id) => ({
    id,
    symbol: id === zec ? 'ZEC.zy' : `asset ${id.slice(0, 8)}…`,
  }));
  if (zec && !seen.includes(zec)) assets.unshift({ id: zec, symbol: 'ZEC.zy' });

  objectHeld = held;
  void revealExits();
  // `renderCoins` reads decimal strings, and these are WAD BigInts.
  holdings = held.map((h) => ({ asset: h.asset, amount: h.amount.toString() }));
  listings = [];
  renderCoins();
  renderItems();
  if (announce) note(`Synced at epoch ${epoch}.`);
}

function renderCoins() {
  const coins = holdings
    .map((h) => ({ ...h, meta: assets.find((a) => a.id === h.asset) }))
    .filter((h) => h.meta && !h.meta.content && decimalRaw(h.amount) > 0n);

  const deposit = assets.find((a) => a.symbol === 'ZEC.zy');
  if (deposit && !coins.some((c) => c.asset === deposit.id)) {
    coins.unshift({ asset: deposit.id, amount: '0', meta: deposit });
  }
  coins.sort((a, b) => (a.meta.symbol === 'ZEC.zy' ? -1 : b.meta.symbol === 'ZEC.zy' ? 1 : a.meta.symbol.localeCompare(b.meta.symbol)));

  const holder = root.querySelector('#coins');
  const count = root.querySelector('#coin-count');
  if (!holder) return;
  count.textContent = String(coins.length);
  holder.innerHTML = coins.map((c, i) => `<div class="coin"><div><p class="kicker">${escapeHtml(c.meta.symbol)}</p><b>${escapeHtml(readableAmount(c.amount))}</b></div>${config.kind === 'object' && decimalRaw(c.amount) > 0n ? `<button class="quiet coin-send" data-coin="${i}">Send →</button>` : (c.meta.symbol === 'ZEC.zy' ? '<span class="muted">deposited ZEC</span>' : '')}</div>`).join('');
  holder.querySelectorAll('[data-coin]').forEach((b) => { b.onclick = () => openObjectSend(coins[Number(b.dataset.coin)]); });
}

function renderItems() {
  for (const url of artUrls) URL.revokeObjectURL(url);
  artUrls.clear();
  // Held items, plus the ones resting in the order book. An escrowed item is
  // still the holder's — it is one cancellation away — so hiding it would be
  // telling them it is gone.
  const held = holdings.map((h) => ({ ...h, meta: assets.find((a) => a.id === h.asset) }))
    .filter((h) => h.meta?.content && decimalRaw(h.amount) > 0n);
  const resting = listings
    .map((o) => ({ asset: o.offerAsset, amount: o.offerAmount, owner: o.owner, via: o.via,
                   meta: assets.find((a) => a.id === o.offerAsset), listing: o }))
    .filter((h) => h.meta?.content);
  const list = [...held, ...resting];
  root.querySelector('#item-count').textContent = String(list.length);
  root.querySelector('#items').innerHTML = list.length ? list.map((h, i) => `<article class="item" data-index="${i}"><div class="art" data-content="${escapeHtml(h.meta.content)}"><img src="/nap-mark.png" alt="Generic placeholder; artwork not verified"></div><div><p class="kicker">ITEM · #${escapeHtml(h.asset)}</p><h3>${escapeHtml(h.meta.symbol || `Item ${h.asset}`)}</h3><p class="media-status">${h.listing ? `Listed · ${escapeHtml(readableAmount(h.listing.wantAmount))} ZEC.zy` : 'Artwork not verified'}</p><button>Review & send →</button> <button data-list>${h.listing ? 'Cancel listing' : 'List for sale'} →</button></div></article>`).join('') : `<div class="empty"><img src="/nap-mark.png" alt=""><h3>This pocket is quiet.</h3><p>Claim a gift or receive a Zyn item at the account below.</p></div>`;
  root.querySelectorAll('.item').forEach((el) => {
    const item = list[Number(el.dataset.index)];
    const [send, listBtn] = el.querySelectorAll('button');
    // An escrowed item cannot be sent — it is not in the account to send from.
    if (item.listing) { send.disabled = true; send.textContent = 'In escrow'; }
    else send.onclick = () => openSend(item);
    listBtn.onclick = () => openList(item);
  });
  // The gate. Nothing reaches the gateway until this is on, so the default
  // render makes no request that names an item.
  if (artEnabled()) {
    list.forEach((h, i) => loadVerifiedArt(root.querySelectorAll('.art')[i], h.meta.content));
  } else {
    root.querySelectorAll('.media-status').forEach((el) => {
      el.textContent = 'Artwork not fetched — privacy';
    });
  }
  const artToggle = root.querySelector('#art-toggle');
  if (artToggle) {
    artToggle.checked = artEnabled();
    artToggle.onchange = () => {
      setArtEnabled(artToggle.checked);
      note(
        artToggle.checked
          ? 'Artwork on. The gateway now learns which items you hold.'
          : 'Artwork off. Nothing is requested for your items.',
      );
      renderItems();
    };
  }
}
function decimalRaw(value) {
  if (!/^(0|[1-9]\d{0,38})(\.\d{1,18})?$/.test(String(value))) throw new Error('Invalid holdings amount');
  const [whole, frac = ''] = String(value).split('.');
  return BigInt(whole || 0) * WAD + BigInt(frac.padEnd(18, '0').slice(0, 18) || 0);
}
async function loadVerifiedArt(el, content) {
  try {
    if (!/^[a-f0-9]{64}$/.test(content)) return;
    const manifestBytes = await boundedBytes(`${config.mediaGateway.replace(/\/$/, '')}/${content}`, MAX_MANIFEST);
    const committed = new Uint8Array(manifestBytes.length + 1); committed.set(manifestBytes, 1);
    if (hex(new Uint8Array(await crypto.subtle.digest('SHA-256', committed))) !== content) return;
    const manifest = parseManifest(manifestBytes);
    const media = manifest.media.find((m) => m.role === 0);
    if (!media) return;
    if (media.bytes > MAX_IMAGE) return;
    const bytes = await boundedBytes(`${config.mediaGateway.replace(/\/$/, '')}/${media.hash}`, MAX_IMAGE);
    if (bytes.length !== media.bytes || hex(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))) !== media.hash) return;
    if (!safeRaster(media, bytes) || !el.isConnected) return;
    const url = URL.createObjectURL(new Blob([bytes], { type: media.mime }));
    artUrls.add(url);
    el.innerHTML = `<img src="${url}" alt="${escapeHtml(manifest.name)}">`;
    el.classList.add('verified');
    el.closest('.item').querySelector('.media-status').textContent = 'Artwork hash verified';
  } catch (_) { /* the committed placeholder remains */ }
}

/// List an item, or take it back off the market.
///
/// The frame is signed by `item.owner` — the receiving account for anything
/// bought through a store — because an offer is escrowed from the account that
/// holds the item, and the identity holds nothing (§124.3).
function openList(item) {
  const owner = item.owner || identity, current = generation;
  const dialog = root.querySelector('#send-dialog');
  const listed = item.listing;
  dialog.innerHTML = listed
    ? `<button class="x">×</button><p class="kicker">LISTING</p><h2>${escapeHtml(item.meta.symbol || `Item ${item.asset}`)}</h2><p class="muted">This item is escrowed in the order book until it is taken or you cancel. Cancelling returns it to this account.</p><div class="review"><span>Offer</span><b>#${listed.id}</b><span>Asking</span><b>${escapeHtml(readableAmount(listed.wantAmount))} ZEC.zy</b></div><form><button class="primary" type="submit">Cancel the listing <span>→</span></button><p class="error" role="alert"></p></form>`
    : `<button class="x">×</button><p class="kicker">LIST FOR SALE</p><h2>${escapeHtml(item.meta.symbol || `Item ${item.asset}`)}</h2><p class="muted">The item is escrowed while it rests. Nobody can take it except at your price, and you can cancel until they do.</p><form><label>Price in ZEC.zy<input id="ask" autocomplete="off" inputmode="decimal" placeholder="1.5"></label><div class="review"><span>Item</span><b>#${item.asset.slice(0, 12)}…</b><span>Rests until</span><b>cancelled</b></div><button class="primary" type="submit">Sign &amp; list <span>→</span></button><p class="error" role="alert"></p></form>`;
  dialog.querySelector('.x').onclick = () => dialog.close();

  let submitted = false;
  dialog.querySelector('form').onsubmit = async (event) => {
    event.preventDefault();
    const button = dialog.querySelector('.primary');
    button.disabled = true;
    try {
      const status = await rpc(config.rpcUrl, 'zyn_status');
      const epoch = checkedEpoch(status, config.chain);
      if (generation !== current) return;
      const intent = listed
        ? cancelOfferIntent(owner.account, BigInt(listed.id))
        // Rests until cancelled: a silent lapse would look like a theft to
        // whoever was about to take it.
        : placeOfferIntent(owner.account, item.asset, WAD, XZEC, parseAmount(dialog.querySelector('#ask').value), 0xffffffffffffffffn);
      const frame = signedFrame(owner, config.chain, epoch, intent);
      submitted = true;
      const accepted = await rpc(config.rpcUrl, 'zyn_sendRawIntent', [hex(frame)]);
      if (generation !== current) return;
      dialog.close(); await refresh(false);
      if (generation !== current) return;
      note(listed ? `Listing cancelled at sequence ${accepted.seq}.` : `Listed at sequence ${accepted.seq}; acceptance is not anchored finality.`);
    } catch (e) {
      if (generation !== current) return;
      dialog.querySelector('.error').textContent = submitted ? `${e.message}. Submission outcome unknown: refresh and check before trying again.` : e.message;
      button.disabled = submitted;
    }
  };
  dialog.showModal();
}

function openSend(item) {
  // The key that holds *this* item, which is the identity for a gift and a
  // receiving account for anything bought through a store. Signing with the
  // identity would be a signature from an account that holds nothing.
  const owner = item.owner || identity, current = generation;
  let reviewedTo = null;
  const dialog = root.querySelector('#send-dialog');
  dialog.innerHTML = `<button class="x">×</button><p class="kicker">SEND ONE ITEM</p><h2>${escapeHtml(item.meta.symbol || `Item ${item.asset}`)}</h2><p class="muted">Nap sends exactly one whole NFT unit. Review the destination carefully; transfers cannot be undone.</p><form><label>Recipient Zyn account<input id="send-to" autocomplete="off" autocapitalize="none" spellcheck="false" placeholder="64 hex characters"></label><div class="review"><span>Asset</span><b>#${item.asset}</b><span>Amount</span><b>1 item</b><span>Network</span><b>${escapeHtml(config.network)} · ${config.chain}</b></div><button class="primary" type="submit">Sign & relay <span>→</span></button><p class="error" role="alert"></p></form>`;
  dialog.querySelector('.x').onclick = () => dialog.close();
  dialog.querySelector('.primary').textContent = 'Review destination →';
  dialog.querySelector('form').onsubmit = async (event) => {
    event.preventDefault(); const button = dialog.querySelector('.primary'); button.disabled = true;
    let submitted = false;
    try {
      const to = unhex(dialog.querySelector('#send-to').value, 32);
      if (generation !== current) throw new Error('Wallet locked or changed. Restore and review again.');
      if (to.every((b) => b === 0) || hex(to) === owner.accountHex) throw new Error('Choose a different, nonzero recipient account');
      if (!reviewedTo) {
        reviewedTo = hex(to);
        dialog.querySelector('#send-to').value = reviewedTo;
        dialog.querySelector('#send-to').readOnly = true;
        button.textContent = 'Confirm: sign & relay 1 item'; button.disabled = false;
        dialog.querySelector('.error').textContent = 'Check all 64 recipient characters above. Close to edit the destination.';
        return;
      }
      if (hex(to) !== reviewedTo) throw new Error('Destination changed; close and review again');
      const status = await rpc(config.rpcUrl, 'zyn_status');
      const epoch = checkedEpoch(status, config.chain);
      if (generation !== current) return;
      const account = await rpc(config.rpcUrl, 'zyn_account', [hex(signedRead(owner, config.chain, epoch))]);
      if (generation !== current) return;
      if (!account?.spendable?.some((h) => h.asset === item.asset && decimalRaw(h.amount) >= WAD)) throw new Error('This account no longer holds one whole item');
      const frame = transferFrame(owner, config.chain, epoch, to, item.asset, WAD);
      submitted = true;
      const accepted = await rpc(config.rpcUrl, 'zyn_sendRawIntent', [hex(frame)]);
      if (generation !== current) return;
      dialog.close(); await refresh(false);
      if (generation !== current) return;
      note(accepted.queued ? `Transfer queued at sequence ${accepted.seq}; not yet settled.` : `Transfer accepted at sequence ${accepted.seq}; acceptance is not anchored finality.`);
    } catch (e) {
      if (generation !== current) return;
      dialog.querySelector('.error').textContent = submitted ? `${e.message}. Submission outcome unknown: refresh holdings and check the node before trying again.` : e.message;
      button.disabled = submitted;
    }
  };
  dialog.showModal();
}

/// Send a coin to another Zyn account on the object ledger: retire notes this
/// wallet holds and create one for the recipient, sealed to the viewing key
/// they registered, plus change sealed to this wallet. Nothing on chain names
/// either account in the clear.
function openObjectSend(coin) {
  const current = generation;
  let reviewed = null;
  const dialog = root.querySelector('#send-dialog');
  dialog.innerHTML = `<button class="x">×</button><p class="kicker">SEND ${escapeHtml(coin.meta.symbol)}</p><h2>To another Zyn account.</h2><p class="muted">The chain records that notes were spent and created — not who sent, who received, or how much. Transfers cannot be undone.</p><form><label>Recipient Zyn account<input id="send-to" autocomplete="off" autocapitalize="none" spellcheck="false" placeholder="64 hex characters"></label><label>Amount<input id="send-amount" inputmode="decimal" autocomplete="off" placeholder="0.0"></label><div class="review"><span>Asset</span><b>${escapeHtml(coin.meta.symbol)}</b><span>You hold</span><b>${escapeHtml(readableAmount(coin.amount))}</b><span>Network</span><b>${escapeHtml(config.network)} · ${config.chain}</b></div><button class="primary" type="submit">Review →</button><p class="error" role="alert"></p></form>`;
  dialog.querySelector('.x').onclick = () => dialog.close();
  dialog.querySelector('form').onsubmit = async (event) => {
    event.preventDefault();
    const button = dialog.querySelector('.primary');
    const error = dialog.querySelector('.error');
    button.disabled = true;
    try {
      const to = unhex(dialog.querySelector('#send-to').value.trim(), 32);
      const amount = parseAmount(dialog.querySelector('#send-amount').value.trim());
      if (to.every((b) => b === 0) || hex(to) === identity.accountHex) throw new Error('Choose a different, nonzero recipient account');
      if (!reviewed) {
        reviewed = `${hex(to)}:${amount}`;
        dialog.querySelector('#send-to').value = hex(to);
        dialog.querySelectorAll('input').forEach((el) => { el.readOnly = true; });
        button.textContent = `Confirm: send ${readableAmount(dialog.querySelector('#send-amount').value.trim())} ${coin.meta.symbol}`;
        button.disabled = false;
        error.textContent = 'Check all 64 recipient characters above. Close to edit.';
        return;
      }
      if (`${hex(to)}:${amount}` !== reviewed) throw new Error('Details changed; close and review again');
      const { viewing } = await objectCall(config.rpcUrl, 'zyn_viewingKey', { account: hex(to) });
      if (generation !== current) return;
      const asset = unhex(coin.asset, 32);
      const { inputs, change } = fundFrom(identity, objectHeld, asset, amount);
      const note = { asset, amount, owner: to, nonce: crypto.getRandomValues(new Uint8Array(32)) };
      const enc = sealNote(unhex(viewing, 32), crypto.getRandomValues(new Uint8Array(32)), note);
      await submitObject(spendIntent(identity.account, OBJECT_PROGRAM.privateTransfer, inputs, [{ note, enc }, ...change]));
      if (generation !== current) return;
      dialog.close();
      note_(`Sent ${readableAmount(dialog.querySelector('#send-amount')?.value || '')} ${coin.meta.symbol}.`);
      refresh(false);
    } catch (e) {
      if (generation !== current) return;
      error.textContent = e.message;
      button.disabled = false;
    }
  };
  dialog.showModal();
}
const note_ = (text) => note(text);

async function claimGift() {
  const owner = identity, current = generation;
  const button = root.querySelector('#claim'); button.disabled = true;
  try {
    checkedEpoch(await rpc(config.rpcUrl, 'zyn_status'), config.chain);
    if (generation !== current) return;
    const claimAttemptId = hex(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(JSON.stringify(['nap.claim.v1', claimCode, owner.accountHex, config.chain])))));
    if (generation !== current) return;
    const response = await fetch(`${config.claimUrl.replace(/\/$/, '')}/api/claims`, { method: 'POST', headers: { 'content-type': 'application/json', 'idempotency-key': claimAttemptId }, body: JSON.stringify({ code: claimCode, account: owner.accountHex, chain: config.chain }), cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer', signal: AbortSignal.timeout(15000) });
    const body = await response.json().catch(() => ({}));
    if (generation !== current) return;
    if (!response.ok) throw new Error(body.message || (response.status === 409 ? 'This gift has already been claimed.' : 'The gift coordinator rejected this claim.'));
    const state = body.state || 'submitted';
    await refresh(false);
    if (generation !== current) return;
    note(`Coordinator reports gift ${state}. Check your holdings; this response is not proof of anchored finality.`);
  } catch (e) { if (generation === current) note(e.message === 'Failed to fetch' ? 'Gift coordinator unavailable. Outcome unknown; retry with this same card and account.' : e.message, true); }
  finally { button.disabled = false; }
}

try {
  const local = ['localhost', '127.0.0.1', '::1'].includes(location.hostname);
  if (!isSecureContext && !local) throw new Error('Nap Web requires HTTPS before it can create or restore an account');
  if (!crypto?.getRandomValues || !crypto?.randomUUID) throw new Error('This browser does not provide the secure randomness Nap requires');
  if (!crypto.subtle || !AbortSignal.timeout) throw new Error('This browser lacks the cryptography or request timeout support Nap requires');
  config = await loadConfig(); onboarding();
}
catch (e) { root.innerHTML = `<main class="fatal"><img src="/nap-mark.png" alt=""><h1>Nap is not configured.</h1><p>${escapeHtml(e.message)}</p></main>`; }

if ('serviceWorker' in navigator && location.protocol === 'https:') navigator.serviceWorker.register('/sw.js').catch(() => {});
