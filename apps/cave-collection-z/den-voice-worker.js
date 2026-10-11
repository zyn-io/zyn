// The den's voice frames, sealed and opened off the main thread where the
// browser offers RTCRtpScriptTransform (Safari, Firefox): AES-GCM under the
// room's voice key, [ciphertext ‖ tag][iv 12][key index 1].

let keys = new Map();
let current = 0;
let sealed = 0;
let opened = 0;

onmessage = (e) => {
  if (e.data.keys) {
    keys = new Map(e.data.keys);
    current = e.data.current;
  }
};

async function seal(frame) {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, keys.get(current), frame.data));
  const out = new Uint8Array(ct.length + 13);
  out.set(ct); out.set(iv, ct.length); out[ct.length + 12] = current;
  frame.data = out.buffer;
  sealed++;
  return frame;
}

async function open(frame) {
  const d = new Uint8Array(frame.data);
  const key = keys.get(d[d.length - 1]);
  frame.data = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: d.subarray(d.length - 13, d.length - 1) }, key, d.subarray(0, d.length - 13));
  opened++;
  return frame;
}

onrtctransform = (e) => {
  const step = e.transformer.options.mode === 'seal' ? seal : open;
  e.transformer.readable
    .pipeThrough(new TransformStream({ transform: async (f, c) => { try { c.enqueue(await step(f)); } catch { /* a frame from before our key */ } } }))
    .pipeTo(e.transformer.writable);
  setInterval(() => postMessage({ count: true, sealed, opened }), 2000);
};
