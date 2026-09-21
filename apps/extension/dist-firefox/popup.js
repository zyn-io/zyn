window.ZYN_API_BASE = 'http://127.0.0.1:8977';

const $ = (id) => document.getElementById(id);
let state = null;
let busy = false;

// Where the app is. In the browser tab it is this origin; in the desktop
// and mobile shells it is in-process (Tauri); in the extension popup it is
// the local app on loopback.
const BASE = window.ZYN_API_BASE || '';
async function api(path, body) {
  if (window.__TAURI__ && window.__TAURI__.core) {
    const r = await window.__TAURI__.core.invoke('api_call', { method: body ? 'POST' : 'GET', path, input: body || {} });
    if (r && r.error) throw new Error(r.error);
    return r;
  }
  const opts = body ? { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Zyn': '1' }, body: JSON.stringify(body) } : {};
  const r = await fetch(BASE + path, opts).catch(() => { throw new Error('Nap Wallet is not running'); });
  const j = await r.json().catch(() => ({ error: 'bad reply' }));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}

function logLine(text, err) {
  const li = document.createElement('li');
  li.textContent = new Date().toLocaleTimeString() + '  ' + text;
  if (err) li.className = 'err';
  $('log').prepend(li);
  if (err) toast(text, true);
}
function toast(text, err) { $('sync-hint').textContent = text; $('sync-hint').style.color = err ? 'var(--bad)' : ''; }

async function run(btn, label, fn) {
  if (busy) return;
  busy = true;
  const old = btn.textContent;
  btn.disabled = true; btn.textContent = label;
  try { await fn(); await waitForJob(); } catch (e) { logLine(e.message, true); }
  busy = false; btn.disabled = false; btn.textContent = old;
  await load();
}
// A started job runs on the app's own thread; poll until it is gone.
async function waitForJob() {
  for (;;) {
    let o; try { o = await api('/api/overview'); } catch { return; }
    render(o);
    if (!o.job) return;
    await new Promise(r => setTimeout(r, 1500));
  }
}
// Review before anything that moves value. Resolves true on Confirm.
function confirmDialog(title, rows) {
  return new Promise((resolve) => {
    $('dlg-title').textContent = title;
    $('dlg-body').innerHTML = rows.map(([k, v]) => `<div class="k">${esc(k)}</div><div class="v ${/address|account|memo/i.test(k) ? 'mono' : ''}">${esc(v)}</div>`).join('');
    const done = (v) => { $('dlg').close(); $('dlg-yes').onclick = $('dlg-no').onclick = null; resolve(v); };
    $('dlg-yes').onclick = () => done(true);
    $('dlg-no').onclick = () => done(false);
    $('dlg').onclose = () => resolve(false);
    $('dlg').showModal();
  });
}

function fmt(x, d = 8) { let s = Number(x).toFixed(d); if (s.includes('.')) s = s.replace(/\.?0+$/, ''); return s === '' || s === '-' ? '0' : s; }
function short(s, n = 10) { return s && s.length > 2 * n ? s.slice(0, n) + '…' + s.slice(-6) : (s || ''); }
// A long string shown short. The whole of it stays on the element, for the
// What this account has put into Zyn. Three buckets because they behave
// differently: idle can leave today, pooled is exposed to the pair, and an
// item's floor is the least it is worth rather than what it would fetch.
// Everything this wallet holds, across both chains. The Zyn side is priced by
// the chain itself; anything it cannot price is listed without a value and left
// out of the total, the same way the portfolio treats it.
function renderHoldings(s) {
  const z = s.zyn || {};
  const price = new Map((z.assets || []).map(a => [a.id, a.price_zec]));
  const lp = new Map((z.positions || []).map(q => [q.lp_asset, q]));
  const rows = [];
  if (Number(s.balance.total) > 0) {
    rows.push({ sym: s.unit, on: 'Zcash · shielded', amount: s.balance.total, zec: Number(s.balance.total) });
  }
  for (const h of (z.record && z.record.spendable) || []) {
    const n = Number(h.amount);
    if (!(n > 0)) continue;
    const q = lp.get(h.asset);
    const p = price.get(h.asset);
    rows.push({
      sym: h.symbol,
      on: q ? `Zyn · ${q.symbol0}/${q.symbol1} pool` : 'Zyn',
      amount: h.amount,
      zec: q ? Number(q.value_zec) : (p != null ? n * Number(p) : null),
    });
  }
  rows.sort((a, b) => (b.zec || 0) - (a.zec || 0));
  $('holdings').querySelector('tbody').innerHTML = rows.map((r) =>
    `<tr><td class="sym">${esc(r.sym)}<small>${esc(r.on)}</small></td>` +
    `<td class="amt">${esc(amt(r.amount))}</td>` +
    `<td class="val">${valueCell(r, s.unit, s.zec_usd)}</td></tr>`).join('');

  // The chain's own total for the Zyn side, not a sum re-derived here.
  const total = Number(s.balance.total) + Number((z.portfolio && z.portfolio.total_zec) || 0);
  $('bal').textContent = fmt(total);
  const un = (z.portfolio && z.portfolio.unpriced) || 0;
  $('holdings-note').textContent = !rows.length
    ? 'Nothing held yet.'
    : un ? `${un} holding(s) the chain cannot price are listed without a value and are not counted above.` : '';
  setFiat(total, s);
}

function valueCell(r, unit, usd) {
  if (r.zec == null) return '—';
  if (usd != null) return esc(money(r.zec * usd));
  // No price to convert with. Fall back to the ZEC quote — except on a
  // ZEC-denominated holding, where printing the same number twice on one row
  // reads as a bug rather than as information.
  if (Math.abs(r.zec - Number(r.amount)) < 1e-12) return '';
  return esc(fmt(r.zec)) + ' ' + esc(unit);
}

// Holdings worth a fraction of a cent are ordinary here, and rounding them all
// to $0.00 says nothing. Widen the places rather than flatten the number.
function money(x) {
  if (!isFinite(x) || x === 0) return '$0.00';
  const a = Math.abs(x);
  if (a < 0.0001) return '<$0.0001';
  const d = a < 0.01 ? 4 : 2;
  return '$' + x.toLocaleString(undefined, { minimumFractionDigits: d, maximumFractionDigits: d });
}

// Zyn amounts arrive as high-precision decimal strings. Show them at a size a
// person reads, without implying more precision than matters.
function amt(v) {
  const n = Number(v);
  if (!isFinite(n)) return String(v);
  return n >= 1000 ? n.toLocaleString(undefined, { maximumFractionDigits: 2 }) : fmt(n, 8);
}

// The price is fetched and cached by the app, never by this page: one wallet
// asking once every ten minutes, rather than a request each time the popup is
// opened. Absent when the holder has the switch off in Settings.
function setFiat(totalZec, s) {
  const usd = s.zec_usd;
  $('bal-fiat').hidden = usd == null;
  if (usd == null) return;
  // Testnet coins convert arithmetically and are still worth nothing. Say so
  // rather than let the figure imply otherwise.
  const note = s.network === 'mainnet' ? '' : ' <small>· testnet, not real value</small>';
  $('bal-fiat').innerHTML = '≈ ' + esc(money(totalZec * usd)) + note;
}

// The ZYN market's schedule, as the chain publishes it.
//
// Three states, and the honest one to show changes with them. Before the pot
// fills there is no date, only progress toward one — so a countdown would be a
// guess. Once it fills the date is fixed and public, so this counts down to a
// block height rather than to anything anyone announced.
function renderMarket(z) {
  const m = z.market;
  const sec = $('zyn-market');
  if (!m) { sec.hidden = true; return; }
  sec.hidden = false;

  const chipFor = { filling: 'muted', scheduled: 'warn', open: 'ok' }[m.state] || 'muted';
  $('zm-state').className = 'record ' + chipFor;

  if (m.state === 'open') {
    $('zm-state').textContent = 'open';
    $('zm-line').textContent = `Trading since Zcash height ${m.open_height}.`;
    $('zm-bar').hidden = true;
    $('zm-detail').textContent = `Opened at ${m.open_price_zec} ZEC per ZYN.`;
    return;
  }

  if (m.state === 'scheduled') {
    $('zm-state').textContent = 'opens soon';
    const blocks = Number(m.blocks_remaining || 0);
    $('zm-line').innerHTML = blocks > 0
      ? `Opens at Zcash height <b>${m.open_height}</b> — about <b>${humanDuration(m.seconds_remaining)}</b> from now.`
      : `Due to open at Zcash height <b>${m.open_height}</b>, on the next block the chain sees.`;
    // Progress through the grace period rather than through the fill.
    const done = Math.max(0, Number(m.grace_blocks) - blocks);
    $('zm-bar').hidden = false;
    $('zm-bar').firstElementChild.style.width =
      Math.min(100, Math.round(100 * done / Number(m.grace_blocks))) + '%';
    $('zm-detail').textContent =
      `${blocks.toLocaleString()} Zcash blocks to go. ${m.zyn_in_pot} ZYN will open against ` +
      `${m.zec_target} ZEC.zy at ${m.open_price_zec} ZEC per ZYN — both fixed when the date was set.`;
    return;
  }

  $('zm-state').textContent = 'not scheduled';
  const have = Number(m.zec_in_pot || 0), want = Number(m.zec_target || 420);
  $('zm-line').innerHTML =
    `No opening date yet. One is set when the pot reaches <b>${want} ZEC.zy</b>, ` +
    `and the market opens ${Number(m.grace_blocks).toLocaleString()} Zcash blocks after that.`;
  $('zm-bar').hidden = false;
  $('zm-bar').firstElementChild.style.width =
    Math.min(100, Math.round(100 * have / want)) + '%';
  $('zm-detail').textContent =
    `${m.zec_in_pot} of ${want} ZEC.zy accrued · ${m.zyn_in_pot} of ${m.zyn_target} ZYN matched. ` +
    `The pot fills from trading fees, so the date exists only once the backing does.`;
}

// Blocks are the chain's unit; hours and days are the reader's.
function humanDuration(secs) {
  const s = Number(secs || 0);
  if (s <= 0) return 'moments';
  const d = Math.floor(s / 86400), h = Math.round((s % 86400) / 3600);
  if (d >= 1) return h > 0 ? `${d}d ${h}h` : `${d}d`;
  const m = Math.round((s % 3600) / 60);
  const hh = Math.floor(s / 3600);
  return hh >= 1 ? `${hh}h ${m}m` : `${m}m`;
}

function renderPortfolio(z) {
  const p = z.portfolio || {};
  $('pf-total').textContent = p.total_zec != null ? `${p.total_zec} ZEC` : '—';
  $('pf-split').textContent = p.total_zec == null ? '' :
    `${p.idle_zec} idle · ${p.pooled_zec} in pools · ${p.items_floor_zec} backing items`;
  $('pf-unpriced').textContent = p.unpriced ? `${p.unpriced} holding(s) the chain cannot price — not counted above.` : '';
  $('pf-orders').textContent = p.open_orders ? `${p.open_orders} open order(s)` : '';

  const tb = $('pf-positions').querySelector('tbody');
  tb.innerHTML = '';
  for (const q of z.positions || []) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${esc(q.symbol0)}/${esc(q.symbol1)}</td>` +
      `<td class="r">${(q.share * 100).toFixed(2)}%</td>` +
      `<td class="r">${esc(q.amount0)} ${esc(q.symbol0)}<br>${esc(q.amount1)} ${esc(q.symbol1)}</td>` +
      `<td class="r">${q.value_zec ? esc(q.value_zec) + ' ZEC' : '—'}</td>`;
    tb.appendChild(tr);
  }
  $('pf-positions-empty').hidden = (z.positions || []).length > 0;
  $('pf-positions-empty').textContent = 'No liquidity provided yet.';

}

// An identicon from the content hash.
//
// There is no art to show until something serves the bytes, and a broken
// image tells a holder nothing. This is derived from the hash itself, so two
// items never look alike and a *changed* hash never looks the same — which is
// the one property a placeholder for a content commitment should have.
function identicon(hex, cells) {
  const n = cells || 5;
  const b = [];
  for (let i = 0; i + 1 < hex.length; i += 2) b.push(parseInt(hex.slice(i, i + 2), 16));
  if (!b.length) return '';
  const hue = (b[0] * 360) / 256, hue2 = (hue + 40 + (b[1] % 80)) % 360;
  const fg = `hsl(${hue.toFixed(0)} 62% 52%)`, fg2 = `hsl(${hue2.toFixed(0)} 58% 42%)`;
  const half = Math.ceil(n / 2);
  let rects = '';
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < half; x++) {
      const bit = b[(y * half + x) % b.length] & (1 << (x % 8));
      if (!bit) continue;
      const c = ((x + y) % 3 === 0) ? fg2 : fg;
      for (const xx of [x, n - 1 - x]) {
        rects += `<rect x="${xx}" y="${y}" width="1" height="1" fill="${c}"/>`;
      }
    }
  }
  return `<svg viewBox="0 0 ${n} ${n}" preserveAspectRatio="none" width="100%" height="100%" shape-rendering="crispEdges">${rects}</svg>`;
}

let selectedItem = null;
let itemSendOpen = false;

// ---------------------------------------------------------------------------
// Media: bytes are checked, never trusted.
//
// An item commits `content = H(manifest)`, and the manifest names each file by
// its SHA-256 and nothing else. So a file may arrive from anywhere — a gateway,
// a peer at an event, a USB stick — and be checked against what was committed
// before it is shown. A wrong or hostile source is detected, not trusted.
//
// Fetching is OFF until a gateway is configured. Asking a server for the art of
// the item you own tells that server your IP, which item you hold, and when —
// the same disclosure that made us bundle the fonts (C23). Anything already in
// the local store renders with no network at all, which is why handing someone
// the file along with the item is the private path: it arrives once, by hand,
// and is never requested again.
const MEDIA_DB = 'nap-media';
// Where art can be fetched from, when the holder has said it may be. Empty
// until then: asking a host for the picture of an item tells that host which
// item you are looking at, which on a shielded chain is the one thing the
// chain itself will not reveal.
const DEFAULT_ART_GATEWAY = 'https://cavebro.win/blobs';
let mediaSources = [];
try { mediaSources = JSON.parse(localStorage.getItem('nap_media_sources') || '[]'); } catch (_) { mediaSources = []; }

// The toggle is the source of truth; `mediaSources` is derived from it, so
// turning art off empties the list rather than leaving a stale gateway behind.
function applyArtSetting() {
  const on = localStorage.getItem('nap_art') === '1';
  const base = (localStorage.getItem('nap_art_gateway') || '').trim() || DEFAULT_ART_GATEWAY;
  mediaSources = on ? [base] : [];
  return { on, base };
}
applyArtSetting();

function mediaStore(mode) {
  return new Promise((ok, no) => {
    const r = indexedDB.open(MEDIA_DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore('blobs');
    r.onerror = () => no(r.error);
    r.onsuccess = () => ok(r.result.transaction('blobs', mode).objectStore('blobs'));
  });
}

function cacheGet(hash) {
  return mediaStore('readonly').then((st) => new Promise((ok) => {
    const q = st.get(hash);
    q.onsuccess = () => ok(q.result || null);
    q.onerror = () => ok(null);
  })).catch(() => null);
}

function cachePut(hash, buf) {
  return mediaStore('readwrite').then((st) => { st.put(buf, hash); }).catch(() => {});
}

async function sha256Hex(buf) {
  const d = await crypto.subtle.digest('SHA-256', buf);
  return Array.from(new Uint8Array(d)).map((b) => b.toString(16).padStart(2, '0')).join('');
}

// Accept bytes for `hash` from any source. Returns true only if they are what
// was committed. This is the one door media comes through.
async function acceptMedia(hash, buf) {
  const plain = await sha256Hex(buf);
  const leaf = await contentHashHex(buf);
  if (plain !== hash && leaf !== hash) return false;
  await cachePut(hash, buf);
  return true;
}

// Local first, then each configured gateway in turn, verifying every time.
// `digest` says how the name is computed from the bytes: plain SHA-256 for
// media, leaf-prefixed for a manifest.
async function loadMediaRaw(hash, digest) {
  const hit = await cacheGet(hash);
  if (hit) return hit;
  for (const base of mediaSources) {
    try {
      const res = await fetch(base.replace(/\/$/, '') + '/' + hash);
      if (!res.ok) continue;
      const buf = await res.arrayBuffer();
      if (await digest(buf) === hash) { await cachePut(hash, buf); return buf; }
    } catch (_) { /* try the next one */ }
  }
  return null;
}

function loadMedia(hash) {
  return loadMediaRaw(hash, sha256Hex);
}

// A manifest is itself a blob, addressed by the content hash the item
// committed — so from 32 bytes on chain a wallet can find and check everything
// else with no registry and nothing to trust. `content` is hash_leaf(preimage),
// i.e. SHA-256 of 0x00 followed by the bytes, which is what separates a leaf
// from an internal node.
const MANIFEST_DOMAIN = 'zyn.manifest.v1';
const manifestCache = new Map();

async function contentHashHex(buf) {
  const pre = new Uint8Array(buf.byteLength + 1);
  pre[0] = 0x00;
  pre.set(new Uint8Array(buf), 1);
  return sha256Hex(pre);
}

function parseManifest(buf) {
  const v = new DataView(buf);
  const dec = new TextDecoder();
  let o = 0;
  const take = (n) => { const b = new Uint8Array(buf, o, n); o += n; return b; };
  const u32 = () => { const x = v.getUint32(o); o += 4; return x; };
  const u64 = () => { const x = Number(v.getBigUint64(o)); o += 8; return x; };
  const str = () => dec.decode(take(u32()));
  const hex = (b) => Array.from(b).map((x) => x.toString(16).padStart(2, '0')).join('');

  if (dec.decode(take(MANIFEST_DOMAIN.length)) !== MANIFEST_DOMAIN) throw new Error('not a manifest');
  const name = str();
  const description = str();
  const media = [];
  for (let i = 0, n = u32(); i < n; i++) {
    const role = v.getUint8(o); o += 1;
    const sha256 = hex(take(32));
    media.push({ role, sha256, mime: str(), bytes: u64(), width: u32(), height: u32(), duration_ms: u32() });
  }
  const attributes = [];
  for (let i = 0, n = u32(); i < n; i++) attributes.push({ trait_type: str(), value: str() });
  return { name, description, media, attributes };
}

// Fetch and check an item's manifest. Rejects one that does not hash to the
// content the chain committed, so a substituted description is caught.
async function loadManifest(content) {
  if (!content) return null;
  if (manifestCache.has(content)) return manifestCache.get(content);
  const buf = await loadMediaRaw(content, (b) => contentHashHex(b));
  if (!buf) return null;
  let m = null;
  try { m = parseManifest(buf); } catch (_) { m = null; }
  manifestCache.set(content, m);
  return m;
}

// Render an item's art into `el`, falling back to the identicon when the bytes
// are not here. The identicon is derived from the content hash, so it is always
// honest about which item it stands for even when the art is absent.
async function paintArt(el, item) {
  if (!el) return;
  const man = await loadManifest(item.content);
  if (!man) return;
  const media = man.media.find((m) => m.role === 0);
  if (!media) return;
  const buf = await loadMedia(media.sha256);
  if (!buf) return;
  if (buf.byteLength !== media.bytes) return;
  const url = URL.createObjectURL(new Blob([buf], { type: media.mime || 'application/octet-stream' }));
  if ((media.mime || '').startsWith('video/')) {
    el.innerHTML = `<video src="${url}" autoplay loop muted playsinline></video>`;
  } else {
    el.innerHTML = `<img src="${url}" alt="${esc(man.name || '')}">`;
  }
  el.dataset.verified = '1';
}

function renderItems(z) {
  const items = z.items || [];
  $('it-account-qr').innerHTML = z.account_qr_svg || '';
  setAddr('it-account', z.account || '', 16);
  // Settle the selection *before* drawing the grid. Choosing afterwards left
  // the first render showing a detail panel with nothing marked as chosen.
  if (selectedItem != null && !items.some((i) => i.asset === selectedItem)) selectedItem = null;
  if (selectedItem == null && items.length) selectedItem = items[0].asset;
  const grid = $('it-grid');
  grid.innerHTML = '';
  for (const it of items) {
    const d = document.createElement('div');
    d.className = 'nft' + (selectedItem === it.asset ? ' sel' : '');
    d.innerHTML = `<div class="art" data-art="${esc(it.asset)}">${identicon(it.content || '')}</div>` +
      `<div class="sym">${esc(it.symbol)}</div>` +
      `<div class="back">${it.redeem_price ? 'backed ' + esc(it.redeem_price) + ' ZEC' : 'no floor yet'}</div>`;
    d.onclick = () => { selectedItem = it.asset; itemSendOpen = false; renderItems(z); };
    grid.appendChild(d);
    // Verified art replaces the identicon only once the bytes check out.
    paintArt(d.querySelector('.art'), it);
  }
  $('it-count').textContent = items.length ? String(items.length) : '';
  $('it-empty').hidden = items.length > 0;
  $('it-empty').textContent = 'Nothing here yet. Anything you buy in the store appears here, visible only to this key.';

  const it = items.find((i) => i.asset === selectedItem);
  $('it-detail').hidden = !it;
  if (!it) { itemSendOpen = false; return; }

  const coll = (z.collections || []).find((c) => c.id === it.collection);
  $('it-sym').textContent = it.symbol;
  $('it-art').innerHTML = identicon(it.content || '', 7);
  paintArt($('it-art'), it);
  $('it-back').textContent = it.redeem_price ? `${it.redeem_price} ZEC` : 'not redeemable yet';
  $('it-coll').textContent = coll ? `${coll.symbol} (#${coll.id})` : '—';
  $('it-phase').textContent = coll ? coll.phase_name : '—';
  $('it-supply').textContent = coll ? `${coll.minted} of ${coll.cap} minted · ${coll.outstanding} outstanding` : '—';
  $('it-pool').textContent = coll ? `${coll.pool} ZEC behind ${coll.outstanding}` : '—';
  setAddr('it-asset', it.asset || '');
  setAddr('it-content', it.content || '');

  const sendPanel = $('it-send-panel');
  sendPanel.hidden = !itemSendOpen;
  $('it-send').onclick = () => {
    itemSendOpen = true;
    sendPanel.hidden = false;
    $('it-send-to').focus();
  };
  $('it-send-cancel').onclick = () => {
    itemSendOpen = false;
    sendPanel.hidden = true;
    $('it-send-to').value = '';
    $('it-send-force').checked = false;
  };
  $('it-send-review').onclick = () => run($('it-send-review'), 'Sending…', async () => {
    const to = $('it-send-to').value.trim();
    if (!/^[0-9a-fA-F]{64}$/.test(to)) throw new Error('recipient must be a 64-character Zyn account ID');
    const force = $('it-send-force').checked;
    const ok = await confirmDialog('Send this NFT?', [
      ['Item', `${it.symbol} · asset ${it.asset}`],
      ['To account', to],
      ['Amount', '1 whole NFT'],
      ['Settlement', force ? 'Zcash recovery path' : 'private on Zyn this epoch'],
      ['After sending', 'it leaves this wallet'],
    ]);
    if (!ok) return;
    const r = await api('/api/transfer', { to, asset: it.asset, amount: '1', force });
    logLine(force ? `NFT forced via Zcash in tx ${r.txid.slice(0, 12)}…` : `NFT sent on Zyn at seq ${r.seq}`);
    selectedItem = null;
    itemSendOpen = false;
    $('it-send-to').value = '';
    $('it-send-force').checked = false;
    toast('NFT sent');
  });

  // Redeem burns the item for its share of the pool. It needs no counterparty
  // and it cannot be undone, so it asks twice and says what it costs — a
  // single click on an irreversible action is a trap, however clear the label.
  const btn = $('it-redeem');
  const live = coll && coll.phase === 3 && it.redeem_price;
  btn.disabled = !live;
  btn.dataset.armed = '';
  btn.textContent = 'Redeem';
  $('it-redeem-hint').textContent = live
    ? `burns this item for ${it.redeem_price} ZEC`
    : (coll ? `not redeemable while the collection is ${coll.phase_name}` : 'this item has no collection behind it');
  btn.onclick = () => {
    if (btn.dataset.armed !== '1') {
      btn.dataset.armed = '1';
      btn.textContent = 'Confirm — this destroys it';
      $('it-redeem-hint').textContent = `you receive ${it.redeem_price} ZEC on Zyn; the item is gone`;
      return;
    }
    run(btn, 'Redeeming…', async () => {
      await api('/api/redeem', { asset: it.asset });
      selectedItem = null;
      toast('redeemed');
    });
  };
}

// copy button and for a click, so nothing a person needs is thrown away.
function setAddr(id, value, n = 12) {
  const el = $(id);
  if (el.dataset.full === value) return;
  el.dataset.full = value || '';
  el.title = value ? value + '  (click to show in full)' : '';
  el.classList.remove('full');
  el.textContent = short(value, n);
}
document.addEventListener('click', (ev) => {
  const c = ev.target.closest('.addr code');
  if (!c || !c.dataset.full) return;
  const full = c.classList.toggle('full');
  c.textContent = full ? c.dataset.full : short(c.dataset.full, 12);
});
function chip(id, cls, text) { const c = $(id); c.className = 'chip ' + cls; c.lastElementChild.textContent = text; }
function esc(s) { return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c])); }

// Tabs
for (const b of document.querySelectorAll('nav button')) b.onclick = () => showTab(b.dataset.tab);
function showTab(name) {
  for (const b of document.querySelectorAll('nav button')) b.classList.toggle('on', b.dataset.tab === name);
  for (const m of document.querySelectorAll('main')) m.hidden = m.dataset.tab !== name;
  try { localStorage.setItem('tab', name); } catch {}
  if (name === 'settings') loadZynKey().catch((e) => { $('zyn-key-status').textContent = e.message; });
}

function render(s) {
  state = s;
  const net = s.network, main = net === 'mainnet';
  $('netname').textContent = net;
  document.title = 'Nap Wallet — ' + net;
  $('unit').textContent = s.unit; $('send-unit').textContent = 'any ' + s.unit + ' address';
  renderHoldings(s);
  const pend = s.balance.pending > 0 ? ` · <span class="in-pending">${fmt(s.balance.pending)} ${s.unit} pending</span>` : '';
  const onZyn = Number((s.zyn && s.zyn.portfolio && s.zyn.portfolio.total_zec) || 0);
  $('bal-detail').innerHTML = onZyn > 0
    ? `${fmt(s.balance.total)} on Zcash · ${fmt(onZyn)} on Zyn${pend}`
    : `Ironwood ${fmt(s.balance.ironwood / 1e8)} · Orchard ${fmt(s.balance.orchard / 1e8)}${pend}`;
  const behind = s.synced_to ? s.tip - s.synced_to : null;
  $('synced').textContent = s.synced_to ? `synced to ${s.synced_to} · tip ${s.tip}` : `not synced · birthday ${s.birthday}`;
  chip('chip-sync', s.job && s.job.kind === 'sync' ? 'warn' : behind !== null && behind <= s.confirmations + 2 ? 'ok' : 'warn', s.job && s.job.kind === 'sync' ? 'syncing…' : behind === null ? 'not synced' : behind <= s.confirmations + 2 ? 'in sync' : `${behind} blocks behind · catching up`);
  const job = s.job;
  $('job').hidden = !job;
  if (job) {
    const p = job.progress;
    $('job-text').textContent = job.kind === 'sync' ? (p ? `scanning ${p.at.toLocaleString()} of ${p.to.toLocaleString()}` : 'syncing…') : job.kind === 'send' || job.kind === 'deposit' ? 'proving the payment (about a minute)…' : job.kind === 'shield' ? 'moving it in privately (about a minute)…' : job.kind + '…';
    $('job-bar').hidden = !(job.kind === 'sync' && p);
    if (p) $('job-bar').firstElementChild.style.width = Math.min(100, Math.round(100 * (p.at - s.birthday) / Math.max(1, p.to - s.birthday))) + '%';
  }
  for (const b of ['sync', 'send', 'deposit', 'withdraw', 'bind', 'zt-send', 'import', 'reset', 'save-settings', 't-shield']) $(b).disabled = !!job || ((b === 'withdraw' || b === 't-shield') && $(b).disabled);
  setAddr('address', s.address);
  $('t-recv').hidden = !s.transparent_address;
  if (s.transparent_address) setAddr('t-address', s.transparent_address);
  $('qr').innerHTML = s.qr_svg || '';
  $('faucet-hint').textContent = main ? 'Shielded (Orchard + Ironwood) unified address. Private from the moment it lands.' : 'Testnet. No TAZ yet? Paste the address into a testnet faucet.';
  $('birthday').textContent = s.birthday;

  // Activity
  const h = $('history'); h.innerHTML = '';
  $('activity-count').textContent = s.history.length ? `${s.history.length} entries` : '';
  if (!s.history.length) h.innerHTML = '<div class="empty">Nothing yet. Receive some ' + s.unit + ' to begin.</div>';
  for (const e of s.history) h.insertAdjacentHTML('beforeend', entryHtml(e, s));

  // Notes (settings → about)
  const nb = $('notes').querySelector('tbody'); nb.innerHTML = '';
  if (!s.notes.length) nb.innerHTML = '<tr><td colspan="4" class="muted">No spendable notes.</td></tr>';
  for (const n of s.notes) nb.insertAdjacentHTML('beforeend', `<tr><td class="mono">${short(n.txid, 8)}</td><td>${n.pool}</td><td class="r num">${fmt(n.zec)}</td><td class="r num">${n.height}</td></tr>`);
  $('about').textContent = `Nap Wallet · ${net} · block server ${main ? s.settings.lightd_mainnet : s.settings.lightd_testnet}` + (s.zyn.available ? ` · Zyn node ${s.zyn.node} chain ${s.zyn.chain}` : '');

  // Settings
  for (const r of document.querySelectorAll('input[name=net]')) r.checked = r.value === s.settings.network;
  if (document.activeElement !== $('lightd-testnet')) $('lightd-testnet').value = s.settings.lightd_testnet;
  if (document.activeElement !== $('lightd-mainnet')) $('lightd-mainnet').value = s.settings.lightd_mainnet;
  if (document.activeElement !== $('set-price')) $('set-price').checked = s.settings.price !== false;
  if (document.activeElement !== $('set-art')) $('set-art').checked = localStorage.getItem('nap_art') === '1';
  if (document.activeElement !== $('art-gateway')) {
    $('art-gateway').value = localStorage.getItem('nap_art_gateway') || '';
  }

  // Zyn
  const z = s.zyn;
  $('zyn-off').hidden = z.available; $('zyn-account').hidden = !z.available; $('zyn-move').hidden = !z.available; $('zyn-send').hidden = !z.available;
  if (!z.available) { $('zyn-why').textContent = z.why || ''; chip('chip-zyn', 'warn', 'zyn · mainnet later'); return; }
  $('zyn-exit').textContent = z.exit && z.exit.epoch !== null && z.exit.epoch !== undefined
    ? `Exit proof kept for epoch ${z.exit.epoch}${z.exit.verified ? ' ✓' : ' (does not verify)'} — enough to leave even if the sequencer disappears.`
    : 'Exit proof: none yet — kept automatically after the first anchor reaches Zcash.';
  const st = z.status;
  chip('chip-zyn', st.ok ? 'ok' : 'bad', st.ok ? `zyn · epoch ${st.epoch}` : 'zyn node down');
  $('epoch').textContent = st.ok ? `chain ${z.chain} · epoch ${st.epoch}` : (st.error || '');
  setAddr('account', z.account);
  // One shielded address per account: paying it from any wallet is a deposit,
  // with nothing to remember and no memo to get wrong. Older nodes cannot
  // issue one, and then the shared vault address plus the memo is still it.
  const dep = z.deposit_address || null;
  // The fallback used to hand over the shared vault address as an ordinary
  // copyable string. A deposit there is attributed by *memo*, and no faucet or
  // exchange sets one — the funds arrive, sit unattributed, and wait for an
  // operator to place them by hand. That is what happened to a 0.1 TAZ test
  // deposit on 16 Sep (§71.39). So the bare address is no longer offered as
  // though it were safe to paste anywhere; the memo travels with it.
  setAddr('dep-addr', dep || z.vault);
  $('dep-addr').classList.toggle('needs-memo', !dep);
  $('dep-memo-warning').hidden = !!dep;
  $('dep-memo-line').textContent = dep ? '' : 'ZYN1:' + (z.account || '');
  renderPortfolio(z);
  renderMarket(z);
  renderItems(z);
  $('dep-qr').innerHTML = z.deposit_qr_svg || '';
  $('dep-uri').value = z.deposit_uri || '';
  $('dep-how').textContent = dep
    ? 'This address is yours alone. Anything sent to it from any shielded Zcash wallet is credited to this account — no memo needed. Deposit below pays it from this wallet.'
    : 'This node issues no per-account address, so deposits go to the shared vault and must carry the memo naming your account. Deposit below attaches it for you.';

// Whether more of a token can ever exist. Shown only when the answer is yes:
// a fixed-supply token is the ordinary case and needs no badge, while a
// mintable one is a fact a holder should not have to go looking for.
function mintBadge(assetId) {
  const a = (z.assets || []).find((x) => x.id === assetId);
  if (!a || !a.supply_policy || a.supply_policy === 'fixed') return '';
  const title = a.supply_policy === 'capped'
    ? `More can be issued, up to a cap of ${esc(String(a.supply_cap || '?'))}`
    : 'The issuer can mint more of this at any time, without limit';
  return ` <span class="badge warn" title="${title}">mintable</span>`;
}

  const r = z.record;
  const bb = $('balances').querySelector('tbody'); bb.innerHTML = '';
  for (const b of r.spendable) bb.insertAdjacentHTML('beforeend', `<tr><td>${b.symbol}${mintBadge(b.asset)}</td><td class="r num">${fmt(b.amount)}</td><td></td></tr>`);
  for (const b of r.unreleased) bb.insertAdjacentHTML('beforeend', `<tr><td>${b.symbol}${mintBadge(b.asset)}</td><td class="r num">${fmt(b.amount)}</td><td><span class="tag pending">arriving · epoch ${b.epoch}</span></td></tr>`);
  for (const b of r.exiting) bb.insertAdjacentHTML('beforeend', `<tr><td>${b.symbol}</td><td class="r num">${fmt(b.amount)}</td><td><span class="tag exit">leaving · since ${b.since}</span></td></tr>`);
  if (!bb.children.length) bb.innerHTML = `<tr><td colspan="3" class="muted">${r.known ? 'Nothing held on Zyn.' : 'Zyn has not seen this account yet. A deposit creates it.'}</td></tr>`;
  const bound = r.bound_here;
  $('bind-state').textContent = bound ? `Withdrawals go to this wallet${z.binding && z.binding.revealed ? '' : ' (reveal pending)'}.` : r.bound ? 'This account is bound elsewhere; rebinding waits out a delay.' : 'Bind once; then a withdrawal is just an amount. The chain holds only a commitment to your address.';
  $('bind').textContent = bound ? 'Bound to this wallet ✓' : 'Bind this wallet as the exit destination';
  $('withdraw').disabled = !bound;
  const sel = $('zt-asset'); const cur = sel.value;
  sel.innerHTML = z.assets.map(a => `<option value="${a.id}">${a.symbol}</option>`).join('');
  if (cur) sel.value = cur;

  const lg = $('log');
  if (lg.dataset.n !== String(s.log.length)) { lg.innerHTML = ''; for (const l of s.log) { const li = document.createElement('li'); li.textContent = l; li.className = /^error/.test(l) ? 'err' : ''; lg.prepend(li); } lg.dataset.n = String(s.log.length); }
}

function entryHtml(e, s) {
  const inb = e.kind === 'received';
  const when = e.at ? new Date(e.at * 1000).toLocaleString() : '';
  const status = e.height ? `height ${e.height}` : '<span class="tag pending">pending</span>';
  const title = inb ? 'Received' : e.kind === 'deposit' ? 'Deposit to Zyn' : e.to ? `Sent to ${short(e.to, 8)}` : 'Sent (found on rescan; fee included)';
  const tx = window.__TAURI__ ? `<button data-copy="${esc(e.txid)}">${short(e.txid, 8)}</button>` : `<a href="${s.explorer}${esc(e.txid)}" target="_blank" rel="noopener">${short(e.txid, 8)}</a>`;
  const meta = `${when} · ${status} · ${tx}`;
  const memo = e.memo && e.memo.length > 44 && !/\s/.test(e.memo) ? short(e.memo, 16) : e.memo;
  return `<div class="item"><div><div class="t">${title}</div>${memo ? `<div class="m" title="${esc(e.memo)}">“${esc(memo)}”</div>` : ''}<div class="s">${meta}</div></div><div class="amt ${inb ? 'in' : 'out'}">${inb ? '+' : '−'}${fmt(e.zec)} ${s.unit}</div></div>`;
}
document.addEventListener('click', (ev) => { const b = ev.target.closest('button[data-copy]'); if (b) navigator.clipboard.writeText(b.dataset.copy).then(() => toast('txid copied')).catch(() => {}); });

// Identifiers are 32-byte derived addresses (§66.9), not small integers. The
// previous version coerced with Number() and filtered on Number.isInteger, which
// discards every hex id and left the field permanently invalid.
const ID_RE = /^[0-9a-f]{64}$/;
function assetId(value) {
  const v = String(value || '').trim().toLowerCase();
  if (!ID_RE.test(v)) throw new Error('expected a 32-byte id as 64 hex characters');
  return v;
}
function ids(value) {
  const out = value.split(',').map(x => String(x).trim().toLowerCase()).filter(Boolean);
  if (!out.length || out.some(x => !ID_RE.test(x)) || new Set(out).size !== out.length) {
    throw new Error('enter unique 64-character hex ids separated by commas');
  }
  return out;
}
function rawAmount(raw) {
  const n = BigInt(raw), wad = 1000000000000000000n;
  const whole = n / wad, frac = String(n % wad).padStart(18, '0').replace(/0+$/, '');
  return frac ? `${whole}.${frac}` : String(whole);
}
async function loadAgents() {
  let result;
  try { result = await api('/api/agent/mandates'); } catch { return; }
  const list = $('agent-list'); list.innerHTML = '';
  $('agent-count').textContent = `${result.mandates.length} total`;
  if (!result.mandates.length) { list.innerHTML = '<div class="empty">No agent has authority.</div>'; return; }
  for (const m of result.mandates) {
    const limits = m.max_per_action_raw.map(x => `${rawAmount(x.amount)} of asset ${x.asset}`).join(', ');
    list.insertAdjacentHTML('beforeend', `<div class="item"><div><div class="t">${esc(m.state)} · until epoch ${m.valid_until_epoch}</div><div class="m">assets ${m.allowed_assets.join(', ')} · pools ${m.allowed_pools.join(', ')} · max ${esc(limits)} · slippage ${m.max_slippage_bps / 100}%</div><div class="s">${esc(m.id)}</div></div><div class="row">${m.state === 'active' ? `<button class="btn small" data-agent-pause="${esc(m.id)}">Pause</button>` : ''}${m.state !== 'closed' ? `<button class="btn small danger" data-agent-close="${esc(m.id)}">Close</button>` : ''}</div></div>`);
  }
}

async function load() {
  try {
    const o = await api('/api/overview');
    render(o);
    loadAgents();
    // Keep up on our own: if the scan has fallen behind and nothing else is
    // running, start a sync in the background and let the poll pick it up.
    const behind = o.synced_to ? o.tip - o.synced_to : Infinity;
    if (!o.job && !busy && behind > o.confirmations + 2) { api('/api/sync', {}).catch(() => {}); }
  } catch (e) { chip('chip-sync', 'bad', 'app unreachable'); toast(e.message, true); }
}

$('sync').onclick = () => run($('sync'), 'Syncing…', async () => { await api('/api/sync', {}); });
$('send-max').onclick = () => { if (state) $('send-amount').value = fmt(Math.max(0, state.balance.total - state.fee_estimate)); };
$('send').onclick = () => run($('send'), 'Sending…', async () => {
  const to = $('send-to').value.trim(), amount = $('send-amount').value.trim(), memo = $('send-memo').value;
  if (!to || !amount) throw new Error('recipient and amount are required');
  const ok = await confirmDialog('Send ' + state.unit, [['To', to], ['Amount', `${amount} ${state.unit}`], ['Fee', `about ${fmt(state.fee_estimate)} ${state.unit}`], ['Memo', memo || '(none)']]);
  if (!ok) return;
  await api('/api/send', { to, amount, memo });
  $('send-amount').value = ''; $('send-memo').value = ''; $('send-to').value = '';
});
$('deposit').onclick = () => run($('deposit'), 'Sending…', async () => {
  const amount = $('dep-amount').value.trim();
  if (!amount) throw new Error('amount is required');
  const ok = await confirmDialog('Deposit to Zyn', [['Amount', `${amount} ${state.unit}`], ['To', 'the Zyn vault'], ['Memo', state.zyn.memo], ['Credit', 'as ZEC.zy after 3 confirmations and one epoch']]);
  if (!ok) return;
  await api('/api/deposit', { amount });
  $('dep-amount').value = '';
});
$('import').onclick = () => run($('import'), 'Restoring…', async () => {
  const secret = $('import-key').value.trim();
  if (!secret) throw new Error('a recovery phrase, backup, or spending key is required');
  const isBackup = secret.startsWith('{'), isMnemonic = !isBackup && secret.split(/\s+/).length > 1;
  const birthdayText = $('import-birthday').value.trim();
  if (!isBackup && !birthdayText) throw new Error('birthday height is required for a phrase or spending key');
  const birthday = birthdayText ? Number(birthdayText) : null;
  if (birthday !== null && (!Number.isSafeInteger(birthday) || birthday < 0)) throw new Error('birthday must be a block height');
  const accountText = $('import-account').value.trim();
  const account = accountText ? Number(accountText) : 0;
  if (!Number.isSafeInteger(account) || account < 0 || account >= 2 ** 31) throw new Error('account must be a number below 2^31');
  let body;
  if (isBackup) {
    try { body = { backup: JSON.parse(secret) }; } catch (_) { throw new Error('backup JSON is invalid'); }
  } else if (isMnemonic) {
    body = { mnemonic: secret, passphrase: $('import-passphrase').value, account, birthday };
  } else {
    body = { key: secret, birthday };
  }
  const kind = isBackup ? 'Nap backup' : isMnemonic ? `BIP-39 phrase, account ${account}` : 'legacy spending key';
  const shownBirthday = isBackup ? 'from backup' : String(birthday);
  const ok = await confirmDialog('Restore a wallet', [['Network', state.network], ['Source', kind], ['Birthday', shownBirthday], ['Replaces', 'the current, empty wallet for this network']]);
  if (!ok) return;
  const r = await api('/api/import', body);
  logLine('restored; address ' + short(r.address, 12) + (r.warning ? ' · WARNING: ' + r.warning : ''));
  $('import-key').value = ''; $('import-passphrase').value = '';
});
$('bind').onclick = () => run($('bind'), 'Binding…', async () => { await api('/api/bind', {}); logLine('bound and revealed'); });
$('withdraw').onclick = () => run($('withdraw'), 'Submitting…', async () => {
  const amount = $('wd-amount').value.trim();
  const ok = await confirmDialog('Withdraw from Zyn', [['Amount', `${amount} ZEC.zy`], ['To', 'this wallet (' + short(state.address, 12) + ')'], ['Paid', 'shielded, at the end of the epoch']]);
  if (!ok) return;
  const r = await api('/api/withdraw', { amount }); logLine(`withdrawal requested at seq ${r.seq}; it is paid at the end of the epoch`); $('wd-amount').value = '';
});
$('zt-send').onclick = () => run($('zt-send'), 'Sending…', async () => {
  const to = $('zt-to').value.trim(), amount = $('zt-amount').value.trim(), sym = $('zt-asset').selectedOptions[0]?.textContent || '';
  const ok = await confirmDialog('Send on Zyn', [['To account', to], ['Amount', `${amount} ${sym}`], ['Settles', 'this epoch, privately']]);
  if (!ok) return;
  const force = $('zt-force').checked;
  const r = await api('/api/transfer', { to, asset: assetId($('zt-asset').value), amount, force }); logLine(force ? `forced via Zcash in tx ${r.txid.slice(0, 12)}… — the node has 20 blocks` : `sent on Zyn at seq ${r.seq}`); $('zt-amount').value = ''; $('zt-force').checked = false;
});
$('save-settings').onclick = () => run($('save-settings'), 'Saving…', async () => { const net = document.querySelector('input[name=net]:checked').value; await api('/api/settings', { network: net, lightd_testnet: $('lightd-testnet').value, lightd_mainnet: $('lightd-mainnet').value, price: $('set-price').checked }); 
  localStorage.setItem('nap_art', $('set-art').checked ? '1' : '0');
  localStorage.setItem('nap_art_gateway', $('art-gateway').value.trim());
  const art = applyArtSetting();
  logLine('settings saved; now on ' + net + (art.on ? ' · art from ' + art.base : ' · art off'));
  if (state) render(state);
});
$('agent-create').onclick = () => run($('agent-create'), 'Opening…', async () => {
  const allowed_assets = ids($('agent-assets').value), allowed_pools = ids($('agent-pools').value);
  const asset = assetId($('agent-input-asset').value), amount = $('agent-max').value.trim();
  const max_slippage_bps = Math.round(Number($('agent-slip').value) * 100), valid_for_epochs = Number($('agent-epochs').value);
  if (!amount || !Number.isInteger(valid_for_epochs)) throw new Error('maximum amount and whole epochs are required');
  const ok = await confirmDialog('Open agent mandate', [['Can do', 'exact-input swaps only'], ['Assets', allowed_assets.join(', ')], ['Pools', allowed_pools.join(', ')], ['Maximum per swap', `${amount} of asset ${asset}`], ['Maximum slippage', `${max_slippage_bps / 100}%`], ['Expires', `after ${valid_for_epochs} epochs`], ['Cannot do', 'transfer, withdraw, add liquidity, bind, delegate, export keys, or change settings']]);
  if (!ok) return;
  await api('/api/agent/mandates', { allowed_assets, allowed_pools, max_per_action_raw: [{ asset, amount }], max_slippage_bps, valid_for_epochs });
  await loadAgents();
  logLine('agent mandate opened');
});
document.addEventListener('click', async (ev) => {
  const pause = ev.target.closest('[data-agent-pause]'), close = ev.target.closest('[data-agent-close]');
  const button = pause || close; if (!button) return;
  const action = close ? 'close' : 'pause', mandate_id = close ? close.dataset.agentClose : pause.dataset.agentPause;
  const ok = await confirmDialog(action === 'close' ? 'Close mandate' : 'Pause mandate', [['Mandate', mandate_id], ['Local effect', 'Nap stops signing immediately'], ['On chain', 'the signed session still expires at its displayed epoch']]);
  if (!ok) return;
  await run(button, action === 'close' ? 'Closing…' : 'Pausing…', async () => { await api('/api/agent/' + action, { mandate_id }); await loadAgents(); });
});
$('export').onclick = () => run($('export'), '…', async () => {
  const r = await api('/api/export', {}); $('key').hidden = false;
  $('key').textContent = r.mnemonic ? `Recovery phrase:\n${r.mnemonic}\n\n${r.backup ? 'Portable backup:\n' + r.backup : 'Your independent Zyn key is NOT recovered by these words. Use Save complete backup; that secret is written directly to a private file.'}` : 'Legacy wallet: no phrase exists for this key. Use Save complete backup to save both identities directly to a private file.';
});
$('download-backup').onclick = () => run($('download-backup'), 'Preparing…', async () => {
  if (!await confirmDialog('Save complete recovery backup', [['Contains', 'Both Zcash and Zyn recovery secrets'], ['Location', 'A private file on the device running Nap'], ['Next step', 'Move a copy to secure offline storage; never upload it to a bug report']])) return;
  const r = await api('/api/export-file', { confirm: true });
  $('key').hidden = false;
  $('key').textContent = `Complete backup saved on the device running Nap:\n${r.path}\n\nKeep this file secret. Copy it to secure offline storage. The page never received the independent Zyn seed.`;
});
async function loadZynKey() {
  const r = await api('/api/zyn-key');
  const legacy = r.source === 'legacy-ed25519';
  $('zyn-key-status').textContent = legacy
    ? 'This existing wallet still uses an independent Zyn key. Its seed is included in the portable backup; the recovery phrase alone does not restore this account.'
    : 'This Zyn account is derived from the same recovery phrase as Nap. Browser and desktop restore the same account.';
  $('migrate-zyn').hidden = !r.migration_available;
  const lines = [
    `Current account: ${r.account}`,
    r.derived_account ? `Phrase-derived account: ${r.derived_account}` : 'Phrase-derived account: unavailable for a legacy Zcash key',
    `Transferable holdings: ${r.transferable.length}`,
    `Pending exit/release records: ${r.exiting + r.unreleased}`,
    `Creator authorities that cannot move: ${r.non_transferable_creator_authorities.length}`,
    r.migration_blocked_reason || '',
  ];
  $('zyn-key-review').textContent = lines.join('\n');
  $('zyn-key-review').hidden = !legacy;
}
$('migrate-zyn').onclick = () => run($('migrate-zyn'), 'Migrating…', async () => {
  const r = await api('/api/zyn-key');
  const ok = await confirmDialog('Move to phrase-derived Zyn account', [
    ['From', r.account], ['To', r.derived_account],
    ['Movable holdings', String(r.transferable.length)],
    ['Creator authorities', `${r.non_transferable_creator_authorities.length} stay on the old key`],
    ['Legacy key', 'Preserved as zyn.key.legacy; keep it backed up'],
    ['Agent sessions', 'Cleared and must be recreated'],
  ]);
  if (!ok) return;
  const moved = await api('/api/zyn-key/migrate', { confirm: true });
  logLine(`migrated Zyn account to ${moved.to.slice(0, 12)}…; moved ${moved.moved.length} holdings`);
  await loadZynKey();
});
$('reset').onclick = () => run($('reset'), 'Resetting…', async () => { await api('/api/reset', { birthday: +$('reset-height').value }); logLine('rescanning from ' + $('reset-height').value); });
// The amount is whatever is in the box when you copy, so the link a person
// sends to their phone already says how much.
$('copy-dep-uri').onclick = () => {
  let uri = $('dep-uri').value;
  const a = parseFloat($('dep-amount').value);
  if (uri && a > 0) uri += '&amount=' + String(a);
  navigator.clipboard.writeText(uri).then(() => toast('payment link copied')).catch(() => {});
};
// Unlike everything else on this tab, the doormat is a lookup over the
// network rather than local state, so it is fetched when the drawer opens
// instead of on every render tick.
async function loadTransparent() {
  const d = $('t-recv');
  if (!d.open) return;
  $('t-wait').hidden = true;
  $('t-state').textContent = 'Checking…';
  let t;
  try { t = await api('/api/transparent'); } catch (e) { $('t-state').textContent = e.message; return; }
  const unit = $('unit').textContent || 'ZEC';
  if (!t.total) { $('t-state').textContent = 'Nothing waiting here.'; return; }
  $('t-state').textContent = '';
  $('t-wait').hidden = false;
  $('t-total').textContent = fmt(t.total) + ' ' + unit + ' waiting';
  // net === 0 means the fee is worth more than the money. Say so plainly
  // rather than offering a button that would spend it all on the fee.
  const dead = !t.net;
  $('t-net').textContent = dead
    ? 'Too small to move — the network fee would take all of it.'
    : fmt(t.net) + ' ' + unit + ' arrives after the ' + fmt(t.fee) + ' ' + unit + ' network fee.';
  $('t-shield').disabled = dead;
  $('t-shield').dataset.net = dead ? '' : fmt(t.net) + ' ' + unit;
}
$('t-recv').addEventListener('toggle', loadTransparent);
$('t-shield').onclick = async () => {
  const ok = await confirmDialog('Make it private', [
    ['Moving', $('t-total').textContent.replace(' waiting', '')],
    ['You receive', $('t-shield').dataset.net || '—'],
    ['To', $('address').dataset.full || ''],
  ]);
  if (!ok) return;
  await run($('t-shield'), 'Moving in…', async () => { await api('/api/shield', {}); });
  await loadTransparent();
};
for (const [b, src] of [['copy-addr', 'address'], ['copy-acct', 'account'], ['copy-dep', 'dep-addr'], ['copy-memo', 'dep-memo-line'], ['copy-content', 'it-content'], ['copy-taddr', 't-address'], ['copy-it-account', 'it-account']]) $(b).onclick = () => navigator.clipboard.writeText($(src).dataset.full || $(src).textContent).then(() => toast('copied')).catch(() => {});

try { const t = localStorage.getItem('tab'); if (t) showTab(t); } catch {}
load();
setInterval(() => { if (!busy) load(); }, 20000);
