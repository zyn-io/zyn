// The ledger table: recent epochs as ZynScan's independent replay sees them.
const SCAN = { mainnet: 'https://zyn-rpc.cavebro.win/scan', testnet: 'https://zyn-rpc.cavebro.win/scan-testnet' };
const SCAN_PAGE = { mainnet: 'https://zynscan.pages.dev', testnet: 'https://zynscan.pages.dev' };
const ROWS = 6;

const $ = (id) => document.getElementById(id);
const esc = (v) => String(v ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const hex64 = (h) => typeof h === 'string' && /^[0-9a-f]{64}$/.test(h);
const short = (h) => (hex64(h) ? `${h.slice(0, 10)}…${h.slice(-4)}` : '—');
const num = (n) => (Number.isFinite(n) ? n.toLocaleString('en-US') : '—');

function ago(unix) {
  const s = Math.max(0, Math.round(Date.now() / 1000 - unix));
  if (s < 90) return `${s} s ago`;
  if (s < 5400) return `${Math.round(s / 60)} min ago`;
  if (s < 172800) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} days ago`;
}

async function get(net, path) {
  const r = await fetch(SCAN[net] + path, { cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer' });
  if (!r.ok) throw new Error(String(r.status));
  return r.json();
}

// Everything placed in markup below is a checked number or 64-hex string, or escaped.
const epochLink = (net, n) => `${SCAN_PAGE[net]}/epoch/${Number.isSafeInteger(n) ? n : 0}${net === 'testnet' ? '?network=testnet' : ''}`;

let shown = null;
async function show(net) {
  shown = net;
  document.querySelectorAll('.switch button').forEach((b) => b.setAttribute('aria-selected', String(b.dataset.net === net)));
  $('ledger-status').textContent = 'Reading the journal…';
  try {
    const [summary, epochs, txs] = await Promise.all([get(net, '/api/summary'), get(net, '/api/epochs'), get(net, '/api/txs').catch(() => [])]);
    if (shown !== net) return;
    const agrees = summary.agrees === true && summary.replayed_root === summary.node_root;
    const anchored = Number.isFinite(summary.anchored_through) ? `anchored to Zcash through epoch ${num(summary.anchored_through)}` : 'no epoch anchored to Zcash yet';
    $('ledger-status').innerHTML = `Epoch ${num(summary.epoch)}, ${num(summary.transactions)} transactions so far, ${anchored}. ZynScan's replay of the journal <b>${agrees ? 'agrees' : 'does not agree'}</b> with the node.`;

    const rows = (Array.isArray(epochs) ? epochs : []).filter((e) => e.sealed).sort((a, b) => b.epoch - a.epoch).slice(0, ROWS);
    $('epochs').innerHTML = rows.length ? rows.map((e) => `<tr>
      <td><a href="${epochLink(net, e.epoch)}">${num(e.epoch)}</a></td>
      <td>${e.sealed_at ? ago(e.sealed_at) : '—'}</td>
      <td class="num">${num(e.txs)}</td>
      <td class="root" title="${esc(e.root)}">${short(e.root)}</td>
      <td>${e.anchor ? `block ${num(e.anchor.zcash_height)}` : '<span class="none">not yet</span>'}</td>
    </tr>`).join('') : '<tr><td colspan="5" class="muted">No sealed epochs yet.</td></tr>';

    const a = rows.find((e) => e.anchor && hex64(e.anchor.zcash_txid));
    $('trace').innerHTML = a
      ? `Follow one: epoch ${num(a.epoch)} applied ${num(a.txs)} transaction${a.txs === 1 ? '' : 's'} and sealed to root <code>${short(a.root)}</code>. That root was written into Zcash ${net} block ${num(a.anchor.zcash_height)}, in transaction <code>${short(a.anchor.zcash_txid)}</code>.`
      : 'None of these epochs is anchored to Zcash yet; their roots are sealed on Zyn and replayed by ZynScan.';

    const kinds = [];
    for (const t of Array.isArray(txs) ? txs : []) if (t.ok && !kinds.includes(t.kind)) kinds.push(t.kind);
    $('recent').textContent = kinds.length ? `Recent activity: ${kinds.slice(0, 6).join(', ')}.` : '';
  } catch {
    if (shown !== net) return;
    $('ledger-status').textContent = 'ZynScan is not answering right now; the table fills in when it does.';
    $('epochs').innerHTML = '<tr><td colspan="5" class="muted">—</td></tr>';
    $('trace').textContent = '';
    $('recent').textContent = '';
  }
}

document.querySelectorAll('.switch button').forEach((b) => b.addEventListener('click', () => show(b.dataset.net)));
show('testnet');
