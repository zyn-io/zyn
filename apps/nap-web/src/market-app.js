// Nap Market: swap, pools and launches on the object ledger, from the browser.
//
// The same origin as the wallet, on purpose: the kept key, the chain route and
// the content policy are the wallet's, so this page is not a second place a key
// can live or a second endpoint a holder has to trust.
//
// Nothing here asks the chain what an account holds. Balances come from opening
// published notes with the holder's own viewing key; what the market owes comes
// from claim ids only this wallet can derive. The node learns what a sequencer
// must — the intents — and the public learns pool reserves and sale totals.

import { checkedEpoch, deriveIdentity, forgetIdentity, openSealedWallet, parseAmount } from './core.js';
import { hex, scan, signedFrame, submitSigned, unhex } from './object.js';
import * as mk from './market.js';
import './styles.css';
import './market.css';

const root = document.querySelector('#app');
const KEPT_KEY = 'nap.kept.v1';
const NETWORK_KEY = 'nap.network';
/// Epochs a signature stays valid. Long enough to survive a slow submit,
/// short enough that a frame found later is useless.
const VALIDITY = 100n;
/// How far past the last nonce this device used to look for claims — the
/// other device, or a restore, may have used some.
const LOOKAHEAD = 24;
/// Claims per redemption: the VM's I/O limit.
const PER_CLAIM = 16;

let config;
let identity = null;
let view = 'swap';
let state = { epoch: 0n, epochSeconds: 0, pools: [], launches: [], held: [], balances: new Map(), owed: [] };
let busy = false;
let timer = null;
const ui = { pool: null, sellA: true, slippage: 100 };

// ---------------------------------------------------------------------------
// Small things.
// ---------------------------------------------------------------------------

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const short = (h) => `${h.slice(0, 6)}…${h.slice(-4)}`;
const $ = (sel) => root.querySelector(sel);
const read = (key) => { try { return localStorage.getItem(key); } catch { return null; } };
const write = (key, v) => { try { localStorage.setItem(key, v); } catch { /* this session only */ } };

/// A WAD amount as a decimal, trimmed, to `places` decimals. Never through a
/// double: the balance is shown exactly or not at all.
function fmt(raw, places = 6) {
  const v = BigInt(raw);
  const neg = v < 0n;
  const a = neg ? -v : v;
  const whole = a / mk.WAD;
  let frac = (a % mk.WAD).toString().padStart(18, '0').slice(0, places).replace(/0+$/, '');
  const w = whole.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${neg ? '−' : ''}${w}${frac ? '.' + frac : ''}`;
}

/// `num/den` as a decimal with `places`, for prices.
function ratio(num, den, places = 6) {
  if (den === 0n) return '—';
  return fmt((num * mk.WAD) / den, places);
}

function parse(value) {
  const s = String(value || '').trim().replace(/,/g, '');
  return parseAmount(s);
}

function name(assetHex) {
  if (assetHex === config.zecAsset) return 'ZEC.zy';
  const launch = state.launches.find((l) => l.token === assetHex);
  if (launch) return launch.symbol || short(assetHex);
  const pool = state.pools.find((p) => p.lp === assetHex);
  if (pool) return `LP ${name(pool.a)}/${name(pool.b)}`;
  return short(assetHex);
}

const balance = (assetHex) => state.balances.get(assetHex) || 0n;

/// Epochs as wall-clock time. An epoch is at most `epoch_seconds` long while
/// a sale is open — the node seals on the clock then — so this is an upper
/// bound, and said as one.
function span(epochs) {
  const secs = Number(epochs) * state.epochSeconds;
  if (!state.epochSeconds || secs <= 0) return '';
  if (secs < 90) return `≤ ${secs}s`;
  if (secs < 5400) return `≤ ${Math.round(secs / 60)} min`;
  if (secs < 172800) return `≤ ${Math.round(secs / 3600)} h`;
  return `≤ ${Math.round(secs / 86400)} days`;
}

// ---------------------------------------------------------------------------
// Configuration and identity — the wallet's, read the wallet's way.
// ---------------------------------------------------------------------------

async function loadConfig() {
  const response = await fetch('/config.json', { cache: 'no-store' });
  if (!response.ok) throw new Error('Deployment is missing config.json');
  const value = await response.json();
  const networks = Array.isArray(value.networks) && value.networks.length ? value.networks : [{ id: 'default', ...value }];
  const wanted = read(NETWORK_KEY);
  const chosen = networks.find((n) => n.id === wanted) || networks.find((n) => n.chain === value.chain) || networks[0];
  const url = new URL(chosen.rpcUrl, location.origin);
  if (url.origin !== location.origin) throw new Error('rpcUrl must be a same-origin path');
  if (chosen.kind !== 'object') throw new Error('The market runs on an object ledger, and this network is not one');
  if (!/^[0-9a-f]{64}$/.test(String(chosen.zecAsset))) throw new Error('This network names no ZEC asset');
  return { ...chosen, rpcUrl: url.pathname, networks };
}

const keptWallet = () => { try { return JSON.parse(read(KEPT_KEY) || 'null'); } catch { return null; } };

function lock() {
  forgetIdentity(identity);
  identity = null;
  state = { epoch: 0n, pools: [], launches: [], held: [], balances: new Map(), owed: [] };
  clearInterval(timer);
  locked();
}

// ---------------------------------------------------------------------------
// Talking to the chain.
// ---------------------------------------------------------------------------

async function call(method, fields = {}) {
  const id = crypto.randomUUID();
  const response = await fetch(config.rpcUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id, method, params: [], ...fields }),
    cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer',
    signal: AbortSignal.timeout(15000),
  });
  if (!response.ok) throw new Error(`The chain answered HTTP ${response.status}`);
  const body = await response.json();
  if (body.id !== id) throw new Error('The chain answered a different question');
  if (body.error) throw new Error(body.error.message || 'The chain refused');
  return body.result;
}

// Nonces are receipts, so they are counted per chain and account and never
// handed out twice by this device.
const counterKey = () => `nap.market.n.${config.chain}.${identity.accountHex}`;
const counter = () => Number(read(counterKey()) || 0);
function nextNonce() {
  const n = counter();
  write(counterKey(), String(n + 1));
  return mk.marketNonce(identity.secretKey, n);
}

/// Every claim this wallet could be owed, from every nonce it may have used.
function claimSources(limit) {
  const out = [];
  const me = identity.account;
  for (let n = 0; n < limit; n++) {
    const nonce = mk.marketNonce(identity.secretKey, n);
    for (const source of [mk.orderId(me, nonce), mk.liquidityId(me, nonce), mk.commitId(me, nonce), mk.launchSource(me, nonce)]) {
      out.push(...mk.claimIds(source).map(hex));
    }
  }
  return out;
}

async function loadOwed() {
  const ids = claimSources(counter() + LOOKAHEAD);
  const rows = [];
  for (let i = 0; i < ids.length; i += 240) {
    const part = await call('zyn_claims', { ids: ids.slice(i, i + 240).join(',') });
    if (Array.isArray(part)) rows.push(...part);
  }
  return rows.filter((r) => /^[0-9a-f]{64}$/.test(r.id) && /^[0-9a-f]{64}$/.test(r.asset) && /^\d+$/.test(r.amount));
}

// What the node says goes into markup, ids and arithmetic, so its shape is
// checked first: a row that is not exactly hex ids and integer strings is
// dropped rather than rendered.
const isHex = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
const isInt = (v) => typeof v === 'string' && /^\d{1,40}$/.test(v);
const isSmall = (v) => Number.isSafeInteger(v) && v >= 0;
const validPool = (p) => ['id', 'a', 'b', 'lp'].every((k) => isHex(p[k]))
  && ['reserve_a', 'reserve_b', 'shares', 'locked'].every((k) => isInt(p[k]))
  && BigInt(p.reserve_a) > 0n && BigInt(p.reserve_b) > 0n && BigInt(p.shares) > 0n && isSmall(p.fee_bps);
const validLaunch = (l) => ['token', 'quote', 'meta'].every((k) => isHex(l[k]))
  && ['total_supply', 'sale_supply', 'pool_supply', 'soft_cap', 'hard_cap', 'raised'].every((k) => isInt(l[k]))
  && ['open', 'graduated', 'failed'].includes(l.state) && isSmall(l.closes_at_epoch) && isSmall(l.pool_bps) && isSmall(l.fee_bps);

async function refresh(quiet = false) {
  if (!identity) return;
  try {
    const status = await call('zyn_status');
    const epoch = checkedEpoch(status, config.chain);
    const [market, published, owed] = await Promise.all([call('zyn_market'), call('zyn_published'), loadOwed()]);
    const { held, balances } = scan(published, { secret: identity.viewing, nkey: identity.nullifier, owner: identity.account });
    state = {
      epoch,
      epochSeconds: isSmall(market.epoch_seconds) ? market.epoch_seconds : 0,
      pools: (market.pools || []).filter(validPool).map((p) => ({ ...p, pending: Number(p.pending) || 0 })),
      launches: (market.launches || []).filter(validLaunch).map((l) => ({ ...l, ...(mk.decodeMeta(l.meta) || {}) })),
      held,
      balances,
      owed,
    };
    if (!ui.pool && state.pools.length) ui.pool = state.pools[0].id;
    render();
    if (!quiet) notice(`Up to date at epoch ${epoch}.`);
  } catch (e) {
    notice(e.message === 'Failed to fetch' ? 'The chain is not answering. Nothing was sent.' : e.message, true);
  }
}

/// Sign an intent and hand it to the holders' door. A refusal inside the
/// receipts is an error here too — the frame was accepted, the intent was not.
async function submit(intent, what) {
  if (busy) return;
  busy = true;
  setBusy(true);
  try {
    const status = await call('zyn_status');
    const epoch = checkedEpoch(status, config.chain);
    const frame = await signedFrame(identity.secretKey, config.chain, epoch + VALIDITY, intent);
    const result = await submitSigned(config.rpcUrl, frame);
    const refused = /Refused\("([^"]*)"\)/.exec(String(result?.receipts || ''));
    if (refused) throw new Error(`The chain refused: ${refused[1]}`);
    notice(what);
    await refresh(true);
    return true;
  } catch (e) {
    notice(e.message, true);
    return false;
  } finally {
    busy = false;
    setBusy(false);
  }
}

// ---------------------------------------------------------------------------
// Screens.
// ---------------------------------------------------------------------------

function shell(content) {
  root.innerHTML = `<div class="grain"></div>
  <header class="mk-header">
    <a class="brand" href="/" aria-label="Nap Wallet"><img src="/nap-mark.png" alt=""><span>Nap</span><em>Market</em></a>
    <div class="mk-head-right">
      <span class="network"><i></i>${esc(config.network)} · chain ${config.chain}</span>
      <a class="mk-link" href="/">Wallet →</a>
    </div>
  </header>${content}
  <footer><span>Keys stay in this tab. Orders clear together, once an epoch.</span><span>Nap Market · Zyn</span></footer>`;
}

function locked(error = '') {
  const kept = keptWallet();
  shell(`<main class="onboard mk-onboard">
    <section class="intro">
      <p class="eyebrow">PRIVATE MARKET / OBJECT LEDGER</p>
      <h1>Nobody<br><em>cuts in line.</em></h1>
      <p class="lede">Swaps clear together at one price at the end of each epoch, so there is no queue to jump. Launches sell at one price to everyone. Who traded is not shown to the public; the operator sees it today, and the plan is that one day nobody does.</p>
      <div class="promise"><span>BATCH</span><p><b>Nothing to sandwich.</b><br>Every order in an epoch pays the same price, so where yours sits in the batch cannot matter.</p></div>
      <div class="promise"><span>CLAIM</span><p><b>Yours to collect.</b><br>What you are owed is findable only with your key.</p></div>
    </section>
    <section class="entry panel">
      ${kept ? `<form id="kept-form"><p class="kicker">THIS DEVICE</p><h2>Open your wallet.</h2><label>Password<input id="kept-password" type="password" autocomplete="current-password" autofocus></label><button class="primary" type="submit">Open <span>→</span></button></form><p class="muted"><a href="" id="use-phrase">Use a recovery phrase instead</a></p>` : ''}
      <form id="phrase-form" ${kept ? 'hidden' : ''}><p class="kicker">RECOVERY PHRASE</p><h2>Wake your wallet.</h2><label>24-word recovery phrase<textarea id="words" rows="4" autocomplete="off" autocapitalize="none" spellcheck="false"></textarea></label><div class="split"><label>Passphrase<input id="passphrase" type="password" autocomplete="off" placeholder="Blank if unused"></label><label>Account<input id="account-index" type="number" min="0" value="0"></label></div><button class="primary" type="submit">Open <span>→</span></button></form>
      <p class="error" id="form-error" role="alert">${esc(error)}</p>
      <div class="scope"><b>${esc(config.network)}</b><span>The market reads the same account as your wallet. Nothing new is stored here.</span></div>
    </section>
  </main>`);
  $('#use-phrase')?.addEventListener('click', (e) => { e.preventDefault(); $('#phrase-form').hidden = false; $('#kept-form').hidden = true; });
  $('#kept-form')?.addEventListener('submit', async (e) => {
    e.preventDefault();
    try {
      const opened = await openSealedWallet(keptWallet(), $('#kept-password').value);
      begin(opened.identity);
    } catch (err) { $('#form-error').textContent = err.message; }
  });
  $('#phrase-form').addEventListener('submit', (e) => {
    e.preventDefault();
    try {
      const id = deriveIdentity($('#words').value, $('#passphrase').value, Number($('#account-index').value));
      $('#words').value = '';
      begin(id);
    } catch (err) { $('#form-error').textContent = err.message; }
  });
}

function begin(id) {
  if (!id.viewing || !id.nullifier) {
    locked('This kept wallet predates the object ledger. Open it with the recovery phrase once.');
    return;
  }
  identity = id;
  frame();
  refresh();
  clearInterval(timer);
  timer = setInterval(() => { if (!document.hidden && !busy) refresh(true); }, 20000);
}

function frame() {
  shell(`<main class="wallet mk">
    <aside class="rail">
      <div><p class="kicker">TRADING AS</p><b class="mk-account">${short(identity.accountHex)}</b><p class="mk-epoch" id="epoch">epoch —</p></div>
      <nav id="tabs">
        ${['swap', 'pools', 'launchpad', 'owed'].map((t) => `<button data-view="${t}">${t === 'owed' ? 'Owed to you' : t[0].toUpperCase() + t.slice(1)}<span class="mk-badge" id="badge-${t}"></span></button>`).join('')}
      </nav>
      <button class="quiet" id="lock">Lock</button>
    </aside>
    <section class="content">
      <div class="notice" id="notice" role="status"></div>
      <div id="view"></div>
    </section>
  </main>`);
  $('#lock').onclick = lock;
  root.querySelectorAll('#tabs [data-view]').forEach((b) => { b.onclick = () => { view = b.dataset.view; render(); }; });
}

function notice(text, bad = false) {
  const el = $('#notice');
  if (!el) return;
  el.textContent = text;
  el.classList.add('show');
  el.classList.toggle('bad', bad);
}

function setBusy(on) {
  root.querySelectorAll('.primary, .mk-act').forEach((b) => { b.disabled = on; });
}

function render() {
  if (!identity || !$('#view')) return;
  $('#epoch').textContent = `epoch ${state.epoch}`;
  root.querySelectorAll('#tabs [data-view]').forEach((b) => b.classList.toggle('active', b.dataset.view === view));
  const owedBadge = $('#badge-owed');
  owedBadge.textContent = state.owed.length ? String(state.owed.length) : '';
  const open = state.launches.filter((l) => l.state === 'open').length;
  $('#badge-launchpad').textContent = open ? String(open) : '';
  ({ swap: renderSwap, pools: renderPools, launchpad: renderLaunchpad, owed: renderOwed })[view]();
}

// ---------------------------------------------------------------------------
// Swap.
// ---------------------------------------------------------------------------

function renderSwap() {
  const pools = state.pools;
  if (!pools.length) {
    $('#view').innerHTML = `${heading('SWAP', 'No pools yet.')}<div class="mk-empty"><p>A pool opens when someone provides both sides of a pair. Launches open theirs when their sale closes.</p><button class="mk-act" id="go-pools">Open a pool →</button></div>`;
    $('#go-pools').onclick = () => { view = 'pools'; render(); };
    return;
  }
  const pool = pools.find((p) => p.id === ui.pool) || pools[0];
  ui.pool = pool.id;
  const sell = ui.sellA ? pool.a : pool.b;
  const buy = ui.sellA ? pool.b : pool.a;
  $('#view').innerHTML = `${heading('SWAP', 'One price<br><em>per epoch.</em>')}
  <div class="mk-grid">
    <form class="mk-card mk-swap" id="swap-form">
      <label class="mk-field"><span>Pair</span><select id="pool-pick">${pools.map((p) => `<option value="${p.id}" ${p.id === pool.id ? 'selected' : ''}>${esc(name(p.a))} / ${esc(name(p.b))}</option>`).join('')}</select></label>
      <div class="mk-leg">
        <div class="mk-leg-top"><span>You sell</span><button type="button" class="mk-max" id="max">Balance ${fmt(balance(sell))}</button></div>
        <div class="mk-leg-row"><input id="amount" inputmode="decimal" autocomplete="off" placeholder="0.0"><b>${esc(name(sell))}</b></div>
      </div>
      <button type="button" class="mk-flip" id="flip" aria-label="Swap direction">⇅</button>
      <div class="mk-leg mk-leg-out">
        <div class="mk-leg-top"><span>You receive, if alone in the batch</span><span id="impact"></span></div>
        <div class="mk-leg-row"><output id="quote">0</output><b>${esc(name(buy))}</b></div>
      </div>
      <div class="mk-slip"><span>Accept at worst</span>${[50, 100, 300].map((b) => `<button type="button" data-slip="${b}" class="${ui.slippage === b ? 'on' : ''}">${b / 100}%</button>`).join('')}<span class="mk-min" id="min"></span></div>
      <button class="primary mk-act" type="submit">Place in batch #${state.epoch} <span>→</span></button>
      <p class="mk-fine">Your ${esc(name(sell))} is held in the batch now. At the epoch's seal every order in this pool clears at one price; if that price is worse than your minimum you get everything back. Either way it arrives under <b>Owed to you</b>.</p>
    </form>
    <aside class="mk-card mk-side">
      ${poolFacts(pool)}
      <div class="mk-explain"><h3>Why nobody can jump the queue</h3><p>All orders in an epoch pay <code>p = (y + 2·b′) / (x + 2·a′)</code>, the price at which the pool ends where it traded. There is no order inside the batch, so there is nothing to sandwich — not even for the sequencer, who is the only one who sees it.</p></div>
    </aside>
  </div>`;
  const amount = $('#amount');
  const update = () => {
    let raw = 0n;
    try { raw = amount.value ? parse(amount.value) : 0n; } catch { raw = 0n; }
    const q = raw > 0n ? mk.quoteAlone(pool, ui.sellA, raw) : 0n;
    $('#quote').textContent = fmt(q);
    const min = mk.withSlippage(q, ui.slippage);
    $('#min').textContent = raw > 0n ? `min ${fmt(min)} ${name(buy)}` : '';
    const x = BigInt(pool.reserve_a), y = BigInt(pool.reserve_b);
    const spot = ui.sellA ? (raw * y) / x : (raw * x) / y;
    $('#impact').textContent = raw > 0n && spot > 0n ? `${fmt(((spot - q) * 10000n) / spot * mk.WAD / 100n, 2)}% below spot` : '';
  };
  amount.oninput = update;
  $('#max').onclick = () => { amount.value = fmt(balance(sell), 18).replace(/,/g, ''); update(); };
  $('#flip').onclick = () => { ui.sellA = !ui.sellA; renderSwap(); };
  $('#pool-pick').onchange = (e) => { ui.pool = e.target.value; renderSwap(); };
  root.querySelectorAll('[data-slip]').forEach((b) => { b.onclick = () => { ui.slippage = Number(b.dataset.slip); root.querySelectorAll('[data-slip]').forEach((x) => x.classList.toggle('on', x === b)); update(); }; });
  $('#swap-form').onsubmit = async (e) => {
    e.preventDefault();
    try {
      const raw = parse(amount.value);
      const minOut = mk.withSlippage(mk.quoteAlone(pool, ui.sellA, raw), ui.slippage);
      const { inputs, change } = mk.fund(identity, state.held, unhex(sell), raw);
      const nonce = nextNonce();
      await submit(mk.swapIntent({ trader: identity.account, pool: unhex(pool.id), sellA: ui.sellA, amountIn: raw, minOut, inputs, change, nonce }),
        `Order placed in batch #${state.epoch}. It clears at the seal; collect it under Owed to you.`);
    } catch (err) { notice(err.message, true); }
  };
}

function poolFacts(p) {
  const x = BigInt(p.reserve_a), y = BigInt(p.reserve_b);
  const mine = balance(p.lp);
  const shares = BigInt(p.shares);
  return `<dl class="mk-facts">
    <dt>Price</dt><dd>1 ${esc(name(p.a))} = ${ratio(y, x)} ${esc(name(p.b))}</dd>
    <dt>Reserves</dt><dd>${fmt(x, 4)} ${esc(name(p.a))}<br>${fmt(y, 4)} ${esc(name(p.b))}</dd>
    <dt>Fee</dt><dd>${(p.fee_bps / 100).toFixed(2)}% to liquidity</dd>
    <dt>This batch</dt><dd>${p.pending ? `${p.pending} order${p.pending === 1 ? '' : 's'} waiting` : 'empty so far'}</dd>
    ${mine > 0n ? `<dt>Your share</dt><dd>${ratio(mine * 100n, shares, 3)}%</dd>` : ''}
    ${BigInt(p.locked) === shares ? '<dt>Liquidity</dt><dd>locked for good</dd>' : ''}
  </dl>`;
}

function heading(eyebrow, title) {
  return `<div class="topline"><div><p class="eyebrow">${eyebrow}</p><h1 class="mk-h1">${title}</h1></div><button class="refresh" id="refresh">↻ Refresh</button></div>`;
}

// ---------------------------------------------------------------------------
// Pools.
// ---------------------------------------------------------------------------

function renderPools() {
  const assets = [...new Set([config.zecAsset, ...state.held.map((h) => h.asset)])];
  $('#view').innerHTML = `${heading('POOLS', 'Provide both sides,<br><em>earn every batch.</em>')}
  <section class="mk-list">${state.pools.length ? state.pools.map((p) => `
    <article class="mk-card mk-pool">
      <header><h3>${esc(name(p.a))} <span>/</span> ${esc(name(p.b))}</h3><span class="mk-pill">${(p.fee_bps / 100).toFixed(2)}%</span></header>
      ${poolFacts(p)}
      <div class="mk-row">
        <button class="mk-act" data-trade="${p.id}">Trade</button>
        <button class="mk-act" data-add="${p.id}">Add</button>
        ${balance(p.lp) > 0n ? `<button class="mk-act" data-remove="${p.id}">Remove</button>` : ''}
      </div>
      <form class="mk-inline" id="add-${p.id}" hidden>
        <label class="mk-field"><span>${esc(name(p.a))}</span><input name="a" inputmode="decimal" autocomplete="off" placeholder="0.0"></label>
        <label class="mk-field"><span>${esc(name(p.b))}</span><input name="b" inputmode="decimal" autocomplete="off" placeholder="matched to the ratio"></label>
        <p class="mk-fine" data-preview></p>
        <button class="primary mk-act" type="submit">Add liquidity <span>→</span></button>
      </form>
      <form class="mk-inline" id="remove-${p.id}" hidden>
        <label class="mk-field"><span>Shares (you hold ${fmt(balance(p.lp))})</span><input name="s" inputmode="decimal" autocomplete="off"></label>
        <button class="primary mk-act" type="submit">Remove <span>→</span></button>
      </form>
    </article>`).join('') : '<div class="mk-empty"><p>No pools yet. Open the first one below.</p></div>'}</section>
  <form class="mk-card mk-create" id="create-pool">
    <h2>Open a pool</h2>
    <p class="mk-fine">You set the first price by what you put in. A sliver of the first shares is locked for good so the pool can never be emptied and restarted at a different price.</p>
    <div class="mk-two">
      <label class="mk-field"><span>First asset</span><select name="a">${assets.map((a) => `<option value="${a}">${esc(name(a))} · ${fmt(balance(a), 4)}</option>`).join('')}</select></label>
      <label class="mk-field"><span>Amount</span><input name="amount_a" inputmode="decimal" autocomplete="off"></label>
      <label class="mk-field"><span>Second asset</span><select name="b">${assets.map((a, i) => `<option value="${a}" ${i === 1 ? 'selected' : ''}>${esc(name(a))} · ${fmt(balance(a), 4)}</option>`).join('')}</select></label>
      <label class="mk-field"><span>Amount</span><input name="amount_b" inputmode="decimal" autocomplete="off"></label>
    </div>
    <label class="mk-field"><span>Fee</span><select name="fee"><option value="5">0.05%</option><option value="30" selected>0.30%</option><option value="100">1.00%</option></select></label>
    <button class="primary mk-act" type="submit">Open pool <span>→</span></button>
  </form>`;
  root.querySelectorAll('[data-trade]').forEach((b) => { b.onclick = () => { ui.pool = b.dataset.trade; view = 'swap'; render(); }; });
  root.querySelectorAll('[data-add]').forEach((b) => { b.onclick = () => { const f = $(`#add-${b.dataset.add}`); f.hidden = !f.hidden; }; });
  root.querySelectorAll('[data-remove]').forEach((b) => { b.onclick = () => { const f = $(`#remove-${b.dataset.remove}`); f.hidden = !f.hidden; }; });

  for (const p of state.pools) {
    const add = $(`#add-${p.id}`);
    const x = BigInt(p.reserve_a), y = BigInt(p.reserve_b);
    add.elements.a.oninput = () => {
      try {
        const a = parse(add.elements.a.value);
        add.elements.b.value = fmt((a * y + x - 1n) / x, 18).replace(/,/g, '');
        const pv = mk.liquidityPreview(p, a, parse(add.elements.b.value));
        add.querySelector('[data-preview]').textContent = `≈ ${ratio(pv.shares * 100n, BigInt(p.shares) + pv.shares, 3)}% of the pool. Anything the ratio does not use comes back under Owed to you.`;
      } catch { add.querySelector('[data-preview]').textContent = ''; }
    };
    add.onsubmit = async (e) => {
      e.preventDefault();
      try {
        const a = parse(add.elements.a.value), b = parse(add.elements.b.value);
        const pv = mk.liquidityPreview(p, a, b);
        const taken = new Set();
        const fa = mk.fund(identity, state.held, unhex(p.a), a, taken);
        const fb = mk.fund(identity, state.held, unhex(p.b), b, taken);
        await submit(mk.addLiquidityIntent({
          provider: identity.account, pool: unhex(p.id), amountA: a, amountB: b,
          minShares: mk.withSlippage(pv.shares, 100), inputs: [...fa.inputs, ...fb.inputs], change: [...fa.change, ...fb.change], nonce: nextNonce(),
        }), 'Liquidity added. Your shares are under Owed to you.');
      } catch (err) { notice(err.message, true); }
    };
    const rm = $(`#remove-${p.id}`);
    rm.onsubmit = async (e) => {
      e.preventDefault();
      try {
        const s = parse(rm.elements.s.value);
        const shares = BigInt(p.shares);
        const f = mk.fund(identity, state.held, unhex(p.lp), s);
        await submit(mk.removeLiquidityIntent({
          provider: identity.account, pool: unhex(p.id), shares: s,
          minA: mk.withSlippage((s * x) / shares, 100), minB: mk.withSlippage((s * y) / shares, 100),
          inputs: f.inputs, change: f.change, nonce: nextNonce(),
        }), 'Liquidity removed. Both sides are under Owed to you.');
      } catch (err) { notice(err.message, true); }
    };
  }

  $('#create-pool').onsubmit = async (e) => {
    e.preventDefault();
    const f = e.target.elements;
    try {
      let a = f.a.value, b = f.b.value;
      let amountA = parse(f.amount_a.value), amountB = parse(f.amount_b.value);
      if (a === b) throw new Error('A pool needs two different assets');
      // The VM names a pool's assets in byte order.
      if (mk.compareBytes(unhex(a), unhex(b)) > 0) { [a, b] = [b, a]; [amountA, amountB] = [amountB, amountA]; }
      const taken = new Set();
      const fa = mk.fund(identity, state.held, unhex(a), amountA, taken);
      const fb = mk.fund(identity, state.held, unhex(b), amountB, taken);
      await submit(mk.createPoolIntent({
        provider: identity.account, assetA: unhex(a), assetB: unhex(b), amountA, amountB, feeBps: Number(f.fee.value),
        inputs: [...fa.inputs, ...fb.inputs], change: [...fa.change, ...fb.change], nonce: nextNonce(),
      }), 'Pool open. Your shares are under Owed to you.');
    } catch (err) { notice(err.message, true); }
  };
}

// ---------------------------------------------------------------------------
// Launchpad.
// ---------------------------------------------------------------------------

function renderLaunchpad() {
  const order = { open: 0, graduated: 1, failed: 2 };
  const launches = [...state.launches].sort((p, q) => order[p.state] - order[q.state] || q.closes_at_epoch - p.closes_at_epoch);
  $('#view').innerHTML = `${heading('LAUNCHPAD', 'Everyone pays<br><em>the same price.</em>')}
  <section class="mk-list">${launches.length ? launches.map(launchCard).join('') : '<div class="mk-empty"><p>No sales yet. Start the first one below.</p></div>'}</section>
  ${createLaunchForm()}`;
  for (const l of launches) {
    const form = $(`#commit-${l.token}`);
    if (!form) continue;
    form.onsubmit = async (e) => {
      e.preventDefault();
      try {
        const amount = parse(form.elements.amount.value);
        const f = mk.fund(identity, state.held, unhex(l.quote), amount);
        await submit(mk.commitIntent({ buyer: identity.account, launch: unhex(l.token), amount, inputs: f.inputs, change: f.change, nonce: nextNonce() }),
          `Committed to ${l.symbol || 'the sale'}. At the close your tokens — and any refund — arrive under Owed to you.`);
      } catch (err) { notice(err.message, true); }
    };
  }
  bindCreateLaunch();
}

function launchCard(l) {
  const raised = BigInt(l.raised), soft = BigInt(l.soft_cap), hard = BigInt(l.hard_cap), sale = BigInt(l.sale_supply);
  const left = BigInt(l.closes_at_epoch) - state.epoch;
  const title = l.symbol ? `${esc(l.symbol)} <span>${esc(l.name)}</span>` : `<span>${short(l.token)}</span>`;
  const accepted = raised < hard ? raised : hard;
  const price = sale > 0n && accepted > 0n ? ratio(accepted, sale, 8) : '—';
  const pct = (v) => (hard > 0n ? Number((v * 1000n) / hard) / 10 : 0);
  const status = l.state === 'open'
    ? (left >= 0n ? `closes after epoch ${l.closes_at_epoch} · ${span(left + 1n) || `${left} to go`}` : 'closing at the next seal')
    : l.state === 'graduated' ? 'sold · trading in its pool' : 'missed its soft cap · refunded';
  return `<article class="mk-card mk-launch mk-${l.state}">
    <header><h3>${title}</h3><span class="mk-pill">${l.state}</span></header>
    <div class="mk-raise">
      <div class="mk-raise-top"><b>${fmt(raised, 4)}</b><span>of ${fmt(hard, 4)} ${esc(name(l.quote))}</span></div>
      <meter min="0" max="100" low="${pct(soft)}" optimum="100" value="${Math.min(100, pct(raised))}"></meter>
      <div class="mk-raise-marks"><span>soft cap ${fmt(soft, 4)}</span><span>${status}</span></div>
    </div>
    <dl class="mk-facts">
      <dt>For sale</dt><dd>${fmt(sale, 0)} of ${fmt(BigInt(l.total_supply), 0)}</dd>
      <dt>${l.state === 'open' ? 'Price if it closed now' : l.state === 'graduated' ? 'Sold at' : 'Would have sold at'}</dt><dd>${price} ${esc(name(l.quote))}</dd>
      <dt>To the pool</dt><dd>${l.pool_bps / 100}% of the raise + ${fmt(BigInt(l.pool_supply), 0)} tokens, locked</dd>
    </dl>
    ${l.state === 'open' && left >= 0n ? `<form class="mk-inline" id="commit-${l.token}"><label class="mk-field"><span>Commit ${esc(name(l.quote))} · you hold ${fmt(balance(l.quote), 4)}</span><input name="amount" inputmode="decimal" autocomplete="off" placeholder="0.0"></label><button class="primary mk-act" type="submit">Commit <span>→</span></button><p class="mk-fine">Held until the close, then everyone pays one price. Over the hard cap, each commitment is filled in the same proportion and the rest refunded.</p></form>` : ''}
  </article>`;
}

function createLaunchForm() {
  return `<form class="mk-card mk-create" id="create-launch">
    <h2>Start a sale</h2>
    <p class="mk-fine">The whole supply is issued into escrow. If the raise misses its soft cap everyone is refunded and the token is burned — yours too.</p>
    <div class="mk-two">
      <label class="mk-field"><span>Symbol</span><input name="symbol" maxlength="8" autocomplete="off" placeholder="MOLE"></label>
      <label class="mk-field"><span>Name</span><input name="name" maxlength="24" autocomplete="off" placeholder="Nap Mole"></label>
      <label class="mk-field"><span>Total supply</span><input name="total" inputmode="decimal" value="1000000"></label>
      <label class="mk-field"><span>For sale (%)</span><input name="sale" inputmode="numeric" value="60"></label>
      <label class="mk-field"><span>Soft cap (ZEC.zy)</span><input name="soft" inputmode="decimal" value="10"></label>
      <label class="mk-field"><span>Hard cap (ZEC.zy)</span><input name="hard" inputmode="decimal" value="100"></label>
      <label class="mk-field"><span>Tokens to the pool (%)</span><input name="pool" inputmode="numeric" value="20"></label>
      <label class="mk-field"><span>Raise to the pool (%)</span><input name="pool_raise" inputmode="numeric" value="50"></label>
      <label class="mk-field"><span>Open for (epochs)</span><input name="epochs" inputmode="numeric" value="60"></label>
      <label class="mk-field"><span>Pool fee</span><select name="fee"><option value="30" selected>0.30%</option><option value="100">1.00%</option></select></label>
    </div>
    <p class="mk-fine" id="launch-preview"></p>
    <button class="primary mk-act" type="submit">Start sale <span>→</span></button>
  </form>`;
}

function bindCreateLaunch() {
  const form = $('#create-launch');
  const f = form.elements;
  const pctOf = (total, p) => (total * BigInt(p)) / 100n;
  const preview = () => {
    try {
      const total = parse(f.total.value);
      const sale = pctOf(total, f.sale.value), pool = pctOf(total, f.pool.value);
      const mine = total - sale - pool;
      $('#launch-preview').textContent = mine < 0n
        ? 'The sale and the pool are more than the supply.'
        : `Open ${span(f.epochs.value) || `${f.epochs.value} epochs`}. You keep ${fmt(mine, 0)} tokens. At the hard cap they sell at ${ratio(parse(f.hard.value), sale, 8)} ZEC.zy each; the pool opens at ${ratio(pctOf(parse(f.hard.value), f.pool_raise.value), pool || 1n, 8)}.`;
    } catch { $('#launch-preview').textContent = ''; }
  };
  form.oninput = preview;
  preview();
  form.onsubmit = async (e) => {
    e.preventDefault();
    try {
      const total = parse(f.total.value);
      const salePct = Number(f.sale.value), poolPct = Number(f.pool.value), raisePct = Number(f.pool_raise.value);
      for (const v of [salePct, poolPct, raisePct]) if (!Number.isInteger(v) || v < 0 || v > 100) throw new Error('Percentages are whole numbers from 0 to 100');
      if (salePct === 0) throw new Error('A sale sells something');
      if ((poolPct === 0) !== (raisePct === 0)) throw new Error('A pool needs both tokens and part of the raise, or neither');
      const epochs = Number(f.epochs.value);
      if (!Number.isInteger(epochs) || epochs < 1 || epochs > 100000) throw new Error('Open for 1 to 100,000 epochs');
      await submit(mk.createLaunchIntent({
        creator: identity.account, nonce: nextNonce(), quote: unhex(config.zecAsset),
        totalSupply: total, saleSupply: pctOf(total, salePct), poolSupply: pctOf(total, poolPct),
        softCap: parse(f.soft.value), hardCap: parse(f.hard.value), poolBps: raisePct * 100, feeBps: Number(f.fee.value),
        closesAtEpoch: state.epoch + BigInt(epochs), meta: mk.encodeMeta({ symbol: f.symbol.value, name: f.name.value }),
      }), `Sale open. It closes after epoch ${state.epoch + BigInt(epochs)}; your share arrives under Owed to you then.`);
    } catch (err) { notice(err.message, true); }
  };
}

// ---------------------------------------------------------------------------
// Owed.
// ---------------------------------------------------------------------------

function renderOwed() {
  const byAsset = new Map();
  for (const c of state.owed) byAsset.set(c.asset, (byAsset.get(c.asset) || 0n) + BigInt(c.amount));
  $('#view').innerHTML = `${heading('OWED TO YOU', 'Settled,<br><em>waiting for you.</em>')}
  <p class="mk-fine mk-lead">Swap proceeds, refunds, pool shares and sale allocations are held by the ledger until you collect them. Only this wallet can find them — they are listed by ids derived from your key, and nobody else can ask for them.</p>
  ${state.owed.length ? `<section class="coins">${[...byAsset].map(([a, v]) => `<div class="coin"><div><span>${esc(name(a))}</span><b>${fmt(v)}</b></div></div>`).join('')}</section>
  <button class="primary mk-act mk-collect" id="collect">Collect all ${state.owed.length} <span>→</span></button>`
    : '<div class="mk-empty"><p>Nothing owed right now. Orders settle at the end of the epoch they were placed in.</p></div>'}`;
  $('#collect')?.addEventListener('click', async () => {
    const all = [...state.owed];
    for (let i = 0; i < all.length; i += PER_CLAIM) {
      const batch = all.slice(i, i + PER_CLAIM);
      const ok = await submit(mk.claimIntent(mk.redeem(identity, batch)), `Collected ${Math.min(i + PER_CLAIM, all.length)} of ${all.length}. They are in your wallet now.`);
      if (!ok) break;
    }
  });
}

// Refresh buttons live inside each view.
root.addEventListener('click', (e) => { if (e.target.closest('#refresh')) refresh(); });

(async () => {
  try {
    config = await loadConfig();
    locked();
  } catch (e) {
    root.innerHTML = `<main class="fatal"><img src="/nap-mark.png" alt=""><h1>Market unavailable.</h1><p>${esc(e.message)}</p></main>`;
  }
})();
