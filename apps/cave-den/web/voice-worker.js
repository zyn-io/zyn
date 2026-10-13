// The den's voice frames, sealed and opened off the main thread: AES-GCM under
// the sender's own voice key for the epoch (the MLS exporter, labelled with the
// sender's key), so a slot opens only under the key of the member the slot map
// names. Frame: [ciphertext ‖ tag][iv 12][counter 8][key index 1]; the last
// nine bytes are authenticated, and a counter already seen is refused. The
// counter is the send time in microseconds (kept rising), so a listener also
// refuses a frame more than a minute old: the den cannot replay what a member
// said earlier to someone who joins voice later.
//
// Streams arrive by transfer (Chrome) or through RTCRtpScriptTransform
// (Safari, Firefox).

const TAIL = 21;
const TAG = 16;
const WINDOW = 64n;

let keys = new Map(); // key index → Map(sender key hex → CryptoKey)
let current = -1;
let me = '';
let slots = new Map(); // slot → sender key hex
const seen = new Map(); // sender key hex → { max, mask }
let counter = Date.now() * 1000; // the send time in µs, and above any counter this device sent before
const STALE_MS = 60_000;
let sealed = 0;
let opened = 0;
let failed = 0;

onmessage = (e) => {
  const d = e.data;
  if (d.keys) {
    keys = new Map(d.keys.map(([i, ks]) => [i, new Map(ks)]));
    current = d.current;
    me = d.me;
  }
  if (d.slots) slots = new Map(d.slots);
  if (d.readable) pipe(d.readable, d.writable, d.mode, d.slot);
};

onrtctransform = (e) => pipe(e.transformer.readable, e.transformer.writable, e.transformer.options.mode, e.transformer.options.slot);

setInterval(() => postMessage({ count: true, sealed, opened, failed }), 2000);

function pipe(readable, writable, mode, slot) {
  const step = mode === 'seal' ? seal : (f) => open(f, slot);
  readable
    .pipeThrough(new TransformStream({ transform: async (f, c) => { try { c.enqueue(await step(f)); } catch { failed++; } } }))
    .pipeTo(writable);
}

async function seal(frame) {
  const key = keys.get(current)?.get(me);
  if (!key) throw new Error('no key');
  counter = Math.max(counter + 1, Date.now() * 1000);
  const head = new Uint8Array(9);
  const v = new DataView(head.buffer);
  v.setUint32(0, Math.floor(counter / 2 ** 32));
  v.setUint32(4, counter >>> 0);
  head[8] = current;
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData: head }, key, frame.data));
  const out = new Uint8Array(ct.length + TAIL);
  out.set(ct);
  out.set(iv, ct.length);
  out.set(head, ct.length + 12);
  frame.data = out.buffer;
  sealed++;
  return frame;
}

async function open(frame, slot) {
  const d = new Uint8Array(frame.data);
  if (d.length < TAG + TAIL) throw new Error('short');
  const sender = slots.get(slot);
  const head = d.slice(d.length - 9);
  const key = sender && keys.get(head[8])?.get(sender);
  if (!key) throw new Error('no key');
  const v = new DataView(head.buffer);
  const n = BigInt(v.getUint32(0)) * 2n ** 32n + BigInt(v.getUint32(4));
  const iv = d.subarray(d.length - TAIL, d.length - 9);
  const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv, additionalData: head }, key, d.subarray(0, d.length - TAIL));
  if (Math.abs(Number(n / 1000n) - Date.now()) > STALE_MS) throw new Error('stale');
  if (!fresh(sender, n)) throw new Error('replay');
  frame.data = plain;
  opened++;
  return frame;
}

/// A sliding window of 64 frames per sender: newer counters move it on, a
/// late frame inside it is taken once, anything older is refused.
function fresh(sender, n) {
  const s = seen.get(sender);
  if (!s) { seen.set(sender, { max: n, mask: 1n }); return true; }
  if (n > s.max) {
    const shift = n - s.max;
    s.mask = shift >= WINDOW ? 1n : ((s.mask << shift) | 1n) & ((1n << WINDOW) - 1n);
    s.max = n;
    return true;
  }
  const back = s.max - n;
  if (back >= WINDOW) return false;
  const bit = 1n << back;
  if (s.mask & bit) return false;
  s.mask |= bit;
  return true;
}
