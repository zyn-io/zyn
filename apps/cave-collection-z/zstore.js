// Cave's shelf, item and market pages on the object ledger, through Nap.
//
// What anyone may see is read from the public chain: the order book, and the
// collection's own index (name, art, rarity, traits — the frozen metadata).
// What only a holder may see — their items, what the market owes them — is
// asked of the Nap they connected, and every write (list, cancel, buy,
// collect) waits for them to approve it in Nap's window. This page never
// holds a key and never learns more than Nap chooses to tell it.

import { connect, discover, NAP_WEB } from './nap-connect.js';

const params = new URLSearchParams(location.search);
const NETWORK = params.get('network') === 'testnet' ? 'testnet' : 'mainnet';
const RPC = NETWORK === 'testnet' ? 'https://zyn-rpc.cavebro.win/object' : 'https://zyn-rpc.cavebro.win/';
const LOCAL = ['localhost', '127.0.0.1'].includes(location.hostname);
const NAP = (LOCAL && params.get('nap')) || NAP_WEB;
const CAVE_Z = '024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e';
const WAD = 10n ** 18n;
const PAGE = document.body.dataset.page;

const el = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const isHex = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
const isInt = (v) => typeof v === 'string' && /^\d{1,40}$/.test(v);
const withNet = (path) => (NETWORK === 'testnet' ? `${path}${path.includes('?') ? '&' : '?'}network=testnet` : path);

function zec(raw) {
  const v = BigInt(raw);
  const frac = (v % WAD).toString().padStart(18, '0').replace(/0+$/, '').slice(0, 8);
  return `${v / WAD}${frac ? `.${frac}` : ''}`;
}

function show(name) {
  document.querySelectorAll('.ledger-body > .state').forEach((s) => { s.hidden = s.id !== `s-${name}`; });
}

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

async function publicOffers() {
  const id = crypto.randomUUID();
  const r = await fetch(RPC, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id, method: 'zyn_offers', params: [] }),
    credentials: 'omit',
    referrerPolicy: 'no-referrer',
  });
  const body = await r.json();
  if (body.error) throw new Error(body.error.message || 'The chain did not answer');
  return (body.result?.offers || []).filter((o) => isHex(o.item) && isHex(o.asset) && isHex(o.collection)
    && isInt(o.pay_raw) && isInt(o.price_raw) && Number.isSafeInteger(o.serial) && o.serial >= 0);
}

// ---------------------------------------------------------------------------
// The wallet: Nap, through Nap Connect.

let current = null;
const walletListeners = new Set();

function canAskQuietly() {
  return !!current && (!current.provider.isNapWeb || current.provider.isOpen());
}

function ask(method, p) {
  if (!current) { pick(); return Promise.reject(new Error('Connect Nap first.')); }
  return current.provider.request({ method, params: p });
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
    body.querySelector('.zwallet-off').onclick = () => { current = null; renderWalletButtons(); walletListeners.forEach((f) => f()); dialog.close(); };
    return;
  }
  body.innerHTML = '<p class="say">Looking for Nap…</p>';
  const list = await discover({ web: NAP });
  body.innerHTML = list.map((w, i) => `<button type="button" class="act zwallet-option" data-i="${i}">${esc(w.info.name)}</button>`).join('')
    + '<p class="out" id="zwallet-status"></p><p class="say small">Connecting tells this site your Zyn account on this network — not your balances or keys. Every sale, purchase and collection is approved in Nap.</p>';
  body.querySelectorAll('[data-i]').forEach((b) => {
    b.onclick = async () => {
      const entry = list[Number(b.dataset.i)];
      const status = body.querySelector('#zwallet-status');
      status.textContent = entry.provider.isNapWeb ? "Nap's window is open — unlock it there and approve." : 'Approve the connection in Nap.';
      try {
        current = await connect(entry, { network: NETWORK });
        current.provider.on?.('disconnect', () => { current = null; renderWalletButtons(); walletListeners.forEach((f) => f()); });
        renderWalletButtons();
        dialog.close();
        walletListeners.forEach((f) => f());
      } catch (e) {
        status.textContent = e.code === 4001 ? 'Not connected. Nothing was shared.' : e.message;
      }
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
  const read = async () => {
    if (!current) return show('connect');
    if (!canAskQuietly()) return show('ready');
    show('looking');
    try {
      const [mine, owed] = await Promise.all([ask('zyn_nftItems'), ask('zyn_owed')]);
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
      el('shelf').innerHTML = cards.join('');
      show('shelf');
    } catch (e) {
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
    } catch (e) { say(e.message, true); } finally { el('collect').disabled = false; }
  };
  walletListeners.add(read);
  show('connect');
}

// ---------------------------------------------------------------------------
// One item: public art and traits; list, cancel or buy through Nap.

async function itemPage() {
  const collection = isHex(params.get('collection')) ? params.get('collection') : CAVE_Z;
  const serial = Number(params.get('serial'));
  if (!Number.isSafeInteger(serial) || serial < 0) return show('noid');
  const d = await describe(collection, serial);
  el('name').textContent = d.name;
  el('sub').textContent = d.rarity ? `${d.rarity} · rank ${d.rank} of 4,444` : 'in the cave';
  document.title = `${d.name} — Cave`;
  el('art').innerHTML = art(d, 'art big');
  el('facts').innerHTML = Object.entries(d.traits).map(([k, v]) => `<div><dt>${esc(k)}</dt><dd>${esc(v)}</dd></div>`).join('')
    + `<div><dt>Serial</dt><dd>${serial}</dd></div>`;

  let offer = null;
  let mine = null;
  const read = async () => {
    say('');
    try {
      offer = (await publicOffers()).find((o) => o.collection === collection && o.serial === serial) || null;
    } catch (e) { say(`The order book could not be read: ${e.message}`, true); }
    mine = null;
    if (current && canAskQuietly()) {
      try {
        mine = (await ask('zyn_nftItems'))?.items?.find((i) => i.collection === collection && i.serial === serial) || null;
      } catch { /* the public half still stands */ }
    }
    el('state-tag').textContent = mine ? (mine.listed ? 'yours · listed' : 'yours') : offer ? 'for sale' : 'held';
    el('asking').textContent = offer ? `${zec(offer.price_raw)} ZEC.zy (you pay ${zec(offer.pay_raw)})` : 'not listed';
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
    try { await ask('zyn_nftList', { asset: mine.asset, price }); say('Listed. It rests on the book once the epoch seals.'); setTimeout(read, 12000); }
    catch (e) { say(e.message, true); } finally { el('list').disabled = false; }
  };
  el('cancel').onclick = async () => {
    el('cancel').disabled = true;
    try { await ask('zyn_nftCancel', { asset: mine.asset }); say('Taken back. It is yours to hold or list again.'); setTimeout(read, 12000); }
    catch (e) { say(e.message, true); } finally { el('cancel').disabled = false; }
  };
  el('take').onclick = async () => {
    if (!current) return pick();
    el('take').disabled = true;
    try { await ask('zyn_nftBuy', { item: offer.item, pay: zec(offer.pay_raw) }); say(`Bought ${d.name}. It is in your Nap wallet once the epoch seals.`); setTimeout(read, 12000); }
    catch (e) { say(e.message, true); } finally { el('take').disabled = false; }
  };
  el('again').onclick = read;
  el('open-nap').onclick = async () => { await pick(); };
  walletListeners.add(read);
  await read();
}

// ---------------------------------------------------------------------------
// The market: every resting listing, public; bought through Nap.

async function marketPage() {
  const read = async () => {
    show('looking');
    let offers;
    try { offers = await publicOffers(); } catch (e) { el('broke').textContent = e.message; return show('broke'); }
    el('count').textContent = String(offers.length);
    if (!offers.length) return show('empty');
    // The book does not say whose a listing is. The holder's own Nap does,
    // so their listings read "Yours" instead of offering them to themselves.
    const yours = new Set();
    if (current && canAskQuietly()) {
      try {
        for (const it of (await ask('zyn_nftItems'))?.items || []) if (it.listed) yours.add(it.asset);
      } catch { /* the public book still stands */ }
    }
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
    el('rows').innerHTML = rows.join('');
    el('rows').querySelectorAll('[data-buy]').forEach((b) => {
      b.onclick = async () => {
        if (!current) return pick();
        const o = offers[Number(b.dataset.buy)];
        b.disabled = true;
        try {
          await ask('zyn_nftBuy', { item: o.item, pay: zec(o.pay_raw) });
          say(`Bought ${(await describe(o.collection, o.serial)).name}. It is in your Nap wallet once the epoch seals.`);
          setTimeout(read, 12000);
        } catch (e) { say(e.message, true); b.disabled = false; }
      };
    });
    show('book');
  };
  el('again').onclick = read;
  el('retry').onclick = read;
  walletListeners.add(read);
  await read();
}

wireWallet();
({ shelf: shelfPage, item: itemPage, market: marketPage })[PAGE]?.();
