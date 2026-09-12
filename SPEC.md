# The Zyn Microchain VM Specification

**Status:** v1 draft. Normative for `zyn-vm` 0.1.
**Conformance:** `zyn_vm::conformance::check` — executable, and it fails a
broken VM (proven by `swapvm::microchain::tests::the_conformance_suite_catches_a_broken_vm`).

---

## 0. Why this document exists

Zyn is the execution layer under ZynZap. It will eventually be the execution
layer under other things. This document defines the boundary between the two so
that the second application arrives as *a new crate*, not as a rewrite of the
first.

The rule the layering is built to satisfy:

> **The infrastructure may not know what the application is for.**

Sequencing, epoch policy, anchoring to Zcash, threshold settlement, the exit
hatch and crash recovery are the same problem for a swap, an auction and a
game. They are solved once, above the spec. Pricing, fees, pools and tokens are
ZynZap's, and live below it.

```text
  zyn-vm    the specification: what any Zyn VM must be
     |
     +-- swapvm    ZynZap — application #1
     +-- <yours>   a VM Zyn has never seen
     |
  zyn           the microchain: sequencing, anchoring, exits, recovery
                generic over zyn-vm; does not depend on swapvm at all
```

The last line is enforced by Cargo, not by discipline: `zyn` lists `swapvm`
only as a dev-dependency. If the infrastructure could name a swap, it would
eventually reach for one.

---

## 1. The model

A Zyn microchain is a deterministic state machine driven by an ordered stream of
**intents**, producing **receipts**, committing to a **state root**, sealing
**epochs**, and anchoring a small number of those epochs to Zcash.

```text
   intents ──(seq)──> VM ──> receipts
                       │
                       ├──> state root ──┐
                       └──> checkpoint ──┴──> anchor ──> Zcash
```

Zcash keeps custody and final settlement. Zyn keeps the interaction. The reason
the arrangement pays for itself is compression: thousands of microchain actions
per Zcash transaction, which is what makes a small trade worth making at all.

---

## 2. Normative rules

Each rule states the requirement and the failure it prevents. Rules marked
**[checked]** are verified mechanically by the conformance suite.

### S1 — Determinism **[partially checked]**

`apply` MUST be a pure function of `(state, seq, intent)`. No I/O, no clock, no
randomness, no threads, no floating point, no iteration over an unordered map.

Two nodes given the same intents in the same order MUST reach byte-identical
state roots, on any machine, forever.

*Prevents:* a chain whose nodes cannot agree, and a proof that attests to an
execution nobody else can reproduce.

*Checked:* the suite runs the same script twice and compares roots and
encodings, which catches the ordinary cause — an unordered map in the encoding.
It cannot establish purity in general.

### S2 — A contiguous sequence space **[checked]**

An intent applies only at `state.seq() + 1`. Any other sequence number is
refused and changes **nothing at all** — not the state, not the sequence, not
the intent commitment.

*Prevents:* a fork from a lost message, and an intent that was never in the
history appearing in the epoch's commitment.

### S3 — Rejections are events **[checked]**

An intent that breaks a rule produces a rejection receipt and leaves application
state untouched, but still consumes its sequence number and still folds into the
intent commitment.

*Prevents:* a replay that diverges because it skipped what the original refused.

### S4 — Outcomes are emitted, never accepted

The VM computes prices, fills and allocations. It MUST NOT accept them as input.

*Prevents:* the failure that hollows out a proof. If a sequencer could submit a
fill, a proof over the VM would attest that bookkeeping was applied faithfully
while saying nothing about whether the pricing was honest — which is the entire
property the proof exists to establish.

### S5 — Total commitment **[checked]**

Every field that can affect a future transition MUST be committed by
`sections()`.

*Prevents:* two nodes disagreeing about a field neither is hashing.

### S6 — Section layout **[checked]**

The state root is a Merkle root over an ordered list of section commitments:

| Index | Section | Owner |
|---|---|---|
| 0 | header | spec |
| 1 | accounts | spec |
| 2+ | application-defined | you |

Section 1's leaves are the holder records the exit hatch opens. Account ids are
32 bytes and strictly ascending. `account_record(id)` MUST hash to that holder's
leaf under `leaf_of_record`.

*Prevents:* every application inventing — and getting wrong — the one path its
users' funds depend on. Fixing the layout is what lets one settlement verifier,
one light client and one escape hatch serve every VM.

### S7 — Sealing **[checked]**

`seal_intent()` is an ordinary intent: it takes a sequence number and folds into
the commitment it closes. The `Checkpoint` it produces commits the root **as of
the seal**, before any epoch advance. `as_sealed(cp)` MUST recover that view for
as long as the accounts section has not moved, and MUST return `None` rather
than a view that does not reproduce `cp.state_root`.

*Prevents:* the failure this codebase actually hit. An anchor carries the sealed
root; the chain keeps executing the moment the epoch turns over; a snapshot
taken afterwards opens a root Zcash never saw, and every exit proof built from
it fails. A VM that cannot reproduce its own sealed view cannot publish a usable
escape hatch.

### S8 — Carriable state **[checked]**

`encode`/`decode` MUST round-trip to the same state root, and a decoded state
MUST continue execution identically.

*Prevents:* a restarted node silently diverging from one that never stopped.

### S9 — Checked arithmetic

Every operation is checked. Arithmetic that cannot be completed is reported
through `unprovable`, and the batch containing it is discarded whole.

*Prevents:* a value engine that mints money by wrapping, and a half-applied
intent reaching a committed root.

### S10 — No panics on input **[partially checked]**

A VM MUST NOT abort on bytes it did not choose. Decoding returns a value.

*Prevents:* a node an attacker stops by sending it eleven bytes.

*Checked:* the suite feeds truncated encodings and requires refusal.

### S11 — Conservation **[checked]**

A VM MUST declare what it conserves and be able to check it —
`MicrochainVm::conserved`. `zvm::apply_batch` checks it against a scratch copy
before the new state is allowed to become the real one, and discards the whole
batch if it fails.

*Prevents:* a committed root that asserts value was created. Every other rule
here protects the *process* — that two nodes agree, that a signature is narrow,
that a decoder does not abort. This one protects the *result*, and it is the
only rule that can catch a bug nobody anticipated: an arithmetic slip in a
handler no reviewer looked at still has to balance at the end of the batch.

*Checked:* enforced in `apply_batch`, not merely in tests. It began life as a
`check_invariants` that only tests called, which meant nothing stopped a live
node committing a root that minted money.

### S12 — Authorised intents carry a domain **[checked]**

What a user signs MUST bind the intent to a chain id, a program identity, and a
validity window — `zyn_vm::auth::signing_payload`. A VM MUST NOT accept a bare
intent encoding as an authorisation.

*Prevents:* two replays. A bare intent encoding is identical on every microchain
running the same VM, so a signature taken on one authorises it on all of them —
the failure EIP-712's domain separator exists for. And a sequence number stops
an intent executing twice but not *late*: without an expiry, a signed swap can
be held and submitted a week later at whatever price suits whoever held it,
which is ordinary sequencer MEV rather than a hypothetical.

Validity is expressed in **epochs**, not wall-clock time. A VM has no clock and
must not acquire one (**S1**), but epochs advance under a policy that already
carries a time failsafe — so this is Solana's `recent_blockhash` window,
deterministic enough to replay.

*Checked:* `zyn/tests/wallets.rs` signs a real swap, then shows that the same
signature resolves to a different account on a second chain and is refused
outright once its window has closed — the envelope being checked before any
curve arithmetic, so an unauthenticated submitter cannot choose how much work
the sequencer does.

### S14 — Signing is not entitlement **[checked]**

A VM MUST declare which accounts an intent requires the authorisation of —
`MicrochainVm::intent_authorities`. A sequencer MUST refuse an intent unless
every declared account has signed it. The default is empty, which means no user
signature can authorise the intent at all.

*Prevents:* the quietest theft available. Verifying a signature establishes
*who signed*; it says nothing about *what they may touch*. A perfectly valid
signature over an intent naming somebody else's account is, without this rule, a
perfectly valid authorisation to drain it — and nothing fails, no test goes red,
no log line appears, because every cryptographic check passed.

A list rather than one account, because some intents are two agreements: a
settled trade between a maker and a taker is one intent, and letting either side
alone authorise it lets either side alone impose it.

*Checked:* `zyn/tests/entitlement.rs` signs an intent over another account's
balance, asserts the signature itself verifies, and asserts it authorises
nothing. Also that one side cannot settle a two-party trade, that an operator
intent is unreachable by any user signature, and that a credential entitling
nothing is refused rather than ignored.

### S13 — An intent describes itself to a wallet **[checked]**

A VM MUST be able to render an intent as named, typed fields —
`MicrochainVm::typed_intent`. Every field that changes the outcome MUST appear.
The default renders an opaque hash, which is honest about a VM that declines to
describe itself; a VM meant for people MUST override it.

*Prevents:* blind signing, and what blind signing enables. A wallet displays
exactly these fields and nothing else, so this is the VM's user interface at the
one moment that decides anything. If a swap's route is not in the signature, a
compromised front end can reroute a good trade through a pool it owns without
invalidating what the user approved — and the user, shown a hex blob, had no way
to notice.

Field names travel into signatures already given. Renaming one is a version
bump, not a cosmetic change.

*Checked:* `swapvm::typed` asserts that rerouting a swap and weakening a
slippage bound both change what was signed, and `zyn::verify` reproduces the
EIP-712 specification's own worked example — recovering the address that
specification's own signature was made by, which is the only test in the tree
that proves the encoding matches the rest of the world's rather than merely
matching itself.

---

## 2b. Non-fungibles: no special primitive

There is no NFT type in this specification, and there should not be one.

An ERC is an interface *between contracts*; Zyn has no contracts calling each
other, so there is nothing to standardise in that sense. Two of the three VMs
worth learning from reached the same conclusion by different routes: a Solana
NFT is an SPL mint with supply 1 and zero decimals, with "NFT-ness" living in
Metaplex as a PDA-addressed convention; a Sui NFT is any owned object with
`key`, with fungibility emergent from whether a type can merge and split.
Ethereum's dedicated ERC-721 bought a fixed interface and, with it, an approval
surface and `tokenURI` link rot.

What does need standardising is the **state shape**, because the settlement
verifier and the exit hatch have to understand it. Three tiers, by scale:

| Items | Shape | Cost |
|---|---|---|
| a few | indivisible asset (`TokenInfo::unit == ONE`) | one balance entry, in the account leaf |
| many | compressed collection (`zyn_vm::collection`) | **32 bytes of state at any size** |
| — | one asset entry per item | never: linear state growth, paid in every proof forever |

The middle tier is Solana's state-compression answer, built from machinery Zyn
already had: sections of Merkle trees, plus a data-availability layer that
publishes leaves so a holder can prove one without the sequencer. A collection
is that construction pointed at items rather than accounts. A transfer is a
single leaf change, so a new root costs `log(n)` hashes rather than a rebuild.

Two rules that are normative:

- **Metadata is committed by hash, never stored.** A leaf carries `H(content)`;
  the bytes live in data availability or off-chain. ERC-721 stores a mutable
  pointer to a resource that can change or vanish under the token; a content
  hash cannot.
- **An item leaf binds its collection.** An item proved under one collection's
  root must not verify under another's — the same binding a signature needs to a
  chain id (**S12**).

The cost, stated plainly: item ownership lives in the collection tree rather
than in an account's balances, so *"I own item X"* and *"I hold N ZEC.zy"* are two
different proof paths, and a wallet needs the published leaves to build the
first. That is the trade Solana made — it is why compressed NFTs need an
indexer — and it is worth paying only above the size where the first tier stops
fitting.

---

## 3. What a VM author writes

Implement `zyn_vm::spec::MicrochainVm`. The obligations are: a transition, a
section list, a seal, a codec, and holder records. Everything else is derived.

`Provable` is blanket-implemented, so the exit path — leaf, inclusion proof,
verification against an anchored root — comes for free from S6 alone.

Then, in your own test suite:

```rust
zyn_vm::conformance::assert_conforms(Fixture {
    state:    /* a chain with history */,
    accepted: /* an intent that will be accepted */,
    rejected: /* an intent that will be rejected */,
});
```

`zyn/tests/second_app.rs` is a worked example: a ~200-line counting VM with no
swap, no fees and no tokens, which gets sequencing, epoch policy, anchoring,
threshold settlement, exits and recovery without `zyn` changing.

---

## 4. What the infrastructure provides

| Concern | Where | Application-specific? |
|---|---|---|
| Sequencing, epoch policy | `zyn::node` | no |
| Compression ratio, unit economics | `zyn::epoch` | no |
| Zcash anchor, lineage, fork refusal | `zyn::anchor` | no |
| Threshold custody | `zyn::anchor::SignerSet` | no |
| Exit hatch | `zyn::da` | no |
| Crash recovery | `zyn::store` | no |
| Pricing, fees, revenue | the application | **yes** |

Revenue is the boundary case worth naming. A fee is an application concept — an
auction has a clearing spread, a game has nothing — so `Revenue` lives in
`swapvm`, not in `zyn`. The microchain measures compression and settlement cost,
which are true of every VM.

---

## 5. Deliberately out of scope for v1

- **Cross-VM composition.** Two microchains cannot call each other. Atomicity
  stops at one VM's transition, which is why ZynZap holds all its pools in one
  execution environment rather than one per pair.
- **Permissionless deployment.** The spec is what a third party would implement;
  the process for *admitting* one — registration, resource limits, whether they
  share a settlement vault — is a later document, and a governance question more
  than a technical one.
- **Proof-carrying settlement.** `swapvm::guest` is shaped for it and the crate
  builds for `riscv32im-unknown-none-elf`, but V1 settles with threshold
  signatures. The spec is written so that swapping one for the other does not
  touch an application.

---

## 6. Open questions

1. **Shared or isolated custody.** If two applications settle through the same
   threshold vault, a bug in one reaches the other's ZEC. Isolation is safer and
   fragments liquidity. Unresolved, and it should be resolved before the second
   application ships rather than after.
2. **Resource limits.** `Params` is enforced per VM, but nothing bounds how much
   work one `apply` may do. A permissionless deployment needs a metering story.
3. **Section 1 semantics.** The spec fixes that section 1 holds holder records
   and that a record hashes to its leaf. It does not say what a record *means* —
   so settlement can verify inclusion but not solvency. For ZynZap the backing
   invariant closes that gap inside the VM; a general answer does not exist yet.
