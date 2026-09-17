const XZEC = '0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201';
const CURVE_SUPPLY = 660_000_000;
const OFFICIAL_ASSETS = Object.freeze({
  '49eb9241e2a0b1163a1eed09bb91a575c93bbc6e76cd7afeb65263640fd169d5': 'ZYN',
  '893e745e33b40310003011a14651dc3cf173f71b92ee6e77ef62b7ba5a84e56e': 'ZYN testnet',
  [XZEC]: 'ZEC.zy',
});

const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => [...document.querySelectorAll(selector)];
const esc = (value) => String(value).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const short = (value, n = 7) => value?.length > n * 2 ? `${value.slice(0, n)}…${value.slice(-n)}` : (value || '—');
const number = (value) => Number.parseFloat(value || '0') || 0;
const compact = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 2 });
const fmt = (value, digits = 6) => {
  const n = number(value);
  if (!n) return '0';
  if (Math.abs(n) < 0.000001) return n.toExponential(4);
  return n.toLocaleString('en-US', { maximumFractionDigits: digits });
};
const official = (asset) => OFFICIAL_ASSETS[asset] ? `<b class="official-badge" title="Reserved protocol address">official ${esc(OFFICIAL_ASSETS[asset])}</b>` : '';

function rawDecimal(value) {
  const source = String(value || '0').trim();
  if (!/^\d+(?:\.\d+)?$/.test(source)) throw new Error('Enter a decimal amount');
  const [whole, fraction = ''] = source.split('.');
  if (fraction.length > 18) throw new Error('At most 18 decimal places are supported');
  return BigInt(whole) * 10n ** 18n + BigInt((fraction + '0'.repeat(18)).slice(0, 18));
}

function decimalRaw(raw) {
  const whole = raw / 10n ** 18n;
  const fraction = String(raw % 10n ** 18n).padStart(18, '0').replace(/0+$/, '');
  return fraction ? `${whole}.${fraction}` : String(whole);
}

function withBps(value, bps, roundUp = false) {
  const raw = rawDecimal(value);
  const numerator = raw * BigInt(bps);
  return decimalRaw(roundUp ? (numerator + 9_999n) / 10_000n : numerator / 10_000n);
}

function normalizeCurve(c) {
  if (!c) return null;
  return {
    asset: c.asset,
    creator: c.creator,
    symbol: c.symbol,
    name: c.display_name ?? c.displayName,
    metadataHash: c.metadata_hash ?? c.metadataHash,
    feeBps: Number(c.fee_bps ?? c.feeBps),
    sold: c.sold,
    curveReserve: c.curve_reserve ?? c.curveReserve,
    marketZec: c.market_zec ?? c.marketZec,
    creatorFees: c.creator_fees ?? c.creatorFees,
    graduationFees: c.graduation_fees ?? c.graduationFees,
    tokenLiquidity: c.graduated_token_liquidity ?? c.graduatedTokenLiquidity,
    zecLiquidity: c.graduated_zec_liquidity ?? c.graduatedZecLiquidity,
    overflow: c.graduation_overflow ?? c.graduationOverflow,
    lockedLp: c.graduated_locked_lp ?? c.graduatedLockedLp,
    price: c.zec_per_token ?? c.zecPerToken ?? c.marginalPrice,
    graduated: Boolean(c.graduated),
    pool: c.pool,
  };
}

const views = Object.fromEntries($$('[data-view]').map((view) => [view.dataset.view, view]));
const routeLinks = $$('[data-route]');
const gallery = $('#coin-gallery');
const search = $('#gallery-search');
const sort = $('#gallery-sort');
const stageButtons = $$('[data-filter]');
const walletButton = $('#wallet-button');
let curves = [];
let account = null;
let currentStage = 'revealed';
let currentCoin = null;
let tradeSide = 'buy';
let lastQuote = null;
let quoteTimer = null;

const provider = () => window.zyn?.isZynZap ? window.zyn : null;
const request = (method, params = {}) => {
  const p = provider();
  if (!p) throw new Error('Nap Wallet provider not found');
  return p.request({ method, params });
};

function progress(coin) {
  return Math.min(1, number(coin.sold) / CURVE_SUPPLY);
}

const stage = (coin) => coin.graduated ? 'revealed' : 'trading';
const stoneMarkup = (coin) => `<div class="card-stone" aria-hidden="true"><span>${esc([...coin.symbol][0] || '?')}</span></div>`;
const coinCard = (coin) => {
  const cut = progress(coin);
  return `
    <a class="coin-card" href="/coin/${coin.asset}" data-coin="${coin.asset}">
      <div class="coin-title">${stoneMarkup(coin)}<div><p>${coin.graduated ? 'REVEALED JADE' : 'WINDOW OPEN'}</p><h2>${esc(coin.name)}</h2><span>${esc(coin.symbol)} / ZEC.zy ${official(coin.asset)}</span></div></div>
      <div class="coin-metric"><strong>${fmt(coin.price, 12)}</strong><span>ZEC.zy</span></div>
      <div class="coin-metric"><strong class="positive">${(cut * 100).toFixed(2)}%</strong><span>${compact.format(number(coin.sold))} / 660M sold</span></div>
      <div class="coin-metric"><strong>${fmt(coin.marketZec, 8)} ZEC.zy</strong><span>${coin.graduated ? 'current pool reserve' : 'reversible reserve'}</span></div>
      <div class="coin-metric"><strong class="mono">${short(coin.asset)}</strong><span>${coin.graduated ? 'ZynZap pool' : 'curve asset'}</span></div>
    </a>`;
};

function renderGallery() {
  const query = search.value.trim().toLocaleLowerCase();
  const results = curves
    .filter((coin) => currentStage === 'all' || stage(coin) === currentStage)
    .filter((coin) => !query || `${coin.name} ${coin.symbol} ${coin.asset}`.toLocaleLowerCase().includes(query))
    .sort((a, b) => sort.value === 'price'
      ? number(b.price) - number(a.price)
      : sort.value === 'name'
        ? a.name.localeCompare(b.name)
        : progress(b) - progress(a));
  gallery.innerHTML = results.length ? results.map(coinCard).join('') : `<div class="empty-gallery">${provider() ? 'No stones match this view.' : 'Open Nap Wallet with its browser extension to read the yard.'}</div>`;
  const revealed = curves.filter((c) => c.graduated).length;
  const trading = curves.length - revealed;
  $('#count-revealed').textContent = revealed;
  $('#count-trading').textContent = trading;
  $('#count-all').textContent = curves.length;
  $('#tally-revealed').textContent = revealed;
  $('#tally-trading').textContent = trading;
  $('#tally-zec').textContent = `${fmt(curves.reduce((sum, c) => sum + number(c.marketZec), 0), 8)} ZEC.zy`;
}

function pathRoute() {
  const path = location.pathname.replace(/\/+$/, '') || '/';
  if (path === '/launch') return { view: 'launch' };
  const match = path.match(/^\/coin\/([0-9a-f]{64})$/i);
  return match ? { view: 'coin', asset: match[1].toLowerCase() } : { view: 'gallery' };
}

async function resolveCoin(asset) {
  const cached = curves.find((coin) => coin.asset === asset);
  if (cached) return cached;
  const found = normalizeCurve(await request('zyn_curve', { asset }));
  if (found) curves.push(found);
  return found;
}

async function renderCoin(asset) {
  try {
    const coin = await resolveCoin(asset);
    if (!coin) throw new Error('No launch exists at this asset address');
    currentCoin = coin;
    $('#coin-crumb').textContent = coin.name;
    $('#coin-title').textContent = coin.name;
    $('#coin-symbol').innerHTML = `${esc(coin.symbol)} / ZEC.zy ${official(coin.asset)}`;
    $('#coin-stage').textContent = coin.graduated ? 'REVEALED JADE / ZYNZAP' : 'WINDOW OPEN / REVERSIBLE CURVE';
    $('#coin-stone span').textContent = [...coin.symbol][0] || '?';
    $('#coin-price').textContent = `${fmt(coin.price, 12)} ZEC.zy`;
    $('#coin-change').textContent = coin.graduated ? 'live ZynZap spot' : `${(progress(coin) * 100).toFixed(2)}% revealed`;
    $('#coin-liquidity').textContent = `${fmt(coin.marketZec, 8)} ZEC.zy`;
    $('#coin-supply').textContent = '1B';
    $('#coin-holders').textContent = short(coin.creator, 6);
    $('#coin-holders').title = coin.creator;
    $('#coin-block').textContent = short(coin.asset, 6);
    $('#coin-block').title = coin.asset;
    const cut = progress(coin);
    $('#chart-point').setAttribute('cx', String(54 + 816 * cut));
    $('#chart-point').setAttribute('cy', String(342 - 278 * cut));
    $('#chart-progress').textContent = `${(cut * 100).toFixed(2)}% revealed`;
    $('#chart-title').textContent = coin.graduated ? 'Curve complete. ZynZap spot is now live.' : `${compact.format(number(coin.sold))} of 660M pieces discovered.`;
    document.title = `${coin.name} — Cave Jade`;
    configureTrade();
    scheduleQuote();
  } catch (error) {
    currentCoin = null;
    $('#coin-title').textContent = 'Stone not found';
    $('#trade-status').textContent = error.message;
  }
}

function syncRoute({ scroll = true } = {}) {
  const route = pathRoute();
  Object.entries(views).forEach(([name, view]) => { view.hidden = name !== route.view; });
  routeLinks.forEach((link) => {
    const active = link.dataset.route === route.view || (route.view === 'coin' && link.dataset.route === 'gallery');
    link.toggleAttribute('aria-current', active);
  });
  if (route.view === 'coin') void renderCoin(route.asset);
  else document.title = route.view === 'gallery' ? 'Cave Jade — Revealed Coins' : 'Pick a Stone — Cave Jade';
  document.body.dataset.route = route.view;
  if (scroll) window.scrollTo({ top: 0, behavior: 'instant' });
}

function navigate(path) {
  history.pushState({}, '', path);
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
  stageButtons.forEach((item) => {
    const active = item === button;
    item.classList.toggle('active', active);
    item.setAttribute('aria-selected', String(active));
  });
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

const form = $('#stone-form');
const nameInput = $('#stone-name');
const symbolInput = $('#stone-symbol');
const feeInput = $('#stone-fee');
const devBuyInput = $('#stone-dev-buy');
const maxZecInput = $('#stone-max-zec');
const descriptionInput = $('#stone-description');
const launchButton = $('#launch-stone');
const utf8 = new TextEncoder();
const symbolValue = () => symbolInput.value || 'STONE';
const validSymbol = (value) => {
  const bytes = utf8.encode(value);
  return bytes.length >= 1 && bytes.length <= 32 && !/[\u0000-\u0020\u007f-\u009f\u061c\u200e\u200f\u2028\u2029\u202a-\u202e\u2066-\u2069]/u.test(value);
};
const validName = (value) => {
  const bytes = utf8.encode(value);
  return bytes.length >= 1 && bytes.length <= 96 && value === value.trim() && !/[\u0000-\u001f\u007f-\u009f\u00a0\u1680\u2000-\u200f\u2028-\u202f\u205f\u2060-\u206f\u3000]/u.test(value);
};

function formValid() {
  const fee = Number(feeInput.value);
  const dev = Number(devBuyInput.value);
  const max = Number(maxZecInput.value);
  return validSymbol(symbolValue()) && validName(nameInput.value) && fee >= 0.5 && fee <= 3 && dev >= 0 && dev <= 50_000_000 && (dev === 0 || max > 0);
}

function renderPreview() {
  const symbol = symbolValue();
  const okay = formValid();
  $('#preview-name').textContent = nameInput.value.trim() || 'Unnamed Stone';
  $('#preview-symbol').textContent = symbol;
  $('#preview-letter').textContent = [...symbol][0] || '?';
  $('#preview-description').textContent = descriptionInput.value.trim() || 'No inscription supplied.';
  $('#preview-supply').textContent = '1B';
  symbolInput.setAttribute('aria-invalid', String(!validSymbol(symbol)));
  nameInput.setAttribute('aria-invalid', String(!validName(nameInput.value)));
  launchButton.disabled = !(provider() && okay);
  $('#stone-status').textContent = !provider()
    ? 'Connect Nap Wallet to place this stone. A local draft can still be saved.'
    : okay
      ? 'Ready for wallet approval. Launch costs 0.01 ZEC.zy: 0.009 seeds the pair and 0.001 goes to Treasury 26460.'
      : 'Check the name, Unicode ticker, 0.5–3% fee, and a positive dev-buy limit when a dev buy is used.';
}

form.querySelectorAll('input,textarea').forEach((input) => input.addEventListener('input', renderPreview));
$('#save-stone').addEventListener('click', () => {
  if (!formValid()) {
    $('#stone-status').textContent = 'Fix the marked launch fields before keeping this draft.';
    return;
  }
  sessionStorage.setItem('cave-jade-draft', JSON.stringify({
    name: nameInput.value,
    symbol: symbolValue(),
    supply: 1_000_000_000,
    feeBps: Math.round(Number(feeInput.value) * 100),
    devBuy: devBuyInput.value,
    maxZec: maxZecInput.value,
    description: descriptionInput.value,
  }));
  $('#stone-status').textContent = 'Stone kept in this browser session. Nothing was published.';
});

async function metadataHash() {
  const metadata = JSON.stringify({ name: nameInput.value, symbol: symbolValue(), description: descriptionInput.value });
  const digest = await crypto.subtle.digest('SHA-256', utf8.encode(metadata));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

form.addEventListener('submit', async (event) => {
  event.preventDefault();
  if (!formValid() || !provider()) return renderPreview();
  launchButton.disabled = true;
  launchButton.textContent = 'Awaiting approval…';
  const symbol = symbolValue();
  try {
    await request('zyn_launchCurve', {
      symbol,
      display_name: nameInput.value,
      metadata_hash: await metadataHash(),
      fee_bps: Math.round(Number(feeInput.value) * 100),
      dev_buy: devBuyInput.value || '0',
      max_zec: maxZecInput.value || '0',
    });
    await loadMarket();
    const created = curves.find((coin) => coin.creator === account?.account && coin.symbol === symbol);
    $('#stone-status').textContent = 'Stone placed onchain.';
    if (created) navigate(`/coin/${created.asset}`);
  } catch (error) {
    $('#stone-status').textContent = error.message;
  } finally {
    launchButton.textContent = 'Place in the yard';
    renderPreview();
  }
});

function configureTrade() {
  if (!currentCoin) return;
  const graduated = currentCoin.graduated;
  $('#trade-label').textContent = graduated && tradeSide === 'buy' ? 'You pay' : 'Exact token amount';
  $('#trade-unit').textContent = graduated && tradeSide === 'buy' ? 'ZEC.zy' : currentCoin.symbol;
  $('#trade-route').textContent = graduated ? 'ZynZap pool' : 'Reversible curve';
  $('.trade-submit').textContent = provider() ? (tradeSide === 'buy' ? 'Pick this jade' : 'Return this jade') : `Connect wallet to ${tradeSide === 'buy' ? 'pick' : 'return'}`;
  $('.trade-submit').disabled = !provider();
}

$$('[data-trade]').forEach((button) => button.addEventListener('click', () => {
  tradeSide = button.dataset.trade;
  $$('[data-trade]').forEach((item) => item.classList.toggle('active', item === button));
  configureTrade();
  scheduleQuote();
}));

async function refreshQuote() {
  lastQuote = null;
  if (!provider() || !currentCoin) return configureTrade();
  const amount = $('#trade-input').value;
  if (!amount || number(amount) <= 0) return;
  $('#trade-status').textContent = 'Reading an exact onchain quote…';
  try {
    if (!currentCoin.graduated) {
      const quote = await request('zyn_curveQuote', { side: tradeSide, asset: currentCoin.asset, tokens: amount });
      const limit = withBps(quote.settlement, tradeSide === 'buy' ? 10_100 : 9_900, tradeSide === 'buy');
      lastQuote = { kind: 'curve', quote, amount, limit };
      $('#trade-output-label').textContent = tradeSide === 'buy' ? 'Total you pay' : 'Net you receive';
      $('#trade-output').textContent = `${fmt(quote.settlement, 12)} ZEC.zy`;
      $('#trade-fee').textContent = `${fmt(quote.fee, 12)} ZEC.zy`;
      $('#trade-min').textContent = `${tradeSide === 'buy' ? '≤' : '≥'} ${fmt(limit, 12)} ZEC.zy`;
    } else {
      const assetIn = tradeSide === 'buy' ? XZEC : currentCoin.asset;
      const assetOut = tradeSide === 'buy' ? currentCoin.asset : XZEC;
      const quote = await request('zyn_quote', { asset_in: assetIn, asset_out: assetOut, amount });
      const minimum = withBps(quote.out, 9_900);
      const fee = quote.hops?.[0]?.fee || '0';
      lastQuote = { kind: 'swap', quote, amount, assetIn, assetOut };
      $('#trade-output-label').textContent = 'Expected output';
      $('#trade-output').textContent = `${fmt(quote.out, 12)} ${tradeSide === 'buy' ? currentCoin.symbol : 'ZEC.zy'}`;
      $('#trade-fee').textContent = `${fmt(fee, 12)} ZEC.zy`;
      $('#trade-min').textContent = `≥ ${fmt(minimum, 12)} ${tradeSide === 'buy' ? currentCoin.symbol : 'ZEC.zy'}`;
    }
    $('#trade-status').textContent = 'Quote read from the node. Submission still requires Nap Wallet approval.';
  } catch (error) {
    $('#trade-output').textContent = '—';
    $('#trade-fee').textContent = '—';
    $('#trade-min').textContent = '—';
    $('#trade-status').textContent = error.message;
  }
}

function scheduleQuote() {
  clearTimeout(quoteTimer);
  quoteTimer = setTimeout(() => void refreshQuote(), 180);
}
$('#trade-input').addEventListener('input', scheduleQuote);

$('.trade-submit').addEventListener('click', async () => {
  if (!lastQuote || !currentCoin) return void refreshQuote();
  const button = $('.trade-submit');
  button.disabled = true;
  button.textContent = 'Awaiting approval…';
  try {
    if (lastQuote.kind === 'curve') {
      await request(tradeSide === 'buy' ? 'zyn_buyCurve' : 'zyn_sellCurve', tradeSide === 'buy'
        ? { asset: currentCoin.asset, tokens: lastQuote.amount, max_zec: lastQuote.limit }
        : { asset: currentCoin.asset, tokens: lastQuote.amount, min_zec: lastQuote.limit });
    } else {
      await request('zyn_swap', { asset_in: lastQuote.assetIn, asset_out: lastQuote.assetOut, amount: lastQuote.amount });
    }
    await loadMarket();
    currentCoin = curves.find((coin) => coin.asset === currentCoin.asset) || currentCoin;
    await renderCoin(currentCoin.asset);
    $('#trade-status').textContent = 'Transaction accepted by the node.';
  } catch (error) {
    $('#trade-status').textContent = error.message;
  } finally {
    configureTrade();
  }
});

async function loadMarket() {
  if (!provider()) {
    $('#gallery-status').textContent = 'Waiting for the Nap Wallet provider. No public RPC is contacted by this page.';
    walletButton.innerHTML = '<span></span> Connect wallet';
    renderGallery();
    renderPreview();
    return;
  }
  try {
    const [nextAccount, nextCurves] = await Promise.all([
      request('zyn_account'),
      request('zyn_curves'),
    ]);
    account = nextAccount;
    curves = (nextCurves || []).map(normalizeCurve).filter(Boolean);
    walletButton.innerHTML = `<span></span> ${esc(short(account.account, 5))}`;
    walletButton.title = account.account;
    $('#gallery-status').textContent = `Live chain ${account.chain}. Asset addresses are identity; names and Unicode tickers may repeat.`;
    renderGallery();
    renderPreview();
    const route = pathRoute();
    if (route.view === 'coin') await renderCoin(route.asset);
  } catch (error) {
    $('#gallery-status').textContent = error.message;
    renderGallery();
  }
}

walletButton.addEventListener('click', () => void loadMarket());
window.addEventListener('zyn#initialized', () => void loadMarket(), { once: true });
document.addEventListener('visibilitychange', () => { if (!document.hidden) void loadMarket(); });

renderGallery();
renderPreview();
syncRoute({ scroll: false });
void loadMarket();
document.documentElement.dataset.jadeReady = 'true';
window.__caveJade = { ready: true, live: true, providerOnly: true, vocabulary: ['rough', 'window', 'reveal'] };
