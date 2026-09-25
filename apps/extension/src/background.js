// The extension's background: it opens Nap's connect window, and nothing else.
//
// The wallet lives in the extension's own pages (the popup, and connect.html
// for sites), which hold the keys; sites talk to connect.html directly over a
// runtime port from the content script. This worker keeps no state a site
// depends on, so the browser may stop it between events without dropping a
// request — only the window's id is kept, in session storage, to focus the
// window rather than open a second one.

const CONNECT = 'connect.html';

async function openConnect() {
  const { connectWindow } = await chrome.storage.session.get('connectWindow');
  if (connectWindow != null) {
    try {
      await chrome.windows.update(connectWindow, { focused: true });
      return;
    } catch { /* closed since; open another */ }
  }
  const win = await chrome.windows.create({ url: chrome.runtime.getURL(CONNECT), type: 'popup', width: 440, height: 680, focused: true });
  await chrome.storage.session.set({ connectWindow: win.id });
}

chrome.windows.onRemoved.addListener(async (id) => {
  const { connectWindow } = await chrome.storage.session.get('connectWindow');
  if (connectWindow === id) await chrome.storage.session.remove('connectWindow');
});

chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  // Only a site's content script asks, and only to have the window opened.
  if (!msg || msg.kind !== 'nap:open' || !sender.tab) return false;
  openConnect().then(() => reply({ ok: true }), (e) => reply({ error: e.message }));
  return true;
});
