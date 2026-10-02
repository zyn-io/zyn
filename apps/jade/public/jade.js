// Jade — Zyn's launchpad (DECISIONS §141).
//
// Reads come from the chain's public RPC: curves, prices, per-epoch fills —
// anyone can browse without a wallet. Pictures and descriptions come
// from this site's own store, attached by each coin's creator with a proof.
// Everything that moves value goes through Nap (Nap Connect), which shows the
// holder what the chain says and asks them to approve it in Nap's window.

import { connect, discover, NAP_WEB } from './nap-connect.js';
import { createChart } from './vendor/lightweight-charts.js';
import { installTips } from './tip.js';

const XZEC = '0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201';

/// The curve every token shares (objectvm::curve). Consensus, not settings.
const CURVE = Object.freeze({ lots: 80_000, lotTokens: 12_500n, baseZat: 7_721n, slopeZat: 1n, graduateAt: 51_098 });
const WAD = 10n ** 18n;
const ZAT = 10_000_000_000n;
const LOT = CURVE.lotTokens * WAD;
const BPS = 10_000n;
const SUPPLY = 1_000_000_000n;

const params = new URLSearchParams(location.search);
const NETWORK = params.get('network') === 'testnet' ? 'testnet' : 'mainnet';
/// Only a page served from this machine may point elsewhere (development).
const LOCAL = ['localhost', '127.0.0.1'].includes(location.hostname);
const RPC = (LOCAL && params.get('rpc')) || (NETWORK === 'testnet' ? 'https://zyn-rpc.cavebro.win/object' : 'https://zyn-rpc.cavebro.win/');
const NAP = (LOCAL && params.get('nap')) || NAP_WEB;

const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => [...document.querySelectorAll(selector)];
const esc = (value) => String(value).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const short = (value, n = 6) => value?.length > n * 2 ? `${value.slice(0, n)}…${value.slice(-n)}` : (value || '—');
const isHex = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
const isInt = (v) => typeof v === 'string' && /^\d{1,40}$/.test(v);
const now = () => Math.floor(Date.now() / 1000);

// ---------------------------------------------------------------------------
// Numbers.

const num = (raw) => Number(BigInt(raw)) / 1e18;
function wad(raw, places = 6) {
  const v = BigInt(raw);
  const neg = v < 0n;
  const a = neg ? -v : v;
  const frac = (a % WAD).toString().padStart(18, '0').slice(0, places).replace(/0+$/, '');
  return `${neg ? '-' : ''}${(a / WAD).toLocaleString('en-US')}${frac ? `.${frac}` : ''}`;
}
const decimal = (raw) => { const v = BigInt(raw); const f = (v % WAD).toString().padStart(18, '0').replace(/0+$/, ''); return `${v / WAD}${f ? `.${f}` : ''}`; };
function parseDecimal(value) {
  // Many phone keypads offer a comma: read it as the decimal point.
  const s = String(value || '').trim().replace(',', '.');
  if (!/^\d+(\.\d{1,18})?$/.test(s)) throw new Error('Enter an amount');
  const [w, f = ''] = s.split('.');
  return BigInt(w) * WAD + BigInt(f.padEnd(18, '0'));
}
/// An amount in full, with thousands marks: what is shown is what is signed.
const full = (raw) => { const v = BigInt(raw); const f = (v % WAD).toString().padStart(18, '0').replace(/0+$/, ''); return `${(v / WAD).toLocaleString('en-US')}${f ? `.${f}` : ''}`; };
/// The page's own links keep the network, so a coin opened in a new tab is
/// the same coin on the same chain.
const withNet = (path) => (NETWORK === 'testnet' ? `${path}?network=testnet` : path);
const zecText = (zec) => (zec >= 1000 ? zec.toLocaleString('en-US', { maximumFractionDigits: 0 }) : zec >= 1 ? zec.toLocaleString('en-US', { maximumFractionDigits: 2 }) : zec.toLocaleString('en-US', { maximumSignificantDigits: 3 }));
const usdText = (usd) => (usd >= 1e6 ? `$${(usd / 1e6).toFixed(2)}M` : usd >= 1e3 ? `$${(usd / 1e3).toFixed(1)}K` : `$${usd.toFixed(2)}`);
function ago(t) {
  if (!t) return '—';
  const s = Math.max(0, now() - t);
  return s < 60 ? `${s}s ago` : s < 3600 ? `${Math.floor(s / 60)}m ago` : s < 86400 ? `${Math.floor(s / 3600)}h ago` : `${Math.floor(s / 86400)}d ago`;
}
const pct = (a, b) => (b > 0 ? ((a - b) / b) * 100 : null);
const changeText = (p) => (p == null ? '—' : `${p >= 0 ? '+' : ''}${p.toFixed(p >= 100 || p <= -100 ? 0 : 1)}%`);

// The curve's own arithmetic, for previews. Each epoch's orders settle
// together at one price, so these are what an order gets alone.
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
  return { lots, tokens: BigInt(lots) * LOT, paid, fee: (paid * BigInt(feeBps)) / BPS };
}
function previewSell(sold, lots, feeBps) {
  const n = Math.min(lots, sold);
  const gross = n ? cost(sold - n, n) : 0n;
  const fee = (gross * BigInt(feeBps)) / BPS;
  return { gross, fee, net: gross - fee };
}
function poolQuote(pool, sellA, amountIn) {
  const x = BigInt(sellA ? pool.reserve_a : pool.reserve_b);
  const y = BigInt(sellA ? pool.reserve_b : pool.reserve_a);
  const a = (amountIn * (BPS - BigInt(pool.fee_bps))) / BPS;
  return (a * y) / (x + 2n * a);
}

/// A token's name lives in its 32 bytes of meta: 8 for the ticker, 24 for the name.
/// The same rules as Nap's (market.js): printable ASCII, zero padding all the
/// way, so the name shown here is the name Nap's approval shows.
function decodeMeta(hexMeta) {
  const b = Uint8Array.from(hexMeta.match(/../g).map((h) => parseInt(h, 16)));
  const text = (x) => { const end = x.indexOf(0); return new TextDecoder().decode(end === -1 ? x : x.subarray(0, end)); };
  const symbol = text(b.subarray(0, 8));
  const name = text(b.subarray(8));
  const zeros = (from, to) => b.subarray(from, to).every((x) => x === 0);
  const good = /^[A-Z0-9]{1,8}$/.test(symbol) && name && /^[\x20-\x7e]+$/.test(name) && zeros(symbol.length, 8) && zeros(8 + name.length, 32);
  return good ? { symbol, name } : { symbol: '?', name: 'Unnamed coin' };
}

// ---------------------------------------------------------------------------
// State.

let curves = [];
let pools = [];
let stats = new Map();
let terms = { set: false, fee_bps: 0, bond: '0' };
let usd = null;
let wallet = null;
let mine = null; // { zec, holdings: Map token -> amount, created: [] }
let sortBy = 'movers';
let currentCoin = null;
let tradeSide = 'buy';
let unit = 'zec';
let tf = 300;
/// Each coin's epoch points, by token, so one coin's chart never shows under another.
const histories = new Map();
const historyOf = (token) => histories.get(token) || [];
/// The last order's status, per coin: written by the order, kept across the
/// page's refreshes, shown only on its own coin's page.
const orderStatus = new Map();
function setOrder(token, text) {
  orderStatus.set(token, text);
  if (currentCoin?.token === token) $('#trade-status').textContent = text;
}
let creating = false;
const content = new Map(); // token -> {description, image} | null (none) | 'loading'

async function rpc(method, fields = {}) {
  const id = crypto.randomUUID();
  let r;
  try {
    r = await fetch(RPC, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id, method, params: [], ...fields }),
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
      signal: AbortSignal.timeout(15000),
    });
  } catch { throw new Error('The chain could not be reached. Check your connection.'); }
  if (!r.ok) throw new Error(`The chain did not answer (HTTP ${r.status}). Try again shortly.`);
  const body = await r.json().catch(() => null);
  if (!body) throw new Error('The chain sent something unreadable.');
  if (body.error) throw new Error(body.error.message || 'The chain did not answer');
  return body.result;
}

const validCurve = (c) => isHex(c.token) && isHex(c.meta) && c.quote === XZEC && Number.isSafeInteger(c.sold) && c.sold >= 0
  && ['reserve', 'graduation', 'price_per_token'].every((k) => isInt(c[k])) && typeof c.graduated === 'boolean';

async function loadMarket() {
  try {
    const [m, s] = await Promise.all([rpc('zyn_market'), rpc('zyn_curveStats').catch(() => [])]);
    terms = m.curve_terms || terms;
    pools = (m.pools || []).filter((p) => isHex(p.id) && isInt(p.reserve_a) && isInt(p.reserve_b));
    curves = (m.curves || []).filter(validCurve).map((c) => ({ ...c, ...decodeMeta(c.meta) }));
    stats = new Map((Array.isArray(s) ? s : []).filter((x) => isHex(x.token)).map((x) => [x.token, x]));
    $('#gallery-status').textContent = curves.length ? `Live, ${NETWORK}. A coin is its address; names and tickers may repeat.` : '';
  } catch (e) {
    $('#gallery-status').textContent = `The chain is not answering: ${e.message}`;
  }
  render();
}

async function loadUsd() {
  try {
    const r = await fetch('/api/zec-usd', { credentials: 'omit' });
    const v = Number((await r.json())?.usd);
    usd = Number.isFinite(v) && v > 0 ? v : null;
  } catch { usd = null; }
  $('#zec-usd').textContent = usd ? `ZEC $${usd.toLocaleString('en-US', { maximumFractionDigits: 2 })}` : 'ZEC';
  $$('[data-unit="usd"]').forEach((b) => { b.hidden = !usd; });
  render();
}

const poolOf = (c) => pools.find((p) => p.id === c.pool);
/// ZEC.zy per token: the next lot on the curve, or the pool's spot.
function price(c) {
  const p = c.graduated && poolOf(c);
  if (!p) return BigInt(c.price_per_token);
  const [z, t] = p.a === XZEC ? [p.reserve_a, p.reserve_b] : [p.reserve_b, p.reserve_a];
  return (BigInt(z) * WAD) / BigInt(t);
}
/// Market cap in ZEC: price per token times the whole supply.
const mcap = (c) => num(price(c) * SUPPLY);
const mcapOf = (priceRaw) => num(BigInt(priceRaw) * SUPPLY);
function zecSet(c) {
  if (!c.graduated) return BigInt(c.reserve);
  const p = poolOf(c);
  return p ? BigInt(p.a === XZEC ? p.reserve_a : p.reserve_b) : 0n;
}
const progress = (c) => (c.graduated ? 1 : Math.min(1, c.sold / CURVE.graduateAt));
const change24 = (c) => { const s = stats.get(c.token); return s?.price_24h ? pct(Number(price(c)), Number(s.price_24h)) : null; };
const change1h = (c) => { const s = stats.get(c.token); return s?.price_1h ? pct(Number(price(c)), Number(s.price_1h)) : null; };
const capText = (zec) => (unit === 'usd' && usd ? usdText(zec * usd) : `${zecText(zec)} ZEC`);
const capBoth = (zec) => `${zecText(zec)} ZEC${usd ? ` · ${usdText(zec * usd)}` : ''}`;

// ---------------------------------------------------------------------------
// Pictures and descriptions.

async function loadContent(token) {
  if (content.has(token)) return content.get(token);
  content.set(token, 'loading');
  try {
    const r = await fetch(`/api/content/${token}`, { credentials: 'omit' });
    const v = r.ok ? await r.json() : null;
    const image = typeof v?.image === 'string' && /^data:image\/(png|jpeg|webp|gif);base64,[A-Za-z0-9+/=]+$/.test(v.image) ? v.image : '';
    content.set(token, v ? { description: String(v.description || ''), image } : null);
  } catch { content.set(token, null); }
  return content.get(token);
}
/// What goes inside a coin's square: its picture, or its ticker's first letter.
function thumbInner(c) {
  const k = content.get(c.token);
  return k && k !== 'loading' && k.image ? `<img src="${esc(k.image)}" alt="" />` : `<span>${esc([...c.symbol][0] || '?')}</span>`;
}
const thumb = (c) => `<div class="j-thumb">${thumbInner(c)}</div>`;

// ---------------------------------------------------------------------------
// Coins.

const views = Object.fromEntries($$('[data-view]').map((view) => [view.dataset.view, view]));
const search = $('#gallery-search');

function sorted() {
  const q = search.value.trim().toLocaleLowerCase();
  let list = curves.filter((c) => !q || `${c.name} ${c.symbol} ${c.token}`.toLocaleLowerCase().includes(q));
  const st = (c) => stats.get(c.token) || {};
  if (sortBy === 'near') list = list.filter((c) => !c.graduated);
  if (sortBy === 'grad') list = list.filter((c) => c.graduated);
  const by = {
    movers: (a, b) => Math.abs(change1h(b) ?? change24(b) ?? 0) - Math.abs(change1h(a) ?? change24(a) ?? 0) || (st(b).last_trade_time || 0) - (st(a).last_trade_time || 0),
    new: (a, b) => (st(b).created_time || b.created_at) - (st(a).created_time || a.created_at),
    mcap: (a, b) => mcap(b) - mcap(a),
    last: (a, b) => (st(b).last_trade_time || 0) - (st(a).last_trade_time || 0),
    near: (a, b) => progress(b) - progress(a),
    grad: (a, b) => mcap(b) - mcap(a),
  }[sortBy];
  return list.sort(by);
}

const coinCard = (c) => {
  const s = stats.get(c.token) || {};
  const ch = change24(c);
  const k = content.get(c.token);
  return `<a class="j-card" href="${withNet(`/coin/${c.token}`)}" data-coin="${c.token}">
      ${thumb(c)}
      <div class="j-card-body">
        <h3>${esc(c.name)} <span class="j-ticker">${esc(c.symbol)}</span></h3>
        ${k && k !== 'loading' && k.description ? `<p class="j-desc">${esc(k.description)}</p>` : ''}
        <p class="j-card-cap"><b>${esc(capText(mcap(c)))}</b> <span class="${ch == null ? '' : ch >= 0 ? 'up' : 'down'}">${changeText(ch)}</span></p>
        <div class="j-bar"><i style="width:${(progress(c) * 100).toFixed(1)}%"></i></div>
        <p class="j-card-meta"><span>${c.graduated ? 'graduated · in its pool' : `${(progress(c) * 100).toFixed(1)}% bonded`}</span><span>${s.created_time ? `made ${ago(s.created_time)}` : ''}${s.last_trade_time ? ` · traded ${ago(s.last_trade_time)}` : ''}</span></p>
      </div>
    </a>`;
};

/// The coin nearest graduation, with what is left to raise.
function renderSpotlight() {
  const box = $('#spotlight');
  const open = curves.filter((c) => !c.graduated && c.sold > 0).sort((a, b) => progress(b) - progress(a));
  const c = open[0];
  if (!c || search.value.trim()) { box.hidden = true; return; }
  const left = cost(c.sold, CURVE.graduateAt - c.sold);
  const k = content.get(c.token);
  box.hidden = false;
  box.innerHTML = `<a class="j-spot" href="${withNet(`/coin/${c.token}`)}">
      <div class="j-thumb j-thumb-lg">${thumbInner(c)}</div>
      <div class="j-spot-body">
        <p class="eyebrow">Closest to graduating</p>
        <h2>${esc(c.name)} <span class="j-ticker">${esc(c.symbol)}</span></h2>
        ${k && k !== 'loading' && k.description ? `<p class="j-desc">${esc(k.description)}</p>` : ''}
      </div>
      <div class="j-spot-figs">
        <div><span>Market cap</span><b>${esc(capBoth(mcap(c)))}</b></div>
        <div><span>To graduate</span><b>~${wad(left, 2)} ZEC</b></div>
      </div>
      <div class="j-spot-bar"><div class="j-bar j-bar-lg"><i style="width:${(progress(c) * 100).toFixed(1)}%"></i></div><span>${(progress(c) * 100).toFixed(1)}% bonded</span></div>
    </a>`;
}

function renderGallery() {
  renderSpotlight();
  const list = sorted();
  $('#coin-gallery').innerHTML = list.length ? list.map(coinCard).join('') : `<div class="j-empty">${curves.length ? 'No coins match.' : 'No coins yet. <a href="/launch">Create the first one.</a>'}</div>`;
  $('#tally-all').textContent = curves.length;
  $('#tally-grad').textContent = curves.filter((c) => c.graduated).length;
  $('#tally-zec').textContent = `${wad(curves.reduce((s, c) => s + zecSet(c), 0n), 4)} ZEC`;
  // Pictures for what is on screen, a few at a time.
  for (const c of list.slice(0, 48)) {
    if (!content.has(c.token)) loadContent(c.token).then((k) => { if (k) renderGallery(); });
  }
}

$$('[data-sort]').forEach((b) => b.addEventListener('click', () => {
  sortBy = b.dataset.sort;
  $$('[data-sort]').forEach((x) => { x.classList.toggle('active', x === b); x.setAttribute('aria-selected', String(x === b)); });
  renderGallery();
}));
search.addEventListener('input', renderGallery);
document.addEventListener('keydown', (event) => {
  if (event.key === '/' && document.activeElement !== search && !/INPUT|TEXTAREA/.test(document.activeElement.tagName)) {
    event.preventDefault();
    search.focus();
  }
});

// ---------------------------------------------------------------------------
// One coin.

let chart = null;
let candles = null;
let volume = null;

function ensureChart() {
  if (chart) return;
  const el = $('#chart');
  chart = createChart(el, {
    autoSize: true,
    layout: { background: { color: 'transparent' }, textColor: '#8a9691', fontFamily: 'Geist Mono, ui-monospace, monospace' },
    grid: { vertLines: { color: 'rgba(242,241,235,.04)' }, horzLines: { color: 'rgba(242,241,235,.05)' } },
    timeScale: { timeVisible: true, secondsVisible: false, borderColor: 'rgba(242,241,235,.09)' },
    rightPriceScale: { borderColor: 'rgba(242,241,235,.09)' },
    crosshair: { mode: 0 },
  });
  candles = chart.addCandlestickSeries({ upColor: '#35e6ae', downColor: '#e8794f', borderVisible: false, wickUpColor: '#35e6ae', wickDownColor: '#e8794f', priceFormat: { type: 'custom', formatter: (v) => capText(v / (unit === 'usd' && usd ? usd : 1)) , minMove: 1e-9 },
    // A quiet coin moves a hair: show at least ±2% around it, not a hair blown up to the full height.
    autoscaleInfoProvider: (base) => {
      const a = base();
      if (!a?.priceRange) return a;
      const { minValue: lo, maxValue: hi } = a.priceRange;
      const mid = (lo + hi) / 2;
      const half = Math.max((hi - lo) / 2, Math.abs(mid) * 0.02);
      return { ...a, priceRange: { minValue: mid - half, maxValue: mid + half } };
    } });
  volume = chart.addHistogramSeries({ priceScaleId: '', priceFormat: { type: 'volume' }, color: 'rgba(53,230,174,.18)', lastValueVisible: false, priceLineVisible: false });
  volume.priceScale().applyOptions({ scaleMargins: { top: 0.82, bottom: 0 } });
}

/// Candles of market cap, one bucket per `tf` seconds, from the epoch points.
function drawChart() {
  if (!currentCoin) return;
  ensureChart();
  const factor = unit === 'usd' && usd ? usd : 1;
  const pts = historyOf(currentCoin.token).filter((p) => isInt(p.price));
  const buckets = new Map();
  let prev = null;
  for (const p of pts) {
    const t = Math.floor(p.time / tf) * tf;
    const v = mcapOf(p.price) * factor;
    const b = buckets.get(t) || { time: t, open: prev ?? v, high: prev ?? v, low: prev ?? v, close: v, vol: 0 };
    b.high = Math.max(b.high, v); b.low = Math.min(b.low, v); b.close = v;
    b.vol += Math.abs(Number(BigInt(p.delta_zec || '0'))) / 1e18;
    buckets.set(t, b);
    prev = v;
  }
  if (!buckets.size) {
    const v = mcap(currentCoin) * factor;
    const t = Math.floor(now() / tf) * tf;
    buckets.set(t, { time: t, open: v, high: v, low: v, close: v, vol: 0 });
  }
  const rows = [...buckets.values()].sort((a, b) => a.time - b.time);
  candles.setData(rows.map(({ time, open, high, low, close }) => ({ time, open, high, low, close })));
  volume.setData(rows.map((r) => ({ time: r.time, value: r.vol, color: r.close >= r.open ? 'rgba(53,230,174,.22)' : 'rgba(232,121,79,.22)' })));
  // Few candles stay candle-sized; many fit the width.
  if (rows.length > 40) chart.timeScale().fitContent();
  else { chart.timeScale().applyOptions({ barSpacing: 14, rightOffset: 6 }); chart.timeScale().scrollToRealTime(); }
}

function renderFills() {
  if (!currentCoin) return;
  const moves = historyOf(currentCoin.token).filter((p) => p.delta_lots !== 0 || (p.delta_zec && p.delta_zec !== '0')).slice(-60).reverse();
  $('#fills').innerHTML = moves.length ? moves.map((p) => {
    const buy = p.venue === 'curve' ? p.delta_lots > 0 : BigInt(p.delta_zec) > 0n;
    const tokens = p.venue === 'curve' ? (BigInt(Math.abs(p.delta_lots)) * CURVE.lotTokens).toLocaleString('en-US') : '—';
    return `<tr><td>${ago(p.time)}</td><td class="${buy ? 'up' : 'down'}">${buy ? 'Buy' : 'Sell'}</td><td>${tokens}</td><td>${wad(BigInt(p.delta_zec) < 0n ? -BigInt(p.delta_zec) : BigInt(p.delta_zec), 5)}</td><td>${esc(capText(mcapOf(p.price)))}</td></tr>`;
  }).join('') : '<tr><td colspan="5" class="j-muted">No fills yet. The first buyers all pay one price.</td></tr>';
}

async function renderCoin(token, fresh = false) {
  const c = curves.find((x) => x.token === token);
  if (!c) {
    currentCoin = null;
    $('#coin-title').textContent = curves.length ? 'Coin not found' : 'Reading coins…';
    // Nothing of the last coin stays on screen under this one's address.
    for (const id of ['coin-crumb', 'coin-symbol', 'coin-age', 'coin-mcap', 'coin-change', 'coin-price', 'coin-liquidity', 'coin-pot', 'coin-fee', 'coin-holders', 'curve-pct', 'curve-text', 'trade-output', 'trade-fee', 'trade-min', 'trade-balance']) { const el = $(`#${id}`); if (el) el.textContent = '—'; }
    $('#coin-description').textContent = curves.length
      ? `No coin priced in ZEC.zy has this address on ${NETWORK}.${NETWORK === 'mainnet' ? ' A testnet coin? Add ?network=testnet to the address.' : ''}`
      : '';
    $('#fills').innerHTML = '';
    $('#trade-status').textContent = '';
    return;
  }
  const first = currentCoin?.token !== token;
  currentCoin = c;
  if (first) {
    // A new coin starts with a clean form, and its own last order status.
    tradeSide = 'buy';
    $$('[data-trade]').forEach((item) => item.classList.toggle('active', item.dataset.trade === 'buy'));
    $('#trade-input').value = '0.1';
    $('#trade-status').textContent = orderStatus.get(token) || 'One price per epoch for every buyer, another for every seller.';
  }
  const s = stats.get(token) || {};
  $('#coin-crumb').textContent = c.name;
  $('#coin-title').textContent = c.name;
  $('#coin-symbol').textContent = c.symbol;
  $('#coin-age').textContent = s.created_time ? `made ${ago(s.created_time)}` : `made in epoch ${c.created_at}`;
  $('#coin-copy').textContent = short(c.token);
  $('#coin-description').textContent ||= 'No description.';
  $('#coin-stage').textContent = c.graduated ? 'GRADUATED · TRADING IN ITS LOCKED POOL' : 'ON ITS BONDING CURVE';
  $('#coin-mcap').textContent = capBoth(mcap(c));
  const ch = change24(c);
  $('#coin-change').textContent = `${changeText(ch)} 24h`;
  $('#coin-change').className = ch == null ? '' : ch >= 0 ? 'up' : 'down';
  $('#coin-price').textContent = `${num(price(c)).toLocaleString('en-US', { maximumSignificantDigits: 4, maximumFractionDigits: 20 })} ZEC`;
  $('#coin-liquidity').textContent = `${wad(zecSet(c), 4)} ZEC.zy ${c.graduated ? 'in its pool' : 'in its curve'}`;
  $('#coin-pot').textContent = c.graduated ? 'in its pool' : `${wad(c.graduation, 6)} ZEC.zy`;
  $('#coin-fee').textContent = c.graduated ? `${poolOf(c)?.fee_bps ?? 30} bps, kept by the pool` : c.fee_bps ? `${c.fee_bps / 100}% a trade` : 'none';
  $('#coin-holders').textContent = c.graduated ? 'private' : `private — ${(100 - (c.sold / CURVE.lots) * 100).toFixed(1)}% of supply is still in the curve`;
  const cut = progress(c);
  $('#curve-pct').textContent = `${(cut * 100).toFixed(2)}%`;
  $('#curve-bar').style.width = `${(cut * 100).toFixed(2)}%`;
  $('#curve-text').textContent = c.graduated
    ? 'Graduated. Every unsold lot and everything the curve raised opened a pool whose shares nobody holds.'
    : `${wad(c.reserve, 4)} ZEC.zy in the curve · ${wad(cost(c.sold, CURVE.graduateAt - c.sold), 3)} ZEC.zy of buying to graduate · it graduates at ~47 ZEC market cap${usd ? ` (${usdText(47 * usd)})` : ''}`;
  $('#chart-point').setAttribute('cx', String(54 + 816 * cut));
  $('#chart-point').setAttribute('cy', String(342 - 278 * cut));
  document.title = `${c.symbol} ${capText(mcap(c))} — Jade`;
  const k = await loadContent(token);
  if (currentCoin?.token !== token) return;
  $('#coin-thumb').innerHTML = thumbInner(c);
  $('#coin-description').textContent = k && k !== 'loading' && k.description ? k.description : 'No description.';
  if (first || fresh || !histories.has(token)) {
    try { histories.set(token, await rpc('zyn_history', { token })); } catch { if (!histories.has(token)) histories.set(token, []); }
    if (currentCoin?.token !== token) return;
  }
  drawChart();
  renderFills();
  configureTrade();
}

$('#coin-copy').addEventListener('click', async () => {
  if (!currentCoin) return;
  try { await navigator.clipboard.writeText(currentCoin.token); $('#coin-copy').textContent = 'copied'; setTimeout(() => { $('#coin-copy').textContent = short(currentCoin.token); }, 1200); } catch { /* the address is on screen */ }
});
$$('[data-tf]').forEach((b) => b.addEventListener('click', () => { tf = Number(b.dataset.tf); $$('[data-tf]').forEach((x) => x.classList.toggle('active', x === b)); drawChart(); }));
$$('[data-unit]').forEach((b) => b.addEventListener('click', () => { unit = b.dataset.unit; $$('[data-unit]').forEach((x) => x.classList.toggle('active', x === b)); render(); }));

// ---------------------------------------------------------------------------
// Trading.

const heldOf = (token) => (mine?.holdings.get(token) ?? 0n);

function configureTrade() {
  const c = currentCoin;
  if (!c) return;
  const buying = tradeSide === 'buy';
  $('#trade-unit').textContent = buying ? 'ZEC.zy' : c.symbol;
  const presets = buying
    ? ['0.01', '0.05', '0.1', '0.5'].map((v) => `<button type="button" data-preset="${v}">${v} ZEC</button>`)
    : [25, 50, 75, 100].map((p) => `<button type="button" data-pct="${p}">${p}%</button>`);
  $('#trade-presets').innerHTML = presets.join('');
  $$('[data-preset]').forEach((b) => b.addEventListener('click', () => { $('#trade-input').value = b.dataset.preset; quote(); }));
  $$('[data-pct]').forEach((b) => b.addEventListener('click', () => {
    const lotsHeld = heldOf(c.token) / LOT;
    let lots = c.graduated ? heldOf(c.token) * BigInt(b.dataset.pct) / 100n : (lotsHeld * BigInt(b.dataset.pct)) / 100n * LOT;
    // A share of a few lots can round to none: then the smallest sale there is.
    if (!c.graduated && lots === 0n && lotsHeld > 0n) lots = LOT;
    $('#trade-input').value = decimal(lots);
    quote();
  }));
  $('#trade-balance').textContent = mine
    ? `You hold ${wad(heldOf(c.token), 0)} ${c.symbol} · ${wad(mine.zec, 4)} ZEC.zy`
    : 'Connect a wallet to see your balance.';
  $('.trade-submit').textContent = wallet ? (buying ? `Buy ${c.symbol}` : `Sell ${c.symbol}`) : 'Connect wallet';
  quote();
}

function quote() {
  const c = currentCoin;
  const out = $('#trade-output'), fee = $('#trade-fee'), min = $('#trade-min');
  out.textContent = fee.textContent = min.textContent = '—';
  // What is wrong with the amount goes here; the last order's status keeps
  // its own line, which a refresh never wipes.
  const hint = $('#trade-hint');
  hint.textContent = '';
  const submit = $('.trade-submit');
  if (!c) return null;
  if (wallet) submit.disabled = true;
  const slip = BigInt($('#trade-slip').value);
  try {
    const value = parseDecimal($('#trade-input').value);
    if (value <= 0n) throw new Error('Enter an amount above zero');
    if (tradeSide === 'buy' && mine && value > mine.zec) throw new Error(`You have ${full(mine.zec)} ZEC.zy`);
    if (c.graduated) {
      const p = poolOf(c);
      if (!p) throw new Error('Its pool is not visible yet');
      const sellA = tradeSide === 'buy' ? p.a === XZEC : p.a === c.token;
      if (tradeSide === 'sell' && mine && value > heldOf(c.token)) throw new Error(`You hold ${full(heldOf(c.token))}`);
      const q = poolQuote(p, sellA, value);
      if (q <= 0n) throw new Error('Too small to trade');
      out.textContent = `${full(q)} ${tradeSide === 'buy' ? c.symbol : 'ZEC.zy'}`;
      fee.textContent = `${p.fee_bps / 100}% kept by the pool`;
      min.textContent = `${full((q * (BPS - slip)) / BPS)}`;
      if (wallet) submit.disabled = false;
      return { kind: 'swap', pool: p.id, sellA, amount: value, slippageBps: Number(slip) };
    }
    if (tradeSide === 'buy') {
      if (value < 100_000n * ZAT) throw new Error('A buy is at least 0.001 ZEC.zy');
      const q = previewBuy(c.sold, value, c.fee_bps);
      if (!q.lots) throw new Error('That buys less than one lot of 12,500');
      // At least one lot: a floor of none would be no floor, and Nap refuses it.
      const floorLots = (BigInt(q.lots) * (BPS - slip)) / BPS;
      const minTokens = (floorLots > 0n ? floorLots : 1n) * LOT;
      out.textContent = `${wad(q.tokens, 0)} ${c.symbol}`;
      fee.textContent = c.fee_bps ? `${full(q.fee)} ZEC.zy` : 'none';
      min.textContent = `${wad(minTokens, 0)} ${c.symbol}`;
      if (wallet) submit.disabled = false;
      return { kind: 'buy', amount: value, minTokens };
    }
    if (value % LOT !== 0n) throw new Error(`Sell whole lots of ${CURVE.lotTokens.toLocaleString('en-US')}`);
    if (mine && value > heldOf(c.token)) throw new Error(`You hold ${wad(heldOf(c.token), 0)}`);
    const q = previewSell(c.sold, Number(value / LOT), c.fee_bps);
    const minZec = (q.net * (BPS - slip)) / BPS;
    if (minZec <= 0n) throw new Error('Too small to sell');
    out.textContent = `${full(q.net)} ZEC.zy`;
    fee.textContent = c.fee_bps ? `${full(q.fee)} ZEC.zy` : 'none';
    min.textContent = `${full(minZec)} ZEC.zy`;
    if (wallet) submit.disabled = false;
    return { kind: 'sell', tokens: value, minZec };
  } catch (e) {
    hint.textContent = e.message;
    return null;
  }
}

$$('[data-trade]').forEach((button) => button.addEventListener('click', () => {
  tradeSide = button.dataset.trade;
  $$('[data-trade]').forEach((item) => item.classList.toggle('active', item === button));
  $('#trade-input').value = tradeSide === 'buy' ? '0.1' : '';
  configureTrade();
}));
$('#trade-input').addEventListener('input', quote);
$('#trade-slip').addEventListener('change', quote);

/// After an order: wait for its epoch to seal, collect what it was paid, and
/// say what arrived. Collecting needs no approval — it only moves what the
/// ledger owes this wallet into it.
async function settle(placedEpoch, token, side, symbol) {
  const before = heldOf(token);
  let sealed = false;
  for (let i = 0; i < 120 && !sealed; i++) {
    await new Promise((r) => setTimeout(r, 3000));
    let epoch;
    try { epoch = (await rpc('zyn_status')).epoch; } catch { continue; }
    if (epoch > placedEpoch) sealed = true;
  }
  if (!sealed) {
    setOrder(token, `Still waiting for epoch ${placedEpoch} to seal. Your order is safe; it settles when it does.`);
    return;
  }
  try {
    if (wallet && (!wallet.provider.isNapWeb || wallet.provider.isOpen())) {
      await ask('zyn_collect', {});
      await loadMine();
      const got = heldOf(token) - before;
      setOrder(token, got > 0n ? `Filled: +${wad(got, 0)} ${symbol}. It is in your wallet.`
        : got < 0n ? 'Filled: sold. The ZEC.zy is in your wallet.'
          : side === 'buy' ? 'Not filled — the price moved past your floor, so your ZEC.zy came back to your wallet.'
            : `Not filled — the price moved past your floor, so your ${symbol} came back to your wallet.`);
    } else {
      setOrder(token, "Settled. Open Nap's window to collect it.");
    }
  } catch (e) { setOrder(token, e.message); }
  await loadMarket();
}

$('.trade-submit').addEventListener('click', async () => {
  const c = currentCoin;
  if (!c) return;
  if (!wallet) return pickWallet();
  const q = quote();
  if (!q) return;
  const button = $('.trade-submit');
  button.disabled = true;
  const side = tradeSide, symbol = c.symbol;
  try {
    // Asked before anything is awaited: Nap may need to open its window.
    const pending = q.kind === 'swap' ? ask('zyn_swap', { pool: q.pool, sellA: q.sellA, amount: decimal(q.amount), slippageBps: q.slippageBps })
      : q.kind === 'buy' ? ask('zyn_curveBuy', { token: c.token, amount: decimal(q.amount), minTokens: decimal(q.minTokens) })
        : ask('zyn_curveSell', { token: c.token, tokens: decimal(q.tokens), minZec: decimal(q.minZec) });
    await pending;
    const epoch = (await rpc('zyn_status')).epoch;
    setOrder(c.token, `Placed. Settling when epoch ${epoch} seals…`);
    settle(epoch, c.token, side, symbol);
  } catch (e) {
    // 4001 is Nap's "no"; anything else says what happened in its own words.
    setOrder(c.token, e.code === 4001 ? 'Not placed. Nothing left your wallet.' : e.message);
  } finally {
    button.disabled = false;
    quote();
  }
});

// ---------------------------------------------------------------------------
// The wallet.

async function pickWallet() {
  const dialog = $('#jade-wallet');
  dialog.showModal();
  const body = dialog.querySelector('.jade-wallet-body');
  if (wallet) {
    body.innerHTML = `<p>Connected: <b>${esc(wallet.info.name)}</b></p><p class="mono">${esc(short(wallet.session.account, 8))}</p><button type="button" class="jade-wallet-off">Disconnect</button>`;
    body.querySelector('.jade-wallet-off').onclick = () => {
      // Nap forgets this site too, so connecting again asks again.
      const was = wallet;
      wallet = null; mine = null;
      if (!was.provider.isNapWeb || was.provider.isOpen()) was.provider.request({ method: 'nap_disconnect' }).catch(() => {});
      render(); dialog.close();
    };
    return;
  }
  body.innerHTML = '<p>Looking for Nap…</p>';
  const list = await discover({ web: NAP });
  body.innerHTML = list.map((w, i) => `<button type="button" class="jade-wallet-option" data-i="${i}"><b>${esc(w.info.name)}</b><small>${esc(w.info.detail || '')}</small></button>`).join('')
    + '<p class="jade-wallet-status" role="status"></p><p class="jade-wallet-fine">Connecting shares your Zyn account, a Zcash address and your holdings of these coins with this page — never your keys. Every trade is approved in Nap.</p>';
  body.querySelectorAll('[data-i]').forEach((b) => {
    b.onclick = async () => {
      const status = body.querySelector('.jade-wallet-status');
      status.textContent = list[b.dataset.i].provider.isNapWeb ? "Nap's window is open — unlock it there and approve." : 'Approve in Nap.';
      try {
        const got = await connect(list[b.dataset.i], { network: NETWORK });
        if (got.session?.network !== NETWORK) throw new Error(`Nap answered on ${got.session?.network || 'another network'}, and this page is on ${NETWORK}. Connect again.`);
        wallet = got;
        wallet.provider.on?.('disconnect', () => { wallet = null; mine = null; render(); });
        dialog.close();
        await loadMine();
        render();
      } catch (e) { status.textContent = e.code === 4001 ? 'Not connected. Nothing was shared.' : e.message; }
    };
  });
}

/// Ask the connected Nap. Called from clicks: Nap in the browser may need to
/// open its window, and only a click may.
function ask(method, p) {
  if (!wallet) { pickWallet(); return Promise.reject(new Error('Connect a wallet first.')); }
  // The network rides every request, so Nap refuses one answered on the
  // wrong chain instead of answering it.
  return wallet.provider.request({ method, params: { ...(p || {}), network: NETWORK } });
}

async function loadMine() {
  if (!wallet || (wallet.provider.isNapWeb && !wallet.provider.isOpen())) return;
  try {
    const r = await ask('zyn_curveHoldings', {});
    mine = {
      zec: BigInt(isInt(r?.zec) ? r.zec : '0'),
      holdings: new Map((r?.holdings || []).filter((h) => isHex(h.token) && isInt(h.amount)).map((h) => [h.token, BigInt(h.amount)])),
      created: (r?.created || []).filter((x) => isHex(x.token)).map((x) => x.token),
    };
  } catch { /* keep what we had */ }
}

$('#wallet-button').addEventListener('click', pickWallet);

function renderWallet() {
  const b = $('#wallet-button');
  b.textContent = wallet ? short(wallet.session.account, 5) : 'Connect wallet';
  b.classList.toggle('on', !!wallet);
}

// ---------------------------------------------------------------------------
// Portfolio.

function renderPortfolio() {
  if (!wallet || !mine) {
    $('#portfolio-status').textContent = wallet ? "Nap's window is closed. Connect again with the wallet button to read your holdings." : 'Connect a wallet to see what you hold and what you made.';
    $('#held').innerHTML = '';
    $('#made').innerHTML = '';
    $('#held-total').textContent = '—';
    return;
  }
  $('#portfolio-status').textContent = `${wad(mine.zec, 4)} ZEC.zy ready to trade.`;
  let total = 0;
  const rows = [...mine.holdings].map(([token, amount]) => {
    const c = curves.find((x) => x.token === token);
    if (!c) return '';
    const worth = num((price(c) * amount) / WAD);
    total += worth;
    return `<tr><td><a href="${withNet(`/coin/${token}`)}">${esc(c.name)} <span class="j-ticker">${esc(c.symbol)}</span></a></td><td>${wad(amount, 0)}</td><td>${esc(capText(worth))}</td><td><a href="${withNet(`/coin/${token}`)}">Trade →</a></td></tr>`;
  });
  $('#held').innerHTML = rows.join('') || '<tr><td colspan="4" class="j-muted">Nothing yet.</td></tr>';
  $('#held-total').textContent = `worth ~${capText(total)}`;
  const made = mine.created.map((t) => curves.find((c) => c.token === t)).filter(Boolean);
  $('#made').innerHTML = made.length ? made.map(coinCard).join('') : '<div class="j-empty">No coins made from this wallet yet. <a href="/launch">Create one.</a></div>';
}

// ---------------------------------------------------------------------------
// Create.

const form = $('#create-form');
const nameInput = $('#create-name');
const symbolInput = $('#create-symbol');
const descInput = $('#create-description');
const imageInput = $('#create-image');
const launchButton = $('#create-go');
let imageData = '';
const validSymbol = (v) => /^[A-Z0-9]{1,8}$/.test(v);
const validName = (v) => v.length >= 1 && v.length <= 24 && v === v.trim() && /^[\x20-\x7e]+$/.test(v);

/// Shrink a picture to a square ≤ 512 px and under 300 KB, here, before it
/// goes anywhere.
async function shrink(file) {
  const bitmap = await createImageBitmap(file);
  const side = Math.min(512, Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = side;
  const scale = Math.max(side / bitmap.width, side / bitmap.height);
  const w = bitmap.width * scale, h = bitmap.height * scale;
  canvas.getContext('2d').drawImage(bitmap, (side - w) / 2, (side - h) / 2, w, h);
  for (const q of [0.9, 0.8, 0.7, 0.55, 0.4, 0.3]) {
    const url = canvas.toDataURL('image/webp', q);
    if (url.startsWith('data:image/webp') && url.length <= 400_000) return url;
    const jpg = canvas.toDataURL('image/jpeg', q);
    if (jpg.length <= 400_000) return jpg;
  }
  throw new Error('That picture will not shrink under 300 KB');
}

imageInput.addEventListener('change', async () => {
  const f = imageInput.files?.[0];
  imageData = '';
  if (f) {
    if (!/^image\/(png|jpeg|webp|gif)$/.test(f.type)) {
      imageInput.value = '';
      $('#create-status').textContent = 'A picture is a PNG, JPEG, WebP or GIF.';
    } else if (f.size > 15 * 1024 * 1024) {
      imageInput.value = '';
      $('#create-status').textContent = 'That picture is over 15 MB; choose a smaller one.';
    } else {
      try { imageData = await shrink(f); } catch (e) { imageInput.value = ''; $('#create-status').textContent = e.message === 'That picture will not shrink under 300 KB' ? e.message : 'That file could not be read as a picture.'; }
    }
  }
  renderPreview();
});

function renderPreview() {
  const typed = symbolInput.value.trim().toUpperCase();
  const symbol = typed || 'COIN';
  // The placeholder shows in the preview; it is not a ticker anyone chose.
  const ok = validSymbol(typed) && validName(nameInput.value);
  $('#preview-name').textContent = nameInput.value.trim() || 'Unnamed coin';
  $('#preview-symbol').textContent = symbol;
  $('#preview-description').textContent = descInput.value.trim() || 'No description.';
  $('#preview-thumb').innerHTML = imageData ? `<img src="${esc(imageData)}" alt="" />` : `<span id="preview-letter">${esc(symbol[0] || '?')}</span>`;
  launchButton.disabled = !ok || creating;
  const bond = BigInt(terms.bond || '0');
  $('#create-terms').textContent = terms.set
    ? `${terms.fee_bps / 100}% on every curve trade — half to the ZYN pot, 40% to this coin's pool, 10% to you when it graduates. ${bond ? `Bond ${wad(bond)} ZEC.zy — returned only if the coin graduates; otherwise it stays in the curve.` : 'No bond.'}`
    : 'No fee and no bond are set on this network yet.';
  if (creating) return;
  if (!ok) $('#create-status').textContent = 'A ticker is 1–8 letters or digits; a name is 1–24 plain characters.';
  else if (!/Created|refused|picture|Picture|attach/.test($('#create-status').textContent)) $('#create-status').textContent = wallet ? 'Ready: Nap shows you everything before it is created.' : 'Connect a wallet to create it.';
}

form.querySelectorAll('input,textarea').forEach((input) => input.addEventListener('input', renderPreview));
/// Attach a coin's picture and description with the proof Nap made once.
/// Kept for this tab until it succeeds, so a failed upload can be retried.
async function attach(token, pending) {
  const up = await fetch(`/api/content/${token}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(pending),
  });
  if (up.ok || up.status === 409) { try { sessionStorage.removeItem(`jade.attach.${token}`); } catch { /* none kept */ } content.delete(token); return null; }
  return (await up.json().catch(() => ({}))).error || `HTTP ${up.status}`;
}

form.addEventListener('submit', async (event) => {
  event.preventDefault();
  if (creating) return;
  const symbol = symbolInput.value.trim().toUpperCase();
  if (!validSymbol(symbol) || !validName(nameInput.value)) return renderPreview();
  creating = true;
  launchButton.disabled = true;
  const status = $('#create-status');
  let token = null;
  try {
    const description = descInput.value.trim();
    status.textContent = 'Approve it in Nap.';
    const r = await ask('zyn_curveCreate', { symbol, name: nameInput.value.trim(), description, image: imageData });
    if (!isHex(r?.token)) throw new Error('Nap did not say which coin it made');
    token = r.token;
    // The coin exists from here on, whatever happens to the picture.
    let note = `Created ${symbol} (${short(token)}). It can be traded right away.`;
    if (r.content?.proof && (description || imageData)) {
      const pending = { content: { description, image: imageData }, proof: r.content.proof };
      try { sessionStorage.setItem(`jade.attach.${token}`, JSON.stringify(pending)); } catch { /* this call only */ }
      let err;
      try { err = await attach(token, pending); } catch { err = 'the upload did not go through'; }
      note += err ? ` The picture and description were not attached yet (${err}). ` : ' Picture and description attached.';
      if (err) {
        status.innerHTML = `${esc(note)}<button type="button" class="j-link" id="attach-again">Attach again</button>`;
        $('#attach-again').onclick = async () => {
          const again = await attach(token, pending).catch(() => 'the upload did not go through');
          status.textContent = again ? `Still not attached (${again}). Try again in a moment.` : 'Picture and description attached.';
        };
        return;
      }
    }
    status.textContent = note;
    form.reset(); imageData = '';
    setTimeout(async () => { await loadMarket(); navigate(`/coin/${token}`); }, 4000);
  } catch (e) {
    status.textContent = e.code === 4001 ? 'Not created. Nothing left your wallet.' : e.message;
  } finally {
    creating = false;
    // The button stays off once a coin is made from this form; it was reset.
    renderPreview();
    if (token) launchButton.disabled = true;
  }
});

// ---------------------------------------------------------------------------
// Routes.

function pathRoute() {
  const path = location.pathname.replace(/\/+$/, '') || '/';
  if (path === '/launch') return { view: 'launch' };
  if (path === '/portfolio') return { view: 'portfolio' };
  const match = path.match(/^\/coin\/([0-9a-f]{64})$/i);
  return match ? { view: 'coin', token: match[1].toLowerCase() } : { view: 'gallery' };
}

function render() {
  renderWallet();
  const route = pathRoute();
  if (route.view === 'gallery') renderGallery();
  if (route.view === 'coin') renderCoin(route.token);
  if (route.view === 'launch') renderPreview();
  if (route.view === 'portfolio') renderPortfolio();
}

function syncRoute({ scroll = true } = {}) {
  const route = pathRoute();
  Object.entries(views).forEach(([name, view]) => { view.hidden = name !== route.view; });
  $$('.tabs [data-route]').forEach((link) => {
    if (link.dataset.route === route.view || (route.view === 'coin' && link.dataset.route === 'gallery')) link.setAttribute('aria-current', 'page');
    else link.removeAttribute('aria-current');
  });
  document.body.dataset.route = route.view;
  if (route.view !== 'coin') document.title = { gallery: 'Jade', launch: 'Create a coin — Jade', portfolio: 'Portfolio — Jade' }[route.view];
  if (route.view === 'coin') renderCoin(route.token, true); else render();
  if (route.view === 'portfolio') loadMine().then(renderPortfolio);
  if (scroll) window.scrollTo({ top: 0, behavior: 'instant' });
}

function navigate(path) {
  history_push(path);
  syncRoute();
}
const history_push = (path) => window.history.pushState({}, '', path + location.search);

document.addEventListener('click', (event) => {
  const link = event.target.closest('a[href^="/"]');
  // The network pill loads the other chain: a full page load, not a route.
  if (!link || link.id === 'net' || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  event.preventDefault();
  navigate(link.getAttribute('href'));
});
window.addEventListener('popstate', () => syncRoute());

// Live: a coin page every 5 seconds (epochs are about 10), the list every 20.
setInterval(async () => {
  if (document.hidden) return;
  const route = pathRoute();
  if (route.view !== 'coin') return;
  await loadMarket();
  try {
    const h = await rpc('zyn_history', { token: route.token });
    histories.set(route.token, h);
  } catch { /* keep */ }
  if (currentCoin?.token !== route.token || pathRoute().token !== route.token) return;
  drawChart();
  renderFills();
}, 5_000);
setInterval(() => { if (!document.hidden && pathRoute().view !== 'coin') loadMarket(); }, 20_000);
setInterval(loadUsd, 300_000);

// The network pill: what this page reads, and a link to the other one.
const net = $('#net');
net.textContent = NETWORK === 'testnet' ? 'Testnet' : 'Mainnet';
net.href = NETWORK === 'testnet' ? '/' : '/?network=testnet';
net.title = NETWORK === 'testnet' ? 'Switch to mainnet' : 'Switch to testnet';
installTips();

syncRoute({ scroll: false });
loadMarket();
loadUsd();
document.documentElement.dataset.jadeReady = 'true';
