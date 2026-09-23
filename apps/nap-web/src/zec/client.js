// The page's side of the Zcash wallet: a worker to do the work, and a place
// to keep what it has scanned.
//
// The scan state (note trees, the notes held, how far it got) is not a key,
// but it is a list of what this wallet received and when. It is kept in
// IndexedDB sealed under a key derived from the spending key, so it opens
// only while the wallet is unlocked — a locked device shows nothing about
// its ZEC, not even that there is some. Losing it costs a rescan; the money
// is on the chain.

const DB = 'nap-zec';
const STORE = 'state';
const te = new TextEncoder();

let worker = null;
let seq = 0;
const waiting = new Map();

function ensureWorker() {
  if (worker) return worker;
  worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
  worker.onmessage = ({ data }) => {
    const job = waiting.get(data.id);
    if (!job) return;
    if ('step' in data) { job.onStep?.(data.step); return; }
    waiting.delete(data.id);
    if (data.ok) job.resolve(data.result); else job.reject(new Error(data.error));
  };
  worker.onerror = (e) => {
    const err = new Error(e.message || 'The Zcash wallet stopped');
    for (const job of waiting.values()) job.reject(err);
    waiting.clear();
    worker = null;
  };
  return worker;
}

function ask(op, args = [], onStep) {
  const w = ensureWorker();
  const id = ++seq;
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject, onStep });
    w.postMessage({ id, op, args });
  });
}

/// Both networks' spending keys for a phrase. Runs in the worker, which is
/// where the derivation code is; the words go no further.
export async function deriveZecKeys(words, passphrase, account) {
  return ask('derive', [words, passphrase, account]);
}

/// Stop the worker and everything in it. The keys it held go with it.
export function stopZec() {
  if (!worker) return;
  worker.terminate();
  worker = null;
  for (const job of waiting.values()) job.reject(new Error('Locked'));
  waiting.clear();
}

// --- Sealed state ---------------------------------------------------------

function openDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function stateKey(sk, network) {
  const base = await crypto.subtle.importKey('raw', sk, 'HKDF', false, ['deriveKey', 'deriveBits']);
  const info = te.encode(`nap.zec.state.v1/${network}`);
  const name = new Uint8Array(await crypto.subtle.deriveBits(
    { name: 'HKDF', hash: 'SHA-256', salt: te.encode('nap.zec.state.id'), info }, base, 128));
  const key = await crypto.subtle.deriveKey(
    { name: 'HKDF', hash: 'SHA-256', salt: te.encode('nap.zec.state.key'), info },
    base, { name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt']);
  // The record's name is derived too, so the database does not say whose
  // wallet it is — only that this browser has held one.
  return { id: [...name].map((b) => b.toString(16).padStart(2, '0')).join(''), key };
}

async function loadState(sk, network) {
  try {
    const { id, key } = await stateKey(sk, network);
    const db = await openDb();
    const rec = await new Promise((resolve, reject) => {
      const r = db.transaction(STORE).objectStore(STORE).get(id);
      r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error);
    });
    db.close();
    if (!rec) return null;
    return new Uint8Array(await crypto.subtle.decrypt({ name: 'AES-GCM', iv: rec.iv }, key, rec.ct));
  } catch {
    return null; // private window, blocked storage, a damaged record: rescan
  }
}

async function saveState(sk, network, state) {
  try {
    const { id, key } = await stateKey(sk, network);
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, state));
    const db = await openDb();
    await new Promise((resolve, reject) => {
      const tx = db.transaction(STORE, 'readwrite');
      tx.objectStore(STORE).put({ iv, ct, at: Date.now() }, id);
      tx.oncomplete = resolve; tx.onerror = () => reject(tx.error);
    });
    db.close();
    return true;
  } catch {
    return false;
  }
}

// --- The account ------------------------------------------------------------

/// One network's Zcash wallet. `open` resumes or seeds; `sync` and `send`
/// report progress through `onStep` and keep the state as they go.
export class ZecAccount {
  constructor({ sk, network, url, birthday }) {
    this.sk = sk;
    this.network = network;
    this.url = url;
    this.birthday = birthday;
    this.view = null;
    this.saving = Promise.resolve();
  }

  keep(state) {
    if (!state) return;
    const copy = new Uint8Array(state);
    this.saving = this.saving.then(() => saveState(this.sk, this.network, copy));
  }

  /// The wallet from its keys and whatever state this device kept — no
  /// network. Enough for the address, which is all connecting to a site needs;
  /// a block server that is down must not stop anyone connecting.
  async peek() {
    await ask('endpoint', [this.url]);
    const state = await loadState(this.sk, this.network);
    this.view = await ask('open', [this.sk, this.network, state]);
    return this.view;
  }

  /// `fresh` is true for a wallet made today: nothing can have been paid to
  /// it before now, so it starts at the tip rather than scanning back.
  async open({ fresh = false } = {}) {
    await this.peek();
    if (!this.view.hasState) {
      this.view = await ask('seed', [fresh ? null : this.birthday]);
      this.keep(this.view.state);
    }
    return this.view;
  }

  async sync(onStep) {
    this.view = await ask('sync', [], (s) => { this.keep(s.state); onStep?.(s); });
    this.keep(this.view.state);
    return this.view;
  }

  async send(to, zatoshi, memo, onStep) {
    this.view = await ask('send', [to, zatoshi, memo], (s) => { if (s.state) this.keep(s.state); onStep?.(s); });
    this.keep(this.view.state);
    return this.view;
  }

  /// Start again from `height`: for a phrase whose ZEC predates the default.
  async rescanFrom(height) {
    this.view = await ask('seed', [height]);
    this.keep(this.view.state);
    return this.view;
  }
}
