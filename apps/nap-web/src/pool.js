// The private bridge channel in Nap (DECISIONS §158).
//
// A note is two secrets derived from this wallet's key and an index — so any
// Nap holding the same phrase can find its notes again — whose commitment
// sits in a pool on the other chain. Spending one proves "one of the pool's
// notes is mine" without saying which, bound to what the spend is for: a Zyn
// deposit code (in), or an address, a relayer and its fee (out). The relayer
// submits spends, so no wallet of the holder's pays for one.
//
// Only commitments and states are stored; secrets are derived when needed.

import { sha512 } from '@noble/hashes/sha2.js';
import { hex, unhex } from './core.js';
import * as ledger from './ledger.js';
import * as bridges from './bridges.js';
import { b58decode } from './solana.js';

const DOMAIN = new TextEncoder().encode('zyn-pool.note.v1');
const LAMPORT = 1_000_000_000n; // one lamport in WAD
/// The pools' sizes, in lamports: 0.1, 1 and 10 SOL.
export const SIZES = [100_000_000, 1_000_000_000, 10_000_000_000];

const read = (k) => { try { return localStorage.getItem(k); } catch { return null; } };
const write = (k, v) => { try { localStorage.setItem(k, v); } catch { /* this session only */ } };
const u32be = (n) => { const b = new Uint8Array(4); new DataView(b.buffer).setUint32(0, n); return b; };

// ------------------------------------------------------------------ core

let ready = null;
/// The WASM core with the proving key loaded, once.
function core() {
  if (!ready) {
    ready = (async () => {
      const m = await import('./pool-pkg/nap_pool.js');
      const inNode = typeof window === 'undefined';
      if (inNode) {
        const { readFileSync } = await import('node:fs');
        await m.default({ module_or_path: readFileSync(new URL('./pool-pkg/nap_pool_bg.wasm', import.meta.url)) });
        m.load_key(readFileSync(new URL('../public/pool/spend.pk.raw', import.meta.url)));
      } else {
        await m.default();
        const r = await fetch('/pool/spend.pk.raw');
        if (!r.ok) throw new Error('The private channel could not load its proving key');
        m.load_key(new Uint8Array(await r.arrayBuffer()));
      }
      return m;
    })().catch((e) => { ready = null; throw e; });
  }
  return ready;
}

const randomHex = () => hex(crypto.getRandomValues(new Uint8Array(32)));

// ------------------------------------------------------------------ notes

const key = (config, identity) => `nap.pool.v1.${config.chain}.${identity.accountHex}`;

function book(config, identity) {
  try { return JSON.parse(read(key(config, identity)) || '') || { next: 0, notes: [] }; } catch { return { next: 0, notes: [] }; }
}
const save = (config, identity, b) => write(key(config, identity), JSON.stringify(b));

async function secrets(config, identity, i) {
  const seed = sha512(new Uint8Array([...DOMAIN, ...identity.secretKey, ...u32be(config.chain >>> 0), ...u32be(i)]));
  return JSON.parse((await core()).note(hex(seed)));
}

/// A fresh note of `size` lamports for this wallet: `in` (to deposit into a
/// pool) or `out` (a private exit's destination).
export async function newNote(config, identity, size, kind) {
  if (!SIZES.includes(Number(size))) throw new Error('Pools take 0.1, 1 or 10 SOL');
  const b = book(config, identity);
  const i = b.next;
  const n = await secrets(config, identity, i);
  b.next = i + 1;
  b.notes.push({ i, size: Number(size), kind, commitment: n.commitment, state: 'new', at: Date.now() });
  save(config, identity, b);
  return { i, size: Number(size), commitment: n.commitment };
}

function mark(config, identity, commitment, state, extra = {}) {
  const b = book(config, identity);
  const n = b.notes.find((x) => x.commitment === commitment);
  if (n) Object.assign(n, { state }, extra);
  save(config, identity, b);
}

export const info = (config) => ledger.call(config, 'zyn_poolInfo');
const pools = async (config, size) => ledger.call(config, 'zyn_poolLeaves', { size: String(size) });
const leaves = async (config, size) => (await pools(config, size)).leaves;
const SPENT = new Set(['to zyn', 'sent', 'spent']);
/// How far past the last note found a restored wallet keeps looking.
const GAP = 20;

/// This wallet's notes, each with where it stands: `new` or `burned` (not in
/// its pool yet), `in pool` (unspent: into Zyn or to an address), or spent.
/// Found again from the phrase alone: every index is tried against every
/// pool's notes, and a note whose nullifier hash a spend published is spent.
export async function notes(config, identity) {
  const b = book(config, identity);
  const seen = new Map();
  for (const size of SIZES) {
    const r = await pools(config, size).catch(() => ({ leaves: [], spent: [] }));
    seen.set(size, { leaves: new Set(r.leaves), spent: new Set(r.spent || []) });
  }
  let last = b.next - 1;
  for (let i = 0; i <= Math.max(b.next, last + 1) + GAP; i++) {
    const n = await secrets(config, identity, i);
    for (const size of SIZES) {
      const p = seen.get(size);
      if (!p.leaves.has(n.commitment)) continue;
      last = Math.max(last, i);
      let note = b.notes.find((x) => x.commitment === n.commitment);
      if (!note) {
        note = { i, size, kind: 'note', commitment: n.commitment, state: 'in pool', at: null };
        b.notes.push(note);
      }
      if (p.spent.has(n.nullifier_hash)) {
        if (!SPENT.has(note.state)) note.state = 'spent';
      } else note.state = 'in pool';
    }
  }
  b.next = Math.max(b.next, last + 1);
  b.notes.sort((x, y) => x.i - y.i);
  save(config, identity, b);
  return b.notes.map(({ i, size, kind, commitment, state, at, signature }) => ({ i, size, kind, commitment, state, at, signature }));
}

async function proveFor(config, identity, note, binding) {
  const ls = await leaves(config, note.size);
  if (!ls.includes(note.commitment)) throw new Error('That note is not in its pool yet; try again once its deposit or exit is confirmed');
  const s = await secrets(config, identity, note.i);
  const m = await core();
  return { m, p: JSON.parse(m.prove(JSON.stringify(ls), s.nullifier, s.secret, binding, randomHex())), count: ls.length };
}

/// A note of this wallet's that can still be spent — whichever way: a note
/// deposited on Solana can go to an address, and an exit note into Zyn.
function own(config, identity, commitment) {
  const n = book(config, identity).notes.find((x) => x.commitment === commitment);
  if (!n) throw new Error('No such note in this wallet');
  if (SPENT.has(n.state)) throw new Error('That note is already spent');
  return n;
}

/// A deposited note, into Zyn: a proof bound to this wallet's deposit code,
/// submitted by the relayer. Arrives as SOL.zy of the note's size.
export async function toZyn(config, identity, commitment) {
  const note = own(config, identity, commitment);
  const code = await bridges.depositCode(config, identity);
  const m = await core();
  const { p } = await proveFor(config, identity, note, m.binding_to_zyn(code));
  const r = await ledger.call(config, 'zyn_poolRelay', { size: String(note.size), kind: 'to_zyn', proof: p.proof, root: p.root, nullifier_hash: p.nullifier_hash, code });
  mark(config, identity, commitment, 'to zyn', { signature: r.signature });
  return r;
}

/// SOL.zy out privately: burn one size on Zyn with a fresh note as the
/// destination; the bridge puts the note into the pool.
export async function withdraw(config, identity, held, size, bps) {
  const b = bridges.bridgesOf(config).find((x) => x.key === 'sol');
  if (!b) throw new Error('No SOL bridge on this network');
  const note = await newNote(config, identity, size, 'out');
  const address = new Uint8Array(48);
  address.set(new TextEncoder().encode('zyn-pool'), 0);
  new DataView(address.buffer).setBigUint64(8, BigInt(size), true);
  address.set(unhex(note.commitment, 32), 16);
  await bridges.bridgeOutTo(config, identity, held, b, address, BigInt(size) * LAMPORT, bps);
  mark(config, identity, note.commitment, 'burned');
  return note;
}

/// An exit note, to any Solana address, through the relayer (which takes its
/// fee from the note).
export async function sendOut(config, identity, commitment, to) {
  const note = own(config, identity, commitment);
  const recipient = b58decode(String(to).trim());
  if (recipient.length !== 32) throw new Error('That is not a Solana address');
  const i = await info(config);
  const m = await core();
  const binding = m.binding_to_address(hex(recipient), hex(b58decode(i.relayer)), BigInt(i.fee));
  const { p } = await proveFor(config, identity, note, binding);
  const r = await ledger.call(config, 'zyn_poolRelay', { size: String(note.size), kind: 'to_address', proof: p.proof, root: p.root, nullifier_hash: p.nullifier_hash, recipient: String(to).trim(), fee: String(i.fee) });
  mark(config, identity, commitment, 'sent', { signature: r.signature });
  return { ...r, fee: i.fee };
}
