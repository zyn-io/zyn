export const STALE_AFTER_MS = 120_000;

const HASH_PATTERN = /^[0-9a-f]{64}$/i;

export function normalizeStatus(payload, now = Date.now()) {
  if (!payload || typeof payload !== 'object') {
    throw new TypeError('status response is not an object');
  }

  const network = String(payload.network ?? payload.chain ?? '');
  const height = Number(payload.height ?? payload.blocks);
  const estimatedHeight = Number(payload.estimatedHeight ?? payload.estimatedheight);
  const tip = String(payload.tip ?? payload.bestblockhash ?? '').toLowerCase();
  const sampledAt = new Date(payload.sampledAt ?? payload.sampled_at ?? 0);
  const latencyMs = Number(payload.latencyMs ?? payload.latency_ms ?? 0);

  if (network !== 'main') throw new TypeError('node is not on Zcash mainnet');
  if (!Number.isSafeInteger(height) || height < 1) throw new TypeError('invalid node height');
  if (!Number.isSafeInteger(estimatedHeight) || estimatedHeight < height) {
    throw new TypeError('invalid estimated height');
  }
  if (!HASH_PATTERN.test(tip)) throw new TypeError('invalid best block hash');
  if (!Number.isFinite(sampledAt.getTime())) throw new TypeError('invalid sample time');
  if (!Number.isFinite(latencyMs) || latencyMs < 0) throw new TypeError('invalid latency');

  const ageMs = Math.max(0, now - sampledAt.getTime());
  const gap = estimatedHeight - height;
  const stale = ageMs > STALE_AFTER_MS;
  const state = stale ? 'stale' : gap <= 3 ? 'online' : 'syncing';

  return {
    state,
    network,
    height,
    estimatedHeight,
    gap,
    tip,
    sampledAt,
    latencyMs: Math.round(latencyMs),
    stale,
  };
}

export function formatHeight(height) {
  return new Intl.NumberFormat('en-US').format(height);
}

export function shortHash(hash) {
  return `${hash.slice(0, 16)}…${hash.slice(-16)}`;
}

export function relativeSampleTime(sampledAt, now = Date.now()) {
  const seconds = Math.max(0, Math.round((now - sampledAt.getTime()) / 1000));
  if (seconds < 5) return 'just now';
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ago`;
}
