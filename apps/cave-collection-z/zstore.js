// Cave's shelf, item and market pages on the object ledger, through Nap.
//
// What anyone may see is read from the public chain: the order book, and the
// collection's own index (name, art, rarity, traits — the frozen metadata).
// What only a holder may see — their items, what the market owes them — is
// asked of the Nap they connected, and every write (list, cancel, buy,
// collect) waits for them to approve it in Nap's window. This page never
// holds a key and never learns more than Nap chooses to tell it.

import { connect, discover, NAP_WEB } from './nap-connect.js';
import * as sol from './solana-burn.js';
import * as evm from './evm-burn.js';

const initial = new URLSearchParams(location.search);
const NETWORK = initial.get('network') === 'testnet' ? 'testnet' : 'mainnet';
const RPC = NETWORK === 'testnet' ? 'https://zyn-rpc.cavebro.win/object' : 'https://zyn-rpc.cavebro.win/';
const LOCAL = ['localhost', '127.0.0.1'].includes(location.hostname);
const NAP = (LOCAL && initial.get('nap')) || NAP_WEB;
const CAVE_Z = '024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e';
/// ZEC.zy, the same asset on both networks: what the DAO treasury pays in.
const ZEC_ZY = '0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201';
const WAD = 10n ** 18n;
const PAGE = document.body.dataset.page;
/// The members' chat: the den (Elixir, on Contabo). A local build may point at
/// another den with ?chat=.
const CHAT = (LOCAL && initial.get('chat')) || 'https://chat.cavebro.win';


const el = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const isHex = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
const isInt = (v) => typeof v === 'string' && /^\d{1,40}$/.test(v);
const withNet = (path) => (NETWORK === 'testnet' ? `${path}${path.includes('?') ? '&' : '?'}network=testnet` : path);

/// A ZEC.zy amount for display, to 8 decimals (cut, never rounded up).
function zec(raw) {
  const v = BigInt(raw);
  const frac = (v % WAD).toString().padStart(18, '0').replace(/0+$/, '').slice(0, 8);
  return `${v / WAD}${frac ? `.${frac}` : ''}`;
}
/// The same amount in full, for what is paid: Nap checks it to the last digit
/// against the chain, and a price with a 1% fee can run past 8 decimals.
function exact(raw) {
  const v = BigInt(raw);
  const frac = (v % WAD).toString().padStart(18, '0').replace(/0+$/, '');
  return `${v / WAD}${frac ? `.${frac}` : ''}`;
}

/// A whole number from a field, within 1..max, or null.
function whole(id, max) {
  const raw = String(el(id)?.value ?? '').trim();
  const n = Number(raw);
  return /^\d+$/.test(raw) && n >= 1 && n <= max ? n : null;
}

/// What a failed request means to the holder. 4001 is Nap's "no" — nothing
/// happened; anything else (a closed or blocked window, the chain) says so
/// in its own words.
const failText = (e, no) => (e?.code === 4001 ? no : e?.message || String(e));

function show(name) {
  document.querySelectorAll('.ledger-body > .state').forEach((s) => { s.hidden = s.id !== `s-${name}`; });
}

/// Look again after a write: an epoch seals about once a minute, so a few
/// looks over two minutes rather than one that comes too early. A look from a
/// page since left does nothing (each read checks its generation).
const later = (fn) => { for (const t of [15000, 40000, 70000, 110000]) setTimeout(fn, t); };

function say(text, bad = false) {
  const out = el('out');
  if (!out) return;
  out.textContent = text;
  out.classList.toggle('bad', bad);
}

// ---------------------------------------------------------------------------
// The collection's index: what every item is, public and frozen.

let indexPromise = null;
function collectionIndex() {
  indexPromise ??= fetch('/collection-z.json', { credentials: 'omit' }).then((r) => (r.ok ? r.json() : null)).catch(() => null);
  return indexPromise;
}

async function describe(collection, serial) {
  const idx = collection === CAVE_Z ? await collectionIndex() : null;
  const e = idx?.items?.[serial];
  if (!e) return { name: `${collection.slice(0, 6)}… #${serial}`, image: null, rarity: null, rank: null, traits: {} };
  return { name: e.n, image: isHex(e.i) ? `/blobs/${e.i}` : null, rarity: e.r, rank: e.k, traits: e.t || {} };
}

const art = (d, cls = 'art') => (d.image
  ? `<img class="${cls}" src="${esc(d.image)}" alt="" loading="lazy" width="192" height="192" />`
  : `<img class="${cls}" src="/assets/cave-mark.svg?v=1" alt="" />`);
const rarityTag = (d) => (d.rarity ? `<span class="tag rarity r-${esc(d.rarity.toLowerCase())}">${esc(d.rarity)} · #${esc(d.rank)}</span>` : '');

// ---------------------------------------------------------------------------
// The chain's public order book.

/// One read of the chain: a clear error for an HTTP failure or a hang, never
/// "Unexpected token '<'" or a spinner forever.
async function chain(method, fields = {}) {
  let r;
  try {
    r = await fetch(RPC, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: crypto.randomUUID(), method, params: [], ...fields }),
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
      signal: AbortSignal.timeout(15000),
    });
  } catch (e) {
    throw new Error(e?.name === 'TimeoutError' ? 'The chain took too long to answer. Try again shortly.' : 'The chain could not be reached. Check your connection and try again.');
  }
  if (!r.ok) throw new Error(`The chain did not answer (HTTP ${r.status}). Try again shortly.`);
  const body = await r.json().catch(() => null);
  if (!body) throw new Error('The chain sent something unreadable. Try again shortly.');
  if (body.error) throw new Error(body.error.message || 'The chain did not answer');
  return body.result;
}

async function publicOffers() {
  const result = await chain('zyn_offers');
  return (result?.offers || []).filter((o) => isHex(o.item) && isHex(o.asset) && isHex(o.collection)
    && isInt(o.pay_raw) && isInt(o.price_raw) && Number.isSafeInteger(o.serial) && o.serial >= 0);
}

// ---------------------------------------------------------------------------
// The wallet: Nap, through Nap Connect.

let current = null;
const walletListeners = new Set();
/// Bumped on every in-place page change: a read or timer from the page you
/// left must not write into the page you are on.
let generation = 0;

/// Whether a read can be answered without putting Nap's window in front of
/// the holder: Nap in the browser only while its window is open; the
/// extension only while its window holds an unlocked session for this site —
/// asked with nap_session, which never opens anything.
function canAskQuietly() {
  return !!current && (!current.provider.isNapWeb || current.provider.isOpen());
}
async function quietly() {
  if (!canAskQuietly()) return false;
  if (current.provider.isNapWeb) return true;
  try { return !!(await current.provider.request({ method: 'nap_session' })); } catch { return false; }
}

function ask(method, p) {
  if (!current) { pick(); return Promise.reject(new Error('Connect Nap first.')); }
  // The network rides every request, so Nap refuses one answered on the
  // wrong chain instead of answering it.
  return current.provider.request({ method, params: { ...(p || {}), network: NETWORK } });
}

function renderWalletButtons() {
  document.querySelectorAll('[data-wallet-button]').forEach((b) => {
    b.textContent = current ? `Nap · ${current.session.account.slice(0, 6)}…` : 'Connect Nap';
  });
}

let dialog = null;
async function pick() {
  if (!dialog) {
    dialog = document.createElement('dialog');
    dialog.className = 'zwallet';
    dialog.innerHTML = '<h2>Connect a wallet.</h2><div class="zwallet-body"></div><button type="button" class="act ghost zwallet-close">Close</button>';
    document.body.append(dialog);
    dialog.querySelector('.zwallet-close').onclick = () => dialog.close();
  }
  const body = dialog.querySelector('.zwallet-body');
  // Nap in the browser whose window has closed cannot answer: connect again,
  // which reopens it, rather than showing a connection that is not there.
  if (current && !canAskQuietly()) current = null;
  dialog.showModal();
  if (current) {
    body.innerHTML = `<p class="say">Connected: <strong>${esc(current.info.name)}</strong> · <code>${esc(current.session.account.slice(0, 10))}…</code> · ${esc(current.session.network)}</p>
      <button type="button" class="act ghost zwallet-off">Disconnect</button>`;
    body.querySelector('.zwallet-off').onclick = () => {
      // Nap forgets this site too, so connecting again asks the holder again.
      const was = current;
      current = null;
      if (was.provider.isNapWeb ? was.provider.isOpen() : true) was.provider.request({ method: 'nap_disconnect' }).catch(() => {});
      renderWalletButtons(); walletListeners.forEach((f) => f()); dialog.close();
    };
    return;
  }
  body.innerHTML = '<p class="say">Looking for Nap…</p>';
  const list = await discover({ web: NAP });
  body.innerHTML = list.map((w, i) => `<button type="button" class="act zwallet-option" data-i="${i}">${esc(w.info.name)}</button>`).join('')
    + `<p class="out" id="zwallet-status" role="status" aria-live="polite"></p><p class="say small">Connecting shares your Zyn account and a Zcash address with this site, and lets it read your items, tickets and what the market owes you. It never sees your keys. Every payment is approved in Nap.${NETWORK === 'testnet' ? ' <strong>Testnet:</strong> nothing here has value.' : ''}</p>`;
  const options = [...body.querySelectorAll('[data-i]')];
  options.forEach((b) => {
    b.onclick = async () => {
      const entry = list[Number(b.dataset.i)];
      const status = body.querySelector('#zwallet-status');
      status.classList.remove('bad');
      status.textContent = entry.provider.isNapWeb ? "Nap's window is open — finish there." : 'Approve the connection in Nap.';
      options.forEach((o) => { o.disabled = true; });
      try {
        const got = await connect(entry, { network: NETWORK });
        if (got.session?.network !== NETWORK) throw new Error(`Nap answered on ${got.session?.network || 'another network'}, and this page is on ${NETWORK}. Connect again.`);
        current = got;
        current.provider.on?.('disconnect', () => { current = null; renderWalletButtons(); walletListeners.forEach((f) => f()); });
        renderWalletButtons();
        dialog.close();
        walletListeners.forEach((f) => f());
      } catch (e) {
        status.textContent = failText(e, 'Not connected. Nothing was shared.');
        status.classList.toggle('bad', e?.code !== 4001);
      } finally { options.forEach((o) => { o.disabled = false; }); }
    };
  });
}

function wireWallet() {
  document.querySelectorAll('[data-wallet-button]').forEach((b) => b.addEventListener('click', pick));
  renderWalletButtons();
}

// ---------------------------------------------------------------------------
// The shelf: what this holder's Nap holds, and what they are owed.

async function shelfPage() {
  const gen = generation;
  const read = async () => {
    if (gen !== generation) return;
    if (!current) return show('connect');
    if (!(await quietly())) { if (gen === generation) show('ready'); return; }
    if (gen !== generation) return;
    show('looking');
    try {
      const [mine, owed] = await Promise.all([ask('zyn_nftItems'), ask('zyn_owed')]);
      if (gen !== generation) return;
      const items = (mine?.items || []).filter((i) => isHex(i.asset) && isHex(i.collection) && Number.isSafeInteger(i.serial));
      const claims = (owed?.owed || []).filter((c) => isHex(c.asset) && isInt(c.amount));
      el('count').textContent = String(items.length);
      const owedBox = el('owed');
      owedBox.hidden = !claims.length;
      owedBox.querySelector('span').textContent = `The market owes you ${claims.length} payment${claims.length === 1 ? '' : 's'} — sale proceeds, refunds or floor shares.`;
      if (!items.length) return show(claims.length ? 'shelf' : 'empty');
      const cards = await Promise.all(items.map(async (it) => {
        const d = await describe(it.collection, it.serial);
        return `<a class="item" href="${esc(withNet(`/item?collection=${it.collection}&serial=${it.serial}`))}">
          ${art(d)}
          <div class="sym">${esc(d.name)}</div>
          <span class="tag ${it.listed ? 'listed' : ''}">${it.listed ? 'Listed' : 'Held'}</span> ${rarityTag(d)}
        </a>`;
      }));
      if (gen !== generation) return;
      el('shelf').innerHTML = cards.join('');
      show('shelf');
    } catch (e) {
      if (gen !== generation) return;
      el('broke').textContent = e.message;
      show('broke');
    }
  };
  el('connect').onclick = pick;
  el('ask').onclick = () => (canAskQuietly() ? read() : pick());
  el('again').onclick = read;
  el('retry').onclick = read;
  el('collect').onclick = async () => {
    el('collect').disabled = true;
    try {
      const r = await ask('zyn_collect', {});
      say(`Collected${r?.collected ? ` ${r.collected}` : ''}. It is in your Nap wallet.`);
      await read();
    } catch (e) { say(failText(e, 'Not collected.'), e?.code !== 4001); } finally { if (el('collect')) el('collect').disabled = false; }
  };
  walletListeners.add(read);
  if (current) read(); else show('connect');
}

// ---------------------------------------------------------------------------
// One item: public art and traits; list, cancel or buy through Nap.

async function itemPage() {
  const collection = isHex(params.get('collection')) ? params.get('collection') : CAVE_Z;
  const raw = params.get('serial');
  // A serial is a whole number the collection has: no serial is not item 0,
  // and Collection Z stops at 4,443.
  if (!/^\d{1,5}$/.test(raw ?? '') || (collection === CAVE_Z && Number(raw) >= 4444)) return show('noid');
  const serial = Number(raw);
  show('looking');
  const gen = generation;
  const d = await describe(collection, serial);
  if (gen !== generation) return;
  el('name').textContent = d.name;
  el('sub').textContent = d.rarity ? `${d.rarity} · rank ${d.rank} of 4,444` : 'in the cave';
  document.title = `${d.name} — Cave`;
  el('art').innerHTML = art(d, 'art big');
  el('facts').innerHTML = Object.entries(d.traits).map(([k, v]) => `<div><dt>${esc(k)}</dt><dd>${esc(v)}</dd></div>`).join('')
    // The chain counts serials from 0 across the collection; the name counts
    // each species from 1. Both shown, labelled, so neither looks like a typo.
    + `<div><dt>Chain serial</dt><dd>${serial} (from 0)</dd></div>`;

  let offer = null;
  let mine = null;
  const read = async () => {
    if (gen !== generation) return;
    say('');
    try {
      offer = (await publicOffers()).find((o) => o.collection === collection && o.serial === serial) || null;
    } catch (e) { if (gen === generation) say(`The order book could not be read: ${e.message}`, true); }
    if (gen !== generation) return;
    mine = null;
    if (current && (await quietly())) {
      try {
        mine = (await ask('zyn_nftItems'))?.items?.find((i) => i.collection === collection && i.serial === serial) || null;
      } catch { /* the public half still stands */ }
    }
    if (gen !== generation) return;
    el('state-tag').textContent = mine ? (mine.listed ? 'yours · listed' : 'yours') : offer ? 'for sale' : 'not for sale';
    el('asking').textContent = offer ? `${zec(offer.price_raw)} ZEC.zy (you pay ${exact(offer.pay_raw)} with the 1% fee)` : 'not listed';
    el('sell').hidden = !(mine && !mine.listed);
    el('unsell').hidden = !(mine && mine.listed);
    el('buy').hidden = !(offer && !mine);
    el('connect-hint').hidden = !!current;
    el('open-hint').hidden = !current || canAskQuietly();
    show('object');
  };
  el('list').onclick = async () => {
    const price = el('price').value.trim();
    if (!/^\d+(\.\d{1,8})?$/.test(price) || Number(price) <= 0) return say('Name a price in ZEC.zy, like 0.05.', true);
    el('list').disabled = true;
    try { await ask('zyn_nftList', { asset: mine.asset, price }); say('Listed. It rests on the book once the epoch seals — about a minute.'); later(read); }
    catch (e) { say(failText(e, 'Not listed.'), e?.code !== 4001); } finally { if (el('list')) el('list').disabled = false; }
  };
  el('cancel').onclick = async () => {
    el('cancel').disabled = true;
    try { await ask('zyn_nftCancel', { asset: mine.asset }); say('Taken back once the epoch seals — about a minute. It is yours to hold or list again.'); later(read); }
    catch (e) { say(failText(e, 'Not cancelled.'), e?.code !== 4001); } finally { if (el('cancel')) el('cancel').disabled = false; }
  };
  el('take').onclick = async () => {
    if (!current) return pick();
    el('take').disabled = true;
    try { await ask('zyn_nftBuy', { item: offer.item, pay: exact(offer.pay_raw) }); say(`Bought ${d.name}. It is in your Nap wallet once the epoch seals — about a minute.`); later(read); }
    catch (e) { say(failText(e, 'Not bought. Nothing was paid.'), e?.code !== 4001); } finally { if (el('take')) el('take').disabled = false; }
  };
  el('again').onclick = read;
  el('open-nap').onclick = async () => { await pick(); };
  walletListeners.add(read);
  await read();
}

// ---------------------------------------------------------------------------
// The market: every resting listing, public; bought through Nap.

async function marketPage() {
  const gen = generation;
  const read = async () => {
    if (gen !== generation) return;
    show('looking');
    let offers;
    try { offers = await publicOffers(); } catch (e) { if (gen !== generation) return; el('broke').textContent = e.message; return show('broke'); }
    if (gen !== generation) return;
    el('count').textContent = String(offers.length);
    if (!offers.length) return show('empty');
    // The book does not say whose a listing is. The holder's own Nap does,
    // so their listings read "Yours" instead of offering them to themselves.
    const yours = new Set();
    if (current && (await quietly())) {
      try {
        for (const it of (await ask('zyn_nftItems'))?.items || []) if (it.listed) yours.add(it.asset);
      } catch { /* the public book still stands */ }
    }
    if (gen !== generation) return;
    const rows = await Promise.all(offers.map(async (o, i) => {
      const d = await describe(o.collection, o.serial);
      const action = yours.has(o.asset)
        ? '<span class="tag listed">Yours</span>'
        : `<button class="act small" type="button" data-buy="${i}">${current ? 'Buy' : 'Connect to buy'}</button>`;
      return `<tr>
        <td><a class="obj" href="${esc(withNet(`/item?collection=${o.collection}&serial=${o.serial}`))}">${art(d, 'thumb')}<span>${esc(d.name)}</span></a> ${rarityTag(d)}</td>
        <td class="r">${zec(o.price_raw)}</td>
        <td class="r">${zec(o.pay_raw)}</td>
        <td class="r">${action}</td>
      </tr>`;
    }));
    if (gen !== generation) return;
    el('rows').innerHTML = rows.join('');
    el('rows').querySelectorAll('[data-buy]').forEach((b) => {
      b.onclick = async () => {
        if (!current) return pick();
        const o = offers[Number(b.dataset.buy)];
        b.disabled = true;
        try {
          await ask('zyn_nftBuy', { item: o.item, pay: exact(o.pay_raw) });
          say(`Bought ${(await describe(o.collection, o.serial)).name}. It is in your Nap wallet once the epoch seals — about a minute.`);
          later(read);
        } catch (e) { say(failText(e, 'Not bought. Nothing was paid.'), e?.code !== 4001); b.disabled = false; }
      };
    });
    show('book');
  };
  el('again').onclick = read;
  el('retry').onclick = read;
  walletListeners.add(read);
  await read();
}

// ---------------------------------------------------------------------------
// The sale (§144): tickets, the draw, claims and the open mint. The sale's
// state is public and read from the chain; which tickets are whose only Nap
// can say, because only the wallet that bought one can derive its id.

async function chainSale() {
  const result = await chain('zyn_market');
  const s = (result?.sales || []).find((x) => x.collection === CAVE_Z);
  if (!s || !['ticket_price', 'claim_price', 'open_price'].every((k) => isInt(s[k]))) return null;
  return s;
}

// Sale credit (§182): ZCAVE burned on Solana or REGI burned on Arc, worth
// what was burned at its USD price, out of the credit the team paid in. The
// tokens, per network, and the reads pricing them — asked of each chain
// directly, since their public RPCs refuse Cloudflare's workers.
let burnConfig = null;
const SOLANA_ADDR = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/;
const EVM_ADDR = /^0x[0-9a-fA-F]{40}$/;
function validBurn(c) {
  if (!c || typeof c.symbol !== 'string' || !Number.isInteger(c.decimals)) return false;
  const priced = (c.chain === 'solana' && (SOLANA_ADDR.test(c.curve || '') || /^\d*\.?\d+$/.test(c.zec_per_token || '')))
    || (c.chain === 'evm' && (/^[0-9a-f]{64}$/.test(c.v4?.slot || '') && EVM_ADDR.test(c.v4?.manager || '') && EVM_ADDR.test(c.v4?.quote || '') || Number(c.usd) > 0));
  return priced && (c.chain === 'solana' ? SOLANA_ADDR.test(c.mint) : EVM_ADDR.test(c.token) && Number.isInteger(c.chain_id) && /^https:\/\//.test(c.rpc || ''));
}
async function burnTokens() {
  burnConfig ??= fetch(NETWORK === 'testnet' ? '/burn-testnet.json' : '/burn.json', { credentials: 'omit' })
    .then((r) => (r.ok ? r.json() : null))
    .then((c) => Object.entries(c || {}).filter(([, t]) => validBurn(t)).map(([key, t]) => ({ ...t, key })))
    .catch(() => []);
  return burnConfig;
}

/// Burns are valued at the lower of the last hour's prices (the watchers'
/// rule), so the page burns this much more than the credit asked for.
const BURN_HEADROOM = 5n;

/// ZEC's USD price in this browser: Coinbase, Kraken and CoinGecko, the
/// median of those that answer — only for showing how much to burn; the
/// watcher values the burn with its own reads.
async function zecUsd() {
  const got = await Promise.all([
    fetch('https://api.coinbase.com/v2/prices/ZEC-USD/spot', { credentials: 'omit' }).then((r) => r.json()).then((j) => Number(j?.data?.amount)),
    fetch('https://api.kraken.com/0/public/Ticker?pair=ZECUSD', { credentials: 'omit' }).then((r) => r.json()).then((j) => Number(Object.values(j?.result || {})[0]?.c?.[0])),
    fetch('https://api.coingecko.com/api/v3/simple/price?ids=zcash&vs_currencies=usd', { credentials: 'omit' }).then((r) => r.json()).then((j) => Number(j?.zcash?.usd)),
  ].map((p) => p.catch(() => NaN)));
  const ok = got.filter((x) => Number.isFinite(x) && x > 0).sort((a, b) => a - b);
  if (!ok.length) throw new Error("ZEC's price did not load; try again shortly.");
  return ok[Math.floor(ok.length / 2)];
}

/// A token's price now, in ZEC (WAD) per whole token.
async function zecPerToken(t) {
  if (t.chain === 'solana') {
    if (t.zec_per_token) return BigInt(Math.floor(Number(t.zec_per_token) * 1e18));
    // pump.fun's bonding curve prices at its virtual reserves.
    const info = await solanaRead(t, 'getAccountInfo', [t.curve, { encoding: 'base64', commitment: 'confirmed' }]);
    const b = Uint8Array.from(atob(info?.value?.data?.[0] || ''), (c) => c.charCodeAt(0));
    if (b.length < 49) throw new Error("The token's curve did not load.");
    if (b[48]) throw new Error('The token has left its curve; burning waits for its pool to be set.');
    const le = (i) => b.subarray(i, i + 8).reduceRight((v, x) => v * 256n + BigInt(x), 0n);
    const [vt, vq] = [le(8), le(16)];
    if (!vt || !vq) throw new Error("The token's curve is empty.");
    return (vq * 10n ** BigInt(18 - (t.zec_decimals ?? 8)) * 10n ** BigInt(t.decimals)) / vt;
  }
  const usd = Number(t.usd) > 0 ? Number(t.usd) : await evm.v4Usd(t);
  const p = usd / (await zecUsd());
  if (!(p > 0)) throw new Error("The token's price did not load.");
  return BigInt(Math.floor(p * 1e18));
}

/// Base units of the token to burn for `credit` (ZEC in WAD), with headroom.
/// Rounded up to a figure a person can read and type: whole tokens from a
/// thousand up, four places below — so the page burns what it shows.
const burnFor = (credit, price, decimals) => {
  const exact = (credit * 10n ** BigInt(decimals) * (100n + BURN_HEADROOM) + price * 100n - 1n) / (price * 100n);
  const keep = exact >= 1000n * 10n ** BigInt(decimals) ? 0 : 4;
  const step = decimals > keep ? 10n ** BigInt(decimals - keep) : 1n;
  return ((exact + step - 1n) / step) * step;
};
/// What a token is called where it lives.
const chainName = (t) => t.chain_name || (t.chain === 'solana' ? 'Solana' : 'its chain');

const SOLANA_PUBLIC = { mainnet: 'https://api.mainnet-beta.solana.com', devnet: 'https://api.devnet.solana.com' };
async function solanaRead(cfg, method, params) {
  const r = await fetch(cfg.rpc || SOLANA_PUBLIC[cfg.cluster] || SOLANA_PUBLIC.mainnet, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }), credentials: 'omit', referrerPolicy: 'no-referrer',
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok || j.error || !('result' in j)) throw new Error('Solana did not answer; try again shortly.');
  return j.result;
}

const units = (raw, decimals) => {
  const v = BigInt(raw); const d = 10n ** BigInt(decimals);
  const f = (v % d).toString().padStart(decimals, '0').replace(/0+$/, '');
  return `${(v / d).toLocaleString('en-US')}${f ? `.${f}` : ''}`;
};

/// Nap's code for one burn: a fresh one every time, so a second burn made
/// before the first is posted still earns its own credit.
async function burnCode() {
  const { memo } = await ask('zyn_burnMemo', { fresh: true });
  if (!/^zyn:[0-9a-f]{64}$/.test(memo || '')) throw new Error('Nap did not give a burn code.');
  return memo;
}

/// Burn ZCAVE: the holder's Solana wallet signs one transaction, a
/// `burnChecked` and the memo. Returns the Solana signature.
async function burnSolana(t, amount, memo, report) {
  const wallets = sol.solanaWallets();
  await new Promise((r) => setTimeout(r, 300));
  if (!wallets.length) throw new Error('No Solana wallet found in this browser. Install one (Phantom, Solflare, Backpack) and reload.');
  const wallet = wallets[0];
  report(`Connecting ${wallet.name}…`);
  const { accounts } = await wallet.features['standard:connect'].connect();
  const account = accounts?.[0];
  if (!account) throw new Error(`${wallet.name} shared no account.`);
  const held = await solanaRead(t, 'getTokenAccountsByOwner', [account.address, { mint: t.mint }, { encoding: 'jsonParsed', commitment: 'confirmed' }]);
  const ta = (held?.value || []).find((a) => BigInt(a.account?.data?.parsed?.info?.tokenAmount?.amount || '0') >= amount);
  if (!ta) throw new Error(`That wallet holds less than ${units(amount, t.decimals)} ${t.symbol}.`);
  const { value } = await solanaRead(t, 'getLatestBlockhash', [{ commitment: 'finalized' }]);
  const message = sol.burnMessage({
    owner: account.address, tokenAccount: ta.pubkey, mint: t.mint, tokenProgram: ta.account.owner,
    amount, decimals: t.decimals, memo, blockhash: value.blockhash,
  });
  report(`Approve the burn of ${units(amount, t.decimals)} ${t.symbol} in ${wallet.name}.`);
  return sol.signAndSend(wallet, account, sol.unsigned(message), t.cluster === 'devnet' ? 'solana:devnet' : 'solana:mainnet');
}

/// Burn REGI: the holder's EVM wallet sends the token's transfer to 0x…dEaD
/// with the code appended. Returns the transaction hash.
async function burnEvm(t, amount, memo, report) {
  report('Connecting your EVM wallet…');
  const account = await evm.connect(t);
  if ((await evm.balanceOf(t, account)) < amount) throw new Error(`That wallet holds less than ${units(amount, t.decimals)} ${t.symbol}.`);
  report(`Approve the burn of ${units(amount, t.decimals)} ${t.symbol} in your wallet.`);
  return evm.send(t, account, amount, memo.slice(4));
}

const PHASE_LABEL = { open: 'open', closed: 'drawing', drawn: 'claims open', openmint: 'open mint' };
/// §144.8: claims run for the terms' 16,128 Zcash blocks after the close.
const CLAIM_BLOCKS = 16128;

async function mintPage() {
  const gen = generation;
  let sale = null;
  // Sale credit (§182): the tokens burnable for it, their prices as last
  // read (per token, for half a minute), and this wallet's credit.
  let tokens = [];
  const prices = new Map();
  let credit = null;
  // A gated sale (§168): this wallet's place on the list and its next prices,
  // as Nap reads them from the chain. Null when the sale is not gated.
  let access = null;
  const gated = () => !!sale?.gate;
  const ticketsTotal = (n) => (gated() && access?.prices ? access.prices.slice(0, n).reduce((t, x) => t + BigInt(x), 0n) : BigInt(sale.ticket_price) * BigInt(n));
  /// A total after this wallet's credit: what ZEC.zy it still takes.
  const afterCredit = (total) => {
    if (!credit) return `${exact(total)} ZEC.zy`;
    const used = credit < total ? credit : total;
    return `${exact(total - used)} ZEC.zy (${exact(used)} from credit)`;
  };
  const setTotal = () => {
    if (!sale || !el('total')) return;
    const n = whole('count-in', 20);
    el('total').textContent = n ? afterCredit(ticketsTotal(n)) : 'a whole number from 1 to 20';
    el('buy').disabled = !n || (gated() && !access?.listed);
  };
  const token = () => tokens.find((t) => t.key === el('burn-token')?.value) || tokens[0];
  const creditWanted = () => {
    const v = el('credit-in')?.value.trim();
    if (!/^\d*\.?\d+$/.test(v || '')) return null;
    const [i, f = ''] = v.split('.');
    if (f.length > 18) return null;
    const w = BigInt(i || '0') * WAD + BigInt(f.padEnd(18, '0'));
    return w > 0n ? w : null;
  };
  const priceOf = async (t) => {
    const p = prices.get(t.key);
    if (p && Date.now() - p.at < 30000) return p.value;
    const value = await zecPerToken(t);
    prices.set(t.key, { value, at: Date.now() });
    return value;
  };
  const setBurnTotal = async () => {
    const t = token();
    if (!t || !el('burn-total')) return;
    el('burn-go').textContent = t.chain === 'solana' ? 'Burn with a Solana wallet' : `Burn with an EVM wallet on ${chainName(t)}`;
    const want = creditWanted();
    if (!want) { el('burn-total').textContent = 'enter the credit you want, in ZEC'; el('burn-go').disabled = true; return; }
    try {
      const price = await priceOf(t);
      if (t !== token() || want !== creditWanted()) return;
      el('burn-total').textContent = `about ${units(burnFor(want, price, t.decimals), t.decimals)} ${t.symbol}, with ${BURN_HEADROOM}% headroom`;
      el('burn-go').disabled = false;
    } catch (e) {
      el('burn-total').textContent = e.message;
      el('burn-go').disabled = true;
    }
  };
  const setMintTotal = () => {
    if (!sale || !el('mint')) return;
    const left = sale.open_count - sale.open_next;
    const n = whole('mint-in', Math.min(10, left));
    el('mint').disabled = !n;
    el('mint').textContent = n ? `Mint ${n} for ${afterCredit(BigInt(sale.open_price) * BigInt(n))}` : `Mint 1 to ${Math.min(10, left)}`;
  };
  const mine = async () => {
    const box = el('mine-box');
    if (!current || !(await quietly())) { if (box) box.hidden = true; return null; }
    const r = await ask('zyn_saleTickets', {});
    if (gen !== generation) return null;
    const tickets = (r?.tickets || []).filter((t) => isHex(t.ticket) && Number.isSafeInteger(t.index) && Array.isArray(t.serials));
    box.hidden = !tickets.length;
    const late = sale?.phase === 'openmint';
    el('mine').innerHTML = tickets.map((t) => {
      const text = t.result === 'claimed' ? 'claimed'
        : t.result === 'won' ? (late ? 'not claimed in time — back in the open mint' : esc((t.names || []).join(', ')))
          : t.result === 'no win' ? 'no item this time'
            : sale?.phase === 'open' || sale?.phase === 'closed' ? 'waiting for the draw' : 'Nap could not check the draw yet';
      return `<li class="${t.result === 'won' && !late ? 'won' : ''}"><span>Ticket ${esc(t.index + 1)}${t.burned ? ' · burned' : ''}</span><b>${text}</b></li>`;
    }).join('');
    return tickets;
  };
  const read = async () => {
    if (gen !== generation) return;
    try {
      sale = await chainSale();
      if (gen !== generation) return;
      el('phase').textContent = sale ? PHASE_LABEL[sale.phase] || sale.phase : 'not open';
      // The lede sells tickets; once tickets are over it would be selling the past.
      el('lede').hidden = !!sale && sale.phase !== 'open';
      if (!sale) return show('none');
      el('lede-price').textContent = `${zec(sale.ticket_price)} ZEC.zy`;
      el('claim-price').textContent = `${zec(sale.claim_price)} ZEC.zy`;
      el('open-price').textContent = `${zec(sale.open_price)} ZEC.zy`;
      await readAccess();
      if (gen !== generation) return;
      setTotal();
      if (sale.phase === 'openmint') {
        const open = sale.open_count - sale.open_next;
        el('left').textContent = String(open);
        show(open > 0 ? 'openmint' : 'soldout');
        setMintTotal();
      } else show(sale.phase === 'open' ? 'open' : sale.phase === 'closed' ? 'closed' : 'drawn');
      // After show(), which hides every other section: credit sits beside each phase.
      await readCredit();
      if (gen !== generation) return;
      setTotal();
      setMintTotal();
      if (sale.phase === 'drawn' && Number.isSafeInteger(sale.close_zcash_height)) {
        el('claim-deadline').textContent = `Claims close at Zcash block ${(sale.close_zcash_height + CLAIM_BLOCKS).toLocaleString('en-US')} — about two weeks after the door shut. A win not claimed by then goes to the open mint.`;
      }
      const tickets = await mine().catch(() => null);
      if (gen !== generation || sale.phase !== 'drawn') return;
      const wins = (tickets || []).filter((t) => t.result === 'won');
      const unchecked = (tickets || []).some((t) => t.result === 'waiting');
      const serials = wins.flatMap((t) => t.serials);
      el('claim').disabled = !serials.length;
      el('claim-total-row').hidden = !serials.length;
      el('claim-total').textContent = afterCredit(BigInt(sale.claim_price) * BigInt(serials.length));
      const cards = await Promise.all(serials.map(async (x) => { const d = await describe(CAVE_Z, x); return `<figure>${art(d)}<figcaption>${esc(d.name)} ${rarityTag(d)}</figcaption></figure>`; }));
      if (gen !== generation) return;
      el('won-art').innerHTML = cards.join('');
      const out = el('out-claim');
      out.classList.remove('bad');
      if (!current) out.textContent = 'Connect Nap to see what your tickets drew.';
      else if (!tickets) out.textContent = current.provider.isNapWeb ? "Nap's window is closed — connect again to see your tickets." : 'Open Nap to see your tickets.';
      else if (unchecked) { out.textContent = 'Nap could not check the posted draw against the chain yet. Try again in a moment — nothing is lost.'; out.classList.add('bad'); }
      else if (!serials.length) out.textContent = tickets.some((t) => t.result === 'claimed') ? 'Everything this wallet drew is claimed.' : (tickets.length ? 'None of this wallet’s tickets drew an item.' : 'This wallet has no tickets in this sale.');
      else out.textContent = '';
    } catch (e) {
      if (gen !== generation) return;
      el('broke').textContent = e.message;
      show('broke');
    }
  };
  // Sale credit (§182): what this wallet holds — taking up any burn's credit
  // that has landed, which needs no approval — and what is left to burn for.
  const readCredit = async () => {
    const box = el('credit-box');
    if (!box) return;
    tokens = await burnTokens();
    const budget = BigInt(sale?.credit_budget || '0');
    credit = null;
    if (current && (await quietly())) {
      let c = await ask('zyn_saleCredit', {}).catch(() => null);
      if (c && BigInt(c.waiting || '0') > 0n) {
        await ask('zyn_takeUpCredit', {}).catch(() => null);
        c = await ask('zyn_saleCredit', {}).catch(() => c);
      }
      if (c) credit = BigInt(c.held || '0') + BigInt(c.waiting || '0');
    }
    const over = sale?.phase === 'openmint' && sale.open_next >= sale.open_count;
    box.hidden = !(tokens.length && sale && !over && (budget > 0n || (credit ?? 0n) > 0n));
    if (box.hidden) return;
    el('credit-held').textContent = credit != null ? `${zec(credit)} ZEC` : current ? 'open Nap to see it' : 'connect Nap to see it';
    el('credit-budget').textContent = `${zec(budget)} ZEC`;
    // Listed wallets only, while tickets sell in a gated sale: credit would buy nothing else then.
    el('burn-form').hidden = budget === 0n || (gated() && sale.phase === 'open' && !access?.listed);
    const select = el('burn-token');
    if (select.options.length !== tokens.length) {
      select.innerHTML = tokens.map((t) => `<option value="${esc(t.key)}">${esc(t.symbol)} · ${esc(chainName(t))}</option>`).join('');
      // A first guess at the credit wanted: one of whatever the sale sells now.
      el('credit-in').value = zec(sale.phase === 'openmint' ? sale.open_price : sale.phase === 'drawn' ? sale.claim_price : sale.ticket_price);
    }
    wireTips();
    setBurnTotal();
  };
  // Where this wallet stands in a gated sale, and what the box says about it.
  const readAccess = async () => {
    const box = el('gate');
    if (!box) return;
    box.hidden = !gated() || sale?.phase !== 'open';
    if (!gated()) { access = null; el('buy-box').hidden = false; return; }
    access = null;
    if (current && (await quietly())) access = await ask('zyn_saleAccess', {}).catch(() => null);
    const say = el('gate-say');
    const listed = !!access?.listed;
    el('gate-link').hidden = !current || listed;
    el('buy-box').hidden = !listed;
    if (!current) say.textContent = 'This sale is for the whitelist. Connect Nap to check your place.';
    else if (!access) say.textContent = 'This sale is for the whitelist. Nap could not check this wallet\u2019s place yet \u2014 update Nap, or try again in a moment.';
    else if (listed) {
      const base = Number(access.base);
      const bought = Number(access.bought);
      say.textContent = bought < base
        ? `On the list. ${zec(sale.ticket_price)} ZEC.zy per ticket for your first ${base} (${bought} bought), then ${zec(sale.ticket_price)} more for each ticket after.`
        : `On the list. You have ${bought} tickets; your next one is ${zec(access.next)} ZEC.zy, and each after that ${zec(sale.ticket_price)} more.`;
    } else say.textContent = 'This sale is for the whitelist, and this wallet is not on it yet. Enter the code you were sent to link it.';
  };
  el('gate-go').onclick = async () => {
    const out = el('out-gate');
    out.classList.remove('bad');
    if (!current) return pick();
    const code = el('gate-code').value.trim();
    if (!code) { out.textContent = 'Enter the code you were sent.'; out.classList.add('bad'); return; }
    el('gate-go').disabled = true;
    try {
      const r = await fetch('/api/whitelist/bind', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code, account: current.session.account }), credentials: 'omit' });
      const j = await r.json().catch(() => ({}));
      if (!j.ok) throw new Error(j.error || `HTTP ${r.status}`);
      out.textContent = 'Linked. This wallet goes on the sale\u2019s list when the next batch is posted on Zyn \u2014 this page shows it then.';
      el('gate-code').value = '';
      for (const wait of [60000, 300000, 900000]) setTimeout(read, wait);
    } catch (e) {
      out.textContent = e.message;
      out.classList.add('bad');
    } finally { if (el('gate-go')) el('gate-go').disabled = false; }
  };

  const act = (button, out, method, p, done, no) => async () => {
    if (!current) return pick();
    const params_ = p();
    if (!params_) return;
    el(button).disabled = true;
    el(out).classList.remove('bad');
    el(out).textContent = "Approve it in Nap's window.";
    try {
      const r = await ask(method, params_);
      if (gen !== generation) return;
      el(out).textContent = done(r);
      later(read);
    } catch (e) {
      if (gen !== generation) return;
      el(out).textContent = failText(e, no);
      el(out).classList.toggle('bad', e?.code !== 4001);
    } finally { if (el(button)) el(button).disabled = false; }
  };
  el('count-in').oninput = setTotal;
  el('credit-in').oninput = setBurnTotal;
  el('burn-token').onchange = setBurnTotal;
  el('mint-in').oninput = setMintTotal;
  el('burn-go').onclick = async () => {
    if (!current) return pick();
    const out = el('out-burn');
    out.classList.remove('bad');
    const fail = (t) => { out.textContent = t; out.classList.add('bad'); };
    // The sale again, right before burning: the tokens are gone either way,
    // so nothing is burned for more credit than the sale has left.
    try { sale = await chainSale(); } catch (e) { return fail(e.message); }
    const t = token();
    const want = creditWanted();
    const budget = BigInt(sale?.credit_budget || '0');
    if (gen !== generation || !t || !sale) return fail('Burning is closed.');
    if (!want) return fail('Enter the credit you want, in ZEC.');
    if (budget === 0n) return fail("The sale's credit is all taken.");
    if (want > budget) return fail(`Only ${exact(budget)} ZEC of credit is left in the sale. Ask for that much or less.`);
    el('burn-go').disabled = true;
    try {
      prices.delete(t.key);
      const amount = burnFor(want, await priceOf(t), t.decimals);
      const memo = await burnCode();
      const report = (x) => { out.textContent = x; };
      const id = t.chain === 'solana' ? await burnSolana(t, amount, memo, report) : await burnEvm(t, amount, memo, report);
      out.textContent = `Burned. Your credit shows here once ${chainName(t)} finalizes the burn and it is posted — usually within a minute or two. Transaction: ${String(id).slice(0, 14)}…`;
      for (const wait of [30000, 60000, 120000, 240000]) setTimeout(read, wait);
    } catch (e) {
      fail(e.code === 4001 || /reject|declin|cancel|denied/i.test(e.message) ? 'Not burned. Nothing left your wallet.' : e.message);
    } finally { if (el('burn-go')) el('burn-go').disabled = false; }
  };
  el('buy').onclick = act('buy', 'out', 'zyn_buyTickets', () => {
    const count = whole('count-in', 20);
    if (!count) { el('out').textContent = 'Enter a whole number of tickets, 1 to 20.'; el('out').classList.add('bad'); return null; }
    return { count, pay: exact(ticketsTotal(count)) };
  }, (r) => `${r?.tickets?.length || 0} ticket${r?.tickets?.length === 1 ? '' : 's'} bought. They are drawn once the door shuts.`, 'Not bought. Nothing was paid.');
  el('claim').onclick = act('claim', 'out-claim', 'zyn_claimItems', () => ({}), (r) => `Claimed ${r?.claimed?.length || 0}. They are in your Nap wallet once the epoch seals — about a minute.`, 'Not claimed. Nothing was paid.');
  el('mint').onclick = act('mint', 'out-mint', 'zyn_mintOpen', () => {
    const count = whole('mint-in', Math.min(10, sale.open_count - sale.open_next));
    if (!count) { el('out-mint').textContent = 'Enter a whole number to mint, 1 to 10.'; el('out-mint').classList.add('bad'); return null; }
    return { count };
  }, (r) => `Minted ${(r?.names || []).join(', ') || 'nothing'}.`, 'Not minted. Nothing was paid.');
  el('retry').onclick = read;
  walletListeners.add(read);
  await read();
}

// ---------------------------------------------------------------------------
// Moving between the shelf, an item and the market without reloading, so the
// connection to Nap — which for Nap in the browser is a window this page
// opened — carries across. Any other link loads normally.

// ---------------------------------------------------------------------------
// ZCaveDAO: membership is checked by the holder's own Nap, in this page; roles,
// proposals, the treasury and setup are read from public data — the chain for
// the floor and the sale, /dao.json for what the team publishes.

/// The "i" bubbles: what needs explaining opens beside the words it explains.
function wireTips() {
  document.querySelectorAll('.tip[data-tip]').forEach((b) => {
    b.onclick = () => {
      const open = b.getAttribute('aria-expanded') === 'true';
      b.setAttribute('aria-expanded', String(!open));
      const host = b.closest('p, div');
      host.nextElementSibling?.classList.contains('bubble') && host.nextElementSibling.remove();
      if (!open) {
        const bubble = document.createElement('p');
        bubble.className = 'bubble';
        bubble.textContent = b.dataset.tip;
        host.after(bubble);
      }
    };
  });
}

// ---------------------------------------------------------------------------
// The members' chat: the den (DECISIONS §184). den.js holds this device's MLS
// state and talks to the den; this page draws the rooms and asks Nap for the
// member pass only when no token is kept for today.

async function chatPage() {
  const gen = generation;
  wireTips();
  const out = el('chat-out');
  const say = (t, bad = false) => { out.textContent = t; out.classList.toggle('bad', bad); };
  const [{ DenClient }, { mountDen }] = await Promise.all([import('./den.js'), import('./den-ui.js')]);
  const den = new DenClient({ base: CHAT, keysUrl: '/den-keys.json', prefix: NETWORK });
  // Local builds only: the tests reach the client.
  if (LOCAL) window.__den = den;
  const me = await den.start();
  const open = (session) => {
    if (gen !== generation) return;
    el('me').textContent = me.name;
    document.querySelector('.ledger').hidden = true;
    document.querySelector('.register h1').hidden = true;
    el('den').hidden = false;
    mountDen(den, el, me);
    den.connect(session);
  };
  // A session or yesterday's token kept on this device: in without Nap.
  const kept = await den.session().catch(() => null);
  if (kept) return open(kept);
  el('enter').onclick = async () => {
    if (!current) return pick();
    el('enter').disabled = true;
    try {
      say("Approve the member pass in Nap's window.");
      const r = await ask('zyn_daoPass', {});
      if (gen !== generation) return;
      if (!isHex(r?.token)) throw new Error('Nap did not give a pass code. Update Nap and try again.');
      say('The door is checking your pass…');
      open(await den.fromPass(r.token));
      say('');
    } catch (e) {
      if (gen !== generation) return;
      say(/holds no item/i.test(e?.message || '') ? 'This wallet holds no Collection Z item, so it has no member pass.' : failText(e, 'Not entered. Nothing was registered.'), e?.code !== 4001);
    } finally { if (el('enter')) el('enter').disabled = false; }
  };
}

async function daoPage() {
  const gen = generation;
  wireTips();

  // Membership: Nap lists this wallet's items; only the count is used, here.
  let wanted = false;
  let members = 0;
  let dao = null;
  // 'loading' | 'set' | 'unset' | 'error': an unreachable chain is not "no DAO".
  let daoState = 'loading';
  // Set once the proposal list can be drawn: a membership check that answers
  // before the page has finished loading draws it then, not earlier.
  let proposalsReady = false;
  // What this visit already voted, so its buttons do not offer it again.
  const acted = new Set();
  const memberActs = () => {
    const on = members > 0 && daoState === 'set';
    el('get-pass').hidden = !on;
    el('new-prop').hidden = !on;
  };
  const check = async () => {
    const out = el('member-out');
    if (!current) { wanted = true; pick(); return; }
    wanted = false;
    out.className = 'out';
    out.textContent = 'Asking Nap…';
    try {
      const mine = await ask('zyn_nftItems');
      if (gen !== generation) return;
      const n = (mine?.items || []).filter((i) => i.collection === CAVE_Z && Number.isSafeInteger(i.serial)).length;
      out.textContent = n ? `You are a member: ${n} item${n === 1 ? '' : 's'}, ${n} vote${n === 1 ? '' : 's'}.` : 'No Cave item in this wallet yet.';
      out.classList.add(n ? 'good' : 'bad');
      members = n;
      memberActs();
      if (proposalsReady) drawProposals();
    } catch (e) {
      if (gen !== generation) return;
      out.textContent = failText(e, 'Nap was not asked. Nothing was shared.');
      out.classList.toggle('bad', e?.code !== 4001);
    }
  };
  el('check').onclick = check;
  // Another account (or none): what the last one held no longer counts.
  walletListeners.add(() => {
    members = 0;
    acted.clear();
    el('pass-box').hidden = true;
    memberActs();
    if (proposalsReady) drawProposals();
    if (wanted && current) check();
  });

  // A member pass (§172): Nap proves one item to the ledger and hands back a
  // code; the ledger keeps only the code's hash.
  el('get-pass').onclick = async () => {
    const out = el('member-out');
    el('get-pass').disabled = true;
    out.className = 'out';
    out.textContent = 'Approve the pass in Nap.';
    try {
      const r = await ask('zyn_daoPass', { collection: CAVE_Z });
      if (gen !== generation) return;
      el('pass-code').textContent = r.token;
      el('pass-box').hidden = false;
      out.textContent = 'Pass made. It lasts while you hold the item.';
      out.classList.add('good');
    } catch (e) {
      out.textContent = failText(e, 'No pass was made.');
      out.classList.toggle('bad', e?.code !== 4001);
    } finally { el('get-pass').disabled = false; }
  };
  el('pass-copy').onclick = () => navigator.clipboard?.writeText(el('pass-code').textContent).then(() => { el('pass-copy').textContent = 'Copied'; }).catch(() => {});

  // Roles: a note to the team until names carry roles.
  const dialog = el('role-dialog');
  const form = el('role-form');
  let opened = 0;
  document.querySelectorAll('[data-role]').forEach((b) => {
    b.onclick = () => {
      form.reset();
      form.role.value = b.dataset.role;
      el('role-title').textContent = `Take on ${b.closest('.role').querySelector('h3').textContent}`;
      el('role-out').textContent = '';
      opened = Date.now();
      dialog.showModal();
    };
  });
  el('role-close').onclick = () => dialog.close();
  form.onsubmit = async (e) => {
    e.preventDefault();
    const out = el('role-out');
    const send = el('role-send');
    send.disabled = true;
    out.className = 'out';
    out.textContent = 'Sending…';
    try {
      const r = await fetch('/api/roles', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        credentials: 'omit',
        body: JSON.stringify({ role: form.role.value, contact: form.contact.value, note: form.note.value, burrow: form.burrow.value, dwell: Date.now() - opened }),
      });
      const body = await r.json().catch(() => ({}));
      if (!body.ok) throw new Error(body.error || `Not sent (HTTP ${r.status}).`);
      out.textContent = body.already ? 'Already noted.' : 'Sent. The team will reach out.';
      out.classList.add('good');
    } catch (err) {
      out.textContent = err.message.replace(/^./, (c) => c.toUpperCase());
      out.classList.add('bad');
    } finally { send.disabled = false; }
  };

  // What the team publishes: the setup steps.
  let data = null;
  try {
    const r = await fetch('/dao.json', { credentials: 'omit', cache: 'no-cache' });
    data = r.ok ? await r.json() : null;
  } catch { data = null; }
  if (gen !== generation) return;

  // A proposal's text, read by its hash and checked against it.
  const texts = new Map();
  const textOf = async (h) => {
    if (texts.has(h)) return texts.get(h);
    let t = null;
    try {
      const r = await fetch(`/api/proposal-text?hash=${h}&network=${NETWORK}`, { credentials: 'omit' });
      const b = r.ok ? await r.json() : null;
      if (typeof b?.text === 'string') {
        const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(b.text));
        if ([...new Uint8Array(d)].map((x) => x.toString(16).padStart(2, '0')).join('') === h) t = b.text;
      }
    } catch { t = null; }
    if (t !== null) texts.set(h, t);
    return t;
  };

  // Chain data is checked before it is drawn: numbers are whole, ids are hex.
  const whole = (v) => Number.isSafeInteger(v) && v >= 0;
  const STATES = ['backing', 'voting', 'passed', 'unfunded', 'failed', 'lapsed'];
  const validProp = (x) => whole(x?.id) && isHex(x.text) && STATES.includes(x.state) && whole(x.backers) && whole(x.yes) && whole(x.no)
    && (x.until == null || whole(x.until)) && (x.pay == null || (isInt(x.pay.amount) && isHex(x.pay.asset) && isHex(x.pay.to)))
    && (x.terms == null || (whole(x.terms.backers) && whole(x.terms.quorum)))
    && (x.track == null || x.track === 'collection' || x.track === 'zyn') && (x.team == null || ['none', 'yes', 'no'].includes(x.team));
  // The Zyn track (§176): what the team's own vote weighs, from the chain.
  const teamRule = () => (dao?.zyn?.rule === 'weighted' ? `the team's vote is ${Number(dao.zyn.share_bps) / 100}% of the tally` : 'the team can veto, not pass');
  const short = (h) => `${h.slice(0, 8)}…${h.slice(-6)}`;
  const span = (secs) => {
    const m = Math.max(0, Math.floor(secs / 60));
    return m >= 2880 ? `${Math.floor(m / 1440)} days` : m >= 120 ? `${Math.floor(m / 60)} hours` : `${m} min`;
  };
  // Strict proposals end at a Zcash height (§173); first-rules ones at an epoch.
  const left = (x) => {
    if (x.strict) {
      const h = dao?.clock?.height;
      if (!whole(h)) return 'time left unknown';
      return `about ${span((x.until - h) * (dao.block_seconds || 75))} left`;
    }
    return `about ${span((x.until - (dao?.epoch ?? 0)) * (dao?.epoch_seconds || 15))} left`;
  };

  async function drawProposals() {
    if (gen !== generation) return;
    try {
      if (daoState !== 'set') {
        el('props-count').textContent = '—';
        el('props').innerHTML = `<p class="say">${daoState === 'error' ? 'The DAO could not be read from the chain. Try again shortly.' : daoState === 'loading' ? 'Loading…' : "This network's DAO is not set yet."}</p>`;
        return;
      }
      const props = (dao.proposals || []).filter(validProp);
      el('props-count').textContent = props.length ? `${props.length}` : 'none yet';
      if (!props.length) { el('props').innerHTML = `<p class="say">No proposals yet.${members ? ' Yours can be the first.' : ''}</p>`; return; }
      const rows = await Promise.all(props.map(async (x) => {
        const t = await textOf(x.text);
        const zyn = x.track === 'zyn';
        const pay = zyn ? `Zyn · ${teamRule()} · team ${x.team === 'yes' || x.team === 'no' ? `voted ${x.team}` : 'has not voted'}`
          : x.pay ? `Pays ${zec(x.pay.amount)} ${x.pay.asset === ZEC_ZY ? 'ZEC' : `of ${short(x.pay.asset)}`} to ${short(x.pay.to)}` : 'Pays nothing';
        const need = x.terms ? x.terms.backers : dao.backers;
        const quorum = x.terms ? ` · needs ${x.terms.quorum} voting` : '';
        const count = x.state === 'backing' ? `${x.backers} of ${need} backers · ${left(x)}` : x.state === 'voting' ? `${x.yes} yes · ${x.no} no${quorum} · ${left(x)}` : `${x.yes} yes · ${x.no} no`;
        const can = (c) => !acted.has(`${x.id}:${c === 'back' ? 'back' : 'vote'}`);
        const acts = !members ? '' : x.state === 'backing' && can('back') ? `<button class="act small" data-vote="back" data-p="${x.id}">Back</button>`
          : x.state === 'voting' && can('yes') ? `<button class="act small" data-vote="yes" data-p="${x.id}">Yes</button><button class="act small ghost" data-vote="no" data-p="${x.id}">No</button>` : '';
        return `<div class="prop"><div class="prop-head"><b>#${x.id}</b>${zyn ? '<span class="tag">Zyn</span>' : ''}<span class="tag ${x.state === 'voting' ? 'now' : ''}">${esc(x.state)}</span></div>
          <p class="prop-text">${t === null ? '<em>Text not found under its hash.</em>' : esc(t)}</p>
          <p class="prop-meta">${esc(pay)} · ${esc(count)}</p>${acts ? `<div class="row">${acts}</div>` : ''}</div>`;
      }));
      if (gen !== generation) return;
      el('props').innerHTML = rows.join('');
      el('props').querySelectorAll('[data-vote]').forEach((b) => {
        b.onclick = async () => {
          const out = el('props-out');
          const mine = el('props').querySelectorAll(`[data-p="${b.dataset.p}"]`);
          mine.forEach((x) => { x.disabled = true; });
          out.className = 'out';
          out.textContent = 'Approve in Nap.';
          try {
            const r = await ask('zyn_daoVote', { collection: CAVE_Z, proposal: Number(b.dataset.p), choice: b.dataset.vote });
            acted.add(`${b.dataset.p}:${b.dataset.vote === 'back' ? 'back' : 'vote'}`);
            out.textContent = r.items ? `Counted: ${r.items} item${r.items === 1 ? '' : 's'}.` : 'Your items had all acted on this proposal already.';
            out.classList.add('good');
            later(loadDao);
            drawProposals();
          } catch (e) {
            mine.forEach((x) => { x.disabled = false; });
            out.textContent = failText(e, 'Nothing was voted.');
            out.classList.toggle('bad', e?.code !== 4001);
          }
        };
      });
    } catch (e) {
      el('props').innerHTML = `<p class="say">The proposals could not be shown: ${esc(e?.message || String(e))}</p>`;
    }
  }

  // A new proposal: Nap proposes, then the text is published under its hash —
  // the store takes only texts the chain has a proposal for.
  const propDialog = el('prop-dialog');
  const propForm = el('prop-form');
  // The Zyn track shows only once the chain has one; a Zyn proposal pays nothing.
  const trackShown = () => {
    const zyn = propForm.track.value === 'zyn';
    propForm.querySelectorAll('.pay-field').forEach((f) => { f.hidden = zyn; });
    el('prop-zyn-note').hidden = !zyn;
    el('prop-zyn-note').textContent = zyn ? `About Zyn: it pays nothing, and ${teamRule()}. The team's vote is public.` : '';
  };
  propForm.track.onchange = trackShown;
  el('new-prop').onclick = () => {
    propForm.reset();
    el('prop-out').textContent = '';
    el('prop-track-box').hidden = !dao?.zyn;
    trackShown();
    propDialog.showModal();
  };
  el('prop-close').onclick = () => propDialog.close();
  const publish = async (text) => {
    for (let i = 0; i < 9; i += 1) {
      const r = await fetch('/api/proposal-text', { method: 'POST', headers: { 'content-type': 'application/json' }, credentials: 'omit', body: JSON.stringify({ text, network: NETWORK }) });
      const b = await r.json().catch(() => ({}));
      if (b.ok) return;
      if (r.status !== 409) throw new Error(b.error || `HTTP ${r.status}`);
      await new Promise((res) => setTimeout(res, 10000));
    }
    throw new Error('the chain did not show the proposal in time');
  };
  propForm.onsubmit = async (e) => {
    e.preventDefault();
    const out = el('prop-out');
    out.className = 'out';
    const text = propForm.text.value.trim();
    const track = dao?.zyn && propForm.track.value === 'zyn' ? 'zyn' : 'collection';
    const amount = track === 'zyn' ? '' : propForm.amount.value.trim();
    const to = track === 'zyn' ? '' : propForm.to.value.trim().toLowerCase();
    if (amount && !/^[0-9a-f]{64}$/.test(to)) { out.textContent = 'A payment needs the Nap account it goes to (64 hex characters).'; out.classList.add('bad'); return; }
    if (to && !amount) { out.textContent = 'A recipient needs an amount, or leave both empty.'; out.classList.add('bad'); return; }
    el('prop-send').disabled = true;
    try {
      out.textContent = 'Approve the proposal in Nap.';
      const made = await ask('zyn_daoPropose', { collection: CAVE_Z, text, track, pay: amount ? { asset: ZEC_ZY, amount, to } : undefined });
      // An older Nap ignores the track and proposes to the collection: say so.
      const stale = track === 'zyn' && made?.track !== 'zyn';
      // Done: the form gets out of the way, and the list says what comes next.
      propDialog.close();
      const po = el('props-out');
      po.className = 'out';
      po.textContent = 'Proposed. Publishing its text…';
      try {
        await publish(text);
        po.textContent = stale ? 'Your Nap is out of date: it proposed this to Cave, not Zyn. Update Nap and propose again.' : 'Proposed, and its text is published.';
        po.classList.add(stale ? 'bad' : 'good');
      } catch (err) {
        po.textContent = `Proposed, but its text is not published yet (${err.message}). Keep a copy of it.`;
        po.classList.add('bad');
      }
      later(loadDao);
      loadDao();
    } catch (err) {
      out.textContent = failText(err, 'Nothing was proposed.');
      out.classList.toggle('bad', err?.code !== 4001);
    } finally { el('prop-send').disabled = false; }
  };

  // The rules, as the chain has them: the proposals bubble says them.
  const rulesTip = () => {
    const b = document.querySelector('#h-props')?.closest('section')?.querySelector('.tip');
    if (!b || daoState !== 'set') return;
    const q = `${dao.quorum_bps / 100}%`;
    b.dataset.tip = dao.strict
      ? `${dao.backers} item${dao.backers === 1 ? '' : 's'} must back a proposal within ${span(dao.voting_epochs * (dao.block_seconds || 75))}; its vote then runs ${span(dao.voting_epochs * (dao.block_seconds || 75))}, counted in Zcash blocks. It passes with more yes than no and at least ${q} of the items outstanding when its vote opens taking part. A proposal keeps the rules it opened under.`
      : `${dao.backers} item${dao.backers === 1 ? '' : 's'} must back a proposal within ${span(dao.voting_epochs * (dao.epoch_seconds || 15))}; its vote then runs as long. It passes with more yes than no and at least ${q} of the items outstanding taking part.`;
  };

  async function loadDao() {
    try {
      const d = await chain('zyn_dao', { collection: CAVE_Z });
      if (gen !== generation) return;
      dao = d?.set ? d : null;
      daoState = dao ? 'set' : 'unset';
    } catch {
      if (gen !== generation) return;
      dao = null;
      daoState = 'error';
    }
    memberActs();
    rulesTip();
    const t = dao?.treasury?.find((x) => x.asset === ZEC_ZY);
    el('f-treasury-box').hidden = daoState !== 'set';
    if (daoState === 'set') el('f-treasury').textContent = `${zec(isInt(t?.amount) ? t.amount : '0')} ZEC`;
    el('statement').innerHTML = daoState === 'set'
      ? '<p class="say small">Only a passed proposal can pay out of the treasury; no key can move it. The operator orders transactions, so it could delay or refuse a vote, but it cannot speed one up or change a proposal\'s rules.</p>'
      : daoState === 'error' ? '<p class="say small">The treasury could not be read from the chain.</p>' : '';
    drawProposals();
  }
  proposalsReady = true;
  await loadDao();
  // Time left and tallies move: look again every minute while the page is open.
  const tick = setInterval(() => { if (gen !== generation) { clearInterval(tick); return; } loadDao(); }, 60000);

  if (data?.updated) el('updated').textContent = `updated ${data.updated}`;
  const steps = Array.isArray(data?.setup) ? data.setup : [];
  const label = { done: 'done', now: 'now', next: 'next', later: 'later' };
  el('steps').innerHTML = steps.map((x) => `<li><span>${esc(x.step)}</span><span class="tag ${esc(x.state)}">${esc(label[x.state] || x.state)}</span></li>`).join('');

  // The floor and the sale, from the chain itself.
  el('treasury-net').textContent = NETWORK;
  try {
    const m = await chain('zyn_market');
    if (gen !== generation) return;
    const c = (m?.collections || []).find((x) => x.asset === CAVE_Z);
    const sale = (m?.sales || []).find((x) => x.collection === CAVE_Z);
    if (c && isInt(c.floor) && isInt(c.outstanding)) {
      el('f-floor').textContent = `${zec(c.floor)} ZEC`;
      el('f-out').textContent = `${c.outstanding} of ${Number(c.cap).toLocaleString('en-US')}`;
      el('f-each').textContent = BigInt(c.outstanding) > 0n ? `${zec(BigInt(c.floor) / BigInt(c.outstanding))} ZEC` : '—';
    }
    const phase = { open: 'tickets on sale', closed: 'waiting for the draw', drawn: 'claims open', openmint: 'open mint' }[sale?.phase] || (sale ? 'sale on' : 'not open yet');
    el('f-sale').textContent = sale && Number.isSafeInteger(sale.tickets) ? `${phase} · ${sale.tickets} tickets` : phase;
  } catch (e) {
    if (gen !== generation) return;
    el('treasury-out').textContent = e.message;
    el('treasury-out').classList.add('bad');
  }
}

const PAGES = { shelf: shelfPage, item: itemPage, market: marketPage, mint: mintPage, dao: daoPage, chat: chatPage };
const ROUTES = { '/items/': 'shelf', '/item/': 'item', '/market/': 'market', '/mint/': 'mint', '/dao/': 'dao', '/chat/': 'chat' };
let params = new URLSearchParams(location.search);

function start(page) {
  generation += 1;
  walletListeners.clear();
  wireWallet();
  testnetBanner();
  // A page that fails to start says so on its own; nothing is left unhandled.
  Promise.resolve(PAGES[page]?.()).catch((e) => { const b = el('broke'); if (b) { b.textContent = e?.message || String(e); show('broke'); } });
}

/// Testnet is a different chain with worthless coins: said on every page.
function testnetBanner() {
  if (NETWORK !== 'testnet' || document.querySelector('.testnet-banner')) return;
  const b = document.createElement('p');
  b.className = 'testnet-banner';
  b.setAttribute('role', 'note');
  b.textContent = 'Testnet — prices, listings and coins here are for trying things out and have no value.';
  document.body.prepend(b);
}

async function go(url, push = true) {
  const html = await (await fetch(url.pathname, { credentials: 'omit' })).text();
  const next = new DOMParser().parseFromString(html, 'text/html');
  const page = next.body.dataset.page;
  if (!PAGES[page]) { location.href = url.href; return; }
  // Each page keeps its own layout in <head><style>; the next page's styles
  // come with its <main>, or it renders in the last page's.
  document.querySelectorAll('head style').forEach((st) => st.remove());
  next.querySelectorAll('head style').forEach((st) => document.head.append(document.adoptNode(st)));
  document.querySelector('main').replaceWith(document.adoptNode(next.querySelector('main')));
  // The top bar is outside <main> and stays; only its current-page mark moves.
  const here = url.pathname.replace(/\/$/, '');
  document.querySelectorAll('.topbar-nav a').forEach((a) => {
    if (a.getAttribute('href') === here) a.setAttribute('aria-current', 'page');
    else a.removeAttribute('aria-current');
  });
  document.body.dataset.page = page;
  document.title = next.title;
  if (push) history.pushState({}, '', url.href);
  params = new URLSearchParams(url.search);
  window.scrollTo(0, 0);
  // Screen readers and keyboards land on the new page, not the old link.
  const main = document.querySelector('main');
  main.setAttribute('tabindex', '-1');
  main.focus({ preventScroll: true });
  start(page);
}

document.addEventListener('click', (e) => {
  const a = e.target.closest('a[href]');
  if (!a || e.metaKey || e.ctrlKey || e.shiftKey || a.target) return;
  const url = new URL(a.getAttribute('href'), location.href);
  const path = url.pathname.endsWith('/') ? url.pathname : `${url.pathname}/`;
  if (url.origin !== location.origin || !ROUTES[path]) return;
  e.preventDefault();
  url.pathname = path;
  if (NETWORK === 'testnet' && !url.searchParams.has('network')) url.searchParams.set('network', 'testnet');
  go(url).catch(() => { location.href = url.href; });
});
window.addEventListener('popstate', () => go(new URL(location.href), false).catch(() => location.reload()));

start(PAGE);
