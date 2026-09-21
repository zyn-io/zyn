// Shows a first-run screen when the desktop app is not reachable.
//
// The extension holds no keys: everything it does is a call to Nap Wallet on
// the loopback address. Without the app running the popup can only fail, and
// a wallet that opens onto an error is indistinguishable from a broken one —
// to a new user and to a store reviewer alike. So the unreachable case gets a
// screen of its own that says what is missing and how to get it.
const DOWNLOAD_URL = 'https://nap.cavebro.win';
const PROBE_PATH = '/api/overview';
const PROBE_TIMEOUT_MS = 2500;

function base() {
  return window.ZYN_API_BASE || '';
}

/// Whether the desktop app answers. Any reply at all counts: a 401 or a 500
/// still means something is listening, and only a transport failure means
/// the app is absent. Distinguishing those keeps the screen honest — it
/// claims the app is not running only when it truly cannot be reached.
async function reachable() {
  const stop = new AbortController();
  const timer = setTimeout(() => stop.abort(), PROBE_TIMEOUT_MS);
  try {
    await fetch(base() + PROBE_PATH, { signal: stop.signal });
    return true;
  } catch (_) {
    return false;
  } finally {
    clearTimeout(timer);
  }
}

function build() {
  const el = document.createElement('div');
  el.id = 'nap-agent-gate';
  el.innerHTML = `
    <style>
      #nap-agent-gate {
        position: fixed; inset: 0; z-index: 2147483647;
        display: flex; flex-direction: column; align-items: center;
        justify-content: center; gap: 14px; padding: 28px; text-align: center;
        background: #070907; color: #e8efe8;
        font: 14px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif;
      }
      #nap-agent-gate img { width: 56px; height: 56px; opacity: .9; }
      #nap-agent-gate h1 { font-size: 16px; font-weight: 600; margin: 0; }
      #nap-agent-gate p { margin: 0; max-width: 30ch; color: #9bb09b; }
      #nap-agent-gate .row { display: flex; gap: 8px; margin-top: 4px; }
      #nap-agent-gate a, #nap-agent-gate button {
        font: inherit; padding: 8px 16px; border-radius: 8px;
        border: 1px solid #2b3a2b; background: #101a10; color: #e8efe8;
        cursor: pointer; text-decoration: none;
      }
      #nap-agent-gate a { background: #1c5e32; border-color: #1c5e32; }
      #nap-agent-gate a:hover { background: #227038; }
      #nap-agent-gate button:hover { background: #172517; }
      #nap-agent-gate .note { font-size: 12px; color: #6b806b; max-width: 32ch; }
    </style>
    <img src="nap-mark.png" alt="">
    <h1>Nap Wallet isn't running</h1>
    <p>This extension holds no keys. It talks to the Nap Wallet app on your
       own machine, and that app isn't answering.</p>
    <div class="row">
      <a id="nap-gate-get" href="${DOWNLOAD_URL}" target="_blank" rel="noopener">Get Nap Wallet</a>
      <button id="nap-gate-retry" type="button">Retry</button>
    </div>
    <p class="note">Already installed? Open the app, then retry.</p>
  `;
  el.querySelector('#nap-gate-retry').addEventListener('click', check);
  return el;
}

let gate = null;

async function check() {
  if (await reachable()) {
    if (gate) { gate.remove(); gate = null; }
    return;
  }
  if (!gate) {
    gate = build();
    document.body.appendChild(gate);
  }
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', check);
} else {
  check();
}
