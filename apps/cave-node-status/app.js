import { formatHeight, normalizeStatus, relativeSampleTime, shortHash } from './status.js';

const root = document.documentElement;
const statusCopy = document.querySelector('[data-status-copy]');
const stateLabel = document.querySelector('[data-state-label]');
const heightValue = document.querySelector('[data-height]');
const estimatedValue = document.querySelector('[data-estimated]');
const gapValue = document.querySelector('[data-gap]');
const observedValue = document.querySelector('[data-observed]');
const latencyValue = document.querySelector('[data-latency]');
const tipValue = document.querySelector('[data-tip]');
const tipFull = document.querySelector('[data-tip-full]');
const refreshValue = document.querySelector('[data-refresh-label]');
const refreshTrack = document.querySelector('[data-refresh-track]');
const copyButton = document.querySelector('[data-copy]');
const previewFlag = document.querySelector('[data-preview]');

const REFRESH_MS = 15_000;
const IS_LOCAL = ['127.0.0.1', 'localhost'].includes(location.hostname);
let currentTip = '';
let refreshStartedAt = performance.now();
let requestInFlight = false;

function setState(state, message) {
  root.dataset.nodeState = state;
  statusCopy.textContent = message;
  stateLabel.textContent = {
    online: 'Online',
    syncing: 'Catching up',
    stale: 'Signal stale',
    offline: 'No signal',
    loading: 'Listening',
  }[state];
}

function render(status) {
  setState(
    status.state,
    status.state === 'online'
      ? 'Zebra is validating Zcash mainnet.'
      : status.state === 'syncing'
        ? 'Zebra is online and closing the distance.'
        : 'The last valid signal is older than two minutes.',
  );
  heightValue.textContent = formatHeight(status.height);
  estimatedValue.textContent = formatHeight(status.estimatedHeight);
  gapValue.textContent = String(status.gap).padStart(2, '0');
  observedValue.textContent = relativeSampleTime(status.sampledAt);
  latencyValue.textContent = `${status.latencyMs} ms`;
  tipValue.textContent = shortHash(status.tip);
  tipFull.textContent = status.tip;
  currentTip = status.tip;
}

function localPreview() {
  previewFlag.hidden = false;
  return {
    network: 'main',
    height: 3_487_459,
    estimatedHeight: 3_487_461,
    tip: '000000000039a1c000c527d13c04f107f753af0c7cd097724549d419e0082d43',
    sampledAt: new Date().toISOString(),
    latencyMs: 28,
  };
}

async function fetchStatus() {
  if (requestInFlight) return;
  requestInFlight = true;
  root.classList.add('is-refreshing');
  try {
    let payload;
    if (IS_LOCAL) {
      await new Promise((resolve) => setTimeout(resolve, 420));
      payload = localPreview();
    } else {
      const controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), 5_000);
      const response = await fetch('/api/status', {
        cache: 'no-store',
        headers: { Accept: 'application/json' },
        signal: controller.signal,
      });
      clearTimeout(timeout);
      if (!response.ok) throw new Error(`status endpoint returned ${response.status}`);
      payload = await response.json();
    }
    render(normalizeStatus(payload));
  } catch {
    setState('offline', 'The public signal cannot reach the private node.');
    observedValue.textContent = 'unavailable';
    latencyValue.textContent = '—';
  } finally {
    requestInFlight = false;
    root.classList.remove('is-refreshing');
    refreshStartedAt = performance.now();
  }
}

copyButton.addEventListener('click', async () => {
  if (!currentTip) return;
  try {
    await navigator.clipboard.writeText(currentTip);
    copyButton.textContent = 'Copied';
    setTimeout(() => { copyButton.textContent = 'Copy hash'; }, 1_500);
  } catch {
    copyButton.textContent = 'Copy failed';
  }
});

function animateRefresh(now) {
  const elapsed = Math.min(REFRESH_MS, now - refreshStartedAt);
  const remaining = Math.max(0, Math.ceil((REFRESH_MS - elapsed) / 1_000));
  const progress = elapsed / REFRESH_MS;
  refreshTrack.style.transform = `scaleX(${progress})`;
  refreshValue.textContent = `next sample / ${String(remaining).padStart(2, '0')}s`;
  requestAnimationFrame(animateRefresh);
}

setState('loading', 'Opening a one-way signal into the cave.');
fetchStatus();
setInterval(fetchStatus, REFRESH_MS);
requestAnimationFrame(animateRefresh);

window.__caveNode = { refresh: fetchStatus, endpoint: '/api/status' };
