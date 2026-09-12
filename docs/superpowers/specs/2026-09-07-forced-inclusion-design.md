# Forced inclusion via Zcash memos (rung 1)

Status: approved 7 Sep 2026. Removes the sequencer's power to censor; adds the
replay index the design always assumed.

## The property

A user the sequencer ignores can still get an intent applied, and a sequencer
that keeps ignoring it is detectably in breach — detectable by the replica,
without the sequencer's cooperation.

## Mechanism

**Wire.** A third memo tag, `ZYF` (deposit `ZYN`, anchor `ZYA`):

```
"ZYF" ‖ 0x01 ‖ len u16 ‖ frame            zero-padded to 512
frame = vm_id[32] ‖ valid_until_epoch u64 ‖ scheme u8 ‖ key[32] ‖ sig[64] ‖ encode_intent(intent)
```

The frame is byte-for-byte what `OP_SUBMIT` carries after the op and chain id.
Ed25519 and Solana schemes only in v1 (the account derives from the key without
the secp256k1 recovery the custody crate does not have). Oversize frames are
refused at the wallet with a clear message; no chunking.

**The note.** A `ZYF` memo rides a small note (10,000 zat) to the vault. Its
value is credited to the signer's account as an ordinary deposit — the account
is derived from the frame's key in the scanner, so the note enters the normal
attest-then-credit path. Being censored costs the Zcash fee and nothing else.

**The sequencer.** The vault scanner reports forced sightings alongside
deposits (`ScannedAction.forced`, `Observed.forced()`). The bridge, after each
watcher pass, takes every sighting at `confirmations` depth, authorises the
frame through the **same** path as RPC (`authorize_intent` — the user's own
signature, never operator authority), consults the replay index, and submits.
Accepted or rejected by the VM, it is applied and journaled: both are inclusion.
Handled txids are persisted so a sighting is processed once.

**Replay index** (`zyn::replay`, std). `sha256(scheme ‖ signature) → expiry
epoch`, persisted beside the journal, pruned as epochs pass. Consulted on
`OP_SUBMIT`, `OP_SUBMIT_MULTI` and the forced path. Closes a pre-existing gap:
until now a captured signed intent could be applied again anywhere inside its
validity window.

**The rule the replica enforces.** A forced intent seen on Zcash at height H
must be applied by the anchor whose Zcash height is ≥ H + GRACE (20 blocks).
The replica scans `ZYF` memos with the viewing key, verifies each frame's
signature (junk is ignored), and keeps the pending set. During replay it
records the hash of every applied intent's bytes; when an anchor at height A
is verified, every pending sighting with `H + GRACE ≤ A` must have its intent
hash among what was applied since the sighting. Otherwise **`CENSORSHIP`**: an
alert, a status counter, a persisted record — never a halt, because the roots
are still correct; the operator is refusing service, not forging state.

**The holder.** Nap Wallet: every Zyn action gains "force via Zcash" — it
builds the identical frame, wraps it, and pays the vault 10,000 zat with the
memo. `zyn-cli force-memo` prints the memo and the vault address for any other
wallet.

## Out of scope

EVM-signed forced intents; multi-memo frames; forced intents as a general
message channel (only what RPC accepts).

## Tests

Memo tags are mutually exclusive (three tags). Scanner: a forced note credits
the derived account and reports the frame. Replay index: fresh once, then
refused; pruned by expiry. Bridge: a forced sighting at depth is applied once;
a second sighting of the same signature is not; RPC-then-memo applies once.
Replica: a due forced intent absent from the journal raises `CENSORSHIP`; one
present does not; the record survives a restart. Live: force an intent the
sequencer is told to ignore (`ZYN_IGNORE_FORCED=1`, testnet only); watch the
replica call it out; turn the flag off; watch it clear.
