// The live strip: each network as ZynScan's independent replay sees it.
const SCAN = { mainnet: 'https://zyn-rpc.cavebro.win/scan/api/summary', testnet: 'https://zyn-rpc.cavebro.win/scan-testnet/api/summary' };

function ago(unix) {
  const s = Math.max(0, Math.round(Date.now() / 1000 - unix));
  if (s < 90) return `${s}s ago`;
  if (s < 5400) return `${Math.round(s / 60)} min ago`;
  if (s < 172800) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} days ago`;
}

function set(box, key, text, cls) {
  const el = box.querySelector(`[data-k="${key}"]`);
  if (!el) return;
  el.textContent = text;
  if (cls) el.classList.add(cls);
}

async function load(net) {
  const box = document.querySelector(`.net[data-net="${net}"]`);
  try {
    const r = await fetch(SCAN[net], { cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer' });
    if (!r.ok) throw new Error(String(r.status));
    const d = await r.json();
    set(box, 'epoch', Number(d.epoch).toLocaleString('en-US'));
    if (d.last_sealed_at) set(box, 'sealed', ago(d.last_sealed_at));
    if (Number.isFinite(d.transactions)) set(box, 'tx', Number(d.transactions).toLocaleString('en-US'));
    const agrees = d.agrees === true && d.replayed_root === d.node_root;
    set(box, 'replay', agrees ? 'agrees' : 'disagrees', agrees ? 'agrees' : null);
    set(box, 'anchor', Number.isFinite(d.anchored_through) ? `through epoch ${d.anchored_through}` : 'not yet');
    box.classList.add(agrees ? 'ok' : 'bad');
  } catch {
    box.querySelectorAll('dd').forEach((dd) => { dd.textContent = '—'; });
    set(box, 'replay', 'unreachable');
    box.classList.add('bad');
  }
}

load('mainnet');
load('testnet');
