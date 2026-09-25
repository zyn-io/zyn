// Where the wallet's endpoints live.
//
// On nap.cavebro.win the pages and the endpoints share one origin, and every
// configured URL must be a path on it. The extension ships the same pages
// inside itself (chrome-extension://…), so there the endpoints stay on Nap's
// server — the one origin its host permission and page policy admit — and the
// same rule applies against that origin instead. Keys never leave the page's
// own origin either way; only chain reads and signed frames go out.

/// Set only by the extension build (`vite build --mode extension`).
const EXT_API = import.meta.env.VITE_NAP_API || '';

export const IN_EXTENSION = !!EXT_API;
export const API_ORIGIN = EXT_API ? new URL(EXT_API).origin : location.origin;

/// A configured endpoint, checked to be a plain path on API_ORIGIN. On the web
/// it stays a path (fetched same-origin); in the extension it becomes the full
/// URL on Nap's server.
export function endpoint(value, key) {
  const url = new URL(value, API_ORIGIN);
  if (!value || url.origin !== API_ORIGIN || url.username || url.password || url.search || url.hash) {
    throw new Error(`${key} must be a path on ${API_ORIGIN}`);
  }
  return IN_EXTENSION ? url.origin + url.pathname : url.pathname;
}

/// The extension's toolbar popup (opened as `index.html?popup=1`): narrow, and
/// it closes when it loses focus, so other pages open in a tab instead.
export const IN_POPUP = IN_EXTENSION && new URLSearchParams(location.search).has('popup');

/// Go to another of Nap's pages: in place, or in a new tab from the popup.
export function openPage(path) {
  if (IN_POPUP) window.open(path, '_blank');
  else location.href = path;
}
