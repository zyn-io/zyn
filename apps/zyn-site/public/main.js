// The ledger as strata: recent epochs from ZynScan, newest on top, each
// anchored layer pinned down to the Zcash block that carries its root.
// Everything placed in markup is a checked number, a 64-hex string or escaped.
const SCAN = { mainnet: 'https://zyn-rpc.cavebro.win/scan', testnet: 'https://zyn-rpc.cavebro.win/scan-testnet' };
const CHAIN = { mainnet: 26460, testnet: 212 };
const LAYERS = 8;
const TONES = ['#1b3a31', '#16302a', '#1f4035', '#122822', '#20382f', '#0f231d', '#1a342c', '#14291f'];

const $ = (id) => document.getElementById(id);
const hex64 = (h) => typeof h === 'string' && /^[0-9a-f]{64}$/.test(h);
const short = (h) => (hex64(h) ? `${h.slice(0, 8)}…${h.slice(-4)}` : '—');
const num = (n) => (Number.isFinite(n) ? n.toLocaleString('en-US') : '—');
const int = (n) => (Number.isSafeInteger(n) ? n : 0);

function ago(unix) {
  const s = Math.max(0, Math.round(Date.now() / 1000 - unix));
  if (s < 90) return `${s}s ago`;
  if (s < 5400) return `${Math.round(s / 60)}m ago`;
  if (s < 172800) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

async function get(net, path) {
  const r = await fetch(SCAN[net] + path, { cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer' });
  if (!r.ok) throw new Error(String(r.status));
  return r.json();
}

const epochLink = (net, n) => `https://zynscan.pages.dev/epoch/${int(n)}${net === 'testnet' ? '?network=testnet' : ''}`;

function drawPins() {
  const core = $('core');
  core.querySelectorAll('.pin').forEach((p) => p.remove());
  const bedrock = core.querySelector('.bedrock');
  const layers = [...core.querySelectorAll('.layer[data-anchored]')];
  const width = core.clientWidth;
  layers.forEach((layer, i) => {
    const pin = document.createElement('span');
    pin.className = 'pin';
    const x = Math.round(width * (0.56 + 0.03 * (i % 8)));
    const top = layer.offsetTop + layer.offsetHeight / 2;
    pin.style.left = `${x}px`;
    pin.style.top = `${top}px`;
    pin.style.height = `${bedrock.offsetTop - top + 22}px`;
    core.appendChild(pin);
  });
}

// Colour a JSON value for the terminal; strings are escaped before markup.
const escHtml = (v) => String(v).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
function json(v, pad = '') {
  if (v === null) return '<span class="n">null</span>';
  if (typeof v === 'number' || typeof v === 'boolean') return `<span class="n">${escHtml(v)}</span>`;
  if (typeof v === 'string') return `<span class="s">"${escHtml(v)}"</span>`;
  const inner = pad + '  ';
  if (Array.isArray(v)) return `[\n${v.map((x) => inner + json(x, inner)).join(',\n')}\n${pad}]`;
  return `{\n${Object.entries(v).map(([k, x]) => `${inner}<span class="k">"${escHtml(k)}"</span>: ${json(x, inner)}`).join(',\n')}\n${pad}}`;
}

let shown = null;
async function show(net) {
  shown = net;
  document.querySelectorAll('.switch button').forEach((b) => b.setAttribute('aria-selected', String(b.dataset.net === net)));
  try {
    const [summary, epochs, txs] = await Promise.all([get(net, '/api/summary'), get(net, '/api/epochs'), get(net, '/api/txs').catch(() => [])]);
    if (shown !== net) return;
    const agrees = summary.agrees === true && summary.replayed_root === summary.node_root;
    const anchoredThrough = Number.isFinite(summary.anchored_through) ? `anchored through <b>${num(summary.anchored_through)}</b>` : 'not yet anchored';
    $('readout').innerHTML = `<span><b>${net === 'testnet' ? 'Testnet' : 'Mainnet'}</b></span><span>chain ${CHAIN[net]}</span><span>epoch <b>${num(summary.epoch)}</b></span><span><b>${num(summary.transactions)}</b> transactions</span><span>${anchoredThrough}</span><span class="${agrees ? 'ok' : ''}">replay ${agrees ? 'agrees' : 'disagrees'}</span>`;

    const rows = (Array.isArray(epochs) ? epochs : []).filter((e) => e.sealed).sort((a, b) => b.epoch - a.epoch).slice(0, LAYERS);
    // Built with DOM calls: the page's CSP allows no inline style attributes.
    const box = $('layers');
    box.replaceChildren(...rows.map((e, i) => {
      const anchored = e.anchor && Number.isFinite(e.anchor.zcash_height);
      const layer = document.createElement('a');
      layer.className = 'layer';
      layer.href = epochLink(net, e.epoch);
      if (anchored) layer.dataset.anchored = '';
      layer.style.minHeight = `${Math.round(34 + 16 * Math.log2(1 + Math.max(0, int(e.txs))))}px`;
      layer.style.setProperty('--tone', TONES[int(e.epoch) % TONES.length]);
      layer.style.animationDelay = `${(i * 0.07).toFixed(2)}s`;
      layer.innerHTML = `<span><span class="ep">EPOCH ${num(e.epoch)}</span> · ${num(e.txs)} tx · ${e.sealed_at ? ago(e.sealed_at) : ''} <span class="root">· ${short(e.root)}</span></span>
        ${anchored ? `<span class="pinned">block ${num(e.anchor.zcash_height)}</span>` : '<span class="loose">not yet anchored</span>'}`;
      return layer;
    }));
    const heights = rows.filter((e) => e.anchor && Number.isFinite(e.anchor.zcash_height)).map((e) => e.anchor.zcash_height);
    $('bedrock-range').textContent = heights.length ? `blocks ${num(Math.min(...heights))} – ${num(Math.max(...heights))}` : 'no anchors yet';
    requestAnimationFrame(() => setTimeout(drawPins, 50));

    const a = rows.find((e) => e.anchor && hex64(e.anchor.zcash_txid));
    $('trace').innerHTML = a
      ? `Epoch ${num(a.epoch)} applied ${num(a.txs)} transaction${a.txs === 1 ? '' : 's'} and sealed to root <code>${short(a.root)}</code>, written into Zcash ${net} block ${num(a.anchor.zcash_height)} by transaction <code>${short(a.anchor.zcash_txid)}</code>.`
      : `No ${net} epoch is anchored to Zcash yet. Its roots are sealed on Zyn and replayed by ZynScan.`;
    // The live API panel: the newest sealed epoch, exactly as the API returned it.
    const newest = rows[0];
    $('api-net').textContent = net === 'testnet' ? 'scan-testnet' : 'scan';
    $('api-out').innerHTML = newest ? json(newest) : '[]';
    const kinds = [];
    for (const t of Array.isArray(txs) ? txs : []) if (t.ok && !kinds.includes(t.kind)) kinds.push(t.kind);
    $('recent').textContent = kinds.length ? `Recent: ${kinds.slice(0, 6).join(' / ')}` : '';
  } catch {
    if (shown !== net) return;
    $('readout').innerHTML = '<span>ZynScan is not answering right now</span>';
    $('layers').innerHTML = '';
    $('trace').textContent = 'The layers fill in when ZynScan answers.';
    $('recent').textContent = '';
  }
}

document.querySelectorAll('.switch button').forEach((b) => b.addEventListener('click', () => show(b.dataset.net)));
addEventListener('resize', () => drawPins());

// The product showcase: one real screen at a time.
const tabs = [...document.querySelectorAll('.tabs button')];
function pick(btn) {
  tabs.forEach((b) => b.setAttribute('aria-selected', String(b === btn)));
  document.querySelectorAll('.shot').forEach((img) => img.classList.toggle('is-on', img.dataset.shot === btn.dataset.shot));
  $('shot-url').textContent = btn.dataset.url;
  $('shot-link').href = btn.dataset.href;
}
tabs.forEach((b) => b.addEventListener('click', () => pick(b)));
show('testnet');

// Sections below the first screen settle in as they arrive. Only elements
// that start off-screen are held back, so nothing on screen at load, in a
// thumbnail or with motion reduced is ever hidden.
if (!matchMedia('(prefers-reduced-motion: reduce)').matches && 'IntersectionObserver' in window) {
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting) { e.target.classList.remove('pre'); io.unobserve(e.target); }
  }, { rootMargin: '0px 0px -8% 0px' });
  document.querySelectorAll('.reveal').forEach((el, i) => {
    if (el.getBoundingClientRect().top > innerHeight) {
      el.classList.add('pre');
      if (el.matches('.tiles li')) el.style.transitionDelay = `${(i % 3) * 0.08}s`;
      io.observe(el);
    }
  });
}

let glideTo = null; // set by the section lock below, used by in-page links

// Section lock. The page never comes to rest across two sections: when
// scrolling stops with a section boundary inside the screen, it finishes the
// move in the direction you were going (down: the next section fills the
// screen; up: back to the previous one, its start if it fits, else its end).
// Inside a section taller than the screen nothing happens until its edge.
// The glide is its own eased animation, identical in every browser, and any
// new wheel, touch or key input stops it at once.
if (!matchMedia('(prefers-reduced-motion: reduce)').matches) {
  const SECTIONS = '.hero, .strata, .product, .api, .index, .trust-wrap, .faq';
  const maxY = () => document.documentElement.scrollHeight - innerHeight;
  const tops = () => [...document.querySelectorAll(SECTIONS)].map((el) => Math.round(el.getBoundingClientRect().top + scrollY));
  let lastY = scrollY;
  let dir = 1;
  let gliding = false;
  let glideDir = 0;
  let touching = false;
  let timer = 0;

  function target(y) {
    const H = innerHeight;
    const t = tops();
    const inside = t.filter((e) => e > y + 2 && e < y + H - 2);
    if (!inside.length) return null;
    if (dir > 0) return Math.min(inside[0], maxY());
    const e = inside[inside.length - 1];
    const prev = t.filter((x) => x < e).pop() ?? 0;
    // a section not much taller than the screen is entered at its start
    return e - prev <= H * 1.25 ? prev : e - H;
  }

  function glide(to, done) {
    const from = scrollY;
    const d = to - from;
    if (Math.abs(d) < 2) { if (done) done(); return; }
    const dur = Math.min(720, 320 + Math.abs(d) * 0.5);
    const t0 = performance.now();
    gliding = true;
    glideDir = Math.sign(d);
    const step = (now) => {
      if (!gliding) return;
      const k = Math.min(1, (now - t0) / dur);
      const e = 1 - (1 - k) ** 3;
      scrollTo({ top: from + d * e, behavior: 'instant' });
      if (k < 1) requestAnimationFrame(step);
      else { gliding = false; lastY = scrollY; if (done) done(); }
    };
    requestAnimationFrame(step);
  }

  function rest() {
    if (gliding || touching) return;
    const to = target(scrollY);
    if (to !== null) glide(to);
  }

  addEventListener('scroll', () => {
    if (gliding) return;
    const y = scrollY;
    if (y !== lastY) dir = y > lastY ? 1 : -1;
    lastY = y;
    clearTimeout(timer);
    // scrollend arrives after momentum has finished; the timer covers browsers without it
    timer = setTimeout(rest, 'onscrollend' in window ? 400 : 140);
  }, { passive: true });
  if ('onscrollend' in window) addEventListener('scrollend', () => { clearTimeout(timer); timer = setTimeout(rest, 140); });
  const stop = () => { gliding = false; clearTimeout(timer); };
  // More wheel the same way while gliding belongs to the same move: one
  // gesture, one section. The other way stops the glide at once.
  addEventListener('wheel', (e) => { if (gliding && Math.sign(e.deltaY) === glideDir) return; stop(); }, { passive: true });
  addEventListener('touchstart', () => { touching = true; stop(); }, { passive: true });
  addEventListener('touchend', () => { touching = false; }, { passive: true });
  addEventListener('keydown', stop);
  glideTo = (y, done) => { stop(); dir = Math.sign(y - scrollY) || 1; glide(y, done); };
}

// In-page links (the navbar, "What you are trusting") glide to their section
// the same way the lock does. On a long jump the headings passed on the way
// skip their crumble, so the landing is not a shower of cubes.
document.addEventListener('click', (e) => {
  const a = e.target.closest('a[href^="#"]');
  if (!a || e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey) return;
  const target = document.querySelector(a.getAttribute('href'));
  if (!target) return;
  e.preventDefault();
  const section = target.closest('.trust-wrap') || target;
  const max = document.documentElement.scrollHeight - innerHeight;
  const y = Math.max(0, Math.min(max, Math.round(section.getBoundingClientRect().top + scrollY)));
  history.replaceState(null, '', a.getAttribute('href'));
  const land = () => { delete document.documentElement.dataset.jump; target.setAttribute('tabindex', '-1'); target.focus({ preventScroll: true }); };
  if (glideTo) {
    if (Math.abs(y - scrollY) > innerHeight * 1.2) document.documentElement.dataset.jump = '1';
    glideTo(y, land);
  } else {
    scrollTo({ top: y, behavior: 'instant' });
    land();
  }
});

// The phone menu: the section links in a panel under the header.
{
  const btn = document.getElementById('menu');
  const nav = document.getElementById('nav');
  const set = (open) => { nav.classList.toggle('open', open); btn.setAttribute('aria-expanded', String(open)); btn.textContent = open ? 'Close' : 'Menu'; };
  btn.addEventListener('click', () => set(!nav.classList.contains('open')));
  nav.addEventListener('click', (e) => { if (e.target.closest('a')) set(false); });
  addEventListener('keydown', (e) => { if (e.key === 'Escape') set(false); });
  addEventListener('scroll', () => { if (nav.classList.contains('open') && !document.documentElement.dataset.jump) set(false); }, { passive: true });
}

// The header: frosted once the page has moved, and the link for the section
// in view lit.
{
  const top = document.getElementById('top');
  const links = [...document.querySelectorAll('#nav a[href^="#"]')];
  const map = [['.strata', '#strata'], ['.api', '#strata'], ['.product', '#product'], ['.index', '#index'], ['.trust-wrap', '#trust'], ['.faq', '#faq']];
  let queued = false;
  const update = () => {
    queued = false;
    top.classList.toggle('scrolled', scrollY > 8);
    let current = null;
    for (const [sel, href] of map) {
      const r = document.querySelector(sel).getBoundingClientRect();
      if (r.top <= innerHeight * 0.4 && r.bottom > innerHeight * 0.4) current = href;
    }
    for (const l of links) l.setAttribute('aria-current', String(l.getAttribute('href') === current));
  };
  addEventListener('scroll', () => { if (!queued) { queued = true; requestAnimationFrame(update); } }, { passive: true });
  update();
}
