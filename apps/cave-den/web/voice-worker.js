// The den's voice and screen frames, sealed and opened off the main thread:
// AES-GCM under the sender's own key for the epoch (the MLS exporter,
// labelled 'den voice' or 'den screen' with the sender's key), so a slot
// opens only under the key of the member the slot map names, and the stage
// only under the presenter's. The layout is media-seal.js's; its last nine
// bytes are authenticated, and a counter already seen is refused. The
// counter is the send time in microseconds (kept rising), so a listener also
// refuses a frame more than a minute old (a screen's, 15 s): the den cannot
// replay what a member said or showed earlier to someone who joins later.
//
// Streams arrive by transfer (Chrome) or through RTCRtpScriptTransform
// (Safari, Firefox); relayed frames, one by one as messages.

import { clearLength, sealFrame, openFrame, indexOf, tooOld, Window } from './media-seal.js';

let keys = new Map(); // key index → Map(sender key hex → CryptoKey), 'den voice'
let screenKeys = new Map(); // the same, 'den screen'
let current = -1;
let me = '';
let presenter = ''; // whose screen the stage carries, as the den names them
let slots = new Map(); // slot → sender key hex
const voiceSeen = new Window();
const screenSeen = new Window();
let counter = Date.now() * 1000; // the send time in µs, and above any counter this device sent before
let sealIndex = null; // the epoch this device seals under: the one before, for a moment after a change
let sealed = 0;
let opened = 0;
let failed = 0;
// The screen's own counts, besides the totals.
let sealedScreen = 0;
let openedScreen = 0;
let failedScreen = 0;

onmessage = (e) => {
  const d = e.data;
  // Relayed voice (frames over the den's channel, not WebRTC): the same
  // seal and open, by message.
  if (d.op) {
    const step = d.op === 'seal' ? seal({ data: d.data }) : open({ data: d.data }, d.slot);
    step.then((f) => postMessage({ id: d.id, data: f.data }, [f.data]), () => { failed++; postMessage({ id: d.id }); });
    return;
  }
  if (d.keys) {
    keys = new Map(d.keys.map(([i, ks]) => [i, new Map(ks)]));
    if (d.screenKeys) screenKeys = new Map(d.screenKeys.map(([i, ks]) => [i, new Map(ks)]));
    current = d.current;
    sealIndex = Number.isInteger(d.sealIndex) ? d.sealIndex : null;
    me = d.me;
  }
  if (typeof d.presenter === 'string') presenter = d.presenter;
  if (d.slots) slots = new Map(d.slots);
  if (d.readable) pipe(d.readable, d.writable, d.mode, d.slot);
};

onrtctransform = (e) => pipe(e.transformer.readable, e.transformer.writable, e.transformer.options.mode, e.transformer.options.slot);

setInterval(() => postMessage({ count: true, sealed, opened, failed, sealedScreen, openedScreen, failedScreen }), 2000);

// Modes: seal and open (voice), seal-screen and open-screen (the stage).
function pipe(readable, writable, mode, slot) {
  const step = mode === 'seal' ? (f) => seal(f)
    : mode === 'seal-screen' ? (f) => seal(f, 'screen')
      : mode === 'open-screen' ? (f) => open(f, null, 'screen')
        : (f) => open(f, slot);
  const screen = mode === 'open-screen' || mode === 'seal-screen';
  readable
    .pipeThrough(new TransformStream({ transform: async (f, c) => { try { c.enqueue(await step(f)); } catch { failed++; if (screen) failedScreen++; } } }))
    .pipeTo(writable);
}

async function seal(frame, kind = 'audio') {
  const index = sealIndex ?? current;
  const key = (kind === 'screen' ? screenKeys : keys).get(index)?.get(me);
  if (!key) throw new Error('no key');
  counter = Math.max(counter + 1, Date.now() * 1000);
  const data = new Uint8Array(frame.data);
  frame.data = (await sealFrame(key, index, BigInt(counter), data, clearLength(data, kind))).buffer;
  sealed++;
  if (kind === 'screen') sealedScreen++;
  return frame;
}

// A frame opens only under the key of the member the den says it is from:
// a slot's speaker, or the presenter on the stage.
async function open(frame, slot, kind = 'audio') {
  const d = new Uint8Array(frame.data);
  const sender = kind === 'screen' ? presenter : slots.get(slot);
  const key = sender && (kind === 'screen' ? screenKeys : keys).get(indexOf(d))?.get(sender);
  if (!key) throw new Error('no key');
  const { plain, counter: n } = await openFrame(key, d, clearLength(d, kind));
  if (tooOld(n, Date.now(), kind)) throw new Error('stale');
  if (!(kind === 'screen' ? screenSeen : voiceSeen).fresh(sender, n)) throw new Error('replay');
  frame.data = plain.buffer;
  opened++;
  if (kind === 'screen') openedScreen++;
  return frame;
}
