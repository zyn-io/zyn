# Zyn

**Zyn is an application execution layer built for Zcash.** It runs deterministic
application state machines, batches their activity, and anchors state
commitments to Zcash.

> ZEC has private money. Zyn gives it somewhere to do things.

**Cave** is the first consumer application powered by Zyn — a ZEC-settled object
market. **ZynZap / SwapVM** was Zyn's first execution environment; since 23 Sep
2026 both live chains (testnet 212, mainnet 26460) run the **object ledger**
(`objectvm`), where value is held as notes rather than balances, and a
market in the VM — batch-cleared swaps, pools and one-price token launches —
whose participants are hidden from the public and visible to the operator
(`DECISIONS.md` §137).

## The trust model, up front

Read this before the rest. It decides whether anything else matters to you.

- **Zyn is operator-sequenced.** One sequencer orders transactions. There are no
  fraud proofs and no validity proofs.
- **Bridged ZEC is backed by ZEC in a 2-of-3 threshold vault operated by the Zyn
  team.** Withdrawals depend on those custodians.
- **Zcash records deposits, withdrawals and anchor commitments. It does not
  validate Zyn state transitions.** Built *for* Zcash, not *of* it: Zcash does
  not recognise Zyn, and Zyn does not inherit Zcash's security.
- **Account reads require the holder's signature**, so a stranger cannot look up
  a balance. The operator and its verifiers can see activity in the journal.

Present-tense facts, not a roadmap. Where this is heading is `DECISIONS.md`
§84 and §103.

## What is actually strong

Not adjectives — things a custodian cannot produce:

| | |
|---|---|
| `zynzapd/src/backing.rs` | A signer refuses to endorse a credit its **own** Zcash node cannot see. Proof of reserves by construction, not an attestation letter |
| `zynzapd/src/exitproof.rs` | A holder keeps evidence of what they held, checkable against an anchored root without our cooperation. **Evidence, not a withdrawal** — releasing the backing still needs the custodians |
| Forced inclusion (`ZYF` memos) | An intent published through Zcash that the sequencer does not apply becomes a recorded refusal. Censorship becomes detectable, not impossible |
| Per-signer liveness + backing requirement | A threshold that clears cannot hide a signer who verified nothing |

Before writing any public copy, read [`docs/MESSAGING.md`](docs/MESSAGING.md):
it lists what may and may not be claimed, and why.

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
| `swapvm` | the ZynZap AMM VM (retired chains; kept for their history) |
| `objectvm` | the object ledger both live chains run: notes, listings, exits, and the private market (`market.rs`) |
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
