# Signer federation (rung 2): a root deposits release on is one k verifiers reproduced

Status: approved 8 Sep 2026. Turns the anchor certificate from a stub into the
thing that gates settlement, and makes each signer a verifier rather than a
rubber stamp.

## The property

A deposit becomes spendable only after **k of n independent replicas each
replayed the epoch and reproduced its anchored root**. A sequencer that
anchors a forged root puts it on Zcash and gets no signatures: the deposits
behind it never release, and every replica reports `ROOT UNREPRODUCIBLE`. The
sequencer can stall the chain (a liveness failure everyone sees); it cannot
make the chain agree to a state its own inputs do not produce.

## What already exists (and is only wired here)

`anchor::SignerSet` (ed25519 keys, threshold > half), `anchor::Certificate`
(`add`, `weight`, `verify` against the anchor id), `Node::with_signers`,
`Node::submit_certificate`, `Node::confirm_anchor(id, cert, now)` whose
`commit_anchor` **already refuses to settle without a certificate that clears
the set**. The replica already reproduces every anchored root. Rung 1's
`decode_frame`/replay and rung 0's bundle/mirror are the transport.

## Mechanism

**A signer is a replica with a key.** `zyn-replica` gains `ZYN_SIGNER_KEY`
(ed25519 seed) and `ZYN_ENDORSE_TO` (the sequencer's address). Its verify loop
is unchanged; when it commits an anchor (reproduced the root, checked lineage
and the published leaves), it signs the anchor's **id** — the 32 bytes
`Certificate::endorses` checks — and POSTs the endorsement to the sequencer.
It signs only what it verified: no bundle, no signature.

**The endorsement channel.** New `OP_ENDORSE`: `anchor_id[32] ‖ signer[32] ‖
sig[64]`. The sequencer verifies the signer is in its set and the signature
opens the id (`Certificate::endorses`), then folds it into the pending
certificate for that anchor (`Node::submit_certificate` / a new
`Node::add_endorsement`). Unknown signers and bad signatures are dropped. No
authority is granted over the socket — an endorsement is only ever counted
against a configured set.

**Settlement waits for the certificate.** The sequencer is configured with
`ZYN_SIGNER_SET` (`pubkey_hex,pubkey_hex,…`) and `ZYN_SIGNER_THRESHOLD`.
`Node::with_signers` then makes `confirm_anchor` require a clearing
certificate. `AnchorSettler`, which today confirms with `None` at Zcash depth,
instead:
- broadcasts the anchor (unchanged);
- at Zcash depth, if a set is configured, confirms **only once the pending
  certificate clears** (`Node::add_endorsement` gathered enough) — passing
  `Some(cert)`; otherwise it waits, and alerts `AWAITING_SIGNATURES` if the
  wait exceeds a bound;
- with no set configured, confirms with `None` exactly as now (the V0 path).

The anchor is on Zcash the whole time; the certificate gates only *release*.

**The bundle carries the certificate.** `certificate.bin` is already
`node.ledger().certificates()`; in signer mode it holds the real k-of-n
certificate, so anyone — not only the signers — can check an anchor was
endorsed. A **verifying replica configured with the same set**
(`ZYN_SIGNER_SET` on `zyn-replica`) refuses to advance past an anchor whose
certificate does not clear it: `Halt::Unendorsed`. This closes the stub left
in rung 0 ("certificate must clear the configured signer set if one is
configured (V0: none)").

## What this is and is not

It is **verifying** federation: k parties independently reproduce the root.
It is **not** proof-carrying settlement (rung 4) — a verifier still *runs* the
VM rather than checking a proof — and it does **not** change vault custody:
the FROST key that signs the Zcash payout is a separate thing, still a
threshold that needs independent share-holders (its own operational gap). What
rung 2 removes is the sequencer's power to make Zyn *act* on a root nobody
else reproduced.

## The honest limit of the demo

The mechanism is k-of-n independent keys and independent replay. The testnet
demo runs the signer processes on two machines (Mac, Hetzner) sharing one
testnet Zebra, so block-source independence is partial; key and replay
independence are real. Production needs n operators on n infrastructures —
that is the operational gap rung 2 does not close by itself, and the docs say
so.

## Tests

`anchor`: an endorsement over the id verifies; one from outside the set, or a
bad signature, carries no weight; the certificate clears only at threshold.
`node`: with a set, `confirm_anchor(None)` refuses; endorsements accumulate;
at threshold `confirm_anchor` settles and releases; below it, deposits stay
pending. `rpc`: `OP_ENDORSE` folds a valid endorsement and drops an invalid
one; it grants no authority. `replica`: a signer-replica signs an id it
reproduced and refuses one it could not; a replica with a set halts
`Unendorsed` on an anchor whose certificate is short, and does not on a full
one. Live: three signer keys, threshold two; watch a deposit stay pending
until two endorsements arrive, then release; stop one signer and watch the
chain keep settling on the remaining two; feed a fourth key not in the set and
watch its endorsement ignored.
