// One media frame sealed for the den to forward and not read: AES-GCM under
// the sender's key, with the counter and key index after it. Screen frames
// keep the start of the VP8 frame in the clear, which WebRTC needs to
// packetise them: 10 bytes on a keyframe, 3 otherwise. Those bytes are part
// of what the tag covers, so they cannot be changed either.
//
// Frame: [clear prefix][ciphertext ‖ tag][iv 12][counter 8][key index 1].
// Voice frames have no clear prefix: the layout voice always had.
export const TAIL = 21;
export const TAG = 16;

/// How many bytes at the start of a frame stay in the clear.
export function clearLength(bytes, kind) {
  if (kind !== 'screen') return 0;
  const n = (bytes[0] & 1) === 0 ? 10 : 3;
  if (bytes.length < n) throw new Error('short');
  return n;
}

export async function sealFrame(key, index, counter, data, clear) {
  const head = new Uint8Array(9);
  const v = new DataView(head.buffer);
  v.setUint32(0, Number(counter >> 32n));
  v.setUint32(4, Number(counter & 0xffffffffn));
  head[8] = index;
  const prefix = data.subarray(0, clear);
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData: join(prefix, head) }, key, data.subarray(clear)));
  const out = new Uint8Array(clear + ct.length + TAIL);
  out.set(prefix);
  out.set(ct, clear);
  out.set(iv, clear + ct.length);
  out.set(head, clear + ct.length + 12);
  return out;
}

export async function openFrame(key, d, clear) {
  if (d.length < clear + TAG + TAIL) throw new Error('short');
  const head = d.slice(d.length - 9);
  const v = new DataView(head.buffer);
  const counter = BigInt(v.getUint32(0)) * 2n ** 32n + BigInt(v.getUint32(4));
  const iv = d.subarray(d.length - TAIL, d.length - 9);
  const prefix = d.subarray(0, clear);
  const body = new Uint8Array(await crypto.subtle.decrypt({ name: 'AES-GCM', iv, additionalData: join(prefix, head) }, key, d.subarray(clear, d.length - TAIL)));
  return { plain: join(prefix, body), counter, index: head[8] };
}

/// The key index of a sealed frame, read before choosing the key.
export const indexOf = (d) => d[d.length - 1];

function join(a, b) {
  const out = new Uint8Array(a.length + b.length);
  out.set(a);
  out.set(b, a.length);
  return out;
}

/// A sliding window of 64 frames per sender: newer counters move it on, a
/// late frame inside it is taken once, anything older is refused.
export class Window {
  constructor(size = 64n) { this.size = size; this.seen = new Map(); }

  fresh(sender, n) {
    const s = this.seen.get(sender);
    if (!s) { this.seen.set(sender, { max: n, mask: 1n }); return true; }
    if (n > s.max) {
      const shift = n - s.max;
      s.mask = shift >= this.size ? 1n : ((s.mask << shift) | 1n) & ((1n << this.size) - 1n);
      s.max = n;
      return true;
    }
    const back = s.max - n;
    if (back >= this.size) return false;
    const bit = 1n << back;
    if (s.mask & bit) return false;
    s.mask |= bit;
    return true;
  }
}
