# Anchors on Zcash, data availability off the box, and a verifying replica

Status: draft for review, 7 Sep 2026. Piece 1 of three (then: sequencer standby;
then: the replica as a FROST signer). Everything here is designed so that the
replica built now is the thing the next two pieces extend rather than replace.

## 1. Why this piece is bigger than "mirror the snapshots"

The exit hatch's argument is: a holder proves their record against a root
*anyone can read off Zcash*, using leaves *anyone can fetch*, without asking the
sequencer. Reading the code on 7 Sep, neither half exists yet:

| Claim | Reality today |
|---|---|
| Anchors are committed to Zcash | `Node::anchor_now` accepts the anchor into an in-memory `Ledger` as trusted operator. No transaction is built or broadcast. |
| Lineage is checkable | `Node::resume` starts an empty `Ledger`; `previous_root` resets to `[0;32]` at every restart. |
| A verifier can reproduce roots | swapvm folds each intent into `intent_acc` and discards it. There is no journal; nothing can replay. |
| A holder gets a proof against an anchored root | `OP_ACCOUNT_PROOF` calls `Snapshot::of(live state)`. |

So "DA off the sequencer" is four things: make anchoring real, make replay
possible, publish what replay and exits need somewhere durable, and run a
process that consumes it and checks. That process is the replica, and it is the
seed of the standby (piece 2) and the signer (piece 3): a standby is a replica
that can be promoted; a signer is a replica holding a share.

## 2. Components

```
  sequencer (zynzapd)                       mirrors            replica (zyn-replica)
  ┌──────────────────────┐   PUT bundles   ┌────────┐  GET     ┌──────────────────────┐
  │ journal  (A)         │ ──────────────▶ │ da/…   │ ◀─────── │ fetch + verify (D)   │
  │ anchor tx (B) ───────┼─▶ Zcash ◀───────┼────────┼──────────┤ scan self-sends      │
  │ publisher (C)        │                 └────────┘          │ replay, check roots  │
  │ RPC (paged, fixed)   │                                     │ serve read RPC       │
  └──────────────────────┘                                     │ keep Saved per root  │
                                                               └──────────────────────┘
  holder: zyn-cli exit-proof / Nap Wallet keeps record + path at each anchor   (E)
```

### A. Intent journal and persisted ledger — `zyn::journal` (std, beside `store`)

**What.** An append-only record of every intent the sequencer applied, in the
canonical bytes the VM itself hashed, so a replayer recomputes exactly the
`intent_root` each checkpoint committed.

**Format.** One file per epoch, `journal/chain-<id>/epoch-<N>.intents`:

```
magic  "ZYNJRNL1"
chain_id u32   epoch u64
records: [ seq u64 | len u32 | bytes ]*      (bytes == V::encode_intent(intent))
```

Records are appended *before* `Node::submit` applies them (the VM's own rule:
history is recorded before execution, so the commitment covers what was asked).
`fsync` on every seal, and on shutdown. The file for epoch N is complete once
the checkpoint intent that sealed N is its last record.

**Replay contract.** `journal::replay::<V>(genesis_or_saved, files) -> Result<(V, Vec<Checkpoint>), ReplayError>`
applies each record with `V::decode_intent` + `apply(seq, …)`, and after each
`Checkpoint` receipt compares `state_root` and `intent_root` with the sealed
checkpoint. `ReplayError::Diverged { epoch, seq, expected, got }` names the first
mismatch. This is the only function the replica trusts.

**Ledger persistence.** `store::FileStore` gains `save_ledger/load_ledger`
(`chain-<id>.anchors`: `[Anchor ‖ Certificate]*`, canonical bytes). `Node::resume`
takes the loaded ledger so `head_root()` continues from the last anchor Zcash
saw. A `Saved` blob whose root is not the ledger head's root or a later sealed
root is refused at resume.

**Bounded by construction.** The journal is written by the same thread that
applies intents, under the same lock, so it cannot contain an intent the VM did
not apply or miss one it did.

### B. The anchor transaction — `zynzapd::anchor` (beside `settle`)

**What.** Each `Anchor` becomes one Orchard/Ironwood *self-send* from the vault
to its own address 0, carrying the anchor memo, built and signed exactly as an
exit is: `payout::build → sighash → finalize_io → prove → sign_all(quorum) →
extract → send_raw_transaction`, with its own ledger (`anchors.ledger`) of
`Broadcast / Confirmed / Abandoned` entries and an expiry, and rebroadcast
until seen. Value: the ZIP-317 floor for two actions, 10,000 zat, paid from the
vault's fee surplus (never from a holder's balance — the anchor is the
operator's cost, per `Economics`).

**Memo.** A new tag, so an anchor can never be parsed as a deposit or the
reverse:

```
"ZYA" ‖ 0x01 ‖ chain_id u32 ‖ epoch u64 ‖ anchor_id [32]      (48 bytes, zero-padded to 512)
```

`zyn::anchor::MEMO_MAGIC` changes from `b"ZYN"` to `b"ZYA"`. The scanner's
`classify` learns the tag explicitly: a self-send whose memo parses as an anchor
is `ScannedAction::anchor`, not "change", and is reported to the node.

**Acceptance moves to confirmation.** Today `anchor_now` accepts immediately.
After this change the sequencer *builds* the anchor at the policy boundary, broadcasts
it, and calls `Ledger::accept` (and sequences `ConfirmAnchor { epoch }`, which
releases pending deposits) only when Zebra reports `confirmations` on the
transaction. Until then the epoch stays in `pending`. Failure modes:

- Broadcast refused / unseen past expiry → entry `Abandoned`, anchor rebuilt with
  the same checkpoint (same id, so nothing forks) and rebroadcast.
- No pool can fund the anchor → alert `ANCHOR_UNFUNDED`; the chain keeps sealing
  but nothing finalises, which is the correct failure (deposits stay pending).
- Zcash reorg below `confirmations` → the confirmation check is depth-based and
  already tolerates it; the ledger entry returns to `Broadcast`.

**Cost.** At `EpochPolicy::v1` (anchor every 7 epochs or 15 min): ≤ ~100 tx/day,
≈ 0.001 ZEC/day plus fees. Negligible against the exit path it makes real.

### C. DA bundles and the publisher — `zynzapd::publish`

**What.** For every *accepted* anchor, one immutable bundle:

```
da/chain-<id>/<epoch>/
  anchor.bin        148 B, Anchor::encode()
  certificate.bin   Certificate (empty in a V0 trusted-operator run — stated, not hidden)
  published.bin     zyn::da::Published: sections + leaves (leaves only, never records)
  intents/epoch-<M>.intents   one per epoch the anchor covers (from A)
da/chain-<id>/index    append-only: epoch  root  anchor_id  txid  height
```

Everything is checkable without the sequencer: `anchor.bin` against the memo's
`anchor_id`; `published.bin` by `Published::verify()` against `anchor.state_root`;
intents by replay. A mirror that alters one byte fails one of those checks.

**Publishing.** The publisher runs in the sequencer's poll loop after each
accepted anchor: write locally, then HTTP `PUT` each file to every configured
mirror (`ZYN_DA_MIRRORS=url1,url2`; `ureq` is already a dependency; a bearer
token per mirror from the environment). Any static object store or a plain
nginx `dav` location works. A failed mirror is retried every poll and alerted
after N failures — the anchor is *not* held back for it, because holding back
finality on a mirror outage would be the wrong dependency direction; the
sequencer keeps serving over RPC in the meantime and the replica catches up.

**Where.** Recommendation: two mirrors to start — one cheap public object store
(Cloudflare R2 or Backblaze B2, ~€1/month, public GET) and the Windows box via
its WSL nginx as a second copy — and, structurally, **every replica re-serves the
bundles it verified**. That last property is what makes DA outlive the
sequencer without special infrastructure: the set of mirrors grows with the set
of verifiers.

### D. The replica — bin `zyn-replica` (in zynzapd, sharing `rpc`, `zebra`, `shielded`)

**Config.** Chain id, one or more mirror URLs, a Zebra reader (`connect_reader`,
transparent RPC + raw blocks), the vault **full viewing key** (view, never
spend), a data dir, `confirmations`, listen address.

**Loop.**
1. **Find anchors on Zcash.** Scan forward from the last verified height with
   `Scanner` under the FVK; for each self-send whose memo parses as `ZYA`,
   record `(height, txid, chain_id, epoch, anchor_id)` once it has
   `confirmations`. Order by height, then epoch. Two anchors claiming the same
   epoch with different ids is a **fork alarm** — logged, alerted, and the replica
   stops advancing rather than picking one.
2. **Fetch and check the bundle** from any mirror (try in order, fall back):
   `anchor.bin` must hash to the memo's `anchor_id`; `anchor.previous_root` must
   equal the last verified anchor's root (`Ledger::check`); `certificate.bin`
   must clear the configured signer set if one is configured (V0: none).
3. **Replay.** Fetch the intents for the covered epochs and run
   `journal::replay` from the last verified `Saved`. Every checkpoint must
   reproduce; the last must equal `anchor.checkpoint`. A divergence is the
   loudest alarm the system has — the sequencer anchored a root its own inputs
   do not produce — and the replica refuses to advance past it.
4. **Verify the snapshot.** `Published::verify()` and `root == anchor.state_root`.
5. **Persist** `Saved` at the anchored root, the bundle (now re-served), and the
   verified ledger. Advance.

**Serves.** Read-only RPC on the existing frame protocol: `OP_STATUS` (with
`verified_epoch`, `verified_height`, `lag`), `OP_SNAPSHOT` (paged — see F),
`OP_ACCOUNT_PROOF` against the *anchored* snapshot, `OP_POOLS`/`OP_ASSETS` from
the verified state, and the bundle files over HTTP GET. No writes: a replica
never holds operator authority, and `OP_SUBMIT` returns "this is a replica; the
sequencer is at <url>" so a wallet pointed at a replica still knows where to
send. Nap Wallet gains a "verification source" setting (a replica URL) and
shows the anchored epoch it was checked against.

**What it is not, yet.** It does not promote itself (piece 2) and holds no
shares (piece 3). But its state at any anchored root is a `Saved` a sequencer
can `resume` from, and its verified pending-exit list is exactly what a signer
attests to — so both extensions are additive.

### E. The holder's exit — `zyn-cli` and Nap Wallet

- `zyn-cli record <key> [--from URL]` — fetch and store the account's own
  record (signed read, `OP_ACCOUNT`) and its leaf index at the latest anchor.
- `zyn-cli exit-proof <key> [--replica URL]` — from the stored record plus
  `published.bin`, rebuild the leaf, produce the path to the anchored root,
  print `(anchor_id, txid, record, path)`; refuses if the record's leaf is not
  in the published leaves (state moved on — refresh the record).
- `zyn-cli verify-exit <proof>` — anyone: recompute the leaf, walk the path,
  compare to the root of the named anchor, check that anchor's memo is on
  Zcash at `txid`.
- Nap Wallet keeps `record + path` per network at every anchor it sees (one
  record + `log n` hashes — kilobytes), so the exit works even if every mirror is
  gone at the moment it is needed (`da.rs` already designs for this).

### F. Fixes folded in (each its own commit, each with a test)

1. `OP_ACCOUNT_PROOF` served from `publishable()`, not live state; before the
   first anchor it says so (same rule as `OP_SNAPSHOT`).
2. `OP_SNAPSHOT` paged: request carries `(offset u32, limit u32)`; reply carries
   `total`, so 1 MiB frames never truncate a chain with >32k accounts.
3. Ledger persisted and reloaded (A); `previous_root` no longer resets.
4. Anchor memo tag `ZYA`; deposit parser and anchor parser cannot accept each
   other's bytes (test).
5. `Node::resume` keeps the signer set (it drops it today).

## 3. Data-availability sizing

Per anchor at `v1` policy (7 epochs × 250 intents): intents ≈ 7 × 250 × ~100 B ≈
175 KB; leaves 32 B × accounts (3.2 MB at 100k accounts); anchor + certificate
< 1 KB. At one anchor per 15 min that is ≈ 17 MB/day of intents plus one leaf
set per anchor. Against the existing 0.4 GiB/day figure for periodic full
snapshots, this is the cheaper artefact — and it is written once.

## 4. Error handling and alarms

| Condition | Behaviour |
|---|---|
| Anchor tx cannot be funded | seal continues; nothing finalises; `ANCHOR_UNFUNDED` alert |
| Anchor unseen past expiry | entry abandoned, same anchor rebuilt (same id), rebroadcast |
| Mirror PUT fails | retried each poll; alert after N; finality not blocked |
| Replica: bundle hash ≠ memo id | that mirror marked bad; try next; alert if all fail |
| Replica: replay diverges | **stop**; alert `ROOT_UNREPRODUCIBLE` with epoch/seq; serve nothing past the last good root |
| Replica: two anchors for one epoch | **stop**; alert `FORK` |
| Replica: lineage break (`previous_root`) | **stop**; alert `LINEAGE` |
| Journal write fails | the intent is **refused** (not applied) — history cannot outrun its record |

Refusing to advance is always the safe direction for a verifier; a replica that
guessed would be worth less than none.

## 5. Testing

- **Unit:** journal round-trip and replay reproduces `intent_root` for every
  swapvm intent kind; anchor memo round-trip, deposit/anchor tag exclusion;
  ledger persistence and resume continuity; paged snapshot equals unpaged.
- **Integration, in-process** (`zyn/tests/replica.rs`, using a fake `ChainView`
  the way `engine.rs` does): sequencer journals, seals, anchors into the fake
  chain, publishes to a temp dir; replica consumes it and (a) reproduces every
  anchored root, (b) detects a single flipped byte in each artefact, (c)
  produces a holder exit proof that `verify_record` accepts, (d) survives a
  sequencer restart with lineage intact, (e) refuses a forged second anchor.
- **Live, testnet:** the Hetzner sequencer anchors into real testnet Zcash via
  the existing shares; a replica on the Windows box (its Zebra is mainnet — for
  testnet it uses the Hetzner Zebra as a reader) verifies from genesis. Success
  = replica reports `verified_epoch == sequencer epoch - pending` and a
  `zyn-cli exit-proof` for a real account verifies with the sequencer stopped.

## 6. Order of work

1. A — journal + ledger persistence (+ F3, F5)              ← unblocks replay
2. B — anchor transaction, confirmation-gated acceptance      ← unblocks "on Zcash"
3. C — bundles + publisher; choose and stand up two mirrors
4. D — replica: scan, fetch, replay, verify, serve; F1, F2
5. E — holder tooling; Nap Wallet record+path retention
6. Live testnet run; RUNBOOK §4s "Verifying the chain"; DECISIONS §7c status update

## 7. Explicitly out of scope

Promotion / split-brain guard (piece 2). Distributed DKG and a replica holding
a share (piece 3). Proof-carrying settlement (§14e level 3). Anything that
changes swapvm semantics — this piece only records and republishes what the VM
already commits.
