# Forced inclusion — Implementation Plan

> Executed inline (no subagents in this session). Not a git repo: no commits; each task ends green.

**Goal:** a memo-carried signed intent the sequencer must apply, a replay index, and a replica that reports censorship.
**Spec:** `docs/superpowers/specs/2026-09-07-forced-inclusion-design.md`

### Task 1 — memo tag + frame codec (`zyn-custody/src/memo.rs`)
`FORCED_TAG = b"ZYF"`, `encode_forced(frame) -> Option<[u8;512]>` (None if > 506 bytes), `forced_frame(memo) -> Option<&[u8]>`; `is_anchor`/`decode` reject it; tests for three-way exclusion. `forced_account(frame) -> Option<AccountId>` reads scheme+key (`zyn_vm::auth::account_of`), None for EVM.

### Task 2 — scanner (`zyn-custody/src/shielded.rs`)
`ScannedAction.forced: Option<Vec<u8>>`; `take()` on a forced memo: deposit to the derived account **and** `forced = Some(frame)`; `classify` treats it as a deposit. `pub struct ForcedSighting { txid, height, amount: Fixed, frame: Vec<u8> }`; `Observed.forced: Vec<ForcedSighting>` + `forced()`; `feed_block` and `observe` collect them; `Scanner::forced(zebra, from, to)` for the replica. Synthetic test like the anchor one.

### Task 3 — replay index (`zyn/src/replay.rs`, std)
`ReplayIndex { seen: BTreeMap<[u8;32], u64> }`, `key(scheme: u8, sig: &[u8]) -> [u8;32]`, `fresh(&mut self, key, expiry) -> bool`, `prune(epoch)`, `load/save(dir, chain_id)` (`replay-<id>.idx`). Tests. `rpc::Server.replay: Arc<Mutex<ReplayIndex>>`; `submit`/`submit_multi` refuse a seen signature with "already applied"; saved with the state.

### Task 4 — bridge forced path (`zynzapd/src/bridge.rs`)
`pub fn apply_forced(node: &Shared, replay: &Arc<Mutex<ReplayIndex>>, chain_id, sightings: &[ForcedSighting], tip, confirmations, seen: &mut BTreeSet<[u8;32]>, now) -> Result<usize, String>`: for each sighting at depth, not in `seen`: decode frame (shared `rpc::decode_frame(chain_id, &[u8]) -> Result<(Vec<Credential>, Authorization, Intent), String>` refactored out of `submit`), `authorize_intent`, replay `fresh`, `n.submit`; `seen` persisted in `forced-<chain>-<asset>.seen`. Hooked after the watcher pass for the shielded source. `ZYN_IGNORE_FORCED=1` (testnet only) skips submission — the lever for the live test. Integration test `zynzapd/tests/forced.rs` with a fake `Observed`.

### Task 5 — replica censorship check (`zynzapd/src/replica.rs`, `zyn-replica`)
`Replica.pending_forced: Vec<ForcedPending { height, txid, intent_hash: [u8;32], expiry }>`, persisted `forced-pending`; `Replica.censored: Vec<Censored { height, txid, due_at }>`, persisted; `note_forced(sightings, chain_id)` verifies frames (`rpc::decode_frame` + `authorize_intent` against the replica's state is too strict — verify signature only via `zyn::verify::signer_of`), pushes pending. During `verify_one`, hash every replayed record (`sha256(intent bytes)`) into `applied: BTreeSet` while pending non-empty; after commit, resolve pending with `height + GRACE <= anchor height`. `GRACE = 20`. Status appends `forced_pending u32, censored u32`. Alert `CENSORSHIP`. Tests in `tests/replica.rs`.

### Task 6 — wallet + CLI
`client::frame_submission(k, chain, now_epoch, intent) -> Vec<u8>` (refactor `submit` to use it); `Wallet::send_with_memo(to, zat, memo: [u8;512])`; app: `force: bool` on send/swap/withdraw/transfer requests → memo route, history kind `forced`; UI checkbox "force via Zcash (if the node ignores you)"; `zyn-cli force-memo <host:port> <chain> <key> transfer <to-hex> <asset> <amount>`.

### Task 7 — docs + live
RUNBOOK §4t, DECISIONS §33. Live: deploy with `ZYN_IGNORE_FORCED=1`, force a transfer from the Mac wallet, watch the replica report CENSORSHIP after 20 blocks, flip the flag, watch it clear.
