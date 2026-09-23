// Cave Jade — the launchpad (DECISIONS §141).
//
// Reads come from the chain's public RPC: the curves, their prices and what
// they hold are public state, so anyone can walk the yard without a wallet.
// Everything that moves value goes through Nap (Nap Connect), which shows the
// holder what the chain says and asks them to approve it — in Nap's window,
// which this page cannot touch.

import { connect, discover, NAP_WEB } from './nap-connect.js';

const XZEC = '0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201';

/// The curve every token shares (objectvm::curve). Consensus, not settings.
const CURVE = Object.freeze({ lots: 80_000, lotTokens: 12_500n, baseZat: 7_721n, slopeZat: 1n, graduateAt: 51_098 });
const WAD = 10n ** 18n;
const ZAT = 10_000_000_000n;
const LOT = CURVE.lotTokens * WAD;
const BPS = 10_000n;

const params = new URLSearchParams(location.search);
const NETWORK = params.get('network') === 'testnet' ? 'testnet' : 'mainnet';
const RPC = NETWORK === 'testnet' ? 'https://zyn-rpc.cavebro.win/object' : 'https://zyn-rpc.cavebro.win/';
/// Only a page served from this machine may point Nap elsewhere (development).
const LOCAL = ['localhost', '127.0.0.1'].includes(location.hostname);
const NAP = (LOCAL && params.get('nap')) || NAP_WEB;

const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => [...document.querySelectorAll(selector)];
const esc = (value) => String(value).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const short = (value, n = 7) => value?.length > n * 2 ? `${value.slice(0, n)}…${value.slice(-n)}` : (value || '—');
const isHex = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
const isInt = (v) => typeof v === 'string' && /^\d{1,40}$/.test(v);
const compact = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 2 });

/// A WAD amount as a short decimal.
function wad(raw, places = 8) {
  const v = BigInt(raw);
  const frac = (v % WAD).toString().padStart(18, '0').slice(0, places).replace(/0+$/, '');
  return `${(v / WAD).toLocaleString('en-US')}${frac ? `.${frac}` : ''}`;
}
/// A per-token price, which is tiny: shown with its significant digits.
function perToken(raw) {
  const n = Number(BigInt(raw)) / 1e18;
  return n === 0 ? '0' : n < 1e-6 ? n.toExponential(3) : n.toLocaleString('en-US', { maximumSignificantDigits: 4 });
}
function parseDecimal(value) {
  const s = String(value || '').trim();
  if (!/^\d+(\.\d{1,18})?$/.test(s)) throw new Error('Enter a decimal amount');
  const [w, f = ''] = s.split('.');
  return BigInt(w) * WAD + BigInt(f.padEnd(18, '0'));
}

// The curve's own arithmetic, for previews. The ledger settles each epoch's
// orders together at one price, so these are what a lone order gets.
const cost = (q, d) => { const Q = BigInt(q), D = BigInt(d); return (CURVE.baseZat * D + CURVE.slopeZat * ((D * (2n * Q + D - 1n)) / 2n)) * ZAT; };
function lotsFor(q, budget, cap) {
  let lo = 0, hi = cap;
  while (lo < hi) { const mid = lo + Math.ceil((hi - lo) / 2); if (cost(q, mid) <= budget) lo = mid; else hi = mid - 1; }
  return lo;
}
function previewBuy(sold, amountIn, feeBps) {
  const budget = (amountIn * BPS) / (BPS + BigInt(feeBps));
  const lots = lotsFor(sold, budget, Math.max(0, CURVE.graduateAt - sold));
  const paid = cost(sold, lots);
  const fee = (paid * BigInt(feeBps)) / BPS;
  return { lots, tokens: BigInt(lots) * LOT, paid, fee };
}
function previewSell(sold, lots, feeBps) {
  const n = Math.min(lots, sold);
  const gross = n ? cost(sold - n, n) : 0n;
  const fee = (gross * BigInt(feeBps)) / BPS;
  return { gross, fee, net: gross - fee };
}
/// A lone swap at the batch price (objectvm::market::clear, one side).
function poolQuote(pool, sellA, amountIn) {
  const x = BigInt(sellA ? pool.reserve_a : pool.reserve_b);
  const y = BigInt(sellA ? pool.reserve_b : pool.reserve_a);
  const a = (amountIn * (BPS - BigInt(pool.fee_bps))) / BPS;
  return (a * y) / (x + 2n * a);
}

/// A token's name lives in its 32 bytes of meta: 8 for the ticker, 24 for the name.
function decodeMeta(hexMeta) {
  const b = Uint8Array.from(hexMeta.match(/../g).map((h) => parseInt(h, 16)));
  const text = (x) => { const end = x.indexOf(0); return new TextDecoder().decode(end === -1 ? x : x.subarray(0, end)); };
  const symbol = text(b.subarray(0, 8));
  const name = text(b.subarray(8));
  return /^[A-Z0-9]{1,8}$/.test(symbol) && name ? { symbol, name } : { symbol: '?', name: 'Unnamed stone' };
}

let curves = [];
let pools = [];
let terms = { set: false, fee_bps: 0, bond: '0' };
let wallet = null;
let currentStage = 'revealed';
let currentCoin = null;
let tradeSide = 'buy';

// ---------------------------------------------------------------------------
// The chain.

async function rpc(method) {
  const id = crypto.randomUUID();
  const r = await fetch(RPC, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id, method, params: [] }),
    credentials: 'omit',
    referrerPolicy: 'no-referrer',
  });
  const body = await r.json();
  if (body.id !== id || body.error) throw new Error(body.error?.message || 'The chain did not answer');
  return body.result;
}

const validCurve = (c) => isHex(c.token) && isHex(c.meta) && c.quote === XZEC && Number.isSafeInteger(c.sold) && c.sold >= 0
  && ['reserve', 'graduation', 'price_per_token'].every((k) => isInt(c[k])) && typeof c.graduated === 'boolean';

async function loadMarket() {
  try {
    const m = await rpc('zyn_market');
    terms = m.curve_terms || terms;
    pools = (m.pools || []).filter((p) => isHex(p.id) && isInt(p.reserve_a) && isInt(p.reserve_b));
    curves = (m.curves || []).filter(validCurve).map((c) => ({ ...c, ...decodeMeta(c.meta) }));
    $('#gallery-status').textContent = `Live, ${NETWORK}. A stone is its address; names and tickers may repeat.`;
  } catch (e) {
    $('#gallery-status').textContent = `The chain is not answering: ${e.message}`;
  }
  renderGallery();
  renderPreview();
  if (pathRoute().view === 'coin') renderCoin(pathRoute().asset);
}

const progress = (c) => Math.min(1, c.sold / CURVE.graduateAt);
const poolOf = (c) => pools.find((p) => p.id === c.pool);
/// ZEC.zy the stone is set in: the curve's reserve, or its pool's ZEC side.
function zecSet(c) {
  if (!c.graduated) return BigInt(c.reserve);
  const p = poolOf(c);
  return p ? BigInt(p.a === XZEC ? p.reserve_a : p.reserve_b) : 0n;
}
/// ZEC.zy per token: the next lot on the curve, or the pool's spot.
function price(c) {
  const p = c.graduated && poolOf(c);
  if (!p) return BigInt(c.price_per_token);
  const [z, t] = p.a === XZEC ? [p.reserve_a, p.reserve_b] : [p.reserve_b, p.reserve_a];
  return (BigInt(z) * WAD) / BigInt(t);
}

// ---------------------------------------------------------------------------
// The gallery.

const views = Object.fromEntries($$('[data-view]').map((view) => [view.dataset.view, view]));
const routeLinks = $$('[data-route]');
const gallery = $('#coin-gallery');
const search = $('#gallery-search');
const sort = $('#gallery-sort');
const stageButtons = $$('[data-filter]');
const walletButton = $('#wallet-button');

const stage = (c) => (c.graduated ? 'revealed' : 'trading');
const stoneMarkup = (c) => `<div class="card-stone" aria-hidden="true"><span>${esc([...c.symbol][0] || '?')}</span></div>`;
const coinCard = (c) => `
    <a class="coin-card" href="/coin/${c.token}" data-coin="${c.token}">
      <div class="coin-title">${stoneMarkup(c)}<div><p>${c.graduated ? 'REVEALED JADE' : 'WINDOW OPEN'}</p><h2>${esc(c.name)}</h2><span>${esc(c.symbol)} / ZEC.zy</span></div></div>
      <div class="coin-metric"><strong>${perToken(price(c))}</strong><span>ZEC.zy</span></div>
      <div class="coin-metric"><strong class="positive">${(progress(c) * 100).toFixed(2)}%</strong><span>${compact.format(c.sold)} / ${compact.format(CURVE.graduateAt)} lots</span></div>
      <div class="coin-metric"><strong>${wad(zecSet(c), 4)} ZEC.zy</strong><span>${c.graduated ? 'in its locked pool' : 'in its curve'}</span></div>
      <div class="coin-metric"><strong class="mono">${short(c.token)}</strong><span>${c.graduated ? 'pool trading' : 'curve trading'}</span></div>
    </a>`;

function renderGallery() {
  const query = search.value.trim().toLocaleLowerCase();
  const results = curves
    .filter((c) => currentStage === 'all' || stage(c) === currentStage)
    .filter((c) => !query || `${c.name} ${c.symbol} ${c.token}`.toLocaleLowerCase().includes(query))
    .sort((a, b) => sort.value === 'price' ? (price(b) > price(a) ? 1 : -1) : sort.value === 'name' ? a.name.localeCompare(b.name) : progress(b) - progress(a));
  gallery.innerHTML = results.length ? results.map(coinCard).join('') : `<div class="empty-gallery">${curves.length ? 'No stones match this view.' : 'The yard is empty. Pick the first stone.'}</div>`;
  const revealed = curves.filter((c) => c.graduated).length;
  $('#count-revealed').textContent = revealed;
  $('#count-trading').textContent = curves.length - revealed;
  $('#count-all').textContent = curves.length;
  $('#tally-revealed').textContent = revealed;
  $('#tally-trading').textContent = curves.length - revealed;
  $('#tally-zec').textContent = `${wad(curves.reduce((s, c) => s + zecSet(c), 0n), 4)} ZEC.zy`;
}

function pathRoute() {
  const path = location.pathname.replace(/\/+$/, '') || '/';
  if (path === '/launch') return { view: 'launch' };
  const match = path.match(/^\/coin\/([0-9a-f]{64})$/i);
  return match ? { view: 'coin', asset: match[1].toLowerCase() } : { view: 'gallery' };
}

function renderCoin(token) {
  const c = curves.find((x) => x.token === token);
  if (!c) {
    currentCoin = null;
    $('#coin-title').textContent = curves.length ? 'Stone not found' : 'Reading the yard…';
    return;
  }
  currentCoin = c;
  $('#coin-crumb').textContent = c.name;
  $('#coin-title').textContent = c.name;
  $('#coin-symbol').textContent = `${c.symbol} / ZEC.zy`;
  $('#coin-stage').textContent = c.graduated ? 'REVEALED JADE / LOCKED POOL' : 'WINDOW OPEN / BONDING CURVE';
  $('#coin-stone span').textContent = [...c.symbol][0] || '?';
  $('#coin-price').textContent = `${perToken(price(c))} ZEC.zy`;
  $('#coin-change').textContent = c.graduated ? 'pool spot' : `${(progress(c) * 100).toFixed(2)}% of the way to its pool`;
  $('#coin-liquidity').textContent = `${wad(zecSet(c), 4)} ZEC.zy`;
  $('#coin-pot').textContent = c.graduated ? 'in the pool' : `${wad(c.graduation, 6)} ZEC.zy`;
  $('#coin-block').textContent = short(c.token, 6);
  $('#coin-block').title = c.token;
  const cut = progress(c);
  $('#chart-point').setAttribute('cx', String(54 + 816 * cut));
  $('#chart-point').setAttribute('cy', String(342 - 278 * cut));
  $('#chart-progress').textContent = `${(cut * 100).toFixed(2)}% revealed`;
  $('#chart-title').textContent = c.graduated ? 'Curve complete. Its pool is live and locked.' : `${c.sold.toLocaleString('en-US')} of ${CURVE.graduateAt.toLocaleString('en-US')} lots found.`;
  document.title = `${c.name} — Cave Jade`;
  configureTrade();
}

function syncRoute({ scroll = true } = {}) {
  const route = pathRoute();
  Object.entries(views).forEach(([name, view]) => { view.hidden = name !== route.view; });
  routeLinks.forEach((link) => link.toggleAttribute('aria-current', link.dataset.route === route.view || (route.view === 'coin' && link.dataset.route === 'gallery')));
  if (route.view === 'coin') renderCoin(route.asset);
  else document.title = route.view === 'gallery' ? 'Cave Jade — Revealed Coins' : 'Pick a Stone — Cave Jade';
  document.body.dataset.route = route.view;
  if (scroll) window.scrollTo({ top: 0, behavior: 'instant' });
}

function navigate(path) {
  history.pushState({}, '', path + location.search);
  syncRoute();
}

document.addEventListener('click', (event) => {
  const link = event.target.closest('a[href^="/"]');
  if (!link || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  event.preventDefault();
  navigate(link.getAttribute('href'));
});
window.addEventListener('popstate', () => syncRoute());
stageButtons.forEach((button) => button.addEventListener('click', () => {
  currentStage = button.dataset.filter;
  stageButtons.forEach((item) => { item.classList.toggle('active', item === button); item.setAttribute('aria-selected', String(item === button)); });
  renderGallery();
}));
search.addEventListener('input', renderGallery);
sort.addEventListener('change', renderGallery);
document.addEventListener('keydown', (event) => {
  if (event.key === '/' && document.activeElement !== search && !/INPUT|TEXTAREA/.test(document.activeElement.tagName)) {
    event.preventDefault();
    search.focus();
  }
});

// ---------------------------------------------------------------------------
// The wallet.

async function pickWallet() {
  const dialog = $('#jade-wallet');
  dialog.showModal();
  const body = $('.jade-wallet-body', dialog);
  if (wallet) {
    body.innerHTML = `<p>Connected: <b>${esc(wallet.info.name)}</b></p><p class="mono">${esc(short(wallet.session.account, 8))}</p><button type="button" class="jade-wallet-off">Disconnect</button>`;
    $('.jade-wallet-off', dialog).onclick = () => { wallet = null; renderWallet(); dialog.close(); };
    return;
  }
  body.innerHTML = '<p>Looking for Nap…</p>';
  const list = await discover({ web: NAP });
  body.innerHTML = list.map((w, i) => `<button type="button" class="jade-wallet-option" data-i="${i}"><b>${esc(w.info.name)}</b><small>${esc(w.info.detail || '')}</small></button>`).join('')
    + '<p class="jade-wallet-status" role="status"></p><p class="jade-wallet-fine">Connecting shows this page your Zyn account — not your balances or keys. Every trade is approved in Nap.</p>';
  body.querySelectorAll('[data-i]').forEach((b) => {
    b.onclick = async () => {
      const status = $('.jade-wallet-status', dialog);
      status.textContent = list[b.dataset.i].provider.isNapWeb ? "Nap's window is open — unlock it there and approve." : 'Approve in Nap.';
      try {
        wallet = await connect(list[b.dataset.i], { network: NETWORK });
        wallet.provider.on?.('disconnect', () => { wallet = null; renderWallet(); });
        dialog.close();
        renderWallet();
      } catch (e) { status.textContent = e.code === 4001 ? 'Not connected. Nothing was shared.' : e.message; }
    };
  });
}

function renderWallet() {
  walletButton.innerHTML = wallet ? `<span></span> ${esc(short(wallet.session.account, 5))}` : '<span></span> Connect wallet';
  renderPreview();
  configureTrade();
}

/// Ask the connected Nap. Called from clicks: Nap in the browser may need to
/// open its window, and only a click may.
function ask(method, p) {
  if (!wallet) { pickWallet(); return Promise.reject(new Error('Connect a wallet first.')); }
  return wallet.provider.request({ method, params: p });
}

walletButton.addEventListener('click', pickWallet);

// ---------------------------------------------------------------------------
// Placing a stone.

const form = $('#stone-form');
const nameInput = $('#stone-name');
const symbolInput = $('#stone-symbol');
const launchButton = $('#launch-stone');
const validSymbol = (v) => /^[A-Z0-9]{1,8}$/.test(v);
const validName = (v) => v.length >= 1 && v.length <= 24 && v === v.trim() && /^[\x20-\x7e]+$/.test(v);

function renderPreview() {
  const symbol = symbolInput.value.trim().toUpperCase() || 'STONE';
  const ok = validSymbol(symbol) && validName(nameInput.value);
  $('#preview-name').textContent = nameInput.value.trim() || 'Unnamed stone';
  $('#preview-symbol').textContent = symbol;
  $('#preview-letter').textContent = symbol[0] || '?';
  symbolInput.setAttribute('aria-invalid', String(!validSymbol(symbol)));
  nameInput.setAttribute('aria-invalid', String(!validName(nameInput.value)));
  launchButton.disabled = !ok;
  const bond = BigInt(terms.bond || '0');
  $('#stone-terms').textContent = terms.set
    ? `${terms.fee_bps / 100}% on every curve trade — half to the ZYN pot, 40% to this stone's pool, 10% to you when it graduates. ${bond ? `Bond ${wad(bond)} ZEC.zy, returned at graduation.` : 'No bond.'}`
    : 'No fee and no bond are set on this network yet.';
  $('#stone-status').textContent = !ok
    ? 'A ticker is 1–8 letters or digits; a name is 1–24 plain characters.'
    : wallet ? 'Ready: Nap will show you the terms before anything is placed.' : 'Connect a wallet to place it.';
}

form.querySelectorAll('input').forEach((input) => input.addEventListener('input', renderPreview));
form.addEventListener('submit', async (event) => {
  event.preventDefault();
  const symbol = symbolInput.value.trim().toUpperCase();
  if (!validSymbol(symbol) || !validName(nameInput.value)) return renderPreview();
  launchButton.disabled = true;
  try {
    const r = await ask('zyn_curveCreate', { symbol, name: nameInput.value.trim() });
    $('#stone-status').textContent = 'Placed. It opens for trading when this epoch seals.';
    setTimeout(async () => { await loadMarket(); if (isHex(r?.token)) navigate(`/coin/${r.token}`); }, 3000);
  } catch (e) {
    $('#stone-status').textContent = e.message;
  } finally {
    launchButton.disabled = false;
  }
});

// ---------------------------------------------------------------------------
// Trading.

function configureTrade() {
  const c = currentCoin;
  if (!c) return;
  const buying = tradeSide === 'buy';
  $('#trade-label').textContent = buying ? 'You put in' : 'Tokens to return';
  $('#trade-unit').textContent = buying ? 'ZEC.zy' : c.symbol;
  $('#trade-input').step = buying ? '0.001' : String(CURVE.lotTokens);
  $('#trade-route').textContent = c.graduated ? 'Locked pool' : 'Bonding curve';
  $('.trade-submit').textContent = wallet ? (buying ? 'Pick this jade' : 'Return this jade') : 'Connect wallet';
  $('.trade-submit').disabled = false;
  quote();
}

function quote() {
  const c = currentCoin;
  const out = $('#trade-output'), fee = $('#trade-fee'), min = $('#trade-min');
  out.textContent = fee.textContent = min.textContent = '—';
  if (!c) return null;
  try {
    const value = parseDecimal($('#trade-input').value);
    if (c.graduated) {
      const p = poolOf(c);
      if (!p) throw new Error('Its pool is not visible yet');
      const sellA = tradeSide === 'buy' ? p.a === XZEC : p.a === c.token;
      const q = poolQuote(p, sellA, value);
      out.textContent = `${wad(q, 6)} ${tradeSide === 'buy' ? c.symbol : 'ZEC.zy'}`;
      fee.textContent = `${p.fee_bps / 100}% kept by the pool`;
      min.textContent = '1% below, or it comes back';
      return { kind: 'swap', pool: p.id, sellA, amount: value };
    }
    if (tradeSide === 'buy') {
      if (value < 100_000n * ZAT) throw new Error('A buy is at least 0.001 ZEC.zy');
      const q = previewBuy(c.sold, value, c.fee_bps);
      out.textContent = `${Number(q.tokens / WAD).toLocaleString('en-US')} ${c.symbol}`;
      fee.textContent = c.fee_bps ? `${wad(q.fee, 8)} ZEC.zy` : 'none';
      min.textContent = 'shared price this epoch';
      return { kind: 'buy', amount: value };
    }
    if (value % LOT !== 0n) throw new Error(`Return whole lots of ${CURVE.lotTokens.toLocaleString('en-US')}`);
    const q = previewSell(c.sold, Number(value / LOT), c.fee_bps);
    out.textContent = `${wad(q.net, 8)} ZEC.zy`;
    fee.textContent = c.fee_bps ? `${wad(q.fee, 8)} ZEC.zy` : 'none';
    min.textContent = 'shared price this epoch';
    return { kind: 'sell', tokens: value };
  } catch (e) {
    $('#trade-status').textContent = e.message;
    return null;
  }
}

$$('[data-trade]').forEach((button) => button.addEventListener('click', () => {
  tradeSide = button.dataset.trade;
  $$('[data-trade]').forEach((item) => item.classList.toggle('active', item === button));
  $('#trade-input').value = tradeSide === 'buy' ? '0.1' : String(CURVE.lotTokens * 10n);
  configureTrade();
}));
$('#trade-input').addEventListener('input', () => { $('#trade-status').textContent = ''; quote(); });

const dec = (raw) => { const v = BigInt(raw); const f = (v % WAD).toString().padStart(18, '0').replace(/0+$/, ''); return `${v / WAD}${f ? `.${f}` : ''}`; };

$('.trade-submit').addEventListener('click', async () => {
  const c = currentCoin;
  if (!c) return;
  if (!wallet) return pickWallet();
  const q = quote();
  if (!q) return;
  const button = $('.trade-submit');
  button.disabled = true;
  try {
    // Asked before anything is awaited: Nap may need to open its window.
    const pending = q.kind === 'swap' ? ask('zyn_swap', { pool: q.pool, sellA: q.sellA, amount: dec(q.amount) })
      : q.kind === 'buy' ? ask('zyn_curveBuy', { token: c.token, amount: dec(q.amount) })
        : ask('zyn_curveSell', { token: c.token, tokens: dec(q.tokens) });
    await pending;
    $('#trade-status').textContent = 'Placed. It settles when this epoch seals — then collect it in Nap (or press Collect).';
    $('#trade-collect').hidden = false;
  } catch (e) {
    $('#trade-status').textContent = e.message;
  } finally {
    button.disabled = false;
  }
});

$('#trade-collect').addEventListener('click', async () => {
  try {
    await ask('zyn_collect', {});
    $('#trade-status').textContent = 'Collected. It is in your Nap wallet.';
    loadMarket();
  } catch (e) { $('#trade-status').textContent = e.message; }
});

// ---------------------------------------------------------------------------

renderGallery();
renderPreview();
syncRoute({ scroll: false });
loadMarket();
setInterval(() => { if (!document.hidden) loadMarket(); }, 30_000);
document.documentElement.dataset.jadeReady = 'true';
