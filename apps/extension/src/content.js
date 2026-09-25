// Bridges a page's Nap provider (inpage.js) to Nap's connect window.
//
// Requests go over a runtime port to connect.html, the extension page that
// holds the keys and asks the holder; the browser tells that page which
// origin the port belongs to. If the window is not open, the background opens
// it and this script connects once it answers. Closing the window rejects
// whatever was waiting, as closing Nap in the browser does.

const s = document.createElement('script');
s.src = chrome.runtime.getURL('inpage.js');
s.onload = () => s.remove();
(document.head || document.documentElement).appendChild(s);

const pending = new Map(); // page request id -> true, while Nap holds it
let port = null;
const toPage = (msg) => window.postMessage({ target: 'nap-inpage', ...msg }, location.origin);

/// One attempt to reach an open connect window: resolves once it says ready.
function tryConnect() {
  return new Promise((resolve, reject) => {
    let p;
    try { p = chrome.runtime.connect({ name: 'nap-site' }); } catch (e) { reject(e); return; }
    const timer = setTimeout(() => { try { p.disconnect(); } catch { /* gone */ } reject(new Error('no answer')); }, 1500);
    p.onDisconnect.addListener(() => { clearTimeout(timer); void chrome.runtime.lastError; reject(new Error('closed')); });
    p.onMessage.addListener(function first(m) {
      if (!m || m.nap !== 'ready') return;
      clearTimeout(timer);
      p.onMessage.removeListener(first);
      resolve(p);
    });
  });
}

function adopt(p) {
  port = p;
  p.onMessage.addListener((m) => {
    if (!m || m.nap !== 'response' || !pending.has(m.id)) return;
    pending.delete(m.id);
    toPage({ id: m.id, result: m.result, error: m.error });
  });
  p.onDisconnect.addListener(() => {
    if (port === p) port = null;
    for (const id of pending.keys()) toPage({ id, error: { code: 4001, message: "Nap's window was closed" } });
    pending.clear();
  });
  return p;
}

/// The port to Nap, opening its window first when `open` is set.
async function reach(open) {
  if (port) return port;
  try { return adopt(await tryConnect()); } catch { /* not open */ }
  if (!open) return null;
  const r = await chrome.runtime.sendMessage({ kind: 'nap:open' });
  if (r && r.error) throw new Error(r.error);
  for (let i = 0; i < 20; i++) {
    try { return adopt(await tryConnect()); } catch { await new Promise((ok) => setTimeout(ok, 300)); }
  }
  throw new Error("Nap's window did not open");
}

window.addEventListener('message', async (ev) => {
  if (ev.source !== window || !ev.data || ev.data.target !== 'nap-content') return;
  const { id, method, params } = ev.data;
  if (typeof method !== 'string') return;
  try {
    // Asking whether a session exists never opens a window by itself.
    const p = await reach(method !== 'nap_session');
    if (!p) { toPage({ id, result: null }); return; }
    pending.set(id, true);
    p.postMessage({ nap: 'request', id, method, params: params && typeof params === 'object' ? params : {} });
  } catch (e) {
    pending.delete(id);
    toPage({ id, error: { code: 4001, message: e.message || 'Nap could not be reached' } });
  }
});
