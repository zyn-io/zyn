// Jade — Zyn's launchpad (DECISIONS §141), as a trading terminal.
//
// Reads come from the chain's public RPC: curves, prices, per-epoch fills —
// anyone can browse without a wallet. Pictures and descriptions come
// from this site's own store, attached by each coin's creator with a proof.
// Everything that moves value goes through Nap (Nap Connect), which shows the
// holder what the chain says and asks them to approve it in Nap's window.
//
// The gateway limits each client (about 8 requests a second), so the page is
// frugal: the board reads the market every 10 s and a coin's fills only when
// its last trade changed; a coin view reads every 5 s, and the epoch every
// second only while a coin view is open (that is how the seal is timed).

import { connect, discover, NAP_WEB } from './nap-connect.js';
import { createChart } from './vendor/lightweight-charts.js';
import { installTips, tip } from './tip.js';

const XZEC = '0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201';

/// The curve every token shares (objectvm::curve). Consensus, not settings.
const CURVE = Object.freeze({ lots: 80_000, lotTokens: 12_500n, baseZat: 7_721n, slopeZat: 1n, graduateAt: 51_098 });
const WAD = 10n ** 18n;
const ZAT = 10_000_000_000n;
const LOT = CURVE.lotTokens * WAD;
const BPS = 10_000n;
const SUPPLY = 1_000_000_000n;
/// Fully diluted value at graduation, in units of a curve's size (§166).
const GRADUATION_MC_UNITS = (CURVE.baseZat + CURVE.slopeZat * BigInt(CURVE.graduateAt)) * BigInt(CURVE.lots);

const params = new URLSearchParams(location.search);
const NETWORK = params.get('network') === 'testnet' ? 'testnet' : 'mainnet';
/// Only a page served from this machine may point elsewhere (development).
const LOCAL = ['localhost', '127.0.0.1'].includes(location.hostname);
const RPC = (LOCAL && params.get('rpc')) || (NETWORK === 'testnet' ? 'https://zyn-rpc.cavebro.win/object' : 'https://zyn-rpc.cavebro.win/');
const NAP = (LOCAL && params.get('nap')) || NAP_WEB;
/// Nap's Chrome extension, from its GitHub releases: the newest build, and its install steps.
const NAP_EXTENSION = 'https://github.com/zyn-io/nap-wallet/releases/latest/download/nap-wallet-chrome.zip';
const NAP_RELEASE = 'https://github.com/zyn-io/nap-wallet/releases/latest';

/// The bridged coins a coin can be priced in (§166), by asset id. Each
/// curve is sized for its coin by the ledger; these only name them.
const QUOTE_ASSETS = {
  mainnet: {
    [XZEC]: 'ZEC',
    '9f5a2c088a0071ba7afcb2c74b9c7cc23f0f94c297d97aef68e50392c04acc6c': 'ETH',
    'ddf2b7ecc2634e39a891a19020205ae26a87e9f753e492e03b84e7dc627f250f': 'BOLD',
  },
  testnet: {
    [XZEC]: 'ZEC',
    '56737e143259c754d165074ad15058139bc0b116f5a22b2f3d50129a2a7a24a6': 'ETH',
    '9204324e0511b4d1a90d0ff36242f044e894e1b71489d47abd824df273092ad1': 'SOL',
    '8c1a1b0616cef63250e9f5bab8b20908320cc47ead0f60215a229d34b5c93ac2': 'BOLD',
  },
}[NETWORK];

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
const zecText = (zec) => (zec >= 1000 ? zec.toLocaleString('en-US', { maximumFractionDigits: 0 }) : zec >= 1 ? zec.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) : zec.toLocaleString('en-US', { maximumSignificantDigits: 3 }));
const usdText = (usd) => (usd >= 1e6 ? `$${(usd / 1e6).toFixed(2)}M` : usd >= 1e3 ? `$${(usd / 1e3).toFixed(1)}K` : `$${usd.toFixed(2)}`);
function ago(t) {
  if (!t) return '—';
  const s = Math.max(0, now() - t);
  return s < 60 ? `${s}s ago` : s < 3600 ? `${Math.floor(s / 60)}m ago` : s < 86400 ? `${Math.floor(s / 3600)}h ago` : `${Math.floor(s / 86400)}d ago`;
}
/// The same, as a terminal shows it in a cell: "42s", "7m", "3h", "12d".
const agoShort = (t) => (t ? ago(t).replace(' ago', '') : '—');
const pct = (a, b) => (b > 0 ? ((a - b) / b) * 100 : null);
const changeText = (p) => (p == null ? '—' : `${p >= 0 ? '+' : ''}${p.toFixed(p >= 100 || p <= -100 ? 0 : 1)}%`);
const changeCls = (p) => (p == null || Math.abs(p) < 0.05 ? 'flat' : p > 0 ? 'up' : 'down');
/// ZEC amounts in a cell: short, never in exponent form.
const zecCell = (zec) => (zec === 0 ? '0' : zec >= 1e6 ? `${(zec / 1e6).toFixed(2)}M` : zec >= 1e4 ? `${(zec / 1e3).toFixed(1)}K` : zec >= 100 ? zec.toFixed(0) : zec >= 1 ? zec.toFixed(2) : zec >= 0.0001 ? zec.toFixed(4).replace(/0+$/, '').replace(/\.$/, '') : zec.toLocaleString('en-US', { maximumSignificantDigits: 2, maximumFractionDigits: 12 }));
/// A price per token, which is tiny: the zeros after the point counted as a
/// subscript, the way trading screens write it — 0.0₈6295 is 0.000000006295.
/// Markup (a <sub>); `plain` gives the same as text.
function priceText(v, plain = false) {
  if (!Number.isFinite(v) || v <= 0) return '—';
  if (v >= 0.001) return v.toLocaleString('en-US', { maximumSignificantDigits: 5 });
  const zeros = Math.floor(-Math.log10(v));
  const digits = Math.round(v * 10 ** (zeros + 4)).toString().slice(0, 5).replace(/0+$/, '') || '0';
  return plain ? `0.0${String(zeros).split('').map((d) => '₀₁₂₃₄₅₆₇₈₉'[d]).join('')}${digits}` : `0.0<sub>${zeros}</sub>${digits}`;
}

// The curve's own arithmetic, for previews. Each epoch's orders settle
// together at one price, so these are what an order gets alone.
// A curve's `unit` is its size: the ledger amount one "zatoshi" of the
// shared shape stands for (ZEC.zy's is one zatoshi).
const cost = (q, d, unit = ZAT) => { const Q = BigInt(q), D = BigInt(d); return (CURVE.baseZat * D + CURVE.slopeZat * ((D * (2n * Q + D - 1n)) / 2n)) * unit; };
function lotsFor(q, budget, cap, unit = ZAT) {
  let lo = 0, hi = cap;
  while (lo < hi) { const mid = lo + Math.ceil((hi - lo) / 2); if (cost(q, mid, unit) <= budget) lo = mid; else hi = mid - 1; }
  return lo;
}
function previewBuy(sold, amountIn, feeBps, unit = ZAT) {
  const budget = (amountIn * BPS) / (BPS + BigInt(feeBps));
  const lots = lotsFor(sold, budget, Math.max(0, CURVE.graduateAt - sold), unit);
  const paid = cost(sold, lots, unit);
  return { lots, tokens: BigInt(lots) * LOT, paid, fee: (paid * BigInt(feeBps)) / BPS };
}
function previewSell(sold, lots, feeBps, unit = ZAT) {
  const n = Math.min(lots, sold);
  const gross = n ? cost(sold - n, n, unit) : 0n;
  const fee = (gross * BigInt(feeBps)) / BPS;
  return { gross, fee, net: gross - fee };
}
function poolQuote(pool, sellA, amountIn) {
  const x = BigInt(sellA ? pool.reserve_a : pool.reserve_b);
  const y = BigInt(sellA ? pool.reserve_b : pool.reserve_a);
  const a = (amountIn * (BPS - BigInt(pool.fee_bps))) / BPS;
  return (a * y) / (x + 2n * a);
}
/// A route alone in its venues' batches (§167): each hop fed by the one
/// before — Nap's `previewRoute`, on the same venues.
function previewRoute(hops, amountIn, venues) {
  let amount = BigInt(amountIn);
  const steps = [];
  for (const h of hops) {
    if (h.pool) amount = poolQuote(venues.pool(h.pool), h.sellA, amount);
    else {
      const c = venues.curve(h.curveSell || h.curveBuy);
      const unit = c.unit && c.unit !== '0' ? BigInt(c.unit) : ZAT;
      amount = h.curveSell ? previewSell(c.sold, Number(amount / LOT), c.fee_bps, unit).net : previewBuy(c.sold, amount, c.fee_bps, unit).tokens;
    }
    steps.push(amount);
    if (amount <= 0n) return null;
  }
  return { out: amount, steps };
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
/// What new coins launch in: [{ asset, symbol, unit, bond }]. A node from
/// before §166 names none, and coins launch in ZEC.zy at ZEC's size.
let quotes = [];
/// Dollars per coin, by symbol (ZEC, ETH, SOL, BOLD); `usd` is ZEC's.
let usdBy = {};
let usd = null;
let wallet = null;
let mine = null; // { zec, quotes: Map asset -> amount | null, holdings: Map token -> amount, created: [] }
let currentCoin = null;
let tradeSide = 'buy';
let unit = 'zec';
let tf = 300;
let marketRead = false;
/// The chain's epoch as last read, and when it was seen to change: the seal
/// cadence is timed here, from the page's own polling (the chain publishes no
/// seal times). `epochSeconds` is what the market says, until it is measured.
let epochSeconds = 60;
const live = { epoch: null, seen: 0, changedAt: 0, cadence: null };
/// Each coin's epoch points, by token, so one coin's chart never shows under another.
const histories = new Map();
const historyOf = (token) => histories.get(token) || [];
/// When each history was read, and the coin's last trade then: a history is
/// read again only when the coin has traded since.
const historyRead = new Map();
/// The last order's status, per coin: written by the order, kept across the
/// page's refreshes, shown only on its own coin's page.
const orderStatus = new Map();
function setOrder(token, text, step = 'idle') {
  orderStatus.set(token, { text, step });
  if (currentCoin?.token === token) showOrder(token);
}
function showOrder(token) {
  const o = orderStatus.get(token);
  $('#trade-status').textContent = o ? o.text : 'One price per epoch for every buyer, another for every seller.';
  $('#trade-status').classList.toggle('lit', !!o);
  const steps = $('#trade-steps');
  const step = o?.step || 'idle';
  steps.dataset.step = step;
  const order = ['placed', 'sealing', 'done'];
  const at = { placed: 0, sealing: 1, done: 2, missed: 2, error: 0 }[step] ?? -1;
  steps.classList.toggle('bad', step === 'missed' || step === 'error');
  steps.querySelectorAll('li').forEach((li) => {
    const i = order.indexOf(li.dataset.s);
    li.classList.toggle('done', i < at || (step === 'done' && i === at));
    li.classList.toggle('now', i === at && step !== 'done');
  });
  steps.querySelector('[data-s="done"]').textContent = step === 'missed' ? 'Not filled' : 'Settled';
}
let creating = false;
const content = new Map(); // token -> {description, image} | null (none) | 'loading'

async function rpc(method, fields = {}) {
  const id = crypto.randomUUID();
  let r;
  try {
    r = await fetch(RPC, {
      method: 'POST',
      // JSON as text/plain: no CORS preflight for the gateway's rate limit to refuse.
      headers: { 'content-type': 'text/plain;charset=UTF-8' },
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

const validCurve = (c) => isHex(c.token) && isHex(c.meta) && !!QUOTE_ASSETS[c.quote] && Number.isSafeInteger(c.sold) && c.sold >= 0
  && ['reserve', 'graduation', 'price_per_token'].every((k) => isInt(c[k])) && typeof c.graduated === 'boolean';

/// A coin's quote: its symbol, size, and dollars per unit of it.
const symOf = (asset) => QUOTE_ASSETS[asset] || `${String(asset).slice(0, 6)}…`;
const qsym = (c) => symOf(c?.quote || XZEC);
const unitOf = (c) => (isInt(c?.unit) && c.unit !== '0' ? BigInt(c.unit) : ZAT);
const usdPer = (c) => usdBy[qsym(c)] ?? null;
/// Its market cap at graduation, in its quote.
const gradMc = (unit) => num(GRADUATION_MC_UNITS * unit);
/// What this wallet holds of a quote asset, or null when its Nap does not say.
function balanceOf(asset) {
  if (!mine) return null;
  if (mine.quotes?.has(asset)) return mine.quotes.get(asset);
  return asset === XZEC ? mine.zec : null;
}
/// Amounts in several coins: in dollars when every one is priced, else each in its own.
function sumText(by, cell = zecCell) {
  const parts = [...by].filter(([, v]) => v > 0);
  if (!parts.length) return '0';
  if (parts.every(([s]) => usdBy[s])) return usdText(parts.reduce((t, [s, v]) => t + v * usdBy[s], 0));
  return parts.map(([s, v]) => `${cell(v)} ${s}`).join(' · ');
}

async function loadMarket() {
  try {
    const [m, s] = await Promise.all([rpc('zyn_market'), rpc('zyn_curveStats').catch(() => [])]);
    terms = m.curve_terms || terms;
    const named = (m.curve_quotes || []).filter((q) => isHex(q.asset) && QUOTE_ASSETS[q.asset] && isInt(q.unit) && isInt(q.bond));
    quotes = named.length
      ? named.map((q) => ({ asset: q.asset, symbol: symOf(q.asset), unit: BigInt(q.unit), bond: BigInt(q.bond) }))
      : [{ asset: XZEC, symbol: 'ZEC', unit: ZAT, bond: BigInt(terms.bond || '0') }];
    pools = (m.pools || []).filter((p) => isHex(p.id) && isInt(p.reserve_a) && isInt(p.reserve_b));
    curves = (m.curves || []).filter(validCurve).map((c) => ({ ...c, pending: Number.isSafeInteger(c.pending) && c.pending > 0 ? c.pending : 0, ...decodeMeta(c.meta) }));
    stats = new Map((Array.isArray(s) ? s : []).filter((x) => isHex(x.token)).map((x) => [x.token, x]));
    if (Number.isSafeInteger(m.epoch_seconds) && m.epoch_seconds > 0) epochSeconds = m.epoch_seconds;
    if (Number.isSafeInteger(m.epoch)) noteEpoch(m.epoch, false);
    marketRead = true;
    $('#board-status').textContent = curves.length ? `Live on ${NETWORK}. A coin is its address; names and tickers may repeat.` : '';
  } catch (e) {
    $('#board-status').textContent = `The chain is not answering: ${e.message}`;
  }
  render();
  refreshHistories();
}

async function loadUsd() {
  try {
    const r = await fetch('/api/usd', { credentials: 'omit' });
    const j = await r.json();
    usdBy = Object.fromEntries(Object.entries(j?.usd || {}).map(([k, v]) => [k, Number(v)]).filter(([, v]) => Number.isFinite(v) && v > 0));
  } catch { usdBy = {}; }
  usd = usdBy.ZEC ?? null;
  $('#zec-usd').textContent = usd ? `ZEC $${usd.toLocaleString('en-US', { maximumFractionDigits: 2 })}` : 'ZEC';
  $$('[data-unit="usd"]').forEach((b) => { b.hidden = !usd; });
  render();
}

const poolOf = (c) => pools.find((p) => p.id === c.pool);
/// Its quote per token: the next lot on the curve, or the pool's spot.
function price(c) {
  const p = c.graduated && poolOf(c);
  if (!p) return BigInt(c.price_per_token);
  const [z, t] = p.a === c.quote ? [p.reserve_a, p.reserve_b] : [p.reserve_b, p.reserve_a];
  return (BigInt(z) * WAD) / BigInt(t);
}
/// Market cap in its quote: price per token times the whole supply.
const mcap = (c) => num(price(c) * SUPPLY);
const mcapOf = (priceRaw) => num(BigInt(priceRaw) * SUPPLY);
function zecSet(c) {
  if (!c.graduated) return BigInt(c.reserve);
  const p = poolOf(c);
  return p ? BigInt(p.a === c.quote ? p.reserve_a : p.reserve_b) : 0n;
}
const progress = (c) => (c.graduated ? 1 : Math.min(1, c.sold / CURVE.graduateAt));
const change24 = (c) => { const s = stats.get(c.token); return s?.price_24h ? pct(Number(price(c)), Number(s.price_24h)) : null; };
const change1h = (c) => { const s = stats.get(c.token); return s?.price_1h ? pct(Number(price(c)), Number(s.price_1h)) : null; };
/// The change since `secs` ago, from the coin's own batches (5m is not in the stats).
function changeSince(c, secs) {
  const cut = now() - secs;
  let ref = null;
  for (const p of historyOf(c.token)) { if (!isInt(p.price)) continue; if (p.time <= cut) ref = p; else break; }
  return ref ? pct(Number(price(c)), Number(ref.price)) : null;
}
const vol24 = (c) => { const v = stats.get(c.token)?.volume_24h; return isInt(v) ? num(v) : 0; };
const moves24 = (c) => stats.get(c.token)?.moves_24h || 0;
const pendingOf = (c) => (c.graduated ? (poolOf(c)?.pending || 0) : c.pending) || 0;
/// An amount in a coin's quote, in dollars when the toggle says so and it is priced.
const capText = (v, c = currentCoin) => (unit === 'usd' && usdPer(c) ? usdText(v * usdPer(c)) : `${zecText(v)} ${qsym(c)}`);
/// For ranking across quotes: dollars when priced, else the quote amount.
const mcSort = (c) => (usdPer(c) ? mcap(c) * usdPer(c) : mcap(c));
const volSort = (c) => (usdPer(c) ? vol24(c) * usdPer(c) : vol24(c));

/// A batch fill's side and size, from either venue.
function fillOf(p) {
  const dz = BigInt(p.delta_zec || '0');
  const buy = p.venue === 'curve' ? p.delta_lots > 0 : dz > 0n;
  return { buy, zec: Math.abs(Number(dz)) / 1e18, zecRaw: dz < 0n ? -dz : dz, tokens: p.venue === 'curve' ? BigInt(Math.abs(p.delta_lots)) * CURVE.lotTokens : null };
}
const moved = (p) => p.delta_lots !== 0 || (p.delta_zec && p.delta_zec !== '0');

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
const hasPicture = (c) => { const k = content.get(c.token); return !!(k && k !== 'loading' && k.image); };

// ---------------------------------------------------------------------------
// Histories: every coin's batches, read politely. The board's tape and the
// coin view's chart both come from these.

let historyBusy = false;
async function loadHistory(token) {
  const last = stats.get(token)?.last_trade_time || 0;
  try {
    const h = await rpc('zyn_history', { token });
    if (Array.isArray(h)) histories.set(token, h.filter((p) => Number.isSafeInteger(p.time) && Number.isSafeInteger(p.epoch)).sort((a, b) => a.time - b.time || a.epoch - b.epoch));
    historyRead.set(token, { last, at: Date.now() });
  } catch { if (!histories.has(token)) histories.set(token, []); }
}
/// The 24 most recently traded coins, each read again only once it has traded
/// since; two at a time, so a first visit stays far under the gateway's limit.
async function refreshHistories() {
  if (historyBusy || document.hidden) return;
  historyBusy = true;
  try {
    const want = curves.map((c) => [c.token, stats.get(c.token)?.last_trade_time || 0])
      .filter(([, t]) => t).sort((a, b) => b[1] - a[1]).slice(0, 24)
      .filter(([token, t]) => historyRead.get(token)?.last !== t)
      .map(([token]) => token);
    if (!want.length) return;
    const queue = [...want];
    await Promise.all([0, 1].map(async () => { while (queue.length) await loadHistory(queue.shift()); }));
    renderTape();
    if (pathRoute().view === 'board') scheduleBoard();
  } finally { historyBusy = false; }
}

// ---------------------------------------------------------------------------
// The tape: the latest batch fills across every coin read so far.

let tapeKey = '';
let tapeNewest = 0;
function renderTape() {
  const fills = [];
  for (const c of curves) {
    for (const p of historyOf(c.token)) if (moved(p) && isInt(p.price)) fills.push({ c, p });
  }
  fills.sort((a, b) => b.p.time - a.p.time || b.p.epoch - a.p.epoch);
  const top = fills.slice(0, 30);
  const key = top.map(({ c, p }) => `${c.token.slice(0, 8)}${p.epoch}`).join(',') + Object.keys(usdBy).join('');
  const track = $('#tape-track');
  if (!top.length) {
    if (marketRead && tapeKey !== 'none') { track.className = 'tape-track'; track.innerHTML = `<span class="tape-empty">${curves.length ? 'No fills yet.' : 'No coins yet.'}</span>`; tapeKey = 'none'; }
    return;
  }
  if (key === tapeKey) { tickAgo(track); return; }
  tapeKey = key;
  const before = tapeNewest;
  tapeNewest = Math.max(tapeNewest, top[0].p.time);
  const item = ({ c, p }) => {
    const f = fillOf(p);
    return `<a class="tp${before && p.time > before ? ' fresh' : ''}" href="${withNet(`/coin/${c.token}`)}" data-go="${c.token}" title="${esc(c.name)} · epoch ${p.epoch}">
      ${thumb(c)}<b>${esc(c.symbol)}</b><span class="s ${f.buy ? 'up' : 'down'}">${f.buy ? '▲ Buy' : '▼ Sell'}</span><span>${zecCell(f.zec)} ${esc(qsym(c))}</span><span class="t">MC ${usdPer(c) ? usdText(mcapOf(p.price) * usdPer(c)) : `${zecCell(mcapOf(p.price))} ${esc(qsym(c))}`}</span><span class="t" data-ago="${p.time}"></span></a>`;
  };
  const html = top.map(item).join('');
  track.className = 'tape-track';
  track.style.removeProperty('--tape-dur');
  track.innerHTML = html;
  tickAgo(track);
  // A tape longer than the screen runs; one that fits stays still.
  requestAnimationFrame(() => {
    const view = $('#tape-view');
    const w = track.scrollWidth;
    if (w > view.clientWidth + 8 && !matchMedia('(prefers-reduced-motion: reduce)').matches) {
      track.innerHTML = html + html.replace(/ fresh"/g, '"');
      tickAgo(track);
      track.style.setProperty('--tape-dur', `${Math.max(30, w / 38)}s`);
      track.classList.add('run');
    }
  });
}

/// Every "ago" on screen, kept current once a second without re-rendering.
function tickAgo(root = document) {
  root.querySelectorAll('[data-ago]').forEach((el) => {
    const t = Number(el.dataset.ago);
    const text = el.dataset.agoLong ? ago(t) : agoShort(t);
    if (el.textContent !== text) el.textContent = text;
  });
}

// ---------------------------------------------------------------------------
// The board: New, Bonding, Graduated.

const views = Object.fromEntries($$('[data-view]').map((view) => [view.dataset.view, view]));
const search = $('#search');
const SIX_HOURS = 6 * 3600;
const filters = new Set();
const COLS = ['new', 'bonding', 'graduated'];
const colSort = { new: { key: 'age', dir: -1 }, bonding: { key: 'bonded', dir: -1 }, graduated: { key: 'mc', dir: -1 } };
const SORTS = [
  [null],
  ['l', ['age', 'Age'], ['last', 'Last']],
  ['', ['mc', 'MCap']],
  ['', ['ch1', '1h'], ['ch24', '24h']],
  ['', ['vol', 'Vol'], ['moves', 'Moves']],
  ['', ['bonded', 'Bonded']],
];

function columnOf(c) {
  if (c.graduated) return 'graduated';
  const made = stats.get(c.token)?.created_time || 0;
  return c.sold === 0 || (made && now() - made < SIX_HOURS) ? 'new' : 'bonding';
}
function matches(c) {
  const q = search.value.trim().toLocaleLowerCase();
  if (q && !`${c.name} ${c.symbol} ${c.token}`.toLocaleLowerCase().includes(q)) return false;
  const s = stats.get(c.token) || {};
  if (filters.has('traded') && !(moves24(c) > 0 || (s.last_trade_time && now() - s.last_trade_time < 86400))) return false;
  if (filters.has('bonded') && progress(c) < 0.1) return false;
  if (filters.has('picture') && !hasPicture(c)) return false;
  if (filters.has('held') && heldOf(c.token) <= 0n) return false;
  return true;
}
function sortValue(c, key) {
  const s = stats.get(c.token) || {};
  switch (key) {
    case 'age': return s.created_time || 0;
    case 'last': return s.last_trade_time || 0;
    case 'mc': return mcSort(c);
    case 'ch1': return change1h(c) ?? -Infinity;
    case 'ch24': return change24(c) ?? -Infinity;
    case 'vol': return volSort(c);
    case 'moves': return moves24(c);
    default: return progress(c);
  }
}

function renderSortHeads() {
  for (const col of COLS) {
    const box = $(`[data-sorts="${col}"]`);
    const { key, dir } = colSort[col];
    box.innerHTML = SORTS.map(([align, ...keys]) => (align === null ? '<span></span>'
      : `<div class="grp ${align}">${keys.map(([k, label]) => `<button type="button" data-sort="${k}" data-col="${col}" class="${k === key ? `on${dir > 0 ? ' asc' : ''}` : ''}" aria-pressed="${k === key}">${label}</button>`).join('')}</div>`)).join('');
  }
}
document.addEventListener('click', (event) => {
  const b = event.target.closest('[data-sort]');
  if (!b) return;
  const s = colSort[b.dataset.col];
  if (s.key === b.dataset.sort) s.dir = -s.dir; else { s.key = b.dataset.sort; s.dir = -1; }
  renderSortHeads();
  renderBoard();
});

const rowEls = new Map();
const rowSnap = new Map();
let boardPainted = false;

function rowHtml(c) {
  const s = stats.get(c.token) || {};
  const mc = mcap(c);
  const h1 = change1h(c), h24 = change24(c);
  const q = pendingOf(c);
  const cut = progress(c) * 100;
  return `<div class="j-thumb">${thumbInner(c)}${q ? '<i class="dot" aria-hidden="true"></i>' : ''}</div>
    <div class="c-coin"><p class="nm"><b>${esc(c.name)}</b></p>
      <p class="sub"><span class="tk">${esc(c.symbol)}</span>${s.created_time ? `<span class="ago" data-ago="${s.created_time}" title="Age"></span>` : ''}${s.last_trade_time ? `<span class="lt" data-ago="${s.last_trade_time}" title="Last trade"></span>` : ''}${q ? `<span class="tag-q">${q} in batch</span>` : ''}</p></div>
    <div class="c-num"><b data-k="mc">${usdPer(c) ? usdText(mc * usdPer(c)) : zecCell(mc)}</b><span>${usdPer(c) ? `${zecCell(mc)} ` : ''}${esc(qsym(c))}</span></div>
    <div class="c-num"><b data-k="h1" class="${changeCls(h1)}">${changeText(h1)}</b><span data-k="h24" class="${changeCls(h24)}">${changeText(h24)}</span></div>
    <div class="c-num"><b data-k="vol">${usdPer(c) ? usdText(vol24(c) * usdPer(c)) : zecCell(vol24(c))}</b><span data-k="mv">${moves24(c)} mv</span></div>
    <div class="c-bond">${c.graduated ? '<span class="grad">pool</span>' : `<b data-k="bd">${cut < 10 ? cut.toFixed(2) : cut.toFixed(1)}%</b>`}<i class="bar"><i style="width:${cut.toFixed(2)}%"></i></i></div>`;
}

/// A cell whose number changed since the last read flashes mint up, oxide down.
function flashCells(el, token, vals) {
  const prev = rowSnap.get(token);
  rowSnap.set(token, vals);
  if (!prev) return;
  for (const [k, v] of Object.entries(vals)) {
    if (prev[k] == null || v == null || prev[k] === v) continue;
    const cell = el.querySelector(`[data-k="${k}"]`);
    if (!cell) continue;
    cell.classList.remove('fl-up', 'fl-dn');
    void cell.offsetWidth;
    cell.classList.add(v > prev[k] ? 'fl-up' : 'fl-dn');
  }
}

function renderColumn(col, list) {
  const box = $(`#col-${col}`);
  const els = list.map((c) => {
    let el = rowEls.get(c.token);
    if (!el) {
      el = document.createElement('a');
      el.className = 'row';
      el.dataset.coin = c.token;
      el.href = withNet(`/coin/${c.token}`);
      if (boardPainted) el.classList.add('fresh');
      rowEls.set(c.token, el);
    }
    const html = rowHtml(c);
    if (el.jadeHtml !== html) { el.innerHTML = html; el.jadeHtml = html; }
    flashCells(el, c.token, { mc: mcSort(c), h1: change1h(c), h24: change24(c), vol: volSort(c), mv: moves24(c), bd: progress(c) });
    return el;
  });
  if (!els.length) {
    const any = curves.some((c) => columnOf(c) === col);
    const text = !marketRead ? 'Reading coins…'
      : any ? 'No coins match.'
        : col === 'graduated' ? 'No coin has graduated yet.'
          : col === 'bonding' ? 'No coins on their curve with trades yet.'
            : `No new coins. <a href="${withNet('/launch')}">Create one.</a>`;
    const html = `<div class="col-empty">${text}</div>`;
    if (box.jadeEmpty !== html) { box.innerHTML = html; box.jadeEmpty = html; }
    return;
  }
  box.jadeEmpty = '';
  const cur = [...box.children];
  if (els.length !== cur.length || els.some((e, i) => e !== cur[i])) box.replaceChildren(...els);
  tickAgo(box);
}

function renderBoard() {
  const by = { new: [], bonding: [], graduated: [] };
  for (const c of curves) if (matches(c)) by[columnOf(c)].push(c);
  for (const col of COLS) {
    const { key, dir } = colSort[col];
    by[col].sort((a, b) => (sortValue(a, key) - sortValue(b, key)) * dir || (sortValue(b, 'last') - sortValue(a, 'last')));
    renderColumn(col, by[col]);
    $(`#count-${col}`).textContent = by[col].length;
    $(`#count-tab-${col}`).textContent = by[col].length;
  }
  boardPainted = marketRead;
  $('#tally-all').textContent = curves.length;
  $('#tally-grad').textContent = curves.filter((c) => c.graduated).length;
  const bySym = (f) => curves.reduce((m, c) => m.set(qsym(c), (m.get(qsym(c)) || 0) + f(c)), new Map());
  $('#tally-zec').textContent = sumText(bySym((c) => num(zecSet(c))));
  $('#tally-vol').textContent = sumText(bySym(vol24));
  $('#tally-moves').textContent = curves.reduce((s, c) => s + moves24(c), 0);
  $('[data-filter="held"]').hidden = !mine;
  // Pictures for what is on screen, a few at a time.
  for (const c of curves.slice(0, 90)) {
    if (!content.has(c.token)) loadContent(c.token).then((k) => { if (k) { scheduleBoard(); renderTape(); } });
  }
}
let boardFrame = 0;
const scheduleBoard = () => { if (!boardFrame) boardFrame = requestAnimationFrame(() => { boardFrame = 0; if (pathRoute().view === 'board') renderBoard(); }); };

$$('[data-filter]').forEach((b) => b.addEventListener('click', () => {
  const f = b.dataset.filter;
  if (filters.has(f)) filters.delete(f); else filters.add(f);
  b.setAttribute('aria-pressed', String(filters.has(f)));
  renderBoard();
}));
$$('[data-col-tab]').forEach((b) => b.addEventListener('click', () => {
  $('#cols').dataset.show = b.dataset.colTab;
  $$('[data-col-tab]').forEach((x) => x.setAttribute('aria-selected', String(x === b)));
}));
search.addEventListener('input', () => {
  if (pathRoute().view !== 'board') navigate('/');
  else renderBoard();
});
search.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') { search.blur(); return; }
  if (event.key !== 'Enter') return;
  // Enter opens the first coin on screen that matches.
  const first = $$('.cols .row').find((r) => r.offsetParent);
  if (first) { event.preventDefault(); search.blur(); navigate(`/coin/${first.dataset.coin}`); }
});

// ---------------------------------------------------------------------------
// One coin.

let chart = null;
let candles = null;
let volume = null;
let chartRows = [];

function ensureChart() {
  if (chart) return;
  const el = $('#chart');
  const local = (t) => new Date(t * 1000);
  chart = createChart(el, {
    autoSize: true,
    layout: { background: { color: 'transparent' }, textColor: '#8a9691', fontFamily: 'Geist Mono, ui-monospace, monospace', fontSize: 11 },
    grid: { vertLines: { color: 'rgba(242,241,235,.035)' }, horzLines: { color: 'rgba(242,241,235,.045)' } },
    timeScale: { timeVisible: true, secondsVisible: false, borderColor: 'rgba(242,241,235,.09)',
      tickMarkFormatter: (t, type) => (type >= 3 ? local(t).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' }) : local(t).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })) },
    localization: { timeFormatter: (t) => local(t).toLocaleString('en-GB', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }) },
    rightPriceScale: { borderColor: 'rgba(242,241,235,.09)', scaleMargins: { top: 0.1, bottom: 0.22 } },
    crosshair: { mode: 0, vertLine: { color: 'rgba(88,232,187,.35)', labelBackgroundColor: '#173029' }, horzLine: { color: 'rgba(88,232,187,.35)', labelBackgroundColor: '#173029' } },
  });
  candles = chart.addCandlestickSeries({ upColor: '#35e6ae', downColor: '#e8794f', borderVisible: false, wickUpColor: '#35e6ae', wickDownColor: '#e8794f', priceFormat: { type: 'custom', formatter: (v) => capText(v / (unit === 'usd' && usdPer(currentCoin) ? usdPer(currentCoin) : 1)), minMove: 1e-9 },
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
  chart.subscribeCrosshairMove((param) => {
    const row = param?.time != null ? chartRows.find((r) => r.time === param.time) : null;
    legend(row);
  });
}

/// Open, high, low, close and volume of the candle under the cursor, or the last one.
function legend(row = chartRows[chartRows.length - 1]) {
  const el = $('#chart-legend');
  if (!row) { el.textContent = ''; return; }
  const factor = unit === 'usd' && usdPer(currentCoin) ? usdPer(currentCoin) : 1;
  const f = (v) => esc(capText(v / factor));
  const ch = pct(row.close, row.open);
  el.innerHTML = `<span class="muted">O</span> <b>${f(row.open)}</b><span class="muted">H</span> <b>${f(row.high)}</b><span class="muted">L</span> <b>${f(row.low)}</b><span class="muted">C</span> <b class="${changeCls(ch)}">${f(row.close)}</b><span class="muted">Vol</span> <b>${zecCell(row.vol)} ${esc(qsym(currentCoin))}</b>`;
}

/// Candles of market cap, one bucket per `tf` seconds, from the epoch points.
function drawChart() {
  if (!currentCoin) return;
  ensureChart();
  const factor = unit === 'usd' && usdPer(currentCoin) ? usdPer(currentCoin) : 1;
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
  chartRows = rows;
  candles.setData(rows.map(({ time, open, high, low, close }) => ({ time, open, high, low, close })));
  volume.setData(rows.map((r) => ({ time: r.time, value: r.vol, color: r.close >= r.open ? 'rgba(53,230,174,.22)' : 'rgba(232,121,79,.22)' })));
  // Few candles stay candle-sized; many fit the width.
  if (rows.length > 40) chart.timeScale().fitContent();
  else { chart.timeScale().applyOptions({ barSpacing: 14, rightOffset: 6 }); chart.timeScale().scrollToRealTime(); }
  legend();
}

function renderFills() {
  if (!currentCoin) return;
  const moves = historyOf(currentCoin.token).filter(moved).slice(-80).reverse();
  $('#fills').innerHTML = moves.length ? moves.map((p) => {
    const f = fillOf(p);
    return `<tr><td data-ago="${p.time}" data-ago-long="1">${ago(p.time)}</td><td class="muted">${p.epoch.toLocaleString('en-US')}</td><td><span class="${f.buy ? 'side-b' : 'side-s'}">${f.buy ? 'Buy' : 'Sell'}</span></td><td class="r">${f.tokens == null ? '—' : f.tokens.toLocaleString('en-US')}</td><td class="r ${f.buy ? 'up' : 'down'}">${wad(f.zecRaw, 5)}</td><td class="r">${priceText(num(p.price))}</td><td class="r">${esc(capText(mcapOf(p.price)))}</td></tr>`;
  }).join('') : '<tr><td colspan="7" class="empty">No fills yet. The first buyers all pay one price.</td></tr>';
}

/// The batch panel: what waits in this epoch, the last clearing, recent batches.
function renderBatch() {
  const c = currentCoin;
  const e = live.epoch;
  $('#batch-epoch').textContent = e != null ? `epoch ${e.toLocaleString('en-US')}` : 'epoch —';
  const box = $('#batch .pending');
  if (!c) {
    $('#batch-pending').textContent = '—';
    $('#batch-clear').textContent = '—';
    $('#batch-last').textContent = '—';
    $('#batch-spark').innerHTML = '';
    box.classList.remove('on');
    return;
  }
  const q = pendingOf(c);
  $('#batch-pending').textContent = q;
  $('#batch-pending-unit').textContent = q === 1 ? 'order' : 'orders';
  box.classList.toggle('on', q > 0);
  const fills = historyOf(c.token).filter(moved);
  const last = fills[fills.length - 1];
  if (last) {
    const f = fillOf(last);
    $('#batch-clear').innerHTML = `${priceText(num(last.price))} <span class="muted">· MC ${esc(capText(mcapOf(last.price), c))}</span>`;
    $('#batch-last').innerHTML = `<span class="${f.buy ? 'up' : 'down'}">${f.buy ? 'Buy' : 'Sell'} ${zecCell(f.zec)} ${esc(qsym(c))}</span> <span class="muted" data-ago="${last.time}" data-ago-long="1">${ago(last.time)}</span>`;
  } else {
    $('#batch-clear').textContent = 'no fills yet';
    $('#batch-last').textContent = '—';
  }
  // The last 36 batches that moved, as bars: height is the amount, colour is side.
  const recent = fills.slice(-36);
  const max = Math.max(...recent.map((p) => fillOf(p).zec), 0);
  const spark = $('#batch-spark');
  const html = recent.length ? recent.map((p) => { const f = fillOf(p); return `<i class="${f.buy ? '' : 's'}" style="height:${Math.max(8, (f.zec / (max || 1)) * 100).toFixed(0)}%" title="epoch ${p.epoch}: ${f.buy ? 'buy' : 'sell'} ${zecCell(f.zec)} ${esc(qsym(c))}"></i>`; }).join('') : '<span class="spark-empty">No batches yet</span>';
  if (spark.jadeHtml !== html) { spark.innerHTML = html; spark.jadeHtml = html; }
  tickRing();
}

/// The countdown to the seal. Measured: the moment the page saw the epoch
/// change, plus the cadence it measured (or the market's epoch length until
/// it has). Before that, an estimate from the last fill's time, marked "~".
function sealIn() {
  const e = live.epoch;
  if (e == null) return null;
  const cadence = live.cadence || epochSeconds;
  if (live.changedAt) return { left: live.changedAt / 1000 + cadence - Date.now() / 1000, cadence, sure: true };
  let lastPt = null;
  for (const [, h] of histories) { const p = h[h.length - 1]; if (p && (!lastPt || p.epoch > lastPt.epoch)) lastPt = p; }
  if (lastPt && e - lastPt.epoch <= 5 && e > lastPt.epoch) {
    const left = lastPt.time + (e - lastPt.epoch) * cadence - Date.now() / 1000;
    if (left > -2 && left < cadence + 2) return { left, cadence, sure: false };
  }
  return null;
}
function tickRing() {
  if (pathRoute().view !== 'coin') return;
  const ring = $('#ring');
  const s = sealIn();
  const fg = $('#ring-fg');
  if (!s) {
    ring.className = 'ring wait';
    fg.setAttribute('stroke-dashoffset', '0');
    $('#ring-secs').textContent = '…';
    $('#ring-label').textContent = 'timing';
    return;
  }
  const left = Math.max(0, s.left);
  const done = Math.min(1, Math.max(0, 1 - left / s.cadence));
  ring.className = `ring${left <= 0.5 ? ' sealing' : ''}`;
  fg.setAttribute('stroke-dashoffset', String((100 - done * 100).toFixed(2)));
  $('#ring-secs').textContent = left <= 0.5 ? '0s' : `${s.sure ? '' : '~'}${Math.ceil(left)}s`;
  $('#ring-label').textContent = left <= 0.5 ? 'sealing' : 'to seal';
}

/// What the page knows of the epoch. `timed` is true only for the 1 s poll,
/// whose changes are close enough to the seal to time it by.
function noteEpoch(e, timed = true) {
  if (!Number.isSafeInteger(e)) return;
  const t = Date.now();
  // The 1 s poll times the seal; a slower read must not reset its clock.
  if (!timed && live.seen && t - live.seen < 2500) return;
  if (live.epoch != null && e > live.epoch) {
    if (timed && live.changedAt && live.seen && t - live.seen < 2500) {
      const per = (t - live.changedAt) / 1000 / (e - live.epoch);
      if (per > 2 && per < 900) live.cadence = live.cadence ? live.cadence * 0.6 + per * 0.4 : per;
    }
    live.changedAt = timed && live.seen && t - live.seen < 2500 ? t : 0;
    sealed(e);
  }
  if (live.epoch == null || e >= live.epoch) live.epoch = e;
  if (timed) live.seen = t;
  renderEpoch();
}
function renderEpoch() {
  $('#epoch-text').textContent = live.epoch != null ? `Epoch ${live.epoch.toLocaleString('en-US')}` : 'Epoch —';
  $('#epoch-pill').classList.toggle('stale', live.epoch == null);
}
/// An epoch sealed while a coin is open: read its batch as soon as it is out.
let sealTimer = 0;
function sealed() {
  if (pathRoute().view !== 'coin') return;
  clearTimeout(sealTimer);
  sealTimer = setTimeout(refreshCoin, 1500);
}

/// Markup, written only when it changed (a rewrite would close an open bubble
/// or reload a picture).
function setHtml(id, html) {
  const el = $(`#${id}`);
  if (el.jadeHtml !== html) { el.innerHTML = html; el.jadeHtml = html; tickAgo(el); }
  return el;
}
function setText(id, text, cls) {
  const el = $(`#${id}`);
  if (el.textContent !== text) el.textContent = text;
  if (cls !== undefined) el.className = cls;
  return el;
}
const coinSnap = new Map();
function flashStats(token, vals) {
  const prev = coinSnap.get(token);
  coinSnap.set(token, vals);
  if (!prev) return;
  for (const [id, v] of Object.entries(vals)) {
    if (prev[id] == null || prev[id] === v) continue;
    const el = $(`#${id}`);
    el.classList.remove('fl-up', 'fl-dn'); void el.offsetWidth; el.classList.add(v > prev[id] ? 'fl-up' : 'fl-dn');
  }
}

async function renderCoin(token, fresh = false) {
  const c = curves.find((x) => x.token === token);
  if (!c) {
    currentCoin = null;
    $('#coin-title').textContent = curves.length ? 'Coin not found' : 'Reading coins…';
    // Nothing of the last coin stays on screen under this one's address.
    for (const id of ['coin-symbol', 'coin-age', 'coin-stage', 'coin-copy', 'coin-mcap', 'coin-change', 'coin-ch5', 'coin-ch1', 'coin-vol', 'coin-moves', 'coin-price', 'coin-liquidity', 'coin-pot', 'coin-fee', 'coin-holders', 'coin-addr', 'curve-pct', 'curve-pct-2', 'curve-text', 'trade-output', 'trade-fee', 'trade-min', 'trade-balance']) { const el = $(`#${id}`); if (el) { el.textContent = '—'; el.jadeHtml = null; } }
    $('#coin-mcap-usd').textContent = '';
    setHtml('coin-thumb', '<span>?</span>');
    $('#curve-bar').style.width = '0%';
    $('#coin-description').textContent = curves.length
      ? `No coin has this address on ${NETWORK}.${NETWORK === 'mainnet' ? ' A testnet coin? Add ?network=testnet to the address.' : ''}`
      : '';
    $('#fills').innerHTML = '';
    $('#position').innerHTML = '';
    $('#trade-status').textContent = '';
    $('#trade-steps').dataset.step = 'idle';
    $('.trade-submit').textContent = wallet ? 'Not available' : 'Connect wallet';
    if (chart) { candles.setData([]); volume.setData([]); chartRows = []; legend(); }
    renderBatch();
    // A missing coin opens on About, where it says why.
    if (curves.length) showPane('about');
    return;
  }
  const first = currentCoin?.token !== token;
  currentCoin = c;
  if (first) {
    // A new coin starts with a clean form, and its own last order status.
    tradeSide = 'buy';
    tradeVia = null;
    $$('[data-trade]').forEach((item) => item.classList.toggle('active', item.dataset.trade === 'buy'));
    $('#trade-input').value = presetsFor(c)[2];
    showOrder(token);
    if (chart) { candles.setData([]); volume.setData([]); chartRows = []; }
    showPane(paneNow);
  }
  const s = stats.get(token) || {};
  const mc = mcap(c);
  const q = qsym(c), per = usdPer(c);
  setText('unit-quote', q);
  $('[data-unit="usd"]').hidden = !per;
  setText('coin-title', c.name);
  setText('coin-symbol', c.symbol);
  setHtml('coin-age', s.created_time ? `made <span data-ago="${s.created_time}" data-ago-long="1">${ago(s.created_time)}</span>` : `made in epoch ${c.created_at}`);
  if (!/copied/.test($('#coin-copy').textContent)) setText('coin-copy', short(c.token, 4));
  $('#coin-description').textContent ||= 'No description.';
  setText('coin-stage', c.graduated ? 'Graduated' : 'Bonding curve', `stage${c.graduated ? ' grad' : ''}`);
  setHtml('coin-price', `${priceText(num(price(c)))} ${esc(q)}`);
  setText('coin-mcap', `${zecText(mc)} ${q}`);
  setText('coin-mcap-usd', per ? usdText(mc * per) : '');
  const ch5 = changeSince(c, 300), ch1 = change1h(c), ch = change24(c);
  setText('coin-ch5', changeText(ch5), changeCls(ch5));
  setText('coin-ch1', changeText(ch1), changeCls(ch1));
  setText('coin-change', changeText(ch), changeCls(ch));
  setText('coin-vol', `${zecCell(vol24(c))} ${q}`);
  setText('coin-moves', String(moves24(c)));
  setText('coin-liq-label', c.graduated ? 'In pool' : 'In curve');
  setText('coin-liquidity', `${wad(zecSet(c), 4)} ${q}`);
  setHtml('coin-holders', `private${tip('Zyn does not publish who holds a coin or how much: balances are private to each wallet.', 'About holders')}`);
  $('#coin-pot').textContent = c.graduated ? 'in its pool' : `${wad(c.graduation, 6)} ${q}.zy`;
  $('#coin-fee').textContent = c.graduated ? `${poolOf(c)?.fee_bps ?? 30} bps, kept by the pool` : c.fee_bps ? `${c.fee_bps / 100}% a trade` : 'none';
  $('#coin-addr').textContent = c.token;
  const cut = progress(c);
  setText('curve-pct', `${(cut * 100).toFixed(2)}%`);
  setText('curve-pct-2', `${(cut * 100).toFixed(2)}%`);
  $('#curve-bar').style.width = `${(cut * 100).toFixed(2)}%`;
  $('#curve-bar-2').style.width = `${(cut * 100).toFixed(2)}%`;
  $('#curve-text').textContent = c.graduated
    ? 'Graduated. Every unsold lot and everything the curve raised opened a pool whose shares nobody holds.'
    : `${wad(c.reserve, 4)} ${q}.zy in the curve · ${wad(cost(c.sold, CURVE.graduateAt - c.sold, unitOf(c)), 3)} ${q}.zy of buying to graduate · it graduates at ~${zecText(gradMc(unitOf(c)))} ${q} market cap${per ? ` (${usdText(gradMc(unitOf(c)) * per)})` : ''}`;
  $('#chart-point').setAttribute('cx', String(54 + 816 * cut));
  $('#chart-point').setAttribute('cy', String(342 - 278 * cut));
  flashStats(token, { 'coin-price': Number(price(c)), 'coin-mcap': mc, 'coin-vol': vol24(c), 'coin-moves': moves24(c), 'curve-pct': cut, 'coin-liquidity': Number(zecSet(c)) });
  document.title = `${c.symbol} ${capText(mc)} — Jade`;
  renderBatch();
  renderPosition();
  const k = await loadContent(token);
  if (currentCoin?.token !== token) return;
  setHtml('coin-thumb', thumbInner(c));
  $('#coin-description').textContent = k && k !== 'loading' && k.description ? k.description : 'No description.';
  if (first || fresh || !histories.has(token)) {
    await loadHistory(token);
    if (currentCoin?.token !== token) return;
  }
  setText('coin-ch5', changeText(changeSince(c, 300)), changeCls(changeSince(c, 300)));
  drawChart();
  renderFills();
  renderBatch();
  configureTrade();
}

/// A coin view's own refresh: the market, and its fills when it has traded.
let coinBusy = false;
async function refreshCoin() {
  const route = pathRoute();
  if (route.view !== 'coin' || coinBusy) return;
  coinBusy = true;
  try {
    await loadMarket();
    const read = historyRead.get(route.token);
    const last = stats.get(route.token)?.last_trade_time || 0;
    if (!read || read.last !== last || Date.now() - read.at > 30_000) await loadHistory(route.token);
    if (currentCoin?.token !== route.token || pathRoute().token !== route.token) return;
    drawChart();
    renderFills();
    renderBatch();
    renderTape();
  } finally { coinBusy = false; }
}

let paneNow = 'batches';
function showPane(name) {
  paneNow = name;
  $$('[data-pane]').forEach((b) => b.setAttribute('aria-selected', String(b.dataset.pane === name)));
  $$('[data-pane-body]').forEach((p) => { p.hidden = p.dataset.paneBody !== name; });
}
$$('[data-pane]').forEach((b) => b.addEventListener('click', () => showPane(b.dataset.pane)));

$('#coin-copy').addEventListener('click', async () => {
  if (!currentCoin) return;
  try { await navigator.clipboard.writeText(currentCoin.token); $('#coin-copy').textContent = 'copied'; setTimeout(() => { if (currentCoin) $('#coin-copy').textContent = short(currentCoin.token, 4); }, 1200); } catch { /* the address is on screen */ }
});
$$('[data-tf]').forEach((b) => b.addEventListener('click', () => { tf = Number(b.dataset.tf); $$('[data-tf]').forEach((x) => x.classList.toggle('active', x === b)); drawChart(); }));
$$('[data-unit]').forEach((b) => b.addEventListener('click', () => { unit = b.dataset.unit; $$('[data-unit]').forEach((x) => x.classList.toggle('active', x === b)); render(); }));

// ---------------------------------------------------------------------------
// Your position, and the average cost of what this browser bought.

const heldOf = (token) => (mine?.holdings.get(token) ?? 0n);
const costKey = () => (wallet ? `jade.cost.${NETWORK}.${wallet.session.account}` : null);
function costBook() {
  const k = costKey();
  if (!k) return {};
  try { return JSON.parse(localStorage.getItem(k) || '{}') || {}; } catch { return {}; }
}
/// A settled order, as this browser saw it: tokens in or out, ZEC.zy out or in.
/// A sale takes its share of the cost out, so the average of what is left stays.
function recordFill(token, gotTokens, spentZec) {
  const k = costKey();
  if (!k || gotTokens === 0n) return;
  const book = costBook();
  const e = book[token] || { tokens: '0', zec: '0' };
  let t = BigInt(e.tokens), z = BigInt(e.zec);
  if (gotTokens > 0n && spentZec > 0n) { t += gotTokens; z += spentZec; }
  else if (gotTokens < 0n && t > 0n) { const out = -gotTokens > t ? t : -gotTokens; z -= (z * out) / t; t -= out; }
  book[token] = { tokens: t.toString(), zec: z.toString() };
  try { localStorage.setItem(k, JSON.stringify(book)); } catch { /* this tab only */ }
}
function avgCost(token) {
  const e = costBook()[token];
  if (!e) return null;
  const t = BigInt(e.tokens), z = BigInt(e.zec);
  return t > 0n && z > 0n ? { perToken: (z * WAD) / t, tokens: t, zec: z } : null;
}

function renderPosition() {
  const box = $('#position');
  const c = currentCoin;
  if (!c) { box.innerHTML = ''; return; }
  if (!wallet || !mine) {
    box.innerHTML = `<div class="note">${wallet ? "Nap's window is closed. Connect again with the wallet button to read your holdings." : 'Connect a wallet to see your position.'}</div>`;
    return;
  }
  const held = heldOf(c.token);
  const worth = num((price(c) * held) / WAD);
  const ac = avgCost(c.token);
  const ch = change24(c);
  const cells = [
    ['Holding', `${wad(held, 0)} ${esc(c.symbol)}`],
    ['Value', `${zecText(worth)} ${esc(qsym(c))}${usdPer(c) ? ` <span class="muted">${usdText(worth * usdPer(c))}</span>` : ''}`],
    ['24h', `<span class="${changeCls(ch)}">${changeText(ch)}</span>`],
    ['Share of supply', `${((Number(held / WAD) / 1e9) * 100).toFixed(4)}%`],
    ['Sellable lots', c.graduated ? 'any amount' : (held / LOT).toLocaleString('en-US')],
    [`${esc(qsym(c))}.zy free`, balanceOf(c.quote) == null ? '—' : `${wad(balanceOf(c.quote), 4)}`],
  ];
  if (ac && held > 0n) {
    const tracked = ac.tokens < held ? ac.tokens : held;
    const pnl = num((price(c) * tracked) / WAD) - num((ac.perToken * tracked) / WAD);
    cells.push(['Avg cost', `${priceText(num(ac.perToken))} ${esc(qsym(c))}`], ['P/L, tracked', `<span class="${changeCls(pnl)}">${pnl >= 0 ? '+' : '−'}${zecText(Math.abs(pnl))} ${esc(qsym(c))}</span>`]);
  }
  box.innerHTML = cells.map(([k, v]) => `<div><dt>${k}</dt><dd>${v}</dd></div>`).join('')
    + `<div class="note">${ac ? 'Average cost counts only trades placed and settled in this browser.' : 'Average cost appears after a trade placed here settles; Zyn does not publish your fills.'}</div>`;
}

// ---------------------------------------------------------------------------
// Trading.

const tradeInput = $('#trade-input');

// ---------------------------------------------------------------------------
// Routes (§167): pay for a coin with another bridged coin, or sell it for one,
// through the pools between bridged coins — one order, one epoch, all or nothing.

/// What a buy pays with or a sale receives: the coin's quote unless another is picked.
let tradeVia = null;
const viaOf = (c) => (tradeVia && tradeVia !== c.quote ? tradeVia : c.quote);
const venues = {
  pool: (id) => pools.find((p) => p.id === id),
  curve: (t) => curves.find((c) => c.token === t),
};
/// Pools between two bridged coins, with something in them.
const basePools = () => pools.filter((p) => QUOTE_ASSETS[p.a] && QUOTE_ASSETS[p.b] && BigInt(p.reserve_a) > 0n && BigInt(p.reserve_b) > 0n);
/// Every way from `from` to `to` through at most `max` base pools, as hops.
function poolPaths(from, to, max) {
  const out = [];
  const walk = (at, hops, seen) => {
    if (at === to && hops.length) { out.push(hops); return; }
    if (hops.length === max) return;
    for (const p of basePools()) {
      if (seen.has(p.id) || (p.a !== at && p.b !== at)) continue;
      const next = p.a === at ? p.b : p.a;
      walk(next, [...hops, { pool: p.id, sellA: p.a === at }], new Set([...seen, p.id]));
    }
  };
  walk(from, [], new Set());
  return out;
}
/// The coin's own venue: its curve, or its pool once it has graduated.
function coinHop(c, buy) {
  if (!c.graduated) return buy ? { curveBuy: c.token } : { curveSell: c.token };
  const p = poolOf(c);
  return p ? { pool: p.id, sellA: buy ? p.a === c.quote : p.a === c.token } : null;
}
/// The best route for this coin, side and coin to pay or receive, for this
/// amount: the one that delivers most if alone in its batches.
function bestRoute(c, buy, via, amount) {
  const own = coinHop(c, buy);
  if (!own) return null;
  const legs = buy ? poolPaths(via, c.quote, 2).map((p) => [...p, own]) : poolPaths(c.quote, via, 2).map((p) => [own, ...p]);
  let best = null;
  for (const hops of legs.filter((h) => h.length <= 4)) {
    const q = previewRoute(hops, amount, venues);
    if (q && (!best || q.out > best.q.out)) best = { hops, q };
  }
  return best;
}
/// The coins a buy can pay with (or a sale receive): the quote, and every
/// launch coin a route reaches.
function viaChoices(c, buy) {
  const own = coinHop(c, buy);
  const reach = (a) => a === c.quote || (own && (buy ? poolPaths(a, c.quote, 2) : poolPaths(c.quote, a, 2)).length > 0);
  return [...new Set([c.quote, ...quotes.map((q) => q.asset)])].filter(reach);
}
const pathText = (c, hops, buy) => {
  const assets = [buy ? viaOf(c) : c.token];
  for (const h of hops) {
    if (h.curveBuy) assets.push(c.token);
    else if (h.curveSell) assets.push(c.quote);
    else { const p = venues.pool(h.pool); assets.push(h.sellA ? p.b : p.a); }
  }
  return assets.map((a) => (a === c.token ? c.symbol : symOf(a))).join(' → ');
};

/// Buy presets sized to the coin it is paid with: 0.01, 0.05, 0.1 and 0.5
/// ZEC's worth, to two significant figures.
function presetsFor(c) {
  const via = viaOf(c);
  const unit = via === c.quote ? unitOf(c) : (quotes.find((q) => q.asset === via)?.unit ?? ZAT);
  const scale = Number(unit) / Number(ZAT);
  const nice = (v) => { const p = 10 ** (Math.floor(Math.log10(v)) - 1); return String(Number((Math.round(v / p) * p).toPrecision(2))); };
  return [0.01, 0.05, 0.1, 0.5].map((v) => nice(v * scale));
}

function configureTrade() {
  const c = currentCoin;
  if (!c) return;
  const buying = tradeSide === 'buy';
  const choices = viaChoices(c, buying);
  if (!choices.includes(viaOf(c))) tradeVia = null;
  const via = viaOf(c);
  $('#trade-via-row').hidden = choices.length < 2;
  $('#trade-via-label').textContent = buying ? 'Pay with' : 'Receive';
  const viaHtml = choices.map((a) => `<button type="button" data-via="${a}" class="${a === via ? 'active' : ''}">${esc(symOf(a))}</button>`).join('');
  const viaBox = $('#trade-via');
  if (viaBox.jadeHtml !== viaHtml) { viaBox.innerHTML = viaHtml; viaBox.jadeHtml = viaHtml; }
  $('#trade-unit').textContent = buying ? `${symOf(via)}.zy` : c.symbol;
  const presets = buying
    ? presetsFor(c).map((v) => `<button type="button" data-preset="${v}">${v}</button>`)
    : [25, 50, 100].map((p) => `<button type="button" data-pct="${p}">${p}%</button>`);
  const box = $('#trade-presets');
  const html = presets.join('');
  if (box.jadeHtml !== html) {
    box.innerHTML = html;
    box.jadeHtml = html;
    box.classList.toggle('three', !buying);
    $$('[data-preset]').forEach((b) => b.addEventListener('click', () => { tradeInput.value = b.dataset.preset; quote(); }));
    $$('[data-pct]').forEach((b) => b.addEventListener('click', () => {
      const cc = currentCoin;
      const lotsHeld = heldOf(cc.token) / LOT;
      let lots = cc.graduated ? heldOf(cc.token) * BigInt(b.dataset.pct) / 100n : (lotsHeld * BigInt(b.dataset.pct)) / 100n * LOT;
      // A share of a few lots can round to none: then the smallest sale there is.
      if (!cc.graduated && lots === 0n && lotsHeld > 0n) lots = LOT;
      tradeInput.value = decimal(lots);
      quote();
    }));
  }
  const bal = balanceOf(via);
  $('#trade-balance').textContent = mine
    ? `You hold ${wad(heldOf(c.token), 0)} ${c.symbol}${bal == null ? '' : ` · ${wad(bal, 4)} ${symOf(via)}.zy`}`
    : 'Connect a wallet to see your balance.';
  const submit = $('.trade-submit');
  submit.textContent = wallet ? (buying ? `Buy ${c.symbol}` : `Sell ${c.symbol}`) : 'Connect wallet';
  submit.classList.toggle('sell', !!wallet && !buying);
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
  $$('#trade-presets [data-preset]').forEach((b) => b.classList.toggle('picked', b.dataset.preset === tradeInput.value.trim()));
  if (!c) return null;
  if (wallet) submit.disabled = true;
  const slip = BigInt($('#trade-slip').value);
  $('#trade-route-row').hidden = true;
  try {
    const value = parseDecimal($('#trade-input').value);
    if (value <= 0n) throw new Error('Enter an amount above zero');
    const via = viaOf(c);
    if (via !== c.quote) return quoteRoute(c, value, via, slip);
    const q$ = `${qsym(c)}.zy`;
    const bal = balanceOf(c.quote);
    if (tradeSide === 'buy' && bal != null && value > bal) throw new Error(`You have ${full(bal)} ${q$}`);
    if (c.graduated) {
      const p = poolOf(c);
      if (!p) throw new Error('Its pool is not visible yet');
      const sellA = tradeSide === 'buy' ? p.a === c.quote : p.a === c.token;
      if (tradeSide === 'sell' && mine && value > heldOf(c.token)) throw new Error(`You hold ${full(heldOf(c.token))}`);
      const q = poolQuote(p, sellA, value);
      if (q <= 0n) throw new Error('Too small to trade');
      out.textContent = `${full(q)} ${tradeSide === 'buy' ? c.symbol : q$}`;
      fee.textContent = `${p.fee_bps / 100}% kept by the pool`;
      min.textContent = `${full((q * (BPS - slip)) / BPS)}`;
      if (wallet) submit.disabled = false;
      return { kind: 'swap', pool: p.id, sellA, amount: value, slippageBps: Number(slip) };
    }
    if (tradeSide === 'buy') {
      const least = 100_000n * unitOf(c);
      if (value < least) throw new Error(`A buy is at least ${full(least)} ${q$}`);
      const q = previewBuy(c.sold, value, c.fee_bps, unitOf(c));
      if (!q.lots) throw new Error('That buys less than one lot of 12,500');
      // At least one lot: a floor of none would be no floor, and Nap refuses it.
      const floorLots = (BigInt(q.lots) * (BPS - slip)) / BPS;
      const minTokens = (floorLots > 0n ? floorLots : 1n) * LOT;
      out.textContent = `${wad(q.tokens, 0)} ${c.symbol}`;
      fee.textContent = c.fee_bps ? `${full(q.fee)} ${q$}` : 'none';
      min.textContent = `${wad(minTokens, 0)} ${c.symbol}`;
      if (wallet) submit.disabled = false;
      return { kind: 'buy', amount: value, minTokens };
    }
    if (value % LOT !== 0n) throw new Error(`Sell whole lots of ${CURVE.lotTokens.toLocaleString('en-US')}`);
    if (mine && value > heldOf(c.token)) throw new Error(`You hold ${wad(heldOf(c.token), 0)}`);
    const q = previewSell(c.sold, Number(value / LOT), c.fee_bps, unitOf(c));
    const minZec = (q.net * (BPS - slip)) / BPS;
    if (minZec <= 0n) throw new Error('Too small to sell');
    out.textContent = `${full(q.net)} ${q$}`;
    fee.textContent = c.fee_bps ? `${full(q.fee)} ${q$}` : 'none';
    min.textContent = `${full(minZec)} ${q$}`;
    if (wallet) submit.disabled = false;
    return { kind: 'sell', tokens: value, minZec };
  } catch (e) {
    hint.textContent = e.message;
    return null;
  }
}

/// A route's quote: what it delivers alone, its floor, and its path.
function quoteRoute(c, value, via, slip) {
  const out = $('#trade-output'), fee = $('#trade-fee'), min = $('#trade-min');
  const buy = tradeSide === 'buy';
  const v$ = `${symOf(via)}.zy`;
  if (buy) {
    const bal = balanceOf(via);
    if (bal != null && value > bal) throw new Error(`You have ${full(bal)} ${v$}`);
  } else {
    if (!c.graduated && value % LOT !== 0n) throw new Error(`Sell whole lots of ${CURVE.lotTokens.toLocaleString('en-US')}`);
    if (mine && value > heldOf(c.token)) throw new Error(`You hold ${wad(heldOf(c.token), 0)}`);
  }
  const r = bestRoute(c, buy, via, value);
  if (!r) throw new Error(`No route to ${buy ? c.symbol : v$} at these prices`);
  const toCurve = buy && !c.graduated;
  // A curve delivers whole lots: the floor is whole lots too, at least one.
  const floor = toCurve
    ? (() => { const lots = (r.q.out / LOT * (BPS - slip)) / BPS; return (lots > 0n ? lots : 1n) * LOT; })()
    : (r.q.out * (BPS - slip)) / BPS;
  if (floor <= 0n) throw new Error('Too small to route');
  out.textContent = buy ? `${wad(r.q.out, 0)} ${c.symbol}` : `${full(r.q.out)} ${v$}`;
  const pools = r.hops.filter((h) => h.pool).map((h) => venues.pool(h.pool).fee_bps / 100);
  fee.textContent = [!c.graduated && c.fee_bps ? `${c.fee_bps / 100}% on the curve` : null, ...pools.map((f) => `${f}% pool`)].filter(Boolean).join(' + ') || 'none';
  $('#trade-route').textContent = `${pathText(c, r.hops, buy)} · one epoch`;
  $('#trade-route-row').hidden = false;
  min.textContent = buy ? `${wad(floor, 0)} ${c.symbol}` : `${full(floor)} ${v$}`;
  if (wallet) $('.trade-submit').disabled = false;
  return { kind: 'route', hops: r.hops, amount: value, minOut: floor, via };
}

$('#trade-via').addEventListener('click', (event) => {
  const b = event.target.closest('[data-via]');
  if (!b || !currentCoin) return;
  tradeVia = b.dataset.via === currentCoin.quote ? null : b.dataset.via;
  if (tradeSide === 'buy') $('#trade-input').value = presetsFor(currentCoin)[2];
  $('#trade-presets').jadeHtml = null;
  configureTrade();
});

function setSide(side) {
  tradeSide = side;
  $$('[data-trade]').forEach((item) => item.classList.toggle('active', item.dataset.trade === side));
  $('#trade-input').value = tradeSide === 'buy' && currentCoin ? presetsFor(currentCoin)[2] : '';
  configureTrade();
}
$$('[data-trade]').forEach((button) => button.addEventListener('click', () => setSide(button.dataset.trade)));
$('#trade-input').addEventListener('input', quote);
$$('[data-slip]').forEach((b) => b.addEventListener('click', () => {
  $('#trade-slip').value = b.dataset.slip;
  $$('[data-slip]').forEach((x) => x.classList.toggle('active', x === b));
  quote();
}));
$$('[data-jump]').forEach((b) => b.addEventListener('click', () => {
  setSide(b.dataset.jump);
  $('#trade').scrollIntoView({ behavior: 'smooth', block: 'center' });
}));

/// After an order: wait for its epoch to seal, collect what it was paid, and
/// say what arrived. Collecting needs no approval — it only moves what the
/// ledger owes this wallet into it.
async function settle(placedEpoch, token, side, symbol, quote, quoteBefore = null, tracked = true) {
  const q$ = `${symOf(quote)}.zy`;
  const before = heldOf(token);
  let sealed = false;
  // The coin view's 1 s poll, while it is fresh; otherwise the chain every 3 s.
  for (let i = 0; i < 360 && !sealed; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    let epoch = live.epoch != null && Date.now() - live.seen < 2500 ? live.epoch : null;
    if (epoch == null && i % 3 === 2) { try { epoch = (await rpc('zyn_status')).epoch; noteEpoch(epoch, false); } catch { continue; } }
    if (epoch != null && epoch > placedEpoch) sealed = true;
  }
  if (!sealed) {
    setOrder(token, `Still waiting for epoch ${placedEpoch} to seal. Your order is safe; it settles when it does.`, 'sealing');
    return;
  }
  try {
    if (wallet && (!wallet.provider.isNapWeb || wallet.provider.isOpen())) {
      await ask('zyn_collect', {});
      await loadMine();
      const got = heldOf(token) - before;
      const after = balanceOf(quote);
      // Average cost counts trades in the coin's own quote only.
      if (tracked && got !== 0n && quoteBefore != null && after != null) recordFill(token, got, quoteBefore - after);
      setOrder(token, got > 0n ? `Filled: +${wad(got, 0)} ${symbol}. It is in your wallet.`
        : got < 0n ? `Filled: sold. The ${q$} is in your wallet.`
          : side === 'buy' ? `Not filled — the price moved past your floor, so your ${q$} came back to your wallet.`
            : `Not filled — the price moved past your floor, so your ${symbol} came back to your wallet.`, got !== 0n ? 'done' : 'missed');
    } else {
      setOrder(token, "Settled. Open Nap's window to collect it.", 'done');
    }
  } catch (e) { setOrder(token, e.message, 'error'); }
  await loadMarket();
}

async function placeOrder() {
  const c = currentCoin;
  if (!c) return;
  if (!wallet) return pickWallet();
  const q = quote();
  if (!q) return;
  const button = $('.trade-submit');
  if (button.disabled) return;
  button.disabled = true;
  const side = tradeSide, symbol = c.symbol;
  // What it pays with or receives: the quote, or another coin by a route.
  const via = q.kind === 'route' ? q.via : c.quote;
  const quoteBefore = balanceOf(via);
  try {
    // Asked before anything is awaited: Nap may need to open its window.
    const pending = q.kind === 'route' ? ask('zyn_route', { hops: q.hops, amount: decimal(q.amount), minOut: decimal(q.minOut) })
      : q.kind === 'swap' ? ask('zyn_swap', { pool: q.pool, sellA: q.sellA, amount: decimal(q.amount), slippageBps: q.slippageBps })
      : q.kind === 'buy' ? ask('zyn_curveBuy', { token: c.token, amount: decimal(q.amount), minTokens: decimal(q.minTokens) })
        : ask('zyn_curveSell', { token: c.token, tokens: decimal(q.tokens), minZec: decimal(q.minZec) });
    await pending;
    const epoch = (await rpc('zyn_status')).epoch;
    setOrder(c.token, `Placed. Settling when epoch ${epoch} seals…`, 'sealing');
    settle(epoch, c.token, side, symbol, via, quoteBefore, via === c.quote);
  } catch (e) {
    // 4001 is Nap's "no"; anything else says what happened in its own words.
    setOrder(c.token, e.code === 4001 ? 'Not placed. Nothing left your wallet.' : e.message, e.code === 4001 ? 'idle' : 'error');
  } finally {
    button.disabled = false;
    quote();
  }
}
$('.trade-submit').addEventListener('click', placeOrder);

// Keys: / search, B buy, S sell, Enter place, Esc clear.
document.addEventListener('keydown', (event) => {
  if (event.metaKey || event.ctrlKey || event.altKey || event.isComposing) return;
  const el = document.activeElement;
  const typing = el && (/^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || el.isContentEditable);
  if (event.key === '/' && !typing) {
    event.preventDefault();
    search.focus();
    search.select();
    return;
  }
  if ($('#jade-wallet').open || pathRoute().view !== 'coin' || !currentCoin) return;
  if (event.key === 'Escape') {
    if (el === tradeInput) { tradeInput.value = ''; quote(); tradeInput.blur(); }
    return;
  }
  if (typing && el !== tradeInput) return;
  const k = event.key.toLowerCase();
  if (!typing && (k === 'b' || k === 's')) {
    event.preventDefault();
    if (tradeSide !== (k === 'b' ? 'buy' : 'sell')) setSide(k === 'b' ? 'buy' : 'sell');
    tradeInput.focus();
    tradeInput.select();
    return;
  }
  if (event.key === 'Enter' && (el === tradeInput || el === document.body || !el)) {
    event.preventDefault();
    placeOrder();
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
    + (list.some((w) => !w.provider?.isNapWeb) ? '' : `<p class="jade-wallet-get"><a href="${NAP_EXTENSION}" rel="noopener">Get Nap for Chrome</a> · <a href="${NAP_RELEASE}" target="_blank" rel="noopener">how to install</a>${tip('Download the zip and unzip it. Open chrome://extensions, turn on Developer mode, click Load unpacked and choose the folder. Works in Chrome, Brave, Edge and Arc.')}</p>`)
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
      // A Nap from before §166 says only ZEC.zy.
      quotes: Array.isArray(r?.quotes) ? new Map(r.quotes.filter((q) => isHex(q.asset) && isInt(q.amount)).map((q) => [q.asset, BigInt(q.amount)])) : null,
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

const coinCell = (c) => `<a class="coin-cell" href="${withNet(`/coin/${c.token}`)}">${thumb(c)}<span>${esc(c.name)}</span> <span class="tk">${esc(c.symbol)}</span></a>`;
const bondCell = (c) => (c.graduated ? '<span class="up">pool</span>' : `${(progress(c) * 100).toFixed(2)}%<i class="bar"><i style="width:${(progress(c) * 100).toFixed(2)}%"></i></i>`);

function renderPortfolio() {
  if (!wallet || !mine) {
    $('#portfolio-status').textContent = wallet ? "Nap's window is closed. Connect again with the wallet button to read your holdings." : 'Connect a wallet to see what you hold and what you made.';
    $('#held').innerHTML = '<tr><td colspan="8" class="empty">No wallet connected.</td></tr>';
    $('#made').innerHTML = '<tr><td colspan="7" class="empty">No wallet connected.</td></tr>';
    for (const id of ['held-total', 'pf-zec', 'pf-24h', 'pf-count']) $(`#${id}`).textContent = '—';
    $('#pf-24h').className = '';
    return;
  }
  const free = quotes.map((q) => [q.symbol, balanceOf(q.asset)]).filter(([, v]) => v != null);
  $('#portfolio-status').textContent = `${free.map(([s, v]) => `${wad(v, 4)} ${s}.zy`).join(' · ') || '—'} ready to trade.`;
  const totals = new Map(), befores = new Map();
  let count = 0;
  const rows = [...mine.holdings].filter(([, amount]) => amount > 0n).map(([token, amount]) => {
    const c = curves.find((x) => x.token === token);
    if (!c) return '';
    count++;
    const worth = num((price(c) * amount) / WAD);
    const ch = change24(c);
    const sym = qsym(c);
    totals.set(sym, (totals.get(sym) || 0) + worth);
    befores.set(sym, (befores.get(sym) || 0) + (ch == null ? worth : worth / (1 + ch / 100)));
    const ac = avgCost(token);
    return `<tr><td>${coinCell(c)}</td><td class="r">${wad(amount, 0)}</td><td class="r">${priceText(num(price(c)))}</td><td class="r">${esc(zecText(worth))} ${esc(sym)}${usdPer(c) ? ` <span class="muted">${usdText(worth * usdPer(c))}</span>` : ''}</td><td class="r ${changeCls(ch)}">${changeText(ch)}</td><td class="r">${bondCell(c)}</td><td class="r">${ac ? priceText(num(ac.perToken)) : '<span class="muted">—</span>'}</td><td class="r"><a href="${withNet(`/coin/${token}`)}">Trade</a></td></tr>`;
  });
  $('#held').innerHTML = rows.join('') || '<tr><td colspan="8" class="empty">Nothing held yet.</td></tr>';
  $('#held-total').textContent = sumText(totals, zecText);
  $('#pf-zec').textContent = free.map(([s, v]) => `${wad(v, 4)} ${s}`).join(' · ') || '—';
  $('#pf-count').textContent = count;
  // A 24h change across coins priced in different quotes only in dollars.
  const inUsd = (m) => [...m].reduce((t, [s, v]) => (t == null || !usdBy[s] ? null : t + v * usdBy[s]), 0);
  const [total, before] = totals.size === 1 ? [[...totals.values()][0], [...befores.values()][0]] : [inUsd(totals), inUsd(befores)];
  const ch = total != null && before > 0 ? pct(total, before) : null;
  $('#pf-24h').textContent = changeText(ch);
  $('#pf-24h').className = changeCls(ch);
  const made = mine.created.map((t) => curves.find((c) => c.token === t)).filter(Boolean);
  $('#made').innerHTML = made.length ? made.map((c) => {
    const s = stats.get(c.token) || {};
    const ch24 = change24(c);
    return `<tr><td>${coinCell(c)}</td><td class="r">${esc(capText(mcap(c), c))}</td><td class="r ${changeCls(ch24)}">${changeText(ch24)}</td><td class="r">${zecCell(vol24(c))} ${esc(qsym(c))}</td><td class="r">${bondCell(c)}</td><td class="r muted">${s.created_time ? ago(s.created_time) : '—'}</td><td class="r"><a href="${withNet(`/coin/${c.token}`)}">Open</a></td></tr>`;
  }).join('') : `<tr><td colspan="7" class="empty">No coins made from this wallet yet. <a href="${withNet('/launch')}">Create one.</a></td></tr>`;
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
/// The coin a new coin is priced in: ZEC.zy unless another is picked.
let createQuote = XZEC;
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
  // The coins it can be priced in, and what its curve is in the one picked.
  if (!quotes.some((q) => q.asset === createQuote)) createQuote = (quotes.find((q) => q.asset === XZEC) || quotes[0])?.asset || XZEC;
  const qt = quotes.find((q) => q.asset === createQuote) || { asset: XZEC, symbol: 'ZEC', unit: ZAT, bond: 0n };
  const picker = $('#create-quote');
  const chips = quotes.map((q) => `<label class="chip"><input type="radio" name="create-quote" value="${q.asset}"${q.asset === createQuote ? ' checked' : ''} /><span>${esc(q.symbol)}</span></label>`).join('');
  if (picker.jadeHtml !== chips) { picker.innerHTML = chips; picker.jadeHtml = chips; }
  $('#create-quote-row').hidden = quotes.length < 2;
  const start = num(CURVE.baseZat * BigInt(CURVE.lots) * qt.unit);
  const per = usdBy[qt.symbol];
  const both = (v) => `${zecText(v)} ${qt.symbol}${per ? ` · ${usdText(v * per)}` : ''}`;
  $('#fact-start').textContent = both(start);
  $('#fact-raise').textContent = `~${both(num(cost(0, CURVE.graduateAt, qt.unit)))}`;
  $('#fact-cap').textContent = `~${both(gradMc(qt.unit))}`;
  $('#preview-mc').textContent = per ? usdText(start * per) : zecCell(start);
  $('#preview-usd').textContent = per ? `${zecCell(start)} ${qt.symbol}` : qt.symbol;
  setHtml('preview-price', `${priceText(num(cost(0, 1, qt.unit) / CURVE.lotTokens))} ${esc(qt.symbol)}`);
  $('#preview-mcap').textContent = `${zecText(start)} ${qt.symbol}`;
  launchButton.disabled = !ok || creating;
  const bond = qt.bond;
  $('#create-terms').textContent = terms.set
    ? `${terms.fee_bps / 100}% on every curve trade — half to the ZYN pot, 40% to this coin's pool, 10% to you when it graduates. ${bond ? `Bond ${wad(bond)} ${qt.symbol}.zy — returned only if the coin graduates; otherwise it stays in the curve.` : 'No bond.'}`
    : 'No fee and no bond are set on this network yet.';
  if (creating) return;
  const status = $('#create-status');
  // A coin just made keeps its message; the form was cleared, not refused.
  if (/^Created /.test(status.textContent)) return;
  if (!ok && !typed && !nameInput.value) status.textContent = 'Choose a name and a ticker.';
  else if (!ok) status.textContent = 'A ticker is 1–8 letters or digits; a name is 1–24 plain characters.';
  else if (!/refused|picture|Picture|attach/.test(status.textContent)) status.textContent = wallet ? 'Ready: Nap shows you everything before it is created.' : 'Connect a wallet to create it.';
}

$('#create-quote').addEventListener('change', (event) => {
  if (event.target.name !== 'create-quote') return;
  createQuote = event.target.value;
  renderPreview();
});

// Typing starts a new coin: the last one's message gives way.
form.querySelectorAll('input,textarea').forEach((input) => input.addEventListener('input', () => {
  const status = $('#create-status');
  if (/^Created /.test(status.textContent)) status.textContent = '';
  renderPreview();
}));
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
    const sym = symOf(createQuote);
    // A Nap from before §166 would make it in ZEC.zy whatever is asked: its
    // card says what it is priced in, and an older card does not.
    status.textContent = createQuote === XZEC ? 'Approve it in Nap.' : `Approve it in Nap. Its card says "Priced in ${sym}.zy"; if it does not, your Nap is older: reject it and update Nap.`;
    const r = await ask('zyn_curveCreate', { symbol, name: nameInput.value.trim(), description, image: imageData, ...(createQuote === XZEC ? {} : { quote: createQuote }) });
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
  return match ? { view: 'coin', token: match[1].toLowerCase() } : { view: 'board' };
}

function render() {
  renderWallet();
  renderEpoch();
  const route = pathRoute();
  if (route.view === 'board') renderBoard();
  if (route.view === 'coin') renderCoin(route.token);
  if (route.view === 'launch') renderPreview();
  if (route.view === 'portfolio') renderPortfolio();
  renderTape();
}

function syncRoute({ scroll = true } = {}) {
  const route = pathRoute();
  Object.entries(views).forEach(([name, view]) => { view.hidden = name !== route.view; });
  $$('.tabs [data-route]').forEach((link) => {
    if (link.dataset.route === route.view || (route.view === 'coin' && link.dataset.route === 'board')) link.setAttribute('aria-current', 'page');
    else link.removeAttribute('aria-current');
  });
  document.body.dataset.route = route.view;
  if (route.view !== 'coin') document.title = { board: 'Jade', launch: 'Create — Jade', portfolio: 'Portfolio — Jade' }[route.view];
  if (route.view === 'coin') renderCoin(route.token, true); else render();
  if (route.view === 'portfolio') loadMine().then(renderPortfolio);
  if (scroll) window.scrollTo({ top: 0, behavior: 'instant' });
}

function navigate(path) {
  history_push(path);
  syncRoute();
}
const history_push = (path) => window.history.pushState({}, '', path.replace(/\?network=testnet$/, '') + location.search);

document.addEventListener('click', (event) => {
  const link = event.target.closest('a[href^="/"]');
  // The network pill loads the other chain: a full page load, not a route.
  if (!link || link.id === 'net' || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  event.preventDefault();
  navigate(link.getAttribute('href'));
});
window.addEventListener('popstate', () => syncRoute());

// Live. A coin view: its market every 5 s and the epoch every second (the
// seal countdown). Elsewhere: the market every 10 s. Nothing while hidden.
let statusBusy = false;
setInterval(async () => {
  if (document.hidden || pathRoute().view !== 'coin' || statusBusy) return;
  statusBusy = true;
  try { noteEpoch((await rpc('zyn_status')).epoch); } catch { /* the next second */ } finally { statusBusy = false; }
}, 1_000);
setInterval(() => { if (!document.hidden) refreshCoin(); }, 5_000);
setInterval(() => { if (!document.hidden && pathRoute().view !== 'coin') loadMarket(); }, 10_000);
setInterval(loadUsd, 300_000);
setInterval(() => { if (document.hidden) return; tickAgo(); tickRing(); }, 1_000);
document.addEventListener('visibilitychange', () => { if (!document.hidden) { live.changedAt = 0; live.seen = 0; } });

// The network pill: what this page reads, and a link to the other one.
const net = $('#net');
net.textContent = NETWORK === 'testnet' ? 'Testnet' : 'Mainnet';
net.href = NETWORK === 'testnet' ? '/' : '/?network=testnet';
net.title = NETWORK === 'testnet' ? 'Switch to mainnet' : 'Switch to testnet';
installTips();
renderSortHeads();
if (matchMedia('(max-width: 780px)').matches) search.placeholder = 'Search coins';

syncRoute({ scroll: false });
loadMarket();
loadUsd();
document.documentElement.dataset.jadeReady = 'true';
