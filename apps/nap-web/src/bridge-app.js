// Nap Bridge: SOL and ETH in and out of Zyn as SOL.zy and ETH.zy
// (DECISIONS §146).
//
// In: a deposit on the other chain carries a code from this wallet — a
// commitment to this account and one of its nonces, never the account — and
// the bridge records it on Zyn, where it waits until this wallet claims it.
// Claiming happens here, on its own, whenever something is waiting.
//
// Out: a withdrawal commits to `keccak(address ‖ salt)`, so the chain never
// carries where the money went; the address is then shown to the bridge that
// pays it, and only to it.

import { checkedEpoch, deriveIdentity, forgetIdentity, openSealedWallet, parseAmount } from './core.js';
import { hex, scan, unhex, withdrawIntent } from './object.js';
import * as ledger from './ledger.js';
import * as mk from './market.js';
import * as sol from './solana.js';
import { keccak_256 } from '@noble/hashes/sha3.js';
import './styles.css';
import './market.css';
import './bridge.css';

const root = document.querySelector('#app');
const KEPT_KEY = 'nap.kept.v1';
const NETWORK_KEY = 'nap.network';
let config;
let identity = null;
let view = 'in';
let busy = false;
let state = { epoch: 0n, bridged: [], balances: new Map(), held: [], bps: 0n, waiting: [] };

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const $ = (sel) => root.querySelector(sel);
const read = (k) => { try { return localStorage.getItem(k); } catch { return null; } };
const write = (k, v) => { try { localStorage.setItem(k, v); } catch { /* this session only */ } };
const short = (h) => `${h.slice(0, 6)}…${h.slice(-4)}`;
const LAMPORT = 1_000_000_000n; // one lamport in WAD

function fmt(raw, places = 9) {
  const v = BigInt(raw);
  const w = v / mk.WAD;
  const f = (v % mk.WAD).toString().padStart(18, '0').slice(0, places).replace(/0+$/, '');
  return `${w}${f ? `.${f}` : ''}`;
}

// ---------------------------------------------------------------------------
// The bridges this network has, from config.json, checked against the chain.

function bridgesOf(net) {
  const b = net.bridges || {};
  const out = [];
  if (b.sol && /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(b.sol.vault || '')) {
    out.push({ key: 'sol', symbol: 'SOL', zy: 'SOL.zy', decimals: 9, network: b.sol.network, asset: hex(mk.bridgedAsset(b.sol.network)), ...b.sol });
  }
  if (b.eth && /^0x[0-9a-fA-F]{40}$/.test(b.eth.vault || '') && Number.isSafeInteger(b.eth.chainId)) {
    out.push({ key: 'eth', symbol: 'ETH', zy: 'ETH.zy', decimals: 18, network: `eip155:${b.eth.chainId}`, asset: hex(mk.bridgedAsset(`eip155:${b.eth.chainId}`)), ...b.eth });
  }
  return out;
}

async function loadConfig() {
  const response = await fetch('/config.json', { cache: 'no-store' });
  if (!response.ok) throw new Error('Deployment is missing config.json');
  const value = await response.json();
  const networks = Array.isArray(value.networks) && value.networks.length ? value.networks : [{ id: 'default', ...value }];
  const wanted = read(NETWORK_KEY);
  const chosen = networks.find((n) => n.id === wanted) || networks[0];
  const url = new URL(chosen.rpcUrl, location.origin);
  if (url.origin !== location.origin) throw new Error('rpcUrl must be a same-origin path');
  return { ...chosen, rpcUrl: url.pathname, networks, bridges: bridgesOf(chosen) };
}

const keptWallet = () => { try { return JSON.parse(read(KEPT_KEY) || 'null'); } catch { return null; } };
const call = (method, fields = {}) => ledger.call(config, method, fields);

// ---------------------------------------------------------------------------
// Exits waiting to be revealed: kept on this device until the node files them.

const exitsKey = () => `nap.bridge.exits.${config.chain}`;
const pendingExits = () => { try { return JSON.parse(read(exitsKey()) || '[]'); } catch { return []; } };
const savePendingExits = (v) => write(exitsKey(), JSON.stringify(v));

async function revealExits() {
  const left = [];
  for (const x of pendingExits()) {
    try {
      await call('zyn_bridgeReveal', { asset: x.asset, address: x.address, salt: x.salt });
    } catch (e) {
      if (!/no unpaid exit/.test(e.message)) left.push(x);
    }
  }
  savePendingExits(left);
}

// ---------------------------------------------------------------------------

async function refresh(quiet = false) {
  try {
    const status = await call('zyn_status');
    const epoch = checkedEpoch(status, config.chain);
    const [bridged, market] = await Promise.all([ledger.loadBridged(config), call('zyn_market').catch(() => ({}))]);
    const live = new Set(bridged.map((b) => b.asset));
    const next = { ...state, epoch, bridged, bps: BigInt(Number(market?.fees?.bridge_bps) || 0) };
    for (const b of config.bridges) b.live = live.has(b.asset);
    if (identity) {
      const me = identity;
      let waiting = await ledger.loadWaiting(config, me, bridged).catch(() => []);
      if (waiting.length && identity === me) {
        notice(`Claiming ${waiting.length} deposit${waiting.length > 1 ? 's' : ''} that arrived…`);
        await ledger.claimWaiting(config, me);
        waiting = [];
      }
      const published = await call('zyn_published');
      const { held, balances } = scan(published, { secret: me.viewing, nkey: me.nullifier, owner: me.account });
      Object.assign(next, { held, balances, waiting });
      await revealExits();
    }
    state = next;
    render();
    if (!quiet) notice(`Up to date at epoch ${epoch}.`);
  } catch (e) {
    notice(e.message === 'Failed to fetch' ? 'The chain is not answering. Nothing was sent.' : e.message, true);
  }
}

// ---------------------------------------------------------------------------
// In.

async function solanaBlockhash(b) {
  const r = await fetch(b.rpc, {
    method: 'POST', headers: { 'content-type': 'application/json' }, credentials: 'omit', referrerPolicy: 'no-referrer',
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'getLatestBlockhash', params: [{ commitment: 'finalized' }] }),
  });
  const j = await r.json().catch(() => ({}));
  const h = j?.result?.value?.blockhash;
  if (!h) throw new Error('Solana did not answer; try again shortly.');
  return h;
}

async function depositSol(b, amountText, say) {
  const wad = parseAmount(amountText);
  if (wad % LAMPORT !== 0n) throw new Error('SOL has nine decimal places');
  const { memo } = await ledger.bridgeMemo(config, identity);
  const wallets = sol.solanaWallets();
  await new Promise((r) => setTimeout(r, 300));
  if (!wallets.length) throw new Error('No Solana wallet in this browser. Install one (Phantom, Solflare, Backpack), switch it to devnet, and reload.');
  const w = wallets[0];
  say(`Connecting ${w.name}…`);
  const { accounts } = await w.features['standard:connect'].connect();
  const account = accounts?.[0];
  if (!account) throw new Error(`${w.name} shared no account.`);
  const message = sol.depositMessage({ owner: account.address, vault: b.vault, lamports: wad / LAMPORT, memo, blockhash: await solanaBlockhash(b) });
  say(`Approve the deposit of ${fmt(wad)} SOL in ${w.name}.`);
  return sol.signAndSend(w, account, sol.unsigned(message), `solana:${b.cluster || 'devnet'}`);
}

const selector = (s) => keccak_256(new TextEncoder().encode(s)).slice(0, 4);

async function depositEth(b, amountText, say) {
  const wei = parseAmount(amountText);
  const eth = window.ethereum;
  if (!eth?.request) throw new Error('No Ethereum wallet in this browser. Install one (MetaMask, Rabby) and reload.');
  const { commitment } = await ledger.bridgeMemo(config, identity);
  const [from] = await eth.request({ method: 'eth_requestAccounts' });
  const chainHex = `0x${b.chainId.toString(16)}`;
  if ((await eth.request({ method: 'eth_chainId' })) !== chainHex) {
    say('Switch your wallet to Sepolia…');
    await eth.request({ method: 'wallet_switchEthereumChain', params: [{ chainId: chainHex }] });
  }
  const asset = new Uint8Array(32);
  new DataView(asset.buffer).setUint32(28, Number(b.assetId || 1));
  const data = `0x${hex(selector('depositNative(bytes32,uint32)'))}${commitment}${hex(asset)}`;
  say(`Approve the deposit of ${fmt(wei, 18)} ETH in your wallet.`);
  return eth.request({ method: 'eth_sendTransaction', params: [{ from, to: b.vault, value: `0x${wei.toString(16)}`, data }] });
}

// ---------------------------------------------------------------------------
// Out.

function destinationBytes(b, text) {
  const t = String(text).trim();
  if (b.key === 'sol') return sol.solanaKey(t);
  if (!/^0x[0-9a-fA-F]{40}$/.test(t)) throw new Error('That is not an Ethereum address');
  return unhex(t.slice(2).toLowerCase());
}

async function withdraw(b, to, amountText) {
  const raw = parseAmount(amountText);
  if (b.key === 'sol' && raw % LAMPORT !== 0n) throw new Error('SOL has nine decimal places');
  if (b.key === 'sol' && raw <= 5_000n * LAMPORT) throw new Error('Withdraw more than the 0.000005 SOL network fee');
  const address = destinationBytes(b, to);
  const salt = crypto.getRandomValues(new Uint8Array(32));
  const destination = mk.exitCommitment(address, salt);
  const fee = (raw * state.bps) / 10_000n;
  const { inputs, change } = mk.fund(identity, state.held, unhex(b.asset), raw + fee);
  // Kept before submitting: if the page closes between the two steps, the
  // next visit still knows where this exit was meant to go.
  savePendingExits([...pendingExits(), { asset: b.asset, address: hex(address), salt: hex(salt) }]);
  await ledger.submitIntent(config, identity, withdrawIntent({ holder: identity.account, inputs, asset: unhex(b.asset), amount: raw, destination, outputs: change }));
  await revealExits();
}

// ---------------------------------------------------------------------------
// Screens.

function shell(content) {
  root.innerHTML = `<div class="grain"></div>
  <header class="mk-header">
    <a class="brand" href="/" aria-label="Nap Wallet"><img src="/nap-mark.png" alt=""><span>Nap</span><em>Bridge</em></a>
    <div class="mk-head-right">
      <span class="network"><i></i>${esc(config.network)} · chain ${config.chain}</span>
      <a class="mk-link" href="/">Wallet →</a>
    </div>
  </header>${content}
  <footer><span>A deposit carries a code, not your account. An exit's address is shown only to the bridge that pays it.</span><span>Nap Bridge · Zyn</span></footer>`;
}

function frame() {
  shell(`<main class="wallet mk">
    <aside class="rail">
      <div>${identity ? `<p class="kicker">BRIDGING AS</p><b class="mk-account">${short(identity.accountHex)}</b>` : '<p class="kicker">NOT OPEN</p><button class="mk-act mk-open" id="open-wallet">Open wallet</button>'}<p class="mk-epoch" id="epoch">epoch —</p></div>
      <nav id="tabs">${[['in', 'Bring in'], ['out', 'Send out']].map(([k, t]) => `<button data-view="${k}">${t}</button>`).join('')}</nav>
      ${identity ? '<button class="quiet" id="lock">Lock</button>' : ''}
    </aside>
    <section class="content">
      <div class="notice" id="notice" role="status"></div>
      <div id="view"></div>
    </section>
  </main>`);
  $('#lock')?.addEventListener('click', () => { forgetIdentity(identity); identity = null; frame(); render(); });
  $('#open-wallet')?.addEventListener('click', () => { view = 'wallet'; render(); });
  root.querySelectorAll('#tabs [data-view]').forEach((b) => { b.onclick = () => { view = b.dataset.view; render(); }; });
}

function notice(text, bad = false) {
  const el = $('#notice');
  if (!el) return;
  el.textContent = text;
  el.classList.add('show');
  el.classList.toggle('bad', bad);
}

function heading(eyebrow, title, lede) {
  return `<div class="topline"><div><p class="eyebrow">${esc(eyebrow)}</p><h1 class="mk-h1">${esc(title)}</h1></div><button class="refresh" id="refresh">↻ Refresh</button></div>${lede ? `<p class="br-lede">${esc(lede)}</p>` : ''}`;
}

function bindRefresh() {
  $('#refresh')?.addEventListener('click', () => refresh());
}

function balanceOf(b) {
  return state.balances.get(b.asset) || 0n;
}

function renderIn() {
  if (!config.bridges.length) {
    $('#view').innerHTML = `${heading('BRIDGES', 'Not on this network yet.', 'SOL and ETH bridge on the testnet today (Solana devnet, Ethereum Sepolia). Switch networks in the wallet to try them.')}`;
    return;
  }
  $('#view').innerHTML = `${heading('BRING IN', 'SOL and ETH, private on Zyn.', 'Send from your own Solana or Ethereum wallet. The deposit carries a one-time code from this wallet — not your account — and arrives here as a note only you can see. A new code every deposit, so two deposits are not linked by it.')}
    <div class="br-cards">${config.bridges.map((b) => `<section class="mk-card br-card" data-bridge="${b.key}">
      <p class="eyebrow">${esc(b.symbol)} · ${esc(b.network)}</p>
      <h3>${esc(b.zy)} <span>${identity ? `you hold ${fmt(balanceOf(b), b.decimals)}` : ''}</span></h3>
      ${b.live ? '' : '<p class="br-say error">This network has not registered it yet.</p>'}
      <label class="mk-field"><span>Amount of ${esc(b.symbol)}</span><input class="amt" inputmode="decimal" autocomplete="off" placeholder="0.05"></label>
      <button class="primary go" ${identity && b.live ? '' : 'disabled'}>Deposit from a${b.key === 'eth' ? 'n Ethereum' : ' Solana'} wallet <span>→</span></button>
      <p class="br-say say" role="status"></p>
      <p class="mk-fine">Into the vault <code>${esc(b.key === 'sol' ? b.vault : b.vault.toLowerCase())}</code>. Arrives once ${b.key === 'sol' ? 'Solana' : 'Ethereum'} finalizes it${b.key === 'eth' ? ' (about 15 minutes)' : ' (under a minute)'}, and this page claims it for you.${state.bps ? ` Bridge fee ${Number(state.bps) / 100}%.` : ''}</p>
    </section>`).join('')}</div>`;
  root.querySelectorAll('[data-bridge]').forEach((card) => {
    const b = config.bridges.find((x) => x.key === card.dataset.bridge);
    const say = (t, bad = false) => { const el = card.querySelector('.say'); el.textContent = t; el.classList.toggle('error', bad); };
    card.querySelector('.go').onclick = async () => {
      if (busy || !identity) return;
      busy = true;
      try {
        const id = b.key === 'sol' ? await depositSol(b, card.querySelector('.amt').value, say) : await depositEth(b, card.querySelector('.amt').value, say);
        const link = (b.explorer || '').replace('{tx}', id);
        say(`Sent. It arrives here once final${link ? '' : ''} — ${String(id).slice(0, 14)}…`);
        if (link) card.querySelector('.say').insertAdjacentHTML('beforeend', ` <a href="${esc(link)}" target="_blank" rel="noreferrer">view</a>`);
        for (const wait of [20_000, 45_000, 90_000, 240_000, 600_000, 1_000_000]) setTimeout(() => refresh(true), wait);
      } catch (e) {
        say(e.code === 4001 || /reject|declin|cancel/i.test(e.message) ? 'Not sent. Nothing left your wallet.' : e.message, true);
      } finally { busy = false; }
    };
  });
}

function renderOut() {
  const live = config.bridges.filter((b) => b.live);
  if (!identity) {
    $('#view').innerHTML = `${heading('SEND OUT', 'Open your wallet to send out.', '')}`;
    return;
  }
  $('#view').innerHTML = `${heading('SEND OUT', 'Back to Solana or Ethereum.', 'The chain records only a commitment to where it goes. The address is then shown to the bridge that pays it, and to nobody else.')}
    <section class="mk-card"><form id="out-form" class="br-out">
      <label class="mk-field"><span>From</span><select id="out-asset">${live.map((b) => `<option value="${b.key}">${esc(b.zy)} — you hold ${fmt(balanceOf(b), b.decimals)}</option>`).join('')}</select></label>
      <label class="mk-field"><span>To address</span><input id="out-to" autocomplete="off" spellcheck="false" placeholder="a Solana address, or 0x… on Ethereum"></label>
      <label class="mk-field"><span>Amount</span><input id="out-amt" inputmode="decimal" autocomplete="off" placeholder="0.02"></label>
      <button class="primary" type="submit" ${live.length ? '' : 'disabled'}>Send out <span>→</span></button>
      <p class="mk-fine">${state.bps ? `A ${Number(state.bps) / 100}% bridge fee rides on top. ` : ''}SOL pays its 0.000005 network fee out of the amount. The bridge pays once the other chain finalizes it.</p>
    </form></section>`;
  $('#out-form').onsubmit = async (e) => {
    e.preventDefault();
    if (busy) return;
    busy = true;
    try {
      const b = config.bridges.find((x) => x.key === $('#out-asset').value);
      await withdraw(b, $('#out-to').value, $('#out-amt').value);
      notice(`Sent out. The bridge pays it once it is final on ${b.key === 'sol' ? 'Solana' : 'Ethereum'}.`);
      await refresh(true);
    } catch (err) { notice(err.message, true); } finally { busy = false; }
  };
}

function renderWallet(error = '') {
  const kept = keptWallet();
  $('#view').innerHTML = `<section class="entry panel mk-entry">
    ${kept ? '<form id="kept-form"><p class="kicker">THIS DEVICE</p><h2>Open your wallet.</h2><label>Password<input id="kept-password" type="password" autocomplete="current-password" autofocus></label><button class="primary" type="submit">Open <span>→</span></button></form><p class="muted"><a href="" id="use-phrase">Use a recovery phrase instead</a></p>' : ''}
    <form id="phrase-form" ${kept ? 'hidden' : ''}><p class="kicker">RECOVERY PHRASE</p><h2>Wake your wallet.</h2><label>24-word recovery phrase<textarea id="words" rows="4" autocomplete="off" autocapitalize="none" spellcheck="false"></textarea></label><div class="split"><label>Passphrase<input id="passphrase" type="password" autocomplete="off" placeholder="Blank if unused"></label><label>Account<input id="account-index" type="number" min="0" value="0"></label></div><button class="primary" type="submit">Open <span>→</span></button></form>
    <p class="error" id="form-error" role="alert">${esc(error)}</p>
  </section>`;
  $('#use-phrase')?.addEventListener('click', (e) => { e.preventDefault(); $('#phrase-form').hidden = false; $('#kept-form').hidden = true; });
  $('#kept-form')?.addEventListener('submit', async (e) => {
    e.preventDefault();
    try { begin((await openSealedWallet(keptWallet(), $('#kept-password').value)).identity); } catch (err) { $('#form-error').textContent = err.message; }
  });
  $('#phrase-form').addEventListener('submit', (e) => {
    e.preventDefault();
    try {
      const id = deriveIdentity($('#words').value, $('#passphrase').value, Number($('#account-index').value));
      $('#words').value = '';
      begin(id);
    } catch (err) { $('#form-error').textContent = err.message; }
  });
}

function begin(id) {
  if (!id.viewing || !id.nullifier) {
    renderWallet('This kept wallet predates the object ledger. Open it with the recovery phrase once.');
    return;
  }
  identity = id;
  view = 'in';
  frame();
  render();
  refresh();
}

function render() {
  if (!$('#view')) return;
  $('#epoch').textContent = `epoch ${state.epoch}`;
  root.querySelectorAll('#tabs [data-view]').forEach((b) => b.classList.toggle('active', b.dataset.view === view));
  if (view === 'wallet' && identity) view = 'in';
  ({ in: renderIn, out: renderOut, wallet: renderWallet })[view]();
  bindRefresh();
}

(async () => {
  try {
    config = await loadConfig();
  } catch (e) {
    root.textContent = e.message;
    return;
  }
  frame();
  render();
  refresh(true);
  setInterval(() => { if (identity && !busy) refresh(true); }, 30_000);
})();
