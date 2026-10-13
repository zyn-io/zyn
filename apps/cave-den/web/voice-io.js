// The den's relayed voice, in the audio thread: the microphone out in 20 ms
// pieces (960 samples at 48 kHz) for the encoder, and each slot's decoded
// audio in, held 60 ms against jitter and mixed to the speakers.

const FRAME = 960;
const START = 48000 * 0.06;
const MOST = 48000 * 0.3;

class DenVoiceIo extends AudioWorkletProcessor {
  constructor() {
    super();
    this.up = new Float32Array(FRAME);
    this.n = 0;
    this.slots = new Map(); // slot → { chunks, off, len, playing }
    this.port.onmessage = (e) => {
      const { slot, pcm, reset } = e.data;
      if (reset) { this.slots.delete(slot); return; }
      let b = this.slots.get(slot);
      if (!b) this.slots.set(slot, (b = { chunks: [], off: 0, len: 0, playing: false }));
      b.chunks.push(pcm);
      b.len += pcm.length;
      // Too far behind: drop the oldest, so the delay stays short.
      while (b.len - b.off > MOST && b.chunks.length > 1) { b.len -= b.chunks.shift().length; b.off = 0; }
    };
  }

  process(inputs, outputs) {
    const mic = inputs[0]?.[0];
    if (mic) {
      for (let i = 0; i < mic.length; i++) {
        this.up[this.n++] = mic[i];
        if (this.n === FRAME) { this.port.postMessage(this.up.slice()); this.n = 0; }
      }
    }
    const out = outputs[0][0];
    out.fill(0);
    for (const b of this.slots.values()) {
      if (!b.playing) { if (b.len - b.off < START) continue; b.playing = true; }
      let i = 0;
      while (i < out.length && b.chunks.length) {
        const c = b.chunks[0];
        const take = Math.min(c.length - b.off, out.length - i);
        for (let k = 0; k < take; k++) out[i + k] += c[b.off + k];
        i += take;
        b.off += take;
        if (b.off >= c.length) { b.len -= c.length; b.chunks.shift(); b.off = 0; }
      }
      // Ran dry: wait for 60 ms again before playing on.
      if (!b.chunks.length) b.playing = false;
    }
    return true;
  }
}

registerProcessor('den-voice-io', DenVoiceIo);
