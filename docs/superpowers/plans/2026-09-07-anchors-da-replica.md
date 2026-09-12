# Anchors on Zcash, DA off the box, verifying replica — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the exit hatch real: anchors actually reach Zcash, every applied intent is journaled so a third party can replay, each anchored epoch is published as a self-verifying bundle to mirrors, and a `zyn-replica` process verifies the chain from Zcash + mirrors alone and serves read-only RPC.

**Architecture:** (A) `zyn::journal` records intents before the VM applies them and can replay them; the anchor `Ledger` is persisted. (B) `zynzapd::anchor` turns an `Anchor` into a 10,000-zat Orchard self-send carrying a `ZYA` memo through the existing payout path, and the node accepts the anchor only when Zebra confirms the transaction. (C) `zynzapd::publish` writes one immutable bundle per accepted anchor and pushes it to HTTP mirrors; a tiny static HTTP server (`zyn-mirror`) serves/accepts bundles. (D) `zynzapd::replica` + bin `zyn-replica` scans Zcash for `ZYA` self-sends, fetches bundles, replays, verifies, persists, and serves read-only RPC. (E) `zyn-cli` gains `record` / `exit-proof` / `verify-exit`; Nap Wallet keeps its record+path per anchor.

**Tech Stack:** Rust workspace (`zyn`, `zyn-vm`, `swapvm`, `zyn-custody`, `zynzapd`), `ureq` (HTTP client, already a dep), `orchard`/`zcash_*` (already deps), `frost` via `zyn-custody::signing` (already). No new crates.

**Spec:** `docs/superpowers/specs/2026-09-07-anchors-da-replica-design.md`

## Global Constraints

- Not a git repository: there are no commit steps. Each task ends with `cargo test -p <crate>` green and, at group boundaries, `cargo test --workspace`.
- `zyn` stays `no_std`-capable except modules gated by `#[cfg(feature = "std")]` (`store`, and the new `journal`).
- `zyn-custody` must not depend on `zyn` (it depends on `zyn-vm` only). The anchor memo tag is duplicated as a constant in `zyn_custody::memo` with a cross-crate equality test in `zynzapd`.
- Anchor memo: `"ZYA" ‖ 0x01 ‖ chain_id u32 BE ‖ epoch u64 BE ‖ anchor_id [32]` = 48 bytes, zero-padded to 512 in the note.
- Anchor transaction value: `10_000` zatoshi (`payout::zip317_fee(2, 0)`), self-send to `fvk.address_at(0, Scope::External)`.
- Bundle layout (relative to the DA root): `chain-<id>/<epoch>/anchor.bin`, `certificate.bin`, `published.bin`, `intents/epoch-<M>.intents`, and `chain-<id>/index` (append-only text: `epoch root_hex anchor_id_hex txid height`).
- A verifier never guesses: on divergence, fork, or lineage break the replica stops advancing and keeps serving its last verified root.
- Existing wire ops keep their layouts; new fields are appended (the pattern used for `clearing` and pool `reference`). `OP_SNAPSHOT` paging fields are optional trailing request bytes.

---

## File Structure

| File | Responsibility |
|---|---|
| `zyn/src/journal.rs` (new, std) | Append-only per-epoch intent files; reader; `replay` |
| `zyn/src/node.rs` (modify) | `Recorder` trait + hook; `propose_anchor` / `confirm_anchor`; `set_published`; `resume_with_ledger` |
| `zyn/src/anchor.rs` (modify) | `MEMO_MAGIC = b"ZYA"`; `Anchor::decode`; `Certificate::encode/decode`; `Ledger` keeps certificates; `Ledger::restore` |
| `zyn/src/store.rs` (modify) | `save_ledger` / `load_ledger` |
| `zyn/src/da.rs` (modify) | `Published::encode/decode` |
| `zyn-custody/src/memo.rs` (modify) | `ANCHOR_TAG`, `is_anchor` |
| `zyn-custody/src/shielded.rs` (modify) | `ScannedAction.anchor`; classify skips anchors; `Scanner::anchors` |
| `zynzapd/src/anchor.rs` (new) | `AnchorSettler`: propose → self-send → confirm |
| `zynzapd/src/publish.rs` (new) | `Bundle`, local write, mirror push, minimal HTTP file server |
| `zynzapd/src/replica.rs` (new) | `Replica`: sightings → fetch → replay → verify → persist |
| `zynzapd/src/bin/zyn-replica.rs` (new) | Process wrapper |
| `zynzapd/src/bin/zyn-mirror.rs` (new) | Static bundle server with token-gated PUT |
| `zynzapd/src/rpc.rs` (modify) | `read_only`; `OP_ACCOUNT_PROOF` from published; paged `OP_SNAPSHOT`; status appends |
| `zynzapd/src/config.rs`, `main.rs` (modify) | Journal, ledger, anchor settler, publisher wiring; env vars |
| `zynzapd/src/bin/zyn-cli.rs`, `zynzapd/src/app.rs` (modify) | Holder tooling; wallet record+path retention |
| `zynzapd/tests/replica.rs` (new) | End-to-end in-process: sequencer → bundles → replica |
| `RUNBOOK.md`, `DECISIONS.md` (modify) | §4s "Verifying the chain"; §7c status |

---

## Group A — Journal and persisted ledger

### Task 1: `Anchor::decode`, `Certificate::encode/decode`, `Ledger` keeps certificates

**Files:**
- Modify: `zyn/src/anchor.rs`

**Interfaces:**
- Produces: `Anchor::decode(bytes: &[u8]) -> Option<Anchor>` (exact inverse of `encode`, 148 bytes, refuses wrong domain/length); `Certificate::encode(&self) -> Vec<u8>` (`anchor[32] ‖ n u32 ‖ (signer[32] ‖ sig[64])*`), `Certificate::decode(&[u8]) -> Option<Certificate>`; `Ledger::certificates(&self) -> &[Certificate]` parallel to `anchors()`; `Ledger::restore(chain_id: u32, entries: Vec<(Anchor, Certificate)>) -> Result<Ledger, LineageError>` (re-runs `check` per entry; does **not** re-verify signatures — the set may not be configured at load); `accept_trusted_operator` pushes `Certificate::new(a.id())`.

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `anchor.rs`)

```rust
#[test]
fn an_anchor_round_trips_through_its_bytes() {
    let a = anchor(3, 300, 1, 2, 50);
    assert_eq!(Anchor::decode(&a.encode()), Some(a));
    assert_eq!(Anchor::decode(&a.encode()[..147]), None, "truncated");
    let mut junk = a.encode(); junk[0] ^= 1;
    assert_eq!(Anchor::decode(&junk), None, "wrong domain");
}

#[test]
fn a_certificate_round_trips_and_keeps_signer_order() {
    let a = anchor(1, 100, 0, 1, 10);
    let set = SignerSet::new(signers(3), 2).unwrap();
    let c = certify(&a, &set, 2);
    assert_eq!(Certificate::decode(&c.encode()), Some(c.clone()));
    assert!(Certificate::decode(&c.encode()[..40]).is_none());
}

#[test]
fn a_ledger_restores_from_its_entries_and_refuses_a_broken_lineage() {
    let mut l = Ledger::new(1);
    let a1 = anchor(1, 100, 0, 1, 10);
    let a2 = anchor(2, 200, 1, 2, 10);
    l.accept_trusted_operator(a1).unwrap();
    l.accept_trusted_operator(a2).unwrap();
    let entries: Vec<(Anchor, Certificate)> =
        l.anchors().iter().cloned().zip(l.certificates().iter().cloned()).collect();
    let r = Ledger::restore(1, entries.clone()).unwrap();
    assert_eq!(r.head_root(), l.head_root());
    let mut broken = entries; broken.swap(0, 1);
    assert!(Ledger::restore(1, broken).is_err());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p zyn anchor -- round_trips restores`
Expected: compile errors (`decode`, `certificates`, `restore` missing).

- [ ] **Step 3: Implement**

```rust
impl Anchor {
    pub const ENCODED_LEN: usize = 8 + 4 + 8 + 32 + 32 + 32 + 8 + 8 + 16 + 32 + 8 + 8; // = 196? -> compute from encode(); assert in a test
    pub fn decode(b: &[u8]) -> Option<Anchor> {
        let mut d = zyn_vm::read::Decoder::new(b);
        if d.take_bytes(ANCHOR_DOMAIN.len()).ok()? != ANCHOR_DOMAIN { return None; }
        let checkpoint = Checkpoint {
            chain_id: d.u32().ok()?, epoch: d.u64().ok()?,
            parent_root: d.hash().ok()?, state_root: d.hash().ok()?, intent_root: d.hash().ok()?,
            seq: d.u64().ok()?, intents: d.u64().ok()?, gross_volume: d.fixed().ok()?,
        };
        let previous_root = d.hash().ok()?;
        let epochs = d.u64().ok()?; let actions = d.u64().ok()?;
        if d.remaining() != 0 { return None; }
        Some(Anchor { checkpoint, previous_root, epochs, actions })
    }
}
impl Certificate {
    pub fn encode(&self) -> Vec<u8> { let mut e = Encoder::new(); e.bytes(&self.anchor).u32(self.signatures.len() as u32); for (s, sig) in &self.signatures { e.bytes(s).bytes(sig); } e.finish().to_vec() }
    pub fn decode(b: &[u8]) -> Option<Certificate> { let mut d = Decoder::new(b); let anchor = d.hash().ok()?; let n = d.u32().ok()? as usize; if n > 1024 { return None; } let mut c = Certificate::new(anchor); for _ in 0..n { let s = d.array::<32>().ok()?; let sig = d.array::<64>().ok()?; c.add(s, sig).ok()?; } if d.remaining() != 0 { return None; } Some(c) }
}
// Ledger: add `certificates: Vec<Certificate>`; push in `accept` (cert.clone()) and `accept_trusted_operator` (Certificate::new(a.id())); add `certificates()`, and
pub fn restore(chain_id: u32, entries: Vec<(Anchor, Certificate)>) -> Result<Ledger, LineageError> {
    let mut l = Ledger::new(chain_id);
    for (a, c) in entries { if c.anchor != a.id() { return Err(LineageError::Fork); } l.check(&a)?; l.anchors.push(a); l.certificates.push(c); }
    Ok(l)
}
```
(Use the existing `LineageError` variant that best names a certificate/anchor mismatch; if none fits, add `CertificateMismatch`.) Drop the `ENCODED_LEN` const if `Decoder::remaining` suffices.

- [ ] **Step 4: Run tests** — `cargo test -p zyn` → PASS.

### Task 2: `FileStore::save_ledger` / `load_ledger`

**Files:**
- Modify: `zyn/src/store.rs`
- Test: inline tests in `store.rs` (temp dir)

**Interfaces:**
- Produces: `FileStore::save_ledger(&self, ledger: &Ledger) -> Result<(), StoreError>` writing `chain-<id>.anchors` = `b"ZYNANCH01" ‖ n u32 ‖ (len u32 ‖ Anchor::encode() ‖ len u32 ‖ Certificate::encode())*`, tmp+fsync+rename like `save`; `FileStore::load_ledger(&self, chain_id: u32) -> Result<Option<Ledger>, StoreError>` (None when absent; `StoreError::Decode` on junk; `StoreError::RootMismatch` if `Ledger::restore` fails).

- [ ] **Step 1: Failing test**

```rust
#[test]
fn a_ledger_survives_the_disk() {
    let dir = std::env::temp_dir().join(format!("zyn-ledger-{}", std::process::id()));
    let fs = FileStore::new(&dir).unwrap();
    assert!(fs.load_ledger(7).unwrap().is_none());
    let mut l = Ledger::new(7);
    l.accept_trusted_operator(/* anchor(1,100,0,1,10) built as in anchor.rs tests */).unwrap();
    fs.save_ledger(&l).unwrap();
    let back = fs.load_ledger(7).unwrap().unwrap();
    assert_eq!(back.head_root(), l.head_root());
    std::fs::write(dir.join("chain-7.anchors"), b"garbage").unwrap();
    assert!(fs.load_ledger(7).is_err());
}
```
- [ ] **Step 2: Run, see it fail** (`cargo test -p zyn store`).
- [ ] **Step 3: Implement** with the same tmp/fsync/rename discipline as `save`.
- [ ] **Step 4: `cargo test -p zyn` → PASS.**

### Task 3: `zyn::journal` — writer, reader, `replay`

**Files:**
- Create: `zyn/src/journal.rs` (gate `#[cfg(feature = "std")] pub mod journal;` in `lib.rs`)
- Test: inline tests using `swapvm::state::SwapState` as dev-dependency (already available)

**Interfaces:**
- Produces:
  - `pub struct Journal { dir: PathBuf, chain_id: u32, open: Option<(u64 /*epoch*/, File)> }`
  - `Journal::open(dir: impl AsRef<Path>, chain_id: u32) -> std::io::Result<Journal>` (creates `<dir>/journal/chain-<id>/`)
  - `Journal::record(&mut self, epoch: u64, seq: u64, encoded: &[u8]) -> std::io::Result<()>` — opens/creates `epoch-<epoch>.intents` (writes header `b"ZYNJRNL1" ‖ chain_id u32 ‖ epoch u64` if new), appends `seq u64 ‖ len u32 ‖ bytes`; switching epoch closes the previous file with `sync_all`.
  - `Journal::sync(&mut self) -> std::io::Result<()>`
  - `pub fn epoch_path(dir, chain_id, epoch) -> PathBuf`
  - `pub fn read_epoch(bytes: &[u8]) -> Result<EpochFile, JournalError>` where `pub struct EpochFile { pub chain_id: u32, pub epoch: u64, pub records: Vec<(u64, Vec<u8>)> }`; errors `JournalError::{NotAJournal, Truncated, WrongChain, OutOfOrder}` (seqs must strictly increase within and across files).
  - `pub fn replay<V: MicrochainVm>(start: V, epochs: &[EpochFile]) -> Result<Replayed<V>, ReplayError>` with `pub struct Replayed<V> { pub state: V, pub checkpoints: Vec<Checkpoint> }` and `pub enum ReplayError { Undecodable { epoch: u64, seq: u64 }, SeqGap { expected: u64, got: u64 }, EpochMismatch { file: u64, state: u64 }, NoSeal { epoch: u64 } }`. For each record: require `seq == state.seq()+1`, decode with `V::decode_intent`, `apply`, and if `V::sealed(&receipts)` is `Some(cp)` push it. Each file must end in a seal (`NoSeal` otherwise) except possibly the last (an epoch still open).
- Consumes: `MicrochainVm::{decode_intent, apply, sealed, seq, epoch}`.

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn records_round_trip_and_replay_reproduces_the_checkpoint() {
    use swapvm::{state::SwapState, tx::Intent, types::XZEC, Fixed, Params, wire};
    use zyn_vm::spec::MicrochainVm;
    let dir = std::env::temp_dir().join(format!("zyn-jrnl-{}", std::process::id()));
    let mut j = Journal::open(&dir, 5).unwrap();
    let mut s = SwapState::new(5, Params::v1());
    let observed = s.backing_of(XZEC).add(Fixed::whole(1_000)).unwrap();
    let intents = vec![
        Intent::AttestVaultBalance { asset: XZEC, observed },
        Intent::next_deposit(&s, [1u8; 32], XZEC, Fixed::whole(10), [0u8; 32]),
        Intent::Checkpoint,
    ];
    let mut cps = Vec::new();
    for i in intents {
        let seq = s.seq() + 1;
        j.record(s.epoch(), seq, &wire::encode_intent_bytes(&i)).unwrap();
        let r = s.apply(seq, &i);
        if let Some(cp) = SwapState::sealed(&r) { cps.push(cp); }
    }
    j.sync().unwrap();
    let file = read_epoch(&std::fs::read(epoch_path(&dir, 5, 0)).unwrap()).unwrap();
    assert_eq!(file.records.len(), 3);
    let out = replay(SwapState::new(5, Params::v1()), &[file]).unwrap();
    assert_eq!(out.checkpoints, cps);
    assert_eq!(out.state.state_root(), s.state_root());
}

#[test]
fn a_tampered_record_diverges_or_fails_to_decode() {
    // flip one byte of the deposit amount in the file; replay must not reproduce the checkpoint root
}

#[test]
fn a_gap_in_sequence_numbers_is_refused() { /* drop the middle record; expect ReplayError::SeqGap */ }
```
- [ ] **Step 2: Run, see them fail.**
- [ ] **Step 3: Implement** (`std::fs::OpenOptions::append`, `BufWriter` flushed on `sync`, header written when file length is 0).
- [ ] **Step 4: `cargo test -p zyn` → PASS.**

### Task 4: `Recorder` hook in `Node`; `propose_anchor` / `confirm_anchor`; `set_published`; `resume_with_ledger`

**Files:**
- Modify: `zyn/src/node.rs`
- Test: `mod tests` in `node.rs`

**Interfaces:**
- Produces:
  - `pub trait Recorder { fn record(&mut self, epoch: u64, seq: u64, encoded: &[u8]) -> bool; }` (alloc-only; `Box<dyn Recorder>` stored in `Node`; default none).
  - `Node::with_recorder(self, r: Box<dyn Recorder>) -> Self`
  - `Step` gains `pub unrecorded: bool` — `true` when the recorder refused; then `receipts` is empty and **nothing was applied** (seq not consumed).
  - `Node::propose_anchor(&mut self, now: u64) -> Option<Anchor>`: like `make_anchor` up to (not including) ledger acceptance; stores `proposed: Option<(Anchor, usize /*epochs covered*/)>`; returns `None` if one is already proposed or nothing is pending.
  - `Node::proposed(&self) -> Option<&Anchor>`
  - `Node::confirm_anchor(&mut self, id: AnchorId, cert: Option<Certificate>) -> Result<Anchor, LineageError>`: requires `proposed` with that id; `accept` (with cert if a signer set is configured, else `accept_trusted_operator`), sets `published` from the covered `Sealed`, drains only the covered `pending`, resets anchor counters, applies `finality_intent` (recording it first), clears `proposed`.
  - `Node::anchor_now` is kept (immediate, trusted-operator) and documented as the V0 path; `submit` no longer calls `make_anchor` automatically when a recorder-backed daemon is in "propose" mode — implement by a flag `manual_anchoring: bool` set by `with_manual_anchoring()`; when set, `submit`/`seal` never call `make_anchor`.
  - `Node::set_published(&mut self, s: Snapshot<V>)` (for replicas).
  - `Node::resume_with_ledger(state, policy, economics, ledger: Ledger, now) -> Self`; `resume` delegates with `Ledger::new`.
  - `Node::ledger_mut` **not** exposed.
- Recorder placement: in `submit` before `self.state.apply`; in `seal` before applying `V::seal_intent()` (if refused: return `None`, epoch stays open); in `confirm_anchor` before the finality intent (if refused: return `Err(LineageError::…)` **before** `accept` — reorder so nothing is committed).

- [ ] **Step 1: Failing tests**

```rust
struct Tape(alloc::vec::Vec<(u64, u64, alloc::vec::Vec<u8>)>, bool);
impl Recorder for Tape { fn record(&mut self, e: u64, s: u64, b: &[u8]) -> bool { if !self.1 { return false; } self.0.push((e, s, b.to_vec())); true } }

#[test]
fn every_applied_intent_is_recorded_before_it_is_applied_including_seals() { /* 10 deposits with policy(); expect 11 records (10 + Checkpoint) in seq order, epochs 0 */ }

#[test]
fn a_refused_record_means_nothing_was_applied() { /* Tape(.., false): step.unrecorded && state.seq() unchanged */ }

#[test]
fn an_anchor_is_proposed_then_confirmed_and_only_then_published() {
    let mut n = node().with_manual_anchoring();
    // seal 3 epochs
    let a = n.propose_anchor(0).expect("proposal");
    assert!(n.publishable().is_none());
    assert!(n.propose_anchor(0).is_none(), "one in flight at a time");
    // one more epoch seals while it is in flight
    let confirmed = n.confirm_anchor(a.id(), None).unwrap();
    assert_eq!(confirmed.checkpoint, a.checkpoint);
    assert_eq!(n.publishable().unwrap().root, a.checkpoint.state_root);
    assert_eq!(n.pending().len(), 1, "the epoch sealed during flight stays pending");
    assert_eq!(n.ledger().head_root(), a.checkpoint.state_root);
}

#[test]
fn resume_with_a_ledger_continues_the_lineage() { /* anchor once, take ledger, resume_with_ledger, propose again, previous_root == head */ }
```
- [ ] **Step 2: Run, fail.** - [ ] **Step 3: Implement.** - [ ] **Step 4: `cargo test -p zyn` → PASS** (existing tests must still pass: `anchor_now` unchanged).

### Task 5: Wire journal + ledger into `zynzapd`

**Files:**
- Modify: `zynzapd/src/main.rs` (resume block ~L144-170; `save()` ~L761; shutdown ~L747), `zynzapd/src/config.rs` (add `journal_dir: PathBuf` default `<data_dir>`), `zynzapd/src/lib.rs` (no change)
- Test: `zynzapd/tests/journal_live.rs`? No — unit: `zynzapd/src/main.rs` has no tests; add `zynzapd/tests/resume.rs` that runs `Node` + `FileStore` + `Journal` through the same helper used by the daemon (extract `fn open_node(cfg) -> Result<(Node, FileStore, Journal)>` into `zynzapd/src/boot.rs` so it is testable).

**Interfaces:**
- Produces: `zynzapd::boot::open(cfg: &Config) -> Result<Booted, String>` where `pub struct Booted { pub node: Node<SwapState>, pub store: FileStore, pub journal: Arc<Mutex<Journal>> }`; `boot::save(&store, &shared)` also saves the ledger; a `JournalRecorder(Arc<Mutex<Journal>>)` implementing `zyn::node::Recorder` (returns `false` on I/O error and prints `zynzapd: JOURNAL WRITE FAILED — refusing intent`).
- Resume consistency: refuse when `ledger.last().checkpoint.epoch > saved.epoch` (a ledger ahead of its state means the state file is stale).

- [ ] **Step 1: Failing test** (`zynzapd/tests/resume.rs`): boot a fresh chain in a temp dir, submit 12 operator deposits (policy seal_every=10), `anchor_now`, `save`; boot again → `node.ledger().len()==1`, journal has `epoch-0.intents` and `epoch-1.intents`, `replay` from genesis reproduces the saved root.
- [ ] **Step 2: fail.** - [ ] **Step 3: Implement `boot.rs`, use it from `main.rs`.** - [ ] **Step 4: `cargo test -p zynzapd resume` → PASS; `cargo test --workspace` → PASS.**

---

## Group B — The anchor transaction

### Task 6: Distinct anchor memo tag, on both sides

**Files:**
- Modify: `zyn/src/anchor.rs` (`MEMO_MAGIC = b"ZYA"`), `zyn-custody/src/memo.rs` (`pub const ANCHOR_TAG: &[u8; 3] = b"ZYA"; pub const ANCHOR_LEN: usize = 48; pub fn is_anchor(memo: &[u8]) -> bool` — tag, version 1, length ≥ 48, everything past 48 zero)
- Test: `zyn/src/anchor.rs` tests (update `the_memo_fits_an_op_return_and_round_trips`, `a_foreign_memo_is_not_mistaken_for_an_anchor`); `zyn-custody/src/memo.rs` test `an_anchor_memo_is_not_a_deposit_and_a_deposit_is_not_an_anchor`; `zynzapd/tests/memo_tags.rs` asserting `zyn::anchor::MEMO_MAGIC == zyn_custody::memo::ANCHOR_TAG` and that `Anchor::parse_memo` accepts exactly what `is_anchor` accepts on a 512-byte padded field.

- [ ] Steps 1-4 as above.

### Task 7: The scanner recognises anchor self-sends

**Files:**
- Modify: `zyn-custody/src/shielded.rs`: `ScannedAction { …, pub anchor: bool }`; in `scan_actions_with`, when `memo::is_anchor(&memo_bytes)` set `anchor = true`, `account = None`, and do **not** push a deposit; `classify` skips actions with `anchor == true`; new `Scanner::anchors(&self, zebra: &Zebra, from: u64, to: u64) -> Result<Vec<AnchorSighting>, ScanRangeError>` with `pub struct AnchorSighting { pub height: u64, pub txid: [u8; 32], pub memo: [u8; 48] }` (walks `block_txids` / `raw_transaction_bytes` like `observe`, uses `scan_actions_lenient`).
- Test: synthetic round trip as the existing encrypt-then-decrypt tests do (`zyn-custody/src/shielded.rs` tests): a self-send with an anchor memo yields `anchor == true`, no deposit, and `classify` returns `Ok(vec![])` even with `spends_ours == false`.

- [ ] Steps 1-4.

### Task 8: `AnchorSettler` — build, broadcast, confirm

**Files:**
- Create: `zynzapd/src/anchor.rs`; add `pub mod anchor;` to `lib.rs`
- Modify: `zynzapd/src/settle.rs` — make `Ledger`, `Entry`, `Status` reusable with a `kind` file suffix: `Ledger::load_named(dir, chain_id, name: &str)` / `save_named` (existing `load/save` delegate with `format!("{}", asset)`).
- Test: `zynzapd/src/anchor.rs` unit tests for the memo payload builder and ledger transitions using a fake `Chain` trait (below); the real build/sign path is exercised by `zynzapd/tests/zcash_settle.rs`-style live test later (Task 20).

**Interfaces:**
- Produces:
  - `pub trait Chain { fn tip(&self) -> Result<u64, String>; fn confirmations(&self, txid: &str) -> Result<Option<u64>, String>; fn broadcast(&self, hex: &str) -> Result<String, String>; }` implemented for `Zebra`.
  - `pub struct AnchorSettler { … same fields as ZcashSettler minus reveals/asset, plus ledger: settle::Ledger (name "anchors"), confirmations, chain_id }`
  - `AnchorSettler::new(zebra: Zebra, network, shares, threshold, fvk, notes: PoolStores, confirmations: u64, dir: PathBuf, chain_id: u32) -> Result<Self, String>` (same key check as `ZcashSettler::new`).
  - `AnchorSettler::poll_once(&mut self, node: &rpc::Shared, now: u64) -> Result<Option<Confirmed>, String>` where `pub struct Confirmed { pub anchor: Anchor, pub txid: String, pub height: u64 }`:
    1. `follow_up`: for each `Broadcast` entry, `confirmations(txid)`: `Some(d) if d >= confirmations` → `node.confirm_anchor(id, None)` → entry `Confirmed`, return `Some(Confirmed{..})`; `Some(_)` wait; `None` past expiry → `Abandoned`; `None` → rebroadcast.
    2. if nothing in flight: `node.propose_anchor(now)` (or reuse `node.proposed()` if a proposal exists from an abandoned tx) → build `Payment { to: Destination::Shielded(fvk.address_at(0, Scope::External)), zatoshi: 10_000, memo: padded(anchor.memo()) }` via the exact sequence in `ZcashSettler::settle_next` (Ironwood then Orchard; `build` probe for fee; sighash; finalize_io; prove; sign_all; extract) → ledger entry `Broadcast` with `nonce = anchor.id()`, `covers: vec![]` → `broadcast`.
  - `pub fn anchor_memo_field(a: &Anchor) -> [u8; 512]`.
- Height for `Confirmed`: `tip - depth + 1` from `confirmations` (or add `Zebra::tx_height(txid)` if `getrawtransaction` verbose returns `height` — check; otherwise record `tip` at first sighting).

- [ ] **Step 1: Failing tests** (fake `Chain`, fake node with `propose_anchor` via a real `Node<SwapState>` in manual mode; skip signing by feature-gating the build step behind a `Builder` trait with a fake that returns fixed bytes/txid).
- [ ] **Step 2-4.**

### Task 9: Wire the anchor settler into the daemon

**Files:**
- Modify: `zynzapd/src/main.rs` (construct `AnchorSettler` when `cfg.zebra.settle` is set; `node.with_manual_anchoring()` in that case; in the tick loop replace `if sealed { n.anchor_now(now) }` with `if sealed { n.propose_anchor(now); }` when manual, and poll the settler every `bridge_poll_secs`; on shutdown: `seal_now` only — a proposal survives in the ledger file); `alerter.report("anchors", …)`; log clearly on start: `anchors: ON ZCASH via self-send` vs `anchors: IN-MEMORY ONLY (no ZYN_ZEBRA_SHARES) — not a mainnet posture`.
- Test: covered by Task 5's boot test plus Task 20 live.

- [ ] Steps: implement; `cargo build -p zynzapd`; `cargo test --workspace`.

---

## Group C — Bundles, publisher, mirror

### Task 10: `Published::encode/decode`

**Files:** Modify `zyn/src/da.rs`. Layout: `b"ZYNPUB01" ‖ chain_id u32 ‖ epoch u64 ‖ root[32] ‖ n_sections u32 ‖ sections* ‖ n_leaves u32 ‖ leaves*`. `decode` caps `n_sections ≤ 64`, `n_leaves ≤ 1<<24`.
- Test: round trip; `verify()` still passes after decode; truncated → None.

### Task 11: `zynzapd::publish` — bundle, local write, mirror push, HTTP file server

**Files:**
- Create: `zynzapd/src/publish.rs`; `pub mod publish;`
- Test: unit tests with temp dirs; HTTP server test spins the server on `127.0.0.1:0`, PUTs with a token via `ureq`, GETs back, asserts 401 without token and 404 for missing.

**Interfaces:**
- `pub struct Bundle { pub anchor: Anchor, pub certificate: Certificate, pub published: Published, pub intents: Vec<(u64, Vec<u8>)>, pub txid: String, pub height: u64 }`
- `pub fn rel_paths(chain_id, epoch) -> Vec<String>` (the file list, deterministic)
- `pub fn write_local(root: &Path, chain_id: u32, b: &Bundle) -> Result<Vec<(String, Vec<u8>)>, String>` — writes files (tmp+rename), appends the index line, returns `(rel_path, bytes)` for pushing.
- `pub fn read_local(root, chain_id, epoch) -> Result<Bundle, String>` (index line lookup for txid/height).
- `pub struct Mirror { pub url: String, pub token: Option<String> }`; `pub fn parse_mirrors(spec: &str) -> Vec<Mirror>` (`url[|token]` comma-separated)
- `pub struct Publisher { root, chain_id, mirrors, queue: Vec<(String, Vec<u8>)>, failures: u32 }`; `Publisher::queue(&mut self, files)`; `Publisher::poll_once(&mut self) -> Result<usize, String>` — PUT each queued file to each mirror (`ureq::put(url + "/" + rel).set("Authorization","Bearer …").send_bytes`), drop from queue on success for **all** mirrors, else keep; returns files still queued as `Err` after `failures >= 3`? No: returns `Ok(pushed)`, and `queued_len()` for health.
- `pub mod http { pub fn serve(root: PathBuf, listen: SocketAddr, put_token: Option<String>) -> std::io::Result<()> }` — minimal HTTP/1.1: `GET /<rel>` → 200 + body (`Content-Type: application/octet-stream`), 404; `PUT /<rel>` → 401 unless `Authorization: Bearer <token>` matches, 201 on write (tmp+rename, path traversal refused → 400); anything else 405. One thread per connection like `rpc::serve`. Max body 64 MiB.
- Bin `zynzapd/src/bin/zyn-mirror.rs`: `ZYN_MIRROR_DIR`, `ZYN_MIRROR_LISTEN`, `ZYN_MIRROR_TOKEN`.

- [ ] Steps 1-4 per interface (write tests first).

### Task 12: Publish on every confirmed anchor

**Files:**
- Modify: `zynzapd/src/main.rs` — after `AnchorSettler::poll_once` returns `Some(Confirmed)`: assemble `Bundle` (`node.publishable().published()`, `node.ledger().certificates().last()`, journal files for epochs `cp.epoch - anchor.epochs + 1 ..= cp.epoch` — helper `journal::files_for(dir, chain_id, range) -> Result<Vec<(u64, Vec<u8>)>, io::Error>`), `publish::write_local`, `publisher.queue(files)`; `publisher.poll_once()` each tick with `alerter.report("da mirrors", …)`.
- Config: `ZYN_DA_DIR` (default `<data_dir>/da`), `ZYN_DA_MIRRORS`.
- Test: `zynzapd/tests/publish.rs` — drive a `Node` in manual mode with the journal, fake-confirm an anchor, assert the bundle on disk verifies: `Anchor::decode(anchor.bin).id() == …`, `Published::decode(published.bin).verify()`, intents replay to the anchor's checkpoint.

---

## Group D — The replica

### Task 13: `zynzapd::replica` core (no network)

**Files:**
- Create: `zynzapd/src/replica.rs`; `pub mod replica;`
- Test: `zynzapd/tests/replica.rs` (in-process, no Zebra): sequencer side = `Node` + `Journal` + `publish::write_local` into `dirA`; replica side = `Replica` with `Fetcher::local(dirA)` and a `Vec<Sighting>` source.

**Interfaces:**
- `pub struct Sighting { pub height: u64, pub txid: [u8; 32], pub chain_id: u32, pub epoch: u64, pub id: AnchorId }`; `pub fn sighting_from(s: &zyn_custody::shielded::AnchorSighting) -> Option<Sighting>` (uses `Anchor::parse_memo`).
- `pub trait Fetch { fn get(&self, rel: &str) -> Result<Vec<u8>, String>; }`; `pub struct Local(PathBuf)`; `pub struct Http { mirrors: Vec<String>, agent }` (tries in order; marks a mirror bad for the pass after a hash failure).
- `pub enum Halt { Fork { epoch: u64, seen: AnchorId, other: AnchorId }, Lineage(LineageError), BadBundle { epoch: u64, what: String }, Diverged { epoch: u64, seq: u64 }, Fetch(String) }`
- `pub struct Replica { pub chain_id: u32, dir: PathBuf, ledger: Ledger, state: SwapState, pub verified_epoch: Option<u64>, pub verified_height: u64, pub halted: Option<Halt>, seen: BTreeMap<u64, AnchorId> }`
- `Replica::open(dir, chain_id, params: Params) -> Result<Replica, String>` (resume from `chain-<id>.state` + `.anchors` if present, else genesis).
- `Replica::apply_sightings(&mut self, sightings: &[Sighting], fetch: &dyn Fetch) -> Result<usize, Halt>` — for each sighting with `chain_id == self.chain_id`, in `(height, epoch)` order:
  1. if `seen[epoch]` exists and differs → `Halt::Fork`; if equal → skip.
  2. `anchor = Anchor::decode(fetch("chain-<id>/<epoch>/anchor.bin"))`, require `anchor.id() == sighting.id` else `BadBundle`.
  3. `self.ledger.check(&anchor)` → `Halt::Lineage`.
  4. certificate: decode; if a signer set is configured (later) verify; V0: accept.
  5. intents: fetch `epoch-<M>.intents` for `M in cp.epoch - anchor.epochs + 1 ..= cp.epoch`; `read_epoch` each (WrongChain/OutOfOrder → BadBundle); `journal::replay(self.state.clone(), &files)`; require `checkpoints.last() == Some(&anchor.checkpoint)` else `Diverged`.
  6. `published = Published::decode(...)`; `published.verify()` and `published.root == anchor.checkpoint.state_root` else `BadBundle`.
  7. commit: `self.state = replayed.state` (then apply `finality_intent(cp.epoch)` exactly as the sequencer does after acceptance — record-free), `ledger.accept_trusted_operator(anchor)`, `seen.insert`, persist `Saved` + ledger + copy the bundle into `dir/da/` (so the replica re-serves it), `verified_epoch = Some(cp.epoch)`, `verified_height = height`.
  On `Err(halt)`: set `self.halted`, return.
- `Replica::snapshot(&self) -> Snapshot<SwapState>` (of the verified state — equals the published one by construction; assert in tests) and `Replica::state(&self) -> &SwapState`.

- [ ] **Step 1: Failing tests** (in `zynzapd/tests/replica.rs`):

```rust
#[test] fn a_replica_reproduces_every_anchored_root_from_zcash_and_mirrors_alone() { /* 3 anchors; verified_epoch == last; state_root == sequencer's ledger head */ }
#[test] fn a_flipped_byte_in_any_artefact_is_detected() { /* for each file in the bundle: copy dir, flip byte, fresh replica -> halted BadBundle|Diverged */ }
#[test] fn a_second_anchor_for_the_same_epoch_is_a_fork_and_the_replica_stops() {}
#[test] fn a_holder_exit_proof_from_the_replica_verifies() { /* snapshot.record_proof(id) -> verify_record::<SwapState>(record, path, anchored) */ }
#[test] fn the_replica_resumes_from_disk_and_continues() {}
```
- [ ] **Step 2-4.**

### Task 14: RPC — read-only mode, `OP_ACCOUNT_PROOF` from published, paged `OP_SNAPSHOT`, status appends

**Files:**
- Modify: `zynzapd/src/rpc.rs`: `Server { …, pub read_only: Option<String> /* sequencer url */, pub verified: Option<Arc<Mutex<(Option<u64>, u64)>>> }`; writes (`OP_SUBMIT`, `OP_SUBMIT_MULTI`, `OP_REVEAL`) → `error_frame("read-only replica; submit to <url>")` when `read_only` is set; `account_proof` uses `n.publishable()` (error "no anchored snapshot yet — …" before the first anchor); `snapshot` reads optional `offset u32, limit u32` (absent ⇒ `0, 32_000`), reply appends `total u32` after the leaves; `status_body` appends `role u8` (0 sequencer / 1 replica), `anchored_epoch u64` (ledger last, or 0), `verified_height u64`.
- Modify: `zynzapd/src/client.rs` `Status` decode: read the new trailing fields if present (keep tolerant); `snapshot()` client paging helper `fn snapshot_all(&self) -> Result<Published, String>`.
- Test: `rpc.rs` tests: paging returns identical leaves concatenated; proof before first anchor errors; read-only refuses submit with the sequencer URL in the message.

### Task 15: `zyn-replica` binary

**Files:**
- Create: `zynzapd/src/bin/zyn-replica.rs`
- Env: `ZYN_CHAIN_ID`, `ZYN_LISTEN` (default `127.0.0.1:8100`), `ZYN_DATA_DIR`, `ZYN_DA_MIRRORS` (GET base URLs), `ZYN_VAULT_FVK`, `ZYN_ZEBRA_HOST/PORT/USER/PASS`, `ZYN_CONFIRMATIONS` (default 10), `ZYN_BRIDGE_FROM_HEIGHT`, `ZYN_SEQUENCER` (url for the read-only message), `ZYN_PARAMS`, `ZYN_POLL_SECS` (30), `ZYN_DA_LISTEN` (optional: also serve its verified bundles over HTTP GET via `publish::http::serve` with no PUT).
- Loop: `Scanner::new(VaultKeys::from_full_viewing_key(fvk), from, max_blocks).anchors(&zebra, from, tip - confirmations)` → `sighting_from` → `replica.apply_sightings` → on success update the shared `Node` (`Node::resume(state…)` + `set_published(replica.snapshot())`) behind `rpc::Shared`; print one status line per pass; alerts via `Alerter` (`ROOT_UNREPRODUCIBLE`, `FORK`, `LINEAGE`, `MIRRORS_DOWN`).
- Test: build + `--help`; behaviour covered by Task 13 and Task 20.

---

## Group E — Holder tooling

### Task 16: `zyn-cli record | exit-proof | verify-exit`

**Files:**
- Modify: `zynzapd/src/bin/zyn-cli.rs`; shared encoding in `zynzapd/src/exitproof.rs` (new): `pub struct ExitProof { chain_id, epoch, root, anchor_id: Option<AnchorId>, record: Vec<u8>, index: u32, path: Vec<ProofStep> }` with `to_json` / `from_json` (serde_json is a dep) and `verify(&self) -> bool` (= `zyn::verify_record::<SwapState>`).
- Commands: `record <key-file> [host:port]` → signed `OP_ACCOUNT_PROOF` → write `<key-file>.exit.json`; `exit-proof <key-file> [host:port]` → refresh if reachable else use file; verify; print JSON; `verify-exit <file.json>` → verify; print `epoch`, `root`, `anchor_id`, and "check this anchor id in the memo of Zcash tx … (see `chain-<id>/index` on any mirror)".
- Test: `zynzapd/src/exitproof.rs` unit: round trip; tampered record fails.

### Task 17: Nap Wallet keeps record + path per anchor

**Files:**
- Modify: `zynzapd/src/app.rs` (sync job: after status shows a new `anchored_epoch`, fetch `OP_ACCOUNT_PROOF` and write `exit-<network>.json` via `ExitProof`); `zynzapd/ui/index.html` Settings: "Exit proof: epoch N · verified ✓ / none yet", with a "Verification source" field (replica URL) used for `OP_STATUS`/`OP_SNAPSHOT` reads when set.
- Test: `app.rs` unit for the retention decision (new epoch ⇒ refresh; same epoch ⇒ no call).

---

## Group F — Docs and the live run

### Task 18: RUNBOOK §4s "Verifying the chain", DECISIONS §7c status line, stale entries corrected
- Add `## 4s. Verifying the chain (zyn-replica, zyn-mirror)` with the env vars, the failure alarms and what each means, and "how a holder exits with the sequencer gone" (three commands).
- DECISIONS: §7c item 4 → built; note anchoring is now on-chain; correct 7e.1 (Nap Wallet exists) and the line-2280 "withdrawals are not built".

### Task 19: `cargo test --workspace` and clippy clean
- `cargo test --workspace`; `cargo clippy --workspace --all-targets -- -D warnings` (match the repo's existing posture; if the repo is not clippy-clean today, only new code must be).

### Task 20: Live testnet run (report only; user checks behaviour)
1. Deploy `zynzapd` to the Hetzner box with `ZYN_DA_DIR`, `ZYN_DA_MIRRORS=http://<windows>:8181|<token>`; run `zyn-mirror` on the Windows box (WSL, port 8181, firewall rule).
2. Observe: first anchor tx broadcast, confirmed at depth 10, bundle written and mirrored, `ConfirmAnchor` released a pending deposit.
3. Run `zyn-replica` on the Windows box against the Hetzner Zebra (reader) + the mirror; observe `verified_epoch` catching up to the sequencer's anchored epoch.
4. Stop the sequencer; run `zyn-cli exit-proof` against the replica; `verify-exit` passes; restart the sequencer; lineage continues.
5. Record the observations in RUNBOOK §4s.

---

## Self-review notes

- Spec coverage: A → Tasks 1-5; B → 6-9; C → 10-12; D → 13-15; E → 16-17; F1/F2 → 14; F3 → 2/5; F4 → 6; F5 dropped from this plan (no signer configuration exists yet; belongs to piece 3) — noted in the spec's Order of work.
- Names used across tasks: `Recorder`, `with_recorder`, `with_manual_anchoring`, `propose_anchor`, `confirm_anchor`, `set_published`, `resume_with_ledger`, `Journal::{open, record, sync}`, `journal::{read_epoch, replay, epoch_path, files_for}`, `Published::{encode, decode}`, `publish::{Bundle, write_local, read_local, Publisher, Mirror, parse_mirrors, http::serve}`, `replica::{Sighting, sighting_from, Fetch, Local, Http, Halt, Replica}`, `anchor::{AnchorSettler, Confirmed, Chain, anchor_memo_field}`, `exitproof::ExitProof`, `Scanner::anchors`, `AnchorSighting`, `memo::{ANCHOR_TAG, ANCHOR_LEN, is_anchor}`.
