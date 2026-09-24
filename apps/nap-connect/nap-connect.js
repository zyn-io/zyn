// Nap Connect — how a site finds a Nap wallet and talks to it.
//
// One interface, `provider.request({ method, params })`, over whichever Nap
// the visitor has:
//
//   - The Nap extension (desktop Nap behind it). It announces itself the way
//     EIP-6963 has Ethereum wallets do it — a `nap:announceProvider` event in
//     answer to `nap:requestProvider` — so several wallets can coexist and
//     the site lists them rather than grabbing whichever wrote a global
//     first. Older builds that only set `window.zyn` are picked up too.
//   - Nap in the browser. No install: the wallet opens in its own window on
//     its own origin (`/connect.html`), the pattern web-only wallets use, and
//     the two windows talk with postMessage. The keys never leave Nap's
//     origin, and every approval is drawn by Nap, where this page cannot
//     reach.
//
// Methods (errors carry EIP-1193 codes: 4001 rejected, 4100 not connected,
// 4200 unsupported):
//
//   nap_connect {network?}   → {account, network, chain, zcash: {network, address} | null}
//   nap_session              → the same, or null; never prompts
//   nap_disconnect
//   zec_getAddress           → {network, address}
//   zec_pay {uri} | {address, amount, memo?}  → {txid}   (uri: ZIP-321)
//   zyn_nftOffers, zyn_nftItems, zyn_owed, zyn_nftBuy {item, pay},
//   zyn_nftList {asset, price}, zyn_nftCancel {asset}, zyn_nftRedeem {asset},
//   zyn_collect
//   zyn_saleInfo {collection?}, zyn_saleTickets {collection?},
//   zyn_buyTickets {count, pay?}, zyn_claimItems, zyn_mintOpen {count},
//   zyn_burnMemo → {memo, commitment}  (a burn on Solana with this memo
//     becomes this wallet's tickets, §145)
//     (the sale, §144; collection defaults to Collection Z)
//
// Call `request` from a click. Opening Nap's window is a popup, and browsers
// only allow a popup that a click asked for.
//
// This file is dependency-free and copied verbatim into sites that use it
// (`apps/cave-market/nap-connect.js`); a test keeps the copies identical.

export const NAP_WEB = 'https://nap.cavebro.win';
export const VERSION = 1;

const WEB_RDNS = 'win.cavebro.nap.web';
const LEGACY_RDNS = 'win.cavebro.nap.extension';

/// Every Nap this page can reach, extension first. Resolves after `timeout`
/// ms of listening for announcements.
export function discover({ timeout = 400, web = NAP_WEB } = {}) {
  return new Promise((resolve) => {
    const found = new Map();
    const onAnnounce = (ev) => {
      const d = ev.detail;
      if (!d || !d.info || typeof d.info.uuid !== 'string' || typeof d.provider?.request !== 'function') return;
      found.set(d.info.uuid, { info: Object.freeze({ ...d.info }), provider: d.provider });
    };
    window.addEventListener('nap:announceProvider', onAnnounce);
    window.dispatchEvent(new Event('nap:requestProvider'));
    setTimeout(() => {
      window.removeEventListener('nap:announceProvider', onAnnounce);
      const list = [...found.values()];
      // An extension from before announcements: it only sets window.zyn.
      if (!list.length && typeof window.zyn?.request === 'function') {
        list.push({ info: Object.freeze({ uuid: 'legacy-window-zyn', name: 'Nap Wallet', detail: 'Extension, with desktop Nap', rdns: LEGACY_RDNS, icon: '' }), provider: window.zyn });
      }
      if (web) {
        list.push({
          info: Object.freeze({ uuid: 'nap-web', name: 'Nap in the browser', detail: new URL(web).host, rdns: WEB_RDNS, icon: `${new URL(web).origin}/nap-mark.png` }),
          provider: webWallet({ url: web }),
        });
      }
      resolve(list);
    }, timeout);
  });
}

class NapError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}

/// Nap in the browser, reached through its connect window.
export function webWallet({ url = NAP_WEB, width = 440, height = 680 } = {}) {
  const origin = new URL(url).origin;
  let popup = null;
  let ready = null;
  let markReady = null;
  let watch = 0;
  let seq = 0;
  const pending = new Map();
  const listeners = new Map();

  const emit = (name, value) => (listeners.get(name) || []).forEach((fn) => { try { fn(value); } catch { /* a listener's problem */ } });

  function closed() {
    clearInterval(watch);
    popup = null;
    ready = null;
    const err = new NapError(4001, "Nap's window was closed");
    for (const p of pending.values()) p.reject(err);
    pending.clear();
    emit('disconnect', err);
  }

  window.addEventListener('message', (ev) => {
    // Only Nap's own window, at Nap's own origin, is listened to.
    if (ev.origin !== origin || !popup || ev.source !== popup) return;
    const d = ev.data;
    if (!d || typeof d !== 'object') return;
    if (d.nap === 'ready') { markReady?.(); return; }
    if (d.nap !== 'response') return;
    const p = pending.get(d.id);
    if (!p) return;
    pending.delete(d.id);
    if (d.error) p.reject(new NapError(Number(d.error.code) || -32603, String(d.error.message || 'Nap refused')));
    else p.resolve(d.result);
  });

  function open() {
    if (popup && !popup.closed) return;
    const left = Math.max(0, (window.screenX || 0) + (window.outerWidth || width) - width - 24);
    const top = Math.max(0, (window.screenY || 0) + 64);
    popup = window.open(`${origin}/connect.html`, 'nap-connect', `popup,width=${width},height=${height},left=${left},top=${top}`);
    if (!popup) throw new NapError(4001, "The browser blocked Nap's window. Allow pop-ups for this site, then try again.");
    ready = new Promise((resolve) => { markReady = resolve; });
    clearInterval(watch);
    watch = setInterval(() => { if (!popup || popup.closed) closed(); }, 500);
  }

  return Object.freeze({
    isNapWeb: true,
    /// Whether Nap's window is open. A read can only go through an open
    /// window without a click; a site checks this before asking unprompted.
    isOpen: () => !!popup && !popup.closed,
    /// Must be called from a click (or other user gesture) whenever Nap's
    /// window may need opening: the window is opened before anything waits.
    request({ method, params } = {}) {
      if (typeof method !== 'string') return Promise.reject(new NapError(-32602, 'method is required'));
      // Asking whether a session exists must not open a window by itself.
      if (method === 'nap_session' && (!popup || popup.closed)) return Promise.resolve(null);
      try { open(); } catch (e) { return Promise.reject(e); }
      const id = ++seq;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        ready.then(() => {
          if (!popup || popup.closed) return;
          popup.postMessage({ nap: 'request', id, method, params: params || {} }, origin);
          try { popup.focus(); } catch { /* some browsers refuse; harmless */ }
        });
      });
    },
    on(name, fn) { listeners.set(name, [...(listeners.get(name) || []), fn]); },
    close() { try { popup?.close(); } catch { /* already gone */ } closed(); },
  });
}

/// Connect through one of `discover()`'s entries.
export async function connect(entry, { network } = {}) {
  let session;
  try {
    session = await entry.provider.request({ method: 'nap_connect', params: network ? { network } : {} });
  } catch (e) {
    // An extension from before nap_connect: its account read is the nearest
    // thing, and it asks the desktop app, which the holder already runs.
    if (!/unknown method/i.test(e.message)) throw e;
    const a = await entry.provider.request({ method: 'zyn_account' });
    session = { account: a.account, network: a.network, chain: a.chain, zcash: a.address ? { network: a.network, address: a.address } : null };
  }
  return { ...entry, session };
}
