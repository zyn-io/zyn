// The unlocked wallet, while the browser runs — the extension only.
//
// A toolbar wallet opens and closes with every click; asking for the password
// (600,000 rounds of PBKDF2) each time would make it a lock screen with a
// wallet behind it. So once opened, the derived keys are held in
// `chrome.storage.session`: kept in memory, never written to disk, gone when
// the browser quits, and readable only by the extension's own pages — not by
// the content script that runs on websites. It lapses after IDLE_MS without
// use, and Lock ends it at once. The web and phone builds have no such store
// and keep asking, as before.
import { exportKeys, importKeys } from './core.js';

const store = globalThis.chrome?.storage?.session;
const KEY = 'nap.session';
export const IDLE_MS = 15 * 60 * 1000;
export const hasSession = !!store;

/// Hold the opened wallet for the extension's other pages and the next popup.
export async function saveSession(identity, receiving = []) {
  if (!store) return;
  await store.set({ [KEY]: { keys: exportKeys(identity, receiving), index: identity.accountIndex, until: Date.now() + IDLE_MS } }).catch(() => {});
}

/// The wallet opened earlier in this browser session, or null. Using it
/// pushes the idle deadline out again.
export async function loadSession() {
  if (!store) return null;
  try {
    const held = (await store.get(KEY))[KEY];
    if (!held?.keys) return null;
    if (!(held.until > Date.now())) { await store.remove(KEY); return null; }
    await store.set({ [KEY]: { ...held, until: Date.now() + IDLE_MS } });
    return importKeys(held.keys, held.index);
  } catch { return null; }
}

export async function endSession() {
  await store?.remove(KEY).catch(() => {});
}
