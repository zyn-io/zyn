# Zyn

The shielded microchain: VM, addressing, anchors, bridge, custody, and the
sequencer daemon.

## Layout

One Cargo workspace, because it is one release unit. `zynzapd` has path
dependencies on every other crate and a `STATE_VERSION` bump has to move them
together — splitting them into separate repos would make each cross-crate change
two PRs and a rev bump. C18 and §59.5 are what that lesson cost.

| crate | what it is |
|---|---|
| `zyn-vm` | the VM spec: commitments, derived addresses, collections, manifests, auth |
| `zyn-bridge` | vault and bindings |
| `zyn-custody` | shielded and transparent custody |
| `swapvm` | the ZynZap AMM VM |
| `zyn` | anchors, ledger, epochs |
| `zynzapd` | the sequencer daemon, replica and signer |

`DECISIONS.md`, `BACKLOG.md` and `RUNBOOK.md` are the operational record. Read
DECISIONS before changing anything committed — most of it exists because
something failed first.

## Status: not a release baseline

This repo was seeded from the working tree, and that tree included in-flight
work from `zyn-io/nap-wallet` PR #19 (*Recover Zyn from one phrase and ship Nap
Web*). `zynzapd/src/restore.rs` and the recovery changes in `wallet.rs` and
`app.rs` are byte-identical to that **draft**, whose own description says:

> Automatic legacy migration is disabled before any transfer or key write. The
> initial implementation switched accounts without verified settlement or
> resumable recovery; it is not suitable for release.

Treat the one-phrase recovery and legacy-key migration path as unfinished until
that PR is completed and reviewed. Everything else — the VM, addressing,
manifests, anchors, replica and signer — is the tested baseline: **748 tests
pass** in this tree standalone.

## Known duplication

`zyn-io/nap-wallet` is a public monorepo that still contains a copy of every
crate here, at an older revision (its `derive.rs` predates the version fix in
§66 and it has no `manifest.rs`). Two sources of truth for a chain's state
encoding is how two builds of the same sequencer come to disagree about a root.
Settled on 14 Sep (§76): this repo stays private, so `nap-wallet` keeps its own
copy rather than taking a git dependency — a public wallet that needs a private
repo to build cannot be audited or reproduced by anyone outside the org. The
copy must be kept current: a wallet built from a stale one cannot talk to the
chain at all.
