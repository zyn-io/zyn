// ZynScan — reads zynscan's read-only API (zynzapd/src/bin/zynscan.rs) for
// the chosen network and renders epochs, transactions and public assets.
// Nothing here writes, and nothing here can see more than the API serves.

const APIS = {
  mainnet: 'https://zyn-rpc.cavebro.win/scan',
  testnet: 'https://zyn-rpc.cavebro.win/scan-testnet',
};
const params = new URLSearchParams(location.search);
const LOCAL = ['localhost', '127.0.0.1'].includes(location.hostname);
let network = params.get('network') === 'testnet' ? 'testnet' : 'mainnet';
const api = () => (LOCAL && params.get('api')) || APIS[network];

const main = document.getElementById('main');
const esc = (v) => String(v ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const isHex = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
const short = (h, n = 8) => (isHex(h) ? `${h.slice(0, n)}…${h.slice(-4)}` : esc(h));
const hash = (h) => `<span class="hash" title="${esc(h)}">${short(h)}</span>`;
const num = (n) => (typeof n === 'number' ? n.toLocaleString('en-US') : '—');
const WAD = 10n ** 18n;

function ago(t) {
  if (!t) return '—';
  const s = Math.max(0, Math.floor(Date.now() / 1000) - t);
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}
function wad(raw, places = 4) {
  try {
    const v = BigInt(raw);
    const f = (v % WAD).toString().padStart(18, '0').slice(0, places).replace(/0+$/, '');
    return `${(v / WAD).toLocaleString('en-US')}${f ? `.${f}` : ''}`;
  } catch { return esc(raw); }
}
/// A launchpad coin's 32 bytes of meta: 8 for the ticker, 24 for the name.
function coinName(meta) {
  if (!isHex(meta)) return null;
  const b = Uint8Array.from(meta.match(/../g).map((h) => parseInt(h, 16)));
  const text = (x) => { const end = x.indexOf(0); return new TextDecoder().decode(end === -1 ? x : x.subarray(0, end)); };
  const symbol = text(b.subarray(0, 8));
  const name = text(b.subarray(8));
  return /^[A-Z0-9]{1,8}$/.test(symbol) ? { symbol, name } : null;
}

async function get(path) {
  const r = await fetch(`${api()}${path}`, { credentials: 'omit', referrerPolicy: 'no-referrer' });
  const body = await r.json().catch(() => ({}));
  if (!r.ok) throw Object.assign(new Error(body.error || `HTTP ${r.status}`), { status: r.status });
  return body;
}

// ---------------------------------------------------------------------------
// Pieces.

const KIND_CLASS = {
  'Curve trade': 'trade', Swap: 'trade', 'Offer taken': 'trade', 'Coin created': 'create', 'Pool created': 'create',
  'Collection registered': 'op', 'Fees set': 'op', 'Curve terms set': 'op', 'Item minted': 'op', 'Exit paid': 'op',
  Deposit: 'bridge', 'Withdraw to Zcash': 'bridge', Claim: 'claim',
  'Tickets bought': 'trade', 'Items claimed': 'claim', 'Open mint': 'trade',
  'Sale opened': 'op', 'Sale closed': 'op', 'Draw posted': 'op', 'Open mint posted': 'op',
  'Burn posted': 'bridge', 'Burn tickets capped': 'op',
  'Bridge deposit': 'bridge', 'Bridge deposit claimed': 'claim', 'Bridged asset registered': 'op',
};
const kindChip = (t) => `<span class="chip ${KIND_CLASS[t.kind] || 'plain'}">${esc(t.kind)}</span>`;
const status = (t) => (t.ok ? '<span class="ok">accepted</span>' : `<span class="bad" title="${esc(t.refusal)}">refused</span>`);

function anchorBadge(a, compact = false) {
  if (a) {
    const link = `https://mainnet.zcashexplorer.app/transactions/${esc(a.zcash_txid)}`;
    return `<span class="anchor yes">on Zcash${compact ? '' : ` · height ${num(a.zcash_height)} · <a href="${network === 'mainnet' ? link : '#'}" rel="noreferrer">${short(a.zcash_txid, 6)}</a>`}</span>`;
  }
  return `<span class="anchor no" title="Sealed on Zyn; its root is not yet carried by a Zcash transaction">on Zyn</span>`;
}

function detail(t) {
  const d = t.detail;
  if (!d) return '';
  const parts = [];
  if (d.token) parts.push(`coin ${hash(d.token)}`);
  if (d.pool) parts.push(`pool ${hash(d.pool)}`);
  if (d.collection) parts.push(`collection ${hash(d.collection)}`);
  if (d.serial !== undefined) parts.push(`serial #${esc(d.serial)}`);
  if (d.bridge_bps !== undefined) parts.push(`bridge fee ${esc(d.bridge_bps)} bps`);
  if (d.fee_bps !== undefined) parts.push(`fee ${esc(d.fee_bps)} bps`);
  if (d.bond !== undefined) parts.push(`bond ${wad(d.bond)} ZEC.zy`);
  if (d.cap !== undefined) parts.push(`cap ${num(d.cap)}`);
  if (d.zcash_txid) parts.push(`Zcash tx ${hash(d.zcash_txid)}`);
  if (d.tickets !== undefined) parts.push(`${num(d.tickets)} ticket${d.tickets === 1 ? '' : 's'}`);
  if (Array.isArray(d.serials)) parts.push(d.serials.map((x) => `#${esc(x)}`).join(', '));
  if (d.ticket_price !== undefined) parts.push(`ticket ${wad(d.ticket_price)} · claim ${wad(d.claim_price)} · open ${wad(d.open_price)} ZEC.zy`);
  if (d.zcash_height !== undefined) parts.push(`closed at Zcash ${num(d.zcash_height)}`);
  if (d.root) parts.push(`root ${hash(d.root)}`);
  if (d.items !== undefined) parts.push(`${num(d.items)} in order`);
  if (d.burn) parts.push(`burn ${hash(d.burn)}`);
  if (d.asset && !d.serial && d.serial !== 0) parts.push(`asset ${hash(d.asset)}`);
  if (d.network) parts.push(`${esc(d.network)}${d.origin ? ` · ${hash(d.origin)}` : ''} · ${esc(d.decimals)} decimals`);
  return parts.join(' · ');
}

const txRow = (t) => `<tr>
  <td class="tx"><a href="/tx/${t.seq}" data-link class="mono">#${num(t.seq)}</a></td>
  <td class="kind">${kindChip(t)}</td>
  <td class="epoch"><a href="/epoch/${t.epoch}" data-link class="mono">${num(t.epoch)}</a></td>
  <td class="status">${status(t)}</td>
  <td class="notes mono dim">${t.created == null ? '—' : `${t.created}↑ ${t.spent}↓`}</td>
  <td class="detail">${detail(t)}</td>
</tr>`;
const txTable = (rows, empty) => rows.length
  ? `<div class="scroll"><table class="txs"><thead><tr><th>Tx</th><th>Kind</th><th>Epoch</th><th>Status</th><th title="notes created ↑ / spent ↓">Notes</th><th>Public detail</th></tr></thead><tbody>${rows.map(txRow).join('')}</tbody></table></div>`
  : `<p class="empty">${empty}</p>`;

const epochRow = (e) => `<tr>
  <td class="epoch"><a href="/epoch/${e.epoch}" data-link class="mono">${num(e.epoch)}</a></td>
  <td class="root">${e.sealed ? hash(e.root) : '<span class="dim">open</span>'}</td>
  <td class="txs mono">${num(e.txs)}${e.refused ? ` <span class="bad">(${e.refused} refused)</span>` : ''}</td>
  <td class="sealed dim">${e.sealed ? ago(e.sealed_at) : 'now'}</td>
  <td class="settled">${e.sealed ? anchorBadge(e.anchor, true) : ''}</td>
</tr>`;
const epochTable = (rows) => rows.length
  ? `<div class="scroll"><table class="epochs"><thead><tr><th>Epoch</th><th>State root</th><th>Txs</th><th>Sealed</th><th>Settled</th></tr></thead><tbody>${rows.map(epochRow).join('')}</tbody></table></div>`
  : '<p class="empty">No epochs yet.</p>';

// ---------------------------------------------------------------------------
// Pages.

async function home() {
  const [s, epochs, txs] = await Promise.all([get('/api/summary'), get('/api/epochs?limit=12'), get('/api/txs?limit=15')]);
  const agree = s.agrees === true
    ? `<span class="verify yes">✓ Replay matches the node</span>`
    : s.agrees === false
      ? `<span class="verify no">✗ Replay disagrees with the node</span>`
      : `<span class="verify wait">Replay catching up</span>`;
  const anchored = s.anchored_through != null
    ? `Through epoch ${num(s.anchored_through)}`
    : 'Not yet — the first anchor waits on a funded vault';
  const kinds = Object.entries(s.kinds || {}).sort((a, b) => b[1] - a[1])
    .map(([k, n]) => `<li><span>${esc(k)}</span><b>${num(n)}</b></li>`).join('');
  return `
    <section class="hero">
      <div>
        <p class="eyebrow">${esc(s.network)} · chain ${esc(s.chain)}</p>
        <h1>Every Zyn transaction, replayed.</h1>
        <p class="lede">This page rebuilds the ledger from the sequencer's own journal and compares the result with the root the node reports. ${agree}</p>
      </div>
      <dl class="stats">
        <div><dt>Epoch</dt><dd>${num(s.epoch)}</dd></div>
        <div><dt>Transactions</dt><dd>${num(s.transactions)}</dd></div>
        <div><dt>Sealed epochs</dt><dd>${num(s.sealed_epochs)}</dd></div>
        <div><dt>Last seal</dt><dd>${ago(s.last_sealed_at)}</dd></div>
        <div class="wide"><dt>Settled on Zcash</dt><dd class="small">${anchored}</dd></div>
        <div class="wide"><dt>State root (replayed)</dt><dd class="small">${hash(s.replayed_root)}</dd></div>
      </dl>
    </section>
    ${s.error ? `<p class="warn">The index reports: ${esc(s.error)}</p>` : ''}
    <div class="grid">
      <section class="panel"><h2>Latest transactions</h2>${txTable(txs, 'No transactions yet.')}</section>
      <section class="panel"><h2>Latest epochs</h2>${epochTable(epochs)}
        ${kinds ? `<h3>By kind</h3><ul class="kinds">${kinds}</ul>` : ''}</section>
    </div>`;
}

async function epochPage(n) {
  const e = await get(`/api/epoch/${n}`);
  const prev = n > 0 ? `<a href="/epoch/${n - 1}" data-link>← ${n - 1}</a>` : '';
  const next = `<a href="/epoch/${n + 1}" data-link>${n + 1} →</a>`;
  return `
    <nav class="crumbs"><a href="/" data-link>Home</a> / Epoch ${num(n)} <span class="pager">${prev} ${next}</span></nav>
    <section class="panel">
      <h1>Epoch ${num(n)}</h1>
      <dl class="facts">
        <div><dt>Status</dt><dd>${e.sealed ? `sealed ${ago(e.sealed_at)}` : 'open — still taking transactions'}</dd></div>
        <div><dt>State root</dt><dd class="mono break">${e.root ? esc(e.root) : '—'}</dd></div>
        <div><dt>Transactions</dt><dd>${num(e.txs)}${e.refused ? `, ${e.refused} refused` : ''}</dd></div>
        <div><dt>Settled</dt><dd>${e.sealed ? anchorBadge(e.anchor) : '—'}</dd></div>
      </dl>
      <h2>Transactions</h2>
      ${txTable(e.transactions || [], 'No transactions in this epoch — it only sealed.')}
    </section>`;
}

async function txPage(n) {
  const t = await get(`/api/tx/${n}`);
  return `
    <nav class="crumbs"><a href="/" data-link>Home</a> / <a href="/epoch/${t.epoch}" data-link>Epoch ${num(t.epoch)}</a> / Tx #${num(n)}</nav>
    <section class="panel">
      <h1>Transaction #${num(n)}</h1>
      <dl class="facts">
        <div><dt>Kind</dt><dd>${kindChip(t)} ${t.operator ? '<span class="dim">operator</span>' : '<span class="dim">holder</span>'}</dd></div>
        <div><dt>Status</dt><dd>${status(t)}${t.refusal ? ` — ${esc(t.refusal)}` : ''}</dd></div>
        <div><dt>Epoch</dt><dd><a href="/epoch/${t.epoch}" data-link>${num(t.epoch)}</a></dd></div>
        <div><dt>Notes</dt><dd>${t.created == null ? 'not shown for lending' : `${t.created} created · ${t.spent} spent`}</dd></div>
        <div><dt>Public detail</dt><dd>${detail(t) || '<span class="dim">none — who and how much stay private</span>'}</dd></div>
        <div><dt>Settled</dt><dd>${anchorBadge(t.anchor)}</dd></div>
      </dl>
    </section>`;
}

async function assetsPage() {
  const { market, status: st } = await get('/api/market');
  const m = market || {};
  const issued = Object.entries(st?.issued || {});
  const curves = (m.curves || []).map((c) => {
    const n = coinName(c.meta);
    return `<tr><td>${n ? `<b>${esc(n.symbol)}</b> ${esc(n.name)}` : '—'}</td><td>${hash(c.token)}</td><td class="mono">${c.graduated ? 'graduated' : `${((c.sold / (c.graduate_at || 51098)) * 100).toFixed(1)}%`}</td><td class="mono">${wad(c.reserve)} ZEC.zy</td></tr>`;
  }).join('');
  const pools = (m.pools || []).map((p) => `<tr><td>${hash(p.id)}</td><td class="mono">${wad(p.reserve_a)} / ${wad(p.reserve_b)}</td><td class="mono">${esc(p.fee_bps)} bps</td></tr>`).join('');
  const cols = (m.collections || []).map((c) => `<tr><td>${hash(c.asset)}</td><td class="mono">${esc(c.fee_bps)} bps</td><td class="mono">${num(Number(c.outstanding || 0))}</td><td class="mono">${wad(c.floor)} ZEC.zy</td></tr>`).join('');
  return `
    <nav class="crumbs"><a href="/" data-link>Home</a> / Assets</nav>
    <section class="panel">
      <h1>Assets</h1>
      <dl class="facts">
        <div><dt>Bridge fee</dt><dd>${m.fees ? `${esc(m.fees.bridge_bps)} bps` : 'not set'}</dd></div>
        <div><dt>Launchpad terms</dt><dd>${m.curve_terms?.set ? `${esc(m.curve_terms.fee_bps)} bps · bond ${wad(m.curve_terms.bond)} ZEC.zy` : 'not set'}</dd></div>
      </dl>
      <h2>Issued supply</h2>
      ${issued.length ? `<div class="scroll"><table><thead><tr><th>Asset</th><th>Issued</th></tr></thead><tbody>${issued.map(([a, v]) => `<tr><td>${hash(a)}</td><td class="mono">${esc(v)}</td></tr>`).join('')}</tbody></table></div>` : '<p class="empty">Nothing issued yet.</p>'}
      <h2>Collections</h2>
      ${cols ? `<div class="scroll"><table><thead><tr><th>Collection</th><th>Fee</th><th>Outstanding</th><th>Floor</th></tr></thead><tbody>${cols}</tbody></table></div>` : '<p class="empty">No collections registered.</p>'}
      <h2>Launchpad coins</h2>
      ${curves ? `<div class="scroll"><table><thead><tr><th>Coin</th><th>Address</th><th>Bonded</th><th>In its curve</th></tr></thead><tbody>${curves}</tbody></table></div>` : '<p class="empty">No coins yet.</p>'}
      <h2>Pools</h2>
      ${pools ? `<div class="scroll"><table><thead><tr><th>Pool</th><th>Reserves</th><th>Fee</th></tr></thead><tbody>${pools}</tbody></table></div>` : '<p class="empty">No pools yet.</p>'}
    </section>`;
}

// ---------------------------------------------------------------------------
// Search and routing.

async function search(q) {
  const v = q.trim().toLowerCase();
  if (!v) return;
  const m = v.match(/^(?:epoch\s*|e)(\d+)$/);
  if (m) return navigate(`/epoch/${m[1]}`);
  const t = v.match(/^#?(\d+)$/);
  if (t) return navigate(`/tx/${t[1]}`);
  if (isHex(v)) {
    const { market } = await get('/api/market');
    const hit = (market?.curves || []).some((c) => c.token === v) || (market?.pools || []).some((p) => p.id === v)
      || (market?.collections || []).some((c) => c.asset === v);
    if (hit) return navigate('/assets');
  }
  main.innerHTML = `<p class="empty">Nothing public matches “${esc(q)}”. Try an epoch (e12), a transaction (#34), or a coin, collection or pool address.</p>`;
}

function route() {
  const p = location.pathname.replace(/\/+$/, '') || '/';
  let m;
  if ((m = p.match(/^\/epoch\/(\d+)$/))) return () => epochPage(Number(m[1]));
  if ((m = p.match(/^\/tx\/(\d+)$/))) return () => txPage(Number(m[1]));
  if (p === '/assets') return assetsPage;
  return home;
}

// On phones each table row stacks into a block, so every cell carries its
// column's name for the small label it shows there.
function labelCells(root) {
  for (const table of root.querySelectorAll('table')) {
    const names = [...table.querySelectorAll('thead th')].map((th) => th.textContent.trim());
    for (const tr of table.querySelectorAll('tbody tr')) [...tr.cells].forEach((td, i) => { td.dataset.label = names[i] || ''; });
  }
}

async function render() {
  document.querySelectorAll('[data-net]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.net === network)));
  try {
    main.innerHTML = await route()();
    labelCells(main);
  } catch (e) {
    main.innerHTML = e.status === 404
      ? `<p class="empty">Not on the ${network} ledger (yet). <a href="/" data-link>Back to the latest</a>.</p>`
      : `<p class="warn">ZynScan could not reach the ${network} index: ${esc(e.message)}</p>`;
  }
}

function navigate(path) {
  const q = network === 'testnet' ? '?network=testnet' : '';
  history.pushState({}, '', path + q + (LOCAL && params.get('api') ? `${q ? '&' : '?'}api=${encodeURIComponent(params.get('api'))}` : ''));
  render();
  window.scrollTo(0, 0);
}

document.addEventListener('click', (e) => {
  const a = e.target.closest('a[data-link]');
  if (!a || e.metaKey || e.ctrlKey) return;
  e.preventDefault();
  navigate(a.getAttribute('href'));
});
document.querySelectorAll('[data-net]').forEach((b) => b.addEventListener('click', () => {
  network = b.dataset.net;
  navigate(location.pathname);
}));
document.getElementById('search').addEventListener('submit', (e) => {
  e.preventDefault();
  search(document.getElementById('q').value);
});
window.addEventListener('popstate', render);

render();
// Epochs seal every few seconds when busy: refresh the front page quietly.
setInterval(() => { if (!document.hidden && route() === home) render(); }, 15_000);
