// Trading in the Cave store: the market's listings and the owner's gallery.
//
// Everything goes through `window.zyn`, the provider Nap Wallet's extension
// injects. The page makes no requests of its own: reads are answered by the
// holder's own Nap, and every write — list, buy, cancel, redeem, collect —
// waits for the holder to approve it in Nap's window, which this page cannot
// touch. Without Nap the market stays sealed rather than inventing anything.

const CAVE_Z = '024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e';
const WAD = 10n ** 18n;

const $ = (sel, root = document) => root.querySelector(sel);
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const isHex = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
const isInt = (v) => typeof v === 'string' && /^\d{1,40}$/.test(v);

/// A WAD amount as a decimal, exactly.
function zec(raw) {
  const v = BigInt(raw);
  const frac = (v % WAD).toString().padStart(18, '0').replace(/0+$/, '').slice(0, 8);
  return `${v / WAD}${frac ? `.${frac}` : ''}`;
}

/// Collection Z's species are one index space (§86.1): Mole 0–2221, Owl
/// 2222–3554, Moth 3555–4443, each numbered from 1 within its species.
function itemName(collection, serial) {
  if (collection !== CAVE_Z) return `${collection.slice(0, 6)}… #${serial}`;
  const [species, start] = serial < 2222 ? ['Mole', 0] : serial < 3555 ? ['Owl', 2222] : ['Moth', 3555];
  return `Zyn ${species} #${String(serial - start + 1).padStart(4, '0')}`;
}

const provider = () => (typeof window.zyn?.request === 'function' ? window.zyn : null);
const ask = (method, params) => provider().request({ method, params });

function say(el, text, bad = false) {
  if (!el) return;
  el.textContent = text;
  el.dataset.tone = bad ? 'bad' : 'ok';
  el.hidden = false;
}

// ---------------------------------------------------------------------------
// The market: public listings, bought through Nap.
// ---------------------------------------------------------------------------

async function loadMarket() {
  const catalog = $('#market .market-catalog');
  const wallet = $('#market .market-wallet');
  if (!catalog || !provider()) return;
  wallet.disabled = false;
  wallet.innerHTML = '<span></span> Nap connected';
  let book;
  try {
    book = await ask('zyn_nftOffers');
  } catch (e) {
    return say($('#trade-note-market'), e.message, true);
  }
  const offers = (book?.offers || []).filter((o) => isHex(o.item) && isHex(o.asset) && isHex(o.collection) && isInt(o.pay_raw) && isInt(o.price_raw) && Number.isSafeInteger(o.serial) && o.serial >= 0);
  const metrics = $('#market .catalog-metrics');
  const floor = offers.reduce((m, o) => (m === null || BigInt(o.price_raw) < m ? BigInt(o.price_raw) : m), null);
  metrics.innerHTML = `<span><b>${offers.length}</b> live listings</span><span><b>${floor === null ? '—' : `${zec(floor)} ZEC`}</b> floor</span><span><b>ZEC.zy</b> settlement</span>`;
  $('#market .filter-heading span').textContent = `${offers.length} results`;
  if (!offers.length) return;
  const sealed = $('.sealed-catalog', catalog);
  if (sealed) sealed.hidden = true;
  let grid = $('.trade-grid', catalog);
  if (!grid) {
    grid = document.createElement('div');
    grid.className = 'trade-grid';
    catalog.append(grid);
  }
  grid.innerHTML = offers.map((o, i) => `<article class="trade-card">
      <div class="trade-art" aria-hidden="true"><img src="./assets/cave-mark.svg" alt="" /></div>
      <h3>${esc(itemName(o.collection, o.serial))}</h3>
      <dl><dt>Price</dt><dd>${zec(o.price_raw)} ZEC.zy</dd><dt>You pay</dt><dd>${zec(o.pay_raw)} ZEC.zy</dd></dl>
      <button type="button" class="trade-action" data-buy="${i}">Buy with Nap</button>
    </article>`).join('');
  grid.querySelectorAll('[data-buy]').forEach((b) => {
    b.onclick = async () => {
      const o = offers[Number(b.dataset.buy)];
      b.disabled = true;
      try {
        await ask('zyn_nftBuy', { item: o.item, pay: zec(o.pay_raw) });
        say($('#trade-note-market'), `Bought ${itemName(o.collection, o.serial)}. It is in your Nap wallet.`);
        loadMarket();
      } catch (e) {
        say($('#trade-note-market'), e.message, true);
        b.disabled = false;
      }
    };
  });
}

// ---------------------------------------------------------------------------
// The portfolio: what this holder's Nap holds, and what they are owed.
// ---------------------------------------------------------------------------

async function loadPortfolio() {
  const gallery = $('#portfolio .portfolio-gallery');
  if (!gallery || !provider()) return;
  let mine, owed;
  try {
    [mine, owed] = await Promise.all([ask('zyn_nftItems'), ask('zyn_owed')]);
  } catch (e) {
    return say($('#trade-note-portfolio'), e.message, true);
  }
  const items = (mine?.items || []).filter((i) => isHex(i.asset) && isHex(i.collection) && Number.isSafeInteger(i.serial));
  const claims = (owed?.owed || []).filter((c) => isHex(c.asset) && isInt(c.amount));
  const stats = $('#portfolio .portfolio-stats');
  stats.querySelectorAll('dd')[0].textContent = String(items.length);
  stats.querySelectorAll('dd')[1].textContent = String(new Set(items.map((i) => i.collection)).size);
  stats.querySelectorAll('dd')[2].textContent = String(items.filter((i) => i.listed).length);
  $('#portfolio .portfolio-connect').innerHTML = '<span></span> Nap connected';
  const empty = $('.gallery-empty', gallery);
  const placeholder = $('.gallery-grid', gallery);
  if (items.length) {
    if (empty) empty.hidden = true;
    if (placeholder) placeholder.hidden = true;
  }
  let grid = $('.trade-grid', gallery);
  if (!grid) {
    grid = document.createElement('div');
    grid.className = 'trade-grid';
    gallery.append(grid);
  }
  const collect = claims.length
    ? `<div class="trade-owed"><span>The market owes you ${claims.length} payment${claims.length === 1 ? '' : 's'} — sale proceeds, refunds or floor shares.</span><button type="button" class="trade-action" data-collect>Collect with Nap</button></div>`
    : '';
  grid.innerHTML = collect + items.map((it, i) => `<article class="trade-card">
      <div class="trade-art" aria-hidden="true"><img src="./assets/cave-mark.svg" alt="" /></div>
      <h3>${esc(itemName(it.collection, it.serial))}</h3>
      ${it.listed
        ? `<p class="trade-state">Listed</p><button type="button" class="trade-action ghost" data-cancel="${i}">Cancel listing</button>`
        : `<form class="trade-list" data-list="${i}"><label><span>Price, ZEC.zy</span><input name="price" inputmode="decimal" autocomplete="off" placeholder="0.0" /></label><button type="submit" class="trade-action">List</button></form>
           <button type="button" class="trade-action ghost" data-redeem="${i}">Redeem for floor</button>`}
    </article>`).join('');
  const act = async (el, method, params, done) => {
    el.disabled = true;
    try {
      await ask(method, params);
      say($('#trade-note-portfolio'), done);
      loadPortfolio();
    } catch (e) {
      say($('#trade-note-portfolio'), e.message, true);
      el.disabled = false;
    }
  };
  grid.querySelector('[data-collect]')?.addEventListener('click', (e) => act(e.target, 'zyn_collect', {}, 'Collected. It is in your Nap wallet.'));
  grid.querySelectorAll('[data-cancel]').forEach((b) => { b.onclick = () => act(b, 'zyn_nftCancel', { asset: items[Number(b.dataset.cancel)].asset }, 'Listing cancelled.'); });
  grid.querySelectorAll('[data-redeem]').forEach((b) => { b.onclick = () => act(b, 'zyn_nftRedeem', { asset: items[Number(b.dataset.redeem)].asset }, 'Redeemed: the item is burned and its share of the floor is owed to you.'); });
  grid.querySelectorAll('[data-list]').forEach((f) => {
    f.onsubmit = (e) => {
      e.preventDefault();
      const price = f.elements.price.value.trim();
      if (!/^\d+(\.\d{1,8})?$/.test(price) || Number(price) <= 0) return say($('#trade-note-portfolio'), 'Enter a price in ZEC.zy, up to eight decimals.', true);
      act(f.querySelector('button'), 'zyn_nftList', { asset: items[Number(f.dataset.list)].asset, price }, `Listed for ${price} ZEC.zy.`);
    };
  });
}

// ---------------------------------------------------------------------------

function route() {
  if (location.hash === '#market') loadMarket();
  if (location.hash === '#portfolio') loadPortfolio();
}

// With Nap present, "Connect wallet" loads the gallery instead of explaining
// that no wallet is available.
function wire() {
  if (!provider()) return;
  document.querySelectorAll('#portfolio [data-wallet-dialog]').forEach((b) => {
    b.removeAttribute('data-wallet-dialog');
    b.addEventListener('click', () => loadPortfolio());
  });
  route();
}

window.addEventListener('hashchange', route);
// The extension injects its provider as the page starts; allow it a moment.
if (provider()) wire();
else window.addEventListener('zyn#initialized', wire, { once: true });
setTimeout(() => { if (provider()) wire(); }, 800);
