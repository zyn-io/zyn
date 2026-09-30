# Zyn — settled decisions

## Nap recovery/browser release addendum — 12 September 2026

Nap #15/#16 implementation and release gates are documented in
`docs/NAP-RELEASE.md` and `docs/NAP-ZYN-KEY-DERIVATION.md` and published in
nap-wallet PR #19. Zyn derivation is versioned HKDF from the full BIP-39 seed,
network-independent; existing random keys remain unchanged. Restore uses a
rollback journal. Automatic legacy migration is disabled until verified,
resumable settlement and retained-authority recovery are implemented. The
browser target is pre-release, not an event-ready claim deployment.

The point of this file is to not have these arguments twice. Each entry records
what was decided, **why**, and — where it matters most — **what was rejected and
on what grounds**. A rejected option with a recorded reason is the thing that
stops a decision drifting back.

Companion documents: `SPEC.md` (normative VM rules), `ZVM.md` (execution
target), `VM-COMPARISON.md` (EVM / SVM / Sui Move analysis).

**Status legend:** ✅ settled and built · 🔨 in progress · 🔒 settled, not yet built · ❓ open

---

## 1. Layering

| # | Decision | Status |
|---|---|---|
| 1.1 | Four crates: `zyn-vm` (spec) → `zyn-bridge` (custody) → `swapvm` (ZynZap) + `zyn` (microchain) | ✅ |
| 1.4 | Custody is a **component**, not a VM | ✅ |
| 1.2 | `zyn` does **not** depend on `swapvm`. ZynZap is a dev-dependency only | ✅ |
| 1.3 | The spec is executable: `zyn_vm::conformance` | ✅ |

**1.4 — why "BridgeVM" cannot exist.** It is the natural instinct and it does
not work: a VM's state is its own, so an asset custodied by a separate bridge VM
could not be swapped inside ZynZap without cross-VM atomicity, which does not
exist and whose eventual form is deliberately *not* synchronous calls (§3.1,
§8.5). **Bridged balances have to live in the VM that trades them.**

What does not have to live there is the logic. Deposit indexing, exit timeouts,
dust limits and settlement grouping are identical for every VM holding bridged
value, so they are `zyn-bridge`: an application embeds a `Vault` per bridged
asset and a `PendingExit` per outstanding exit, and calls the rules from its own
handlers. ZynZap keeps what only it can do — moving its own balances — and owns
none of the custody reasoning. The component does not depend on `swapvm` and
contains no swap concepts; `cargo tree` and a grep both check it.

The same split as revenue (application) versus conservation (spec): the question
is always whether an auction or a game would need it too.

**1.2 is the load-bearing one.** If the infrastructure could name a swap it would
eventually reach for one, and the second application would arrive as a rewrite
of the first. Enforced by Cargo, not by discipline. `zyn/tests/second_app.rs`
proves it: a ~200-line counting VM gets sequencing, epoch policy, anchoring,
threshold settlement, exits and recovery with **zero** changes to `zyn`.

The conformance suite is proven to *fail* — `the_conformance_suite_catches_a_broken_vm`
feeds it a VM with mislabelled sections. A suite that cannot fail means nothing.

---

## 2. Execution target

| # | Decision | Status |
|---|---|---|
| 2.1 | **rv32im.** Rust first, TinyGo/Zig/C possible later | 🔒 |
| 2.2 | ZVM ABI: one entry point, bytes in → bytes out. No storage/network/clock/randomness | ✅ |
| 2.3 | The ABI lives in `zyn-vm`, so **one settlement verifier serves every application** | ✅ |
| 2.4 | Public output binds `vm_id = H(name, version)` | ✅ |
| 2.5 | Stage 0 compiled-in → Stage 1 rv32im ELF → Stage 2 permissionless. ZynZap does not move | 🔒 |

**Rejected — WASM.** The better *execution* choice and the worse *system*
choice. Proving it needs either a WASM zkVM (thin ecosystem) or two execution
paths that must agree about every edge of every application — the exact
"two implementations supposed to match" trap. Its determinism is also policy:
floats exist and must be disabled. On rv32im the `F`/`D` extensions are absent,
so S1's no-float rule is enforced by the *instruction set*.

**Rejected — Move.** New language (ruled out), no proving story, object model
doesn't map onto the section-based root.

**Rejected — EVM.** Out of MVP scope per plan §24; imports Solidity as the new
language.

**Rejected — forking SVM.** Costs exactly the two things that differentiate Zyn:
provability (no SBF zkVM ecosystem) and machine-enforced conservation (SVM has
no notion of what's conserved — hence manual account validation as its entire
attack surface). Also: Solana has no per-block state root usable for our exit
proofs, so we'd rebuild that anyway. *The developer-ecosystem argument survives
without the runtime:* make the SDK Anchor-shaped.

**If native speed ever binds:** JIT the rv32im interpreter. Do **not** add WASM
as a second path.

---

## 3. Security posture

| # | Decision | Status |
|---|---|---|
| 3.1 | **No callback surface.** No cross-contract calls, therefore no reentrancy | ✅ |
| 3.2 | **S11 conservation** enforced by `transition`, batch discarded if it fails | ✅ |
| 3.3 | Extensions are **declarative, never imperative** | ✅ |
| 3.4 | No approvals / allowances | ✅ |
| 3.5 | No per-token `decimals` — WAD everywhere | ✅ |
| 3.6 | S12: signed intents bind chain id, `vm_id`, and an epoch expiry | 🔒 |

**3.2 — Move's guarantee without Move's language.** Linear types make "forgot to
decrement the balance" a type error. We can't have those without a new language,
so the guarantee moved from the type system into the specification: every VM
declares what it conserves, and `transition` checks it after every batch.
Coarser than Move (per batch, not per expression); unlike an audit, not
optional. *Found during this work:* `check_invariants` was exhaustive and called
**only from tests** — nothing stopped a committed root asserting value had been
created.

**3.3 — rejected: Uniswap v4 hooks.** A v4 hook is arbitrary contract code called
mid-swap. That reintroduces reentrancy, makes work unbounded (metering becomes
hard instead of a cycle count), risks determinism, and lets value move in ways
S11 cannot account for. The declarative form — a constraint the pool *declares*
and the VM enforces — costs one comparison and none of that. `Pool::min_in0/1`
is the worked example. **Apply this test to every future v4-style request: find
the declarative form, or decline it.**

**3.4/3.5 — what EVM teaches by omission.** ERCs are interfaces *between
contracts*; Zyn has none, so most don't apply. Their failure modes still do, and
we're clean on all of them: no infinite-approval class, no decimals mixing,
rebasing/fee-on-transfer tokens impossible (VM-native tokens can't have custom
transfer logic), ERC-4626 inflation handled by locked liquidity. **Do not add
approvals "for compatibility."**

**3.6 — two replays.** A bare intent encoding is identical on every chain running
the same VM, so a signature authorises it everywhere (EIP-712's lesson). And a
sequence number stops an intent executing twice but not *late* — without expiry
a signed swap can be held and submitted a week later at whatever price suits the
holder. Expiry is in **epochs**, not wall-clock: Solana's `recent_blockhash`
window, made deterministic.

---

## 4. Addressing

| # | Decision | Status |
|---|---|---|
| 4.1 | PDA-style derived addresses: `zyn_vm::derive` | ✅ |
| 4.2 | **No bump seed** | ✅ |
| 4.3 | Seeds are length-prefixed | ✅ |
| 4.4 | Migrate `AssetId`/`PoolId` from `u32` to 32-byte derived addresses | ❓ |
| 4.5 | **One namespace per kind**: `native`, `bridged`, `asset`, `pool`, `lp`, `vault`, `collection`, `item`. Helpers for all of them, not just pools | ✅ |
| 4.6 | **A bridged asset is addressed by its origin *network*, not its chain family.** `zcash` and `zcash-test` are different assets because ZEC and TAZ are | ✅ |
| 4.7 | **A chain's own token carries the Zyn chain id as a seed**; a bridged asset does not. The scoping goes where the ambiguity is | ✅ |
| 4.8 | The genesis address table is **asserted in a test**, not documented in prose | ✅ |

**4.2** — Solana grinds an off-curve bump because addresses *are* ed25519 keys.
Zyn's `AccountId` is an opaque 32 bytes with no curve behind it, so there is
nothing to be off. One hash, no grinding, no canonical-bump confusion.

**4.3** — Solana concatenates seeds, so `["ab","c"]` and `["a","bc"]` share a
preimage, survivable only because seeds are length-capped. An inherited sharp
edge, not a chosen one.

**4.4 is open, and it is a one-way door.** Surface: 57 `AssetId` + 28 `PoolId`
occurrences across 6 files. Go **full width** if done — truncating to `u32`
reinvites grinding a collision with an existing pool, and a lookup table
reinvites the index-divergence problem the linear scan avoids. Half measures are
worse than either end. Better done before a frontend derives addresses one way
and the VM assigns them another.

---

## 5. Economics

| # | Decision | Status |
|---|---|---|
| 5.1 | **No rent.** A refundable/locked bond instead | ✅ |
| 5.2 | `CreateToken` is an atomic **launch**: token + ZEC.zy pool in one intent | ✅ |
| 5.3 | `Params::min_pool_xzec` of launch liquidity is **permanently locked** | ✅ |
| 5.4 | Items post a **refundable** bond (`MintItem` / `BurnItem`) | ✅ |
| 5.5 | Protocol fee is a share **of the existing fee**, defaulted to 0 | ✅ |
| 5.6 | Compression policy = `intents_per_epoch × epochs_per_anchor` | ✅ |

**5.1 — why rent was rejected.** Rent is denominated in the native asset and paid
to nobody, so a successful economy makes building on it more expensive while the
payment stays deadweight — the incentive points at the builder. A bond costs the
same capital and *deploys* it: as liquidity (5.3) or as a returnable deposit
(5.4). Same anti-spam pressure, no extraction.

**5.2 has three consequences worth keeping:** no token exists without a market;
spam is bounded by capital rather than fees; and **ZEC.zy becomes the routing hub
by construction**, so `max_hops = 2` stops being an assumption about
connectivity and becomes a property of how tokens come into existence.

**5.3 — a launch pool can never be fully unwrapped.** The lock is a fraction of
*shares*, so a fraction of the asset is permanently stranded. Fine for a
fungible token; it is why an item is **burned, not unwrapped** (6.5). Pinned as
`a_launched_pool_can_never_be_fully_unwrapped`.

**5.4 — the bond lives on `TokenInfo`, not in a vault account.** The VM checks no
signatures, so a vault would be drainable by a malicious sequencer emitting
`Transfer { from: vault }`. On the token entry there is **no address to transfer
from**. `BurnItem` is the only operation that makes state smaller — the storage
rebate, and what makes the bond a deposit rather than a fee.

**5.5** — the rail exists at zero because turning revenue on must be a parameter
change, not a change to how value moves. Adding it later would alter the swap
path, the state root, and every proof written against it. The cut comes out of
the pool's fee, never the trader's quote, so `k` stays non-decreasing.

**Honest limit:** bonds bound spam by *capital*, not absolutely. Someone with
10,000 ZEC.zy can launch 10,000 tokens — they just cannot get the bonds back.
`min_pool_xzec` is the single knob for how open the chain is.

### 5.7 — bridged pairs: the reference sets the **fee**, never the price 🔒

A pool of SOL against ZEC.zy is not the same object as a pool of a memecoin
against ZEC.zy, and treating it as one loses LPs money.

A constant-product curve is a **price-discovery mechanism**. For a memecoin with
no market anywhere else, discovery is exactly the service an LP provides, and
the fee is payment for it. For a bridged pair, price is already discovered
elsewhere and the pool is a slow mirror — every external move hands an
arbitrageur the difference. At 0.30% that is nowhere near covered:

| external move | arb takes | 0.30% collects | break-even fee |
|---|---|---|---|
| +10% | 2.38 ZEC.zy | 6.1% | 4.88% |
| +20% | 9.11 ZEC.zy | 3.1% | 9.54% |
| +50% | 50.51 ZEC.zy | 1.3% | 22.47% |

The break-even has a closed form: pushing a pool from `p` to `p·r` costs the
arbitrageur `x(√r − 1)` and returns exactly `√r − 1` of it as profit, so the fee
that recaptures it is **`√r − 1`**. `amm::divergence_fee` is that formula, and
the table above is the formula, not an estimate.

**The safety argument, and it is the whole decision.** A reference price may set
the **fee** and must never set the **quote**:

- *Oracle sets the price* → manipulate the oracle, drain the pool. Direct theft,
  and the most exploited pattern in DeFi.
- *Oracle sets the fee* → manipulate the oracle, fees are wrong. The curve still
  decides what a trader receives, so the worst case is bounded by construction
  rather than by how well the oracle is defended.

The plan chose a constant-product AMM partly *because* it needs no oracle (§8).
This keeps that property where it matters — nothing that moves value depends on
a reported price — while letting a reference improve LP economics where it does
not.

**The consequence found in practice (7 Sep 2026).** The fee is exactly the
arbitrage profit, so a pool sitting far from its market cannot be arbitraged
back: at a divergence of 2.39x the fee was 5,460 bps, and correcting the
price would have cost 0.297 SOL.zy to receive 0.208 SOL.zy of ZEC. That is
the formula working, not failing — but it means **a bridged pool must be
seeded at the market price**, because the fee that protects it from drift
also locks in whatever price it opens at.

The way out, when one is already wrong, is not a trade but a sequence: the
LPs remove their liquidity (which never moves the price), the price is then
corrected on the dust that remains — where the same percentage fee is a
trivial absolute amount — and the liquidity goes back at the corrected
ratio. Done on testnet pool 1, it took two swaps totalling 0.006 SOL.zy and
0.0001 ZEC.zy. Note the second swap: a fee is *left in the reserves*, so
correcting a price must account for the whole input, not the part that
reaches the curve — the first attempt overshot 11.3 to 16.1 by ignoring it.

A cap on the divergence fee would let ordinary arbitrage do this instead,
at the cost of handing arbitrageurs the difference. Not taken: seeding at
the market price is the discipline, and the procedure above is the repair.

**Still open:** where the reference comes from. Any answer is a new trust
assumption, and it is the *same* one as §10.7's deposit attestation — a signer
quorum reporting something the VM cannot verify. Worth solving once, for both.
`perpvm` is the precedent: a reported figure with a confidence, a staleness
window, and a band bounding how far it can pull the derived value.

**Not the long-term answer.** For bridged majors, a curve is the wrong venue
whatever the fee: market makers who hedge externally quote tighter than any
curve can, which is what §26E's CLOB-over-AMM is for. A divergence fee makes
CPMM survivable for bridged pairs, not optimal for them.

---

## 5b. Does any of this need zk?

Asked once, worth recording, because the answer is counter-intuitive in two
places.

### A validity proof does not help the bridge

`zyn-vm::zvm` proves: *starting from a state committing to `base_root`, applying
these intents yields `final_root`*. That is faithful **execution of the intents
it was given**. It says nothing about whether those intents were true.

A fabricated `CreditDeposit` is proved exactly as faithfully as a real one. The
prover has no more access to Zcash than the VM does, and for the same reason — a
program that could reach outside would not be deterministic (**S1**).

This is **S4** seen from the other side. S4 says outcomes must be computed here
so a proof covers them; its corollary is that anything the VM genuinely *cannot*
compute stays outside what a proof can reach, however good the prover gets.
Proofs replace the **certificate over a transition**, not the **attestation over
an input**. Attestation is handled where it has to be, by §10.12.

Reading a proof as covering deposits is an easy misreading and a serious one, so
it is now stated in `zvm`'s own docs rather than only here.

### The chain we are built on is the *hardest* one to prove deposits for

The usual assumption is that the home chain is the easy case. It is the
opposite, and for a reason that is the whole point of Zcash.

| Chain | Deposit proof in a guest | Why |
|---|---|---|
| **Zcash** | **hardest** | shielded deposits are not public. An SPV proof cannot see an amount or a recipient; only a viewing-key holder can. A proof would have to be *about a decrypted note*, under a key the signers hold jointly — threshold decryption plus a proof over it. Research-grade. |
| Bitcoin | tractable | transparent. PoW header chain plus a Merkle inclusion proof; done before. |
| Ethereum | tractable | sync-committee light clients in a zkVM are shipped work. |
| Solana | hard | consensus proofs over a large validator set are immature. |

So zk removes bridge trust for **Bitcoin and Ethereum first**, and for Zcash
last if ever — which inverts the order you would guess. It also matches §10.6:
proof-verifying settlement is a per-chain decision, not a global posture.

### Where zk does pay

- **Replacing the settlement certificate.** The guest already exists and builds
  for `riscv32im-unknown-none-elf`. This is the §17/§18 path: today's threshold
  signature becomes tomorrow's validity proof without an application changing.
- **Bitcoin and Ethereum deposit attestation**, per the table above — a genuine
  removal of trust, per chain, later.
- **Private ownership** (plan §12), which is a different use of zk entirely:
  shielding balances *on* Zyn rather than proving anything about another chain.
  Out of MVP scope and unrelated to the bridge.

### What we are not doing

Building a proof system now. V1 settles with threshold signatures, which is what
the plan says (§5), and the deposit gap is closed by quorum endorsement rather
than by cryptography we do not have. **The design is proof-ready; it is not
proof-dependent**, and nothing above the VM changes when a prover is attached.

---

## 6. Non-fungibles

| # | Decision | Status |
|---|---|---|
| 6.1 | **No special NFT primitive** — follow Solana/Sui, not ERC-721 | ✅ |
| 6.2 | Three tiers by scale (below) | ✅ |
| 6.3 | Metadata committed **by hash**, never stored | ✅ |
| 6.4 | The AMM refuses indivisible assets | ✅ |
| 6.5 | Items are **burned, not unwrapped** | ✅ |
| 6.6 | Resting offer book (bids with active/inactive + expiry) | ❓ |

**6.1** — an ERC is an interface *between contracts*, and Zyn has none. A Solana
NFT is an SPL mint with supply 1 and zero decimals; a Sui NFT is any owned object
with `key`. What *does* need standardising is the **state shape**, because the
settlement verifier and the exit hatch must understand it.

**6.2 — the tiers:**

| Items | Shape | State cost |
|---|---|---|
| a few | indivisible asset (`unit == ONE`) | one balance entry, in the account leaf |
| many | compressed collection (`zyn_vm::collection`) | **32 bytes at any size** |
| — | one asset entry per item | never — linear growth, paid in every proof forever |

**6.4 — the curve settles it, not policy.** A constant-product pool can never sell
an indivisible unit: every purchase returns a fraction, and the last unit costs
unbounded ZEC.zy (`out = 1` divides by zero). Measured: at 1 NFT / 100 ZEC.zy,
buying 0.999 costs 100,200 ZEC.zy. Sudoswap uses linear/exponential curves over an
item *list* for exactly this reason. **An NFT market is a different VM.**

**The pool is the fractionalisation vector — not supply, not minting.** A supply
of one, never re-minted, with an owner-built pool, still fractionalises: the LP
share is a *different, divisible* asset and the owner can send a third of it
away. Held directly with `unit == ONE`, the same asset cannot be split at all.
Both pinned as tests. So the two modes *are* the design:

```
held directly     → unit = 1 whole → atomic ownership, traded by AcceptOffer
wrapped in a pool → divisible LP   → fractional ownership, priced by the curve
```

**Rejected — the remove/reconstruct pipeline** (unwrap → escrow PDA → re-pool).
Two reasons: it severs the price link to the previous owner, who then neither
earns nor loses from the sale; and the unwrap step is impossible under 5.3.
Selling the position is one `Transfer`, and `AcceptOffer` makes payment and
delivery atomic — two transfers are two sequencing decisions, and a sequencer
that includes one and drops the other has taken one side's asset without
delivering the other's. (Same argument as routing being one intent.)

---

## 7. Findings that changed the design

Recorded because each was invisible until something forced it into the open.

| Finding | Consequence |
|---|---|
| `out_given_in` multiplied by `fee_keep` (9970) and never divided by BPS — every swap output computed from an input 10,000× too large | Fixed; the test constant was independently wrong too |
| `in_given_out`'s "+1 raw unit" was not a ceiling — at dust scale the fee truncation eats more than a unit | Replaced with an exact two-step ceiling; added `mul_div_ceil` |
| The sealed root was **not reconstructible** after an epoch advance — every DA snapshot opened a root Zcash never saw | `gross_volume` moved into `Checkpoint`; `as_sealed` added; the node captures the snapshot *inside* `seal()` |
| `check_invariants` was called **only from tests** | S11 — conservation enforced by `transition` |
| `credit(0)` left a zero entry in the balance map | Fixed at the source; the root now commits what an account *holds*, not what it was offered |

---

## 7b. The ZEC path: what is done and what is not

Worth stating plainly, because "the deposit rules are finished" and "the bridge
is safe" are different claims and the gap between them is all operational.

**Done, and enforced by code with tests:**

| | |
|---|---|
| Deposits | replay-proof index, transaction reference in the intent commitment, three independent ceilings (total, per-epoch, observed balance), unspendable until their epoch is anchored |
| Exits | dust floor per chain, request → pending → confirm, timeout and cancellation, oldest-paid-first batched settlement |
| Accounting | `supply == confirmed` per asset, enforced by `transition`, holds while deposits *and* exits are in flight |
| Settlement | anchors chain, forks refused, certificates carry verified ed25519 signatures, a configured node fails closed |
| Recovery | full state round-trip; a holder proves a balance against an anchored root with the sequencer gone |

**Built since, in `zyn-custody` — the operational half:**

| | |
|---|---|
| **Signer ceremony** | FROST distributed key generation over RedPallas. 7-of-10 tested end to end; no participant ever holds the key whole, and the ceremony refuses a threshold at or below half. |
| **Threshold signing** | Two-round FROST producing one ordinary Schnorr signature that verifies against the group key. Below threshold there is no signature — not a weak one. |
| **Deposit watcher** | Confirmation depth, reorg handling, per-transaction deduplication, dense indices, and the observe-then-credit order the VM requires. Chain access behind a trait, tested against a mock chain that reorganises. |

**Cryptography is integrated, never written.** `reddsa`'s FROST over RedPallas —
the Zcash Foundation's implementation of the scheme Orchard spend authorization
uses. Threshold signing fails silently and totally when it is got wrong, and a
hand-rolled version that passes its own tests is exactly what that failure looks
like. `zyn-custody` reimplements no part of it.

Two design points worth keeping:

- **`SigningSession::sign` takes `self` by value.** Signing twice from one
  nonce commitment leaks the share, and enough leaked shares is the key. The
  type system refuses the second call rather than trusting an operator not to
  retry.
- **A failed watcher poll makes no progress at all.** A partial scan that
  advanced its cursor would silently skip the heights it failed on, and those
  deposits would never be seen again.

**Still not doable in this repo:**

- **A live Zcash node.** `ChainView` is the seam; behind it a real deployment
  runs zebrad or zcashd, decrypting the vault's notes with the viewing key.
  Nothing here has spoken to a chain.
- **A full Orchard shielded spend.** The signature half is solved — the same
  crate provides the rerandomized path Orchard requires. Assembling the note,
  the proof and the bundle needs the Zcash transaction stack and testing against
  a real node.
- **Reorg *response*.** Detection exists in two places now — the watcher's
  `audit` names a credited deposit that vanished, and the VM refuses an
  attestation below what it has issued. What to *do* about it is governance.

**The honest summary: the logic is built and tested; nothing has touched a
chain.** Every rule enforceable deterministically is enforced in the VM, and
every operational decision around it — when a deposit is real, whether it has
been seen, who signed — is implemented and tested against mocks. What remains
is integration with a live Zcash node and the transaction-building stack, plus
the operational discipline of an actual key ceremony.

---

## 7c. What a node costs to run

Measured, not estimated — `cargo run --release --example loadtest`. Release
profile, so these include the checked arithmetic the VM is required to do.

| | |
|---|---|
| Throughput | **~690,000 swaps/sec**, 1.5 µs each, single-threaded |
| State | **64 bytes per account.** 50,000 accounts = 3 MiB |
| Sealing an epoch | 5 ms at 10k accounts, 27 ms at 50k — **O(accounts)** |
| Restart | 0.8 MiB state: encode 0.2 ms, decode 0.9 ms |
| Serving a withdrawal proof | **0.1 µs** from an index; 27 ms without one |

**The VM is not the bottleneck and will not be.** At 1.5 µs per swap, a single
core absorbs more flow than Zcash could ever settle. Anything that limits this
chain is I/O, networking, or the bridge — not execution.

**Sealing is the real cost, and it is `O(accounts)`.** At 27 ms per seal for
50,000 accounts, `intents_per_epoch` is not only a durability knob (§5.6) — set
it too low on a large chain and the node spends its time hashing. The pairing to
watch is epoch size against account count.

**The read path needed its own structure.** `SwapState::account_proof` rebuilds
the whole tree per call: 27 ms at 50,000 accounts, about forty requests a second
on a core — hopeless for a withdrawal endpoint. `AccountIndex`, built once when
an epoch seals, serves the same proofs in 0.1 µs.

Note *why* the first attempt at this did not work: caching the **leaves** gained
only 2.5×, because `merkle_proof` rebuilds every level on each call. The index
has to hold every level, and then a proof is one sibling read per level. Storing
the obvious thing was not enough, and the measurement is what showed it.

**Hardware, for a chain of ~50,000 accounts:** one modern core, a few hundred MiB
of RAM, and a small disk. This is not a machine problem. Sizing should be driven
by the Zcash node beside it, the RPC fan-out, and the data-availability
publishing — none of which is the VM.

**Not built:** the RPC server itself. `wire.rs` defines the opcodes
(`OP_STATUS`, `OP_ACCOUNT_PROOF`, `OP_SNAPSHOT`, `OP_QUOTE`, …) and the frame
encoding; nothing listens on a socket. `OP_QUOTE` in particular has no
implementation — a UI cannot currently price a route without submitting a swap,
and the plan's own mock shows a price-impact figure the VM does not compute.

---

## 7d. Where the servers go

Sizing follows from §7c, and the answer is not what people expect: **the
microchain is nearly free to host.** One core, a few hundred MiB, 3 MiB of state
at 50,000 accounts, and 0.4 GiB/day of data availability at one publish every
fifteen minutes. Nothing in this repo justifies a large machine.

What actually costs something:

| Role | Driver | Notes |
|---|---|---|
| Zcash full node | **disk and sync time** | hundreds of GB and growing, plus a long initial sync. Verify current figures against Zebra/zcashd docs rather than any number quoted here. This is the machine to size. |
| Sequencer | latency, uptime | trivial CPU. A liveness single point of failure, so keep a warm standby — `store.rs` exists so one can resume from a saved state rather than replay from genesis. |
| RPC | fan-out | stateless reads. Horizontal, cache in front. `AccountIndex` is per-root, so a read replica rebuilds it once per epoch. |
| Data availability | retrievability | must **not** live only on the sequencer's box: its whole purpose is to be there when the sequencer is not. |
| Signers | **independence** | see below. |

### Hetzner: yes for the bulk, no for the signers

For Zcash nodes and RPC it is good value — cheap dedicated NVMe with real
bandwidth, which is exactly the shape of the Zcash-node problem.

Two cautions specific to *this* project rather than to hosting generally:

- **Provider concentration defeats the threshold.** The security model is 7-of-10
  *independent* signers. Ten signers on one provider — worse, one region — is a
  1-of-1 system wearing a costume: one account action or one datacentre event
  takes all ten. Signer diversity across providers, jurisdictions and ideally
  operators is a **security requirement**, not an ops preference, and it is the
  one place to spend effort and money.
- **Privacy infrastructure gets deplatformed.** Hetzner has a history of abrupt
  suspensions for crypto-adjacent workloads, and a single European jurisdiction
  is a concentrated regulatory bet for a Zcash-anchored system. Check their
  current terms directly; do not rely on a summary. Either way, assume any one
  provider can disappear overnight and make sure that is survivable rather than
  fatal.

### What survivability actually requires

Not redundancy everywhere — the microchain is cheap enough that this is not a
budget question:

1. Signers on **different providers and jurisdictions**. Non-negotiable.
2. At least two Zcash nodes, not co-located.
3. A warm sequencer standby with a recent saved state.
4. DA published somewhere that outlives the sequencer. **Built 7 Sep 2026** —
   anchors on Zcash, bundles to mirrors, `zyn-replica` (§32; RUNBOOK §4s).
5. Everything else can be one cheap box until traffic says otherwise.

---

## 7e. Wallets: what we build and what we refuse to

Researched September 2026, not recalled. Two Zcash wallets matter and both are
real: **Zodl** (formerly Zashi, from the Zcash Open Development Lab — the
protocol's own engineers; mobile, web, Keystone hardware support, and it already
ships swaps) and **Noir** (a browser-extension wallet for shielded ZEC with DeFi
access, Zcash Community Grants funded, Keystone 3 Pro support). Noir's team also
won a ZCG grant, approved 31 August 2026, to build **WalletConnect for Zcash** —
wallet discovery, QR connection, and shielded transaction requests carrying
recipient, amount, fee and **memo**, with keys and viewing keys never leaving the
wallet. Milestones run to late 2027.

| # | Decision | Status |
|---|---|---|
| 7e.1 | ~~**We do not build a Zcash wallet**~~ — superseded 6 Sep 2026: **Nap Wallet** exists (RUNBOOK §4p), because no mainstream wallet can see Ironwood or carry a Zyn memo | ✅ superseded |
| 7e.2 | **We do not require our own wallet for Zyn** — mainstream wallets still sign intents; Nap Wallet is one signer among them | ✅ |
| 7e.3 | Accounts are multi-scheme; the scheme is bound into the account id | ✅ |
| 7e.4 | An intent is signed as **EIP-712 typed data**, not a blob (**S13**) | ✅ |
| 7e.5 | Third-party wallets never connect to Zyn — they are signing oracles only (Nap Wallet, being ours, does) | ✅ |
| 7e.6 | `domain.chainId` is **omitted**; chain binding lives in `salt` | ✅ |
| 7e.7 | Integrate the **Wallet Standard**, not individual Solana wallets | ✅ |
| 7e.8 | **Session keys**: one wallet signature authorises many trades | ✅ |
| 7e.9 | Signing ≠ entitlement: `intent_authorities` + `Authorized` (**S14**) | ✅ |
| 7e.10 | No MetaMask Snap, and no dependence on the WalletConnect grant | 🔒 |

### 7e.1 — Why not build a Zcash wallet

A deposit is an ordinary shielded send with a memo; a payout is an ordinary
shielded receive. There is nothing about either that a wallet does not already
do, and doing it ourselves means note management, chain sync and proving — the
hardest, least differentiated work in the ecosystem, already done well by people
who wrote the protocol. Every hour spent there is an hour not spent on the
thing nobody else is building.

The adoption argument points the same way and harder. "Install our wallet"
before "try our AMM" is two leaps of faith stacked, and the first one buys the
user nothing.

### 7e.2 — But a Zcash wallet cannot sign a Zyn intent

It also cannot be made to. A Zyn intent is not a Zcash transaction, and no
existing wallet knows the envelope (**S12**). So there is a signature we must
obtain ourselves, and the naive reading is that we are back to building a
wallet after all.

We are not, because the intent is *our* bytes and we get to choose their shape.
Choose a shape mainstream wallets already sign, and the wallet problem
disappears:

```
  account = H("zyn.account.v1" | scheme | len(key) | key)
```

| scheme | signs | wallets |
|---|---|---|
| 1 — ed25519 | the canonical payload | our own keys, the CLI |
| 2 — secp256k1 / EIP-712 | `keccak(0x1901 ‖ domain ‖ struct)` | **MetaMask**, Rabby, Coinbase, Ledger, WalletConnect |
| 3 — ed25519 / Solana message | the displayed text | **Phantom**, Solflare, Backpack |

This is not a convenience feature; it is the bridge thesis finishing its own
sentence. Someone bridging SOL in already runs Phantom. Someone bridging ETH in
already runs MetaMask. Making them install a third wallet to trade what they
just bridged is a tax collected at the exact moment they have seen the least
proof that any of this works.

### 7e.3 — Why the scheme is in the account id, not in state

The obvious design is a registry: an intent that says *this account uses scheme
2 with this key*. That is state, so it can be changed, so it is a second thing
to authorise and a second thing to steal.

There is no registry. The account id **is** the commitment to the pair. The
scheme cannot be swapped under an account; the same 32 bytes of key material
under two schemes are two unrelated accounts; and an account exists the moment
someone deposits to it, with nothing to register first. Separation then holds
three times over — by account id, by hash function, and by a scheme tag inside
the payload — which is more than necessary and deliberately so, because a
cross-scheme confusion is not a bug that gets noticed.

### 7e.4 — Typed data, because blind signing is the vulnerability

`personal_sign` would have been a day's work: hand MetaMask our payload bytes,
get a signature. It would also have shown the user a hex blob, and a hex blob
is not a decision — it is a habit of approving blobs, which is the precondition
for every wallet-drainer that has ever worked.

EIP-712 shows named fields. `path` is the one that matters most: a swap routed
through an attacker's pool is a different trade at identical amounts, and it is
precisely what a compromised front end would change quietly. It is signed, so
it cannot be.

Amounts appear once, as the raw `int256`. A friendlier decimal string beside it
would be two encodings of one number, and two encodings can disagree — which is
the exact lie a malicious interface wants to tell.

**The safety property that makes any of this acceptable:** an EIP-712 digest is
prefixed `0x19 0x01`. A legacy Ethereum transaction is an RLP list starting
`0xc0..=0xff`; a typed transaction starts with its type byte `0x00..=0x7f`.
Neither can be `0x19`. So a signature a user gives ZynZap **cannot** move their
ETH, whatever we asked them to sign — and the `0x01` version byte separates it
from `personal_sign`'s `0x45`, so it cannot be replayed as a login challenge
either.

### 7e.5 — Neither wallet ever talks to Zyn

The natural objection is that Phantom cannot add a custom RPC and MetaMask
cannot add a non-EVM chain. Both are true, and neither matters, because **we
never ask either wallet to reach Zyn.** They are used as signing oracles and
nothing else: they hold a keypair, we hand them bytes, they hand back a
signature. Our own app talks to the Zyn RPC. The wallet does not know Zyn
exists, does not need a network entry, and pays no fee.

- **Phantom** — `signMessage` signs arbitrary UTF-8 or raw bytes with the
  user's ed25519 key. It is entirely local: no transaction, no RPC, no fee.
- **MetaMask** — `eth_signTypedData_v4` is a core JSON-RPC method needing no
  extension, no install and no review queue.

**One trap, found the hard way.** MetaMask *validates* `domain.chainId` against
the network the user currently has selected and refuses to sign on a mismatch;
if the id is one the wallet has never heard of, the request does not even fail —
[the promise never resolves][mm18276]. A Zyn chain id is by definition not an
EVM network id, so the first version of this bound `chainId: 11` in the domain
and would have hung for every real user, while passing every test we had.

The fix: **omit `chainId` entirely.** All `EIP712Domain` members are optional,
so the chain binding moves into `salt` as `H("zyn.domain.v1" | chain_id |
vm_id)` — bytes32 that no wallet inspects. The signature is bound exactly as
tightly as before; there is simply nothing left for a wallet to disagree with.
Guarded by `the_zyn_domain_declares_no_evm_chain_id`, because this is the kind
of mistake that comes back.

[mm18276]: https://github.com/MetaMask/metamask-extension/issues/18276

### 7e.6 — Which Solana wallets, exactly

`signMessage` is not a Phantom extra. It is **`solana:signMessage`**, a Wallet
Standard feature, so anything implementing the standard exposes it: Phantom,
Solflare, Backpack, Nightly, and — usefully — **MetaMask**, which now supports
Solana natively and implements the same feature. One integration against the
Wallet Standard reaches the whole set; there is no per-wallet work.

Two caveats worth knowing before support tickets arrive.

**Ledger.** Offchain message signing needs Solana Ledger App **1.3.0+**, and
UTF-8 messages require **"Allow blind sign"** to be switched on in the app.
Users who keep that off — the cautious ones, which is to say the ones with the
most at stake — cannot sign our message at all. There is no fix on our side.
It is an argument for the EVM path being the better-supported one, not for
dropping this.

**Not-a-transaction, established rather than inherited.** Solana's offchain
message spec opens with `\xff"solana offchain"`, and `\xff` is chosen because
it is [illegal as the first byte of a transaction `MessageHeader`][srfc38] —
exactly the job EIP-712's `0x19` does. We do not use that envelope: it would
put binary noise in front of the text a wallet displays, and plain text is what
Sign In With Solana and every sign-in-with-wallet flow hands over today.

So the property is ours to prove. A signature over bytes that also deserialise
as a transaction message *is* a valid transaction signature, so
`a_solana_signature_over_our_message_can_never_be_a_transaction` checks two
independent reasons ours cannot — the header is structurally impossible
(90 required signers against 32 declared accounts) and the message is orders of
magnitude too short to hold the accounts that header claims. Both are derived
from the live message, so editing the text cannot quietly invalidate them.

[srfc38]: https://github.com/solana-foundation/SRFCs/discussions/3

### 7e.7 — Session keys: sign once, trade freely

The Ledger gap prompted this, but Ledger is the symptom. The disease is that
**a wallet popup per swap throws away the product.** ZynZap's claim is fast
trading; the engine does 690k swaps/sec and a MetaMask confirmation takes
several seconds of reading a dialog. A fast AMM that asks permission for every
fill is a slow AMM with a fast engine inside it.

So the mainstream wallet is asked **once**, and what it authorises is a key
that can trade — not a trade. A [`Delegation`] is a certificate, not state:
the owner's wallet signs *"this session key may act for my account until epoch
E, and may only trade"*, and thereafter the page signs each intent silently.
The sequencer checks a two-signature chain and looks nothing up.

| # | Decision | Status |
|---|---|---|
| 7e.7a | Delegation is a **certificate**, not a registry entry | ✅ |
| 7e.7b | Sessions carry **capabilities**; `CAP_WITHDRAW` withheld by default | ✅ |
| 7e.7c | `CAP_DELEGATE` is **never grantable** — a session cannot issue a session | ✅ |
| 7e.7d | Unclassified intents require **all** capabilities (fail closed) | ✅ |
| 7e.7e | Revocation is expiry; sessions are short | 🔒 |

**Why a session key leaking is survivable.** It lives in a browser, so it will
leak. It is issued with strictly less authority than the key that issued it: it
expires in epochs, it cannot withdraw, and it cannot rebind a payout
destination. Stacked on the withdrawal binding — redirecting waits out
`exit_timeout_epochs` — the worst a stolen session achieves is bad trades until
it expires. That is a real loss, not a pretended one. It is *bounded*, which
per-swap signing also stops being once users learn to click approve without
reading.

**Fail closed, deliberately.** `MicrochainVm::intent_capability` defaults to
demanding every capability, so no session can execute an intent a VM has not
classified. A VM author who adds an intent and forgets the method loses
convenience, never money. `CAP_DELEGATE` is refused even when a delegation
explicitly names it, so an over-broad grant is void rather than honoured —
the mistake is not expressible instead of merely discouraged.

**What the user reads.** The session prompt is the most consequential dialog in
the system, so it names powers in words: `mayOnly: "trade"`, and the Solana
text says *"It cannot withdraw."* outright. A capability bitmask would be a
number nobody reads.

**The honest cost.** Revocation is expiry, not an instant switch — the price of
having no registry and no round trip before the first trade. If a user's laptop
is stolen mid-session, the window is the session's remaining life. An on-chain
revocation list would close that and is the obvious extension; it is not built.

### 7e.8 — The hole all of this was sitting on

Found while auditing testnet readiness, after the wallet work was done and
green. Nothing anywhere checked that the signer of an intent controlled the
account the intent named.

`signer_of` returned who signed. No caller compared that to anything. So Alice
could sign — correctly, verifiably, with a signature that passes every check in
`verify.rs` — an intent spending Bob's balance, and it would have executed. No
test would have gone red. Every cryptographic assertion in the tree was true.

This is the failure mode worth naming, because it recurs: the checks that get
built are the ones something fails without. Signature verification gets built
because signatures fail to verify. Entitlement does not get built, because
nothing fails.

Two fixes, both structural rather than a check someone must remember:

- **`MicrochainVm::intent_authorities`** declares which accounts must sign, and
  defaults to *empty* — meaning no user signature authorises the intent at all.
  A VM that adds an intent and forgets the method makes it unusable, never
  unguarded.
- **`Authorized<I>`** has no public constructor except one that verifies
  signatures and one that says `Authority::Operator` in writing.
  `Node::submit` takes it, so an RPC that forgets to authenticate does not
  compile.

The honest limit of the second: `Authorized::operator` is public, so it stops a
check that is *forgotten*, not one that is *falsely claimed*. That is the
achievable guarantee, and it is the one that matters — nobody sets out to skip
authentication.

### 7e.9 — No Snap, and no waiting on WalletConnect

MetaMask's account-management Snaps do not extend to non-EVM accounts, so a
"Zcash Snap" that makes MetaMask a Zcash wallet is not available even in
principle right now. It is also not needed: `eth_signTypedData_v4` is a core
JSON-RPC method every EVM wallet exposes with no extension, no install and no
review queue.

The same reasoning applies to the WalletConnect grant. It is genuinely good news
for the **ZEC leg** — standard deposit and payout requests carrying a memo, from
any wallet, instead of copy-paste — and we should adopt it when it lands. But
its milestones run to late 2027 and it is somebody else's schedule. Nothing on
the Zyn side waits for it.

### What is left open

- **Memo UX.** A forgotten memo is the classic way people lose funds depositing
  anywhere, and it is our most likely support burden. Per-account diversified
  deposit addresses remove the memo entirely — this is §8.11, and it should be
  settled before real users, not after. One correction to an earlier worry: the
  memo is *inside* the encrypted note, so it is visible to viewing-key holders,
  not to the public.
- **Session revocation.** Expiry only, today (7e.7). An on-chain revocation
  list would let a user kill a session immediately rather than waiting it out.
- **Sign In With Solana (SIWS).** Phantom supports a structured sign-in message
  format that renders far better than the plain text we hand it today. Ours
  works and is canonical; SIWS would look native. Cosmetic, not blocking.
- **Byte-compatibility with a live wallet.** The EIP-712 encoder is checked
  against the specification's own worked example — we recover the address that
  the specification's own signature was made by, which no self-consistent
  mistake can survive. That is strong, but it is not the same as having clicked
  approve in MetaMask. A browser test against a real extension is still owed.
- **Which key holds what.** The Zyn signing key is necessarily warm, because it
  signs swaps. What stops that being fatal is not in this section: a withdrawal
  destination is bound in state and redirecting it waits out
  `exit_timeout_epochs`, so a stolen key costs trading control rather than the
  balance.

---

## 8. Open

Ordered by how expensive they get if deferred.

1. **Metering.** `MAX_BATCH` is a decode-time sanity limit, not metering. Real
   metering is cycle counting in the executor. **Nothing permissionless ships
   without it.**
2. **32-byte derived addresses** (4.4). A one-way door; cheapest before a
   frontend depends on either scheme.
3. **State passing.** The whole state crosses the ZVM boundary per batch —
   O(state) per call, paid again in a guest. The fix is a merkleised store where
   a program requests only what it touches. It **changes the ABI**, so do it once
   before Stage 2, not during.
4. **Conservation cost.** S11 is a full scan per batch, paid again inside a
   guest. The replacement is incremental conservation (track a transition's
   deltas). Real redesign.
5. **Cross-VM atomic bundles.** Apply to each VM's scratch state, commit only if
   all succeed. Sui's Programmable Transaction Blocks are the validated model.
   **Not** synchronous callbacks — that rebuilds the surface 3.1 deleted.
   **Promoted** (§6c): keeping launchpad mechanics out of ZynZap is only
   coherent if a launchpad *on Zyn* can graduate a token into a ZynZap pool.
   This is what makes "app #2" mean anything. Note it is **not** needed for a
   launchpad that lives off Zyn — that case is deposit-shaped and reuses the
   bridge.
6. **Resting offer book** (6.6). `AcceptOffer` is only the settlement half —
   and deliberately stays that way. A resting book is **secondary priority**
   (§6b): maintaining prices off the curve is the thing Zyn is choosing not to
   be. Settle an agreement, yes; maintain a book, later.
7. **Shared vs isolated threshold custody** across applications. A bug in one app
   reaches the other's ZEC if shared; isolation fragments liquidity. **Settle
   before the second application ships, not after.**
8. **VM upgrade / migration.** `VM_VERSION` exists, but bumping it changes the
   state encoding and therefore the root. Nothing says how a chain migrates.
9. **Settlement verifies inclusion, not solvency**, for a VM whose records it
   cannot interpret. ZynZap closes this internally with the backing invariant; no
   general answer exists.
10. ~~**The RPC server and the quote path.**~~ Built (`zynzapd`). Reads are
    public, writes need a signature, and operator operations are unreachable
    over the socket. `vm::quote` shares `price_hops` with the swap handler, so
    a quote cannot drift from execution — asserted by a test that quotes, then
    swaps, then compares.
11. **Bridge deposit addressing** (10.4) — per-account is a privacy leak,
    per-deposit needs a scan window and a recovery path.
12. **Shared vs separate signer sets across bridges** (§10). Settle before the
    second vault holds real value.
13. **Memo-free deposits** (7e, 6c). Two independent teams — SHLD and Zenrock —
    shipped address-per-action instead of memos. This should close in favour of
    derived deposit addresses rather than staying open. Per-account diversified addresses instead of a
    memo. Forgotten memos are the most common way users lose deposits anywhere;
    settle before real users.
14. **A browser test against a live MetaMask** (7e). The encoder is proved
    against EIP-712's own vector, which is not the same as proved against the
    extension.
15. **Native item mints vs bridge-only** for L1 NFTs.
16. **A permanently-locked launch and claim-based distribution** (6c). What a
    graduation venue needs beyond the RPC: a pool whose LP nobody can pull, and
    a merkle-root claim intent. Both small; the merkle primitives already exist.
17. **Creator-set `fee_bps` on `CreateToken`** (6c). A trading fee a launcher
    chooses is a swap parameter a launcher can set adversarially. Bridge-only needs nothing
    built and matches the plan's adapter framing (§10, §26C); native mints are
    built (5.4) but unbonded creation paths are the spam vector.

### 10.17 — A bridge mints an asset; the AMM trades it

Worth stating flatly because loose phrasing hides it — including mine, earlier,
when I wrote "the SOL-ZEC.zy pool". There is no such pool and there cannot be.
SOL lives on Solana and never moves. What exists on Zyn is **SOL.zy**: units
issued here, custodied there. The pair is **SOL.zy/ZEC.zy**, and both sides are
bridged assets.

This is already how the code is built rather than something to add.
`TokenInfo::bridged(symbol, origin)` is the whole mechanism, and `state.rs`
says outright that *ZEC.zy is simply the first bridged asset*. Adding Solana is:

```rust
tokens.insert(XSOL, TokenInfo::bridged(symbol(b"SOL.zy"), ORIGIN_SOLANA));
```

— one registration. Backing, conservation (**S11**), exits, dust minima,
settlement grouping and the withdrawal binding all follow from `is_bridged()`
without a new code path. That is the payoff for keeping custody rules generic:
the second bridge is a vault and an asset, not a subsystem.

The decomposition also names the latency honestly. A user going from SOL to ZEC
does three things: bridge in (Solana finality), swap (instant), bridge out
(Zcash confirmations plus a signing round). **Only the middle is fast.** The
AMM's speed is a claim about the middle, and saying so is better than letting
someone discover it.

### 10.18 — Bridged assets are `<ASSET>.zy`, native assets have no dot

The first version of this section proposed `x` for bridged and `z` for native,
so bridged SOL would be `xSOL`. That is wrong twice over, and the way it is
wrong is the more useful lesson.

**Both prefixes are taken, on the chain we bridge from.**

- **`xSOL`** is [Hylo's Leveraged SOL][hylo] — a live Solana token, ~$8M market
  cap. Not merely a name clash: it means *2.2× leveraged SOL*. Someone bridging
  SOL to Zyn and seeing `xSOL` would reasonably believe they held a leveraged
  position. A wrong ticker that means nothing is a nuisance; one that means
  something else is a trap.
- **`zSOL`** is also taken — a verified token in Solana's registry.
- `xZEC` happens to be free, but the wrapped-ZEC field already holds `WZEC`,
  `zenZEC`, `TZEC`, a Zolana version and an OmniBridge version.

That last list kills the obvious fix. `zynZEC` would sit one letter from
Zenrock's **zenZEC**, both wrapped ZEC, both aimed at the same users. Worse than
the collision we started with.

**The real lesson is not "pick a different prefix".** Checking two prefixes
found two collisions. The prefix space is crowded and getting worse, and every
asset we add would need the same search with worse odds. A rule that must be
re-verified per asset is not a rule.

So the namespace moves to the end, where it is ours:

| | |
|---|---|
| **`<ASSET>.zy`** | Bridged. Has a vault, a backing figure, and a bridge that can fail. `ZEC.zy`, `SOL.zy`, `USDC.zy`. |
| **no dot** | Native to Zyn. No vault, no external dependency. `zUSD` — the `z` there is branding, not a custody claim. |

Three things this buys beyond ending the collision problem:

1. **The convention already reads as "bridged".** `USDC.e` on Avalanche and
   `USDC.axl` from Axelar have taught users that a dotted suffix means a
   wrapped version. `.zy` inherits that comprehension rather than teaching it.
2. **The base asset comes first**, which is what people scan for. `SOL.zy` is
   recognisably SOL; `xSOL` is recognisably something else.
3. **It never needs checking again.** The suffix is our namespace, so a
   thirtieth bridged asset is as safe as the first.

It also keeps the property the original rule was for. Even ZEC — the asset this
chain exists to serve — is `ZEC.zy`, because units on Zyn are issued against ZEC
custodied in a vault, and that vault can fail. The home asset does not get to
hide its custody risk behind a friendlier name.

**Why `.zy` and not `.z`.** One character buys two things. `z` alone is the
most overloaded letter in the space — Zcash, zk, zkSync, and a `.zk` domain
namespace already exists — so `SOL.z` reads as plausibly "the Zcash one", which
is wrong: it is the *Zyn* one. `.zy` is legibly ours. The pattern itself is
established practice — Gnosis Chain suffixes bridged tokens by chain of origin.

**Constraint to remember:** `Symbol` is `[u8; 8]`, so `.zy` caps the base asset
at **five characters**. Everything current fits — `ZEC.zy` and `SOL.zy` are 6,
`USDC.zy` is 7, `MATIC.zy` is exactly 8. Six-character bases do not: `wstETH.zy`
is 9, and liquid-staking tickers trend long.

**Open, and cheapest right now:** widening `Symbol` to `[u8; 12]` removes the
cap for good. It costs four bytes per token in state and nothing else — but it
changes the state encoding and therefore every root, so it is a one-way door
that gets expensive the moment a chain is live. Worth deciding deliberately
rather than discovering when the first long ticker arrives.

[hylo]: https://www.coingecko.com/en/coins/hylo-leveraged-sol

#### The internal symbol and the displayed name are different decisions

The plan already settled the second one, and this section originally ignored it:

> *"Naming note: **xZEC** is the internal system name. User-facing UI can simply
> display **'ZEC'**."* — plan §, echoed at `types.rs`

That rule is right and survives everything above, because it is about **ZEC
specifically**. Zyn is Zcash's layer 2; ZEC is the home asset; the plan wants
the wrapper invisible to a normal user, who did not choose to bridge anything
and is simply using ZEC. The plan also says xZEC "is not intended to become an
independent speculative asset" — so it is not competing for ticker space at all.

It does **not** transfer to SOL. Someone holding `SOL.zy` chose to bridge, is
holding a foreign asset with custody risk, and is one tab away from real SOL and
from Hylo's `xSOL`. Displaying that as plain "SOL" would be the exact
mislabelling this section exists to prevent.

So:

| asset | internal symbol | displayed as | why |
|---|---|---|---|
| bridged ZEC | `ZEC.zy` | **"ZEC"** | home asset; the plan's rule; wrapper invisible |
| bridged SOL | `SOL.zy` | **"SOL.zy"** | foreign, bridged by choice, custody risk is news |

The internal symbol stays uniform because state should not have special cases;
the display layer is where the asymmetry belongs, and it is asymmetric for a
reason rather than by drift.

**Open:** the internal rename of `xZEC` → `ZEC.zy` overrides a documented plan
decision on consistency grounds alone. It is one line of code
(`state.rs:464`) plus prose. If the plan's `xZEC` should stand internally —
with `SOL.zy` still used for everything bridged later — that is a one-line
revert, and the display rule is unaffected either way.

### 10.19 — ZEC.zy is the hub

Every bridged asset gets its market against ZEC.zy rather than against every
other asset. `SOL.zy → zUSD` is two hops through ZEC.zy, which the existing
`path: Vec<PoolId>` already routes.

Liquidity in one AMM fragments across pairs faster than it fragments across
venues; n assets are n pools against a hub and n²/2 without one. Choosing ZEC.zy
as the numéraire is also the thing that makes this a *Zcash* layer 2 rather
than a general DEX that happens to be anchored to Zcash — every route touches
ZEC.

## 6b. What "enforce a pair at mint" actually means

The plan says a token is launched with its pool in one intent (5.2). Read
literally that breaks every asset whose price is not discovered by trading it:
a liquid-staking token, a binary event token, a DeFi token that already trades
somewhere else. The rule as written would force each of them into a
price-discovery mechanism when there is nothing to discover.

### The conflation

**A bond is the anti-spam mechanism. A pool is the price-discovery mechanism.**
For a memecoin they arrive together, so the plan fused them into one rule. They
are not the same requirement, and the code already knows it: `MintItem` takes a
bond and creates **no pool**, because a curve cannot price an indivisible thing
(6.x). `TokenInfo::genesis_pool` is an `Option<PoolId>`, and nothing anywhere
requires it to be `Some`.

So the architecture is already more general than the sentence. The rule needs
restating, not redesigning:

> An asset whose price is **discovered here** must be born with the liquidity
> to discover it.

That excludes the awkward cases by construction rather than by exception.

### The taxonomy that follows

Sort by *where the price comes from*, because that is what decides which
machinery an asset needs:

| price comes from | example | issuance | pool |
|---|---|---|---|
| **discovered here** | memecoin | `CreateToken` — atomic launch | required, liquidity locked |
| **derived by formula** | liquid-staking token | *missing* | optional, `reference` = redemption rate |
| **discovered elsewhere** | bridged DeFi token | `CreditDeposit` | mirror pool with `reference` |
| **bounded and complementary** | binary event token | *missing* — pair-mint | different shape; expires |
| **not priceable by a curve** | item / NFT | `MintItem` — bond only | none, by design |

Three of the five already work.

### Liquid-staking tokens fit better than expected

An LST's price is a redemption rate, not a market. Forcing discovery is wrong,
but a pool is still *useful* — people want to exit without unbonding. What makes
this safe is a property already built for bridged pairs: **the reference sets
the fee, never the quote** (`amm::divergence_fee`).

For an LST the redemption rate is a far *better* reference than a market price,
because it is computed from staking state rather than observed from a venue
someone can push. As the pool drifts from the rate, the fee rises as `√r − 1`
and the drift pays for itself. That is exactly the behaviour an LST pool wants,
and it needs no new curve.

What an LST must not use is `CreateToken`, and today that is the only native
path to a fungible asset — see the gap below.

### Binary event tokens are structurally a vault

The sharp version: **YES + NO = 1 unit of collateral.** They are minted as a
pair against locked collateral and redeemed against it at resolution. That is
`supply == backing` — the same conservation shape as a bridged asset, where the
"custodied chain" is a locked balance on this one. **S11** already wants exactly
this invariant, and `Vault` already expresses it.

So the accounting is free. What is genuinely missing is two things a bridge does
not need: **pair-minting** as an intent, and **resolution** — an oracle-fed
expiry after which one side redeems 1 and the other 0.

One honest caveat: constant-product is a poor fit for an asset bounded in
`[0, 1]`. It never reaches the bounds and goes thin exactly when a market is
resolving and conviction is highest. Gnosis used LMSR for this; Polymarket
ended up on an order book. Our curve can *host* YES/NO, but "prediction markets
work on ZynZap" would be overclaiming until the endgame behaviour is dealt
with.

### Bridged DeFi tokens already bypass the rule

An LST on Solana bridged to Zyn is not a `CreateToken` at all — it arrives
through `CreditDeposit` as an ordinary bridged asset, and gets a mirror pool
with a `reference`. No new mechanism, and the mint-time rule never applies to
it. Much of the original worry is answered by the bridge already being a
separate issuance path.

### The ordering rule, and what it settles

Stated by the user and adopted:

> Any other pool comes after `ZEC.zy` exists — otherwise it is order-book
> pricing instead of on-chain pricing on Zyn. Order books could exist, but
> they are **secondary priority**.

This is a sharper rule than the taxonomy above, and it subsumes most of it. It
gives a single test for whether an asset type belongs in v1:

> **Can it be priced by a curve against `ZEC.zy`?**
> Yes → primary. Needs a book → secondary.

The same shape as the split test that decides what goes in the spec ("would an
auction need it?"), and it does the same work: it answers cases nobody has
thought of yet.

**What it settles immediately.**

- **Binary event tokens are secondary**, and for a stated reason rather than a
  shrug. Constant product goes thin exactly at resolution, which is why Gnosis
  reached for LMSR and Polymarket ended up on a book. Wanting them badly enough
  would mean making book pricing first-class. That is the trade being declined.
- **`AcceptOffer` stays the settlement half and does not grow a resting book**
  (§8.6). Atomic settlement of an agreement reached elsewhere is what an OTC
  block and an NFT sale need, and it costs nothing in pricing philosophy. A
  *resting* book would be the chain maintaining prices off the curve, which is
  the thing being deprioritised. The line is: settle an agreement, yes;
  maintain a book, later.
- **Liquid-staking tokens stay primary.** They are curve-priceable against
  `ZEC.zy` with the redemption rate as `reference`, so they pass the test.
- **`zUSD` outranks event tokens**, being curve-priceable. The native-fungible
  gap below is therefore the more urgent of the two missing pieces.

**And it converges with the engineering critical path.** `ZEC.zy` exists at
genesis as an asset id, but carries no *backing* until the Zcash bridge runs —
so a pool against it today is a pool against unbacked units. The readiness audit
put the ZEC path first on engineering grounds (§7b, `RUNBOOK` §5); this rule puts
it first on product grounds. They agree, which is a good sign that the ordering
is real rather than a preference.

Resulting order:

1. **`ZEC.zy` with real backing** — the Zcash bridge. Everything waits on it.
2. **Curve-discovered assets** — memecoin launches. Built.
3. **Bridged assets with reference pools** — `SOL.zy`. Mechanism built, needs
   the Solana bridge.
4. **Native fungible mint without a genesis pool** — `zUSD`, native LSTs. Small
   gap, see below.
5. **Anything needing a book** — event tokens, resting offers. Secondary, by
   decision rather than by omission.

### The one real gap

**There is no way to mint a native fungible asset without a genesis pool.**
`MintItem` gives bond-without-pool but only for indivisible items;
`CreateToken` gives a pool you may not want. A natively-issued LST or a
collateral-backed stablecoin has no path.

The fix is small and in the shape of what already exists: an issuance intent
taking a bond and creating no pool, with a pool added later via `CreatePool`
when there is a reason. `genesis_pool: None` is already representable and
already handled, so this is a new entry point rather than a new concept.

**Not built.** Recording it because the plan's sentence would otherwise be read
as forbidding it, and it is not forbidden — it is unimplemented.

## 6c. Prior art: SHLD (shld.fun), memecoins on Zcash today

Studied September 2026. Someone is already shipping our headline use case, which
makes it the most useful thing to read in the space.

**What it is.** Token markets on Zcash where *"the market is public, the buyers
are not, when a buy is paid from a shielded balance."* Note the conditional —
that is an honest privacy claim, not an overclaim, and it is the same claim we
can make.

**How it works**, from its own pages:

> *"Every coin launches the same way: a fixed public curve, reserves at a
> published shielded address, graduation by formula."*

- **A bonding curve, not a pool.** Fixed, public, identical for every coin.
  Graduation to a real market by formula — the pump.fun shape, with a
  `GRADUATION` column in the market table.
- **Custody is one published shielded address.**
- **Creation costs 0.01 ZEC**, and the page says plainly it "is a one-time
  charge, not a deposit; it never backs the coin."
- **Creator supplies four fields**: ticker, name, image, refund address.
- **"No wallet connect needed to create."** It links out to **Zodl** and
  **Zingo** — you pay from an ordinary Zcash wallet.

### Three things worth taking

**1. Their launch has no parameters. Ours has five.**

`CreateToken` takes `supply`, `unit`, `xzec_liquidity`, `token_liquidity` and
`fee_bps`. Every one is a decision a creator can get wrong, and `fee_bps` is one
they can set adversarially. SHLD's answer — *the same curve for every coin* —
removes the whole surface. There is nothing to misconfigure and nothing to
compare, which is also why their market table can rank coins honestly.

**2. Nobody funds liquidity at launch.**

This is the sharper point. Our locked-liquidity bond requires a creator to fund
**both sides** of a pool before anyone has shown interest. A bonding curve
requires no one to fund anything: buyers fund it as they buy, and the curve is
the market until graduation. For the cold-start moment that is strictly easier,
and the barrier we impose is real — 5.3 argued the bond is productive rather
than extractive, which is true, and beside the point if it stops the launch
happening.

It also **fits the ordering rule better than our own design does** (§6b). A
bonding curve needs no LP and no counterparty — it is pure curve pricing rooted
in ZEC.zy, which is exactly what that rule asks for. It is *more* aligned with
the stated philosophy than pool-at-mint is.

**3. It confirms the wallet decision, and extends it.**

"No wallet connect needed to create", with links to Zodl and Zingo, is the memo
model working in production (§7e.1). It also suggests going further than we
planned: if *creating* can be a wallet send, the first touch needs no account,
no signature scheme and no session — the EIP-712 and session-key work is for
**trading**, and does not need to stand between a user and their first action.

### Where we are genuinely different

Their reserves sit at **one published shielded address**. That is a custodian,
and it is the whole security model. Against that:

| | SHLD | Zyn |
|---|---|---|
| custody | one shielded address | FROST *k*-of-*n*, no single holder |
| backing | trust the operator | attested ceiling, conserved (**S11**) |
| exit if operator vanishes | none stated | DA snapshot + timeout refund |
| verifiability | published record | state roots anchored to Zcash |

So the differentiator is **not** speed or features. It is that a holder does not
have to trust us the way they must trust a published address. That is worth
saying plainly in positioning, and it is worth *only* saying once the exit
hatch actually runs — today it is tested, not deployed (§7b).

### The action item — withdrawn, and why

The first version of this section recommended adding a fixed-curve launch preset
to `CreateToken`. That is wrong, and the correction is a boundary worth stating:

> **A bonding curve is launchpad language.** If someone builds a launchpad on
> Zyn they can manage it themselves. It is not ZynZap's concern as a swap.

The split test that decides what belongs in the spec — *would an auction or a
game need it?* — has a sibling for the application layer: **is this a swap
concern, or a different application's?** Bonding curves, graduation thresholds,
presale rules, fair-launch mechanics and creator fee splits are all the second.
A swap's job is that assets which exist can be traded against a curve.

Reading SHLD as a competitor invited copying its shape. It is better read as
**evidence for the platform thesis**: what it does is app #2 on Zyn, and the
right response is that someone can build it, not that ZynZap should absorb it.
Composition is proven elsewhere — pump.fun graduates into Raydium, and neither
is the other.

**What survives from the study**, because it is genuinely about the swap or
about us:

- **"No wallet connect needed."** Applies fully, and extends further than
  planned — the first touch should need no account, no signature scheme and no
  session (§7e).
- **Custody is the differentiator**, not features. One published shielded
  address versus FROST *k*-of-*n* with an exit hatch. Say it only once the hatch
  runs.
- **`fee_bps` on `CreateToken` remains a footgun** — a creator-set trading fee
  is a swap parameter, so this one is ours. Worth revisiting on its own terms,
  not as launchpad mimicry.

### The consequence: composition matters more than it looked

Drawing this boundary correctly **raises the priority of cross-VM composition**
(§8.5). A launchpad as app #2 has to graduate a token into a ZynZap pool, and
that crosses a VM boundary — where §3.1 establishes there is no synchronous call
and no shared state, and bridged balances must live in the VM that trades them.

So "someone else can build the launchpad" is only true once two applications can
hand value to each other. Today they cannot. The boundary is right; the thing
that makes it pay off is unbuilt, and it is now the most load-bearing item on
the open list rather than a distant nicety.

### The latency argument, with numbers

SHLD's own buy instructions end:

> *"Blocks are pretty slow right now so it can take a few minutes for the token
> to land in your wallet. Keep the page open and be patient."*

That sentence is the case for a layer 2, written by someone who does not need
one to be right.

**Where the minutes come from.** Zcash's target spacing is **75 seconds** since
Blossom ([ZIP 218] proposes 25s for NU7). But proof-of-work intervals are
*exponentially distributed*, not regular: the median gap is under a minute, the
mean is 75s, and the tail is long. "A few minutes" is one to three blocks with
variance.

**The structural difference is not the size of the number.** SHLD pays a Zcash
confirmation on **every buy and every sell**. Zyn pays it at the **boundary** —
once on deposit, once on withdrawal — and everything between is in-epoch.

| trades | SHLD | Zyn |
|---|---|---|
| 1 | ~1–4 min | ~1–4 min + instant |
| 10 | ~10–40 min | ~1–4 min + instant |
| 100 | hours | ~1–4 min + instant |

**And the second advantage is bounded latency, not just lower latency.** A PoW
confirmation cannot be promised — that is why the instructions say *be patient*.
An epoch seal is deterministic. "Final now" is a different product from "usually
a few minutes."

**The honest caveats, both of which matter:**

1. **ZIP 218 would narrow it 3×.** If NU7 ships 25-second blocks, per-trade
   latency drops. The structural argument survives — per-trade cost versus
   one-time boundary cost — but the moat should not be phrased as "L1 is slow",
   because L1 is getting faster.
2. **The advantage only exists from the second trade onward.** For a one-shot
   buyer, SHLD is strictly simpler: no deposit, no account, no boundary. We ask
   a user to pay *their per-trade cost, once, up front*, before they have seen
   anything be fast. That is the whole product risk, and it is what "just need
   to do it right" has to mean: the first trade after depositing must feel
   instant enough to justify the wait that preceded it.

[ZIP 218]: https://zips.z.cash/zip-0218

### Two things their UX does that we already beat

Both are already built, which is worth knowing before positioning anything.

**"A wallet is generated in your browser. Save the private key."** That is a
browser-held key with *full authority* and *no recovery*: lose it, lose the
tokens. It is exactly the risk §7e.7 bounds. Our account is derived from a
signature by a wallet the user already has — nothing to save, and recoverable by
signing again — while the browser key is a **session** key that expires and
**cannot withdraw**. Same convenience, without asking a user to be their own
backup service.

**"Enter your refund address."** Per purchase, every time. That is the payout
destination decided per transaction and unattested. We bind once
(`BindWithdrawal`), and redirecting waits out `exit_timeout_epochs` — fewer
keystrokes *and* the stronger property.

**One thing they do that we should copy.** They send to *"the address shown"* —
a fresh address per purchase, no memo. That is §8.11's memo-free alternative,
running in production, avoiding the forgotten-memo failure entirely. Between
this and zenZEC's per-user dMPC deposit addresses, two independent teams shipped
address-per-action rather than memos. That open item should close in favour of
derived addresses.

### On "plugging in" their coins

Their portfolio page says balances are *"recomputed in this browser from the
public settlement feed — every note's commitment re-derived from your keys and
matched against the on-chain record; nothing on this page is a served balance."*

So it is not a plain custodian. There is a **public settlement feed**, a
**commitment** scheme, and **client-side verification** — closer to a validium
than to an exchange ledger. Their access pattern is privacy-aware too: feeds are
fetched identically for every market so a request does not reveal holdings.

That makes their record readable by third parties, but it does **not** make
their tokens bearer assets. The value behind them is ZEC at SHLD's address, so
no one can unilaterally import them — verifying a balance is not the same as
being able to honour it.

Three paths, and the ordering is not close:

1. **Be the venue they graduate into.** Their market table already has a
   `GRADUATION` column. Launchpad → swap is precisely the boundary drawn in §6c,
   and it is the pump.fun → Raydium relationship: cooperative, architecturally
   clean, and requiring no import at all.
2. **A cooperative bridge** — they support withdrawal into a Zyn vault, we issue
   representations. Needs their buy-in and a custody relationship.
3. **Read-only mirroring** — the feed is public, so prices and holdings could be
   *displayed* without custody. Permissionless and weak.

Path 1 is the one consistent with everything else decided here, and the only one
that needs no permission we do not already have a reason to ask for.

### What being a graduation venue actually requires

Four things, and the useful news is that three were already on the list — the
strategy does not fork the roadmap.

**0. `ZEC.zy` with real backing.** Unchanged and still the gate. A graduated
pool prices against ZEC.zy, so nothing here matters until the Zcash bridge runs.

**1. The RPC** (§8.10). Already the top infrastructure item, and this reframes
how load-bearing it is: an integration is *an external system calling us*.
Today nothing can. This is the gate for **any** partnership, not just this one,
and it is the difference between "someone can build on Zyn" being true and
being a claim.

Authorisation is ready for it — `Node::submit` takes an `Authorized`, so an RPC
that forgets to authenticate will not compile (**S14**).

**2. A launch whose liquidity is permanently locked.** New, and small.
`launch_token` today locks `locked_shares(..)` — a minimum — and hands
`creator_shares` to the launcher, who can withdraw them. Graduated liquidity
must be liquidity **nobody** can pull; that is the property that makes a
graduation trustworthy rather than a rug with extra steps. A variant where
`creator_shares == 0` and the whole position is locked.

This is legitimately ours rather than launchpad mechanics: *locked LP* is a
property of a pool, and pools are the swap's business (§6c).

**3. Claim-based distribution.** The real gap, and it solves two problems at
once.

A graduating launchpad has holders. Crediting them requires knowing their Zyn
accounts — which it does not, because its holders are keyed in its own namespace
(SHLD generates a keypair in the browser; a Zyn account is `H(scheme | key)`).
A bulk write would also mean trusting a list, and writing N accounts in one
intent.

The answer is the standard airdrop shape, and every primitive is already in
`zyn-vm::commit`: the launchpad publishes a **merkle root** over
`(holder, amount)`, and each holder claims with an inclusion proof into whatever
Zyn account *they* control. No bulk write, no list to trust, no namespace
mapping, and holders who never claim cost nothing. `merkle_root`,
`merkle_proof` and `verify_proof` exist and are tested; what is missing is an
intent that spends a claim once and cannot be replayed.

**Not required: cross-VM bundles.** Last section promoted §8.5 on the assumption
the launchpad is app #2 *on Zyn*. For a launchpad that lives **off** Zyn — which
SHLD does — graduation is deposit-shaped, not cross-VM, and reuses the bridge.
Both cases are real and they have different timelines:

| launchpad lives | graduation is | needs |
|---|---|---|
| off Zyn (SHLD today) | a deposit plus a claim root | RPC, locked launch, claims |
| on Zyn (app #2 later) | a cross-VM handoff | all the above **and** §8.5 |

So §8.5 stays promoted for the platform thesis, and is **not** a blocker for the
nearer opportunity. That distinction is worth keeping straight, because it is
the difference between "one more thing" and "a redesign".

### One to watch

**Zcash Shielded Assets (ZSAs)**, the QEDIT work on generic shielded assets at
L1. If tokens can live natively shielded on Zcash, the reason to hold them on a
layer 2 becomes *execution* — cheap, fast, frequent trading — rather than
existence. That is still our argument, but it stops being the only one available
and is worth tracking rather than discovering.

## 6d. Governance: the boundary, decided before there is anything to govern

| # | Decision | Status |
|---|---|---|
| 6d.1 | **Custody is never a governance surface** | 🔒 |
| 6d.2 | **The exit hatch is never a governance surface** | 🔒 |
| 6d.3 | No DAO and no governance token while there is one sequencer | 🔒 |
| 6d.4 | Decentralise the **signer set** first — that is a federation, not a vote | ✅ |

### 6d.1–2 — What must stay outside any vote, forever

This is the part worth settling now even if a DAO never happens, because the
mistake is only cheap to prevent in advance.

The differentiator (§6c) is that a holder does not have to trust us the way they
must trust a published address: *k*-of-*n* threshold custody, conserved supply,
an exit that works without the operator. **A vote over the signer set destroys
that.** It converts a cryptographic guarantee into a governance-capture problem,
which is strictly weaker — whoever controls the vote controls the vault, and
every large bridge failure of recent years has a governance step somewhere in
the middle of it.

So, permanently outside governance:

- who holds signing shares, and the threshold
- `exit_timeout_epochs` and the withdrawal binding delay
- the DA snapshot and anything the escape hatch depends on

A parameter a user relies on to *leave* cannot be adjustable by the people they
would be leaving. Fee tiers, pool parameters, listing policy — those are
governable in principle. The exit is not.

### 6d.3 — Why not yet

There is nothing to govern. One sequencer, no permissionless validator set
(§9), no live bridge, no RPC. Governance over a system with a single operator
is not decentralisation; it is a claim of control that does not exist, and
making that claim is worse than making none.

A governance token would also have to earn its place against §5, where revenue
is fees plus a locked-liquidity launch bond and nothing is missing. Rent was
rejected there for being extractive rather than productive; a token whose only
utility is voting on a system that is not decentralised is the same objection
wearing a different hat.

### 6d.4 — Do the real version instead

The genuine need behind "should there be a DAO" is usually legitimacy, funding,
and a credible path away from one operator. Each has a better answer available
now:

- **A signer federation.** Recruit three to five *independent* parties for the
  FROST set. This decentralises the thing that actually matters, is a far
  stronger claim than a vote, and is a prerequisite for mainnet anyway — §7d
  already forbids the shares living on one machine. Independence comes from
  recruiting, not from voting.
- **A Zcash Community Grants application.** Real money and the ecosystem's own
  validation; ZCG funded Noir's WalletConnect work at $50k in August 2026. It
  needs a team, not a DAO.
- **Publish the specification.** `SPEC.md`, `DECISIONS.md` and a conformance
  suite are an unusual thing for a project this early to have. Showing the
  reasoning is a stronger credibility signal than announcing a governance
  structure — and it exists already.

### When to revisit

When there is a decision a person outside the team should be able to make and
currently cannot: parameter changes on a live chain with real users, treasury
allocation once fees accrue, or — the real one — **which applications may run
on Zyn** once it is permissionless. That last question is genuine governance
and it is downstream of §8.5, not of a token.

## 7f. Where the ceiling actually is

Measured on one M1 core, September 2026. The point of writing it down is that
every intuition here was wrong before it was measured.

| | rate | per item |
|---|---|---|
| swap execution | **622,054/sec** | 1.6 µs |
| EIP-712 digest | 250,306/sec | 4 µs |
| **ed25519 verify** | **30,318/sec** | 33 µs |
| **secp256k1 recover** | **8,712/sec** | 115 µs |
| withdrawal proof (indexed) | 6,588,006/sec | 0.2 µs |

**Execution is not the bottleneck and never was.** Verifying a signature costs
**seventy times** more than applying the swap it authorises. The engine figure
that looks impressive is measuring the cheap half.

So the honest single-core ceiling is **~8,700 intents/sec** for wallet-signed
traffic, not 622,000 — and it is set by the wallet the user chose, since
ed25519 is 3.5× faster than secp256k1 recovery.

### Fixed: verification now runs on every core

`verify::resolve_batch` splits a batch across threads, and
`Hosted::submit_signed_batch` uses it. Measured on ten cores:

| | serial | batched | |
|---|---|---|---|
| secp256k1 recover | 8,854/sec | **48,619/sec** | 5.5× |
| ed25519 verify | 32,533/sec | **163,187/sec** | 5.0× |

Below linear, as expected: thread spawn is not free and half an M1's cores are
efficiency cores.

The batch is split into three phases rather than parallelised whole — decode
and reduce (serial, cheap), verify (parallel, expensive), then entitle and
sequence (serial). **Sequence numbers are still assigned strictly in the order
given**, because a batch that executed in a different order on a machine with a
different core count would be **S1** broken by an optimisation. There is a test
asserting the batch path and the one-at-a-time path produce the same chain,
root included.

Two properties that had to survive the fast path, and are tested: a bad frame
fails alone without disturbing its neighbours' sequencing, and **S14**
entitlement is still checked — a valid signature over someone else's account
authorises nothing, batched or not.

That moves the practical ceiling to **~48,000 wallet-signed intents/sec**, at
which point the serial apply path (622k/sec) is still not the constraint.

### The wall is state size, not transaction rate

Sealing is linear in accounts, at a stable ~0.75 µs each:

| accounts | seal | state |
|---|---|---|
| 1,000 | 0.8 ms | 0.13 MiB |
| 10,000 | 7.5 ms | 1.31 MiB |
| 50,000 | 35.8 ms | 6.53 MiB |
| 1,000,000 | **~716 ms** (extrapolated) | ~137 MiB |

At a million accounts an epoch takes most of a second to close, and closing an
epoch is on the critical path for every exit proof. **That is the scaling limit
that will actually bite**, long before transactions per second do — and it is
already on the open list as §8.3 (state passing) and §8.4 (incremental
conservation). Both replace an O(state) pass with an O(delta) one.

Data availability scales the same way and stays cheap: 0.7 GiB/day at 50,000
accounts publishing every fifteen minutes.

### Comparing to other chains, honestly

These numbers are one computer's. Every mainstream chain's throughput is bounded
by **agreement**, and Zyn has none to reach — so putting them in a table
together would be measuring different things and calling it a comparison.

| | what limits it |
|---|---|
| Ethereum L1 | global consensus, gas limit |
| Solana | global consensus, leader bandwidth |
| Rollups | data availability cost, plus a sequencer |
| **Zyn today** | **signature verification on one machine** |

Zyn sits in the rollup-sequencer class: fast because one machine decides,
with the same honest asterisk. What is defensible to claim is not TPS but the
two things anchoring actually buys — **cost per trade**, since N trades amortise
into one Zcash transaction, and **settlement latency**, which is an epoch seal
rather than a proof-of-work confirmation.

Claiming a TPS number against Solana would be claiming credit for not having
solved the hard problem.

## 7g. Is the microchain actually batching? Measured.

Asked directly, and worth answering with a number rather than a claim.

### The batching is real

10,000 transfers through a node sealing every 256 actions and anchoring every
30 epochs:

```
  epochs sealed        39
  anchors produced     1
  actions per Zcash transaction   10,044
  L1 cost per action              1 zatoshi   (against 10,000 settling directly)
```

So the mechanism works and the compression is not rhetorical.
`cargo run -p zyn --example compression` reproduces it.

### The anchor goes nowhere

And this is the part that matters. `zynzapd`'s loop calls `n.anchor_now(now)`
**and discards the result**. `zyn_custody::zebra::send_raw_transaction` exists
and is called by nothing. `publishable()` and `pending()` are consumed only by
tests.

So today: *microchain* yes, *anchored to Zcash* **no**. Every anchor is a
correctly-computed artefact that is then dropped, which makes the compression
figure above arithmetic about a commitment nobody can check. The claim in every
document that state roots are committed to Zcash is, right now, aspirational.

Publishing needs a Zcash transaction, which needs the same spend assembly a
withdrawal needs — so it is blocked behind the same piece as payouts
(`RUNBOOK` §5.5), and finding that out here is the useful part: **anchoring and
withdrawals are one problem, not two.**

### Fixed while looking: the exit hatch served the wrong root

`OP_SNAPSHOT` returned `Snapshot::of(current_state)`. `publishable()` — whose
own documentation says *"the snapshot a holder needs to exit against the newest
anchored root"* — was never consulted.

They are different artefacts and only one is useful. A proof is worth something
because it opens a root that was sealed and committed; current state has been
sealed by nobody, so a holder proving against it would hold a proof of a
commitment that does not exist. Now it serves the published snapshot, and
before the first anchor it says there is nothing to serve rather than
substituting live state — substituting was the bug.

## 11. The Linera analogy, and where it stops

Proposed: *position microchain → pool microchain → app microchain → Zyn*.

### What we can already do

`zyn::host::Registry` holds applications keyed by chain id, and nesting one
chain's root as a leaf in a parent's tree is the same commitment machinery
`sections()` already uses. So the **commitment** hierarchy is cheap and the
**app microchain → Zyn** layer exists today.

### Where it stops, and why it is not a build problem

Two blockers, and only the first is about missing code.

**Cross-chain messaging does not exist** (§8.5). A position chain that cannot
affect its pool chain is a chain with nothing to do.

**An AMM does not partition, and that is the real objection.** Linera's model
works because per-user state is naturally independent — you append to your own
chain and nobody contends. An AMM is the exact opposite: *a pool is a single
contended object, and every swap touches it.* Putting positions on separate
chains parallelises nothing, because they all serialise on the pool anyway. It
buys a message per operation and no throughput.

Taking the layers one at a time:

| layer | verdict |
|---|---|
| **position microchain** | **No.** Positions in a pool share the pool's reserves. Separating them costs messages and buys nothing. |
| **pool microchain** | **Only while pools are independent.** True of pools in isolation, false the moment a route crosses two — and routing is the product. |
| **app microchain** | **Yes, and it works today.** That is `Registry`. |
| **Zyn as the root** | **Yes.** Nesting roots into one anchor is a tree. |

The pool layer is the interesting one: multi-hop routing is precisely what makes
an AMM resist sharding. Two pools on two chains means every route through both
needs atomic cross-chain execution — so the thing that would make sharding
worthwhile is the thing that makes it hard.

**Where per-chain state genuinely fits us: balances.** Holdings *are*
partitionable — a transfer touches two accounts and nothing else. A hybrid is
coherent: user chains for what people hold, one shared chain per pool for what
they trade against. That is worth revisiting once §8.5 exists, and is not worth
designing before it.

---

## 12. Privacy: what we have, and the fork ahead

### What is actually private today

| | |
|---|---|
| Deposit | **Private.** Shielded ZEC; the memo naming the account is inside the encrypted note. |
| Account identity | **Pseudonymous.** A 32-byte id. The operator can link it to a deposit via the viewing key; the public cannot. |
| Trading | **Public.** Balances are in the DA snapshot; intents are in the epoch commitment; pools and status are served openly. |
| Withdrawal destination | **Committed**, `H(address, salt)` — public sees a hash, the operator sees the address. |

So the honest description is **a pseudonymous public ledger with private on- and
off-ramps, where the operator can link everything**. For a chain whose pitch is
Zcash, that is a real gap and it should be written down as one rather than
implied away.

It is roughly SHLD's position — *"the market is public, the buyers are not"* —
with one difference that runs against us: a Zyn account is a **persistent
pseudonym**, so a user's trades link to each other whether or not anyone learns
who they are.

### The thing that cannot be fixed by encrypting harder

You cannot have private swaps **and** continuous per-trade pricing. The price
movement *is* the leak: if each swap moves the curve, observing the curve
reveals each swap. Hiding the amounts does not help when the reserves publish
the difference.

[Penumbra][pen] reached the same conclusion and answered it structurally: swap
intents are **batched by pair and cleared at the end of each block**, amounts
are additively homomorphically encrypted to a **validator threshold key**, only
the *aggregate* is decrypted, and outputs are minted privately. An individual
trade hides inside the batch, and there is no intra-block ordering left to
manipulate.

**Our architecture is already that shape.** An epoch *is* the batch. Sealing
exists for settlement compression, and it is the same mechanism a shielded AMM
needs. We even already have the other half: a **FROST threshold key** held by a
signer set — exactly what decrypts an aggregate without any one party seeing a
trade.

### The fork this forces

Shielded execution needs client-side proving. **MetaMask cannot produce a ZK
swap proof**, and neither can Phantom. So §7e — *do not build a wallet, bring
the one you have* — and shielded execution are in direct conflict.

| | Path A: public execution | Path B: shielded execution |
|---|---|---|
| wallets | MetaMask, Phantom, any | ours, with a prover |
| privacy | edges only | end to end |
| AMM | per-trade curve walking | uniform clearing price per epoch |
| MEV | ordering MEV exists | ordering MEV structurally absent |
| adoption | low friction | high friction, right audience |
| status | **built** | design fits, nothing built |

Path B also changes the AMM itself: batch clearing prices every swap in an
epoch identically, which is a *different market* from walking the curve
trade-by-trade — better on MEV, different on price impact, and incompatible
with the quote path as written (a quote would become an estimate, not the
number).

**Not decided.** Worth deciding before the wallet story is marketed, because
the two paths promise opposite things and only one of them is what a Zcash user
will assume.

The cheap partial win, available on either path: the DA snapshot publishes every
balance, which is the largest unnecessary leak we have. It exists so a holder
can exit without us — a real requirement — but publishing *commitments* plus a
proof path would serve the same purpose without publishing everyone's holdings
to everyone. That is worth doing regardless of the fork.

[pen]: https://protocol.penumbra.zone/main/dex/swap.html

## 6e. The wrapped-ZEC field, and an uncomfortable correction

Checked September 2026, prompted by [zealtoken.com][zeal].

**ZEAL** — "the first wrapped Zcash on Robinhood Chain". Its stated mechanism:
*"Every $ZEAL trade grows a public ZEC reserve, every zZEC trade burns $ZEAL,
and both are enforced by contracts with no reverse gear."*

Read carefully, that is **not** a 1:1 custody bridge. There is no deposit that
mints; a reserve accumulates from trading activity, and `zZEC` is a separate
asset whose volume burns `ZEAL`. It is a reflexive token economy with a reserve,
and its trust story is contract immutability — *no reverse gear* — where ours is
threshold custody plus an exit that works without us. Different claims, and only
one of them survives the operator disappearing. *(Read from their summary; the
tokenomics were not verifiable in depth.)*

### Two facts that matter more than the token

**Robinhood listed ZEC nationwide on 24 April 2026** — millions of verified US
users, with volume up 400% in the first hour. ZEC now has retail distribution it
did not have when this project started, which is a tailwind for anything
denominated in it.

**Robinhood Chain went to mainnet on 1 July 2026**: an Arbitrum-stack Ethereum
L2 with **100 ms block times**, aimed at tokenised assets and 24/7 markets.

### The correction

Our latency argument was written against Zcash L1's 75-second blocks, and §6c
already trimmed it once for being a swipe at the chain we build on. It needs
trimming again for a different reason: **against a modern L2 it is worth
nothing.** Robinhood Chain settles in 100 ms. "Faster than waiting for a
proof-of-work confirmation" distinguishes us from Zcash L1 and from **nobody
else**.

So speed is not the differentiator and should stop being spoken of as one.

### What is actually empty

The count of wrapped ZECs is now five that we know of — WZEC on Ethereum,
zenZEC and Zolana and an OmniBridge version on Solana, and now zZEC on Robinhood
Chain. §10.18's naming rule looks better for it, and the fragmentation argument
strengthens.

But every one of them puts ZEC on a **transparent** chain. A holder who moves
ZEC to any of them has given up the property they chose Zcash for.

That is the empty space, and it is the same fork as §12. On **Path A** we are
the sixth transparent wrapped ZEC, differentiated only by custody quality —
real, but a hard sell against a venue with Robinhood's distribution. On
**Path B** we are the only shielded one.

The field getting crowded is an argument for Path B, not against it.

[zeal]: https://zealtoken.com/

## 12b. Private pools: take the batch boundary, not the chain boundary

Proposed: put each pool on its own microchain and make that chain private.

The instinct is right and lands on the correct structure. One word in it is
carrying the wrong half.

### The part that is right: pool-scoped batches

A shielded AMM has to batch **by trading pair**, because the price movement is
the leak (§12) and a batch is the only thing that hides an individual inside an
aggregate. That is precisely what Penumbra does, and a pool is exactly the scope
at which a batch has to be drawn.

So *pool-scoped batching* is not merely compatible with privacy, it is the
requirement. Our epochs already exist; making the batch pool-scoped **within**
an epoch is a small change.

### The part that is wrong: pool-scoped chains

Chaining per pool buys the batch boundary and pays for three things it did not
have to:

**Routing dies.** A multi-hop swap crossing two pools on two chains needs atomic
cross-chain execution (§8.5, unbuilt). Penumbra does *not* shard by chain —
they batch by pair inside one chain, precisely so the router can still see all
liquidity and execute across it. Sharding the chains is the one change that
breaks routing, and routing is the product.

**The anonymity set fragments.** Privacy is a numbers game: you hide among the
other people doing the same thing at the same time. Per-pool chains cut every
anonymity set down to that pool's own traffic, and a thin pool's set is
approximately nobody. Sharding is a *cost* to privacy, not a source of it.

**Fixed overhead multiplies.** Every chain carries state, a root, an epoch and a
seal. A thousand pools is a thousand of each.

All three are avoided by batching per pool inside one chain. **Take the batch
boundary; leave the chain alone.**

### The sharper correction: hide the traders, not the pool

"Make the pool private" has an appealing sound and is the wrong target.

**A pool you can quote is a pool whose reserves you can reconstruct.** Ask for a
quote on a dust amount and you have the marginal price; ask on several and you
solve for the reserves. Any quote endpoint is an oracle into the state it prices
against, so hiding reserves means removing quoting.

And hiding them buys little, because the *aggregate* leaks anyway: a batch
clears against the curve, so the reserve delta reveals the batch's net flow
whatever else is encrypted.

So the coherent design is the opposite of the instinct:

| | |
|---|---|
| **Reserves** | **public** — they leak through quoting and through net flow regardless |
| **Individual amounts** | **hidden** — encrypted to the signer set's threshold key |
| **Who traded** | **hidden** — inside the batch |
| **Batch aggregate** | **revealed** — it must be, to move the curve |

That is Penumbra's shape, and it preserves most of what is already built:
pools, routing, reserves, and the quote path.

### What it costs us specifically

**`OP_QUOTE` weakens from a promise to an estimate.** `vm::quote` currently
shares `price_hops` with the swap handler so the two cannot disagree — a
property built deliberately, and one that only holds because execution is
sequential and the quote is the next trade. Under batch clearing, what you
receive depends on who else is in your batch, so the honest endpoint returns a
price for the *current* reserves and a note that the clearing price will differ.

That is a real loss and worth naming rather than discovering. It is also the
same loss every batch-clearing venue pays, and it buys the thing that goes with
it: **ordering MEV stops existing**, because there is no intra-batch order left
to sell.

### Verdict

- **Pool-scoped batches: yes.** This is the design, and epochs are already the
  right mechanism.
- **Pool-scoped chains: no.** Costs routing, fragments the anonymity set, and
  the batch boundary does not need a chain boundary to exist.
- **Private pools: no. Private traders: yes.** Reserves are not the secret worth
  keeping and cannot be kept anyway.

Still gated on the §12 fork — none of this is reachable from MetaMask, because
none of it is reachable without client-side proving.

## 12c. Does privacy mean dumping the AMM?

No. It means changing what the AMM is *for*.

### The mechanism that fits privacy is the batch auction

Price discovery requires revealing something; the only question is what. Going
through the options:

| mechanism | why it does or does not fit |
|---|---|
| **CFMM, per-trade** | Each trade moves a public curve, so each trade leaks. This is what we have. |
| **Public order book** | Leaks every order by construction. |
| **Sealed book** | Needs a matcher who sees the orders — an operator to trust, or MPC nobody can afford. |
| **FHE matching** | Match without decrypting. Research-grade; orders of magnitude too slow. |
| **Frequent batch auction** | Orders collected over an interval, cleared at one uniform price. Only the clearing price and the aggregate leak. |

The last one is the answer, and it is not a crypto invention — frequent batch
auctions are a well-studied response to continuous markets in traditional
finance, adopted there for the same reason we would: **continuous trading sells
an ordering advantage, and a batch has no order to sell.**

### The AMM does not go away; it stops being the matching engine

Today the curve *is* the matcher: each trade walks it in sequence. Under batch
clearing the batch is the matcher and the curve is the **liquidity of last
resort**:

1. Collect every intent for a pair over an epoch.
2. Net the two sides against each other — a coincidence of wants clears at the
   uniform price and never touches the curve at all.
3. Push only the **residual** through `out_given_in`.
4. Everyone in the batch receives the same price.

`amm.rs` survives unchanged. `out_given_in`, `mul_div_ceil`, the fee arithmetic,
`divergence_fee` — all still correct, all still used, just called **once per
batch on the net flow** instead of once per trade. Pools, reserves, LP shares
and liquidity accounting are untouched.

What is replaced is the *matching loop*, which is a much smaller thing than the
AMM.

### It buys three things, not one

- **Privacy** — an individual hides inside the aggregate.
- **No ordering MEV** — there is no intra-batch order, so there is nothing to
  sell, front-run or sandwich. This is structural, not a mitigation.
- **Better fills** — internal netting means most volume never pays curve
  slippage at all. On a two-sided pair this is a *price improvement*, not a
  privacy tax.

That third one matters for positioning: batching is usually sold as a privacy
cost. On a busy pair it is an upgrade.

### What it costs

- **Quotes become estimates** (§12b). What you receive depends on who else is in
  your batch.
- **Latency becomes the epoch**, not the intent. A trade lands when its batch
  clears — which, given §6e, we should stop pretending was our differentiator
  anyway.
- **The wallet fork** (§12). Client-side proving is not something MetaMask does.

### The Zcash-native option worth watching

If **ZSAs** ship (§6c), shielded assets exist at L1 and the balance layer could
be Zcash's own shielded pool rather than one we rebuild. Zyn would then be the
*matching and execution* layer over assets that are already private, settling
net positions rather than custodying and re-shielding them.

That is the one design in this document that no other wrapped-ZEC project can
copy, because it requires being built on Zcash rather than pointed at it. It is
also entirely dependent on someone else's roadmap, which is why it is a thing to
watch rather than a thing to plan.

### Verdict

Keep the AMM, demote it from matcher to liquidity source, and replace the
matching loop with a per-pool batch auction. That is a real rewrite of the
execution path and a small one compared to "dump the AMM" — and it is the same
answer §12b arrived at from the other direction, which is mild evidence it is
the right one.

## 12d. Speed under batch clearing

The batch design moves the expensive work, and it mostly moves it somewhere
cheaper. Worth laying out where the time actually goes before optimising
anything.

### The structural win: crypto goes per-batch, not per-trade

Today every intent costs a signature verification (measured: 115 µs for
secp256k1). Under batch clearing the *additional* privacy crypto does **not**
scale with trades:

| | per trade | per batch |
|---|---|---|
| verify the submitter's signature | ✓ | |
| homomorphic add of the encrypted amount | ✓ (cheap: one group op) | |
| **threshold decryption of the aggregate** | | **✓ once per pool** |
| **curve evaluation** | | **✓ once per pool** |

The two expensive things — a distributed signing round and the AMM math — happen
**once per pool per epoch** rather than once per trade. A batch of 500 swaps
pays for one threshold decryption, not 500.

That is the opposite of the usual expectation that privacy multiplies cost.

### The user's proof gets smaller, not bigger

A shielded swap proof has to show: *I own this input, the amount is in range,
and the ciphertext is well-formed*. It does **not** have to prove the curve was
followed — the curve is evaluated once, publicly, on the aggregate, by everyone.

That is the difference between a circuit a browser proves in a second and one it
cannot. It is also an argument for batching independent of privacy: putting the
AMM math inside every user's proof would be the expensive design.

### Batching parallelises better than sequential execution

Sequential swaps serialise on pool state — that is why the apply path is the one
thing in §7f that cannot be threaded. Batches do not: **pools are independent
within an epoch**, so clearing runs across every pair concurrently, and only the
final state write is serial. The workload becomes embarrassingly parallel
exactly where it used to be strictly serial.

### The levers, in order of effect

1. **Per-pool adaptive batch intervals.** Latency to fill *is* the batch
   interval, and it trades against the anonymity set: too short and a batch
   holds one trade, which hides nobody. `EpochPolicy` is already the right dial
   — *seal after N intents or T seconds* — and it needs to become **per pool**,
   so a busy pair batches on count and a quiet one on time. This is the largest
   lever and mostly a matter of moving a knob we already built.
2. **FROST nonce preprocessing.** Signers generate nonce commitments in advance,
   turning the signing round trip from two into one. Roughly halves the
   threshold latency, which is otherwise the dominant network cost.
3. **Pipelining.** Decrypt batch *N+1*'s aggregate while batch *N* executes.
4. **Parallel clearing across pools**, per above.
5. **Client proving in WASM with workers.** The circuit is small by
   construction; this is about not being single-threaded in a browser.

### What is not a lever

Chasing sub-second fills. Latency to fill is the batch interval, and shrinking
it below the point where batches are thinly populated buys speed by giving away
the privacy the design exists for. §6e already established that speed is not our
differentiator against a modern L2; this is the second place where trying to
compete on it costs something real.

### One thing to get right slowly

The **uniform clearing price** is a fixed point: the residual that reaches the
curve depends on the price, and the price depends on the residual. It has a
closed form for constant product, and getting it subtly wrong is getting the
market wrong — every participant mispriced, silently, in a way that balances.

That derivation deserves its own pass with the arithmetic checked the way
`amm.rs` was, not a first attempt bolted onto a batching loop.

## 12e. What batch clearing does to the things that integrate with a pool

A market-design change is paid for by integrators, so it is worth going through
them one at a time rather than asserting the trade is good.

The pattern that emerges: **batch clearing helps everything that reads a price
and hurts everything that needs a firm quote right now.**

| integration | verdict |
|---|---|
| Price oracle | **better** |
| LP economics (LVR, JIT) | **better** |
| Limit orders | **better** — stops needing a keeper |
| Front-running, sandwiching | **broken, deliberately** |
| Swap widget | adapt |
| Yield / auto-compounding | unaffected |
| Launchpad graduation | unaffected |
| Portfolio analytics | adapt — and shrinks |
| Payment / checkout swap | adapt, if batches are short |
| **Liquidations** | **worse, and it is the real problem** |
| **Aggregator routing** | **worse — a strategic cost, not a technical one** |

### Better

**Price oracles.** Reserves stay public (§12b) and update once per batch. That
is a *harder* number to manipulate than a spot price: flash-loan oracle attacks
work by moving the price and reading it inside one transaction, and batch
clearing makes that impossible — your manipulation goes into the batch with
everyone else's and clears at the same price they get. Batching is an oracle
defence, not merely compatible with one.

**LP economics.** LVR is loss to arbitrageurs who pick the moment. Batching
removes the moment: an arbitrageur still corrects the price, but cannot choose
to do it at the instant most profitable to them. This *reduces the LVR itself*,
where `divergence_fee` only recaptures some of it after the fact — the two
compose rather than compete.

**JIT liquidity** stops working for the same reason. An LP cannot add
immediately before a large trade and withdraw immediately after if there is no
"immediately before" inside a batch.

**Limit orders become native.** `min_out` already *is* a limit price. Under
sequential execution an unmet limit means a rejected intent; under batch
clearing it means simply not being included in that batch. The feature stops
needing a keeper, a third party, or a separate order type.

### Users adapt

**Swap widgets.** "You will receive X" becomes "approximately X", and the fill
arrives a batch later. The failure mode improves: instead of a revert, an unmet
limit is a non-fill, which is a better thing to explain to a user.

**Analytics.** Aggregate volume and TVL stay public; individual balances do not.
A dashboard showing *your* portfolio still works — you hold the viewing key. A
dashboard showing *someone else's* stops working, which is the point.

**Checkout swaps.** Fine at a one-to-two second batch, bad at sixty. This is the
strongest argument for the per-pool adaptive intervals in §12d.

### Genuinely worse

**Liquidations, and this is the one that matters.** A lending protocol that
liquidates by swapping collateral now waits for a batch. In a fast drawdown that
delay is bad debt, and no amount of getting used to it helps — the loss is real
and lands on someone who did nothing wrong.

Nothing here solves it. Partial answers: tighter batch intervals under stress,
liquidation paths that settle against a reserve rather than a swap, or simply
not hosting lending against batch-cleared collateral. **It should be decided
before a lending protocol builds on us, not after.**

**Aggregator routing.** Aggregators want a firm quote to route against, and a
batch cannot give one. See §12f — the answer is to quote a **range**, which is
built.

### The summary worth keeping

Every integration that suffers is one that wants **execution at a chosen
instant**. Every integration that improves is one that wants **a price that
nobody could choose the instant of**.

That is the same trade in both directions, and it is exactly the trade the
privacy design is making anyway. It is coherent — but liquidations are where it
costs someone real money, and that deserves an answer rather than a shrug.

## 12f. Answering the two unacceptable ones

§12e's "adapt" cases are an acceptable trade. The two marked *worse* are not,
and they have different answers: one is ours to solve, the other turns out not
to be a Zyn problem at all.

### Aggregators: quote a range, not a point — **built**

A batch cannot promise an exact fill, but it can promise **bounds**, and bounds
are what a router actually compares.

`vm::Quote` now carries both ends:

| | |
|---|---|
| `amount_out` | what this route pays executing **alone** against these reserves — the sequential price |
| `best_case` | the **marginal** rate with fees but no slippage: what a perfectly netted batch pays when the pool never sees a residual |
| caller's `min_out` | a floor the caller sets |

The gap between the two *is* price impact, which is the number a router needs to
rank venues. Both cross the wire on `OP_QUOTE`.

**And our floor is worth more than theirs.** On a sequential AMM `min_out` is a
number a sandwich can push you to exactly; under batch clearing an unmet limit
means **no fill**, not a fill at the worst tolerable price. So the honest
comparison is not "firm quote versus vague quote" — it is *a narrow estimate
that can be violated* against *a wider estimate with a floor that cannot*.

Fees are charged at both ends, deliberately: netting removes price impact, not
the cost of using the pool, and a ceiling quoted without the fee would be a lie.

The strategic shape still stands: CoW Protocol is a batch auction with wide
integration, reached by *being* the aggregator rather than being aggregated. A
venue solvers route **into** rather than one a router quotes **out of**.

### Liquidations: change the liquidation, not the chain

The instinct to change Zyn at a lower level — a privileged fast lane for
liquidations — should be resisted. A lane that skips the batch reintroduces
ordering, and a *privileged* lane reintroduces it for whoever can get their
intent classified as a liquidation. It would recreate exactly the MEV surface
batching removes, at the point of maximum stress.

The better answer is that **instant market-sell is already the wrong liquidation
design**, and the industry moved off it for reasons unrelated to privacy.
Maker's Liquidations 2.0 and Aave v3 both replaced instant sale with a **Dutch
auction**, because dumping collateral into a falling market causes the cascade
it is trying to prevent.

A Dutch auction is **batch-native**. It does not need execution at an instant;
it needs a price that decays over time until someone takes it — and a batch
cadence is a decay step. Bidders submit into batches like everyone else. No
privileged lane, no ordering advantage, and a mechanism that is better practice
regardless.

For the tail — a drawdown faster than the auction can clear — the answer is a
**backstop reserve**: the lending protocol holds a buffer of the debt asset,
liquidation settles against it immediately at an oracle price, and the reserve
rebalances through an ordinary batch afterwards. The reserve carries the timing
risk, which is what a backstop is for.

Both of those are **lending-protocol design**, not Zyn design. Which is the
useful conclusion: the one integration that looked broken is fixed by the
liquidation mechanism people already prefer, and needs nothing from the chain.

### What is still owed

Triggering still needs a price, and a batch price is one epoch stale. That is
fine — DeFi oracles are already delayed and ours is *harder to manipulate*
(§12e) — but "stale but honest" is a claim to check against a real liquidation
model before anyone lends on it.

## 13. What to build next, and why it is not Solana

### The argument against Solana next

A second bridge adds a second asset to a system **the first asset cannot leave**.
Withdrawals were not built when this was written (they are now: RUNBOOK §4f),
so at the time ZEC.zy was a one-way door — and shipping SOL.zy would have
produced two of them.

Three more reasons, none of which is "Solana is hard":

- §6b's own ordering rule says `ZEC.zy` first and everything else after.
  `ZEC.zy` is not finished; deposits work and exits do not.
- §6e put us sixth in a crowded field of transparent wrapped ZECs. Another asset
  does not change that. **Working custody does**, and custody that cannot pay
  out is not custody.
- Solana custody is a *second build*, not a port: ed25519 is a different FROST
  ciphersuite from RedPallas, so the ceremony and signing are new code even
  though the watcher state machine transfers.

### The two things actually blocking

Both are "put a transaction on Zcash", which nothing in this workspace can do.
§7g called them one problem. On closer reading they are one *requirement* and
two very different builds:

| | what it needs | size |
|---|---|---|
| **Anchoring** | a transaction carrying 32 bytes | small |
| **Withdrawals** | an Orchard spend: note selection, tree witnesses, a proof, a bundle | large |

That split is the useful part, because **anchoring can ship first and it makes
the central claim true.** Every document here says state roots are committed to
Zcash. That is currently false (§7g): the anchor is computed and discarded.

### Recommended order

**1. Anchoring, via a transparent transaction.**

A root is public, so it does not need shielding. A transparent Zcash spend is
secp256k1 and Bitcoin-shaped — no proofs, no note tree — and `k256` is already
in the workspace. The approval path exists too: `Certificate` and `SignerSet`
already do ed25519 *k*-of-*n* over an anchor, so the threshold decision is built
and only the broadcast is missing.

Two things to check before committing to it: **ZIP-244 sighash** for v5
transactions is fiddly and must be exactly right, and **transparent testnet
funds** have to come from somewhere — the faucets found in §4c send to unified
and Sapling addresses, so this may need asking.

**2. Spike the two Orchard unknowns before building anything.**

- **The binding signature is not threshold-signed.** It is derived from value
  commitments, so FROST covers spend *authorisation* and not the whole payout.
  Confirm the split first; it changes what the signer ceremony has to produce.
- **Note commitment tree witnesses.** Spending needs each note's position and
  authentication path in Orchard's tree. Whether Zebra can serve those, or
  whether we must build and maintain the tree ourselves, is the largest unknown
  in the project. Answering it may reshape the plan.

A day on these two is worth more than a week of building around a wrong guess.

**3. Orchard spend assembly**, once they are answered. Withdrawals, and with
them the exit hatch stops being half a hatch — a holder can already prove what
they are owed (§7g fix) and still cannot collect it.

**4. Then Solana**, which by that point reuses a proven daemon, watcher,
settlement path and RPC, and adds a signer set.

### What this order buys

After (1) the chain is genuinely anchored and the L2 claim is true. After (3)
money can leave, which is when it is honest to let anyone put money in. Only
then does a second asset add anything, because only then is the first one a
round trip.

## 13a. The two Orchard unknowns, answered

Both were spiked before building anything, per §13. Both came back favourable,
and one of them changes what the signer ceremony has to do — which is exactly
why it was worth an hour rather than a week of building around a guess.

### 1. The binding signature is not a threshold problem

From `orchard-0.15.5/src/builder.rs`:

```rust
let bsk = pre_actions.iter().map(|a| &a.rcv)
    .sum::<ValueCommitTrapdoor>()
    .into_bsk();
```

The binding signing key is **the sum of the value-commitment randomness across
the bundle's actions**. It is not a persistent secret at all — it is derived
fresh, per transaction, from randomness *the builder itself chose*.

So the split is clean, and cleaner than feared:

| | key | who signs |
|---|---|---|
| **Spend authorisation** | `ask` / `ak` — persistent | **FROST**, the signer set |
| **Binding signature** | `bsk` — ephemeral, per transaction | **the builder**, alone |

`zyn-custody`'s ceremony produces exactly what a payout needs and nothing is
missing. The protocol is:

1. the builder assembles actions and the proof, and computes the sighash
2. one FROST round over that sighash yields the spend-authorisation signatures
3. the builder adds its own binding signature
4. broadcast

Knowing `bsk` confers nothing: it binds the value balance, it does not authorise
a spend. A builder cannot move funds without the threshold's signatures. What a
builder *does* see is the bundle's contents, which makes it a privacy role
rather than a custody one — worth naming, and not the same risk.

### 2. Zebra serves the commitment tree, so we do not rebuild it

The larger unknown was whether note witnesses were obtainable at all. **They
are.** Tested against the live testnet node:

```
z_gettreestate 4095000
  → { orchard: { commitments: { finalRoot, finalState } }, sapling: {...} }
```

Zebra implements it and returns the **Orchard frontier** at any height. So the
path is the ordinary wallet one: take the frontier at the vault's birthday
height, then append every Orchard commitment from every block after it, tracking
the positions of notes that are ours.

`incrementalmerkletree 0.8.2` is already in the dependency graph via `orchard`,
so the machinery arrives with a crate we already depend on. Nothing has to be
built from genesis and nothing has to be reimplemented.

### 3. And so the anchoring plan in §13 was wrong

§13 recommended shipping anchoring first, via a transparent transaction, on the
reasoning that *a root is public so it does not need shielding*. That is true
about the **data** and false about the **plumbing**, which is the part that
costs anything.

Checked against the crates:

- `zcash_transparent`'s builder exposes `add_output(&TransparentAddress, value)`
  and **nothing that takes a raw script**. An `OP_RETURN` output is not
  reachable through it, so carrying 32 bytes transparently means constructing
  the transaction outside the builder and implementing **ZIP-244** sighash by
  hand — precisely the bit-exact work the builder exists to avoid.
- It would need **transparent testnet funds**, and the faucets found in §4c send
  to unified and Sapling addresses. Spending shielded funds transparently
  requires a shielded spend, which is the thing being deferred.
- It would be a **second transaction-construction implementation** in a codebase
  that is about to have one.

Meanwhile an Orchard anchor is a **self-send carrying the root in a memo** —
the same machinery as a withdrawal with one field different. One builder, one
sighash, one signing protocol, two uses.

**Revised order:** the spikes replace the transparent anchoring step rather than
preceding it. Go straight to Orchard spend assembly; anchoring falls out of it.

Worth noting why §13 got it wrong, since the mistake is a reusable one: it
scoped a task by the size of its *payload* rather than the size of its
*dependencies*. Thirty-two bytes is small; being able to put thirty-two bytes on
Zcash is not.

### What this changes

Orchard spend assembly is **more tractable than §13 assumed**. The two things
that could have made it a research project — a threshold scheme the ceremony
does not produce, and a Merkle tree we would have to maintain from scratch —
are both answered by things that already exist.

It is still the largest remaining build. It is no longer an unknown one.

## 13b. The ceremony could not have signed a spend

Found while building the spend path, and it is the kind of thing that would have
survived every test we had.

### The gap

An Orchard action is authorised under `rk = ak + alpha·G`, not under `ak`. Each
action carries **its own randomizer**, chosen by the bundle's builder.

`zyn-custody::signing` used plain FROST. It produced a perfectly valid
signature — under the group key — which an Orchard bundle rejects. **The
ceremony would have completed, the signature would have verified against
everything we tested it against, and the vault could not have spent anything.**

Re-randomization is not an implementation detail of Orchard. It is what stops an
observer linking two spends by the key that authorised them, which is a privacy
property, and it is why `reddsa` ships a separate `rerandomized` module at all.

### The fix

`signing::orchard` signs with the action's `alpha`:

```rust
frost_rerandomized::sign(package, session.nonces(), &keys.key_package, alpha)
```

`alpha` comes from the PCZT a signer is asked to authorise
(`spend.alpha()`) — never generated locally. `Randomizer` is a free scalar
rather than one derived from the signing package, because Orchard's builder
picks it and a signer only ever receives it.

Aggregation takes `RandomizedParams::from_randomizer(group_key, alpha)`, whose
own documentation says to use it *"unless your application requires generating a
randomizer outside"* — which is precisely this case.

### Proven against the check Orchard actually makes

`orchard::pczt::Action::apply_signature` verifies a signature against the
action's `rk` before accepting it. The tests verify against exactly that key, so
passing here is passing there:

- a threshold of signers authorises a spend randomized by the bundle's `alpha`
- the **group key does not verify it** — the plain path really would have failed
- a signature does not carry to another action's `alpha`, nor to another sighash
- one signer short produces nothing, not something weaker

### What remains for a payout

The cryptography is answered. What is left is assembly, and it is ordinary:

1. a **note store** — which notes the vault holds, their positions, spentness
2. **witness maintenance** — the frontier from `z_gettreestate` (§13a) plus
   `incrementalmerkletree`, already in the graph
3. **bundle construction and the proof** — `orchard::builder`, and a proving key
4. **the PCZT round** — read `alpha` and `rk`, FROST-sign, `apply_signature`
5. **broadcast** — `send_raw_transaction`, written and never yet called

None of those is an unknown any more. Step 4, the one that could have been a
research problem, is done and tested.

## 13c. Orchard spend assembly, built

A withdrawal is assembled, the signer set authorises it, and `orchard` accepts
the result. `zyn-custody::notes` and `zyn-custody::payout`, 11 tests.

### Three findings, each one silent

**The vault's Orchard identity has to *be* the threshold key.** An FVK is
`ak ‖ nk ‖ rivk`, and `ak` is the spend *validating* key — which is exactly the
group verifying key a DKG produces. `ceremony::orchard_viewing_key` builds the
FVK from it via `FullViewingKey::from_bytes`.

Without that, notes arrive at an address whose `ak` is unrelated to the signer
set: the deposit works, the scan works, the balance is right, and the first
withdrawal cannot be signed. `nk` and `rivk` are chosen rather than derived
(§13a) and grant viewing only, so all signers and the watcher may hold them —
but they must be generated once and distributed with the shares.

**A signer must not sign every action.** A bundle is padded with dummy actions
to hide how many real spends it contains. Each dummy carries its own randomizer
*and its own key*, already signed by the builder. Asking the signer set to
authorise them produces signatures the bundle rejects — which is what happened,
and the error said only "bad signature".

`needs_signatures` filters on `rk == group_key.randomize(alpha)`, the same
comparison `Action::sign` makes internally. So a signer is not merely *told*
which actions are the vault's; it can check.

**A checkpoint marks the start of a block, not the end.** `bridgetree` rewinds
to the state a checkpoint was taken *at*, so checkpointing after a block
restores it rather than undoing it — measured: two appends with a checkpoint
after each needed **two** rewinds to undo one. The API is `begin_block`, named
so it cannot be called at the wrong moment.

### What is proven

- a held note's authentication path opens the tree's anchor, with other
  people's commitments before and after it
- skipping a commitment moves every later note, so paths verify against nothing
- 2-of-3 signers authorise a real withdrawal, and `orchard` accepts it
- a signature under another action's randomizer is **refused by the action**,
  not by the network
- change returns to the vault as an output; a payout can never become a fee
- a reorganised-away block takes its notes with it

No zk proof is generated in the tests, and that is not a gap:
`apply_signature` checks against `rk` and nothing else, so the tests exercise
precisely the step that could be wrong. The proof is expensive, standard, and
`orchard`'s to get right.

### Two things still open

**`BundleVersion::orchard_v3` mandates the cross-address restriction**, and a
payout is inherently cross-address: it spends the vault's note, pays somebody
else, and returns change. Under v3 that bundle cannot be built at all, so the
code uses `orchard_v2`. Whether a v3 payout is expressible some other way is a
**consensus** question, not an API preference, and it has to be answered before
mainnet rather than guessed at.

**The transaction envelope.** The bundle is complete; wrapping it in a v5
transaction, computing the real sighash — the tests use a placeholder, because
a sighash covers the whole transaction and not the Orchard bundle alone — and
broadcasting through `send_raw_transaction` is what remains.

## 14. The bridge, stated plainly

Several models were studied — NEAR's chain signatures, ICP's ckBTC, Zenrock's
dMPC, SHLD's published address, ZEAL's reserve. This is where we landed and
why, in one place, because the answer has been assembled a piece at a time.

### The field

| model | trust | why not for us |
|---|---|---|
| **Single custodian** (SHLD) | one party | the thing we exist to be better than |
| **Threshold / MPC** (zenZEC, ckBTC) | *k* of *n* | **this is ours** |
| **Light client** (IBC-style) | the other chain's consensus | verifying Zcash consensus cheaply is not a solved problem; proof-of-work plus shielded validity is not an IBC header |
| **Chain signatures** (NEAR) | an MPC network you do not run | the same trust model as ours, outsourced to someone else's signer set |
| **Reflexive reserve** (ZEAL) | contract immutability | not a bridge — no deposit mints anything |

So: **threshold custody**, which is the practical state of the art and what the
only comparable shielded-ZEC bridge also chose. We are not claiming a novel
trust model, and a design that claimed one here would be worth distrusting.

### What we add to it, and why it needs a chain to add

Threshold custody answers *who can spend the vault*. It does not answer *how
many units may exist*, and that second question is where bridges actually fail.
Three answers, and all three are only available because we are running the chain
rather than deploying a contract on someone else's:

**1. Minting is bounded by observation.** `AttestVaultBalance` records what the
vault was seen to hold; `CreditDeposit` cannot exceed it. An operator who
credits a deposit that never arrived is refused by the machine, not caught by an
audit.

**2. Conservation is a precondition, not a report.** **S11**: `supply ==
backing` is checked against a scratch copy of the state, and a batch that breaks
it is **discarded before it becomes the state**. Not logged, not alerted —
discarded. A bridge contract on another chain can emit an event when its books
disagree; it cannot refuse to have disagreed.

**3. Credits are provisional until anchored.** A deposit is credited
*unspendable* and released when the epoch containing it reaches Zcash, so a
reorg cannot leave units behind that nothing backs.

### And an exit that does not need us

The part that separates custody from a promise:

- every anchored epoch publishes a **DA snapshot** of balances, so a holder can
  prove what they hold without asking us
- an exit that is committed and not paid is **refundable after
  `exit_timeout_epochs`**
- a payout destination is **bound in state**, and redirecting it waits out that
  same timeout — so a stolen warm key costs trading control, not the balance

**Honest status: this is half-built.** A holder can prove what they are owed
(§7g) and cannot yet collect it, because the payout path is assembled and tested
(§13c) but not yet wrapped in a transaction and broadcast. Until that closes,
"exit without us" is a design, not a property.

### Why this shape suits Zyn specifically

**RedPallas FROST is Zcash's own scheme.** Orchard spend authorisation *is*
RedPallas, so the threshold signing is the protocol's native primitive rather
than an adapter around it. The vault's address is literally the group key
(§13c). Nothing is wrapped, and there is no second cryptosystem to get wrong.

**Being the chain is the leverage.** Every one of the three additions above is
something a VM can enforce and a contract cannot. That is the actual argument
for a microchain existing at all, and it is a stronger one than speed (§6e).

**The custody story is the differentiator, not the asset.** There are five
wrapped ZECs (§6e). Being a sixth is worth nothing; being the one whose supply
cannot exceed its backing by construction is worth something — and only if the
exit works, which is why that is next rather than Solana (§13).

### The endgame that would replace all of it

If **ZSAs** ship, shielded assets exist at L1 and the balance layer becomes
Zcash's own pool rather than one we custody. Zyn would be matching and execution
over assets that are already private, settling net positions instead of holding
anyone's ZEC.

That is the only design in this document a wrapped-ZEC project on another chain
structurally cannot copy, and the only one where the honest answer to "what do I
trust you with" is *nothing*. It depends entirely on somebody else's roadmap,
which is why it is written down as a direction and not a plan.

## 14a. Does it generalise? Checked against each target.

> **The Ethereum row is superseded by §14d.** The custody framing below is the
> wrong half of the problem; deposits, not signatures, are what force the design.

The model is threshold custody plus VM-enforced conservation (§14). Those two
halves generalise very differently, and the split is the whole answer.

### The custody half is per-chain, and one target does not fit

FROST produces **Schnorr** signatures. Whether a chain accepts one is not a
matter of effort:

| chain | scheme it accepts | FROST | status |
|---|---|---|---|
| **Zcash** (Orchard) | RedPallas | `reddsa`, re-randomized | **built and tested** (§13b, §13c) |
| **Solana** | Ed25519 | `frost-ed25519 v3` | available; a new ceremony |
| **Bitcoin** | Schnorr, BIP-340 | `frost-secp256k1-tr v3` | available; **Taproot only** |
| **Ethereum** | **ECDSA** | ✗ **impossible** | needs a different mechanism |

All four ciphersuites exist at v3.0.0, the same major as the
`frost-rerandomized` already in the tree, so they compose rather than fork the
dependency graph.

**Bitcoin works only through Taproot.** A legacy or SegWit-v0 vault is ECDSA and
out of reach; a P2TR vault is a BIP-340 Schnorr key and FROST signs it directly.
That is a constraint on the address, not on the design.

**Ethereum cannot be reached this way at all.** An EOA is authorised by ECDSA,
there is no Schnorr precompile, and FROST does not produce ECDSA. Two ways
round it, and only one is sensible:

- **Threshold ECDSA** (GG18/GG20-style) — a genuinely different protocol, harder
  to implement, with a worse security record than FROST. Adopting it for one
  chain would put the least-trustworthy cryptography in the system on the chain
  with the most value at stake.
- **A contract vault** — *k*-of-*n* approvals verified on-chain, as a Safe does.
  Different mechanism, **identical trust model**, ordinary and audited.

So: contract vault for Ethereum. Worth stating plainly that this is not the same
code path — Ethereum custody would share the accounting and share nothing of the
signing.

### The valuable half generalises for free

Everything that makes this better than a custodian lives in the VM and does not
care where an asset came from:

- the attested ceiling — `AttestVaultBalance` bounds minting by observation
- conservation — **S11**, a batch that unbalances the books is discarded
- provisional credits until anchored
- exits, refunds, destination binding, settlement grouping

`zyn-bridge` is already chain-agnostic and `ChainOrigin` is a `u16`. Adding a
chain is a vault and an asset, not a subsystem (§10.17).

### The watcher generalises too, which I had doubted

§11 and the Solana notes claimed `ChainView` encoded a Zcash assumption by
modelling finality as **confirmation depth**, and that Solana's commitment
levels would not fit. **That was wrong, and it is now tested.**

A chain with explicit finality reports its **finalized height as the tip** and
asks for **zero confirmations**. The depth model is not incorrect there, it is
unnecessary, and the degenerate case works — `zero_confirmations_credits_a_finalized_tip`
alongside `depth_still_holds_back_a_shallow_deposit`, one trait serving both
with no flag deciding which chain it is.

Observation difficulty runs the other way from custody, which is convenient:
Zcash needs trial decryption and is the hardest, and it is done. Solana, Bitcoin
and Ethereum are transparent — an address query, a UTXO scan, a log filter.

### What a new chain actually costs

| | Solana | Bitcoin | Ethereum |
|---|---|---|---|
| ceremony | new (ed25519) | new (secp256k1-tr) | none — a contract |
| watcher | easy | easy | easy |
| accounting, exits, conservation | **free** | **free** | **free** |
| constraint | — | Taproot addresses only | separate signing path |

One more thing to settle before the second vault (§10.12): a signer *set* is
people, a signer *key* is a ceremony. The same people can run several
ceremonies, one per chain — which is almost certainly right, since sharing one
key across chains would mean a bug in one chain's signing path reaches every
vault.

## 14b. A microchain per bridge

Same analogy as §11, applied to bridges instead of pools — and this time it
passes the test that pools failed.

### Why it passes where pools did not

§11 rejected pool-per-chain because **an AMM does not partition**: a pool is one
contended object, every swap touches it, and routing crosses pools constantly.

Bridge state is the opposite. A Zcash deposit touches the Zcash vault and
nothing else. A Solana deposit touches the Solana vault and nothing else. **No
operation spans two bridges.** That is precisely the independence Linera's model
wants, and bridges have it naturally.

The second difference matters as much. Pool-per-chain would have put cross-chain
messaging on the **hot path** — every routed swap. Bridge-per-chain puts it on
**deposit and withdrawal only**: twice in a user's lifetime, not twice a minute.
And those are already asynchronous and already slow, since a deposit waits for
confirmations regardless. A message that costs an epoch is invisible next to a
wait that costs minutes.

### What it buys

**Fault isolation, and this is the real prize.** Conservation (**S11**) is
checked over the *whole state*: if any asset's books fail to balance, the batch
is discarded — including every unrelated intent in it. Today a bug in a Solana
watcher would halt Zcash trading. Split, a Solana problem halts Solana.

**Per-chain signer sets become structural.** §14a concluded one ceremony per
chain; a chain per bridge makes that the shape rather than a convention someone
has to remember.

**Independent cadence and independent pause.** A busy bridge seals often, a
quiet one rarely, and one can be halted without halting the exchange.

### What it costs, and it is the thing we are proudest of

**Conservation stops being one machine's precondition.**

§14's whole argument is that `supply == backing` is checked *before* a batch
becomes state, by a machine that refuses to have been wrong. That works because
one VM holds both numbers. Put the vault on chain A and the minted units on
chain B and **neither can check it alone**.

The recoverable version is two local invariants plus a correct handoff: the
bridge chain conserves `supply == backing`, the exchange chain conserves its
balances against what the bridge issued it. That composes — it is how every
L1↔L2 bridge works — but it is weaker. "One machine cannot commit an unbalanced
state" becomes "two machines are each locally consistent, and the transfer
between them had better be right".

Losing that would trade away the sharpest thing in §14 for isolation we do not
need yet.

### Verdict: right design, wrong time

Blocked on cross-VM composition (§8.5) like everything else in this family. But
the more useful gate is simpler:

**With one bridge, this buys nothing.** There is nothing to isolate a single
bridge from. The fault-isolation argument — the only one strong enough to pay
for weakened conservation — starts at the **second** bridge.

So: revisit when a second vault is live *and* §8.5 exists. Until then it is cost
without benefit, and the cost is the property the whole design is sold on.

### The reusable test

Three times now the analogy has been applied and the answer has come from one
question: **does this state partition, and how often does an operation cross?**

- **positions** — no partition, crossings constant → no
- **pools** — partition, but routing crosses constantly → no
- **bridges** — partition, and crossings are twice a lifetime → **yes, later**

Worth keeping, because it settles the next case without another argument.

## 14c. Does a microchain solve the Ethereum problem? No.

> **Conclusion stands, premise superseded by §14d.** Threshold ECDSA emits a
> standard signature, so the Schnorr trick weighed here is not needed at all.

**No, and it cannot.** The incompatibility is that an Ethereum EOA is authorised
by **ECDSA** and FROST produces **Schnorr**. That is a fact about what the EVM
verifies. Where our accounting lives — one chain, four chains, forty — changes
nothing about it. Chain topology cannot make a verifier accept a different
signature scheme.

What a separate chain *would* buy is that Ethereum's different custody path
stays out of the shared VM (§14b). Organisational, useful, and not a solution.

### But the question exposed a better option than §14a gave

§14a proposed a Safe-style contract: *k* ECDSA signatures verified on-chain.
That is the conventional answer, and there is a sharper one.

**Schnorr can be verified on Ethereum cheaply.** The `ecrecover` precompile can
be coerced into doing the elliptic-curve work, so a Schnorr signature verifies
for roughly **5–13k gas** instead of the cost of implementing curve arithmetic
in Solidity. This is not theoretical — Chainlink has `SchnorrSECP256K1`
deployed on mainnet, and several bridges use the same trick.

That would mean **one aggregated FROST signature** on Ethereum rather than *k*
separate ones: cheaper, and the signer set's size stays invisible on-chain,
which a multisig leaks by construction.

### And why it is still the wrong choice

The trick requires a **modified challenge**: the scheme uses `address(k·G)` in
place of `k·G` when computing the challenge and when verifying. Standard
`frost-secp256k1` does not do that, so its signatures will not verify against
such a contract.

Making it work means writing a **custom FROST ciphersuite** — a new challenge
function inside a threshold signature scheme. That is hand-rolled cryptography,
in the highest-value place in the system, to save gas.

So the recommendation stands, and now for a reason rather than by default:

| | new cryptography | gas | signer set |
|---|---|---|---|
| **Safe-style *k*-of-*n* ECDSA** | **none** | *k* verifications | visible |
| FROST + `ecrecover` Schnorr | **a custom ciphersuite** | one, ~5–13k | hidden |
| Threshold ECDSA (GG20) | a different protocol, worse record | one | hidden |

**The middle row is the tempting one and the first row is the right one.** The
saving is gas and a privacy nicety; the price is bespoke threshold cryptography
guarding the chain with the most value on it. If that ciphersuite is ever
standardised and audited, this decision should be revisited — it is a good idea
waiting on somebody else to make it safe.

### What this means for ordering

Ethereum is the most expensive chain to add: no shared ceremony, no shared
signing path, and a smart contract to write and audit — the only target that
brings its own audit surface. Solana before Ethereum, which was already the plan
(§13), now has a second reason behind it.


## 14d. EVM: a receiver contract. Supersedes §14a and §14c.

§14a treated Ethereum as a custody problem and picked a Safe by elimination.
§14c looked for a cheaper signature. **Both were solving the wrong half.** The
question "what if we deploy a receiver vault" reframes it, and the reframing is
correct.

### An EOA cannot accept a deposit, and the reason is not custody

A deposit has to say **who it is for**. On Zcash we get that from the shielded
memo. On an EVM chain, a transfer to an EOA carries no memo and no calldata —
an ERC-20 `transfer` emits `Transfer(from, to, value)` and nothing else. The
only thing a watcher could key on is the **sender's address**.

That would mean a user's Zyn account is derived from, or permanently linked to,
their Ethereum address. On a chain whose reason to exist is that trades are
shielded, publishing an on-chain link from every depositor's public EVM identity
to their Zyn account is not a compromise. It is the whole leak, at the front
door, for free.

A contract fixes it because **calldata exists**:

```solidity
function deposit(bytes32 zynAccount, address token, uint256 amount) external;
function depositNative(bytes32 zynAccount) external payable;
event Deposited(bytes32 indexed zynAccount, address token, uint256 amount, uint64 index);
```

The destination is an argument. It can be an account the depositor has never
otherwise touched. The watcher reads one canonical event instead of inferring
intent from a bare transfer.

Four things come with it that an EOA cannot do at all: rejecting an unsupported
asset at deposit time rather than stranding it; measuring the **balance delta**
so fee-on-transfer and rebasing tokens credit what actually arrived; a
monotonic per-asset `index` that lines up with the node's existing
`next_deposit_index`; and a pause.

### And it makes the signature question evaporate

Funds sit in the contract, and a withdrawal is a function call carrying a
signature. Verification is:

```solidity
SignatureChecker.isValidSignatureNow(signer, digest, sig)
```

which accepts **both** an ordinary ECDSA signature via `ecrecover` (3,000 gas)
and a contract signer via **EIP-1271**. So `signer` can be:

- a **Safe**, k-of-n, today — ordinary, audited, and its owner set is rotatable
  without changing the Safe's address; or
- a **threshold-ECDSA** address later.

The second is now genuinely available: **`cggmp21` 0.6.3**, maintained under
Linux Foundation Decentralised Trust, audited by Kudelski, with key refresh and
identifiable abort built in. §14a called threshold ECDSA "a worse security
record" — that was true of GG18/GG20 and is **no longer the state of the art**.

Threshold ECDSA emits a **standard** ECDSA signature, so `ecrecover` verifies it
natively. The Schnorr-via-`ecrecover` trick in §14c and its custom FROST
ciphersuite are simply unnecessary: **that trick exists only for people whose
threshold scheme outputs Schnorr.** §14c's conclusion stands (don't hand-roll a
ciphersuite) but its premise is now moot.

### One address on every EVM chain

Deploy through the deterministic CREATE2 proxy with a fixed salt: the vault
gets the **same address on Ethereum, Arbitrum, Base, Optimism, Polygon, BSC**
and any EVM chain that comes later, from one bytecode.

This kills §14c's objection that a contract means an audit per chain. It is
**one contract, audited once, deployed many times**. Adding an EVM chain becomes
a deployment and a config row — no new ceremony, no new key, no new audit.

### The migration path, so the choice is not load-bearing

`signer` is immutable, and it does not need to change: point it at a **Safe**.
The Safe rotates its own owners without changing its address. If threshold ECDSA
is adopted later, the threshold address becomes the Safe's signer — the vault
address never moves and no redeploy occurs.

So the k-of-n vs threshold decision stops being a fork in the road and becomes
an operational change made later, under no time pressure.

### What must be bound in the withdrawal digest

`chainId`, `address(this)`, token, recipient, amount, and a **nonce**. Without
`chainId` and the vault address, a signature authorising a withdrawal on Base
also authorises the identical withdrawal on Arbitrum — the same-address-everywhere
property that makes deployment easy makes this replay the default failure.
EIP-712 is the right encoding here, and unlike §6b there is no MetaMask
`chainId` hazard: the verifier is a contract, not a wallet.

### What this does not change

The valuable half (§14a): `AttestVaultBalance`, conservation under **S11**,
provisional credits, exits, destination binding. All chain-agnostic, all already
built. The EVM vault is a new `ChainOrigin` and a new watcher — not a subsystem.

### Status: built, less the watcher

- `zyn-bridge/src/evm.rs` — per-chain origins with an injectivity test, the
  destination commitment, and the EIP-712 withdrawal digest. 13 tests.
- `contracts/src/ZynVault.sol` — the vault. 19 tests under Foundry.
- The two are tied together by a **shared vector**: Rust prints the digest,
  Solidity reproduces it. A drifted encoding would otherwise show up only as
  every signature failing on a chain holding funds.

- `zyn-custody/src/evm.rs` — the watcher. Tip is the **finalized** height, so
  confirmation depth goes to zero and reorgs are excluded by the chain rather
  than waited out. Amounts arrive WAD-scaled from the contract, so there is no
  second scaling implementation to disagree with the first. 7 tests.
- `zynzapd` now drives a **list** of bridges rather than one, so Zcash and the
  EVM vault run together — the second bridge §14b was waiting for.

Two bugs the build surfaced, both caught by tests rather than by reading:

- The `Deposited` topic constant was wrong. Every filter would have matched
  nothing, forever, with no error anywhere. Now recomputed in Rust and asserted
  against a log the contract actually emitted.
- Bridge progress was keyed by chain alone. A second bridge would have loaded
  the first's scan height and **cleared its dedup set** — which the module docs
  call the real safety property — re-crediting every deposit it had forgotten.
  Now keyed by asset, with the old file adopted on upgrade.

Remaining: standing up a Safe as `signer`, and deploying.



## 14e. Can the EVM vault live on the bridge microchain?

Two readings, and only one is a design question.

**The contract cannot move.** USDC on Base is a row in a contract on Base;
nothing on Zyn can hold it. The vault is on Base because that is where the money
is — not a choice we get to make. That part stays.

**What can move is the authority.** Today §14d has `signer` pointing at a Safe:
humans decide what to sign. The real question is whether the **bridge microchain
decides instead**, and the signers only attest to what it decided. That is worth
doing, and there is a version of it that is meaningfully better than a Safe.

### Level 1 — the microchain is the vault's controller

The withdrawal digest stops being proposed by an operator and becomes a
**deterministic function of bridge-microchain state**: the exits that are due at
this height, in this order, with this nonce. Signers run the microchain, verify
its transitions, and sign what it produces.

Signers lose discretion. They can still **halt**, but a correct signer has
nothing to sign except what the state proves, so "an operator signs a fake
withdrawal" stops being a distinct failure and becomes "a signer abandoned the
protocol" — which the same k-of-n threshold already bounds.

Three concrete things this fixes, all from §14b's isolation:

- **Conservation runs before signing, not after.** Under **S11** a withdrawal
  that would unbalance the books never becomes a digest at all.
- **The attested ceiling is local.** `AttestVaultBalance` for the EVM vault is
  bridge-microchain state, so a compromised EVM vault **cannot mint `ZEC.zy`**.
  This is §14b's fault-isolation argument, and the EVM vault is exactly the
  second bridge that §14b said it starts to pay for.
- **Nonces stop racing.** The withdrawal nonce is chain state, so two signers
  cannot construct conflicting withdrawals for the same slot. That is an
  operational bug class deleted rather than mitigated.

### Level 2 — the contract holds a state root, and exits are permissionless

The stronger version, and the one that actually deserves the name.

The vault stores a **commitment to bridge-microchain state**. Signers attest to
a **root per epoch** instead of a signature per withdrawal, and withdrawal
becomes:

```solidity
function withdraw(bytes32[] calldata proof, Exit calldata exit) external;
```

Anyone may submit it. The contract checks inclusion against the posted root.

What that changes:

- **Exits no longer require an operator's cooperation.** Once a root covering
  your exit is posted, you leave by yourself. Signers can refuse to post a root
  — a halt everyone sees — but they cannot **selectively censor one user**,
  which a per-withdrawal signature scheme lets them do silently.
- **Trusted actions shrink from per-withdrawal to per-epoch**, and each one is
  a single public artefact rather than a stream of private approvals.
- Gas moves the other way: a depth-20 proof is ~30k instead of ~3k for
  `ecrecover`. On the L2s where the liquidity actually is, that is noise.

### The limit, stated plainly

**Base cannot verify Zyn.** The contract checks a signature or a root that
signers vouched for. Neither level removes the k-of-n assumption — a dishonest
threshold posts a wrong root and drains the vault, exactly as it could sign a
wrong withdrawal.

So this is not trustlessness. It is a smaller and more visible trusted surface:
fewer trusted actions, each public, and exits that do not depend on being
served. The honest endpoint is a **Zyn light client on the EVM side**, verifying
our sealing and Zcash anchoring in a contract. That is real, and it is a large
build we are nowhere near.

### Verdict: Level 1, and it is not blocked

**Chosen.** Level 2 stays the shape to build toward — permissionless exit is
what users should demand of any bridge and it costs a Merkle proof — but Level 1
is what ships.

Two corrections to the paragraph that stood here, both found by reading the
code rather than reasoning about it:

**Level 1 is not gated on §8.5.** It needs the withdrawal digest to be a
deterministic function of state, and `zyn_bridge::settle::group` already
produces exactly that — canonical ordering, oldest-first, "two nodes building a
settlement from the same state produce identical bytes for the signers to agree
on". That was written for Zcash and is chain-agnostic. What §8.5 gates is the
**topology**, not the property.

**Minting isolation already holds too.** `TokenInfo` carries `vault:
Option<Vault>` — one vault per asset — and `attest_vault` touches only the
named asset's vault. So an EVM attestation cannot raise `ZEC.zy`'s ceiling
today, on one chain, with no split. §14b's isolation argument was about **halt**
isolation (**S11** discards a whole batch), and that is the part still waiting.

So the split buys less than §14b implied and Level 1 buys more. What remains for
EVM is a per-chain `ChainOrigin`, a canonical digest encoding, the contract, and
a watcher — none of which the eventual microchain refactor would change.


## 15. Solana deposits: a memo, not a program

The EVM vault is a contract because it had to be: an ERC-20 `transfer` carries
nothing that could name a recipient, so the only thing a watcher could key on
was the sender, and that publishes a link from a public EVM identity to a Zyn
account (§14d).

**Solana does not have that problem.** The Memo program exists, so a deposit can
name its recipient without a program of our own — which makes the Solana design
the same shape as the **Zcash** one (`zyn_custody::memo`), not a variant of the
EVM one.

### The reason this is better and not merely cheaper

A Solana program needs an **upgrade authority**, and a key that can replace the
program is a key that can drain the vault. Every mitigation is a way of securing
that key — burn it and lose the ability to fix bugs, or hold it in the same
threshold and add a second ceremony with different semantics.

Not deploying a program **removes that key** rather than securing it. That is a
better outcome than any custody arrangement for it, and it is available only
because Solana has memos.

Two smaller consequences, both good: no program audit, and no Anchor-to-Agave
version coupling to carry (the installed pair, 0.30.1 against 3.0.15, is already
a mismatch we would have had to resolve first).

### What it costs

The same thing it costs on Zcash: a depositor who omits the memo sends
**unattributable funds**, and no rule should decide whose they are.
`crate::watcher` already models this and it stays a manual refund. Being the
second chain where this is true is an argument for solving it once in the UI,
not for a program.

Three cases the decoder refuses rather than resolves, each a way a naive reading
would credit the wrong person:

- a **failed** transaction is still in the block — its transfers did not happen,
  but its memo did;
- transfers to somebody *else* in the same transaction are not ours;
- **two memos naming different accounts** is a manual decision, not a rule.

### The one real difference from Zcash: the memo is public

On Zcash the memo sits inside the encrypted note, so who can read it is exactly
the question of who holds the viewing key. On Solana anyone can read it, so a
deposit links a Solana address to a Zyn account **in public**.

That is a genuine privacy regression relative to the ZEC path and it should be
stated in the interface rather than buried. It is the depositor's to make: the
account named need not be one they have ever otherwise used, and the Zyn side
of it stays shielded. But "bridge in from Solana" is not "bridge in from Zcash",
and pretending otherwise would be the kind of claim §6e was written about.

### Finality, again

Tip is the **finalized slot**, so the confirmation count is zero — the same as
EVM and for the same reason. A **skipped slot is not an error**: Solana produces
no block when a leader fails, and treating that as a failure would stall the
watcher on an entirely ordinary event.

### Exits: level 1, and the first asset that can leave

Built. The ceremony and signing were made **generic over the FROST
ciphersuite** rather than copied for ed25519: one DKG driver and one
nonce-once signing session, instantiated for RedPallas and for ed25519. A
second hand-written driver would have been a second home for the
round-ordering bugs the first exists to prevent.

What Solana gives that the EVM leg had to build:

- **Nothing on-chain.** The vault *is* the threshold key; Solana verifies its
  signatures as ordinary ed25519. No contract, no upgrade authority, no
  audit surface.
- **A durable nonce** in place of a blockhash: the settlement transaction is
  fully deterministic, signable with no time limit, and consumes the nonce
  when it lands. That is the EVM contract's "nonce consumed before payout",
  given by the chain.
- The transaction bytes are assembled here, not by the Solana SDK. Two nodes
  must agree byte-for-byte on what they sign; that is a property to own in a
  page of code with a pinned layout test, not to delegate to a crate whose
  dependency tree fights the Zcash stack.

**Verified on a real validator**, not only in unit tests: a 2-of-3 key born in
a ceremony paid an exit requested on Zyn; exactly the payout plus one fee left
the vault; the nonce advanced; a replay was refused; `ConfirmWithdrawal` burned
the units and released the backing. `zynzapd` drives the whole loop
(`settle.rs`), one transaction in flight at a time, recording each before it
is sent.

So `SOL.zy` is two-way — and, as of §17, so is `ZEC.zy`. `SOL.zy` now exists
on the testnet node as asset 2, created at runtime by `CreateBridgedAsset` —
an operator-only intent added so a bridged asset does not have to be a genesis
fact. The Solana watcher runs against devnet from the same daemon.


## 17. ZEC.zy exits: what NU6.3 allows, and what it does not

`ZEC.zy` can leave. Building the last mile turned up two facts about Zcash
today that reshape the exit, and both are recorded here because neither is
guessable from the plan.

### The note tree is seeded, not rebuilt

§13 named the largest unknown: a spend is proved against the Orchard
commitment tree, which holds every commitment since NU5 — millions of blocks
before any vault existed. Rebuilding it one RPC at a time does not finish.

Zebra serves the tree's **frontier** at any height (`z_gettreestate`, kept for
lightwalletd), and the root alongside it. So the vault's tree starts from the
node's frontier at the vault's creation height and appends only what the
deposit scan already reads — which it must read anyway, since it trial-decrypts
every action. `tests/zebra_live.rs` seeds from the real testnet node, feeds
twelve real blocks through our own scan, and finds the root identical to the
node's: 248,810 commitments, in step. The witness path is real.

Two details that cost time: the node's `finalRoot` is *not* byte-reversed the
way a txid is; and a block is appended all-or-nothing — a scan that fails
halfway rewinds to its checkpoint, or every later position shifts and every
witness with it.

### Under NU6.3 the Orchard pool cannot pay another address

Testnet activated NU6.3 at 4,134,000. From there an Orchard-pool bundle is
**v3**, and v3 *mandates* the cross-address restriction: every action's spend
and output must belong to the same receiver, by consensus. `orchard` refuses
to build anything else (`CrossAddressDisabled`) and the network would refuse
it if built. Paying a user's shielded address from the vault is not
restricted; it is unrepresentable. That capability moved to the **Ironwood**
pool, introduced at NU6.3, which needs v6 transactions.

What an Orchard vault can still do is leave the pool through the value
balance. So an exit is:

- spend the vault's notes;
- change back to the vault's **own receiver** — the one same-address output
  the rules allow, in its own action, whose zero-value spend is keyed to the
  vault and therefore also threshold-signed;
- pay the recipient a **transparent** output, ZIP-317 fee included.

The consequence is stated plainly rather than softened: **the exit is public
on the chain.** The trading that preceded it was not, and the depositor's
identity is not linked to the withdrawal beyond what any transparent receipt
reveals. But "bridge out to ZEC" today means "bridge out to t-ZEC", and the
interface should say so. The earlier code comment calling this "an open
consensus question" is now closed: it is not a question, it is the rule.

**The way out is Ironwood.** A v6 transaction may carry an Orchard bundle with
positive value balance and an Ironwood bundle with negative — a pool-to-pool
move — and Ironwood v3 permits cross-address outputs. The vault would then
hold Ironwood notes and pay shielded exits again. `zcash_primitives` 0.30
carries `from_parts_v6` and an `ironwood_bundle`; whether v6 is accepted on
testnet under NU6.3 or waits for NU7 is the next thing to establish, and it
decides whether shielded exits are a build or a wait. Until then, transparent.

### A circuit is a property of the bundle version

`ProvingKey::build` takes a circuit version, and the right one is not a
constant: a v2 bundle proves under the post-NU6.2 *fixed* circuit, a v3
bundle under the NU6.3 one. Proving under the other produces a proof that
constructs without complaint and fails to verify. The daemon therefore takes
the bundle version from the consensus branch at the height it builds at
(`Envelope::bundle_version`) and derives the key from that, once per version.

### Status

Built and tested end to end short of the network:

- `zyn-custody/src/payout.rs` — the v5 envelope: ZIP-244 sighash over the
  whole transaction, IO finalisation (dummy spends signed), the proof, the
  quorum's re-randomised signatures per action, the binding signature, and
  serialisation. Parsed back with the standard reader; proof verifies; the
  sighash recomputed from the parsed bytes equals the one signed.
- `zynzapd/src/settle.rs` — `ZcashSettler`: pending exits → canonical
  settlement → reveals checked against bindings → build, prove, sign, seal →
  ledger → broadcast → confirmations → `ConfirmWithdrawal` and the spent
  notes forgotten. Expiry replaces Solana's nonce: a transaction unseen past
  its expiry height is abandoned and its exits rebuilt, and the two cannot
  both land because the second is built after the first died.
- `zynzapd/tests/zcash_settle.rs` — the whole loop against a node that answers
  like Zebra, with a real proof.

**Not yet done on the real testnet**: no TAZ has been deposited to a
threshold vault. The faucets in §4c are browser proof-of-work; when one has
been worked, the runbook's §4f is the sequence. Nothing in the path has been
faked except the node's answers, and the tree sync has already been checked
against the real one.

`ZEC.zy` and `SOL.zy` are both two-way. §13's ordering argument is settled.


## 18. The first real deposit arrived in a pool we were not reading

The first testnet deposit to the threshold vault
(`372abd69…af9650`, 0.1 TAZ) was a **v6 transaction with two Ironwood
actions and no Orchard bundle**. §17 treated Ironwood as a pool that exists
and v6 as a version that might be accepted; a wallet answered both questions
in one transaction. Post-NU6.3 wallets send to Ironwood, because the Orchard
pool can no longer pay anyone. The vault's `utest1…` address did not change —
Ironwood uses the same keys and the same receiver encoding — the *pool* did.

Two things were wrong with the scanner and both are fixed:

- It read only `orchard_bundle()`. It now reads both, and decrypts each
  under its own domain: `OrchardDomain` accepts V2 note plaintexts (lead byte
  `0x02`), `IronwoodDomain` V3 (`0x03`). The same viewing key, the wrong
  domain, and the note is invisible — which is exactly how the deposit first
  looked.
- It kept one note tree. Ironwood has its own commitment tree and its own
  anchor, so the vault now keeps **two**, each seeded from the node's frontier
  for that pool (`z_gettreestate` serves both) and each fed by the scan. A
  block is appended to both or rewound from both.

### The deposit carries no memo

It was sent from a wallet with an empty memo, so it names nobody. The scan is
built to **stop** there (§8.11 in the plan, `ScanError::UnaddressedDeposit`):
the money is real and guessing its owner is the one thing not to do.

The mechanism that resolves it is now explicit rather than a support ticket:
`ZYN_ZEBRA_ATTRIBUTIONS` names a file of `<txid> <account>` lines. A human
writes a line; the scanner reads it at startup and credits the deposit to that
account when it reaches the block. The file is the audit trail — every
memo-less deposit ever credited is one line in it, with the txid to check it
against. Nothing else in the path can attribute a deposit.

### Exits move to Ironwood, and become shielded again

§17's transparent exit was the only thing the Orchard pool permits. The vault
now holds Ironwood notes, and Ironwood v3 **permits cross-address outputs**.
So a payout from Ironwood is an `ironwood_v3` bundle in a **v6** transaction
paying the recipient's shielded receiver directly — the shielded exit that
§17 said was a build or a wait. **Built.** The settler tries Ironwood first
and falls back to Orchard for transparent recipients only; the recipient's
address type decides what a payout may contain, the pool decides what it can.
Tested end to end with a real proof: the recipient decrypts exactly the exit.

### Status: the round trip has happened on the real testnet

- deposit `372abd69…af9650` — 0.1 TAZ, Ironwood, height 4,326,873, attributed
  by hand and credited;
- exit `c8bc6fb7…2ef3` — 0.05 TAZ to the user's `utest1…`, built from
  Ironwood in a v6 transaction, proven on the box (13 s for the key), signed
  by a 2-of-3 that was never assembled, mined at 4,327,078, decrypted by the
  recipient's wallet, confirmed on Zyn and burned. Backing 0.1 → 0.05.

The exit found one more bug the moment it landed: the vault's own **change**
came back memo-less and the scanner stopped, calling it a deposit nobody
claimed. A note to us in a transaction that spends our own nullifier is
change — held, never credited. `shielded::classify` decides it now, and the
regression test builds a real payout and scans it.

`zyn-wallet` exists so a tester has a shielded testnet address without a
third-party app, and can deposit *with* the memo. Built from the same code.


## 19. A failed node call must not look like an empty block

The first Solana deposit on devnet was **silently skipped**. The public RPC
rate-limited a `getBlock`, the chain view's infallible `deposits_at` answered
"nothing here", and the watcher recorded the slot as scanned. Nothing failed
loudly; the deposit was simply never going to be seen again.

The infallibility of `ChainView` is right — a view is a snapshot and the
watcher's logic must not be threaded with I/O errors — but it made "I could
not look" and "I looked and saw nothing" the same answer, and those must
never be the same answer for a watcher whose only job is not to miss things.

So a view now carries a **failure flag**, set whenever an answer was forced
by an error, and the bridge checks it after every pass: if set, the pass
records no progress and the same range is read again. A rate-limited node
now costs a delay, not a deposit. The Zcash view is a snapshot built before
the pass and cannot fail mid-way, which is why it never had this bug.

The second half of the fix is Solana-specific. Slot-by-slot `getBlock` is
hundreds of calls a minute against an endpoint that allows far fewer. The
view is now **address-indexed**: one `getSignaturesForAddress` per pass files
every transaction that touched the vault by slot, and only those are
fetched. A slot with nothing filed costs no call at all. The watcher still
walks every slot, unchanged.

Found in production on the first real deposit, fixed, and the deposit
credited on rescan (`ZYN_SOLANA_RESCAN_FROM`, safe because the dedup set —
not the height — prevents a re-credit). The same flag is on the EVM view,
which had the same bug and had not yet met a rate limit.

The Solana leg then completed on devnet: exit `5Rkuyp6n…`, 0.2 SOL paid
against the durable nonce and confirmed on Zyn; and the first pool and swap
followed (`ZEC.zy : SOL.zy`, 0.01 SOL.zy → 0.00191 ZEC.zy). Both assets
are two-way and tradeable against each other on the testnet.


## 20. One binding per account is the wrong shape for a multi-chain user

The first Solana exit was refused (`WrongDestination`) for a reason that is
correct locally and wrong globally: an account holds **one** withdrawal
binding, so binding a Solana address is a *redirect* away from the Zcash one
— and a redirect waits out `exit_timeout_epochs`, 2,016 epochs, because a
stolen key must not be able to move the destination quickly (§8.x, `Binding`).

That protection is right. Applying it to a user who simply also exits to a
second chain is not: the two destinations are not in competition, and a user
of a two-chain bridge would spend most of their time waiting on a delay that
protects nothing.

**The fix is a binding per origin.** An account binds a destination *for
Zcash* and a destination *for Solana*; the first binding for a new origin is
effective at once (nothing existed to protect), and only replacing an
existing one for the same origin waits. `RequestWithdrawal` already knows its
asset and therefore its origin.

Not done tonight, deliberately. It changes the account record's encoding,
which the state root commits to, and the testnet chain holds real funds
against real vaults: a migration has to be one that leaves existing roots
untouched or is sequenced as an intent. That is a careful hour, not a late
one.

**What was done instead**, and it is a testnet posture, labelled as such:
`ZYN_EXIT_TIMEOUT_EPOCHS` lets the operator set the delay on a running chain
through a sequenced `SetParams`. It is 10 epochs on the testnet node. It must
not exist in a mainnet configuration, and the daemon says "TESTNET posture"
when it applies it.


## 21. Items, and ownership nobody else can read

**Historical privacy description — superseded by §70 (13 Sep 2026).** The
current publisher includes replayable intent journals as well as hash-only
snapshots. Transfers, mint recipients and Reblind values are readable in those
journals. Signed reads and blinded leaves alone therefore do not establish
privacy from journal readers. §70 selects operator-trusted public privacy as
the next target; it is not a claim that the current deployment meets it.

### What was public, and why

The data-availability payload published every account record in the clear
— balances, exits in flight, credits pending — so that a holder could prove
an exit against the anchored root without us. Deliberately nothing else was
published: intents (trades, transfers, sales) never were. But an item is
unique, so "account X holds item N" was a fact anyone could read off the
payload, and every transfer changed it in public.

Three designs were weighed for hiding it, and the reasoning is worth keeping:

- **Pool-as-NFT** (the item is a one-unit pool; the owner holds the LP
  shares). Ownership becomes an LP balance — the same exposure, in the same
  payload, plus a public pool registry that names the item. It buys
  fractional ownership and a visible floor, not privacy. Kept as a product
  option for shared items; not a privacy mechanism.
- **A user microchain per holder** (Linera's shape). Items partition cleanly,
  so this passes the test pools failed (§11, §14b) — but a user chain is
  *owned*, not *confidential*: whoever validates it sees it. Hiding contents
  needs commitments and proofs either way, and per-user chains make
  *activity* public (chain X moved at epoch E) where a shared pool hides who
  acted, and they break the atomic sale (§8.5). Right for scale, later; not
  a privacy answer.
- **Commit the payload.** Publish `H(record)`, never the record. Correct, and
  small.

### What is built: commitments, blinds, signed reads

The published snapshot is now a tree of **leaves only** — no ids, no
records. Anyone can rebuild the anchored root from it; nobody can read a
balance from it.

A leaf is `H(record)`, and a record is low-entropy — an id and a few
balances — so a leaf alone could still confirm a guess. So an account
carries a **blind**: a 32-byte secret the holder sets with `Reblind`, folded
into the record. With one, a guess cannot be checked; without one, the
record is exactly what it was before blinds existed, so **every existing
leaf and root is unchanged** — the deployed chain resumed on the same root
after the upgrade. `zyn-cli shield` sets a fresh blind; it costs one intent.

Reads are **signed**: `OP_ACCOUNT` and `OP_ACCOUNT_PROOF` serve a record
only to a request signed by the account's own key, over
`"zyn.read.v1" ‖ chain ‖ account ‖ epoch`, fresh within a hundred epochs.
The proof carries the record, its leaf index and its path — enough to prove
an exit against the anchored root forever, fetched while we are here.

**What this hides, and from whom.** Everyone except the operator: an
observer with the whole published history sees hashes. The operator
executes the intents and sees everything — the same line the single-operator
posture already draws for every trade. The one public link in the system is
still a Solana deposit's memo (§15); the defaults are to fund item accounts
from Zcash or through a fresh account, and to use a fresh account per
collection, which are cheap because accounts are keys.

### What is built: items and sales

- `MintItem` carries a **content hash**, fixed at mint. An item is a symbol,
  a supply of whole units, a bond in `ZEC.zy`, and what it is.
- A sale is `AcceptOffer`, co-signed: the maker signs the whole trade, the
  taker adds a signature and submits, atomically, in one intent — no
  marketplace contract, no resting order, nothing published.
  `OP_SUBMIT_MULTI` carries several credentials; `zyn-cli offer` produces a
  signed line and `zyn-cli accept` finishes it.
- Exercised on the testnet: shield → mint (`ART`, bond 0.001) → fund a buyer
  → offer for 0.005 `ZEC.zy` → accept. Balances moved exactly; the item
  changed hands; nothing about it was published.

### The destination

Items as **notes** — commitment, nullifier, and a transfer intent carrying
a proof of ownership produced by the rv32im guest — so that even the
operator sees nothing. Same road as proof-carrying settlement (§14e level 3);
built once, after the swap and the per-origin bindings (§20).


## 22. Operating it: alerts, persistence, a reveal channel — and the fee

Three things a single operator needs before a second user, built in order,
plus one thing the first exits taught.

**Alerts.** Every bridge and settler reports after each pass. An `Alerter`
reacts to *transitions* — a component down after three failed passes or
fifteen minutes without a good one, recovered on the next good one,
reminded hourly while down — with a fixed journal prefix (`ALERT`) and, if
`ZYN_ALERT_URL` is set, one webhook POST in the `{"text": …}` shape Slack and
Discord accept. Health is also appended to `OP_STATUS`, so `zyn-cli status`
shows it and a dashboard needs no journal. The memo-less-deposit halt is
now a page, not a line nobody reads.

**Note trees on disk.** `bridgetree` has no serialiser; its parts are all
reachable, so `NoteStore::encode/decode` writes them down and a restart
resumes from the saved tree after checking its root against the node at
that height. Written at every state save and at shutdown; anything that
does not decode or does not match is re-seeded from the node. The linear
rescan is gone.

**A reveal channel.** `OP_REVEAL`: the holder sends, signed by the account's
key, the address and salt behind the destination it already bound. The node
accepts it only if `keccak(kind ‖ address ‖ salt)` equals that binding — so
the channel can disclose an exit, never redirect one — hands it to the
settler at once, and appends it to the reveal file for restarts.
`zyn-cli reveal-submit` is the whole user side. No operator, no restart.

### The fee, which the vaults had been paying

Both first exits paid the network fee **from the vault**: the Zcash payout
sent the exit plus 10,000 zat, the Solana one the exit plus 5,000 lamports.
The vault then held slightly less than it had issued — and the next
deposit's attestation would have refused (`AboveObserved`), which is the
invariant doing its job on a bug in the payout.

Fixed at the source: **an exit pays its own fee.** What leaves the vault
equals what is burned on Zyn; the recipient receives the exit less the fee.
The testnet shortfalls (0.0001 TAZ, 0.000005 SOL) were covered by top-ups
to each vault carrying the **all-zero account** in the memo — a deposit
that raises the attested balance and credits nobody, now a defined thing.

### One more the persistence work found

A tree rebuilt from the chain re-held a note the vault had already spent:
the settler forgot spent notes only on its own confirmation, and a rebuild
does not go through the settler. The scan now forgets any held note whose
nullifier it sees on the chain — the chain, not the ledger, is the record of
what is spent.


## 23. Mirrored NFTs: a bridged asset with a supply of one

An NFT from another chain is custodied and issued exactly like SOL: the
vault holds the token there, a Zyn asset backs it one-for-one, conservation
is `1 == 1`. Nothing in custody, signing, the ledger or the reveal channel
changed. What was added:

- **`CreateBridgedItem { symbol, origin, content }`** — an indivisible
  bridged asset (`unit == ONE`), one per `content`, which on Solana is the
  **mint itself**. The item's identity is what it is backed by.
- **Deposits into token accounts.** An SPL token goes to the owner's
  *associated token account*, a program-derived address, and a transfer
  into it names that account, not its owner. So the watcher derives the
  vault's token account for every mirrored mint (SHA-256 over the seeds,
  bump counted down until the result is off the ed25519 curve — checked
  against `spl-token address`) and indexes those alongside the vault. A
  `transferChecked` into one, with the memo, is a deposit of that item.
- **Per-asset attestation.** A deposit now says which asset it credits, and
  the watcher attests that asset's vault balance — the token account's — before
  crediting it. The item's vault is its own conservation check.
- **Exits as SPL transfers.** A token payout is two instructions in the same
  nonce-bound settlement transaction: create the recipient's token account
  if missing (the vault pays that rent, from the surplus a top-up maintains)
  and transfer. SOL and items leave the same vault in the same transactions.

Exercised on devnet end to end: a one-of-one mint deposited with the
account memo, credited as `PUNK.zy` (asset 5) with no human in the loop,
withdrawn with `zyn-cli withdraw-item`, paid back into the wallet's token
account and burned on Zyn. The same round trip runs in `solana_nft_live`
against a local validator.

### What is stated, not discovered

- **The deposit and the exit are public on Solana**, like every deposit: the
  wallet that sent the NFT to the vault is linked to its arrival, and the
  exit names its recipient. Between them, ownership and every sale are
  private to the extent §21 gives.
- **Royalties are not enforced.** Sales are private, co-signed offers; there
  is no marketplace contract to route a cut through and no public price to
  compute it from. A creator who relies on enforced royalties should not
  mirror here. That is a position.
- Metaplex legacy NFTs are SPL mints and work as-is. Metaplex Core assets
  are not SPL mints and are not covered. EVM ERC-721 needs a receive hook on
  the vault contract and a `withdrawItem` — the same shape, not built.

---

## 24. Our own light server, for both pools and both networks

A wallet that holds its own keys needs a block source that does not hold
them. On Zcash that is lightwalletd: compact blocks — per shielded output
the nullifier, the commitment, the ephemeral key and the first 52 bytes of
ciphertext — so a phone trial-decrypts a year of chain in minutes and asks
for the full transaction only where something decrypted.

We do not use lightwalletd. Its compact format has no field for Ironwood
actions and its parser predates v6 transactions, so after NU6.3 it cannot
see the pool our exits are paid into and our deposits arrive from. The
wallets built on it (Zashi, Ywallet) have the same blind spot until they are
updated, which is also the reason §22's wallet UX complaint has no quick fix
from outside.

So `zyn-lightd` is the same idea, in our code, over Zebra: `getblock <h> 0`,
`zcash_primitives::block::Block::read`, and `CompactBlock::from_block`
covering both Orchard and Ironwood bundles. Five operations, no more —
INFO, BLOCKS, TX, SEND, TREESTATE — length-prefixed frames like the Zyn RPC,
one binary per network, a disk cache of compact blocks so Zebra is read
once per height. The wallet learns the network from the server (INFO) and
records it in its state, so a testnet wallet cannot be pointed at mainnet by
mistake.

### What the server learns, stated

The block ranges a wallet fetches (which reveals roughly its birthday) and
the txids it asks for in full (which are the transactions that paid it).
Not the address, not any key. This is lightwalletd's trade exactly; we make
it consciously and write it down. A wallet that wants less can fetch decoy
transactions; the protocol does not stop it. Running your own `zyn-lightd`
against your own Zebra removes the question.

### Why not decrypt server-side

A server holding viewing keys (the "trusted lightd" shortcut) would make
sync instant and the protocol trivial, and would hand a third party the
wallet's whole history. The vault does that for the vault's own address
because the vault *is* the operator; a user's wallet must not.

### What this settles for the wallet

The wallet (§22) is now a client of this server, with persistent state:
birthday, both pool trees, held notes, spends noticed by nullifier. Sync is
incremental and fast (sub-second per thousand testnet blocks); a send
broadcasts through the server. The same binary talks to mainnet the day a
mainnet `zyn-lightd` exists — which is a node-provisioning question (disk,
RAM), not a code one.

### Found by the first full loop through the app

The settler forgot a note from the store the moment it broadcast the
settlement (§22, so a rebuilt tree would not offer it again). The deposit
scanner then met the settlement on the chain, saw it spend a nullifier it no
longer knew, and took the vault's own change for shielded funds with no
memo — and halted, as it must for money nobody can claim. The fix is in the
store: a note we spend stays known as *in flight* (not spendable, not
counted, persisted) until the chain shows its nullifier, so the transaction
that spends it is still recognised as ours. The live node was recovered by
attributing that one settlement txid to the zero account.

The next round (through Nap Wallet) failed the same way, and the in-flight
fix had not been enough because the order was different: the **settler
feeds the tree** up to the tip to build an anchor, applying the spend, and
the **deposit watcher re-reads that block later** on its own pass, by then
finding no held note behind the nullifier. Two readers of one store. The
store now also remembers every nullifier it has seen spent (persisted, one
32-byte entry per spent note), and "is this nullifier ours" answers yes
for held, in-flight and spent alike — so whichever reader comes second
still knows the transaction is the vault's own. The re-read path also
classifies with that answer instead of the strict scan it used. Both
fixes stay; they cover the two orders.

### A node that is still syncing

The first mainnet server was a Zebra at height 2,800 of 3.5 million. Two
things followed. A wallet born "now" on such a node would take its
birthday from a height a year in the past and scan everything the node
later fetched, so INFO now carries the node's estimate of the real chain
height and a new wallet refuses to be created while the two disagree.
And below a pool's activation height there is no tree to seed from, so
TREESTATE says so explicitly (zero root, empty frontier) and the wallet
starts that tree empty, instead of the node's answer being guessed at.

## 25. A local app, not a web app

The wallet is a process on the user's machine with a page on
loopback, not a site. The reason is the same as §24: the keys. Trial
decryption, note selection, proving and signing all need the spending key,
and a page served from our domain that held it would make us a custodian of
everyone who opened it. `nap-wallet` keeps the keys where the CLI kept them
(`~/.zyn/app`), does the cryptography in the same Rust the vault runs, and
lets the browser be what it is good at — showing state and taking a number.

What that costs: a download and a running process, no mobile yet. What it
buys: no custody, no server-side secrets, and one code path from the vault
to the wallet. A packaged desktop shell (Tauri) is the natural next step
and changes nothing here; the page and the API are already the boundary.

It is a **wallet** first — balance, receive, send with memos, history —
on mainnet or testnet, with Zyn as a connected feature (deposit, withdraw,
private transfers on Zyn) rather than the AMM dashboard. The AMM is
ZynZap's product and keeps its own front; the wallet is what people hold
their ZEC in, and the reason to build one is that the existing wallets'
UX (§22) and their blindness to Ironwood (§24) make Zyn unusable from
them. The user named it **Nap Wallet**.

## 26. Three faces, one core

The desktop app, the mobile app and the browser extension are three
transports for the same `zynzapd::app` API and the same page. That was the
point of §25's boundary: the API is `api(method, path, json) -> json`, so
the local server (`nap-wallet`) carries it over loopback HTTP, the Tauri shell carries it
over `invoke` in-process, and the extension carries it back to whichever of
those is running. No face has its own logic to get wrong.

### Where the keys are, per face

| face | keys | why |
|---|---|---|
| `nap-wallet` (loopback) | `~/.zyn/app` | the CLI's convention |
| desktop / mobile (Tauri) | the platform app-data dir | sandboxed, backed up by the OS, no port open |
| extension | **none** | see below |

### The extension holds nothing

A browser extension could hold the keys and prove in WASM; wallets do. We
do not, for now, because the proving (Halo 2 for Orchard/Ironwood) is a
minute of CPU and hundreds of megabytes in a service worker that the
browser may kill at any moment, and because every extension is one
supply-chain review away from every user's spending key. So the extension
is a face for the local app: the popup is the page, and `window.zyn` is a
provider for sites. Reads answer at once; anything that moves value opens
an approval window that the site cannot see, style or dismiss, naming the
site and the action. The local app admits extension origins over CORS and
nothing else, and still demands the `X-Zyn` header, so a page cannot skip
the extension and talk to it directly.

What this means for a user: the extension works when the app is running,
and says so plainly when it is not. A WASM-only extension is possible
later; nothing in the API would change.

### Mobile is the desktop shell

Tauri 2 builds the same Rust core for iOS and Android. The wallet scans
compact blocks from `zyn-lightd` (§24), which is what makes a phone
feasible at all: a year of chain is minutes, not hours. Proving on the
phone is slow but honest — the key never leaves it.

## 27. ZynZap is a page; the wallet signs

The AMM front is a static page with no keys, and Nap Wallet is its signer
— the MetaMask shape, deliberately. Hosted, the page talks to the wallet
through the extension's `window.zyn` provider and every write goes
through an approval window the page cannot touch; served by the wallet at
`/zynzap`, it calls the wallet's API directly on the same origin, which
is how it is developed. Either way the wallet's rules apply: one account
key, one binding, review before value moves.

Why not a richer client in the page: everything the page needs — quotes,
pools, positions, receipts — the node already answers, and everything
that must be signed the wallet already signs. A second signer in a web
page would be a second thing to audit and a second place for a key to
leak. The page stays dumb on purpose.

Earn is liquidity, not emissions. A pool pays its LPs the swap fee it was
created with, taken from every swap and left in the reserves; shares are
a private balance like any other. There is no farm token, and §26's
warning about attaching one to a shielded pool still stands.

## 28. Batch clearing: the seal is the auction

§12b said pool-scoped batching was the privacy requirement and "a small
change". Built now, it is a small change in the VM and a real change in
what a swap is.

**What changed.** With `batch_clearing` on (an operator switch,
`Intent::SetClearing`, part of the header leaf only while on), a single-hop
swap does not touch the pool when it arrives. It is an *order* in the state
— account, pool, input, limit — and at the seal every order on a pool
clears together. Multi-hop routes still execute on arrival; routing across
pools inside one clearing is its own problem and is not solved here.

**How a pool clears.** Only the imbalance crosses the curve. With `u` of
asset0 and `v` of asset1 offered (after fees), and `u·r1 ≥ v·r0`, the
residual `x = (u·r1 − v·r0)/(r1 + v)` of asset0 is sold into the curve at
its average price `p = r1/(r0 + x)`; the asset0 side is paid that output
plus all of `v`, and the asset1 side is paid the matched `u − x`. Everyone
gets `p`. The reserves move by exactly the residual, so the invariant is the
ordinary curve's; a balanced batch never touches the reserves and pays the
marginal rate — the `best_case` the quote has always shown, now reachable;
a lone order pays exactly what it always did. This is the batch auction on
a constant-product curve, the shape Penumbra and CoW arrive at.

**What was tried first and was wrong.** Solving for the one price at which
paying both sides leaves the invariant *exactly* met, by search. The
function is tangent, not crossing: paying both sides one price leaves the
product at most equal to the invariant, so there is no sign change to find.
Maximising the product instead hands the netting surplus to the pool, which
is the opposite of the point. The residual rule is the one that gives the
surplus to the traders and keeps the pool exactly whole.

**No escrow.** An order's input stays in the account until the clearing
debits it. An account that cannot pay by the seal, or whose limit is not met
at the clearing price, gets a `SwapUnfilled` receipt and nothing moves;
the rest of the batch clears again without it. That keeps conservation
trivial (orders hold no value) and makes a queued swap free to cancel by
spending the balance — which is also the one abuse, spoofed volume, and it
costs the spoofer a signature and the chain nothing.

**What it costs a trader.** Up to one seal interval of latency (60 s on
testnet) and a fill they learn about after the fact: the receipt lands in
the seal, the wallet reads it from its balances and its open orders. The
page says so before they sign.

## 29. The reference feed: where the fee's market price comes from

§5.7 left one thing open: a reference may set a bridged pool's fee and
never its quote, but where does the reference come from? Built now, in
the node (`zynzapd::feeds`), and the answer is deliberately unremarkable.

**Public, keyless, several.** Coinbase spot and CoinGecko for crypto and
the big stables; CoinGecko for tokenised treasuries and gold (USDY, OUSG,
BUIDL, PAXG, XAUT); Nasdaq's quote endpoint and Yahoo for stocks and ETFs;
the ECB via Frankfurter for fiat. No API keys, because this data is a fee
input and nothing more, and a key would be a secret guarding something
that cannot move funds. Each symbol reads every source it has; the median
is posted; if the sources disagree by more than 5% nothing is posted; a
symbol with only one source may not jump more than 20% between cycles.
Symbols map from Zyn's `X.zy` names, with issuer decorations of tokenised
stocks (`TSLAx`, `bAAPL`, `wBTC`) stripped when the bare symbol is known.
`ZYN_FEED_MAP` overrides any symbol's sources.

**Per pool, one number.** A pool's reference is `usd(asset0)/usd(asset1)`,
posted as `UpdateReference` when it moved more than 10 bps or the last
post is older than ten minutes, so freshness never lapses on a quiet
pool. The chain then charges `max(pool fee, √r − 1)` where `r` is the
divergence between its own spot and the reference — and shows both on the
pools reply, so the page can say "fee now 412 bps, market 0.21" rather
than surprising anyone.

**What a bad feed can do.** Make a pool's fee wrong: too high and it
stops trading until the next cycle; too low and arbitrage takes what the
fee would have kept. It cannot take a unit from a pool, and it cannot
change what a trader receives for what they paid beyond the fee. That
bound is the whole reason the sources can be this ordinary.

**Tokenised stocks are priced as tokens, by issuer.** "Stocks" on Zyn will
arrive as someone's token, not as shares, and the issuers differ in what a
token is. Robinhood Chain's stock tokens (194 in its on-chain registry,
chain id 4663) reinvest dividends through an on-chain multiplier, so a
token drifts above the share price on purpose — and each has a Chainlink
feed on the chain that already quotes the token. xStocks (Backed), Ondo and
Dinari tokens are one token per share. So the feed keeps namespaces:
`RH:AAPL` is the Robinhood token, read from its Chainlink feed over the
chain's public RPC, from CoinGecko, and from the share price scaled by
`uiMultiplier()`; `X:TSLA`, `ON:AAPL`, `D:AAPL` are the others, from
CoinGecko's token price and the share price; bare `AAPL` is the share. An
asset bridged from Robinhood Chain (`ORIGIN_ROBINHOOD_CHAIN`) is `RH:`;
the issuers' own decorations (`TSLAx`, `AAPLon`, `AAPL.d`) name the rest.
The tables are data (`zynzapd/data/*.tsv`, ~1,760 tokens across the four
issuers, 35 Chainlink feeds), regenerated from the registry and from
CoinGecko's coin list rather than typed. Verified live: the Robinhood AAPL
feed read 320.52, the share times the multiplier 320.15, CoinGecko's
tokens agreed with the share price for the one-to-one issuers.

**What it costs the operator.** One more thing that must be running and
honest, with its health in `OP_STATUS` beside the watchers. Stooq, the
first choice for stocks, had retired its CSV endpoint by the time this was
tested; Nasdaq's took its place the same hour, which is the kind of
churn a fee input can absorb and a price oracle could not.

## 30. ZYN: the original launch design — superseded by §68

> **HISTORICAL — NOT THE CURRENT ZYN TOKENOMICS.** The design in this
> section was implemented and tested, but its monetary policy was abandoned on
> 13 Sep 2026. The bridge-fee genesis, contributor allocation, vesting,
> per-block emissions, halvings, fee-funded burns, LP/bridge emissions and
> mint-funded POL below must not be used for mainnet. The current decision is
> §68: fixed supply created once, no ordinary mint or burn, no gas fee, and the
> Phase-1 ZEC.zy fee split feeding ZYN:ZEC.zy protocol-owned liquidity.

Decided 7 Sep 2026 after the comparison in the conversation (Bitcoin,
Zcash, Ethereum, Uniswap, Osmosis, Arbitrum, Hyperliquid). Built in
`swapvm::launch`, run at the seal like clearing (§28), every unit in a
*pot*: an account nobody holds a key for, so conservation is the ordinary
invariant.

### The old numbers (retained only as implementation history)

| | |
|---|---|
| bridge fee | 50 bps on ZEC.zy deposits and exits, taken from the amount |
| before graduation | every fee to the genesis pot; each payer's share recorded |
| graduation | pot ≥ 100 ZEC.zy (or a Zcash height deadline, unset) |
| genesis | 2,100,000 ZYN: half into the ZYN:ZEC.zy pool with the whole pot, half to contributors pro rata to fees paid, vesting linearly over 207,360 Zcash blocks (~6 months) |
| cap | 21,000,000, eighteen decimals |
| emission | 9 ZYN per Zcash block from graduation, halving every 1,050,000 blocks; series sums to the remaining 18.9M |
| each mint | 50% LP pot, 25% bridge pot, 25% protocol-owned liquidity |
| after graduation | fees split 50% burned (bought on the genesis pool, destroyed) and 50% paired with pot ZYN into the pool; LP pot paid to LPs by the fees their pools earned that epoch; bridge pot rebated to that epoch's fee payers |
| team | nothing from the mint; the operator earns from the fee rail (§5, §6d) |

### Why the old design had this shape

**Fair launch, priced by use.** The operator brings no ZEC. The pot is
built by the bridge's own fees, and the opening price of ZYN is the pot
divided by the pool's half of the genesis — set by how much real bridging
happened, not by a number we picked. Before graduation there is no pool
because a constant-product pool with nothing on one side has no price;
the pot is the pre-market. Contributors' ZYN vests because graduation day
is the pool's thinnest and they are its largest sellers.

**Reward fees, not presence.** "Proof of bridge" rewards the fee paid,
which cannot be farmed by looping. LP rewards weight pools by fees earned
and LPs by share, so parked capital earns nothing and an arbitraged pool
earns what it collected. Rewards are paid at the seal into private
balances — no claim transaction, no timing leak.

**Blocks, not epochs.** Epochs are the operator's to seal; Zcash blocks
are not. Emission and vesting are keyed to the Zcash tip, reported by the
operator (`ZcashHeight`) and only ever moving forward; the operator can
delay the clock but cannot run it fast.

**Pots are accounts.** `POT_GENESIS`, `POT_LP`, `POT_BRIDGE`, `POT_POL`,
`POT_FEES` are account ids with no key behind them. Nothing can move
their balances except the seal's own rules; the protocol's LP shares in
the genesis pool sit in `POT_POL` and are, by construction, unremovable.
Their balances are public in the DA leaves, which is the transparent
treasury a reader would want.

**The pot is a primitive.** Fees in, contributions recorded, a threshold,
a graduation that creates a pool — that is also what a launchpad does for
a new token, and it was built as one so it can be reused there.

### What was said no to

A "circulation" half of the mint with no recipient; TVL-weighted rewards;
single-sided ZYN into the pool (a sale by another name); a team share of
the mint. And a governance token: this is an incentive asset, and §6d.3
stands.

### Two things the first live graduation taught

The launch's seal step now runs on a copy of the state and lands only if
every part of it succeeded; a failure is a `LaunchSkipped` receipt and
the seal goes ahead. The first attempt on testnet failed half-way —
creating the genesis pool tripped the launch bond (a user's pool needs
more than 1 ZEC.zy on its ZEC side; the testnet pot was 0.0001) — and had
already moved the pot and minted the pool's half of ZYN. Two rules came
out of it: a multi-part seal step is atomic or it is wrong, and the
genesis pool is exempt from the bond, because the pot is the bond. Before
graduation nothing is irreversible, so re-setting the numbers also undoes
any such partial attempt; after graduation the numbers are history.

### Historical testnet posture

Historical behavior: `ZYN_LAUNCH=1` set the launch and
`ZYN_LAUNCH_THRESHOLD=0.01` lowered the graduation pot. Current `zynzapd`
rejects either variable at startup. The old plan said mainnet would use
`Launch::v1` as-is; that is no longer a mainnet instruction.

## 31. A market for every bridged asset, opened by its own fees

> **PARTLY SUPERSEDED BY §68.** The generic bridged-asset market and fee-pot
> discussion remains part of the design record. Every statement here that gives
> ZYN a bridge-fee genesis, an unminted/emission reserve, or freshly minted
> cashback is obsolete. Under §68, any ZYN used for distribution or liquidity
> must come from the fixed, pre-minted supply.

Historically, §30 gave ZYN a bridge-fee launch; §68 has since replaced that
ZYN tokenomics. The same machinery opens a market for every other
bridged asset, and it had to, because §5.7's divergence fee makes whatever
price a bridged pool opens at permanent — a pool seeded by guesswork stays
wrong, as testnet pool 1 proved on 7 Sep at a 54.6% fee.

**The shape.** A bridged asset's own bridge fees accumulate in a pot, one
balance per asset in an account nobody holds a key for (`POT_ASSETS`). When
that pot is worth more than `asset_threshold` at a **fresh reference**, and
the protocol holds that much ZEC.zy, a pool opens at exactly the reference
price with the pot on one side and protocol ZEC.zy on the other. The
protocol keeps the shares; every bridged market is protocol-owned and
cannot be withdrawn. The reference goes onto the pool as it is created, so
it opens defended by the divergence fee rather than acquiring one later.

**Historical comparison to the superseded §30 launch.** Under that old design, ZYN's
pot *discovers* a price, because ZYN has no market anywhere else. A bridged
asset's pot only decides the *size*: its price comes from the feed. That is
a third use of a reference, beyond "sets the fee, never the quote", and it
is defensible because a manipulated reference at the moment of opening
costs the pool's owner — the protocol — a one-time arbitrage, not a
repeatable theft, and every trade afterwards is priced by the curve. It is
guarded by freshness, and a market that cannot be priced simply waits:
opening wrong is permanent, so waiting is always better.

**SUPERSEDED — the cashback may not mint ZYN at opening.** The old rule used
`bootstrap_bps` (50, half a
percent) of whatever ZYN is still unminted, shared by that asset's pot
contributors pro rata to fees paid, vesting over the same 207,360 blocks as
the ZYN genesis — a new pool is thinnest on its first day. Each grant
shrinks the base for the next, so the tenth market costs less than the
first and the cap can never be breached: twenty markets take about 8.6% of
supply at this rate, five about 2.2%. It comes from what emission would
otherwise issue, which is an honest transfer from rewarding existing
liquidity to creating new liquidity.

Not SOL.zy: a rebate in the asset paid is a fee discount by another name,
and rewards using a bridge rather than opening a market. Not LP shares:
handing the new pool's shares to contributors lets them withdraw on day
one and drain the market the pot just built. Bootstrappers also keep
earning the ordinary bridge rebate meanwhile — they paid a fee like anyone
else.

**Historical limits for the superseded mint-funded cashback.** An asset only
opens when its pot is worth `asset_threshold` in ZEC.zy, so a trivial asset
never opens one and never costs a grant. Set it in ZEC-equivalent value,
never in units of the asset. It and `bootstrap_bps` stay adjustable after
the ZYN launch has graduated — they govern *future* markets, not history,
and a new asset class may need a different bar. Everything else about a
graduated launch is immutable.

**An asset that already has a market** still pays into a pot; that pot is
paired into the market it belongs to rather than opening a new one, and
earns no grant. The first live run missed exactly this, because only assets
with a launch record were looked at and an asset with a pre-existing pool
never got one. Fixed, and the test is named after it.

### Two roots that had quietly become one root too many

Found while migrating the live state for this change. `SwapState::state_root`
(what a checkpoint commits) and `MicrochainVm::sections` (what a saved state
is checked against) were two separate lists of the same thing, and the second
had not been updated when orders (§28) and the launch (§30) joined the first.
Nothing caught it because each was self-consistent: the store compared its
root to its own recomputation, and checkpoints were compared to nothing
external. A replayer following checkpoints and a node restoring from disk
would have disagreed about what the chain looked like.

`sections()` is now the single definition and the inherent root calls it,
with a test asserting the two agree over a state carrying orders and a
launch. The lesson generalises: a commitment with two implementations has
one too many, and the drift is invisible until something outside the process
checks it.

### What a state-format change costs

The live chain's saved state had to be migrated: decoded with the new code,
its root recomputed, re-encoded. Not because the state changed, but because
the definition of its commitment did. On a chain with external verifiers
that is a hard fork with a documented discontinuity in the root lineage; on
this one it is a file rewrite, recorded here so the next one is expected
rather than discovered at three in the morning. `decode_launch_legacy` reads
the parameters as a state written before bridged markets recorded them.

## 9. Not decided here

Deliberately out of scope, recorded so they are not mistaken for oversights:
concentrated liquidity (plan §8 — and the NFT-position question is *downstream*
of it, not independent), perps, lending, governance, a permissionless validator
set, and proof-carrying settlement (V1 settles with threshold signatures;
`swapvm::guest` is shaped for the swap).

---

## 10. Bridges

| # | Decision | Status |
|---|---|---|
| 10.1 | Backing is **per asset**, not a single pooled figure | ✅ |
| 10.2 | One vault per chain, **one signer set**, one certificate | 🔒 |
| 10.3 | `ChainOrigin` is a `u16` + constants, not an enum | ✅ |
| 10.4 | Deposit addresses derived per account (and per deposit) | ❓ |
| 10.5 | Withdrawals batched into one L1 transaction per chain per epoch | ❓ |
| 10.6 | Threshold now, proof-verifying later — per chain, not globally | 🔒 |
| 10.10 | Vault ceilings: total (staged rollout) and per epoch (bounds a compromise) | ✅ |
| 10.11 | Settlements pay the longest wait first | ✅ |
| 10.12 | **Deposits are provisional until their epoch is anchored** | ✅ |
| 10.13 | **The vault's observed balance is a mint ceiling** | ✅ |
| 10.14 | **Certificates verify real signatures; a configured node fails closed** | ✅ |
| 10.15 | **Signer ceremony, threshold signing, deposit watcher** (`zyn-custody`) | ✅ |
| 10.16 | Live Zcash node, Orchard spend assembly, reorg response | ❓ |
| 10.17 | **A bridge mints an asset; the AMM trades it.** No SOL in any pool | ✅ |
| 10.18 | **`<ASSET>.zy` = bridged, no dot = native.** Suffix, not prefix | 🔒 |
| 10.19 | **`ZEC.zy` is the hub asset**; other pairs route through it | 🔒 |

**10.1 — the one-way door, now closed.** `xzec_backing` was a scalar and
`pending_withdrawal` was one figure per account. Both are now per asset:
`TokenInfo::backing = Some(Backing { origin, confirmed })` and
`Account::pending: BTreeMap<AssetId, Fixed>`. ZEC.zy is no longer special-cased —
it is simply the first bridged asset. Every bridge Zyn will ever have is the
same shape and the same identity, `supply == confirmed`.

Checked **per asset**, deliberately. A single pooled total would let a Bitcoin
custody shortfall hide behind Ethereum's surplus until both were gone
(`one_bridge_cannot_borrow_anothers_reserves`).

**10.2 — we already own the primitive.** V1 custody is FROST 7-of-10 over a
Zcash vault. That is precisely what NEAR calls chain signatures: a threshold
network that produces signatures for *other* chains. `SignerSet` / `Certificate`
/ `Ledger` are already chain-agnostic. So a second bridge is a new vault and a
new asset, not a new architecture.

**What each chain can actually support**, which is where the honesty lives:

| Chain | Deposit attestation | Withdrawal | Proof-verifying later? |
|---|---|---|---|
| Zcash | signer quorum | threshold sig | no — no general contracts |
| Bitcoin | signer quorum | FROST/Taproot | no — no general contracts |
| Ethereum | quorum, or ETH light client | quorum, or contract verifies a proof | **yes** |
| Solana | quorum, or SVM light client | quorum, or program verifies a proof | **yes** (alt_bn128 syscalls) |

BTC and ZEC are threshold-or-nothing. ETH and SOL can graduate to proof
verification once a validity proof exists — which is why 10.6 is per chain, not
a global posture.

**Where NEAR's design is worth copying, and where we can beat it:**

- **Copy — derived deposit addresses.** NEAR derives a per-account address on
  each foreign chain from the MPC root, so a deposit is attributable without a
  memo tag. That removes an entire class of "forgot the memo, funds lost"
  incidents, and it uses the derivation discipline already in `zyn_vm::derive`.
- **Beat it — derive per deposit, not per account.** NEAR's per-account address
  is publicly computable, so anyone can watch your Bitcoin address given your
  NEAR account. That is a privacy leak we specifically must not ship: the whole
  premise is Zcash-anchored, and plan §12 says private ownership. A fresh
  address per deposit is what ordinary Bitcoin wallets already do.
- **Beat it — batch the exits.** Rainbow Bridge spends one L1 transaction per
  withdrawal. We already compress: anchoring exists, and withdrawals to one chain
  in one epoch are one signing round and one transaction. Same argument as §5.6,
  applied to the bridge.
- **Avoid — Rainbow Bridge's light-client cost.** An Ethereum light client on
  the far side is genuinely trustless and genuinely expensive (gas, plus a
  multi-hour optimistic challenge window). Threshold first; proofs when they are
  cheaper than the trust they remove.

**10.7 — deposits carry an index and a transaction reference.** ✅

Internet Computer's ckBTC is the sharpest contrast available, and studying it
exposed a hole. On ICP, deposit *detection* lives inside consensus: the
replicas run Bitcoin adapters and agree on the UTXO set, so nobody is trusted to
say a deposit happened. Zyn cannot copy that — a VM that could observe Bitcoin
would not be deterministic (**S1**) — but the comparison showed our
`CreditDeposit` minted supply and raised backing **with nothing checking either
one**, and nothing stopping the same external deposit being credited twice.

Worse, it made S11 quietly weaker than it reads. `supply == confirmed` is
**self-referential on the deposit side**: both figures move in the same intent,
so a sequencer inflating them together breaks no invariant. Conservation was
proving the chain agreed with itself.

Half of that is now closed. A credit carries:

- **an index**, which must be exactly the asset's next one. A replay collides
  with an index already used; a gap is refused too, or one could be backfilled
  and accepted out of order. `TokenInfo::deposits` is O(1) — a counter, not a
  set of every txid ever seen.
- **an external reference** — the txid or outpoint on the custodying chain. The
  VM does not interpret it, but it rides in the intent, which the epoch's
  `intent_root` already commits. So every minted unit stays permanently
  traceable to a named external transaction that anyone with that chain's node
  can check, and the state does not grow to hold that history.

**The other half is now closed too: deposits are provisional until anchored.** ✅

A credit is minted and backed but lands in `Account::incoming`, not in a
balance. It becomes spendable when `ConfirmAnchor` records that the epoch
containing it reached Zcash under a threshold certificate — which the node emits
only after collecting and verifying one. So a fabricated credit sits in plain
view, inside the intent set a quorum must endorse, **before it can move**.

Before: a deposit required one operator's word, and that word was immediately
spendable — mint, swap for a real asset, take the proceeds. After: **a deposit
requires the same quorum a withdrawal does.** That symmetry was the thing
missing.

The VM cannot check the certificate; what it enforces is the shape, and the
shape is what stops the obvious attacks:

- An epoch cannot be finalised **before it is sealed**, so a sequencer cannot
  fabricate a credit and release it in the same breath.
- Nor **twice**, nor **backwards**.
- A credit takes the **later** epoch when topped up, so a fresh deposit cannot
  ride out on an older one's finality.
- Unclaimed credits count against supply, so the backing identity holds for the
  whole time a deposit is in the air rather than only at its ends.

`ConfirmAnchor` **sweeps** rather than requiring each holder to claim — a deposit
that lands and then waits to be collected is a worse product. The cost is a pass
over the accounts once per anchor, which is real work in a guest and belongs on
the metering list (§8.1).

**10.13 — the vault's observed balance is a mint ceiling.** ✅

A shielded Zcash vault emits nothing: there are no contracts, and its balance is
not public. Someone holding the viewing key has to look. `AttestVaultBalance`
turns that looking into a recorded number the chain enforces against:

> **Units issued can never exceed units the vault was last observed to hold.**

A credit past the observation is refused outright, whatever room the other
ceilings leave. A report *below* what has already been issued is refused too —
loudly, as `AttestedShortfall` — rather than absorbed quietly or allowed to
freeze the chain on a failed invariant. Naming the condition leaves the decision
with whoever can act on it. Stale observations cannot overwrite fresher ones.

This does not remove the trust; it **changes its shape**, which is the point.
Fabricating a credit now requires a separate, dated lie about a balance that
every viewing-key holder can check independently — so the obvious operational
move is to **give the vault's viewing key to every signer**, and have each verify
before endorsing the epoch that releases the deposits (§10.12). That is what
turns a quorum endorsement from a rubber stamp into seven independent checks.

The wider question — whether to publish the viewing key openly — is a real
trade and not settled here. Public means anyone can falsify a bad attestation;
it also exposes the deposit graph, which cuts against the private-ownership goal
(plan §12). Distributing it to signers and named auditors is the middle, and
probably right for V1.

**10.14 — certificates are verified, and a configured node fails closed.** ✅

`Certificate::verify` used to check membership and count and **not a single
signature byte**, so anyone holding public key material could assemble one out
of nothing. Signers are now ed25519 public keys and every endorsement is
verified against the anchor id. A forged certificate carries weight **zero**; a
signature lifted from a different anchor counts for nothing; one flipped bit
drops a signer.

Independent signatures rather than an aggregate scheme, deliberately: an anchor
certificate never leaves Zyn, and aggregation buys a smaller on-chain footprint
where there is no chain to be small on. A threshold scheme *is* needed for the
Zcash side — one signature has to come out of the vault key — and that is a
different problem with a different tool, outside this repo.

`Node::with_signers` makes it live. A node with a set configured will not settle
an anchor without a certificate that clears it: the anchor does not happen, the
epoch stays unfinalised, and the deposits in it stay unspendable. **Failing
closed is the point** — a missing certificate that merely logged a warning would
make every deposit rule in the VM advisory. Without a set the node runs as a
trusted operator, which is the plan's V0 posture and must not be a mainnet one.

**What remains.** A colluding quorum can still endorse a fabricated deposit —
but that is the custody model the plan states, and it is the same assumption a
withdrawal already rests on. The residual is now *stated* rather than hidden:
7-of-10 signers, not one operator.

**10.8 — exits time out and can be taken back.** ✅

`RequestWithdrawal` was a one-way door: units left the balance, and if the
signers never confirmed they sat in a queue nobody could drain. That is a direct
violation of the plan's ninth invariant — *a failed sequencer must not make
reserves permanently inaccessible* — and it was there from the first version of
the withdrawal path.

`CancelWithdrawal` returns them after `Params::exit_timeout_epochs`. Three
properties make it safe rather than a double-spend:

- **Not cancellable on demand.** An exit that could be withdrawn at will could
  race a payout already in flight and be paid on both sides.
- **Topping up restarts the clock**, so a claim cannot be accumulated over many
  small requests and cancelled whole once the oldest ages out.
- **A late confirmation finds nothing pending and is refused**, so the chain
  never releases backing twice.

What the chain *cannot* protect is an operator who broadcasts after the
deadline: they will have paid out on the far chain with nothing here left to
burn. So the timeout is a **coordination deadline**, not a user convenience —
set it far beyond any honest settlement. The default is 2,016 epochs.

**10.9 — exits respect the custodying chain's dust limit.** ✅

`TokenInfo::min_exit`, per asset. Not anti-spam — correctness. Bitcoin will not
relay an output below its dust limit, so an exit under it is a payout that
cannot be broadcast: units burned here and nothing delivered there. Every
chain's floor is different, which is why it is per asset and not a parameter.

**10.10 — vault ceilings, total and per epoch.** ✅

`Vault::cap` is the plan's staged mainnet posture (Phase 4: "limited ZEC deposit
cap"). A ceiling costs nothing while the vault is small and is the difference
between a bad day and a total loss once it is not.

`Vault::epoch_cap` is the one that matters for **10.7**. The VM cannot verify a
deposit happened, so a sequencer able to fabricate credits could otherwise mint
without limit between one anchor and the next. A per-epoch ceiling turns that
from unbounded into one epoch's worth — a quantity signers can be asked to check
*before* they sign the anchor that settles it. The allowance is restored by the
next epoch and only ever by one, however long the vault sat idle.

It does not make fabrication impossible. It makes it **bounded, visible, and
survivable**, which is what is available without putting signature verification
inside a deterministic VM. Say it that way, not more strongly.

**10.11 — settlements pay the longest wait first.** ✅

Payouts sort by request epoch, not by account. Both are deterministic — which is
all signing requires — but if a settlement is ever truncated by a transaction
size limit or a partially funded vault, address order would make some keys
permanently unlucky. Ordering by wait is the only version that is fair under
truncation.

**10.4 is open and it is the next real decision.** Per-deposit derivation needs a
scan window (how many unused addresses a watcher checks) and a recovery story if
the derivation path is lost. Both solved problems in wallet engineering, neither
free.

**Not decided:** whether the three bridges share one signer set or run separate
ones. Same trade as §8.7 — shared means a compromise reaches every chain's
reserves; separate fragments the quorum and multiplies operational cost.
**Settle before the second vault holds real value.**

---

## 32. Anchors on Zcash, DA off the box, a verifying replica (7 Sep 2026)

Reading the code for the "signer federation" work turned up that the exit
hatch's premises did not hold: no anchor had ever been broadcast (`anchor_now`
accepted into memory), the ledger reset at every restart, no intent survived
being applied, and `OP_ACCOUNT_PROOF` used live state. Spec:
`docs/superpowers/specs/2026-09-07-anchors-da-replica-design.md`.

| # | Decision | Status |
|---|---|---|
| 32.1 | **Record before apply.** Every intent is journaled in the VM's own bytes before the VM sees it; a refused record is a refused intent | ✅ |
| 32.2 | **An anchor is a vault self-send carrying the root**, tag `ZYA`, 10,000 zat, built and threshold-signed like an exit | ✅ |
| 32.3 | **Acceptance at confirmation, not at construction.** Propose → broadcast → confirm at depth → release deposits. One in flight; expiry abandons and rebuilds the same anchor | ✅ |
| 32.4 | Sealed epochs and the proposal in flight are persisted; the lineage is persisted; a state older than its ledger is refused | ✅ |
| 32.5 | One immutable bundle per accepted anchor, every file checkable against the memo's id; mirrors can fail, not lie; a mirror outage never blocks finality | ✅ |
| 32.6 | **A verifier never guesses.** The replica stops on fork, lineage break, bad bundle, unreproducible root, or dead mirrors, and keeps serving its last verified root | ✅ |
| 32.7 | Replicas re-serve what they verified: the mirror set grows with the verifier set | ✅ |
| 32.8 | The replica is the seed of the standby (resume from its `Saved`) and of the signer (attest its own pending-exit list). Both are additive | 🔒 |
| 32.9 | Deposit and anchor memos differ in their **first** bytes (`ZYN`/`ZYA`), never by accident of length | ✅ |
| 32.10 | `OP_ACCOUNT_PROOF` and `OP_SNAPSHOT` serve the anchored snapshot only; the snapshot is paged | ✅ |

| 32.11 | **The note trees trail the tip by the confirmation depth.** A tree that believes a block the chain drops names roots the chain never had; the anchor builder checks its root against the node before witnessing | ✅ |
| 32.12 | A consensus refusal of a broadcast is final: abandon and rebuild the same anchor, do not wait for expiry. A fetch failure at the replica is a retry, never a halt | ✅ |

| 32.13 | **The two note trees are one thing.** They are fed together, so they resume together or re-seed together; a builder that finds either diverged heals both in place | ✅ |

| 32.14 | **The trees are fed contiguously from where they stopped**, never from where a deposit scan happens to be reading. Anchors spend Ironwood only | ✅ |

**Corrected while here:** 7e.1/7e.5 (Nap Wallet exists), the "withdrawals are
not built" sentence in §14, and §7c item 4.

---

## 33. Forced inclusion (rung 1 of decentralising the sequencer, 7 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 33.1 | **A memo to the vault is a second door.** `ZYF` carries the exact `OP_SUBMIT` frame; the note pays the signer; the intent is authorised as over RPC | ✅ |
| 33.2 | Applied or rejected by the VM, a forced intent is *included* once journaled | ✅ |
| 33.3 | **Twenty blocks of grace**, then a replica records `CENSORSHIP` — an alarm, never a halt | ✅ |
| 33.4 | **A signature is applied once** (`zyn::replay`), by whichever route; the index is persisted and pruned by expiry | ✅ |
| 33.5 | Ed25519 and Solana schemes only; EVM needs recovery the scanner does not have | ✅ |
| 33.6 | Sequencers that verify before signing (rung 2) refuse to sign an anchor a replica marks censoring | 🔒 |

---

## 34. Signer federation (rung 2, 8 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 34.1 | **A deposit releases only after k of n replicas reproduce its anchored root.** The certificate, a stub since rung 0, is the gate | ✅ |
| 34.2 | **A signer is a replica with a key** — it signs the anchor id it replayed to, nothing else (`OP_ENDORSE`) | ✅ |
| 34.3 | The sequencer counts an endorsement only from its set with a signature that opens the id; the settler waits for the certificate to clear before releasing | ✅ |
| 34.4 | The bundle carries the real certificate; a replica with the set refuses an under-endorsed anchor (`Halt::Unendorsed`) | ✅ |
| 34.5 | A forged root gets no signatures → its deposits never release → visible stuck chain, not stolen funds. The sequencer keeps its liveness role, loses the power to forge acted-on state | ✅ |
| 34.6 | Vault custody is unchanged: the FROST payout key is a separate threshold still needing independent share-holders. Rung 2 federates *verification*, not *custody* | 🔒 |
| 34.7 | True operator independence (n infrastructures) is operational, not code; the mechanism is built and demonstrated on shared infra | 🔒 |
| 34.8 | **The bundle is published at broadcast, not at settlement.** Rung 0 published on settle; with settlement gated on signatures that deadlocked — signers cannot endorse a bundle that is not published. Signers now verify the proposal's bundle (empty certificate) while the tx confirms; the real certificate fills in on settle | ✅ |

---

## 35. Distributed custody signing (custody step 1 of 2, 8 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 35.1 | **No one machine can pay the vault alone.** Each share-holder runs `zyn-custodian` with one share; a threshold must each answer the two signing rounds | ✅ |
| 35.2 | The `Quorum` trait abstracts local vs remote signing; `sign_all` is the in-process `LocalQuorum`, `RemoteQuorum` speaks to custodians. `sign_all_with` drives either | ✅ |
| 35.3 | The coordinator holds no share — only the public package and the group key (both public); it aggregates shares it cannot forge | ✅ |
| 35.4 | A nonce is committed once and signed once; a custodian refuses to re-open a live request and forgets requests on restart. **Fail closed:** a lost attempt is nothing, a reused nonce is the share | ✅ |
| 35.5 | Everything on the wire is public (sighash, randomizers, commitments, shares); a tampered share does not aggregate. The share never crosses | ✅ |
| 35.6 | Zcash (anchors + exits) first; Solana custody is the same abstraction, a later pass — **done, 35.8** | ✅ |
| 35.7 | ~~The shares were still born together~~ — **distributed key generation built (step 2, `zyn-dkg`)**: each participant runs the FROST rounds on its own machine, round-two packages are sealed to their recipient (x25519 + ChaCha20-Poly1305) so the relay forwards ciphertext it cannot open, and each ends holding one share of a key no process ever saw whole | ✅ |
| 35.8 | ~~Solana custody (ed25519) still signs in-process~~ — **built (8 Sep 2026)**: `SolanaQuorum` / `LocalSolanaQuorum` / `RemoteSolanaQuorum`, `custody::sign_with` and `prepare_with`, `ZYN_SOLANA_CUSTODIANS` on the settler | ✅ |
| 35.9 | Solana keeps its own ops (3/4) and its own share directory rather than reusing the Zcash ones. They are different ciphersuites and different ceremonies; a daemon may hold one, the other, or both, and refuses ops for a share it does not hold | ✅ |
| 35.10 | Solana's path is deliberately the simpler one: plain FROST over one message, one nonce per request, no per-action randomizer. The Zcash complexity is Orchard's re-randomization, not FROST's | ✅ |
| 35.11 | **A custodian signs only what it committed to.** Round two must carry the message round one committed the nonce to, on both vaults; otherwise `WrongMessage`. Closes a coordinator swapping the message under a live nonce | ✅ |
| 35.12 | The request id is generated inside the signer, freshly per attempt, so a retry after a timeout never collides with nonces a custodian still holds | ✅ |
| 35.13 | The sequencer takes the vault address from the public package, never from a share — otherwise the federated path indexes a share it does not have (caught by the live test, not the unit tests) | ✅ |

## 36. Signer-set availability and the verification loop (8 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 36.1 | **A signer that cannot see Zcash is a signer that is down.** The 8 Sep stall (one dead SSH tunnel, two blind Mac signers, 1 of 3 endorsing, ~3 h with nothing releasing) is the reference failure | ✅ |
| 36.2 | Everything the Mac contributes runs under `launchd` with `KeepAlive` — tunnel, both signers, custodian, its Zebra — on release binaries in `~/.zyn/bin`. A network change reconnects rather than ending quietly; verified by killing the tunnel and watching the remote reverse forward come back | ✅ |
| 36.3 | **An unendorsed anchor is a fault, not a quiet success.** `endorsement_health(limit)` reports down past `ZYN_ENDORSE_STALL_PASSES` (default 20 ≈ 10 min). Every pass "succeeding" while no deposit releases is exactly the state that hid for three hours | ✅ |
| 36.4 | **Verification must not be circular.** All three signers read Zcash through a tunnel to the sequencer's own Zebra, so a lie from that node would be endorsed unanimously and one dead wire took out two signers. Each signer gets its own node. **10 Sep: signer1 now reads its own (`127.0.0.1:18242`, synced).** signer2 and signer3 still read the sequencer's; the Air is the candidate for the second independent view (`zebrad` built, 80 GB free) | 🔨 |
| 36.5 | Blocking a bad root needs enough independent signers that the rest cannot reach the threshold — at 2-of-3 that means **two** independent views, not one. So the Mac's node alone does not close 36.4 | ✅ |
| 36.6 | **Two of three signer keys on one laptop is the wrong shape**: that machine is both sufficient to certify alone and necessary for any quorum. Moving signer2 to a third host is the fix. **Closed 9 Sep: the host is the idle M1 Air**; signer2 runs there and is `disabled` on the Windows box. Three keys, three machines, none sufficient alone | ✅ |
| 36.7 | The sequencer's box holds 1 of 3 Zcash vault shares at threshold 2, and **2 of 3 Solana shares** (Solana custody is not yet federated in the live config). Owning that box should not be most of a quorum. **10 Sep: fixed for the mainnet vault only** (§42 — shares on Mac/Air/Windows, sequencer holds none). Testnet Zcash and Solana are unchanged | 🔨 |

## 37. Deposit validation (8 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 37.1 | **A signer checks the money, not only the arithmetic.** Reproducing a root proves the sequencer computed honestly over what it published; it says nothing about whether the credits were real. `Halt::Unbacked` is the refusal | ✅ |
| 37.2 | The check is per-txid against the verifier's **own** node — `external_ref` is the Zcash txid, so no range scan, note store or attribution file is needed to ask "did this transaction pay the vault?" | ✅ |
| 37.3 | Credits against one txid are summed before checking, so a replay cannot be split across two indices and pass twice | ✅ |
| 37.4 | Anchors are excluded from what backs a credit: a vault self-send is evidence, not money in | ✅ |
| 37.5 | **Off by default** (`ZYN_VERIFY_DEPOSITS=1`). A replica pointed at the sequencer's own Zebra would be asking the party it is checking — worse than not asking, because it looks like verification | ✅ |
| 37.6 | Where a deposit's memo names an account, the sequencer may not credit a different one. Where no memo does, the money is verified and the **owner remains the operator's word** out of `ZYN_ZEBRA_ATTRIBUTIONS`, which no verifier can see | 🔒 |
| 37.7 | Without a note store a verifier cannot tell the vault's own change from a stranger's memo-less deposit, so 37.6 is also what stops change being credited as a deposit | 🔒 |
| 37.8 | **Per-account deposit addresses** settle 37.6 and 37.7 together: the receiving diversified address names the account, and change goes to the vault's own address. Orchard hides the recipient address from the chain either way, so this is a verifiability decision, not a privacy one — **built and live 8 Sep** | ✅ |
| 37.10 | `deposit_index(account)` = SHA-256 of `"zyn.deposit.index.v1" ‖ account`, first 11 bytes, never zero. Deterministic and domain-separated, so a verifier recomputes it from the account a credit names and checks the note arrived there. Deriving the address needs the viewing key, so knowing an account id does not let the public enumerate addresses | ✅ |
| 37.11 | **Index 0 is the vault's own address** — change, anchors, operator top-ups — and no account is ever assigned it. That single reservation is what separates a stranger's deposit from the vault paying itself, with no note store | ✅ |
| 37.12 | Attribution order is address, then memo, then change, then the operator's file. A note at index 0 falls through to the old order, so deposits made before per-account addresses keep working | ✅ |
| 37.13 | `ZcashBacking::strict` refuses a memo-less credit at index 0 — change dressed as a deposit. Off until live deposits have moved to per-account addresses, because the three credits already on chain were made the old way | ✅ |
| 37.14 | Confirmed independently by zash.fun, which ships the same scheme ("the address is the order") for the same reason: no memo, no wallet plugin, any shielded wallet works | ✅ |
| 37.9 | Validating the attestation itself is secondary: once every credit must match a real transaction, minting is bounded by real deposits whatever `observed` says | ✅ |
| 37.15 | The register (`deposits::Book`) is written **before** the address is handed out. The other order leaves a depositor paying an address the scanner does not know to watch, and that money looks unattributable | ✅ |
| 37.16 | `OP_DEPOSIT_ADDRESS` is unauthenticated: the caller had to name the account to ask, and without the vault's viewing key the address discloses nothing about it. Requiring a signature would only stop people from being paid | ✅ |
| 37.17 | The all-zero account is refused an address — it is the vault's own, and the classifier already treats it as a top-up rather than a credit | ✅ |
| 37.18 | The wallet remembers its address on disk. It is a pure function of the account, so it cannot go stale, and a wallet that cannot reach the node can still be paid into | ✅ |
| 37.19 | Strict verification is its own opt-in (`ZYN_VERIFY_DEPOSITS=strict`), not implied by `=1`: turning it on before deposits have moved would halt a signer on the next memo-less deposit to the old address | ✅ |
| 37.20 | Deposits from other wallets are handed out as **ZIP-321 payment requests** (link + QR), not as an address to retype. A hundred-character shielded address copied by hand is the likeliest way a deposit goes wrong; a memo typed by hand is the likeliest way one arrives unattributable | ✅ |
| 37.21 | The QR carries no amount and the copied link carries whatever is in the amount box. A QR that hardcodes an amount is stale the moment someone wants to send a different one | ✅ |

## 38. The network stops being a constant (9 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 38.1 | `ZYN_NETWORK` defaults to **testnet**. Mainnet is opt-in, never inherited from a forgotten flag | ✅ |
| 38.2 | Address encoding takes the network as a **required argument**. A default would be invisible until a depositor's wallet refused the address; the compiler asks instead | ✅ |
| 38.3 | **The node is asked which chain it is on, and disagreement refuses the boot.** Every other safeguard assumes the chain underneath is the configured one, and nothing downstream can detect otherwise — everything downstream is consistent with itself | ✅ |
| 38.4 | The mainnet refusal moved from `Zebra::connect` to `Zebra::observe`: connecting to mainnet is the point, and it is the **transparent** watcher — addresses in the clear — that was never meant to hold real money | ✅ |
| 38.5 | `zyn-ceremony` prints the network and the expected address prefix, because a vault born for the wrong chain is discovered by whoever funds it | ✅ |
| 38.6 | A mainnet mint still needs a mainnet node (~500–600 GB, days of sync) and a **freshly DKG'd vault key** — the current one's shares have been copied between machines all through development. **Both closed 10 Sep**: mainnet Zebra reached tip on the SanDisk, and the vault key was DKG'd across three machines (§42). The daemon is not yet pointed at it | ✅ |

## 39. Resting offers, escrowed on the chain (9 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 39.1 | **The test for what belongs in the VM is whether an application could build it on what Zyn already exposes.** An app can build a marketplace on co-signed `AcceptOffer` today — but only one where the seller is online to sign for each buyer. Passive, non-custodial offers it cannot build at all, so they are protocol | ✅ |
| 39.2 | `PlaceOffer` / `TakeOffer` / `CancelOffer`, asset-agnostic. No collection fields, no marketplace vocabulary: the primitive unlocks a class — marketplaces, OTC, resting bids — rather than serving the NFT store that motivated it | ✅ |
| 39.3 | **Escrow, not a maker signature carried inside the intent.** The alternative needed a second authorization path *and* a set of spent offer digests in state, because the replay guard keys on the credential and would not have stopped a second taker naming themselves. Escrow makes the double take impossible by construction — the asset is gone — and makes cancellation an ordinary withdrawal | ✅ |
| 39.4 | The asset moves at **placement**, not at the take. An offer whose asset stays spendable by its maker is an advertisement: a taker could satisfy every check and find nothing there | ✅ |
| 39.5 | Deliberately the opposite of the AMM, which escrows nothing (§`state.rs` "the input stays in the account until the seal"). A batch-cleared swap lives one epoch while the account keeps its funds; an offer must outlive its maker's session. Same chain, opposite answer, for a reason | ✅ |
| 39.6 | One keyless escrow account, `POT_OFFERS` (`0xF6`), in the existing pot family. `state.offers` is the ledger of whose it is; the account is only where it sits | ✅ |
| 39.7 | The collection fee moved into `collection_fee_of` / `charge_collection_fee`, shared by `accept_offer` and `take_offer`. **A second way to move an item must not be a cheaper one** — a fee computed in two places is a free channel waiting to open | ✅ |
| 39.8 | Expiry blocks taking, never reclaiming: an expired offer is still cancellable by its maker. Nothing sweeps expired offers, because a transition needs someone who asked for it | ✅ |
| 39.9 | A maker may not take their own offer — the fee would be the only thing that moved | ✅ |
| 39.10 | `STATE_VERSION` 6 → 7, and the offers section joins the root **only while an offer exists**, as collections, orders and the launch already do. A chain that has never had one commits exactly the sections it did before, so **live chain 11 needs no migration and no re-anchoring** | ✅ |
| 39.11 | `OP_OFFERS` (21) is public and unauthenticated. An offer nobody can see is one nobody can take; and because it is a read, a **read-only replica serves it** — so a storefront or explorer can show the order book without the sequencer being trusted or even asked | ✅ |
| 39.12 | Discovery, browsing, filtering, images and metadata stay in the application. The chain holds the offer, not the shop | ✅ |

## 40. The chain becomes checkable from outside (9 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 40.1 | `OP_ANCHORS` (22) serves the DA index — `epoch root anchor_id txid height`. The claim "settled in batches on Zcash" is worth nothing to a viewer unless they can follow an epoch to the transaction carrying its root, and that mapping existed only as a file on the sequencer's disk | ✅ |
| 40.2 | **A replica serves it from its own DA copy.** The mapping can then be read from something that never sequenced anything — the explorer becomes an argument about decentralisation rather than a view of our own claims | ✅ |
| 40.3 | A node holding no bundles **refuses** rather than returning an empty list: silence and "nothing was ever anchored" must not look alike | ✅ |
| 40.4 | HTTP: `/api/offers`, `/api/offer`, `/api/take`, `/api/cancel-offer`, `/api/anchors`, and the collection lifecycle — `/api/collection`, `/api/fund`, `/api/mint`, `/api/advance`. The VM primitives were unreachable from anything speaking HTTP, which is everything a storefront is | ✅ |
| 40.5 | `/api/overview` gains `offers` and a `settlement` summary, so a storefront's status strip is one call rather than three | ✅ |
| 40.6 | **A paid claim is two intents, not one privileged one.** `FundCollection` is signed by whoever pays, so the buyer pays into the pool themselves and the creator only hands the item over. No new VM surface, and no path where the creator moves a buyer's money | ✅ |
| 40.7 | `tests/demo.rs` drives claim → sell → sell back → redeem over a real socket through the client a storefront would use. Its original four-half-fee assertion described the historical 50/50 collection split; §69.9 supersedes that policy with `50%` ZYN POL / `40%` NFT floor / `10%` creator. Current VM unit tests assert all three destinations and the socket demo asserts the resulting 40% floor increase. The beats were unreachable from outside the VM until this ran; a test that only called `apply` would not have noticed | ✅ current split; historical assertion superseded |

## 41. Alerts have to be able to say "fine" (10 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 41.1 | `ZYN_ALERT_URL` set on all four hosts, pointed at Telegram's `sendMessage`. **No code changed** — the webhook body already carried `text`, and Telegram takes `chat_id` from the query string. The alerter had been correct and unheard since it was written | ✅ |
| 41.2 | The evidence for doing it: on 9 Sep the chain sat halted for over a day while `alert.rs` announced `verification DOWN` to journald, where nobody reads. Detection was never the problem; delivery was | ✅ |
| 41.3 | **Censorship is alerted on what is happening now, not on the historical record.** `Replica::censored()` is permanent and stays so — an incident is not undone by age — but the health check now counts only events within `CENSORSHIP_CURRENT` (10 grace periods) of the scanned tip | ✅ |
| 41.4 | Why: the 7 Sep censorship *demonstration* (`ZYN_IGNORE_FORCED=1`) put a real entry in that list, and the check reported `DOWN … last ok never` for three days. An alarm that can never clear trains its reader to scroll past, and the next real one goes unread with it — which is nearly what happened to §41.2 | ✅ |
| 41.5 | The lifetime count is still reported, as the health source's `scanned_to`, so a past incident stays visible to `/signers` without paging anyone | ✅ |
| 41.6 | `zyn-tgbot`: a read-only Telegram bot beside the sequencer. Buttons for chain, signers, anchors, offers and both Zcash nodes. It holds no key and calls only public read ops, so a leaked token discloses the height — which the DA store publishes anyway | ✅ |
| 41.7 | Zcash nodes are judged against Zebra's `estimatedheight`, not their own height. A stalled node reports whatever height it stopped at and looks healthy; only the gap to the network's view exposes it | ✅ |
| 41.8 | Mainnet Zebra reaches the bot through `zyn-zebra-tunnel` — its own systemd unit, so a monitoring forward can never take `zyn-custody-tunnel` down with it | ✅ |

## 42. The mainnet vault key, generated distributed (10 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 42.1 | The mainnet vault key was created by **DKG across three machines**, not by a dealer. No machine has ever held it whole — not at creation, not since. The testnet key cannot make that claim: it was dealt and its shares were copied between machines during development | ✅ |
| 42.2 | Shares live on the **Mac, the M1 Air and the Windows box**. The sequencer holds **none** — compromising Hetzner yields the coordinator and zero shares, where the testnet arrangement (custody1 beside the sequencer) yields the coordinator and one of three | ✅ |
| 42.3 | Verified three ways: all three derived an identical group key, each directory holds **exactly one** share, and all three derived the same Orchard viewing key. The group-key check is the one that catches a participant fed different packages from the others | ✅ |
| 42.4 | The relay bound to Hetzner's **loopback**, reached by each participant through its existing managed tunnel. An open port during a key ceremony is not a convenience worth having | ✅ |
| 42.5 | Mainnet vault address `u1kc38pww6c8ahz0z20g9kv9s7afrc2npmr8s6nngw00qufveq43q5m2vh8u7wxpaltl0vjs7etr7y7d0pcm5dtc24pqhu5qfe5cwtyc2u`, derived independently on each machine and agreeing | ✅ |
| 42.6 | `zyn-dkg address <dir> <mainnet\|testnet>` added. The address must be known and checked *before* funding, and deriving it by hand from a hex blob at the moment of funding is how a vault gets born on the wrong network (§38) | ✅ |
| 42.7 | A rehearsal ran first, all three participants on one box. It proved the protocol and deliberately proved nothing about distribution — that box held all three shares, which is the property the real ceremony exists to avoid. Throwaway shares deleted | ✅ |
| 42.8 | The **testnet** vault is left as it is: dealer-dealt, shares copied. It holds only TAZ, and re-keying a working system buys nothing where there is no money | 🔒 |

## 43. A Zcash outage pauses the witness, not the chain (10 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 43.1 | **Zyn keeps trading while Zcash is unreachable.** Verified 10 Sep: nothing in `main.rs` or `anchor.rs` halts, caps or degrades on an anchor or node failure. Swaps, transfers and offers are pure VM operations and never touch Zcash | ✅ |
| 43.2 | The line that matters is already drawn by the finality gate: **value moving inside Zyn keeps moving; value crossing the boundary waits.** Deposits credited during an outage do not finalise, so they stay unspendable; exits cannot be signed because they *are* Zcash transactions. Neither needs new machinery | ✅ |
| 43.3 | Halting the VM because a node is down would be the wrong trade. A temporary hardware fault on one box should not stop people trading balances that are already backed by an already-anchored vault | ✅ |
| 43.4 | **But DA bundles are gated on anchor confirmation** — `bundle_for` needs the anchor's txid and height — so an outage stops independent verification altogether, not just the Zcash commitment. The sequencer keeps executing and nobody can check what it executed | 🔨 |
| 43.5 | **Split the bundle in two.** "What I executed" (epoch intents + published snapshot) needs no anchor and should publish continuously; "Zcash's witness to it" (anchor, txid, certificate) publishes when Zcash is available. An outage then means *the witness lags*, not *verification stops* | 🔒 |
| 43.6 | Why it is not cosmetic: without it, nothing witnesses the outage window, so a divergence — accidental or not — is only checkable retroactively against a history the operator alone published. It is also a durability risk: if the sequencer's disk fails mid-outage, everything since the last anchor exists nowhere else | 🔒 |
| 43.7 | **Bound the unanchored window.** Alert past N unanchored epochs (the alerter already can), and a hard cap where new intents are refused while queued exits still settle once Zcash returns. Unbounded is the part not to ship: an unwitnessed week is indefensible if anything goes wrong inside it | 🔒 |
| 43.8 | Panic is answered by the status strip, not by halting: *"trading normally · last anchored 3 h ago · Zcash node recovering"* is both honest and better than an unexplained freeze | ✅ |

## 44. The forced scan gets its own marker (10 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 44.1 | **Rung 1's happy path was never demonstrated until today.** §33 recorded forced inclusion as delivered on the strength of the 7 Sep run, which proved only that a replica *notices* censorship. A real intent — txid `1c7ad023…`, confirmed twelve blocks deep — was sent on 10 Sep and never applied | ✅ |
| 44.2 | Ruled out in order: the memo layer (`forced_frame`/`forced_account` round-trip a real submission frame — now a test), `ZYN_IGNORE_FORCED`, the call path, confirmation depth, and the signers. A `ZYN_FORCED_RESCAN_FROM` sweep then found and applied the same transaction, proving the scanner *can* see it and the live pass had skipped it | ✅ |
| 44.3 | **The cause is a range gap.** `observe` collects sightings from two ranges — the note-tree feed and the deposit scan — and they diverge: on 10 Sep the deposit scan stood at 4,337,734 while the tree feed was still at 4,328,862. A block between them is read by neither, and because no sighting is made there is nothing in `forced_pending` to retry. It would have waited forever | ✅ |
| 44.4 | **Fix: `Progress::forced_scanned_to`** — the forced sweep advances on its own contiguous marker and answers to neither other scan. The same shape as the fix §32.14 applied to the note-tree feed, for the same reason | ✅ |
| 44.5 | The marker advances **only after `process_forced` has persisted what it queued**, so a pass that fails later cannot drop a sighting it has already stepped over | ✅ |
| 44.6 | A zero marker seeds from the deposit scan rather than genesis: re-reading the whole chain would be honest and useless | ✅ |
| 44.7 | **The progress file is versioned (`ZYNBRDG2`) rather than length-sniffed.** An old file is byte-for-byte a valid prefix of a new one, so length alone cannot distinguish "written by an older build" from "cut off half way" — and a truncated progress file read as whole is how a scanner silently loses its place | ✅ |
| 44.8 | **Deployed and verified 10 Sep 12:50.** The live `bridge-11-*.progress` files migrated `ZYNBRDG1` → `ZYNBRDG2` on first read, and the restart cost seconds rather than 45 min because the note tree was already current — the long rebuild happens only when the tree is far behind | ✅ |
| 44.9 | **A second forced intent (`bb9ba14d…`) was then applied through the normal path**, with no rescan flag and no intervention: `forced sweep 4337899..=4337900 found 1 intent(s)`, then `applied at seq 11830 (accepted)`. Rung 1's happy path is demonstrated by the mechanism itself, which is what §33 claimed and never showed | ✅ |

## 45. The doormat: transparent in, shielded immediately (10 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 45.1 | **Nap Wallet receives transparent funds.** Every exchange pays out to a t-address, so without one the wallet cannot be funded without a second wallet in between — and "install another wallet first" is where most people stop | ✅ |
| 45.2 | **Transparent is a doormat, not a room.** Funds are received and swept to the holder's own shielded address, and there is deliberately **no way to spend transparent funds anywhere else**. Offering the choice would invite exactly the public spending the system exists to avoid | ✅ |
| 45.3 | This does not weaken §38.4, which forbids a transparent **vault**. A vault holding funds in the clear and a wallet accepting a deposit it immediately shields are different things | ✅ |
| 45.4 | Derivation is **BIP-32/44 at `m/44'/133'/account'/0/index`** — the path every other Zcash wallet uses, so a holder can always sweep these funds with something else. Interoperability is the whole reason not to invent a bespoke derivation from the same seed | ✅ |
| 45.5 | `UTXOS` (op 6) is the light protocol's **one exception** to "nothing that takes an address". A compact block carries only Orchard actions, so there is nothing client-side to scan for transparent funds. What it costs is the association between a t-address and whoever asked; the address is public either way, and shielded balances stay invisible to the server | ✅ |
| 45.6 | **Shielding targets Ironwood, not Orchard.** From NU6.3 the Orchard value balance must be non-negative, so value cannot flow *into* that pool at all — a t→Orchard sweep is refused by consensus. Found by building one and having the node say so | ✅ |
| 45.7 | The sweep's shielded side is a **change** output, not a payment: the money is already the holder's. That is also the only shielded output NU6.3 lets the bundle make, so naming it correctly is what makes it work | ✅ |
| 45.8 | ZIP-317: the shielded side is charged for the bundle's **padded** action count (two), not its one output. Counting the output underpays and the node rejects it as "unpaid actions is higher than the limit" | ✅ |
| 45.9 | `plan()` reports what would actually land before anything is built. Sweeping dust costs more than it moves, and a wallet that spends someone's money to move nothing is worse than one that refuses | ✅ |
| 45.10 | **Proven end to end on testnet**, not merely built: 40,000 zat sent to the wallet's own t-address, swept in `dd13839…f094ef`, transparent balance back to zero and the 25,000 net landed in Ironwood. Today's forced-inclusion bug was exactly a happy path nobody had ever run | ✅ |
| 45.11 | **The doormat has a door.** The wallet tab's Receive panel carries a "Coming from an exchange?" drawer with the t-address and a **Make it private** sweep. Built but unsurfaced, the whole thing was worth nothing to the person it exists for | ✅ |
| 45.12 | The transparent balance is tinted with `--record`, **not** `--private`. It is the one number in this wallet that anyone can see, and the tint is how the layout says so | ✅ |
| 45.13 | The balance is a **network lookup** (`UTXOS`), so it loads when the drawer opens, not on every render tick. `/api/shield` syncs and proves, so it runs as a job like `/api/send` rather than holding the wallet lock inside the request | ✅ |
| 45.14 | `/api/transparent` returns **ZEC**, like every other amount on this API. It shipped in zatoshi with no consumers; a UI written against it would have displayed every figure 100,000,000× too large | ✅ |

## 46. App icons carry the mark (10 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 46.1 | The desktop bundle and the browser extension both ship the **brand kit's light app icon**, replacing the teal-ground gold "N" placeholder generated on 6 Sep. Ivory reads against a dark dock and a light one; a teal ground also contradicts §45's own kit, which reserves teal for action | ✅ |
| 46.2 | **At 32 px and below the `z z Z` motif is dropped**, per the kit's own rule. Rendered at 32 it smears into an orange blur against the mole's ear. `app-icon-light-small.svg` is the simplified master; removing the motif also centres the artwork exactly, which is how the original was constructed | ✅ |
| 46.3 | `.icns` and `.ico` carry **per-size artwork**: simplified at 16 and 32, full at 64 and above. One source scaled to every slot would ship the blur to the Windows taskbar and the macOS sidebar, the two places the icon is smallest and seen most | ✅ |
| 46.4 | Rasterised through **headless Chrome**, not ImageMagick's SVG renderer, which is 36% off by RMSE on this file. The `.ico` is written by Pillow: ImageMagick's uncompressed BMP entries made it 372 KB against 39 KB for the same seven sizes | ✅ |

## 47. The balance widget shows holdings, not activity (10 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 47.1 | The Balance card **lists what the wallet holds** and no longer repeats three activity entries. The Activity tab already exists; a card that duplicates it spends its best space saying something the reader can get one click away | ✅ |
| 47.2 | The big number is the **whole holding in ZEC**, across Zcash and Zyn, not the Zcash balance alone. It reads from the chain's own `portfolio.total_zec` rather than a sum re-derived in the page, so the headline figure and the Portfolio tab can never disagree | ✅ |
| 47.3 | A ZEC-denominated holding does not print its own amount twice. The value column appears only where it converts something — LP shares, ZYN, SOL.zy | ✅ |
| 47.4 | Holdings the chain cannot price are listed **without a value and excluded from the total**, and the card says how many. Inventing a price for them would make the headline figure a guess | ✅ |
| 47.5 | Holdings take ordinary ink, not the `--record` grey: what this wallet holds is private, and only the key sees it. The tint is reserved for what anyone can verify (§45.12) | ✅ |
| 47.6 | Every quote is **in dollars**, both the headline and each holding's value. The ZEC figures stay as the amounts themselves | ✅ |
| 47.7 | The price comes from **`feeds.rs`** — the same Coinbase + CoinGecko pair, spread check and jump guard the sequencer uses for pool references. A wallet growing its own weaker price path would be a second answer to a solved question. This does not touch §8: nothing in the protocol reads it, and no pool prices off it | ✅ |
| 47.8 | Fetched by the **app, not the page**: cached ten minutes, refreshed off the request thread, and never shown past an hour old. The overview is polled every 1.5 s during a job, so a price host having a bad day must not be able to stall the wallet — and one wallet asking six times an hour beats one request per popup open | ✅ |
| 47.9 | A **switch in Settings**, on by default, saying plainly that fetching tells a price host this wallet is awake. Turned off, every figure falls back to ZEC. The cost is real and belongs in front of the holder rather than buried | ✅ |
| 47.10 | On testnet the figure is shown but labelled **"testnet, not real value"**. Hiding it would leave the feature untestable on the only network we run; showing it bare would let arithmetic imply worth that TAZ does not have | ✅ |

## 48. zynscan, and the anchor heights it found missing (10 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 48.1 | `zynscan` serves the anchor index over HTTP: epoch, state root, the Zcash txid carrying it, and the height it confirmed at. The claim "settled in batches on Zcash" was only ever checkable by someone who could speak our binary RPC | ✅ |
| 48.2 | It **points at a replica by default** (§40.2) and names on the page which node it read and whether that node sequenced anything. An explorer fed by the sequencer is a view of our own claims | ✅ |
| 48.3 | The node is read **on a timer, not per request**. A public page that queries the chain once per visitor is a way to make attention into load on the thing being explained | ✅ |
| 48.4 | It holds no key and serves no write route, and says so on the page. `GET`/`HEAD` only, `default-src 'none'` | ✅ |
| 48.5 | **Building it immediately found a real bug.** 170 of 200 published anchors carried `height 0` — settled on Zcash, published as though they never landed. Sampled four across the range against testnet Zebra: all four confirmed, including epoch 4646 at height 4,338,097 | ✅ |
| 48.6 | Cause: `write_local` appended an index line **only when the epoch was absent**. An anchor is written twice — at broadcast with no height, on settle with one — so the second write was always skipped and the height-0 line stood forever. `index_with` replaces it instead, never overwrites a known height, and never walks a settled entry back to 0 | ✅ |
| 48.7 | An anchor genuinely between broadcast and confirmation reads **"broadcast"**, not "0". A height of zero is a claim about block zero | ✅ |
| 48.8 | The fix corrects anchors from here. The ~170 already published as 0 are **stale on disk** and need a one-off backfill against Zebra before the page tells the truth about them | 🔨 |

## 49. JSON-RPC 2.0 is the public protocol; binary stays the peer protocol (10 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 49.1 | The public face of a node is **JSON-RPC 2.0 over HTTP** (`zyn-rpc`). It is what Zcash, Solana and Ethereum all speak, so an integrator learns nothing new; it sits behind nginx on 443, which is the only way `zyn-rpc.cavebro.win` can have TLS, a hidden origin and per-method rate limits; and it can be called from a browser, which the binary socket never could | ✅ |
| 49.2 | The binary framing is **not replaced**. `swapvm::wire` is the committed encoding — the epoch commitment feeds on it — and replicas, signers and the mirror already speak it. Consensus cannot move to JSON, and swapping a tested transport 22 days before the vote is not the risk worth taking | ✅ |
| 49.3 | **A write carries bytes, not fields.** `zyn_sendRawIntent` takes the submission frame as hex and relays it untouched. Re-encoding JSON parameters into wire bytes would break signatures or invent a malleability surface — which is exactly why Bitcoin has `sendrawtransaction` and Solana `sendTransaction(base64)` rather than structured params | ✅ |
| 49.4 | **A signed read stays signed.** `OP_ACCOUNT` derives the account from the signature, which is what stops a stranger reading a balance. `zyn_account` therefore takes a pre-signed payload, never an account id: accepting an id would quietly convert a private read into a public lookup | ✅ |
| 49.5 | Implemented by **splitting** `submit`/`account` into build-and-send and relay-and-decode, not by duplicating the decoders. The existing suite then covers the relay path, because `submit()` now calls `submit_raw()` | ✅ |
| 49.6 | The gateway holds **no key** and cannot sign. A frame it cannot parse is refused by the node, not by it — verified: `deadbeef` came back `-32000 node refused: missing program id` | ✅ |
| 49.7 | Errors use the spec's reserved codes, with node refusals in the implementation-defined band at **-32000**. Batches, notifications (no response, per spec), and both positional and named params all work | ✅ |
| 49.8 | Ports stay separate: binary on 8099 for peers, JSON-RPC on its own listener behind TLS for everyone else | ✅ |

## 50. A confirmation is not final, and an index entry is not a receipt (11 Sep 2026)

Found by the verifying replica halting on chain 11: `LINEAGE break at epoch
4336: StaleBase`. Two separate defects — and the first two diagnoses were wrong,
recorded here because both were plausible.

| # | Decision | Status |
|---|---|---|
| 50.1 | **Six confirmations is too shallow on testnet, and a confirmed anchor is never re-checked.** Anchor 4330's transaction was seen at depth and marked `Confirmed`; a reorg deeper than 6 then orphaned it. The sequencer builds on its own ledger, so 4336 and 4474 were chained onto a link that no longer exists on Zcash. `ZYN_CONFIRMATIONS=6`, against the code's own default of 10 | 🔨 |
| 50.2 | The settler is **not** naive about broadcast, contrary to the first reading: `an_expired_transaction_is_abandoned_and_the_same_anchor_is_rebuilt` is real and works. Epoch 4336's first transaction expired, was abandoned, rebuilt, and confirmed at height 4337902. Only 4330 is genuinely missing | ✅ |
| 50.3 | **The DA index kept the abandoned txid.** 4336's row still names the transaction that expired, not the one that settled — the same append-only `write_local` bug as §48.6, which never replaces a row once written. An index entry is therefore not evidence that *that* transaction settled | ✅ |
| 50.4 | That makes §48.6 load-bearing twice over: it must correct the **txid** as well as the height. Backfilling height alone would leave 4336 pointing at a dead transaction | ✅ |
| 50.5 | The replica was **right to halt**. It verified through 4321, refused anchors it could not find on Zcash, and stopped rather than accept a chain it could not follow. It caught a reorg-induced hole every other component had missed for days | ✅ |
| 50.6 | Healing means **4330's anchor must reach Zcash again**. A published anchor cannot be rewritten — 4336 already commits to 4330's root — so the lineage cannot be routed around it. The `ZYA` memo identifies the anchor, not the transaction, so a fresh vault self-send carrying the same memo settles it under a new txid | 🔨 |
| 50.10 | **The sequencer will not extend a lineage whose foundation has left the chain.** `send_next` refuses while any recently-confirmed anchor is missing from Zcash. Extending costs nothing now and yields a chain nobody can verify later; a stalled chain is recoverable, an unverifiable one needs archaeology | ✅ |
| 50.11 | The loss is folded into the **`anchors` health** ahead of the endorsement stall, so it reaches the alerter rather than the journal. It is the most serious of the three: the pass can succeed and endorsements be fine while nothing built since is verifiable | ✅ |
| 50.12 | `ZYN_CONFIRMATIONS` **6 → 12**. Twelve does not make a reorg impossible, which is why it is the least important of these changes — detection and the gate are what stop a deep one turning into silent corruption | ✅ |
| 50.15 | **Re-basing a replica past a gap forfeits its endorsement of everything before the new base.** It clears the halt and it is not free: with a threshold of 2 of 3, one re-based signer leaves pending older anchors permanently short a vote, and the deposits behind them unspendable. Weigh that before reaching for it — §50.7 refuses it as concealment, and this is the separate, practical cost | ✅ |
| 50.13 | **A repaired anchor lands out of chain order, and that is not enough to heal a lineage.** Epoch 4330 was successfully re-anchored (`86c7c1f2…`, height 4338929) — but 4336 sits at 4337902, and `replica.rs` applies sightings `sort_by_key(|s| (s.height, s.epoch))`, deliberately "in chain order". Sorted by height the replica still meets 4336 first and still halts. Putting the transaction back is necessary and not sufficient | 🔨 |
| 50.14 | Three ways forward, none free: order sightings by lineage (`previous_root` already validates the order, so it is safe — but it drops "what Zcash saw" as the ordering authority); re-anchor the whole tail after the gap so chain order is restored; or accept the gap and start the replica from a base past it (§50.7 refuses this only as a way to *hide* the hole, not as a documented choice) | 🔨 |
| 50.9 | **A historical anchor cannot be re-sent as it was.** The vault spends a chain of single change notes — "the vault's anchors serialise on the previous change note confirming" (`anchor.rs`) — so 4330's input was consumed by the anchors that followed. Rebuilding its transaction is refused on chain: `ironwood double-spend: duplicate nullifier`. Healing must spend a **current** note and carry the same `ZYA` memo, and `ZYN_REANCHOR_EPOCHS` must fire after the scanner knows the vault's notes, not at construction | 🔨 |
| 50.7 | Moving `ZYN_BASE_ROOT` past the gap is **refused as a remedy**. It declares those epochs unverifiable and calls the chain healthy, which is exactly the claim an outside reader checks | ✅ |
| 50.8 | Both wrong diagnoses came from sampling: "all `height 0` rows are confirmed" (four samples, all confirmed — 4330 was not) and "the sequencer treats broadcast as settled" (it does not; its ledger showed the rebuild). Read the sequencer's ledger before theorising about its behaviour | ✅ |

## 51. Agents receive mandates, not wallet authority (11 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 51.1 | **The old `CAP_OPERATE` boundary is a warm-key boundary, not an agent boundary.** Its grouping was deliberate: a warm key may waste value but cannot take it off Zyn. An agent breaks that assumption because `Transfer` to an attacker's Zyn account is a total loss without leaving the chain. Agent sessions therefore default to `CAP_SWAP`; transfer, offers, liquidity, items, collections and privacy each have separate capabilities, while withdrawal and delegation remain separate | ✅ |
| 51.2 | **Unknown future intents remain unavailable to sessions.** The default `intent_capability` demands a capability set containing `CAP_DELEGATE`, which no delegation may possess. Rejected: treating an unclassified intent as ordinary operation, because a new value-moving intent would silently inherit old grants | ✅ |
| 51.3 | **Mandate verification lives in `zyn-vm` and runs from the committed authorization bytes.** Sequencing, journal replay, replicas and the proof guest all re-derive the same decision. Rejected: enforcement only in Nap or at the RPC edge, because those decisions cannot be reproduced by a replica or proof and would make “enforced by the chain” false | ✅ |
| 51.4 | A delegation is consequently a **chain-format change**, not merely RPC op 23: the owner certificate, session signature and signed constraints are inside `ZYNAUTH1`; ZynZap `STATE_VERSION` is 8, VM version is 2, and the journal is `ZYNJRNL2` with explicit v1 rollover rules. Rejected: putting delegation fields in `swapvm::wire` as transport-only metadata, because that wire is the epoch commitment | ✅ |
| 51.5 | **The first agent surface is an allowlisted CLI over a separate authenticated Nap listener.** `nap-agent` has no key path, node address, generic wallet method or raw-RPC escape hatch; Nap owns the session keys and human mandate creation. Rejected for this milestone: direct node access, the wallet's general API, and making the security boundary depend on MCP before the command contract is stable | ✅ |
| 51.6 | Current mandates are **stateless and per action**: assets, pools, input amount, slippage and epoch bounds are chain-enforced; request idempotency and pause/close are local to Nap. Rejected: describing the per-action ceiling as a cumulative budget or local pause as on-chain revocation | ✅ |
| 51.7 | A **dedicated agent account** is the settled way to make its balance a hard cumulative loss bound. Until it exists, the controlled account's whole balance is the cumulative exposure and copied session credentials remain usable until expiry despite a local pause | 🔒 |
| 51.8 | **Chain-format upgrades roll out verifier first:** every signer and read-only replica must accept the new state and committed authorization format before the sequencer can emit either. Rejected: sequencer-first deployment, because an old replica deterministically halts at version 8 or `ZYNAUTH1`; on chain 11, with signer2 already stale and threshold 2 of 3, losing either signer1 or signer3 removes all endorsement quorum | 🔨 |
| 51.9 | **Wallet fonts are application assets.** Nap bundles Bricolage Grotesque, IBM Plex Sans and IBM Plex Mono under their OFL licences; the daemon serves them from its own origin and the desktop and extension builds package the same files. Rejected: Google Fonts, because opening a shielded-by-default wallet must not tell Google that the wallet was opened. Tauri's CSP also refuses remote font and stylesheet origins | ✅ |

## 52. Zyn mainnet is chain 26460 (11 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 52.1 | **`ZYN_CHAIN_ID=26460`**, permanently. The id sits inside the signing domain (`zyn.read.v1 ‖ chain_id`) and every anchor commitment, so changing it is a new chain rather than a migration. Chosen over `1` — which is a code default, and a node with a forgotten `ZYN_CHAIN_ID` must never be able to claim mainnet identity by accident | ✅ |
| 52.2 | **`ZYN_PARAMS=v1`**: production floors. Its only difference from `testnet` is `min_pool_xzec` — 1 ZEC against 0.001 — which gates *token launches*, not collections, so the NFT path is unaffected | ✅ |
| 52.3 | **`ZYN_NETWORK=mainnet` is separate from `ZYN_PARAMS`** and both are required. The VM profile and the Zcash chain are different questions; the node refuses to start when the configured network disagrees with the one the Zcash node reports, which is what caught the first attempt | ✅ |
| 52.4 | **`ZYN_BRIDGE_FROM_HEIGHT=3479600`**, just below the tip at creation. The default is 0, which would rescan 3.48 M blocks; nothing has ever paid this vault, so anything at or below the first deposit is safe | ✅ |
| 52.5 | **`ZYN_CONFIRMATIONS=12`**, not the default 10 (§50.12) | ✅ |
| 52.6 | It runs on **Contabo against that host's own mainnet Zebra** (reached tip 11 Sep), so mainnet depends on no home machine — the thing the whole exercise was for | ✅ |
| 52.7 | The vault's **viewing** key only, mode 600. It finds deposits and cannot spend; spending needs 2 of 3 custodian shares, none of which live on the sequencer (§42) | ✅ |
| 52.8 | **Not yet a mainnet posture, and it says so**: `anchors IN MEMORY ONLY (no ZYN_ZEBRA_SHARES): roots never reach Zcash`. Custody tunnels and `ZYN_DA_MIRRORS` come next; until then nothing settles to Zcash | 🔨 |
| 52.9 | Port 8099 stays **closed** at the firewall. The binary RPC cannot be TLS-fronted (§49.1) and the node is in a devnet posture; `zyn-rpc.cavebro.win` will front the JSON-RPC gateway instead | ✅ |

## 53. `zec-rpc.cavebro.win` — a public Zcash RPC, default deny (11 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 53.1 | Zcash mainnet RPC is public at **`https://zec-rpc.cavebro.win`**, TLS-terminated by nginx on Contabo. Zebra itself stays on **loopback**; nothing reaches it except through the filter | ✅ |
| 53.2 | **Default deny by method.** An allowlist of reads plus `sendrawtransaction` — relaying someone else's transaction is what a node is for. Absent on purpose: `stop` (shuts the node down), `addnode`/`setban` (node control), `getblocktemplate`/`submitblock` (mining), `getpeerinfo` (our topology). A future Zebra release cannot quietly widen the surface | ✅ |
| 53.3 | The filter is **njs, not an nginx `map`**. Two reasons, both found by testing rather than reasoning: `$request_body` is empty during the rewrite phase, so a regex map denies *everything* — it looked like working security and was not; and a JSON-RPC **batch** is a list, which a regex cannot check honestly. One bad method now refuses the whole batch | ✅ |
| 53.4 | Rate limited to **8 r/s, burst 20, 8 connections** per IP. Measured: 80 parallel requests → 21 served, 59 shed. A public node is a shared resource and one client must not be able to spend it | ✅ |
| 53.5 | 64 KB body cap, `POST`/`OPTIONS` only, no server tokens, CORS open for reads — the node holds nothing private | ✅ |
| 53.6 | **`zyn-rpc` is NOT public** and will not be until ZynZap is mature, which is after Cave's secondary trading opens. Port 8099 stays closed at the firewall; the name resolves and answers nothing by design | ✅ |
| 53.7 | Contabo carries **mainnet only** — mainnet Zcash and mainnet Zyn. The testnet Zebra put there on 10 Sep was removed | ✅ |

## 54. Mainnet anchors reach Zcash (11 Sep 2026)

Closing §52.8: chain 26460 went from *"anchors IN MEMORY ONLY … a devnet
posture, not a mainnet one"* to signing against a real threshold.

| # | Decision | Status |
|---|---|---|
| 54.1 | **Remote custody.** The sequencer holds `public.bin` and **no share** — the node says so itself: `anchors signed by 2 custodian(s), threshold 2 — this box holds no share`. Verified on disk: zero `share-*` files on Contabo. §42 is a property of the deployment, not just of the ceremony | ✅ |
| 54.2 | Custodians reach the sequencer over **reverse tunnels they open**, so no share-holder needs a public address or an inbound hole: 8210 is share 01 (Mac), 8211 is share 03 (Windows). Each key is pinned with `permitlisten` to exactly its own port | ✅ |
| 54.3 | **Share 02 (the Air) is not wired**, so a 2-of-3 vault is running with two custodians and no slack: either one going down stops mainnet anchoring. Acceptable while nothing is funded; not acceptable once it is | 🔨 |
| 54.4 | DA bundles are pushed to the **Hetzner mirror** as well as kept locally, so they outlive the box that made them. Verified end to end: `chain-26460/base.state` fetches over HTTP from the other machine | ✅ |
| 54.5 | Hetzner's mirror is not reachable from outside — its **cloud firewall** blocks 8181 while the OS has no firewall at all, so `ss` shows `0.0.0.0:8181` and the port is still filtered. Rather than open it, Contabo reaches it through an **outbound tunnel** pinned with `permitopen`. The mirror stays unexposed and no console change was needed | ✅ |
| 54.6 | `ZYN_ZEBRA_REVEALS` must exist even when empty — a fresh chain has no account attributions, and the node refuses to start on a missing file rather than inventing an empty one | ✅ |

## 55. Share 01 moves to Hetzner, and what that costs (11 Sep 2026)

The Mac travels and sleeps, so it cannot hold a share of a vault whose
availability gates settlement.

| # | Decision | Status |
|---|---|---|
| 55.1 | **Share 01 moved, not copied.** Transferred Mac → Hetzner over SSH, hash-verified at the destination, then deleted from the Mac. At no point after the transfer do two machines hold it. The distinction matters: §42.1 records "dealt and its shares were copied between machines" as exactly what makes the *testnet* key inferior | ✅ |
| 55.2 | **The property that is lost:** the mainnet vault could previously claim no share had ever left the machine that generated it. It cannot now. Share 01 transited the network once, encrypted, direct. That is a real reduction, accepted deliberately for uptime | ✅ |
| 55.3 | **The property that survives, and is the important one:** the mainnet sequencer still holds **no share**. Compromising Contabo yields the coordinator and nothing else (§42.2). Hetzner is not the mainnet sequencer — it runs testnet and the DA mirror | ✅ |
| 55.4 | **`zyn-ceremony` has no resharing path** — it takes `<threshold> <participants>` and runs a fresh DKG. A custodian cannot be *added* to a 2-of-3 group, which is why the choice was move-or-nothing rather than move-or-add | ✅ |
| 55.5 | FileVault is on, so the Mac's deleted remnant is covered by the disk key. Not forensic destruction — that is not achievable on an SSD — but the realistic protection | ✅ |
| 55.6 | Custody is now **Hetzner (01) + Windows (03)**, both always-on-ish, with Contabo reaching Hetzner through a tunnel it opens, pinned by `permitopen` to 8181 and 8210 and nothing else | ✅ |
| 55.7 | **Share 01 now exists in exactly one place.** Losing Hetzner loses it; 2 of 3 would still meet threshold on Air + Windows, but with no redundancy left. An offline backup of each share is worth having before the vault is funded | 🔨 |

## 56. Wallet recovery is a standard seed, with a complete Nap backup (11 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 56.1 | **New wallets generate 24 English BIP-39 words (256 bits) and derive Orchard account 0 at ZIP-32 `m/32'/coin_type'/0'`.** Rejected: turning the old raw Orchard key into words. That would produce pronounceable backup text, not a wallet another ZIP-32 implementation can recover | ✅ |
| 56.2 | A Nap backup is versioned JSON containing the network, birthday, key source, account, optional BIP-39 passphrase, and recovery material. There is no adopted universal Zcash wallet-file interchange format to claim; the mnemonic and derivation are standard, while `nap-wallet-backup` is deliberately named as Nap's portable envelope | ✅ |
| 56.3 | **Legacy 32-byte wallet files and raw-key imports remain byte-compatible.** They export as `raw-orchard` backups and are not silently migrated, because no mnemonic exists that reproduces an independently generated Orchard key | ✅ |
| 56.4 | A mnemonic wallet's transparent BIP-44 receiver derives from the BIP-39 seed and the same account. Its derived Orchard child is not offered as a legacy backup: restoring that child would recover shielded notes but derive a different transparent address | ✅ |
| 56.5 | Generation requires 24 words. Restore accepts a valid shorter BIP-39 phrase for compatibility and emits the ZIP-315 weak-seed warning; refusing it would strand wallets that other software already created | ✅ |
| 56.6 | Restore parses and derives the replacement, checks the backup network, and seeds a staged wallet before removing the current empty wallet. A malformed backup or unavailable tree frontier therefore cannot erase even the current empty key | ✅ |
| 56.7 | **“Standard phrase” does not mean “every historical Zcash pool.”** Nap recovers ZIP-32 account 0 for Orchard and Ironwood plus BIP-44 transparent funds; it does not scan Sapling. The restore UI says this where the phrase is entered, rather than letting a partial balance look like a failed recovery | ✅ |

## 58. Three custodians, and the Air is the reliable one (11 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 58.1 | Share 02 wired from the **M1 Air**: `anchors signed by 3 custodian(s), threshold 2`. Any two now sign, so losing one no longer stops mainnet anchoring — which two-of-two did (§54.3 closed) | ✅ |
| 58.2 | The Air is the **best host of the three**: `pmset` already reports `sleep 0 (sleep prevented by powerd)`, it does not travel, and it runs nothing else. The Mac travels; the Windows box needs a human signed in and has wedged its filesystem twice in a day | ✅ |
| 58.3 | Custodian and tunnel run under **launchd with `KeepAlive`**, not `nohup`. Verified by killing the custodian: port 8210 was back within eight seconds. A process that dies quietly is how a 2-of-3 becomes a 2-of-2 without anyone noticing | ✅ |
| 58.4 | It is a **LaunchAgent, not a LaunchDaemon** — there is no passwordless sudo on that machine — so it starts at login rather than at boot. A reboot with nobody logging in leaves share 02 down, the same trap the Windows box fell into. Worth converting when someone is at the keyboard | 🔨 |
| 58.5 | Each tunnel key is pinned with `permitlisten` to exactly its own port: 8211 Windows, 8212 Hetzner (reached by a tunnel Contabo opens), 8213 Air. A stolen custody key forwards one port and nothing else | ✅ |

## 59. Mainnet gets a verifier (11 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 59.1 | Chain 26460 has a **verifying replica** on the Windows box, chain-scoped and separate from the testnet one it already ran. Until this, mainnet was entirely self-asserted: it had custody and could sign anchors, and nothing checked they were honest | ✅ |
| 59.2 | It reads **that machine's own mainnet Zebra** (at tip, 260 GB), not the sequencer's. A replica pointed at the node it is checking is asking the party under audit (§40.2) | ✅ |
| 59.3 | Viewing key only, `ZYN_LISTEN` on loopback, `ZYN_CONFIRMATIONS=12` to match the sequencer. It verifies and can sign nothing | ✅ |
| 59.4 | Installed as **`zyn-replica-main`, a second binary**, so the healthy testnet replica keeps the one it was running. Swapping a shared binary to fix one service is how an unrelated one breaks | ✅ |
| 59.5 | **The state-version hazard arrived exactly as predicted.** The deployed replica was Sep 10 (`STATE_VERSION` 7, no `ZYNAUTH1`); the sequencer is Sep 11 (v8). It refused the v8 base state with `base state does not match its root: Decode`, because a decoder rejects any version above its own. Replicas and signers are upgraded **before or alongside** a sequencer, never after | ✅ |

## 60. Shares go on paper, one page per place (11 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 60.1 | Each share is rendered **on the machine that holds it** and never sent anywhere. A backup process that collects shares centrally is a quorum in one place, which is worse than the scheme it backs up | ✅ |
| 60.2 | **Paper, not an encrypted file.** 134 bytes is 268 hex characters, which prints and is typeable. A file on a laptop is a second online copy; a page in a safe is genuinely offline, and needs no passphrase that can itself be lost | ✅ |
| 60.3 | Each page carries the hex in numbered groups, the **SHA-256 to check a transcription against**, the exact filename to restore to, and the restore commands. A bare hex string found in two years is not a backup | ✅ |
| 60.4 | **Any two of three reconstruct the vault**, so the pages are stored in three different places — printed on each page, because that is the rule most likely to be forgotten | ✅ |
| 60.5 | No QR: `qrencode` was absent everywhere, and for 268 characters plain text is more durable — no scanner dependency, no print-quality failure | ✅ |

## 61. A replica offers anchors in lineage order, not block order (11 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 61.1 | `in_lineage_order()` sorts sightings by **`(epoch, height)`**, not `(height, epoch)`. A repaired anchor — one re-sent after a reorg dropped the original — necessarily lands at *today's* height while still carrying its *original* epoch. Block order therefore presents epoch 4330 after epoch 4474, the `previous_root` check fails, and the replica halts on a chain that is in fact whole | ✅ |
| 61.2 | The sort decides only what is **offered**. `previous_root` still validates every step, so a wrong order can stall a replica but can never make it accept a bad root. This is why the fix is safe to deploy without re-deriving anything | ✅ |
| 61.3 | Extracted as a **pure function with its own tests**, including one built from the real 4321/4330/4336/4474 inversion. The bug was invisible in integration because it only appears after a reorg *and* a repair | ✅ |
| 61.4 | **This class of halt is a liveness bug, not a safety bug, and it is self-inflicted.** A replica that stops endorsing takes the signer set below threshold and every deposit stops becoming spendable — the same user-visible symptom as an attack, from an ordering mistake | ✅ |
| 61.5 | Deploy order holds (§59.5): **replicas and signers upgrade before or alongside a sequencer, never after** | ✅ |

## 62. A lineage break is not final until the scan is (11 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 62.1 | **§61 was necessary but not sufficient.** Ordering by epoch can only order what has been *seen*. The replica scans Zcash in 200-block windows and applies each window as it lands, so it met epoch 4336 while the block carrying the repaired 4330 was still 1105 blocks ahead of its cursor — and latched a permanent halt on a chain that was whole | ✅ |
| 62.2 | `apply_sightings_scanned(.., scan_complete)` takes the **scan position**, not the error, as the thing that decides. While `to < safe`, a `Halt::Lineage` defers the offending sighting and everything after it to a later pass; once the whole safe range is read, a break that still stands halts as before | ✅ |
| 62.3 | An incomplete view and a broken chain are **indistinguishable from the failing step alone**. `Halt::Fetch` was already treated as retryable for exactly this reason — "the publisher may simply be a few seconds behind". A missing *repair* is the same situation and deserved the same treatment | ✅ |
| 62.4 | Deferring changes **when** the decision is made, never **what** is accepted. `previous_root` is still checked at every step, and a break surviving a complete scan still latches — the second test asserts precisely that | ✅ |
| 62.5 | **The window size was silently load-bearing.** 200 blocks is an RPC batching choice; it had quietly become the width of the evidence the replica was willing to consider before declaring the chain broken. Batching and verification boundaries must not be the same number | ✅ |
| 62.6 | Cost of getting this wrong: a replica that halts stops endorsing, the set drops below threshold, and **every deposit stops becoming spendable** — an outage indistinguishable from an attack, caused by reading an honest chain too early (§61.4) | ✅ |

## 63. ZynZap is hosted without making the Zyn RPC public (12 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 63.1 | **The standalone product is live at `https://zynzap.pages.dev`.** It is the same `apps/zynzap/index.html` that Nap serves at `/zynzap`, not a second landing page or a second implementation of bridge logic | ✅ |
| 63.2 | **Hosted ZynZap talks only through Nap's injected `window.zyn` provider.** The deployed CSP says `connect-src 'none'`; the public page therefore cannot acquire a direct dependency on the private binary node or on `zyn-rpc`. The extension passes allowed requests to the local wallet, and every write still opens the extension-owned approval window (§26–27) | ✅ |
| 63.3 | **The hosted page is mainnet-specific:** it enables the application only when Nap reports Zcash `mainnet` and Zyn chain `26460`. Wrong-network is a stop screen, not a banner over live controls. The same file at Nap's loopback `/zynzap` remains network-flexible for development | ✅ |
| 63.4 | **Bridge is the first surface.** The mainnet chain starts with ZEC ↔ ZEC.zy, while swap and earn need markets and liquidity; presenting an empty AMM before the usable bridge is backwards. Swap and Earn remain present after connection | ✅ |
| 63.5 | **No-extension is a real product state, not a gate.** The complete Bridge / Swap / Earn surface remains navigable in read-only preview, while a compact panel explains the local signer, 12-confirmation path, chain id and 2-of-3 vault. Only private balances, live quotes and actions wait for Nap. It invents no public price or chain data; live public reads can wait for the separately governed public-RPC decision (§53.6) | ✅ |
| 63.6 | The site self-hosts the same audited Bricolage / IBM Plex font files as Nap. Rejected: the old Google Fonts stylesheet, because merely opening the bridge should not disclose interest in a shielded wallet to a font host | ✅ |

## 64. Cave launches are configurable promises, then immutable ones (12 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 64.1 | **Collection launch configuration is a Cave application concern, not a Zyn VM rule.** Cave owns the creator screen, sale policy and allocation receipts; Zyn continues to own collections, items, funding, offers and settlement | ✅ frontend boundary |
| 64.2 | **`0–99` is not a product rule.** A launch chooses one quantity policy: fixed objects per entry, a creator-defined weighted draw, or a buyer-selected range. The engine samples the full positive weight sum with 64-bit rejection sampling; the public policy records outcomes and weights, never an incidental `0–99` implementation | ✅ frontend + engine |
| 64.3 | **Quantity and identity are independent choices.** A fixed one-object receipt may still use a concealed shuffle, while a weighted receipt may issue sequential indices. Cave must not imply that choosing one kind of randomness enables the other | ✅ frontend + engine |
| 64.4 | **A launch policy becomes immutable before the first deposit.** Publish canonical `cave-allocation/v1` bytes, their digest, manifest root, exact deposit-close height, randomness sources and algorithm version. Any change after publication creates a new policy version and requires explicit buyer acknowledgement; an editable database row is not a commitment | ✅ canonical bytes/digest; 🔒 publication service |
| 64.5 | **The frontend simulation is educational and never the draw.** It may sample 10,000 entries with browser cryptographic randomness to show likely frequencies, but it is labelled as a preview and its bytes, results and timing never enter allocation or the published commitment | ✅ |
| 64.6 | **The creator frontend lands before the backend and is honest about that boundary.** `apps/cave-market/#launch` builds a local draft and exposes no wallet, deposit or publish call. “Connect backend to publish” remains disabled until the policy service can return a signed, publicly timestamped commitment | ✅ |
| 64.7 | **Capacity is derived from the policy, not hoped for.** Fixed tickets reserve their fixed quantity, weighted tickets reserve the maximum possible receipt until quantity resolves, and buyer-selected tickets reserve the selected quantity. The allocator rejects an exact batch that exceeds supply; Cave's durable ledger now refuses a deposit before its reservation could exceed supply and reproduces that total on restart | ✅ allocator + reservation ledger |
| 64.8 | **Concealed shuffle remains the recommended identity policy.** It shuffles a manifest committed before deposits using predeclared future public entropy and a reproducible algorithm. Sequential issue is allowed and explicitly marked as having no identity randomness; a server-only random sort is not an offered policy | ✅ frontend + engine |
| 64.9 | **Public entropy is verified, not merely fetched.** `cave-randomness` pins the drand Quicknet chain hash and RFC9380 public key, verifies every distinct BLS beacon in-process, derives `SHA-256(signature)`, and fails closed on missing, early, wrong-round, wrong-chain or invalid evidence. The network client is outside the pure engine and may never fall back to local RNG | ✅ engine; 🔒 fetch/persistence service |
| 64.10 | **Every derived stream has a separate purpose.** HKDF-SHA256 uses the policy digest as salt and distinct domains for weighted quantity, concealed identity and instant identity. ChaCha20 drives unbiased sampling and Fisher–Yates; changing any policy, ticket, manifest, ledger or reveal input changes the applicable stream | ✅ |
| 64.11 | **The concealed reveal combines two independently sourced public facts.** Its seed binds the predeclared drand round and predeclared Zcash block height/hash to the manifest root and final canonical ticket-ledger root. The reveal block must be strictly after the committed deposit-close height and post-close tickets are rejected. Cave must independently prove the supplied Zcash hash is canonical at that height; the pure engine cannot learn chain history itself | ✅ engine; 🔒 Zcash evidence service |
| 64.12 | **Allocation is independently replayable.** Tickets sort by confirmed Zcash height, transaction index, output index and derived ticket id. A ticket id is fixed by the policy digest and every other ticket field: exact Zcash outpoint and amount, Zyn recipient, confirmed position, future drand targets and buyer choice; changing any of them invalidates it. Receipts include manifest inclusion proofs and bind the policy, manifest, ledger prefix/full root and assigned indices; receipts and batches have versioned canonical bytes and pinned digest vectors; `verify_batch` recomputes the entire result. The reference test assigns all 4,444 indices exactly once | ✅ |
| 64.13 | **Claim order cannot change NFT identity.** Every manifest item id must equal `item_address(ADDRESS_SCOPE_V1, collection, global_index)`. `MintCollectionItem` now carries that serial and derives the asset from it, rather than from the number already minted; duplicate and out-of-cap serials fail. A cross-crate test mints two allocation receipts in reverse order and proves the exact item ids still reach their bound recipients. Historical sequential mint wire tag 31 remains decodable and byte-identical; explicit-serial mint uses new tag 41, so this fix does not invalidate an old journal | ✅ engine + VM + client/API |
| 64.14 | **Sale state is fail-closed and replayed after restart.** `zyn-io/cave:cave-sale` locks the exact policy/manifest digest plus a nonzero external publication reference before accepting deposits. It accepts each externally verified Zcash outpoint once, requires it to follow publication and precede close, persists atomically, recomputes reservations on load, closes only after a verified tip passes the committed height, stores the evidence and full allocation batch, re-runs `verify_batch` on load, and produces idempotent receipt-bound mint plans. It deliberately does not label an unverified RPC response as canonical or hold Treasury 26460 authority | ✅ durable ledger; 🔒 chain/publication/signer adapters |
| 64.15 | **Cave ZEC payments occupy their own strict memo namespace and cannot also credit the buyer.** `ZYC/v1` binds purpose (`entry` or `claim`), a policy digest/ticket id and the Zyn recipient. The shared vault scanner recognizes valid Cave payments separately and fails closed on malformed `ZYC` before ordinary address attribution, including at an address issued to that buyer. The canonical Cave scanner view adds block height, transaction index and cross-pool output index. For an accepted entry, `cave-sale` emits one operator `CreditDeposit` of the exact payment to Treasury 26460, waits for that credit's epoch to be anchor-final, then emits one Treasury-signed `FundCollection` for 90%; the 10% remainder stays in Treasury. It verifies the actual VM receipts before recording either step and emits no item mint for that ticket until funding is recorded | ✅ memo/scanner/entry settlement ledger; 🔒 continuous watcher + Treasury signer; ⏳ claim settlement |
| 64.16 | **The complete primary-sale promise must have one canonical commitment before production.** Publish versioned `cave-sale-terms/v1` bytes that bind the allocation-policy digest and manifest root, collection id, Treasury 26460's real AccountId, Zcash payment asset/network, entry price, claim price, open-mint price, minimum confirmation rule, deposit/claim close heights, and the `90% floor / 10% Treasury` routing for every primary-sale payment. The sale-terms digest replaces the bare allocation-policy digest as the `ZYC Entry` reference; each ticket id binds that same digest, so a `ZYC Claim` reference to the ticket is transitive. No price, recipient, split or close rule may live only in a web form or service environment variable. This is a required format/engine migration: today's entry rail still references `cave-allocation/v1`, while claim admission and settlement do not yet exist | 🔒 decision; ⏳ canonical bytes + ticket/entry migration + claim state machine |
| 64.17 | **Collection Z's frozen primary-sale numbers are `0.01 / 0.005 / 0.0133 ZEC` for entry / claim / open mint, with 90% of each payment funding the collection and 10% retained by Treasury 26460.** The entry rail enforces the first split today. Claim and open-mint services must enforce the same split and wait for anchor-final Treasury credit before funding or minting. A claim memo names exactly one ticket and its bound recipient; it cannot redirect the allocated serials. Deposit and claim close heights are explicit published chain heights, not an unpublished wall-clock or a default 30-day timer | 🔒 economics; ✅ entry settlement; ⏳ claim/open-mint settlement |

## 65. NFT custody actions belong to the item, not a generic asset form (12 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 65.1 | **Nap's Items tab is the custody surface.** It lists only item assets the signed Zyn account record says this key holds, then reviews symbol, content commitment, asset id, collection, phase, supply, pool backing and redeem value. Trading remains Cave/ZynZap work; custody must not grow a second marketplace | ✅ |
| 65.2 | **Receiving names the Zyn account directly.** Nap shows the raw 32-byte account id as 64 hex characters and as a QR encoding those exact characters. Rejected: a new `zyn:` URI before any parser or registered scheme exists, because a scan that no current sender can consume is decoration. The account id is shareable; the key and private holdings are not | ✅ |
| 65.3 | **An NFT send has no amount field.** “Send this NFT” submits exactly one whole unit of the selected asset and validates a 64-hex destination before review. The VM's `TokenInfo::admits` remains the authority and rejects fractional amounts; the UI removes an avoidable way to ask for an invalid transfer | ✅ |
| 65.4 | **Review names the consequence, not just the mechanism.** Before submission Nap shows the item symbol and asset id, full recipient account, one-whole-item amount, settlement path, and that the item leaves this wallet. The ordinary private Zyn path is the default; the Zcash forced-inclusion path is an explicit recovery option | ✅ |
| 65.5 | **A content hash is not hosted artwork.** Until a resolver can fetch bytes and verify them against the committed hash without leaking wallet activity, Nap renders a deterministic identicon and exposes the full commitment for checking. Rejected: loading arbitrary remote image URLs from NFT metadata in the wallet | ✅ |
| 65.6 | The daemon creates the account QR in the existing overview response and the shared canonical page consumes it. No QR library, image host or font host is added to the browser; Tauri receives the same source through `apps/nap/build-ui.mjs` | ✅ |

## 66. The address standard, and the bug that made it a no-op (12 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 66.1 | **`derive_for` never hashed the VM version.** It wrote the version at bytes `[32..34]` of a 34-byte buffer and then hashed `[..name.len()+2]` — so for any name shorter than 32 bytes the version sat outside the preimage. `zynzap` v1 and v2 derived **identical** addresses, the exact opposite of what §4 promises. Fixed by building the scope as `name ‖ version`, with a test that two versions disagree | ✅ |
| 66.2 | The bug survived because every existing test asked whether a derivation was *deterministic*. A fault that collapses two namespaces into one passes all of them — determinism is preserved, it is distinctness that is lost. Testing "different inputs differ" is not the same as testing "the same inputs agree" | ✅ |
| 66.3 | Split out `program_scope_of(name, version)`, non-generic, so the rule can be tested without standing up a whole VM. The original was untestable in isolation, which is why it was untested | ✅ |
| 66.4 | **Bridged assets name the origin network** (§4.6). A testnet bridge minting something addressed as mainnet ZEC would be claiming backing it does not hold — so `ZEC.zy` and `TAZ.zy` must differ, and the same rule extends to Solana devnet and Bitcoin signet | ✅ |
| 66.5 | **A chain's own token is chain-scoped** (§4.7). Unscoped, mainnet ZYN and testnet ZYN are one address and a holder reading an address alone cannot tell which they hold. Bridged assets need no such seed because their origin network already separates them | ✅ |
| 66.6 | **Item ids derive from `(collection, serial)`**, so every item's identity is computable before the collection mints — what a reveal schedule or an allowlist needs — and two nodes cannot disagree about which item is which. `item_leaf` already binds the collection, so an item proved against one cannot be replayed against another | ✅ |
| 66.7 | A symbol is unique **per creator**, not globally: two creators may both issue `CAT`, one creator may not issue it twice | ✅ |
| 66.8 | **Core adoption is complete; live rollout is not.** `AssetId`, `PoolId` and `CollectionId` are 32-byte derived identifiers through state, wire, Merkle, receipts, RPC, clients, bridge paths and browser wallet code. Because adopting them changes committed encoding, deployed replicas/signers must be upgraded verifier-first before the sequencer emits the new state | ✅ core; ⏳ coordinated deployment |
| 66.9 | **The derived 32-byte address is token identity; name, display fields and content are not.** A Cave launch derives its asset as `derive(P, "asset", [creator, canonical_symbol])`, its market from the ordered `(asset, ZEC.zy)` pair, and its LP/vault/lock identities from that pool. State, receipts and APIs carry those addresses and never allocate parallel numeric ids | ✅ core adoption |
| 66.10 | **Persistent identities freeze `ADDRESS_SCOPE_V1 = b"zynzap\x00\x02"`.** Execution `VM_VERSION` and `STATE_VERSION` may advance without moving assets. A new address scope requires an explicit incompatible address-schema fork. Persistent callers use the frozen scope and `program_scope_of` rejects names above 32 bytes rather than truncating them | ✅ scope implementation |
| 66.11 | **User symbols are exact Unicode identity bytes:** 1–32 bytes of valid UTF-8, allowing CJK, emoji, mixed case and punctuation. Reject Unicode whitespace, NUL/C0/C1 controls, line separators and bidi override/isolate characters. Consensus does not normalize, case-fold, trim or truncate: visually identical byte sequences may be different assets, so clients show a short address wherever identity matters. The creator seed remains the owner/fee-recipient account, never a session key or relayer | ✅ VM encoding; ⏳ complete client presentation |
| 66.12 | **One creator can never reuse the same exact symbol bytes; different creators can, including official tickers such as `ZYN`.** The second identical `(creator, symbol_bytes)` creation is rejected and the original address is never recycled. Name, content, fee, supply and creation time do not enter identity | ✅ VM collision rule |
| 66.13 | **Origin seeds are registered bytes, never displayed addresses.** Freeze `zcash`, `zcash-test`, `solana`, `solana-devnet`, `bitcoin`, `bitcoin-test`, `bitcoin-signet`, plus EVM `eip155:<canonical decimal chain-id>`. Native origin assets use empty bytes; Solana mints use decoded 32 bytes and EVM contracts raw 20 bytes. An unregistered family cannot bridge an asset | ✅ registry enforcement/vectors |
| 66.14 | **A pool address names the pool record; `vault(pool)` is its real keyless reserve account.** Pool reserves move out of inline state balances during A25. Cave additionally uses separate derived accounts for curve inventory, graduation inventory, refundable reserve, pair seed, graduation fees, creator escrow, graduation LP lock and the global ZEC.zy future-ZYN-POL pot; balances with different movement rules never share an account. The POL address is now pinned as `f40435d1406abc3aa4befaf380acea34a4a87ed76a51538f88d09ba0d2937e61`; the remaining per-launch/pool physical split is still open | 🔒 custody; ✅ derived POL; ⏳ remaining A25/launch accounts |
| 66.15 | **Public addresses are typed, network-aware lowercase Bech32m.** Mainnet HRPs are `zyna/zynp/zync/zyni/zynv`; testnet prefixes them with `t`. Consensus retains raw 32 bytes. Lowercase hex remains a CLI/debug representation, not the primary wallet or URL format | 🔒 presentation; ⏳ clients |
| 66.16 | **Official status is pinned by address, never claimed through a ticker or metadata.** Native, bridged and user assets already have separate namespaces, so a user-launched `ZYN` cannot occupy native ZYN's address. Cave and wallets badge only exact addresses from the protocol's shipped official-address set and show the short address beside duplicate or confusable labels. No symbol is reserved from permissionless launches | ✅ protocol address set/namespaces; ⏳ client badges/short-address UI |
| 66.17 | **The 333 Nap Mole and 4,444 Cave collections share the protocol treasury as creator, named `Treasury 26460`.** Its spendable threshold `AccountId` is the creator seed and receives their creator-fee share. The label and role are fixed; provision and pin the real 32-byte account before recomputing either official collection's addresses. The earlier `0xca7e…ca7e` vectors are placeholders and must not be minted | 🔒 identity; ⏳ treasury AccountId/vectors |

## 67. Where the bytes live, and how a wallet knows they are the right ones (12 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 67.1 | **`content = H(manifest)`**, `zyn.manifest.v1`, defined in `zyn_vm::manifest`. The manifest names each file by SHA-256, mime, length and dimensions — and carries **no URL**. A URL inside a hashed manifest is worse than ERC-721's `tokenURI`, not better: it makes the location *immutable*, so the collection dies with the domain | ✅ |
| 67.2 | Location is a **hint held outside consensus**, per collection, replaceable at any time. A gateway serves objects keyed by their own hash — `GET {gateway}/{sha256}` — which makes it a dumb blob store: identical bytes deduplicate, anyone may mirror it, and changing provider touches no token | ✅ |
| 67.3 | **Size never enters the chain.** 32 bytes are committed whether the media is a 40 KB PNG or a 4 GB film, so GIF and video need no protocol change — only a bigger CDN bill | ✅ |
| 67.4 | **The manifest is itself a blob, addressed by the content hash.** From 32 bytes on chain a wallet finds and checks everything else with no registry and nothing to trust | ✅ |
| 67.5 | **Verify before render, always.** `verify_media` in the VM and the same check in the wallet: bytes are compared to the commitment before they are decoded. That is what makes it safe to accept a file from a gateway, a peer, or a stranger's USB stick — the source is never trusted, only the hash | ✅ |
| 67.6 | **Fetching is off until a gateway is configured** (`nap_media_sources`). Asking a server for the art of the item you own tells it your IP, which item you hold, and when — the disclosure that made us bundle the fonts (C23). Anything already in the local store renders with no network at all | ✅ |
| 67.7 | **Handing someone the file with the item is therefore the private path**, and needs no protocol support: bytes arrive once, by hand, are checked against the commitment, and are never requested again. Media does **not** travel inside an intent — that would put it in DA, on every mirror and every replica, forever | ✅ |
| 67.8 | **A watermark is a courtesy, not a control.** A manifest may commit a public `Preview` beside the `Full` media, but the full bytes cannot be kept secret: the owner has them and may republish, and the first owner always can. Ownership is the chain record, never possession of bytes — the same position the content hash already takes, since it is a commitment and not a secret | ✅ |
| 67.9 | Three implementations of the encoding now exist — Rust (normative), the collection builder in Python, the wallet in JavaScript. All three are pinned to the same vector: Nap Mole #001 is `14ecc291…`. A cross-check test fails if any one drifts | ✅ |

## 68. BOLD.zy and the fixed-supply phase transition (13 Sep 2026)

This supersedes §30's continuing mint-and-burn monetary policy. ZEC remains the
launch reference in Phase 1, but it is not intended to remain ZYN's only
numeraire. The transition away from it is stated before the first market can
trade, rather than invented after holders have priced a permanent ZEC
dependency into the token.

**Precedence rule.** Any earlier passage in this file that describes a ZYN
bridge-fee genesis, emissions, halvings, vesting, fee burn, mint-funded reward,
unminted reserve, gas utility or freshly minted cashback is historical and
superseded by this section. Earlier descriptions of bridge minting and burning
for `.zy` assets are unaffected: those are one-for-one custody accounting, not
ZYN monetary policy.

| # | Decision | Status |
|---|---|---|
| 68.1 | **BOLD.zy is Zyn's first bridged USD stable asset.** Its origin is Ethereum mainnet (`eip155:1`) and the canonical Liquity V2 BOLD token `0x6440f144b7e50D6a8439336510312d2F54beB01D`. Under §66.13 its raw address is `ddf2b7ec…627f250f`. Zyn bridges that ERC-20 directly through its EVM vault; it does not accept a Solana or third-party wrapped copy as the same asset | 🔒 design/address; ✅ derivation vector; ⏳ production bridge |
| 68.2 | **BOLD.zy is the stable reference for Phase 2, not an assertion that BOLD is always worth one dollar.** The feed follows the traded BOLD/USD value and stops being usable when stale; a BOLD depeg must appear in Zyn's reference rather than being hidden behind a hard-coded `$1` | 🔒 |
| 68.3 | BOLD was chosen for the failure mode that rules out USDC: Liquity's core is immutable, the deployed token has renounced ownership, and its transfer guard has no mutable holder blacklist. The remaining risks are Ethereum, collateral, oracle, liquidation and market-liquidity risks — not an issuer freezing Zyn's vault. Canonical references: `https://docs.liquity.org/v2-faq/bold-and-earn` and `https://github.com/liquity/bold/blob/main/contracts/src/BoldToken.sol` | 🔒 |
| 68.4 | **ZYN has a fixed total supply, but its issuance and allocation do not block Cave.** Settled in §71: 21,000,000 minted once, 18,900,000 into keyless incentive pots. When issued, it is created once into auditable distribution and protocol-liquidity pots; ordinary operation neither mints nor burns ZYN. Until then, Cave routes the ZEC.zy share intended for future ZYN liquidity into a keyless pot without minting, promising or distributing ZYN. §30's per-Zcash-block mint, halvings and fee burn are withdrawn. The decision is complete; replacing the old launch code and deploying the new genesis remains open | 🔒 design complete; ⏳ implementation/deployment |
| 68.5 | **No gas token and no per-intent gas fee — in Phase 1.** Cave launch, transfer, swap, curve sell, graduation and exit do not require ZYN or wait for ZYN issuance. Amended 13 Sep (§71.14): this is a Phase-1 rule, not a permanent one. The original clause — that ZYN's demand must come from its own market rather than from making users acquire it before they can use Zyn — holds only while ZYN has no market. Phase-2 gas is open | 🔒 Phase 1; §71.14 |
| 68.6 | **Phase 1 accrues ZEC.zy for future ZYN-owned liquidity without depending on ZYN issuance.** Standard pools split every swap fee actually collected in ZEC.zy equally: half remains with the source pool and half enters the derived future-ZYN-POL pot. A graduated Cave pool instead uses §69's disclosed `45% pool / 45% future ZYN POL / 10% creator` split. Matching or deploying that ZEC.zy waits for the separate ZYN issuance decision; the launchpad may not spend it elsewhere meanwhile. These are splits of the existing fee, not additional trader charges, and fees collected in other assets are not silently converted | 🔒 accrual; ⏳ VM, later deployment |
| 68.7 | **The Phase-2 price signal is `1 ZYN = 0.0002 ZEC`.** Reaching it means the ZEC-supported bootstrap has done its job and the system may move to the BOLD-referenced, market-priced phase | 🔒 threshold |
| 68.8 | ~~A single pool print cannot activate Phase 2.~~ **Retired by §71.11** — with nothing distributed before Phase 2 there is no price to observe, so the trigger is the POL pot reaching 420 ZEC.zy, and an accrual counter needs no observation rule, averaging window or liquidity floor. Original text: a single pool print cannot activate Phase 2. The threshold must be sustained under a manipulation-resistant observation rule with a minimum liquidity/volume floor. The exact averaging window and floor remain to be fixed before Phase 1 launches; until then `0.0002` is a graduation signal, not an automatic state transition | ⏳ parameter |
| 68.9 | **Phase 2 is an announced transition to market pricing, not a redemption event.** ZYN gains a protocol-owned ZYN:BOLD.zy market, the special Phase-1 ZEC support rule winds down under precommitted rules, and neither ZEC nor BOLD is promised as redeemable backing for ZYN. Exact liquidity migration and wind-down steps remain to be specified | 🔒 boundary; ⏳ mechanism |

## 69. Cave is the Phase-1 launchpad; curves graduate into ZynZap (13 Sep 2026)

Phase 1 needs repeated ZEC.zy-denominated use more than it needs a catalogue of
bridges. This changes the ordering in §6c and §14b without discarding their
engineering: Solana and EVM bridge code stays tested, but neither production
bridge is on the Phase-1 critical path. The ZEC bridge, Cave and ZynZap form the
first complete economy.

| # | Decision | Status |
|---|---|---|
| 69.1 | **Phase 1 ships one external value boundary: ZEC ↔ ZEC.zy.** Cave NFT secondary trading, Cave native-token launches and their graduated ZynZap markets are the activity loop. Production SOL.zy is deferred until user demand justifies it. The EVM bridge and BOLD.zy are deferred from Phase 1, but remain prerequisites that must be ready before the Phase-2 transition in §68 | 🔒 priority; ⏳ delivery |
| 69.2 | **Cave owns the product; ZynZap's VM owns the conserved state.** Launch creation, discovery, policy presentation and the curve UI are Cave. Native token balances, curve inventory, ZEC.zy reserve, fee pots and the final AMM pool live in the same `swapvm` state from the token's first transaction. “Migration into ZynZap” is therefore a product-state change, not a cross-VM asset migration | 🔒 architecture |
| 69.3 | This supersedes §6c's assumption that an on-Zyn launchpad must first be a separate application VM and therefore wait for cross-VM atomic bundles. A future permissionless launchpad VM may still use that boundary; Cave v1 does not put user balances on the far side of it | 🔒 |
| 69.4 | **The lifecycle is `Draft → Curve → AMM`; launches do not expire.** Draft is off-chain and moves no value. Curve and AMM are VM state. An inactive launch may be hidden from the default gallery, but its curve continues to accept ordinary buys and sells. Graduation is attempted on scratch state and commits the pool and `AMM` phase together; there is no externally visible half-graduated phase and no trade is accepted against two pricing mechanisms at once | ✅ VM |
| 69.5 | **Every Cave v1 fungible launch uses one linear protocol curve.** With fixed supply `S = 1,000,000,000`, starting price is `p0 = 0.009 / S = 0.000000000009 ZEC.zy` per token. Graduation price is `Pg = 100 / S = 0.0000001 ZEC.zy` per token, meaning fully diluted market cap `p(q) × S` reaches `100 ZEC.zy`. The curve sells `Qg = 660,000,000` tokens, so `s = (Pg − p0) / Qg ≈ 1.5150151515151515×10⁻¹⁶ ZEC.zy per token²` and `R(Qg) = 33.00297 ZEC.zy` before fees. There is no age or expiry gate. Consensus evaluates the exact interval integral with division last: buy principal rounds up and sell principal rounds down to the ZEC.zy WAD, so an atomic early buy is never free. Exact rational/fixed-point constants and boundary vectors, not these rounded display decimals, are consensus | ✅ vectors/VM |
| 69.6 | **Every Cave v1 token has exactly 1,000,000,000 display units, eight decimal places and no later mint authority.** Of the full `100,000,000,000,000,000` atomic units, `660,000,000` display tokens enter the keyless curve inventory and `340,000,000` enter the keyless graduation inventory. There is no free creator allocation: a developer can own tokens only through the paid dev buy in §69.26 or an ordinary trade. Graduation uses only the token amount required for exact price continuity; any unused graduation inventory becomes permanently unreachable. Symbols are unrestricted labels under §66.11–12; official status comes only from §66.16's address set | ✅ VM |
| 69.7 | **Purchase principal is not revenue.** ZEC.zy paid along the curve's integral goes into a refundable curve reserve and is the only value quoted back to pre-graduation sellers. It is logically accounted separately from every fee pot; Cave, the creator and the operator cannot withdraw it. Today the reserve and per-launch fee balances reconcile inside one derived launch vault plus distinct committed fields; A31 moves them to separate derived custody accounts | ✅ accounting/access invariant; ⏳ A31 physical split |
| 69.8 | **Every curve trade charges its fee on the ZEC.zy leg.** A buy charges ZEC.zy input; a sell charges ZEC.zy output. Of fee `F`, `50%` goes to the future-ZYN-POL ZEC.zy pot, `40%` goes to that launch's graduation-liquidity balance, and `10%` accrues to the creator. The creator share is escrowed until graduation so an abandoned launch cannot be farmed for immediate creator revenue. Routing and committed accounting are built; the graduation and creator balances remain fields backed by the shared launch vault until A31 gives each its final derived account | ✅ VM routing/accounting; ⏳ A31 physical split |
| 69.9 | **Cave NFT secondary fees use the same `50/40/10` vocabulary but a different middle destination:** `50%` to the keyless future-ZYN-POL ZEC.zy pot, `40%` to the traded collection's redemption floor, `10%` to its creator. Both the 333 Nap Mole and 4,444 Cave collections use `Treasury 26460` as creator and fee recipient (§66.17). This supersedes §39.7's implemented `50% floor / 50% creator` split. The NFT storefront now discloses the ZEC.zy-only fee, all three destinations and the POL-favouring atomic remainder | ✅ VM routing/receipt/storefront |
| 69.10 | **Graduation occurs when the curve marginal price reaches `0.0000001 ZEC.zy`, so the fixed one-billion supply has a `100 ZEC.zy` fully diluted market cap.** Under §69.5 this occurs at `q = 660,000,000`; no minimum age, wallet count, NFT ownership or separate volume test applies. Gross trading fees do not advance `q` or the price. An optional dev buy can help reach the same public threshold only by paying the same curve price and fee | 🔒 trigger |
| 69.11 | **Graduation is one atomic VM transition.** It stops the curve, combines its refundable reserve, 40% graduation pot and `0.009 ZEC.zy` pair seed into candidate ZEC, applies §69.28's cap and overflow rule, contributes the exact required graduation inventory, creates the canonical token:ZEC.zy AMM at `Pg`, and makes every LP share permanently locked. The current pool records `locked == lp_supply` and credits no account; A31 later gives that locked amount its final derived account. If price continuity, inventory sufficiency or any debit fails, scratch state is discarded and nothing changes | ✅ atomic VM/invariant; ⏳ derived lock storage |
| 69.12 | At graduation, pool price continuity is exact: for pool ZEC `Zp` and `Pg = 0.0000001 ZEC.zy` per token, token deposit is `T = Zp / Pg` rounded down to the token's eight-decimal unit; ZEC is recomputed as `T × Pg` and residual ZEC goes to future ZYN POL. `T` may not exceed the `340,000,000`-token graduation inventory. Any token remainder stays permanently keyless; it is not a creator, treasury or later mint reserve | ✅ math/vectors |
| 69.13 | **There is no failed-launch or refund-only phase.** A launch remains on its reversible curve until it reaches graduation, however long that takes. Sellers always receive the curve principal quoted by `R(q) − R(q−Δ)` less the disclosed ZEC.zy-leg fee. The pair seed, graduation pot and creator escrow remain keyless and unavailable until graduation; inactivity never makes them withdrawable by Cave, treasury or creator | 🔒 persistence |
| 69.14 | **Graduated ZynZap trading continues the ZEC-denominated fee rail and the creator share.** On ZEC.zy → token, the fee comes from ZEC input; on token → ZEC.zy, it comes from gross ZEC output. Of fee `F`, `45%` remains in the source pool, `45%` enters the future-ZYN-POL ZEC.zy pot and `10%` goes to the token creator. Fees are not accumulated in arbitrary launched tokens and are not silently converted. Exact-in, exact-out and sealed-batch paths implement the same rule and reconcile every share in receipts | ✅ VM pricing/routing/receipts/tests |
| 69.15 | More launches are not themselves success. Phase-1 health is repeat traders, net ZEC.zy reserve, graduated pools, retained liquidity, real ZEC.zy volume and fees routed under the committed split. Per-trade ZYN rewards are rejected: they would turn the fixed distribution reserve into finite fee mining and make wash activity look like adoption | 🔒 metrics |
| 69.16 | **Token creation is permissionless from the first public alpha: every valid Zyn wallet may launch.** Each creator account may create at most two tokens in any rolling 60 chain epochs—the deterministic consensus form of “two per hour” under the 60-second seal target. The `0.01 ZEC.zy` fee is the Sybil cost. There is no global launch quota, active-curve cap, global reserve cap or single-buy cap; buys remain balance-limited and sells/exits are never throttled. Nap Mole ownership is not a whitelist, graduation condition, fee discount or priority rule | ✅ VM access/rate boundary |
| 69.17 | **Reuse ZynZap's pool engine, not its existing `CreateToken` launch policy.** Constant-product math, canonical pair identity, LP-share calculation, quote limits, epoch commitment and replica replay are the implementation base. Cave graduation now constructs its canonical pool from those primitives. The legacy `CreateToken` path is not Cave: it opens the AMM immediately, accepts creator-selected liquidity/fee inputs and returns removable LP shares. It remains decodable/executable for historical replay and controlled operator test setup, but has no wallet authority or provider/API surface; ordinary and delegated signatures cannot authorize it. Cave `LaunchCurve` is the only user-authorized fungible creation path | ✅ Cave reuse and legacy capability gate; ⏳ compatibility-code removal with state migration |
| 69.18 | **A pre-graduation Cave asset may have no ZEC.zy pool only while a live launch record proves phase `Curve`.** Graduation atomically sets `genesis_pool` and phase `AMM`; every later pool and route uses that canonical id. This requires a committed state-invariant and codec change, not a Cave-only database flag. Complete the 32-byte identifier migration in A25 before adding launch records so the launchpad does not create a second avoidable state fork | ✅ invariant/codec; ⏳ deployment migration |
| 69.19 | **Graduated pools use an explicit per-pool ZEC-leg fee policy.** The existing global input-asset protocol cut is insufficient. `ZecLegSplit` charges the token's immutable fee once: on a buy, price against ZEC input after `F`; on a sell, price gross ZEC output and pay the trader `gross − F`. In both directions `45%` of `F` remains in the pool, `45%` enters future ZYN POL and `10%` goes to the creator. Creator and pool shares round down and the atomic remainder goes to future POL. Standard pools retain their input-fee policy. Quotes, exact-in, exact-out and batch settlement now share this accounting; hop receipts name the fee asset, total and every destination share | ✅ VM/receipt/test contract; A45 resolved |
| 69.20 | **Graduated liquidity is permanent protocol liquidity.** Every LP share produced at graduation is recorded in the pool's consensus `locked` amount, equal to total LP supply, and is ineligible for `RemoveLiquidity`; none is credited to the creator, operator or a general treasury key. The final address model represents that amount in a derived keyless lock account when A25/A31 move pool custody out of inline reserve/lock fields. The graduated native pool carries no external reference price or divergence fee: its ZEC.zy price is discovered on Zyn | ✅ withdrawal lock/invariant; ⏳ derived-account storage |
| 69.21 | **Launch and pool prices use one public orientation: ZEC.zy per token.** Curve terminal price `P`, launch quotes, graduation receipts, ZynZap reads and chart candles name that orientation explicitly. Canonical reserve ordering remains internal; a generic `reserve1 / reserve0` or current spot helper must not silently invert a graduated market. Public surfaces report `zec_per_token` or `zecPerToken`; the legacy `marginalPrice` JSON alias is retained only for compatibility. The value is curve marginal price before graduation and correctly oriented live pool spot afterward | ✅ API invariant/read path |
| 69.22 | **The curve and the AMM do not share a pricing engine.** Cave v1 curve trades are exact-token/max-or-min-ZEC, have no partial fills, and execute sequentially in committed intent order against the versioned integral; their receipts become final with the containing epoch. Quotes are provisional until execution. After atomic graduation, the canonical pool uses ZynZap's existing exact-input swap and batch-clearing path. Cave Jade labels the venue and changes the ticket semantics at graduation | ✅ VM/UI disclosure |
| 69.23 | **Charts are derived, not consensus state.** Accepted curve receipts and graduated-pool swap receipts are the source events for an OHLCV index keyed by launch and canonical pool id. Cave may cache and serve that index, but reserve, phase, fee-pot and locked-LP claims must still come from replica-verifiable state; a missing chart may never block settlement | 🔒 boundary; ⏳ indexer |
| 69.24 | **Launchpad consensus does not inspect NFT ownership or maintain a public holder map.** A valid wallet, sufficient `0.01 ZEC.zy` launch fee and valid committed launch fields are the authorization for creation. Nap Mole's credit multiplier is evaluated by the separate credit system and cannot gate or alter launch, buy, curve sell or graduation | 🔒 privacy/access boundary |
| 69.25 | **Creation costs exactly `0.01 ZEC.zy` (`1,000,000` zatoshi-equivalent units), separate from any dev buy.** `0.001 ZEC.zy` (`100,000` units) credits the protocol treasury immediately; `0.009 ZEC.zy` (`900,000` units) enters the launch's locked pair-seed balance and joins the ZEC side only at graduation. It is opening liquidity, not refundable curve principal and not a guaranteed price floor: AMM trading can move the price below its opening level. With no expiry, the seed remains committed to that launch until graduation. The current VM holds it in the derived launch vault and commits its amount by rule; A31 gives the seed its own final derived account | ✅ VM fee/routing; ⏳ A31 physical split |
| 69.26 | **The developer may make one optional atomic dev buy inside `LaunchCurve`.** It requests an exact token quantity with an explicit maximum ZEC.zy debit, executes after the launch fee against the same initial curve, rounding and chosen trading fee as any other buyer, and receives no discount or free allocation. It is visible in the creation receipts and may acquire at most `5%` of total supply (`50,000,000` tokens). Later creator purchases are ordinary trades | ✅ VM/client API |
| 69.27 | **Curve-fee rounding is deterministic.** For total ZEC.zy fee `F`, creator escrow is `floor(F × 10 / 100)`, the graduation pot is `floor(F × 40 / 100)`, and the future-ZYN-POL pot receives the remainder. Thus the economic split is `50/40/10`, while every indivisible unit is conserved and any rounding dust favors future protocol-owned liquidity, not the creator | ✅ rounding/vectors |
| 69.28 | **Graduation ZEC.zy is capped by the fixed token-side inventory, and overflow goes to future ZYN POL.** Candidate `Zc = refundable reserve + graduation pot + pair seed`; capacity `Zmax = Pg × 340,000,000 = 34 ZEC.zy`. Deposit `Zp = min(Zc, Zmax)` and `T = Zp/Pg` into the token pair, then route `Zc − Zp` to the future-ZYN-POL pot in the same atomic graduation. Current v4 Cave, NFT and graduated-pool credits use pinned `derive(P, "zyn-pol", [ZEC.zy]) = f40435d1…7e61`; legacy `[0xF3;32]` remains only in historical §30 compatibility state and must be empty or migrated during rollout. The pot can accumulate before ZYN exists and grants no present ZYN claim. Unused token-side graduation inventory becomes unreachable | ✅ VM cap/routing/derived destination; ⏳ live legacy-balance audit |
| 69.29 | **The unresolved “metadata” item was token addressing, not a mutable registry.** Cave launches use §66's derived addresses and accept §66.11 Unicode symbols. Display name is required valid UTF-8 of 1–96 bytes; it may contain ordinary internal whitespace but no leading/trailing whitespace, NUL/C0/C1 controls, line separators or bidi override/isolate characters. `content` is the required 32-byte manifest hash. Symbol and display-name duplication—including official names—is allowed across distinct derived addresses; a creator cannot overwrite its own existing `(creator, symbol)` address. Only an exact address in §66.16's pinned set receives the official badge. Gallery reports may warn or de-index but never alter settlement, supply, curve trading, graduation or exits | ✅ VM/client/UI identity |
| 69.30 | **The fungible launchpad is a separate Cave product surface, not another tab in the pixel-art NFT application.** Its target is the `launch.cavebro.win` subdomain and an independent Cloudflare Pages artifact. The root is the coin gallery, the second primary route is `/launch`, and each asset has `/coin/<address>` for its chart/trade page; there is no marketing landing page in front of the gallery. `build:launchpad` emits only Jade plus its provider-only CSP, and the earlier hidden `Cave Signal` HTML/JS/CSS has been removed from the NFT app. The artifact is live at `cave-jade.pages.dev`; the custom hostname is registered and its DNS activation remains an operator action | ✅ source/build/Pages deployment; ⏳ custom-domain DNS |
| 69.31 | **Working brand direction is Cave Jade.** It uses a dark-jade primary, restrained metallic/mineral texture and jade-yard imagery so fungible markets are visibly separate from Cave's pixel/NFT language and from generic neon launchpads. “Rough / open a window / reveal the jade” and “pick a stone” are the current lifecycle metaphors, inspired by jade-stone discovery rather than signal/momentum language. Exact naming and wordplay remain deliberately editable; product structure and economic terms do not depend on them | ✅ prototype art direction; ⏳ final copy |
| 69.32 | **The checked-in Jade build is wired to committed state, without inventing market history.** It has a provider-only gallery, launch workbench and immutable asset-address pages. Prices, phase, sold amount, active ZEC reserve, opening liquidity, overflow and locked LP come through Nap Wallet from the node; exact curve and AMM quotes precede approval-gated writes. The chart shows the deterministic bonding curve and current committed position, not fake OHLCV. If the provider or future A35 projection is unavailable, the UI says so and never falls back to fabricated catalog, volume, holder or history values | ✅ live state/wallet path; ⏳ A35 history and rollout |
| 69.35 | **Cave Jade uses the same trust boundary as hosted ZynZap.** The static page has `connect-src 'none'` and may call only Nap Wallet's injected EIP-1193-style provider. Public reads are `zyn_curves`, `zyn_curve`, `zyn_curveQuote` and the existing pool quote; writes are `zyn_launchCurve`, `zyn_buyCurve`, `zyn_sellCurve` and graduated `zyn_swap`. Every write opens an extension-owned approval describing the exact token amount, max/min ZEC.zy, immutable fee and launch cost where applicable. The Rust quote client and JSON/wallet APIs preserve every hop's fee asset, total, pool, protocol, creator and POL shares; “best case” is never labeled as a fee | ✅ provider/API/approval path |
| 69.33 | **Use the bonding curve, not a fixed-price sale or immediate creator-seeded AMM.** It gives every permissionless launch the same reversible price-discovery path, needs no creator market maker, keeps pre-graduation liquidity available to sellers and creates a deterministic fee/revenue path into ZynZap. Its costs—more consensus math, sequential execution, thin early atomic prices, curve/AMM discontinuity risk and MEV/slippage—are accepted and bounded by exact division-last arithmetic, buy-up/sell-down rounding, signed limits, one atomic graduation and exact opening-price continuity | ✅ architecture/VM |
| 69.34 | **Public ZynZap swapping follows Cave NFT secondary trading in the Phase-1 rollout.** Building Cave launches may advance the shared pool/RPC/wallet machinery, but it does not bypass the release order: NFT secondary trading and its 50/40/10 accounting are exercised first, then launchpad curve/AMM writes are enabled through the same reviewed wallet approval boundary | 🔒 rollout order; NFT VM split ✅; UI/rehearsal ⏳ |

## 70. NFT ownership privacy from the public, while trusting operators (13 Sep 2026)

**Chosen direction, not implemented or deployment-verified.** This supersedes
earlier privacy assurances in §21 and any implication in §§47, 49 and 65 that
signed reads alone hide ownership from all public data readers. It does not
authorize restricting a live mirror, deleting history or weakening verification.

| # | Decision | Status |
|---|---|---|
| 70.1 | **Near-term target: protect NFT ownership from public readers while trusting authorized operators and replay verifiers.** Those parties still see execution; transaction counterparties and event issuers learn the destinations they serve. No promise of operator privacy, anonymity, hidden activity timing or protection from an authorized reader leaking plaintext | 🔒 scope |
| 70.2 | **Audit every disclosure, not just account RPCs.** The publisher includes raw epoch journals; Transfer names sender/recipient/asset, MintCollectionItem names its recipient, and Reblind carries the blind. Base states, mirrors, caches, proofs, offers, claims, diagnostics and media traffic are in scope. Check deployed exposure separately from source capability and use synthetic examples in public reports | 🔒 work; [#21](https://github.com/zyn-io/nap-wallet/issues/21) |
| 70.3 | **Separate approved public commitments from confidential replay/state artifacts.** Full shared journals and base states need an authenticated, confidential distribution boundary for authorized readers; filtering only NFT opcodes is insufficient. Exact artifact format, encryption/access controls, key lifecycle and metadata disclosure budget remain design work | 🔒 direction; ❓ mechanism; [#22](https://github.com/zyn-io/nap-wallet/issues/22) |
| 70.4 | **State the verification tradeoff.** Authorized independent verifiers must still replay and reject invalid state, preserving quorum and backing checks. Restricting their inputs reduces unrestricted public replay; hash-only roots and operator certificates are not equivalent to permissionless execution verification. Do not claim otherwise or silently bypass checks | 🔒 invariant; [#22](https://github.com/zyn-io/nap-wallet/issues/22) |
| 70.5 | **Holder recovery is a release gate.** Specify authenticated delivery and durable holder export of anchored records and inclusion/exit witnesses; rehearse cold start, lost browser state, operator outage and forced exit. Document any dependency on previously retained witnesses. Do not switch publication modes until the documented recovery path is proven | 🔒 gate; [#22](https://github.com/zyn-io/nap-wallet/issues/22), [#25](https://github.com/zyn-io/nap-wallet/issues/25) |
| 70.6 | **Fresh receiving accounts and blinds are supporting measures, not the privacy boundary.** Use recoverable versioned derivation, prove account discovery after restore, preserve legacy authority, and confirm Reblind execution before displaying a blinded status. A blind present in a public journal is not secret; initialize/rotate only under the reviewed confidential path | 🔒 design; [#23](https://github.com/zyn-io/nap-wallet/issues/23) |
| 70.7 | **Do not reconstruct the holder map in another product surface.** Review public offers and listing opt-in, claim allocation records, explorer exports, support logs and per-item media fetches. Catalogs and deliberate public listings remain public with explicit disclosure. Reuse existing media/claim work, including #17 and #13, rather than creating a competing implementation | 🔒 design; [#24](https://github.com/zyn-io/nap-wallet/issues/24) |
| 70.8 | **Privacy begins at a documented future chain/epoch cutoff, not retroactively.** Previous journal copies and external-chain links remain observable. Rehearse verifier-first rollout, credentials and rollback on disposable testnet; a rollback to plaintext publication must stop/warn privacy-sensitive writes rather than silently leak them. No event privacy promise until the integrated gate passes | 🔒 gate; [#25](https://github.com/zyn-io/nap-wallet/issues/25) |
| 70.9 | **Aliases are deferred. Operator-private NFT notes, nullifiers and zero-knowledge transfers are a separate later scope.** They are not prerequisites for this selected operator-trusted tier, and no bespoke proof system is authorized by this decision | 🔒 scope |

Implementation order and acceptance evidence are in BACKLOG P0–P4. A standalone
public planning copy is `docs/NFT-PUBLIC-PRIVACY.md`; each linked GitHub issue is
self-contained. Current production visibility has not been audited by this
documentation change.

## 71. ZYN supply: minted once, released by activity, never burned (13 Sep 2026)

Completes §68.4, which fixed ZYN's supply as a constant but left issuance and
allocation open. It keeps §30's two numbers — 21,000,000 and the 18,900,000
remainder — and discards the mechanism that produced them: those 18.9M were a
halving *emission* series, created block by block. Here they exist from genesis
and are *released*, never minted.

The distinction matters because an emission schedule makes total supply a
function of time, so no one can state it without also stating a date.

| # | Decision | Status |
|---|---|---|
| 71.1 | **The full 21,000,000 ZYN is minted once, at genesis, and no mint authority survives it.** Eighteen decimals. There is no later issuance path, no inflation parameter and no key that can create a unit | 🔒 |
| 71.2 | **18,900,000 — ninety percent — goes into protocol-controlled keyless incentive pots at genesis.** Keyless in the §69.20 sense: derived addresses no key can spend from, releasing only under committed rules. Nobody can withdraw the reserve, including us | 🔒 |
| 71.3 | **Three supply numbers are tracked and reported separately, and all three are derived from chain state rather than asserted.** `total` is always 21,000,000 and never moves. `unlocked` is what has left the keyless pots. `circulating` is `unlocked` minus ZYN held by protocol-controlled addresses — POL, treasury and buyback reserve. An explorer must be able to recompute all three without trusting us | 🔒 definitions; ⏳ read API |
| 71.4 | **ZYN is never burned.** Burning makes total supply a moving target, invites a deflation story the mechanism does not support, and destroys the only thing the three numbers are good for. Supply is conserved; what changes is which pot holds it | 🔒 |
| 71.5 | **Fee-funded buybacks return ZYN to the incentive or POL reserve for reuse.** A buyback is a transfer from the market into a keyless pot, not a destruction event. This keeps `total` constant while reducing `circulating`, which is the honest description of what a buyback does | 🔒 |
| 71.6 | **Rewards unlock only when the product activity they pay for exists.** An unreleased pot is not a promise of future emission, and time alone never unlocks anything. This is what replaces per-block emission: the reserve is large, but it is inert until something real draws on it | 🔒 principle; ⏳ rules |
| 71.7 | **The unlock rule may not be per-trade volume** (§69.15). Paying ZYN per trade turns a fixed reserve into finite fee mining and makes wash activity indistinguishable from adoption — the same reason per-trade ZYN rewards were already rejected for Cave. Each unlock rule must name an activity a participant cannot manufacture against themselves, and must be committed before its pot can release | 🔒 constraint; ⏳ rules |
| 71.8 | **Phase-1 ZEC support is transitional by design.** ZEC-denominated fees accrue protocol-owned liquidity while ZYN has no market of its own; §68.9's Phase-2 transition winds that support down under precommitted rules as the ZYN:BOLD.zy market takes over price discovery. The hard ZEC reference is scaffolding for the bootstrap, not a permanent dependency, and separating from it is the goal rather than a failure mode | 🔒 |

| 71.9 | **The 2,100,000 remainder is the ZYN side of the opening ZYN:ZEC.zy market.** It waits in its own pot until enough ZEC.zy has accrued to pair against it, and the pairing ratio sets the opening price. Nothing is distributed to contributors — §68 superseded that | 🔒 destination; ⏳ price |
| 71.10 | **Nothing is distributed before Phase 2 opens.** At open the 2.1M sits *in* the pool, so `circulating` is zero and every holder bought at market. There is no free float to sell into the opening price — the distribution risk that usually accompanies a 90% reserve does not exist at launch. It begins at the first reserve release, not before | 🔒 |
| 71.11 | **The Phase-2 trigger is accrual, not a price print.** With no market before Phase 2 there is no price to observe, so §68.7 cannot be read as watching for `0.0002`: the condition is the future-ZYN-POL pot reaching **420 ZEC.zy**, which is what lets the pot open at `0.0002` against 2.1M ZYN. Opening the market *is* Phase 2 beginning, not a consequence of it. This retires §68.8's observation rule, averaging window and liquidity floor: an accrual counter cannot be wash-traded, so none of that machinery is needed | 🔒 trigger; supersedes §68.8 |
| 71.12 | **ZYN cashback is cancelled in every form, not only the minted one.** §68's precedence rule supersedes "freshly minted cashback", which leaves a reserve-funded cashback readable as still available. It is not. Cashback died with mint-and-burn and does not return under §71's reserve | 🔒 |
| 71.13 | **Nobody receives ZYN at all under the current plan.** The 18,900,000 reserve is inert: no cashback, no rewards, no allocation, no airdrop today. A credit system involving a ZYN airdrop is intended, but it is not designed and nothing is owed to anyone. `unlocked` and `circulating` are therefore both zero until Phase 2 opens the market, and the first ZYN anyone holds is bought from the pool | 🔒 |
| 71.14 | **No gas in Phase 1; Phase-2 gas is open.** §68.5 read as permanent and argued for it. Narrowed to Phase 1 on 13 Sep. The reasoning does not survive the narrowing unchanged: if ZYN becomes gas in Phase 2, part of its demand *does* come from users needing it to transact, which is what §68.5 rejected. That is a deliberate reversal at the phase boundary, not an oversight — recorded here so it is not rediscovered later as a contradiction | 🔒 |
| 71.15 | **Gas is the first candidate answer to ZYN's empty utility surface.** Not gas in Phase 1 (§68.5), not redeemable (§68.9), not per-trade rewards (§69.15), not cashback (§71.12) — Phase-2 gas would be the first named use, alongside the credit system in A38. It does not resolve A40: a gas fee is paid, not unlocked, so it says nothing about what releases the 18,900,000 reserve | 🔒 direction; ⏳ design |
| 71.16 | **Phase-2 direction, recorded but not designed: lower the ZEC.zy trading fee and introduce ZYN gas.** The ZEC-denominated fee rail is Phase-1 bootstrap pricing; once ZYN has its own market the rail comes down and ZYN carries transaction cost instead. Further use cases are intended and undecided. Nothing here is specified, nothing is committed, and no Phase-1 work should be shaped around it | ⏳ direction only |
| 71.17 | **A token's supply policy is chosen at creation, recorded on the token, and may only ever move toward `Fixed`.** `SupplyPolicy` is `Fixed`, `Capped { authority, cap }` or `Open { authority }`. `MintToken` issues under it; `RenounceMintAuthority` gives up the power permanently and is the *only* transition the VM offers. A token's supply guarantees can therefore strengthen over its life and never weaken | 🔒 |
| 71.18 | **The risk is undisclosed mintability, not mintability.** "No token can ever mint" was a limitation, not a principle: a chain hosting other people's applications needs stablecoins, game tokens and reward systems. So the answer is disclosure at protocol level — the policy rides on the asset record, the RPC, the JSON and the wallet, where no frontend can misreport it — rather than prohibition | 🔒 |
| 71.19 | **ZYN is issued by the chain, not by an account.** `is_official_asset` decides officialdom by address and holds only because users cannot derive a *native* address. So `IssueToken { native: true }` is operator-only, the node issues ZYN once at its own genesis, and a user reusing the symbol lands in the `asset` namespace with a different address that reads as unofficial | 🔒 |
| 71.20 | **`IssueToken` exists because issuance and launching are different acts.** `CreateToken` is a launch: it demands xZEC above the bond to seed a pool and hands the remainder to the creator. ZYN needs neither — its supply goes straight into keyless pots and its market opens in Phase 2 — and a chain at genesis has no xZEC to seed with. Allocations must sum to `supply` exactly, so `total` is exact by construction and §71.3's numbers mean something | 🔒 |
| 71.21 | **The 18,900,000 sits in one reserve pot, not several.** §71.2 says "pots" and names none; §71.13 says nothing is owed to anyone. Naming allocations now would commit to a design that does not exist. §71.4 already permits the later split — "what changes is which pot holds it" — and a move between keyless pots leaves all three of §71.3's numbers untouched. `ZYN_RESERVE` and `ZYN_POL_ZYN` are pinned constants | 🔒 destination; ⏳ split |
| 71.22 | **The reserve has no release path, and that is deliberate for Phase 1.** §71.2's "releasing only under committed rules" with no rules designed means nothing can leave. Adding one later is a `STATE_VERSION` change, not an upgrade — but §71.16 already makes Phase 2 a different chain version, so the release rules ship with it. The alternative, a treasury-controlled release, contradicts §71.2's "nobody can withdraw the reserve, including us" | 🔒 |
| 71.23 | **Keyless pot balances are public; ordinary balances are not.** `OP_ACCOUNT` derives the account from a signature, so a stranger cannot read a balance. `OP_POTS` is the deliberate exception over a *closed set of pinned addresses*: nobody holds a key to them, so there is no privacy interest, and §71.3 requires the numbers be derivable from chain state rather than asserted. A reserve nobody can audit is only a claim | 🔒 |
| 71.24 | **The reserve matches ZEC.zy accrual into the POL pot at `0.0002`, capped at 2,100,000 ZYN.** `launch::credit_pol` is the single route by which protocol-owned liquidity accrues, so the matching cannot be forgotten at a call site added later. 420 ZEC.zy fills the pot to exactly 2,100,000 — §71.9's number and §71.11's trigger are the same statement seen twice, and the rule makes the opening price a property of how the pot filled rather than a figure to hit | 🔒 |
| 71.25 | **Past the cap the reserve is never drawn on by this path again.** Further accrual moves price, not supply: once the pot's contents are the pool, price is the AMM's job and additional ZEC.zy buys ZYN out of it, which is §71.5's fee-funded buyback. The earlier worry that accrual would drain the reserve assumed the matching continued after open. It does not | 🔒 |
| 71.26 | **No signature authorises a release.** A keyless pot cannot produce one, and that impossibility is what §71.2 rests on. The release is a rule the VM applies inside a handler — the same shape as every existing pot payout — never a withdrawal anyone can request | 🔒 |
| 71.27 | **LP shares from the POL pool go to the treasury, so the liquidity is withdrawable by 2-of-3.** Deliberate, and a departure from "protocol-owned": liquidity nobody can ever move is dead capital, and a stranded pool after a redesign serves no one. The cost is real and is stated rather than hidden — pulling liquidity is the canonical rug, so Zyn's trust claim covers deposits, mint proceeds and the reserve, and **not** the ZYN market's liquidity. Say so plainly wherever the claim is made | 🔒 decision; ⚠️ disclose |
| 71.28 | **Every keyless account is registered in `is_pot`.** The derived ZYN pots were missing when they were added: reward distribution skips pots precisely because a pot cannot spend, so paying one strands the share every epoch. A test now asserts the registry covers all of them, and that the treasury is *not* one — it holds a key and earns LP rewards through the ordinary path | 🔒 |
| 71.29 | **The ZYN market opens on a published Zcash height, 8,064 blocks (~7 days) after the POL pot fills.** Filling the pot *arms* a date; it does not open anything. The opening block is arithmetic from public state — `armed + POL_OPEN_GRACE_BLOCKS` — so nobody has to be trusted to announce it, or to announce it on time | 🔒 |
| 71.30 | **The grace period is measured in Zcash blocks, not Zyn epochs.** Epochs seal faster under load, so a countdown in epochs accelerates exactly when the chain is busy — useless for anything a person plans around. Zcash height is external, ~75s a block, unaccelerable by anyone trading on Zyn, and it is already the chain Zyn anchors to. `SwapState.zcash_height` is tracked chain-level rather than inside the superseded `LaunchState`, and the node feeds it once a minute from its own Zebra | 🔒 |
| 71.31 | **The opening pairs exactly 2,100,000 against exactly 420, whatever else accrued during the grace period.** The price was published with the date; pairing everything in the pot would make the opening price depend on how busy the wait happened to be, which is not a price anyone can prepare for. The surplus stays in the pot as buyback reserve (§71.5) | 🔒 |
| 71.32 | **All 21,000,000 starts in the reserve; the POL pot fills only from accrual.** Genesis pre-filling the pot with 2,100,000 left nothing for §71.24 to match and — because `credit_pol` returned early when the pot was full — meant the market could never become due at all. Found in rehearsal, not by a test. The pot filling *is* the signal that real backing exists, so it must be earned rather than granted | 🔒 |
| 71.33 | **`STATE_VERSION` reached 12, and each step reset mainnet rather than migrating it.** 10 → 11 added the supply policy, 11 → 12 added `zcash_height` and the market's arming height. `codec.rs` hard rejects a mismatch, so there is no migration path by design. Each reset repointed `ZYN_DATA_DIR` at a fresh directory instead of clearing the old one, leaving v10 and v11 intact on disk with their binaries and env files — rollback is a one-line edit. This was cheap only because nothing had been anchored to Zcash; after the first anchor it stops being free | 🔒 |
| 71.34 | **A version bump strands every replica on the old chain.** The §81 signer fix cannot reach testnet: it lives in a v12 tree and chain 11 runs v10, so a v12 `zyn-replica` rejects the state and exits. Chain 11's signers stay on the pre-§81 binary until that chain is reset. The general rule: a `STATE_VERSION` change is a coordinated replacement of the sequencer *and* every replica, never a rolling upgrade | 🔒 |
| 71.35 | **A re-anchor must point the ledger at the transaction it created.** `reanchor` broadcast, logged success and returned the new txid, and `drain_reanchors` discarded it — so `reorged` went on re-checking the *old* txid, which really was gone, and refused to anchor for ever. On 16 Sep three re-anchors sat confirmed on Zcash with ~1,500 confirmations while the sequencer insisted the lineage was broken. Only the txid is updated: the status stays `Confirmed`, because `follow_up` acts on `Broadcast` entries by calling `confirm_anchor`, and moving it back would settle those exits twice (§50.6) | 🔒 |
| 71.36 | **Resetting a chain invalidates everything downstream of it, and they must be cleared in order.** Sequencer state, the DA directory, the mirror's copy, every replica's store, and each replica's pinned `ZYN_BASE_ROOT`. Miss one and the failure names something else: a replica reading a stale mirror reports `BAD BUNDLE`, an unpinned base root reports `Decode`, and a replica scanning from before the reset finds the *previous* chain's anchors on Zcash and demands bundles that never existed | 🔒 |
| 71.37 | **A replica's scan start must be above the old chain's last anchor.** Zcash keeps the anchors of every incarnation of a chain id. A fresh chain 11 with `ZYN_BRIDGE_FROM_HEIGHT` still set to the old value sees those anchors as sightings, asks the mirror for their bundles, and halts. Move it past the last anchor of the chain being replaced | 🔒 |
| 71.38 | **A crash loop during genesis corrupts the epoch-0 journal.** Sixteen boots against one data directory wrote genesis sixteen times and left sequence numbers that no longer increase, so the bundle became unreadable (`OutOfOrder`) and nothing could be published or mirrored. The node refusing superseded config (`ZYN_LAUNCH`) is right; letting systemd restart it into the same directory is what did the damage. After any crash loop at genesis, start from a clean directory rather than repairing | 🔒 |
| 71.39 | **Attribution is by address first, and only then by memo.** `classify_addressed` tries the diversifier index a note arrived at before anything else: an account's deposit address is derived deterministically from the account (`deposit_index`), recovered from the note (`index_of`), and needs no memo and no cooperation from the sender. A note at the *vault's own* index deliberately falls through to memo, then change, then the operator's hand-written attribution — because the vault's base address is for change, anchors and top-ups, not for deposits anyone should be given. **Corrects an earlier reading of this entry**, written from a failed deposit, which said a per-account address does not remove the memo requirement. It does | 🔒 |
| 71.39a | **So the failure mode is handing out the wrong address, not the missing memo.** A depositor given the vault's base address has no way to be credited without a memo their faucet or exchange will never set; the funds arrive, sit unattributed, and wait for a line in `zcash-attributions`. The wallet used to offer that address as an ordinary copyable string whenever no per-account one existed. It now says what the address needs, and shows the exact `ZYN1:<account>` to include | 🔒 |
| 71.40 | **Attributing a txid that never paid the vault is a no-op**, so when a block holds several shielded transactions and the right one cannot be told from outside, listing every candidate is safe rather than sloppy: the map is consulted only for funds the vault actually received | 🔒 |
| 71.41 | **A note tree rebuilt from far behind will spend notes it does not yet know are spent.** Scanning from 4,326,863 toward a spend at 4,352,711 at ~1,800 blocks an hour meant fourteen hours of `ironwood double-spend` — the node was not broken, it was reading history. On a reset chain the scan start belongs *after* the previous incarnation's last spend; the older notes become invisible, which is the right trade when they are already spent | 🔒 |

### Open, and load-bearing

**The opening price and the Phase-2 threshold are currently one number, and
cannot be.** §68.7 makes `1 ZYN = 0.0002 ZEC` the signal that "the ZEC-supported
bootstrap has done its job." If §71.9's pot *opens* at that price, the trigger
fires at launch and Phase 1 never runs. Pairing 2.1M ZYN needs **420 ZEC.zy** to
open at 0.0002, or **4,200** at 0.002 — and opening at 0.002 would launch ZYN ten
times past the wind-down point. Two numbers are needed, with the threshold above
the opening price.

**Opening depth binds from the first reserve release, not at launch.** Under
§71.10 nothing circulates at open, so there is no free float to sell. But the
pool is thin — 2.1M ZYN against 420 ZEC.zy moves 8.9% on a 100,000-ZYN sell and
34.8% on 500,000 — and 18,900,000 ZYN sit behind it. Recipients of a reserve
release *did* get their ZYN free, so the release rate must stay slow relative to
how fast the pool deepens, or each unlock walks the price down its own curve. A
constraint on §71.6's rules as binding as the activity test.

**ZYN has no named utility yet.** Not gas (§68.5), not redeemable (§68.9), not
per-trade rewards (§69.15), not cashback (§71.12). Every *bad* utility has been
removed deliberately and none has replaced them. This blocks §71.6 concretely:
the release rules cannot say what activity unlocks the reserve until it is
decided what the reserve pays for.

**Reserve per token is a tenth of the quoted price.** 420 ZEC.zy behind
21,000,000 ZYN is `0.00002` ZEC per ZYN against a `0.0002` price. The pairing
sets an opening price and some depth; it is not a reserve anyone can claim
against, and "ZEC-backed" invites exactly that computation.

**Who may release from a pot, and on what evidence.** §71.6 says activity
unlocks rewards; it does not yet say who evaluates that. If a human decides,
the pots are a treasury with extra steps and "keyless" is decoration. The rule
has to be a committed predicate over chain state, evaluated by the VM, or the
reserve is discretionary.

**Buyback release authority** (§71.5). ZYN bought back and returned to a pot can
be released again. A buyback whose proceeds can be re-released at someone's
discretion is a treasury operation, not a buyback, and should not be described
as one until the release rule is committed.

**A ninety-percent reserve makes fully diluted value ten times circulating at
launch**, which is the shape critics reach for first. §71.3's three numbers are
the mitigation and only work if they are published from day one, together, with
the reserve's release rules — not `circulating` alone.

## 72. An endorsement must survive the sequencer that collected it (12 Sep 2026)

> Renumbered 14 Sep from §63, which collided with "ZynZap is hosted without
> making the Zyn RPC public". Both were written on 12 Sep in parallel trees.

| # | Decision | Status |
|---|---|---|
| 72.1 | **This was the actual cause of the 2810-pass chain-11 outage**, and it predates §61 and §62. The sequencer keeps signatures for an *in-flight* anchor in memory; only a **settled** anchor's certificate is persisted. It restarted at 09:25 UTC on 11 Sep, after signer1 had endorsed at 08:02 UTC, and silently discarded that signature | ✅ |
| 72.2 | A signer endorsed each anchor **exactly once** — `take_newly_verified()` drains it and `seen` prevents re-verification — so a lost signature could never be replaced and the threshold became permanently unreachable. One restart at the wrong moment was enough to strand the chain forever | ✅ |
| 72.3 | Signers now **re-offer** an endorsement every pass until the sequencer reports the certificate clears, and seed that list at startup with their head anchor — the one a restarted sequencer can still be awaiting. Endorsing is idempotent, so re-offering costs one call and heals the whole class | ✅ |
| 72.4 | **Every layer reported success while the chain was dead.** signer1 said "counted, certificate now clears", the sequencer said "awaiting signatures", and neither was wrong about what it had seen. Liveness failures hide in the gaps between components that are each individually healthy | ✅ |
| 72.5 | **Diagnostic order matters more than fix quality.** §61 and §62 are real bugs, tested and deployed, but neither was on the critical path; each masked the next. The cheap check that would have found this first — the sequencer's 23-hour uptime against signer1's 08:02 log write — was never run because the code looked guilty | ✅ |
| 72.6 | Durable per-anchor certificates on the sequencer remain the stronger fix and stay open. Re-offering heals the symptom from the signer side and needs no sequencer state change, which is why it shipped first | ⏳ |

## 73. A replica only consults its base when its data directory is empty (12 Sep 2026)

> Renumbered 14 Sep from §64, which collided with "Cave launches are configurable
> promises, then immutable ones".

| # | Decision | Status |
|---|---|---|
| 73.1 | signer3 was stranded because the §A17 re-base left its base root **off the lineage**: the chain went 5397 → 6692, and no anchor ever carried the root it was based on. It sat harmless for a day because its base epoch was *above* the live one — there was nothing to check until an anchor appeared above it, and then it broke instantly | ✅ |
| 73.2 | It was rebuilt by **copying signer2's verified state directory**, not by attesting a base. `open_from` consults the base only when `store.load()` returns `None`, so a populated data directory bypasses that path entirely — which is exactly why signer1 and signer2 never met the fault | ✅ |
| 73.3 | The copy was taken **hot**, with signer2 still running, so the set never dropped below threshold. A torn snapshot would have failed only signer3, leaving the chain untouched — the cheap risk was the right one to take first, and it paid | ✅ |
| 73.4 | **Only chain state moved; the signing key did not.** signer3 still endorses as set slot 3 from its own key in its own env file. Copying state between replicas is safe precisely because identity lives outside the data directory | ✅ |
| 73.5 | The cost, named plainly: signer3 now starts from a root **signer2 derived** rather than one it derived itself. That is the same class of trust as attesting a base root out of band — the designed mechanism — but it is a real reduction in independence and was taken as a deliberate decision, not a default | ✅ |
| 73.6 | Why the base path failed at all is **unresolved** (A22): the binary reported a fixed attested root regardless of `ZYN_BASE_ROOT`, including an explicit override, contradicting the only such check in the source. Not blocking, because no replica now starts empty — but a genuinely fresh replica would hit it | ⏳ |

## 74. The 333 Nap Moles are a gift, not a sale (14 Sep 2026)

The 4,444 and the 333 are both collections under `Treasury 26460` (§66.17) and
share the secondary fee split (§69.9), which makes it easy to assume they share
a distribution path. They do not, and the difference is worth stating before
someone builds the wrong one.

| # | Decision | Status |
|---|---|---|
| 74.1 | **The 333 do not use `cave-sale`.** That crate allocates a *paid* sale: every ticket must carry a confirmed Zcash payment whose amount equals the entry price, and an entry with no `deposit_txid` is rejected. A gift has no payment and no txid, so it is not a degenerate case of a sale — it is a different thing | 🔒 |
| 74.2 | **The chain already supports it.** `MintCollectionItem { creator, collection, serial, to, symbol, content }` moves no value and is authorised by the collection creator alone. Nothing new is needed in the VM: the gift is a creator-signed mint at a chosen serial, to a chosen account | ✅ VM |
| 74.3 | **Serials stay bound the same way as a sale's.** Each item is `item_address(ADDRESS_SCOPE_V1, collection, serial)` (§64.13) against a manifest committed before any mint. Being a gift removes the *allocation* question, never the *identity* one — two people must not be able to receive the same mole, and a recipient must be able to check which one they got | 🔒 |
| 74.4 | **No drand, no reveal, no allocation policy.** Those exist to prove a paid sale was fair. The operator chooses who receives which mole, and saying so plainly is more honest than staging randomness over a decision that was never random | 🔒 |
| 74.5 | **A claim code is an operator promise, redeemable once — not a bearer asset.** The creator key can mint to any account regardless, so the claim service is trusted by construction. It must not be described as trustless, and a code must never be presented as something that holds value on its own: it authorises one mint of one serial, and is void once used | 🔒 honesty boundary |
| 74.6 | **Claiming binds the recipient's own account, chosen by them.** The code carries the serial; the claimant supplies the account. Re-submitting the same code with the same account returns the same result rather than minting twice; with a *different* account it is refused, because the first binding is what the recipient already holds | 🔒 idempotency; ⏳ service |
| 74.7 | **The media travels with the gift.** Handing over the file at the event and letting the wallet check it against `content` needs no gateway and leaks nothing (§67.7). For the 333 specifically this is the whole privacy story, and it is also the only one that works in a room with bad wifi | 🔒 |
| 74.8 | **Claiming is a transfer, not a mint — §74.2 is superseded by what genesis did.** All 333 were pre-minted to the treasury and the collection closed, so `MintCollectionItem` cannot run: it needs `Minting` and the cap is full. The trade is not all loss — every serial's `content` was committed on chain before anyone could ask for one, which is the binding §74.3 wanted, reached by another route. What is lost is `minted` as a live measure of how many have been given out | 🔒 |
| 74.9 | **A claim binds before it signs.** The book records which account a code belongs to *before* a transfer is submitted. A crash between submitting and recording would otherwise lose who the item was for, and the next person with that code would take it. The opposite failure — bound but never sent — is repaired by the same retry, so the order is the safe one | 🔒 |
| 74.10 | **The service never reads the chain, because it cannot.** Balances are private: `OP_ACCOUNT` derives the account from a signature, and a threshold account has no single key to sign a read with, so the treasury cannot look up its own holdings. It does not need to. An item is one indivisible unit, so a repeat transfer fails with `InsufficientBalance` — the chain itself prevents a double grant, and this service treats that rejection as the success it is | 🔒 |
| 74.11 | **Codes are stored only as fingerprints.** `derive(b"zyn.claim.code.v1", b"code", [code])`, normalised for case and surrounding space so a code read off paper survives a phone keyboard. Reading the book yields nothing redeemable; a lost code is reissued as a new one, which is the right shape for a promise | 🔒 |
| 74.12 | **A whitelist invitation is an allocation entitlement, not an item and not a payment.** It says "good for up to N mints"; *which* N is decided by `cave_randomness::allocate` from evidence nobody controls, and the price is paid from the buyer's own balance at mint time. Like a claim code it is an operator promise and is trusted by construction | 🔒 |
| 74.13 | **No addresses are collected in advance.** Zyn accounts are private, most invitees will not have one when invited, and asking early would leak who is interested in a collection before they decide to buy. So an invitation is a code and the holder binds their own account on first use — the same shape as §74.6, for the same reason | 🔒 |
| 74.14 | **The whitelist phase is stored, not held in the service.** A window that reopens because a process was bounced is not a window. A registry is closed until explicitly opened, so one that is loaded and never opened cannot sell anything by accident | 🔒 |
| 74.15 | **A failed mint returns the allocation but keeps the binding.** Otherwise a buyer whose mint failed hands their invitation to whoever asks next. The reservation is recorded before anything irreversible happens and released if the mint does not land | 🔒 |
| 74.16 | **The whitelist form collects an X handle and a link, and nothing else.** No email, no wallet, no Zcash address. The link is required rather than optional because it is the only thing a reviewer can open and check — without it the engagement requirement is unverifiable and the form is collecting names | 🔒 |
| 74.17 | **Spam is handled in layers aimed at volume, not identity.** Honeypot, dwell check, per-address rate limit, handle dedupe, post dedupe, and a proof of work. None of them identify anybody and none decide anything; the last word is a person reading the application, which is why they can be modest — they exist to keep the queue readable, not to be a security boundary | 🔒 |
| 74.18 | **The proof of work is tied to the applicant's own handle and a ten-minute window**, so it needs no secret, no round trip and no third party: work done for one handle is worth nothing for another, and work done an hour ago is worth nothing now. It runs when the handle field loses focus, so it completes while the rest of the form is filled and nobody waits for it. No CAPTCHA — Turnstile stays available if volume ever outgrows this | 🔒 |
| 74.19 | **A resolved KV `put` is committed.** Eventual consistency governs when a read elsewhere sees a record, not whether it was stored, so the only way to lose an application is a write that throws — and that reaches the applicant as an error saying nothing was saved. The application is written before the post claim: the reverse order would claim a post for an application that was never stored, locking out the person it belonged to | 🔒 |
| 74.20 | **The register is exported to a copy the operator holds.** KV is durable but it is one company's copy of a list that will be acted on, and the same account serves the site and the art. The Telegram notifications are already a second copy arriving one at a time; `npm run export` is the one that can be grepped. `register/` is gitignored — it holds other people's handles | 🔒 |
| 74.21 | **One bot carries everything, including the applicant list.** Reusing `@Duchinski_bot` rather than standing up a second was right: same chat, same allowlist, same thing already being watched. It does widen what a compromised bot token *discloses* — from the chain height, which the DA store publishes anyway, to who has applied — and does not widen what the bot can *do*: still no key, nothing signed, nothing submitted | 🔒 |
| 74.22 | **A paid mint is three steps, not one transaction, because the VM has no atomic paid mint and this does not invent one.** The buyer signs `FundCollection` themselves — their money, their signature, landing in the collection's pool where it backs every outstanding item rather than in anyone's pocket. The allocator then decides *which* items from evidence nobody controls. The creator then mints, threshold-signed. The buyer pays before they hold anything, so like a claim code this is an operator promise | 🔒 |
| 74.23 | **Payment is relayed, not observed.** Balances are private, so the service cannot look up whether a buyer paid — and does not need to. The buyer hands over a *signed frame*, the service submits it, and acceptance is the proof. The service holds no key of the buyer's and needs no read it is not entitled to | 🔒 |
| 74.24 | **A quote reserves the allocation before the buyer pays**, so two people holding one invitation cannot both be quoted the last item. A quote never paid is released by `abandon`, which returns the allocation but keeps the binding (§74.15): a reservation is a hold, not a sale, and not a way to lose your invitation by hesitating | 🔒 |
| 74.25 | **The settle order is chosen so every crash leaves something a retry can finish, and none leaves a buyer who paid with nothing.** Payment is recorded the moment it is accepted, so a crash after it loses no money and the retry skips to minting. The allocation is written before any mint, so a retry mints the *same* serials rather than drawing fresh ones. Minting is idempotent per serial. All three orderings have tests | 🔒 |

## 75. Mainnet takes A25 first, because it was the chain with nothing on it (14 Sep 2026)

`STATE_VERSION` 10 rejects every earlier version and there is no migration
branch, so adopting A25 is a reset rather than a rolling upgrade. The obvious
order — rehearse on testnet, then mainnet — was the wrong way round here.

| # | Decision | Status |
|---|---|---|
| 75.1 | **Mainnet 26460 rolled first, because it had never credited a deposit.** Zero credit events, epoch 0, no backing obligation: resetting it destroyed nothing. Testnet chain 11 had 9,393 epochs and was healthy, so resetting *it* as the "safe" rehearsal would have been the more destructive choice. Check what a chain holds before deciding which one is disposable | ✅ |
| 75.2 | **New genesis base root `29de5266179130872cfc14b4517f21b042acb7c6e850e423c813331a9b60f25d`.** Every mainnet replica and signer attests this root; the previous `da91df7d…` belongs to the pre-A25 chain and must not be reused | ✅ |
| 75.3 | **The old chain is preserved, not deleted** — `/var/lib/zyn/mainnet.preA25`, `mainnet-da.preA25`, and a tarball. A reset with no way back is a bet, not a deployment | ✅ |
| 75.4 | **Testnet chain 11 stays on the pre-A25 binaries** as a working reference while mainnet proves the new code. It is the only chain with history, so it is the one worth keeping intact | ✅ |
| 75.5 | **Check `uname -m` before building for a host.** The fleet is mixed — Hetzner `aarch64`, Contabo and the WSL box `x86_64`, the Macs `arm64`. An x86_64 binary was installed on the aarch64 testnet sequencer and failed with `Exec format error`; it was restored from the copy kept beside it, which is the only reason this is a footnote rather than an outage | ✅ |

## 76. Zyn's source stays private; the wallet keeps its own copy (14 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 76.1 | **`zyn-io/zyn` stays private.** The layering question is therefore settled the other way from §A22's suggestion: `nap-wallet` cannot consume it as a git dependency, because a public wallet that needs a private repo to build is a wallet nobody outside the org can audit or reproduce | ✅ |
| 76.2 | **`nap-wallet` keeps a full copy of the protocol crates, and that is the arrangement rather than a temporary state.** PR #27's sync is not interim. What matters now is that the copy is *kept current*: two sources of truth for a state encoding is how two builds of one sequencer come to disagree about a root, and the only defence left is discipline | 🔒 |
| 76.3 | **Every protocol change therefore lands in `zyn` and is synced to `nap-wallet` before release.** A wallet built from a stale copy cannot talk to the chain at all — that is not a degraded mode, it is a hard wire-format incompatibility, as A25 demonstrated | 🔒 process |
| 76.4 | `zyn-io/cave` and `zyn-io/zynzap` continue to consume `zyn` as a **rev-pinned git dependency**. They are private too, so the auditability argument does not apply to them | ✅ |

## 77. What settling to Zcash actually costs, and what it does not delay (14 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 77.1 | **Fees are ZIP-317: `5,000` zatoshi per logical action, floor of two** (`zyn_custody::payout::zip317_fee`). An anchor is the floor — `ANCHOR_ZATOSHI = 10_000`, `0.0001` ZEC — and comes back to the vault as a note, so only the fee leaves. A shielded payout is the same floor; a transparent one is `15,000` | ✅ |
| 77.2 | **Redeeming an NFT costs nothing on Zcash.** `RedeemCollectionItem` burns the item and credits its share of the collection pool in ZEC.zy entirely on Zyn. Only *leaving* for ZEC touches L1 | ✅ |
| 77.3 | **The anchor is amortised, never per-item.** One anchor covers everything in its window (§5.6), so a thousand redemptions inside one window still cost one anchor between them. Individual settlement does not multiply anchors | ✅ |
| 77.4 | **Exits already batch, and the grouping is derived rather than chosen.** `settle.rs` groups pending exits canonically oldest-first, signs one transaction-sized group against the vault's durable nonce, and burns units only once that transaction is final. Signers attest to a list the state determined (§14e level 1). Throughput is one group per finality window — twenty-odd exits — which is the bottleneck nowhere | ✅ |
| 77.5 | **Anchor cadence does not affect bridge speed.** A payout is its own Zcash transaction; "final" in the exit path means that transaction confirming, not an anchor covering it. Anchoring less often trades anchor cost against how long a state root waits to be committed, and touches withdrawal latency not at all | ✅ |
| 77.6 | **Batching more than `settle.rs` already does would remove a property, not add one.** A manual or daily settlement run replaces a state-derived group with an operator-chosen one. The fee saving would be trivial in any case: settling 333 exits individually is `0.0333` ZEC against `0.0167` batched, a difference of about six dollars | 🔒 |
| 77.7 | **`may_cancel` cannot see an in-flight payout.** It checks elapsed epochs and nothing else, so a user past the deadline may cancel and be credited on Zyn while the ZEC is already leaving the vault. The defence is margin, not logic: `exit_timeout_epochs` must exceed worst-case settlement plus confirmations plus the longest outage tolerated without intervention. The code says as much — a deadline "far beyond any honest settlement, not a user convenience" | 🔒 invariant; ⏳ mainnet value |
| 77.8 | **Nobody has decided who absorbs the payout fee.** §69.9 routes NFT trading fees and §69.8 routes curve fees, but the `0.0001` ZEC a withdrawal costs the vault is unaccounted. At this magnitude the protocol absorbing it is the obvious answer; it should be written down rather than left implicit | ⏳ |

## 78. The extension ships unlisted, not as an unpacked zip (14 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 78.1 | **Publish to the Chrome Web Store as *unlisted* rather than telling people to enable developer mode and load an unpacked pack.** Unlisted is reviewed but not searchable: a real install link, automatic updates, no security-warning theatre, and it can be made public whenever | 🔒 |
| 78.2 | **"Enable developer mode and load this zip" is the instruction pattern malicious wallets use.** Teaching it to the earliest users of a privacy wallet trains exactly the habit they most need not to have, and unpacked extensions never auto-update, so a security fix reaches nobody | 🔒 rationale |
| 78.3 | Self-hosting a `.crx` is not an escape: Chrome blocks non-store extensions on Windows and macOS unless installed by enterprise policy | ✅ fact |
| 78.4 | **The manifest is already easy to approve** — MV3, `permissions` is only `storage`, host access is just `http://127.0.0.1:8977/*`, no remote code, and it holds no keys. What draws review time is `<all_urls>` at `document_start` plus `web_accessible_resources` on `<all_urls>`. In Phase 1 the only sites needing the provider are ours, so narrowing `matches` to them would make the first review trivial and can be widened later | ⏳ |
| 78.5 | **The web app is the route for the 333 regardless.** The extension needs `127.0.0.1:8977` — it is a provider backed by the desktop app, not a standalone wallet — so at an event it costs two installs before anyone holds anything | 🔒 |

## 79. A pot is one mechanism, reused (14 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 79.1 | **Keyless pots are all the same derivation**, exposed as `zyn_vm::pot_address(program, purpose, scope)`. ZYN's protocol-owned-liquidity pot, every Cave launch's reserve and escrow, and any later raise are one mechanism with different arguments — so a future token's raise is not a new thing to audit | ✅ |
| 79.2 | **Two properties make a pot auditable without trusting the operator.** Nobody can spend it, because a derived address has no private key (§4.2), so value leaves only by the rules that name the pot. And anyone can recompute it: given purpose and scope the address follows, so an auditor can name the account *before* looking at any state and check what it holds without being told where to look | 🔒 |
| 79.3 | **`ZYN_POL` is `f40435d1406abc3aa4befaf380acea34a4a87ed76a51538f88d09ba0d2937e61`** = `derive(ADDRESS_SCOPE_V1, "zyn-pol", [ZEC.zy])`, and **`XZEC` is `0e212262…9201`** = `derive(ADDRESS_SCOPE_V1, "bridged", ["zcash", ""])`. Both verified against the derivation rather than taken on trust | ✅ |
| 79.4 | **Both were pinned literals with nothing stopping them drifting from their derivations.** They agreed, but only by care — and a constant that no longer derives from anything is how the asset whose whole job is to be 1:1 with ZEC ends up identified by an orphan number. The same shape as `derive_for` silently dropping the VM version (§66.1). Tests now assert both | ✅ |
| 79.5 | **Different purpose or different scope is a different account**, so a later raise cannot collide with an existing pot and needs no registry to avoid one. Asserted, not assumed | ✅ |
| 79.6 | **Cave does not yet honour §66.14's separation.** It derives one `launch_vault(asset)` per launch, while §66.14 requires distinct accounts for curve inventory, graduation inventory, refundable reserve, pair seed, graduation fees, creator escrow and the LP lock — "balances with different movement rules never share an account". One account holding refundable principal *and* creator escrow cannot show an auditor which is which | ⏳ gap |

## 80. A treasury is one call, not a project (14 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 80.1 | **`Treasury::provision(threshold, participants)` returns the account and one share per holder.** The same call serves the protocol treasury, a project's raise or a grant pool; only the numbers change. A future treasury is therefore not a new mechanism to review — it is this one, already tested | ✅ |
| 80.2 | **The chain needs no threshold-aware path.** A FROST group over Ed25519 produces an ordinary Ed25519 signature against an ordinary verifying key, so a treasury is a normal `Scheme::Ed25519` account and every existing authorisation path works unchanged. Asserted, not assumed: a test proves the account equals what a single key with the same verifying key would give | ✅ |
| 80.3 | **Only the driver was ever Zcash-specific.** `DkgParticipant<C>`, `Round1Broadcast<C>` and `ThresholdKeys<C>` were already generic; `run_in_memory` alone named Zcash, which is why provisioning for another curve looked like new cryptography when it was a type parameter. Now `run_in_memory_for::<C>`, with the old name kept for existing callers | ✅ |
| 80.4 | **Refuses `1`-of-`n` and any threshold above the holder count.** A one-of-n "threshold" is a single key wearing a costume, and a threshold nobody can reach is an account nobody can ever spend from | ✅ |
| 80.5 | **Shares must never be stored two-to-a-place.** Same rule as the vault's paper backups (§60.4) — the threshold is worth exactly as much as the separation of its shares | 🔒 |
| 80.6 | **Provisioning is not the same as authority**, so the signing path is built too. `zyn_custody::treasury::sign` threshold-signs any message and returns a group key plus an ordinary 64-byte signature — the caller learns nothing about FROST. `zynzapd::treasury::frame_submission_threshold` wraps it into the same submission frame a single-key wallet emits | ✅ |
| 80.7 | **A treasury's frame is byte-identical in shape to a single signer's**, carrying a verifying key and a 64-byte signature. A test verifies one under plain `ed25519_dalek` verification against the treasury account — so the sequencer, the replicas and the forced-inclusion path all accept it with no threshold-aware code anywhere | ✅ |
| 80.8 | **Below the threshold there is no signature**, and any qualifying subset produces one for the same account. Both asserted: one of three fails, and all three pairs of a 2-of-3 sign for an identical account id | ✅ |
| 80.9 | **Each signer commits a fresh nonce to the message being signed.** A nonce committed to one message cannot be reused for another, which is what stops an untrusted coordinator swapping the intent between rounds — the same property the Solana custody path relies on | ✅ |
| 80.10 | **The treasury ceremony is `zyn-dkg treasury`, the same tool and the same relay as the vault's.** It runs the identical three rounds over Ed25519 instead of RedPallas, so no box ever holds more than one share and the relay holds none — the property that made the vault ceremony trustworthy, reused rather than reasoned about again | ✅ |
| 80.11 | **The relay is ciphersuite-blind.** It keys round-one frames by reading the identifier straight off the frame instead of decoding the package, so one relay serves a Zcash ceremony and an Ed25519 one without knowing which it is looking at. It previously decoded as `Zcash`, which would have silently rejected a treasury ceremony | ✅ |
| 80.12 | **Every participant prints the account and they must match.** A ceremony that did not agree produces different accounts, and the tool says so — the check is cheap, and the failure is otherwise invisible until a signature does not verify | ✅ |
| 80.13 | **In-memory provisioning stays available for a treasury an operator holds alone**, but the distributed ceremony is the default for anything real. Generating three shares on one machine means that machine saw all three | 🔒 |

## 81. Keep endorsing until it settles, not until you are told it cleared (14 Sep 2026)

§63 made signers re-offer an endorsement instead of sending it once. It stopped
re-offering the moment the sequencer answered `clears = true` — and that is a
statement about the sequencer's memory *at that instant*, not about the chain.

| # | Decision | Status |
|---|---|---|
| 81.1 | **A signer re-offers until it has seen the anchor settled in its own verified ledger.** The sequencer's "clears" is a claim about signatures it holds in memory for an in-flight anchor; §63.1 already established that a restart drops them. A signer that stopped on the first "clears" never offers again, leaving the rest of the set permanently below threshold | ✅ |
| 81.2 | **This is §63's own failure one level up**, and it cost chain 11 an eight-hour stall before anyone noticed — the re-offer logging made the chain look busy while nothing settled. The fix hid its own gap: every signer was visibly working, on anchors that could never clear | ✅ |
| 81.3 | **Diagnosis came from counting, not reasoning.** signer1 named the stalled anchor seven times and signer2 exactly once — the one time it endorsed and was told it cleared. That asymmetry is the whole bug, and it was visible in a `grep -c` | ✅ |
| 81.4 | **The signer trusts only its own ledger for "settled."** Anything else is asking the party that may have lost the signatures whether it still has them | 🔒 |
| 81.5 | **The backlog is bounded at eight.** Re-offering is idempotent and cheap, but an anchor that never settles would otherwise be offered forever; eight covers a restart mid-flight without letting one stranded anchor grow the list without limit | ✅ |
| 81.6 | **A restart of any one signer is the field remedy**, because §63.3 re-seeds `outstanding` from the head anchor at startup. That is what recovered chain 11 here | ✅ |

## 82. Funding the vault is not a deposit (14 Sep 2026)

A chain that has never taken a deposit cannot anchor: an anchor is a vault
self-send and costs a ZIP-317 fee, and the vault holds no note to spend. So
mainnet's first act has to be putting ZEC behind fees — and doing that wrongly
is worse than not doing it.

| # | Decision | Status |
|---|---|---|
| 82.1 | **Shielded funds arriving with no Zyn memo are fatal to the deposit pass**, by design: crediting the rest "would attest backing that includes money nobody can claim". So a plain send to the vault address does not fund the chain — it stops the bridge and waits for a human to decide who the money belongs to | ✅ existing |
| 82.2 | **The operator top-up is a `ZYN` memo naming the all-zero account.** Classified as held for fees and never credited, so `supply` and `xzec_backing` stay equal and nobody is diluted by the fee reserve | ✅ |
| 82.3 | **`/api/topup` builds that memo rather than asking anyone to type it.** The memo is `"ZYN" ‖ version ‖ 32 zero bytes` — valid UTF-8 and therefore expressible as the string the send path takes, but not something a person can enter by hand. A funding step that depends on hand-crafting binary is a funding step that eventually halts the bridge | ✅ |
| 82.4 | **`memo::TOPUP_ACCOUNT` and `memo::is_topup` name the intent** instead of leaving `[0u8; 32]` at call sites, where it reads like a placeholder rather than a decision | ✅ |
| 82.5 | **Minimum useful top-up is `0.0002` ZEC** — one anchor's self-send plus its fee. Send more so later anchors need no second transfer; the surplus is held, not credited, and is recoverable only as fees | 🔒 |

## 83. Treasury 26460 exists (14 Sep 2026)

| # | Decision | Status |
|---|---|---|
| 83.1 | **`Treasury 26460` is `b3e4c493bb79349270260f7d2e2882b4f1a736348935493331e18c95098cfe99`**, a 2-of-3 threshold account, pinned as `swapvm::types::TREASURY_26460`. §66.17's placeholder is resolved and the `0xca7e…` vectors are dead | ✅ |
| 83.2 | **Provisioned by the vault's own ceremony over Ed25519** (§80.10): one relay, three participants, three rounds. All three independently derived this account and printed it; a ceremony that did not agree would have produced three different ones | ✅ |
| 83.3 | **One share per machine, verified after the fact** — Mac, Air and the Windows box hold identifiers `01`, `02`, `03` respectively, one `share-*.bin` each, and the relay held none. Counting the shares afterwards is part of the ceremony, not an optional check | ✅ |
| 83.4 | **The pin is tested against the ceremony's output.** A transcribed 32-byte constant can be transcribed wrongly, and every official collection address derives from this one — so the test asserts the exact hex rather than trusting the paste | ✅ |
| 83.5 | **Both official collections' addresses can now be computed**, which unblocks the 333 and the 4,444. What remains for them is naming, not cryptography | ✅ |

## 84. Zyn is a programmable execution layer, not yet an L2 (16 Sep 2026)

| # | Decision | |
|---|---|---|
| 84.1 | **Zyn is positioned as "the programmable execution layer for Zcash", not as a Zcash L2.** Zcash provides private money and settlement; Zyn provides the environment around it — trading, liquidity, lending, payments. The L2 claim is a destination, stated as one: *evolving toward Zcash-settled rollup architecture* | 🔒 |
| 84.2 | **Because the L2 claim invites five questions Zyn currently loses.** How does it inherit Zcash security? It does not. Where are state roots verified? By its own replicas, not by Zcash consensus. Who controls bridged ZEC? A 2-of-3 vault whose three shares are all the operator's. What does Zcash verify? That a transaction carrying 32 bytes exists. Anchoring proves data existed at a time; it does not make Zcash validate a transition. Answering those honestly gets Zyn called a sidechain — correctly | 🔒 |
| 84.3 | **The 15 Sep reorg is the operational proof.** Anchoring stopped for a day because Zcash testnet reorged, and recovery was an operator setting an environment variable and restarting a sequencer. Separately, `STATE_VERSION` 10→12 required coordinated replacement of the sequencer *and* every replica. Both are appropriate for what Zyn is; neither is what "L2" promises | 🔒 |
| 84.4 | **What Zyn does have is verifiability without security inheritance, and that is worth claiming precisely.** `zyn-replica` reconstructs and checks the chain from Zcash and DA alone, holding only a viewing key; `forced.rs` proves a user can exit without the sequencer's cooperation. Anyone can check Zyn is honest — they cannot make Zcash enforce it. That is more than "we anchor a hash", and less than a rollup. Say exactly that | 🔒 |
| 84.5 | **The trust-minimisation ladder, ordered by what is actually built.** v0 sequencer + ZEC integration ✓. v2 state commitments on Zcash ✓ — anchoring works. **v1 independent validators is the real gap and the next step**: all three signers are the operator's, so one replica endorsing from someone outside moves the claim from "operator asserts" to "operator and a stranger agree". Then v3 trust-minimised bridge, then v4 validity or fraud verification. Only after v4 is "L2" defensible | 🔒 v0/v2; ⏳ v1 |
| 84.6 | **Positioning is audience-matched, which is not a hedge.** To Zcash engineers, grants and investors: "programmable execution layer for Zcash, evolving toward Zcash-settled rollup architecture." To consumers and hackathon judges: what the apps do — the Colosseum copy says "a shielded layer anchored to Zcash" and never says L2. Both are true; they answer different questions | 🔒 |
| 84.7 | **MagicBlock is a conceptual reference, not a template.** It accelerates Solana programs that already exist and have users; Zyn adds an application environment Zcash does not have. That is a larger opening and a harder go-to-market — the same fact read twice — so it should not be quoted as pure advantage | 🔒 |
| 84.8 | **This makes disclosure routine rather than awkward.** Under "L2", saying the ZYN pool's liquidity is treasury-withdrawable (§71.27) reads as an admission. Under "execution layer, progressively minimising trust", it is ordinary precision about where the trust currently sits. The frame is better because it is true | 🔒 |

## 85. A secondary market on Zyn, priced in ZEC (16 Sep 2026)

| # | Decision | |
|---|---|---|
| 85.1 | **The Cave secondary market is denominated in ZEC.zy, not in a Cave token.** The VM already trades any asset for any asset, so an item-for-ZEC offer needs no new opcode. Introducing a marketplace currency would add a price the buyer has to acquire before they can bid, and a second thing to trust; ZEC is the unit the collection is already priced, funded and floored in | 🔒 |
| 85.2 | **Secondary trading is resting offers, not an order-matching engine.** `PlaceOffer` escrows the item into the keyless `POT_OFFERS` pot, so a listed item is gone from the maker before any taker arrives and gone from escrow after the first one takes it — no double-sale, and no custody by the market. `expires_at_epoch` of `u64::MAX` is the rests-forever case and must be deliberate. Resting offers are public over the RPC, because an offer nobody can see is one nobody can take. Built through every layer but the browser: VM, `zyn-rpc`, the agent's `/api/offers`/`/api/offer`/`/api/take`/`/api/cancel-offer`, and a two-wallet place-then-take demonstrated live in `zyn-settlement-lab` | ✅ VM + RPC + agent |
| 85.3 | **The floor is what makes a secondary market honest here.** Items in a collection share one redemption pool and any holder may redeem for `pool / outstanding`, so that quotient is a price floor: below it, buying to redeem is free money. The 90% of every primary payment that funds the collection (§64.17) is what creates it. The market therefore opens above zero by construction, which is the anti-rug claim stated as arithmetic rather than as a promise | 🔒 |
| 85.4 | **A secondary market's real dependency is the deposit front door, not the exchange.** Trading priced in ZEC is only better UX than sending ZEC hand-to-hand if a buyer can get ZEC *into* Zyn without ceremony. Per-account deposit addresses are issued and stable over `OP_DEPOSIT_ADDRESS`; what is missing is a general deposit UI reachable by someone who has not installed Nap Wallet. Until that exists, a marketplace UI would front a market most visitors cannot pay in | 🔒 |
| 85.5 | **The launch-policy options stay.** `QuantityPolicy`'s three variants and `IdentityPolicy`'s three are the creator's menu for future Cave collections (§64.1-64.3), not a decision to be collapsed to whatever the 4444 picks. Choosing ConcealedShuffle for Collection Z selects that launch's setting and removes nothing | 🔒 |

## 86. Collection Z's manifest (16 Sep 2026)

| # | Decision | |
|---|---|---|
| 86.1 | **Collection Z is 4,444 items across three species in one fixed order: Mole `0..=2221`, Owl `2222..=3554`, Moth `3555..=4443`.** The species were generated separately; the collection is one index space. The order is arbitrary but not revisable — `item_address(ADDRESS_SCOPE_V1, CAVEZ, index)` derives from the index and the manifest root commits to it, so reordering the species after publication renames every item | 🔒 |
| 86.2 | **`CAVEZ` is `024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e`**, derived from `TREASURY_26460` and the symbol. Item 0 is `ae983699…` and item 4443 is `7765c295…`. These are pure derivations that do not depend on the art, so they are pinned in a test (§83.4: a transcribed constant can be transcribed wrongly) | ✅ |
| 86.3 | **The manifest root is `02b01b16da58e0d71ae27099daba30a0a7936cdd852ee325853f679886dd8f19`** over 4,444 entries. This is what §64.16's `cave-sale-terms/v1` binds and what every allocation receipt proves inclusion against. It is a function of the collection address and of every item's index and content, so it moves if the art moves, the species order moves, or one attribute changes — which is what makes publishing it worth anything | ✅ |
| 86.4 | **Python builds the content hashes; Rust derives the identity.** `build_collection_z_manifests.py` reproduces the frozen `zyn.manifest.v1` preimage so the collection can be prepared without a build, but nothing reimplements `item_address` or the Merkle tree — `zyn-cave-manifest` takes those from `zyn-vm` and `cave-randomness`, the same code `AllocationPolicy::validate` checks against. A root that comes back at all has already had all 4,444 item ids re-derived and checked | ✅ |
| 86.5 | **Three adaptations separate this from the 333's builder**, and they are the whole difference: one global index across three directories; the image hash read from `checksums.sha256` rather than inline, still verified against the bytes on disk; and attribute values stringified, because `Canvas Row: 1` is an integer in the source and UTF-8 in the preimage. Left as an int it raises rather than silently differing | ✅ |
| 86.6 | **`name` keeps its species-local number and is not identity.** "Zyn Mole #0001" is item 0 and "Zyn Owl #0001" is item 2222; display text and committed index differ on purpose, and only the index is committed | 🔒 |

## 87. The deposit front door (16 Sep 2026)

| # | Decision | |
|---|---|---|
| 87.1 | **A deposit address has to belong to someone, so the front door cannot remove the key.** The address is derived from the depositor's own Zyn account; without a key there is no account, and crediting a stranger later would make this a custodial promise rather than a deposit. The page therefore asks for a wallet rather than generating one, and says why | 🔒 |
| 87.2 | **The door is at `cavebro.win/deposit`**, alongside `/whitelist`, because that is where buyers already arrive. Same staging shape: `deposit.html` → `deposit/index.html`, a bare path served from a directory index with no redirect rule | ✅ |
| 87.3 | **`zyn_depositAddress` is added to the wallet provider as a read**, returning `{ address, memo }` from the agent's existing `GET /api/deposit-address`. `zyn_account` already leaked the same address as a field; a named method makes the intent legible instead of piggybacking | ✅ |
| 87.4 | **`memo: true` is an alarm, not a detail.** It means the node could not issue a per-account address and the shared vault is being shown, which credits nobody without `ZYN1:<account>` attached — and no exchange will attach it. The page shouts once, here, and nowhere else. An address displayed without that warning is how money arrives belonging to no one (§71.39, A79) | 🔒 |
| 87.5 | **The custody disclosure sits on the page, in the open.** Deposited ZEC is held by the threshold vault; that is custody and the page says so, alongside what is *not* being trusted — the address derives from the depositor's key, vault holdings are public, and an exit is a chain operation rather than a support request. §84.8: under "execution layer, progressively minimising trust" this is precision, not an admission | 🔒 |
| 87.6 | ~~The public JSON-RPC deliberately does not carry `zyn_depositAddress` yet.~~ **Superseded the same day by §88.1.** The reasoning held — issuing writes to the scan register, so an *unauthenticated* method is an unbounded write by strangers — but the conclusion did not: the method can be signature-gated instead of withheld, and the browser wallet can sign | ↩ |

## 88. Browser first, and a QR on every surface (16 Sep 2026)

| # | Decision | |
|---|---|---|
| 88.1 | **`zyn_depositAddress` is on the public JSON-RPC, gated by a signed read** — `OP_DEPOSIT_ADDRESS_SIGNED`, whose payload is `OP_ACCOUNT`'s byte for byte. §87.6 was right that an unauthenticated method is an unbounded write by strangers into the scan register, and wrong that the answer was to withhold it: a signature bounds asking to people who hold a key, which is everyone entitled to an address anyway. Both ops answer through one shared body, so arriving signed or unsigned cannot be told apart from the reply | ✅ |
| 88.2 | **The browser is the first-class wallet for this, not a fallback.** `apps/nap-web` already derives a Zyn identity in the browser from a BIP-39 phrase under the normative `nap.zyn.ed25519.v1` derivation, so the same phrase restores the same account on desktop. Nothing new had to be invented to give a keyless visitor a key — §87.1's requirement that an address belong to someone is met by a wallet that already exists | ✅ |
| 88.3 | **The deposit address is shown in its own card, never beside the account id as an alternative.** They are different things: the account receives Zyn items from Zyn users, the deposit address receives *Zcash*. ZEC sent to an account id is sent to a string that is not an address. Separate cards, separate labels | 🔒 |
| 88.4 | **A missing deposit address fails closed and says so.** An address that is wrong, stale, or someone else's is worse than none, so on any error the field reads `Unavailable` and the QR is cleared rather than left showing the last one | 🔒 |
| 88.5 | **One QR encoder, shared.** `apps/shared/qr.js` is byte-mode, all four levels, automatic version, all eight masks scored. The desktop wallet and the extension popup already had QR from the Rust `qrcode` crate — the popup *is* the wallet page — and `nap-web` has the npm package behind its bundler. What had no encoder was cavebro.win, which has no bundler and cannot take a dependency, so the shared file is staged there as a plain module | ✅ |
| 88.6 | **The encoder is verified against `qrcode` module for module**, at every level, across lengths that straddle a version boundary and on a real unified address. A QR encoder is the kind of code that looks right and is wrong: three bugs were caught this way that no property test would have found — the finder separator drawn as part of the ring, the format bits placed least-significant-first, and bit 7 of the second format copy missing from the row because it also lands on the dark module | ✅ |
| 88.7 | **Rule 4 of the mask penalty follows the reference encoder, not the prettier reading.** The specification's "smallest k such that the proportion deviates by 5k%" is ambiguous just above 50%: a symbol 50.5% dark scores 0 under a floor reading and 10 under a ceiling one, and the difference changes which mask wins and therefore every module. Agreeing exactly with a scanner-tested encoder is worth more than either reading | 🔒 |
| 88.8 | **With no memo the QR is the bare address; with one it is the ZIP-321 request.** A scanned address alone, where a memo is required, produces a payment that credits nobody — so the picture becomes `zcash:<addr>?message=…&memo=…` and the caption names the wallets that honour it rather than implying any scanner will do. The JavaScript and Rust builders of that URI are pinned against each other in a test, because a memo that decodes to nothing looks exactly like a memo that was never set | ✅ |

## 89. Signer 3 was down for 36 hours and nothing said so (16 Sep 2026)

| # | Decision | |
|---|---|---|
| 89.1 | **The alert was real and stale at the same time.** `LINEAGE break at epoch 0: StaleBase` was signer 3 meeting the reset chain with the pre-reset base root. Chain 11 itself was healthy throughout — anchoring every ~3 minutes, both Mac signers endorsing, certificates clearing — because two of three is the threshold. Checking the chain before the alerter is what separated the two | ✅ |
| 89.2 | **A threshold that still clears is exactly why a dead signer goes unnoticed.** Signer 3 stopped at 06:33 on 15 Sep and nothing reported it for 36 hours: the sequencer sees a cleared certificate and has no opinion about who is missing from it. Redundancy hides its own erosion, and 2-of-3 with one silently gone is 2-of-2 with no margin | 🔒 |
| 89.3 | **Three things had to be wrong at once, and each was a separate deploy that skipped a machine.** Signer 3 held the *original* 7 Sep base root (two resets behind), its binary was from 12 Sep (pre-`STATE_VERSION` 12, so the v12 base state failed to decode), and the WSL distro holding it was not running. §71.34 says a version bump is a coordinated replacement of the sequencer *and every replica*; the third machine is the one that gets forgotten because it is the one nobody is logged into | 🔒 |
| 89.4 | **The old ledger is moved aside, never deleted** — `/var/zyn/signer3.prereset-*`. It is the only record of the pre-reset chain this machine verified and it costs nothing to keep | ✅ |
| 89.5 | **The keepalive could not heal itself, and still cannot run unattended.** `zyn-wsl-keepalive` was a one-shot at logon; it was terminated on 15 Sep and had no repeat trigger. It now repeats every five minutes with restart-on-failure, but the task is *Interactive only* — it runs as the desktop user and will not start without a logged-on session, which is why it did not fire even after repair. A detached `wsl --exec sleep infinity` holds the distro up now and survives SSH sessions, but not a reboot | ⏳ |
| 89.6 | **Making it durable needs the operator, not the agent.** "Run whether user is logged on or not" stores the account password, and handling that is not something to automate on someone's behalf. Until it is set, signer 3 survives sessions but not restarts | ⏳ operator |
| 89.7 | **The alerting replica was never signer 3 — it is the Air**, the fourth machine, and the alert's own arithmetic says so. Signer 3's journal has no boot between 15 Sep 06:34 and 16 Sep 19:02, so it emitted nothing that afternoon. Its history does pin the cadence: poll 20 s, remind 3600 s. `1263` and `1623` failures are 7.02 h and 9.02 h of failing, emitted ~16:45 and ~18:45, from a replica whose last good scan was height 4,355,068 at ~09:43 — the morning of the reset | ✅ |
| 89.8 | **A sleeping laptop is why the alerts then stopped, and why they will come back.** Failures count polls, not wall time, so a suspended replica freezes mid-incident rather than recovering; nothing after 18:45 means it slept, not that it healed. Waiting it out is not a fix — it resumes halted | 🔒 |
| 89.11 | **It was the Air, running `dev.zyn.signer2` with the original 7 Sep base root and a 10 Sep binary** — the same two faults as signer 3, on a machine that had been up 6 days and so never reloaded its config. Confirmed from its own log: `scanned to 4355068`, matching the alert exactly. Fixed the same way: current base root, current binary, ledger moved aside, restart. Verifying and endorsing again from epoch 107 | ✅ |
| 89.12 | **Two of the three signer identities live on one machine, and one identity lives on two.** `9ce67be6` and `27048d32` both run on the Mac; `27048d32` *also* runs on the Air; `cc158dc4` is the Windows box. Duplicating an identity gives that key redundancy but no independence — the same secret in two places doubles exposure while adding no signer. 2-of-3 survives losing any one machine today, but the set is three keys across two households' worth of trust, not three operators (A88) | 🔒 |
| 89.9 | **Four machines hold Zyn roles and only three were in the deploy list.** The Air is unreachable from here (no key, and a password is not something to handle on an operator's behalf), which is exactly how it came to be three resets and one `STATE_VERSION` behind. A machine nobody can reach from the build host will be the machine that is forgotten | 🔒 |
| 89.10 | **Mainnet custody drops to the bare threshold whenever the HP naps.** The mainnet sequencer expects three custodians on `8211/8212/8213`; `8211` is the Windows reverse tunnel and was closed while that box was in Modern Standby, leaving 2-of-3 with no margin. It returned on its own when the distro came back. Signing survived, but only because nothing else was down at the same time | 🔒 |

## 90. Monitoring the set, not the threshold (16 Sep 2026)

| # | Decision | |
|---|---|---|
| 90.1 | **Each signer is its own alert component.** `endorsement_health` fires when an anchor sits unendorsed, which cannot see the case that actually happened: with 2-of-3, one signer dies, every certificate still clears, every pass reports success. `Node` now records the anchor count at which each signer was last counted, and the sequencer reports one component per signer — so the quietest member of a set is the one that gets alerted about | ✅ |
| 90.2 | **A repeat endorsement counts as life even though it adds no weight.** Re-offering is how a signer heals a sequencer restart (§63); treating a repeat as silence would report the healthy case as dead | ✅ |
| 90.3 | **Liveness is in memory and never committed.** It is an observation about operators, not state the chain agrees on, and a restart legitimately knows nothing — reported as "never heard from since this node started" rather than as a failure | 🔒 |
| 90.4 | **Custodians are probed by TCP connect, which proves the listener and not the share.** It is the cheapest honest question, and it catches the failure that happened: a reverse tunnel dropped because its host slept, leaving a quorum that looks healthy until a signature is needed (§89.10). Anything stronger would mean asking a custodian to sign something, which is not a health check | ✅ |
| 90.5 | **The bot needed no change.** It renders whatever components the sequencer publishes, so per-signer and per-custodian rows appear in `/signers` for free. A monitor that has to be taught about each new thing is a monitor that will not be | ✅ |

## 91. Enterprise-friendly, as a direction rather than a claim (16 Sep 2026)

| # | Decision | |
|---|---|---|
| 91.1 | **The position is "confidential settlement with provable reserves and an exit no counterparty can block"**, written up in `docs/ENTERPRISE.md`. An institution can use neither a transparent chain (positions are trade secrets, often protected by law) nor an opaque one (auditors need proof). Almost nothing is allowed to be both | 🔒 |
| 91.2 | **It is an addition, not a pivot.** Consumer surfaces stay: they are the proof the thing runs, and every institutional item below makes them more trustworthy too. A full reposition would burn cavebro, the whitelist, the 4,444 and the voice, in exchange for a 6-18 month sales cycle | 🔒 |
| 91.3 | **Nothing may be said to an institution until the row carrying it is built.** The entire value of this position is that it survives diligence; asserting it early is the one way to lose it. `ENTERPRISE.md` is marked "direction, not a claim" for that reason | 🔒 |
| 91.4 | **The gap is operations, not cryptography.** `backing.rs`, `exitproof.rs`, forced inclusion and the separated vault FVK already argue the case. What a first diligence call kills the deal over is §89: a signer dead 36 hours, custody at bare threshold because a machine slept, a vault share on a laptop, a role on a host the build machine cannot reach | 🔒 |
| 91.5 | **Selective disclosure is the product to build next** — per-account viewing keys, scoped and time-boxed, producing an artefact an auditor verifies offline against an anchored root. Weeks, not months, and it improves the consumer product too | ⏳ |
| 91.6 | **Issuer control is per-asset and never protocol-level.** Regulated instruments need transfer restrictions and freeze; `SupplyPolicy::authority()` is already the right shape. A chain with a freeze key is not credibly neutral; an asset whose issuer has one is a regulated security behaving correctly. Every institutional chain that failed resolved this tension in the protocol's favour, one reasonable exception at a time | 🔒 |
| 91.7 | **Permissioning belongs at the gateway, never in consensus.** KYC on who may deposit, an allowlist of issuers, enforced at the edge with the chain seeing none of it. Identity in consensus destroys the only property worth buying | 🔒 |

## 92. What actually protects NFT ownership today (16 Sep 2026)

| # | Decision | |
|---|---|---|
| 92.1 | **NFT ownership is not private yet, and saying otherwise is the §21 claim §70.2 already retracted.** DA bundles ship raw `epoch-N.intents`; `wire.rs` encodes a transfer's sender, recipient and asset, and a mint's recipient. Anyone who can read a bundle can build the owner map. Verified on the live mirror: every bundle carries an `intents/` directory | ✅ observed |
| 92.2 | **What keeps that off the internet today is an undocumented network ACL, not the design.** `zyn-mirror` binds `0.0.0.0:8181`, the host firewall is inactive and `iptables` policy is ACCEPT — yet the port is filtered from outside while `8099` and `8098` answer. Something upstream blocks it. The ownership journal is therefore one cloud-firewall edit, or one host migration, away from being public, and nothing in the repo records the rule it depends on | 🔒 |
| 92.3 | **So the honest answer to "is ownership private" is "not yet, and here is exactly who can see it": the operator, and every authorised replica holding mirror access.** That is §70.1's chosen tier — operator-trusted, not operator-private — and it is a defensible answer. Claiming more is the one thing that cannot be walked back | 🔒 |
| 92.4 | **P0 comes before A85.** Selective disclosure is "prove this to one chosen party"; it is meaningless layered over a journal that already names every recipient. The disclosure audit is the prerequisite, and it is also the cheapest item on either list — mostly reading, plus a fixture that proves a leak is caught | 🔒 |
| 92.5 | **Once ownership is genuinely hidden, A85 is what turns privacy from a limitation into a feature.** "Prove you hold #0042, to this person, for this window, revealing nothing else" is the question collectors actually ask, and it is the same primitive an auditor needs (§91.5). One build, two audiences | 🔒 |

## 93. Sealing the two files that carry ownership (17 Sep 2026)

| # | Decision | |
|---|---|---|
| 93.1 | **The journal and the base state are encrypted, not withheld.** Serving them only to authorised verifiers would have worked and cost more: a mirror would need to authenticate, and the DA argument — "the cheapest hosting that answers `GET` will do" — would be gone. Encrypted, a host that cannot read a file can still keep it, so durability and permissionless *hosting* survive. What is given up is permissionless *replay*, which is the trade §70.4 asks to be stated rather than hidden | ✅ |
| 93.2 | **The public commitments stay public, and that is most of the verification.** `anchor.bin`, `certificate.bin`, `published.bin` and the index are untouched: a stranger can still follow the root chain on Zcash and check that the published leaves rebuild the anchored root. `published.bin` was already blind — a leaf is `H(record)` over a record carrying the holder's own blind — so nothing there needed changing | ✅ |
| 93.3 | **One key per artifact, derived from a root by HKDF over `chain \|\| label`.** Two things follow: a fixed nonce is safe, because nonce reuse only bites when a key is reused and no two artifacts share one; and sealing is deterministic, so republishing produces identical bytes and a mirror push stays idempotent. Per-artifact keys are also the shape selective disclosure needs — one epoch can be handed to an auditor without handing over the chain (A85) | 🔒 |
| 93.4 | **The label is the artifact's path, so a file cannot be opened under another label.** Moving an epoch's journal into a different epoch's directory makes it undecryptable rather than silently acceptable | ✅ |
| 93.5 | **The confidential set is an inverted allowlist.** Only the four named public commitments are published in the clear; anything else in a bundle, including artifacts added years from now, is sealed by default. A mistake then falls towards over-sealing, which is visible and fixable, rather than towards a quiet leak | 🔒 |
| 93.6 | **Not a coordinated replacement, unlike a `STATE_VERSION` bump (§71.34).** Sealed files carry a magic prefix; a reader opens what is sealed and passes through what is not, so publisher and verifiers can move in either order. With no key configured the publisher emits byte-for-byte what it always did, and a test asserts exactly that | ✅ |
| 93.7 | **A reader with no key is not a reader with a wrong key.** Meeting ciphertext without a key names the missing `ZYN_DA_KEY` and suggests verifying roots only; a wrong key reports a wrong key. The two have completely different fixes and must not present identically | ✅ |
| 93.8 | **A file that cannot be sealed is published unusable rather than in the clear.** The one failure this change exists to prevent is ownership leaving in plaintext, so the failure path produces a loud broken artifact instead of a quiet readable one | 🔒 |
| 93.9 | **This closes the public leak, not the operator one.** Ownership is now hidden from anyone holding a mirror; the operator and every verifier with the key still see it. That is exactly §70.1's operator-trusted tier, and it remains the honest answer to "is ownership private" (§92.3). Fresh per-item receiving accounts (P2) are the next step, and notes/nullifiers/ZK the destination | 🔒 |

## 94. One sleeping box, four alerts (17 Sep 2026)

| # | Decision | |
|---|---|---|
| 94.1 | **`mainnet unreachable` was true, and it is the same fault as the other three.** Mainnet Zebra, signer 3, mainnet custody share 03 and five reverse tunnels all live inside one WSL distro on the HP. When it is not running, every one of them is down, and each reports separately as though it were its own incident | ✅ |
| 94.2 | **The distro only runs while someone is running a command against it.** `wsl -l --running` lists it as absent, and the very next `wsl -d zyn -e …` starts it — PID 215 for the tunnel's ssh process is a process that began moments after init. Every diagnostic against that box boots it, which is why it kept appearing healthy while being asked and dead when not | ✅ |
| 94.3 | **The tunnel was never broken.** `permitlisten="127.0.0.1:8232"` is present on the right key, and a clean start establishes the forward and answers `getblockcount` immediately. The repeated "Started" entries with `NRestarts=0` and no matching "Stopped" are a journal recording a fresh boot each time, not a service crash-looping | ✅ |
| 94.4 | **This cannot be fixed from the build host.** A detached keepalive holds the distro only while the session that started it survives; the scheduled task that should hold it is *Interactive only* and will not start without a logged-on desktop. The durable fix stores a Windows account password, which is not something to automate on an operator's behalf (§89.6) | ⏳ operator |
| 94.6 | **Hetzner should not have been asking the HP for mainnet at all.** Contabo already runs a mainnet Zebra — up 6 days, and *ahead* of the HP's (3,486,441 against 3,486,394) — on a host that does not sleep. Hetzner's `127.0.0.1:8232` is fed by a reverse tunnel from the HP only because that is how it was first wired; nothing about the bot needs to change, only which host feeds that port | ✅ |
| 94.7 | **The missing piece is one line of permission, not a new tunnel.** Contabo already holds a working outbound tunnel to Hetzner; its key carries `permitopen` for 8181 and 8210 and no `permitlisten` at all, so a reverse forward is refused. Adding `permitlisten="127.0.0.1:8232"` to that one key, and `-R 127.0.0.1:8232:127.0.0.1:8232` to the existing unit, moves the whole dependency off the sleeping box | ⏳ operator |
| 94.8 | **The HP keeps its own mainnet Zebra, because its mainnet replica reads it locally.** Only Hetzner's *view* moves to Contabo, and the HP's `zyn-zebra-tunnel` is then disabled so the two do not compete for the same remote port | ✅ done 17 Sep |
| 94.9 | **Verified by the test the old wiring always failed: leaving it alone.** Hetzner held the listener and an advancing height for 4.5 minutes with no command run against the HP. Every previous "it works" was an artefact of the diagnostic itself booting the box (§94.2), which is why stability has to be measured by absence of attention rather than by a successful probe | ✅ |
| 94.5 | **The real answer is to stop putting these roles there.** A mainnet vault share, a signer and a mainnet node behind a per-user WSL distro on a desktop box is the arrangement §91.4 says diligence kills a deal over. The password checkbox is the stopgap; moving custody share 03 and signer 3 onto infrastructure that does not depend on a login is A88 | 🔒 |

## 95. Fresh receiving accounts (17 Sep 2026)

| # | Decision | |
|---|---|---|
| 95.1 | **Sealing hid ownership from the public; a fresh account per item hides it from the reader who has the key.** §93 stops anyone holding a mirror from building the owner map. It does nothing about the operator, or about the key leaking later — a reader who can decrypt sees `MintCollectionItem { recipient }` exactly as before. Minting to an account that was never used and never will be again is what survives that | ✅ |
| 95.2 | **Derived, never random.** Generating a key per item is the obvious implementation and it is unrecoverable: an item held by a key that lived only in a browser since cleared is an item nobody owns. §70.6 makes recoverability a release gate for exactly this reason, so receiving accounts come from the same BIP-39 seed and a phrase restores every one | 🔒 |
| 95.3 | **Its own branch, not another account index.** `zyn_signing_key` refuses an index that differs from the wallet's on purpose — there the index means *which wallet*, and two personas must not share an identity. Receiving accounts are a different axis, many per wallet, so they take their own salt `nap.zyn.receive.v1` with info `"receive" \|\| account \|\| index`. A test asserts no receiving account ever coincides with the wallet identity from the same seed | ✅ |
| 95.4 | **The gap limit is a promise the issuing side has to keep.** Discovery stops after 20 consecutive empty indices, like BIP-44 and for the same reasons. That makes "indices are issued in order and never skipped" a correctness requirement, not a style note: a wallet that jumps to index 50 over an empty 0..29 has created an item its own restore will not find. There is a test that demonstrates the orphaning rather than one that hides it | 🔒 |
| 95.5 | **Raising the gap later is safe; lowering it is not**, because a smaller gap can orphan an item a previous version could still reach | 🔒 |
| 95.6 | **The derivation is taken as a closure, so it exists in one place.** `discover` asks for `account_at(index)` rather than a seed, because the caller that can *sign* a probe is the wallet, not the scanner. An earlier draft passed the seed to both and quietly derived twice — which is precisely how a restore ends up scanning the wrong branch while every test still passes | ✅ |
| 95.7 | **An unreachable node must not look like an empty wallet.** A failing probe stops the scan and returns the error; reporting "no holdings" would tell someone their items were gone | ✅ |
| 95.8 | **A legacy raw-key wallet is refused rather than accommodated.** It has no seed to branch from, and inventing one would produce accounts no restore could find — the exact failure the scheme exists to prevent | 🔒 |

## 96. The gateway was the owner map nobody sealed (17 Sep 2026)

| # | Decision | |
|---|---|---|
| 96.1 | **The browser wallet fetched art for every holding on every render, with no consent and no warning.** One page load handed the gateway the complete list of what that IP holds — an owner map that never touches the chain and that §93 and §95 do nothing about. The desktop wallet has always kept this behind an explicit toggle; the two had simply diverged | ✅ fixed |
| 96.2 | **The disclosure goes on the control, not in a help page.** The label names what is given away — IP, which items, and when — because a consent that does not say what it costs is not consent. It sits beside the items it governs, where the holder can see what the choice is about | 🔒 |
| 96.3 | **A browser that refuses storage stays off.** Unremembered consent is not consent, so a `localStorage` failure reads as "off" rather than defaulting to on and re-asking. Tested | ✅ |
| 96.4 | **Bucketing was considered and rejected as over-engineering.** Grouping blobs so a fetch reveals one of ~35 would weaken the link without closing it, leaks across repeated fetches anyway, and would put the two wallets on different models. Informed opt-in is what the desktop wallet already chose; matching it is worth more than a second mechanism | 🔒 |
| 96.5 | **This is the shape of the remaining gaps: off-chain and mundane.** Consolidating a receiving account into the identity, listing an item on the public order book, funding a receiving account from an attributable deposit, and the invitation→account binding in the registry each rebuild part of the map. None is cryptographic, all are reachable, and each needs naming rather than assuming §93 and §95 covered it | 🔒 |

## 97. Narrowing who can link a Zcash payment to a Zyn account (17 Sep 2026)

| # | Decision | |
|---|---|---|
| 97.1 | **The link is deliberate and load-bearing, so it cannot simply be made secret.** `deposit_index(account) = SHA256("zyn.deposit.index.v1" \|\| account)` is derivable on purpose: it is what lets a verifier holding the vault's viewing key confirm a credit's money really arrived, instead of taking the operator's word. Hiding the derivation and keeping verifiable deposits are the same mechanism pulling in opposite directions | ✅ |
| 97.2 | **So narrow who holds the key, not what the key derives — and make that safe.** A replica without the viewing key skips the backing check (`if let Some(backing)`) and endorses anyway. Narrowing naively would let a threshold be met entirely by signers who cannot check a credit at all, settling a fabricated deposit with nobody having looked. `SignerSet::requiring_backing` records the subset and `Certificate::verify` refuses with `InsufficientBacking` | ✅ |
| 97.3 | **`certificate_clears` and `verify` had to learn the same rule.** The sequencer reporting an anchor as ready that `verify` will refuse would send an operator to Zcash before finding out | ✅ |
| 97.4 | **Nothing on any public chain carries the link.** A Zcash observer sees a shielded payment to a diversified vault address; a Zyn observer sees a sealed journal. The exposure is exactly "who holds the vault FVK", which is a deployment question — and the honest claim is to name that set rather than to claim there is none | 🔒 |
| 97.5 | **One receiving account per *collection*, not per item.** Per-item costs a signed read per item on the most-viewed screen and twenty more before a restore shows anything — an O(N) tax to hide which of three moles is which from a reader who already knows three were bought. Deriving from the collection rather than a counter also deletes the gap limit outright: no sequence to issue in order, nothing to skip, nothing to orphan. The indexed form stays for a case that will pay for it | ✅ |
| 97.6 | **Seal cap 60 s → 5 s.** Under batch clearing a swap waits for the seal, so the quiet-period worst case *was* a minute before a trader learned their price. Traffic-driven seals already run near six seconds, so anchor cadence and Zcash fees barely move; what changes is the worst case, which is the number a user actually feels. The fairness batch clearing buys comes from batching, not from the batch being long | ✅ |
| 97.7 | **Operator-side proving is not the thing rejected earlier.** §91/§96's argument was against putting a prover on a user's phone — Zcash's UX tax, imported into the thing Zyn differentiates from. A proof that a note of amount X landed at the diversifier derived from account A, produced once per credit on a server, has no UX cost and would keep verifiable attribution while breaking the link. That is the upgrade path, not client-side ZK | 🔒 |

## 98. The last two off-chain links (17 Sep 2026)

| # | Decision | |
|---|---|---|
| 98.1 | **An item operation is signed by the account that holds the item, not by the identity.** Once items are received into a per-collection account (§97.5) the identity cannot escrow them, so signing with it would simply fail — naming the collection is a correctness requirement and the unlinkability is a consequence rather than a feature bolted on. A public order book must disclose that *this account* holds the item; it need not disclose who that is | ✅ |
| 98.2 | **The whitelist registry stores a commitment, never the account.** It only ever needs to answer "the same account as last time?", which a commitment answers exactly as well. Storing the account made the file an owner map — invitation to account, beside a KV store already mapping handle to invitation | ✅ |
| 98.3 | **And it is honest about what that does not stop.** An operator holding both the registry *and* the journal can test every account the journal names against every entry and rebuild the map. What it stops is this file *alone* being the map — and this is the file that gets copied, backed up and passed around | 🔒 |
| 98.4 | **The salt is not secret from whoever holds the file, and cannot be**, or the commitments could not be checked. Its job is narrower: to stop a stolen registry being tested against a list of accounts obtained somewhere else. It persists with the file, and a test asserts the round trip — a lost salt makes every existing binding uncheckable and every invitation silently rebind | ✅ |
| 98.5 | **Naming a collection the wallet cannot derive an account for is an error, not a fallback to the identity.** Falling back would place an offer from an account that does not hold the item, and the refusal would arrive from the VM with none of this context attached | 🔒 |
| 98.6 | **`collection` is optional, because everything minted before §97.5 sits on the identity** and must keep working | ✅ |

## 99. What an application layer adjusts (17 Sep 2026)

| # | Decision | |
|---|---|---|
| 99.1 | **The positioning retroactively justifies the largest recent choice.** Encrypting the journal costs permissionless replay, which for a rollup is close to disqualifying and for an application layer is an ordinary product call: user privacy over stranger-verifiability. Same code, and §70.4's trade stops being awkward. The single sequencer stops being something to apologise for with a roadmap and becomes how applications normally work — the question moves from "is this decentralised" to "what can the operator do to me", which `backing.rs`, `exitproof.rs` and threshold custody answer with specifics | 🔒 |
| 99.2 | **The product hole is the roadmap.** The chain anchors every few minutes and the wallet works; there is no screen where money changes hands. Under rollup framing that is one item among many; under this framing it is the whole thesis | 🔒 |
| 99.3 | **So the trust-minimisation backlog drops below the product.** A95 and A77 are marked deprioritised rather than dropped: they are rollup credentials, valuable but not what makes an application layer valuable. Zero days on them before money moves | 🔒 |
| 99.4 | **Keep the machinery, change the words.** "Forced inclusion with a grace window" becomes "you can always get a transaction in, through Zcash"; "trustless exits" becomes "you can always leave, even if we disappear" — which is the anti-rug promise in the language of the person it protects. **Anchoring keeps its name**: Zcash-settled is the differentiator and the word does real work | 🔒 |
| 99.5 | **The scoreboard changes.** Not TVL, decentralisation and finality but wallet installs, items sold, trades and users — none of which can be measured today, because nothing has sold. That is a sharper indictment than any technical gap | 🔒 |
| 99.6 | **`ENTERPRISE.md` is reworded from rollup framing to receipts framing**: "we hold your ZEC, and here is exactly what stops us abusing it". Rollup language invites rollup diligence — permissionless verification, decentralised sequencing, fraud proofs — which Zyn does not have and is not currently pursuing. The risk of sounding like "a custodian with extra steps" is answered by `backing.rs` and `exitproof.rs` rather than by rhetoric, so those lead | ✅ |
| 99.7 | **The checkout exists.** `zyn-sale` joins the registry, the order book and the treasury signing path that had all existed separately and never met. Payment is ZEC to the deposit address of the buyer's *collection receiving account*: no memo, the journal names an account with no history, and money and item land in the same place so nothing has to be moved afterwards — a move being exactly what would write the link back in | ✅ |
| 99.8 | **The buyer proves their own payment.** Balances are private, so the service relays the buyer's signed read rather than being given a power to read any account. It holds no key of theirs and learns nothing it was not handed (§74.23) | ✅ |
| 99.9 | **A serial missing from the manifest is refused, never minted against a zero commitment.** An item committing to nothing could not be corrected afterwards. The service reads the same `content.json` the manifest root was derived over, and a test loads the real 4,444 and pins item 0's commitment | ✅ |
| 99.10 | **The allocation policy is still the one undecided value**, and `zyn-sale` refuses rather than inventing one: quantity policy and the two close heights must enter the `cave-sale-terms/v1` commitment first (§64.16). A checkout that allocated before the policy was published would make the fairness claim unprovable for every buyer before it | ⏳ |

## 100. The close is announced; the reveal is derived (17 Sep 2026)

| # | Decision | |
|---|---|---|
| 100.1 | **Collection Z's quantity policy was decided on 9 Sep and is not open: `Weighted`, 1 item 80% · 2 items 19% · 3 items 1%** (`docs/2026-09-09-genesis-collection.md`). Every open ticket reserves three until its draw lands (§64.7). The `Fixed { items: 1 }` proposed on 17 Sep was a re-decision of something already recorded, and is withdrawn. One payment buys one ticket and the *count* is drawn, so the worry about buying three for the price of one never applied | ✅ |
| 100.2 | **The operator names the close; the reveal follows mechanically from it.** How long to run a sale is a judgement about demand and belongs to a person. Which items go to whom does not. An operator free to pick the reveal target *after* seeing the final ticket ledger could try candidates until one produced a mapping they liked, and nobody could tell — so the reveal is `close + offset`, with both offsets committed before any deposit arrives | 🔒 |
| 100.3 | **The drand leg is derived too, from the announcement's own round.** Binding only the Zcash block would leave the reveal resting on one source; §64.11 wants two independently sourced facts. A round ~2 hours ahead of the announcement cannot be ground against, because drand's output does not exist yet | ✅ |
| 100.4 | **A policy is one form or the other, and saying both is refused.** An announced-close policy that also fixes reveal targets is exactly the attack wearing the safe shape, so `validate` rejects it rather than letting one quietly win | ✅ |
| 100.5 | **A close must give notice, and notice is measured from the announcement.** A close naming a height already reached is a door shut and described afterwards. `min_notice_blocks` — 1,152, about a day — makes that a refusal rather than a judgement call | ✅ |
| 100.6 | **The honest limit: nothing here proves *when* an announcement was written.** The type enforces every ordering it can see, but a verifier needs the publication timestamped by something outside the operator's control. That is the publication service §64.4 already requires, and the fairness claim is incomplete without it | 🔒 |
| 100.7 | **`POLICY_VERSION` is now `cave-allocation/v2`.** The canonical bytes changed, so the digests changed; putting the version inside those bytes is what stops a v1 and a v2 digest being confused and keeps an older published commitment meaning what it meant. The two pinned vectors were re-pinned deliberately rather than loosened | ✅ |
| 100.8 | **Chosen shape for Collection Z: reveal +100 blocks (~2 h), drand +2,400 rounds (~2 h), notice 1,152 blocks (~1 day).** Long enough past the close to be clear of ordinary reorg depth, short enough that buyers are not left waiting on nothing | 🔒 |

## 101. A commitment the operator cannot backdate (17 Sep 2026)

| # | Decision | |
|---|---|---|
| 101.1 | **The announcement is stamped onto Zcash, because nothing inside a document can establish its own age.** §100.6 left the fairness claim resting on the operator not having written the announcement late. A `ZYP` memo carrying its digest turns "we announced on the 3rd" into a block height nobody can move | ✅ |
| 101.2 | **Deliberately the same shape as an anchor**, so the scanner needed one condition changed rather than a new path: tag, version, digest, zero padding — recorded, never credited. It is money only in the sense that a postmark is | ✅ |
| 101.3 | **`resolve` now takes the proof and refuses when the claim and the chain disagree.** The announcement's own `announced_at_zcash_height` is the operator's account of when they acted; the proof is Zcash's. Notice is measured from the proof, so an announcement written late and backdated to make a short notice look long fails here instead of passing quietly | ✅ |
| 101.4 | **`resolve_unproven` exists and is named so that using it is a decision.** It is correct while *building* an announcement, which has no proof because it has not been published yet, and wrong for accepting one. An `Option<Proof>` would have made the dangerous case the easy one | 🔒 |
| 101.5 | **The padding is strict, as it is for anchors.** A memo carrying a digest and something else is not a publication: reading one out of it would let a single transaction be claimed as publishing several different things | ✅ |
| 101.6 | **The three vault-stamped namespaces are tested against each other.** A deposit read as a publication is money never credited; a publication read as a deposit is worse. `ZYA`, `ZYP` and `ZYC` are asserted mutually exclusive rather than assumed to be | ✅ |
| 101.7 | **What remains outside the code**: someone must actually broadcast the memo and hand the verifier the txid and height. The engine fetches nothing by design — the same arrangement as the drand and Zcash reveal evidence it already takes | ⏳ |

## 102. The engine gets a caller (17 Sep 2026)

| # | Decision | |
|---|---|---|
| 102.1 | **A ticket is a confirmed Zcash payment, and nothing built one.** `cave-randomness` was complete and tested with zero callers, so a buyer could pay and no ticket existed to allocate over. `zynzapd::tickets` is the bridge: admission checks the amount is exactly the entry price, the confirmations are deep enough, the height is not past the close, and the outpoint is not already admitted | ✅ |
| 102.2 | **Admission is where the rules bite, so a ticket that passes has nothing left to argue about.** That is what lets allocation stay a pure function over the ledger. A near miss on the price is refused *here*, where it can still be refunded by hand, rather than at allocation where it cannot | 🔒 |
| 102.3 | **The order depends only on Zcash, never on when this process saw a deposit.** Height, then transaction index, then output index, then ticket id (§64.12). A test feeds the same deposits forwards and backwards and asserts the ledgers are identical, because a stranger replaying the allocation must arrive at the same sequence | ✅ |
| 102.4 | **The close is passed in, not read from the policy.** Under an announced close the policy does not know it, and a function that quietly admitted everything when the close was unknown would be the worst of both | 🔒 |
| 102.5 | **`SaleTerms` exists, because a policy says nothing about price, recipient or routing** — and those are what a buyer is trusting. Left in a web form they can change between two buyers without either knowing, which is the shape of the thing this collection is trying not to resemble. Repeated fields are checked against the policy, so two documents cannot describe different sales | ✅ |
| 102.6 | **Rounding in the split goes to the pool.** The floor is what buyers are trusting, so a spare zatoshi belongs on their side; a treasury taking the whole payment is refused outright | ✅ |
| 102.7 | **Collection Z's numbers are assembled and printed, never re-decided.** `zyn-cave-terms` repeats what was settled — the draw from 9 Sep, prices from §64.17, the root from §86.3, the close rule from §100.8 — and a test asserts they were transcribed rather than chosen. **`POLICY DIGEST 12da4f1b…`, `TERMS DIGEST 159b454e…`** | ✅ |
| 102.8 | **It prints and does not broadcast.** Publishing is a deliberate act; this is the thing you read before taking it | 🔒 |
| 102.9 | **`cavebro.win/deposit` is live** — the first of three days' work to actually reach a user | ✅ |

## 103. Corrections: three claims the code does not support (17 Sep 2026)

Review of the §84/§99 positioning against the implementation. §84 and §99 stand
as the record of what was decided and when; these rows correct what was claimed.
All three were verified in code before being accepted.

| # | Decision | |
|---|---|---|
| 103.1 | **"An application execution layer *built for* Zcash", never "the execution layer *of* Zcash".** "Of" implies protocol recognition or inherited security, and neither exists: Zcash does not know Zyn exists. The distinction is one preposition and it is the difference between a description and a claim | 🔒 |
| 103.2 | **"You can always leave, even if we disappear" is false, and it was mine.** §99.4 translated "trustless exits" into consumer language and made it stronger in the translation. `ExitProof` has **no consumer**: nothing on Zcash reads one and releases vault funds, and `confirm_withdrawal` requires a settlement signer. What a holder actually gets is *evidence of what they held*, verifiable against an anchored root without our cooperation. That removes "prove you had it", not "make them pay". Approved wording: **"Withdrawals depend on the Zyn-operated threshold custodians."** | ↩ |
| 103.3 | **Zyn does not inherit Zcash settlement finality.** Zcash finalises the anchor *transaction* and the 32-byte commitment inside it. It does not validate the application state that commitment stands for — replicas and signers do. "Zcash keeps custody and final settlement" was too broad and is corrected in `SPEC.md`, `zyn/src/lib.rs` and `ENTERPRISE.md` | ↩ |
| 103.4 | **Forced inclusion makes censorship detectable, not impossible.** `replica.rs` says it plainly — a `Censored` record is "the operator is refusing service, and this is the record of it". §99.4's "you can always get a transaction in" overstates it. Approved: **"Users can publish signed intents through Zcash, making censorship detectable."** The operator can still decline; what it cannot do is decline quietly | ↩ |
| 103.5 | **"Programmable" is ahead of the product.** `host.rs` states it outright: "Zyn today is a chain that runs ZynZap rather than a chain that runs applications." Applications compile into the node; permissionless deployment is out of scope in `SPEC.md`. Say "an application execution layer" and "a programmable architecture for ZEC applications"; do not say "anyone can deploy", "permissionless application platform" or "Zcash smart contracts" | 🔒 |
| 103.6 | **The product hierarchy is named, because it was implicit and kept being flattened.** *Cave is the first consumer application powered by Zyn. ZynZap/SwapVM is Zyn's first execution environment* — which `SPEC.md` already called "application #1" | 🔒 |
| 103.7 | **User-facing privacy copy was stronger than the backlog permits.** "Private by default", "private to your key", "known only to your wallet", "balances on Zyn are private" all read as *nobody* can see, when the truth is that a reader must sign and the operator and its verifiers can see the journal. Corrected in the wallet UI, ZynZap and Cave. §92.3's tier — operator-trusted, not operator-private — is the sentence that may be used | ✅ |
| 103.8 | **The lesson, which is the point of writing this down.** Every one of these came from translating an accurate technical statement into friendlier language and gaining a guarantee in the process. "Trustless exit" became "you can always leave"; "anchored to Zcash" became "final settlement". The translation is where overclaiming happens, and it happens most easily to whoever wrote the accurate version first. `docs/MESSAGING.md` exists so the approved wording is a lookup rather than a fresh translation each time | 🔒 |

## 104. The draw happens once, and orders look it up (18 Sep 2026)

| # | Decision | |
|---|---|---|
| 104.1 | **`sale::settle`'s per-order `allocate(n)` is the wrong shape for a concealed shuffle, and calling it anyway is how two buyers end up with results from two different draws.** `cave_randomness::allocate` takes the *whole* ticket ledger and returns one batch covering every ticket. Run per buyer, it would re-draw against whatever ledger existed at that moment — each result internally consistent, mutually contradictory, and undetectable until two people claimed the same serial | 🔒 |
| 104.2 | **So the draw runs once, after the reveal, and settling is a lookup.** `zynzapd::allocation::run` produces the batch; `serials_for` answers what an account was given. The closure in the sale service reads; it does not compute | ✅ |
| 104.3 | **A re-run over a changed ledger is a *different allocation*, not a continuation, and must be visible as one.** `agrees_with` compares assignment roots, which is the check to make before minting against a run you did not just produce. The test grows the ledger by one ticket and asserts the two runs disagree | ✅ |
| 104.4 | **A concealed collection refuses to draw before its reveal rather than falling back.** Passing no evidence must not quietly produce sequential behaviour — that assigns items before the thing meant to decide them exists | ✅ |
| 104.5 | **An account that did not enter gets an empty answer, not an error.** Not being in a draw is an ordinary fact; a caller that treats absence as failure reports the sale as broken to everyone who never entered it | 🔒 |
| 104.6 | **Fewer items than reserved is refused, not partially filled.** The reservation is what the buyer paid against, so a short set is a support problem rather than a sale | 🔒 |
| 104.7 | **Unallocated items are reported rather than inferred.** They roll into the open mint (§64.17), and the batch names them so nobody has to subtract two lists and hope | ✅ |

## 105. Gathering what the draw needs (18 Sep 2026)

| # | Decision | |
|---|---|---|
| 105.1 | **The tickets were already on Zcash; nothing had turned them into a ledger.** A Cave entry payment carries its own coordinates — txid, height, transaction index, output index, amount — and its memo names the Zyn recipient. `reveal::ledger_from` walks what the scanner saw and admits each one, so the ledger is derived from the chain rather than from anything this process remembers | ✅ |
| 105.2 | **A payment that cannot be a ticket is returned with its reason, never dropped.** Someone's money is in the vault and they are owed an answer; a sale that silently discards near-misses acquires a support queue nobody can reconstruct. Entries for another sale's terms digest are *skipped* rather than refused — they are not ours to answer for | 🔒 |
| 105.3 | **The beacon is verified before it is returned.** `cave-randomness` will not take a signature it has not checked against the pinned Quicknet key, and neither does the fetch. An unverified beacon is a number somebody sent you | ✅ |
| 105.4 | **The endpoint was wrong and every stubbed test passed.** A transport double proves the parsing and the verification; it cannot prove the address. `tests/drand_live.rs` fetches a real past round, verifies it, and flips a bit to confirm the verification is doing something. Ignored by default so an outage is not a failing build, and to be run before any reveal | ✅ |
| 105.5 | **The Zcash block hash is asked of a node, not derived from a header.** A Zcash header is not a Bitcoin header, and re-deriving the hash would be a second implementation of consensus that could disagree with the first. What this cannot establish is that the node is honest about which block is canonical — that is why you ask a node you run | 🔒 |
| 105.6 | **`Sighting` mirrors the scanner's type rather than importing it**, so this is testable without a scanner and a change to the scanner's shape is a compile error here instead of a silent mismatch | 🔒 |

## 106. Proving the sale pipeline composes (18 Sep 2026)

| # | Decision | |
|---|---|---|
| 106.1 | **Every link existed and none was called.** `Scanner::cave_payments` found entry payments with no caller; `reveal::ledger_from` admitted them with no caller; `allocation::run` drew with no caller. Each was tested in isolation, so the places where two links disagree about a type or an ordering had never been reached. `tests/sale_pipeline.rs` runs the sequence: a payment on Zcash becomes a specific serial in a specific account | ✅ |
| 106.2 | **The test that matters is not the happy path.** Three assertions carry it: no serial reaches two accounts; the result is identical whichever order a scanner reported the payments in, because two operators replaying one block range must hand the same buyer the same items; and a refused payment does not move anyone else's allocation, so an underpayment costs its sender and nobody else | ✅ |
| 106.3 | **Collection Z's policy moved from a binary into the library.** The draw needs the same policy the terms were published from, and a policy constructed twice is two policies whose digests differ silently. `cave_z` is now the single construction, with the published digests pinned in a test — `12da4f1b…` and `159b454e…` are unchanged by the move | ✅ |
| 106.4 | **The manifest rebuilds the committed root, and there is a test that says so.** The draw reads `content.json`; the terms committed to `02b01b16…`. If those ever disagree the draw runs over a different collection than the one published, which would be discovered by buyers rather than by us | ✅ |
| 106.5 | **The item id is derived, not read from the file.** It is a function of the collection and the index, and `manifest_root` checks it anyway — a file that disagreed would fail loudly there rather than quietly assigning someone else's item | 🔒 |
| 106.6 | **The `draw` command is not built, and a stub that prints the five calls it would make is not a build.** What blocked it was the policy being unreachable, which §106.3 fixes; what remains is a scanner, a Zcash reader and the announced heights, all of which are configuration. Written down rather than half-written | ⏳ |

## 107. The node gets a public face and keeps its door (18 Sep 2026)

Proof-of-build for the mainnet Zcash node, live at
`cave-node-status.pages.dev`, updating every 15s.

| # | Decision | |
|---|---|---|
| 107.1 | **The page was already right and the server did not exist.** `app.js` fetched `/api/status` and never Zebra JSON-RPC; the README described the helper as future work. Nothing needed rewriting in the frontend — the missing half was a Pages Function, so that is all that was built | ✅ |
| 107.2 | **The shape is the allowlist.** `getblockchaininfo` returns chainwork, `size_on_disk`, the value pools, the supply; `shapeStatus` names six fields and the rest never leaves Cloudflare. Filtering a response is a list you can forget to extend — naming what you serve is not | ✅ |
| 107.3 | **The upstream is `ZEC_RPC_URL`, a project secret**, not a literal. It is absent from the repo, from every built asset, and from every response. A test fails if it becomes a literal, and another fails if an error body is ever built from the upstream's own words — an echoed error is the realistic way an address escapes | ✅ |
| 107.4 | **The page was naming the node itself.** `index.html` printed `zec-rpc.cavebro.win` as a decorative header coordinate next to the words "rpc gate / sealed". Decorative and complete. It now reads `zebra / loopback`, and a test greps the built output so it cannot come back | ✅ |
| 107.5 | **No CORS header, and POST refused.** Same-origin only, matching the page's `connect-src 'self'`. Either one missing would turn a status page into the open RPC proxy the whole design avoids. Verified against production: a JSON-RPC POST gets 405 | ✅ |
| 107.6 | **Readings coalesce for 5s.** Each visitor samples every 15s, so without it a crowd spends the node's 8 r/s (§53.4) on one question. 5s is far inside the 120s the page calls fresh, so a shared reading is never a stale one. Best-effort per isolate: this thins upstream load, it does not cap it | ✅ |
| 107.7 | **This hides no secret, and should not be described as one.** §53.1 made the RPC public deliberately, default-deny by method, and DNS would give the name away regardless. What is withheld is a working endpoint handed to every visitor of a public page, and the traffic that follows one | ✅ |
| 107.8 | **The endpoint's output is checked against the page's own validator**, not against a copy of it. `normalizeStatus` is imported into the endpoint's test, so the two halves cannot drift into a page that refuses what its server serves | 🔒 |

## 108. The draw runs for real (18 Sep 2026)

Closing A98. §106 proved the pipeline composes; nothing ran it against a chain.

| # | Decision | |
|---|---|---|
| 108.1 | **The close and the reveal are derived, never typed.** `Plan::from_announcement` takes the published `CloseAnnouncement` and its `PublicationProof` and calls `resolve`; the reveal height and drand round come out of the rules the policy committed. Reading them from the environment would have been simpler and would have reopened exactly the hole §101 closed — an operator who can type a reveal round can try rounds until one pays | ✅ |
| 108.2 | **The draw is a library function, not a file the command writes and the service reads.** `zyn-sale` already holds `Mutex<Option<Run>>` and refuses to settle until the draw exists, so the obvious shape was a serialised batch. There is no decoder for `AllocationBatch` and there does not need to be: §106.2 says the draw is deterministic, so the service can re-run the same function rather than trust an artefact that can drift from it | ✅ |
| 108.3 | **It refuses to draw over a ledger that can still change.** The tip must have buried the close by the committed confirmations, and the reveal block must exist. Both are refusals rather than warnings, because the draw happens once (§104.1) and a reorg afterwards has no remedy | ✅ |
| 108.4 | **Every refused payment comes out with its reason**, and `NotATicket` grew a `Display` to make that sayable. A payment that cannot be a ticket is someone whose money is in the vault; if all of them are refused the error names the reasons rather than reporting a sale nobody entered | ✅ |
| 108.5 | **The manifest is checked against the committed root before anything else.** Verified with the real file: 4,444 entries rebuild `02b01b16…`. A collection that rebuilds a different root is a different collection, and the command says so instead of drawing over it | ✅ |
| 108.6 | **A scanner that will not advance is an error, not a slow scan.** The range walks in whatever steps the scanner allows; if it comes back below where it was asked, that is its own `from_height` sitting above the sale's, and spinning there would look like progress forever | ✅ |
| 108.7 | **Every source is injected** — the chain is a trait and the drand transport a closure, so the whole sequence is exercised without a node. This is the shape §105.4 arrived at the hard way, when ten stubbed tests passed against an endpoint that returned 500 for every round | 🔒 |
| 108.8 | **What it still needs is not code**: `ZYN_VAULT_FVK`, a reachable Zcash node, and a published announcement to point at. The command refuses clearly on each, in that order | ⏳ |

## 109. `zyn_depositAddress` never reached the wallet (18 Sep 2026)

Reported as *"deposit site showing unknown method zyn_depositAddress"*. Three
faults stacked, and only the outermost was visible.

| # | Decision | |
|---|---|---|
| 109.1 | **The reported error came from a stale installed extension.** `unknown method` is thrown by `background.js` when `METHODS` has no entry, and the string appears nowhere in the Rust. Both `src/` and `dist/` carry `zyn_depositAddress`, so the build Chrome had loaded predated it. Reloading it exposed the next fault rather than fixing the page | ✅ |
| 109.2 | **The real bug: the provider POSTed to a GET-only route.** `app()` sends a POST whenever it is handed a body, and `app('/api/deposit-address', {})` reads as "no parameters" while meaning "POST". The agent answered `{"error":"no such call"}`, which the page showed as the wallet failing when the wallet was fine. Fixed by dropping the argument — the presence of a body *is* the verb | ✅ |
| 109.3 | **`zyn_bind` passes `{}` correctly** — `/api/bind` really is a POST. The trap is not the `{}`, it is that nothing checked the two tables against each other | ✅ |
| 109.4 | **The provider and the agent are now checked against each other by test.** `apps/extension/tests/methods.test.js` parses every `app('/path', …)` call out of `background.js` and every `("VERB", "/path")` out of `app.rs`, and fails naming the mismatch: *"POST /api/deposit-address — the agent serves it as GET"*. Verified by reintroducing the bug | ✅ |
| 109.5 | **`dist/` is asserted equal to `src/`.** Chrome loads `dist`; a fix that lands only in `src` is not a fix, and §109.1 is what that looks like from outside | ✅ |
| 109.6 | **Behind all three, a configuration mismatch that is correct behaviour.** With the verb fixed the route answers *"this wallet is on mainnet but its Zyn node custodies testnet"* — the agent's `ZYN_NODE` defaults to `168.119.53.39:8099`, which is testnet chain 11. Mainnet Zyn is chain 26460 on Contabo and its 8099 is firewalled by §53.6, so a mainnet deposit address needs the agent pointed through a tunnel. Operator configuration, not a code fault | ⏳ |

## 110. Receiving accounts are reachable from a page (18 Sep 2026)

Half of A92. §95/§97.5 built the derivation, discovery and endpoints; §99 noted
nothing called them. Finding out why turned up a reason no caller could have.

| # | Decision | |
|---|---|---|
| 110.1 | **Two of the three routes could not be called from a browser at all.** `/api/receiving/collection` and `/api/receiving` were `GET`, and this agent reads every parameter from the request *body* — which `fetch` refuses to send on a GET. They were not merely uncalled; they were uncallable, which is why no caller existed to notice | ✅ |
| 110.2 | **They become POST, joining every other parameterised read here** — `/api/curve`, `/api/curve-quote`, `/api/quote` are all POST for exactly this reason. `/api/receiving/next` takes no input and stays a GET. Nothing called any of them, so the verb was free to correct rather than to version | ✅ |
| 110.3 | **`zyn_receivingAccount` and `zyn_receivingNext` are on the provider**, as reads. A checkout must quote against the collection's receiving account rather than the identity (§97.5), and until now no page could ask for one. Reads rather than approval-gated: the identity is already a read, and a receiving account is strictly less linkable than the identity it derives from | ✅ |
| 110.4 | **What is still missing is the caller.** The checkout page does not exist, so A92 stays open: the mint taking the receiving account, the wallet showing items held across the branch as one collection, and the restore drill are all still ahead | ⏳ |

## 111. Treasury share 01 moves to Hetzner (18 Sep 2026)

The sale signs a mint when a buyer settles, so the treasury must be reachable
while `zyn-sale` serves. It was 2-of-3 across Mac, Air and Windows — three
machines in one house, two of which sleep.

| # | Decision | |
|---|---|---|
| 111.1 | **Moved, not copied**, following §55.1 exactly: transferred over SSH, `sha256` verified at the destination (`6e05b3a1…`), then deleted from the Mac. `public.bin` stays everywhere — it is public, and `zyn-sale` reads it to derive the treasury account | ✅ |
| 111.2 | **The move was not finished by moving the share.** Hetzner's `zyn-custodian` was the 8 Sep build, which predates treasury support — its own error text listed only `ZYN_CUSTODY_SHARE` and `ZYN_CUSTODY_SOLANA_SHARE`. A share on a box that cannot load it is not custody, and nothing would have said so until a buyer settled | ✅ |
| 111.3 | **The new binary was renamed into place, not scp'd over.** Two vault custodians have been running there for a week holding mainnet anchoring custody (§54.2); overwriting the path would have meant stopping them, and mainnet custody is already at bare threshold (§54.3). `mv` replaces the directory entry and leaves running processes on their own inode. Verified after: both still up, 7d and 6.8d | ✅ |
| 111.4 | **Verified by loading it**, not by inspecting the file: `share treasury 0100… serving on 127.0.0.1:8219 — it signs a share on request and holds no quorum`. Stopped afterwards; nothing left listening | ✅ |
| 111.5 | **Custody is now Hetzner (01), Air (02), Windows (03)** — and this is still not a servable sale. Two holders must be up, and Hetzner is the only always-on one. A83 (Windows runs without a login) is now on the critical path for taking money, not merely for alert noise | 🔨 |
| 111.6 | **Hetzner now holds one share of each group** — vault 01 in `/var/zyn/custody-main`, treasury 01 in `/var/zyn/treasury`. One host compromise yields one share of each 2-of-3 and a threshold of neither, so this is a redundancy cost rather than a custody one. §55.7's missing offline backups now cover two shares instead of one | 🔨 |

## 112. The checkout exists (18 Sep 2026)

A92/A81's missing half. `zyn-sale` served `/quote`, `/settle` and `/order` and
no page had ever called them.

| # | Decision | |
|---|---|---|
| 112.1 | **`/settle` could not be driven from a browser, and nothing said so.** It takes the buyer's own signed read — the service cannot look up a private balance (§74.23) — and no route produced one. `Node::account_read_payload` now returns exactly the bytes `account_raw` consumes (`id ǀ epoch ǀ scheme ǀ key ǀ signature`, without the `op` and chain a relay prepends), `POST /api/receiving/read` signs one with the *receiving* key, and `zyn_receivingRead` exposes it | ✅ |
| 112.2 | **`signed_read_header` is now built from that one function** rather than repeating the construction. Two copies drifting would be a signature over one thing and a claim about another | 🔒 |
| 112.3 | **The signed read is approval-gated; the account lookup is not.** Handing a site a proof discloses that account's balance to whoever receives it, which is a disclosure rather than a lookup, and the approval text says exactly that. Reading *which* account is a read, as the identity already is | ✅ |
| 112.4 | **The page reaches `zyn-sale` only through a Pages Function that names three calls.** `POST /quote`, `POST /settle`, `GET /order` and nothing else — a forwarded prefix would publish every route the service ever grows. Upstream is `ZYN_SALE_URL`, absent from the repo and from every reply; a failure says *"the sale service is not answering"* and never what upstream said (§107.3). Verified: allowed routes reach upstream, `POST /admin`, `GET /quote` and `DELETE /order` all get `no such call` | ✅ |
| 112.5 | **It quotes against the collection's receiving account, never the identity.** That is what makes the money and the item land in the same account with no other history, so nothing has to be moved afterwards — and the move is what would have written the link back in (§97.5). A test fails if the page ever calls `zyn_account` instead | ✅ |
| 112.6 | **It does not promise an item at payment time.** The draw runs once, after the close (§104.1), so the page says nothing is assigned yet and treats *"the draw has not run yet"* as the expected answer — *"Paid and counted"* — rather than showing a buyer an error for having paid on time | ✅ |
| 112.7 | **The odds are on the page where the money is asked for**: 80 / 19 / 1, per entry. A buyer should not have to find the weights in a document to know what an entry buys | ✅ |
| 112.8 | **The order survives a reload** in `localStorage` — the code and the address, both already the buyer's. Paying and closing the tab must not lose the claim | ✅ |
| 112.9 | **What remains is deployment, not code**: `zyn-sale` runs nowhere, so `ZYN_SALE_URL` is unset and the page renders its sealed state. Which is the honest state until the terms are published | ⏳ |

## 113. The terms were approved; the code did not implement them (18 Sep 2026)

Checking the published terms against behaviour rather than against the struct
that prints them. `tests/sale_pipeline.rs` (§106) proved the pipeline composes
— under `Fixed { items: 1 }`, `Sequential`, and a close committed in the policy.
Collection Z sells under `Weighted`, `ConcealedShuffle` and an **announced**
close. That is a different path through the allocator and none of it had ever
run. `tests/terms_match_code.rs` drives the real `cave_z::policy()`.

| # | Decision | |
|---|---|---|
| 113.1 | **The published policy could not be drawn at all.** With an announced close the policy holds `deposit_close_zcash_height: 0` — `validate` *refuses* a policy that both commits and announces one — and `allocate` measured every ticket against that zero. A payment 500 blocks before the close was refused as *"deposit confirmed after the committed close height"*. Every entry, every time | ✅ |
| 113.2 | **`allocate_announced` takes the resolved targets.** The close and reveal come from the announcement a caller has publication-checked (§101), read once at the top so no branch can consult the wrong source. The two forms cannot take each other's path: an announced policy without a resolution is refused, and a committed policy with one is too — silently allocating against numbers nobody committed to is the failure this prevents | ✅ |
| 113.3 | **A weighted concealed sale had no source of randomness for quantity.** `Weighted` demanded a per-ticket drand round; `tickets.rs` sets that to `None` and `evidence_for` supplies no beacon, because under a concealed shuffle both quantity and identity are meant to come from the one collection-wide reveal. Nothing joined the two. Now a ticket with no round of its own seeds from the reveal — the ticket id, txid, height and index still enter the seed, so one beacon gives every entry its own draw | ✅ |
| 113.4 | **And that is a fairness property, not a convenience.** A per-ticket round resolves ~30s after issue, while the sale is still open, so a buyer would learn how many items their entry drew *before* deciding whether to buy another. The collection-wide reveal resolves nobody's quantity until the door is shut | ✅ |
| 113.5 | **The change is additive.** A first attempt rejected per-ticket beacons under a concealed shuffle outright and broke three existing tests that use exactly that combination. Making it a fallback rather than a prohibition serves the announced form without changing what already worked | ✅ |
| 113.6 | **Measured, not asserted: 79.9% / 19.1% / 1.0%** over 2,000 entries against a real Quicknet round, for published weights of 80 / 19 / 1 | ✅ |
| 113.7 | **The service quoted a price nothing checked.** `ZYN_SALE_PRICE` defaulted to `0.01` while admission requires the deposit to equal `entry_price_zatoshi` *exactly*. A wrong value is not a wrong price — it is every buyer paying, and every one of them refused at the draw, after the close, unrecoverably. Price, collection and symbol now come from the published terms, and an environment variable that disagrees stops the service naming the consequence. §64.16 said this in words; nothing enforced it | ✅ |
| 113.8 | **The 90/10 split is committed and unimplemented in this path.** The terms print *"900000 to the pool, 100000 to the treasury"*, `SaleTerms::split` is tested, and no settlement code calls it: `pay()` in `zyn-sale` recognises the buyer's balance and moves nothing. A2 records the routing as built in `zyn-io/cave`, which is not the service the checkout talks to. Left open deliberately — where the pool lives and when it is funded relative to anchor finality is a decision, not a fix | 🔨 |

## 114. The entry funds the pot a redemption is paid from (18 Sep 2026)

Closing §113.8 / A101. The 90% was a committed promise with no implementation:
`pay()` recognised the buyer's balance and moved nothing, so a buyer kept their
ZEC.zy *and* got an item, and the floor every buyer is trusting was funded by
nobody.

| # | Decision | |
|---|---|---|
| 114.1 | **The pool is the collection's, and redemption is what it is for.** `FundCollection` credits `c.pool`; `RedeemCollectionItem` destroys one item and pays `pool / outstanding`, remainder staying behind. Both existed in the VM and neither was reachable from a sale. Nothing new was invented here — the sale was simply not wired to them | ✅ |
| 114.2 | **The buyer signs the payment; the sale sends it.** This service holds no key of theirs, so it relays the frame their wallet signed — `submit_raw` takes the bytes untouched because the signature commits to exactly those bytes. Signing and sending are split on purpose: a payment that leaves the wallet but never reaches the sale is a buyer who paid for nothing | ✅ |
| 114.3 | **Signed from the receiving account**, not the identity — that is the account the deposit credited and the account the item lands in, so the payment discloses no more than the entry already does (§97.5) | ✅ |
| 114.4 | **The split comes from the published terms, never a percentage computed at the call site.** `SaleTerms::split` was tested and had no caller; it has one now. Rounding goes to the pool, because the floor is what buyers are trusting and a spare zatoshi belongs on their side | ✅ |
| 114.5 | **The treasury funds the pool with its own threshold signature**, reusing the path the mint already uses. A failure there says *"the entry was paid but the pool was not funded"* rather than a generic refusal, because those two states need different repairs | ✅ |
| 114.6 | **The quote returns the treasury account** so the page signs against an address the service named rather than one it invented | ✅ |
| 114.7 | **The page says where the money goes**, beside the address it asks for money at: 90% funds the pool a redeemed item is paid out of, 10% is the treasury's | ✅ |
| 114.8 | **Still to prove against a live chain**: none of this has run against a node. The ordering it assumes — buyer's transfer accepted, then the treasury's `FundCollection` — is two intents, and a failure between them leaves an entry paid with its pool share unfunded. The message names that state; a reconciliation pass that finds and completes them does not exist yet | ⏳ |

## 115. The money path, over a socket (18 Sep 2026)

`tests/live.rs` has always driven the swap path over a real node. The sale had
no equivalent, which is why §113 and §114 were possible: every component was
tested and the thing they add up to was not.

| # | Decision | |
|---|---|---|
| 115.1 | **`tests/entry_funds_the_pot.rs` runs the money against an in-process node over TCP**: a credited buyer signs their entry, a gateway relays the bytes untouched, the treasury funds the pool, the item mints, the market opens, and the holder hands the item back for its share | ✅ |
| 115.2 | **It asserts the two promises the terms make about money** and nothing else did: 90% of an entry reaches the collection's pool, and a redemption is paid out of that pool. Both were unasserted until now — including through the whole period when the sale moved no money at all | ✅ |
| 115.3 | **The first version passed while being partly vacuous.** `pool_share` drove both the funding amount and the expected pool, so funding 100% of the entry still passed — it proved only that what you fund is what lands. Expectations are now independent literals tied to the published split, and the mutation that fooled it now fails on *"90% of the entry reached the pool"*. Checked by running it | ✅ |
| 115.4 | **It also pins the honest floor**: `redeem_price` is zero before the collection is live, because until then the quotient is an artefact of how far the sale got rather than something a holder can act on | ✅ |

## 116. A99: the wallet reaches mainnet Zyn (18 Sep 2026)

"Point `ZYN_NODE` at mainnet" was one line of the answer. Four things were
broken underneath it, each hidden by the one above.

| # | Decision | |
|---|---|---|
| 116.1 | **Mainnet Zyn was not running.** `zynzapd-mainnet` had been stopped cleanly on 15 Sep at 15:28 and left disabled; nothing listened on 8099. Started, and chain 26460 answers through the tunnel: seq 348, epoch 5 | ✅ |
| 116.2 | **It still cannot anchor, and that is expected.** An anchor is a self-send from the vault and the mainnet vault has never been paid — the env file says so in a comment. Alerts go to the journal only, so running it unanchored costs nothing. **Deposits must not be taken until the vault is funded**: an unanchored chain has no Zcash commitment behind it | 🔨 |
| 116.3 | **The mainnet block server was on a machine that cannot serve it.** `zyn-lightd` lives in the Windows box's WSL distro and depends on that box's mainnet Zebra, which is stopped because its disk is too small. Contabo has a fully synced mainnet Zebra and had no lightd. Deployed there, loopback-only on 8097, reached over the same tunnel as 8099 — the same move as A91, for the same reason | ✅ |
| 116.4 | **The deposit-address cache was not keyed by chain.** A `utest1…` address cached on 10 Sep survived the switch to mainnet, so a correctly configured mainnet wallet reported that its *node* custodied testnet. The guard caught it, which is the only reason it was a confusing message rather than a buyer paying a testnet address. Now `deposit-address-<chain>`; the stale file is kept aside as evidence | ✅ |
| 116.5 | **The agent token lived only in the shell that started the agent.** Restarting it lost the secret and nothing could recover it — `nap-agent` was not running to read it from, and the runbook generates it fresh. Now in `~/.zyn/nap-wallet.env`, mode 600, with the node and chain beside it | ✅ |
| 116.6 | **Verified end to end**: `/api/deposit-address` returns `u15dljrf4z…` with `memo: false` — a mainnet per-account address that needs no memo, which is the property A75 was closed on | ✅ |
| 116.7 | **The tunnel is not durable.** One `ssh -f -N` with keepalives carries 8099 and 8097; if it drops the wallet loses both. It wants a LaunchAgent before anyone relies on it | 🔨 |

## 117. The parts that must be up are now managed (18 Sep 2026)

Three things the sale depends on were running because someone had started them
in a shell, or were not running at all.

| # | Decision | |
|---|---|---|
| 117.1 | **The mainnet tunnel is a LaunchAgent.** `dev.zyn.mainnet-tunnel` carries 18100→8099 and 8097→8097 with `KeepAlive` and 15s keepalives. The wallet's configuration assumes those ports exist, so a hand-started tunnel made the wallet's health a property of which terminal was open. Verified by killing it: launchd restarted it after the throttle and chain 26460 answered again | ✅ |
| 117.2 | **Treasury custodians run on both always-on holders.** Neither host was serving the treasury at all: Hetzner had the share but no unit, and the Air's only custodian serves the *Zcash vault* on 8210. A mint is signed when a buyer settles, so these must be listening while the sale serves — which makes them managed services rather than something started for a ceremony and stopped after | ✅ |
| 117.3 | **Hetzner (01) under systemd, the Air (02) under launchd**, both loopback-only on 8211, both restarting on death. Verified by killing each: both came back serving their share. Threshold is 2, so the sale can settle without the Windows box, whose WSL distro is stopped | ✅ |
| 117.4 | **Both hosts needed a newer binary first.** Hetzner's was 8 Sep and the Air's 11 Sep; neither knew `ZYN_CUSTODY_TREASURY_SHARE` — their own error text listed only the Zcash and Solana vars. Deployed by rename on both, so the vault custodians (7d and 2d23h) kept their inodes and were never interrupted | ✅ |
| 117.5 | **The sleeping was misdiagnosed and is recorded so.** The Air has been up 89 days with `sleep 0`; the Windows *box* is up and only its `zyn` WSL distro is stopped. A83 is not on the critical path for taking money — the missing piece was custodians nobody was running, not machines that slept | ✅ |
| 117.7 | **Deploying by rename left the running vault custodians on the old binary**, which is the cost of the trick that avoided interrupting them: `mv` replaces the directory entry, so a running process keeps its deleted inode until it restarts. Hetzner's two showed `/usr/local/bin/zyn-custodian (deleted)` after 7 days; the Air's after 2d23h. All five custodians across both hosts have now been restarted onto the current binary, one at a time, and hash-verified against the local build — `5a415a3f…` on Hetzner, `46a39048…` on the Air. The window was chosen deliberately: mainnet cannot anchor while the vault is unfunded, so vault custody being briefly down cost nothing | ✅ |
| 117.6 | **Still to wire when `zyn-sale` is deployed**: these listen on loopback, so the sequencer reaches them over tunnels the custodians open (§54.2). Pointless to build before the service that would call them exists | ⏳ |

## 118. Two testnet Zebras retired (18 Sep 2026)

34 GB each, on two laptops, for one test chain that already has a node on
always-on infrastructure. Asked and answered by checking what read them.

| # | Decision | |
|---|---|---|
| 118.1 | **The Air's served nobody.** Its `signer2` reads `127.0.0.1:18232` — Hetzner's Zebra through the tunnel — while its own node listened on 18242 with no consumer in any agent or config. A fully synced testnet chain, kept current, that nothing asked a question of. Removed outright | ✅ |
| 118.2 | **The Mac's had a real job, and it is gone on purpose.** `signer1` read its own node on 18242; `signer2` already read Hetzner's. Since Hetzner *is* the chain 11 sequencer, that one independent view was the only thing stopping both signers from verifying the sequencer's anchors against the sequencer's own picture of Zcash. Repointed to 18232 and the node deleted | ✅ |
| 118.3 | **What that costs, stated plainly:** chain 11's two endorsements now rest on one Zcash view, supplied by the box being endorsed. A sequencer that could lie about Zcash would be believed by both signers. Accepted because it is testnet, because both signers are the operator's anyway (A77 — "the operator asserting to itself"), and because mainnet's arrangement is separate: Contabo has its own Zebra and its custody lives elsewhere. **It would not be acceptable on mainnet** | 🔨 |
| 118.4 | **Order mattered and was followed**: repoint, reload, confirm the signer came up on the new endpoint and kept endorsing, *then* delete. `launchctl kickstart -k` restarts a job without re-reading its plist — the first attempt silently kept 18242, and the log's startup line is what caught it. `bootout` + `bootstrap` is what applies a changed plist | ✅ |
| 118.5 | **Both plists are kept, disabled**, as `dev.zyn.zebrad.plist.removed-20260918`, and signer1's old plist as `.bak-18242`. Re-syncing a testnet chain takes days; the configuration should not also have to be rebuilt from memory | ✅ |

## 119. Mainnet Zyn gets a public JSON-RPC, and a browser wallet (18 Sep 2026)

§53.6 kept `zyn-rpc` private "until ZynZap is mature". Reversed deliberately:
a browser wallet cannot exist without it, and a buyer with no wallet cannot get
a deposit address.

| # | Decision | |
|---|---|---|
| 119.1 | **The gateway existed in code and ran nowhere.** `zyn-rpc` translates JSON-RPC to the node's binary framing — the node's own socket "cannot be reached from a browser" by design. No binary on any host. That, not the static site, is why the wallet was undeployable | ✅ |
| 119.2 | **Two gateways, one name.** `zyn-rpc.cavebro.win/` is mainnet chain 26460 against Contabo's local node; `/testnet/` is chain 11 against Hetzner's already-public one. The DNS name was reserved by §53.6 and answered nothing; it answers now | ✅ |
| 119.3 | **No njs method filter, unlike the Zcash RPC beside it.** `zyn-rpc` carries its own method table and refuses anything absent from it, so nginx adds only TLS, a 64 KB body cap and 8 r/s. It holds no key: a write arrives as a signed frame relayed byte for byte, and a read arrives already signed so the account comes from the signature, never a parameter | ✅ |
| 119.4 | **Mainnet by default, testnet selectable.** `nap-web` took one fixed chain; it now takes a `networks` list with the top level as the default, so a deployment that never heard of it validates exactly as before. Switching keeps the phrase — the seed is chain-independent, so there was no reason to make a network change cost a restore — and clears only the cached answers | ✅ |
| 119.5 | **Every endpoint is still same-origin.** The wallet refuses a cross-origin URL and its CSP permits none, which is what stops a browser wallet being pointed somewhere bad. Three Pages Functions relay `/rpc`, `/media` and `/claim`; `/rpc` names the two chains it will reach rather than forwarding a prefix, and never echoes the upstream | ✅ |
| 119.6 | **`/claim` refuses in words.** The gift coordinator is not deployed, and a holder following a claim link should learn the service is absent rather than that their link is malformed | ✅ |
| 119.7 | **An nginx reload failed on a config I had just written** — `$binary_remote_addr` expanded to nothing in an unquoted heredoc. nginx kept its running config, so `zec-rpc.cavebro.win` and `signal.cavebro.win` never dropped, but the broken file would have taken nginx down at its next restart. Checked both live services from outside before fixing, and `nginx -t` now passes | ✅ |
| 119.8 | **Live at `nap-web-eaa.pages.dev`; `nap.cavebro.win` is attached but has no DNS record.** Wrangler's OAuth token carries `zone (read)`, not DNS edit, so the CNAME is the one step that needs the operator | ⏳ |

## 120. `unknown operation` was a stale node, not a stuck one (18 Sep 2026)

`nap.cavebro.win` reported *"No deposit address: node refused: unknown
operation"*. Read as a freeze; it was a version skew.

| # | Decision | |
|---|---|---|
| 120.1 | **A frozen node times out or stops advancing. This one answered — with a refusal.** Mainnet 26460 was serving, epoch advancing, and the *local* agent had been issuing mainnet deposit addresses all along. Two different ops: the agent asks `OP_DEPOSIT_ADDRESS` (19, by account id), the browser wallet asks `OP_DEPOSIT_ADDRESS_SIGNED` (28, by pre-signed read). Only 28 was unknown | ✅ |
| 120.2 | **Op 28 arrived with A80 on 16 Sep; Contabo was running the 15 Sep build.** One day of skew, and only the newer of two paths went through it — which is why the desktop path looked healthy while the browser path did not | ✅ |
| 120.3 | **`STATE_VERSION` was checked before deploying, not after.** It is still 12 and the node runs `mainnet-v12`, so the current binary loads the same state — a mismatch is a hard reject with no migration path (§codec), and finding that out by restarting a mainnet sequencer would have been the wrong order. State tarred to `/var/backups/zyn/` first | ✅ |
| 120.4 | **Verified by doing what the browser does**, not by reading the changelog: derived an identity from the BIP-39 test phrase with `nap-web`'s own `core.js`, signed a read for chain 26460 at the live epoch, and called `zyn_depositAddress` through `https://nap.cavebro.win/rpc`. Returns `u1rhlf93n…`, `memo: false` | ✅ |
| 120.5 | **The old binary is kept as `zynzapd.sep15`.** The window was safe for the same reason the custodian restarts were: mainnet cannot anchor while the vault is unfunded, so a sequencer restart costs nothing that was not already stopped | ✅ |
| 120.6 | **A version skew between a node and the things that talk to it is invisible until one of them uses a new op.** Nothing reports the node's build, and the failure surfaced as a wallet error blamed on the wallet. Worth a version in `zyn_status` | 🔨 |

## 121. A wallet may be kept on a device, sealed (18 Sep 2026)

Re-typing 24 words on every reload is not a security property — it is a reason
to write them somewhere worse. Opt-in persistence, with the threat model
changed deliberately rather than by accident.

| # | Decision | |
|---|---|---|
| 121.1 | **What is stored is the derived Ed25519 key, never the phrase.** `identityFromSecretKey` reconstructs everything an identity does from 32 bytes, so the words never need to touch the disk. The words also control the *Zcash* wallet and may be reused elsewhere; the derived key controls one Zyn account at one index. A cracked record costs that account, not the seed behind it | ✅ |
| 121.2 | **AES-256-GCM under PBKDF2-SHA256 at 600,000 iterations**, OWASP's current floor, with a per-record salt and IV. Once ciphertext is on a disk the password *is* the security, so the KDF is the thing that decides how expensive guessing is | ✅ |
| 121.3 | **A wrong password and a tampered record fail identically.** GCM authenticates either way; distinguishing them would tell someone holding a stolen file that they are close | ✅ |
| 121.4 | **A record naming weaker settings is refused before the password is tried.** Editing the stored iteration count down is how an attacker makes a later brute force affordable, and it is the one tamper that the AEAD cannot catch because it is outside the sealed bytes | ✅ |
| 121.5 | **Off unless asked for, and the page no longer claims otherwise.** It advertised "No browser storage"; it now says "Nothing kept unless you say so", and the README states plainly that keeping moves the threat model from "needs the live tab" to "can guess offline forever" | ✅ |
| 121.6 | **Locking returns to what the device can offer** — a password where the wallet is kept, the phrase where it is not — and "Forget this device" removes the record | ✅ |
| 121.7 | **Verified in a browser against live mainnet**, not only in tests: restored with the phrase and a keep password, reloaded, was met by the unlock screen, refused a wrong password, and opened with the right one to account `b85db260…da56ea` — the same account §119.4's cross-implementation vector pins. Test wallet cleared afterwards | ✅ |

## 122. The corner names both chains, and the name is checked (18 Sep 2026)

The wallet spans two networks — ZEC goes in at a Zcash address and arrives as
ZEC.zy on Zyn — and the header named only one of them.

| # | Decision | |
|---|---|---|
| 122.1 | **`ZEC mainnet · Zyn mainnet`**, with the pair carried through the selector: switching gives `ZEC testnet · Zyn testnet`. A Zyn chain sits on exactly one Zcash network and the node refuses to mix them (§116.4), so the pairing is a fact about the deployment rather than two independent settings | ✅ |
| 122.2 | **The chain id stays off the header** (§121's change) and stays on the send review, where it decides something | ✅ |
| 122.3 | **The label is verified, not decorative.** `loadDepositAddress` took whatever address string the node returned. It now checks the address against the network the header names — `u1…` for mainnet, `utest1…` for testnet — and shows nothing at all if they disagree. Without that check, "ZEC mainnet" above a `utest1…` address is exactly the confusion §116.4 caused on the desktop side, except here it would be above an address someone is about to send money to | ✅ |
| 122.4 | **A network that does not name its Zcash side is a configuration error**, refused at load. An optional field would have let a deployment show a label it never checked | ✅ |
| 122.5 | **`config.json` revalidates on every request** (`max-age=0, must-revalidate`, `cf-cache-status: DYNAMIC`), which matters because the new code *requires* the new fields — a cached older config would be fatal rather than merely stale. Checked rather than assumed | ✅ |

## 123. The wallet shows what it holds, not only what it owns one of (18 Sep 2026)

`nap-web` listed items and no balances. The data was already being fetched and
thrown away: `refresh` reads `account.spendable`, and `renderItems` filtered it
to holdings with a `content` hash — every fungible coin fell out of the render.

| # | Decision | |
|---|---|---|
| 123.1 | **A Coins section above Items**, from the same `spendable` the items come from. Mainnet carries two fungible assets — `ZEC.zy` (which is `XZEC`) and `ZYN` — and neither had ever been visible to a holder | ✅ |
| 123.2 | **`ZEC.zy` is listed even at zero.** Somebody who has just sent a deposit needs to watch it arrive; an empty list would read as the wallet not having noticed, which is exactly when a buyer sends a second one | ✅ |
| 123.3 | **`readableAmount` lives in `core.js`, not the view.** It is a pure function and the tests are there. Written as display-only and documented so: anything that decides something re-reads the raw string, because a trimmed amount must never be what a signature is computed over | ✅ |
| 123.4 | **A dust balance is not rounded away.** `0.000000000000000001` renders whole — a formatter that collapsed it would tell a holder they have nothing while the chain says otherwise | ✅ |
| 123.5 | **First styling attempt was wrong and the screenshot caught it.** The coin card was written with the dark cave palette the other sites use; the wallet is cream. It now uses the same card as an item — `#fffaf0`, `#1f24282d`, 14px — rather than a grey block floating on the page | ✅ |

## 124. The wallet looks where bought items actually land (19 Sep 2026)

§97.5 sends a purchase to the collection's *receiving* account so the money and
the item share an account with no other history. The wallet read only the
identity. A buyer would have paid, been minted an item, opened Nap and seen an
empty pocket — indistinguishable, to them, from not having received it.

| # | Decision | |
|---|---|---|
| 124.1 | **`deriveCollectionAccount` ports `receive::collection_key` to the browser**: HKDF-SHA256, salt `nap.zyn.receive.v1`, info `"collection" ‖ u32(account) ‖ collection`. Pinned on both sides against the same phrase — Collection Z resolves to `ccb0a7aa…` in Rust and in JavaScript. A drift here does not error, it looks in the wrong place | ✅ |
| 124.2 | **The kept wallet had to carry them.** Sealing only the identity key (§121.1) left a reopened wallet unable to derive anything, because the seed is deliberately not stored. The sealed record is now a bundle — identity key plus one key per collection — and still contains no mnemonic, so a cracked record costs those accounts and not the Zcash wallet the same words control | ✅ |
| 124.3 | **`refresh` reads every account and tags each holding with the key that holds it.** Not merely cosmetic: `openSend` now signs with `item.owner`, so sending a bought item is signed by the account that actually holds it rather than by an identity holding nothing | ✅ |
| 124.4 | **Collections are declared per network** in `config.json`, so a wallet never derives a receiving account for a chain that has no such collection | ✅ |
| 124.5 | **Verified against live mainnet, both paths**: restore-from-phrase synced at epoch 510, and reopening a *kept* wallet issued **two** `zyn_account` reads — identity and Collection Z — which is the specific thing that would silently regress if the seal ever stopped carrying them | ✅ |
| 124.6 | **Caught by asking what a buyer sees, not by a failing test.** Nothing was broken; the wallet did exactly what it was written to do. A92's acceptance drill — clear state, restore, prove every item comes back — is what should have caught it, and is still unrun | 🔨 |

## 125. The terms are published (19 Sep 2026)

A96, first half. §64.16 required canonical `cave-sale-terms/v1` bytes published
before the first deposit; they existed only as a digest a binary could print.

| # | Decision | |
|---|---|---|
| 125.1 | **The bytes are published, not only the digest.** `zyn-cave-terms bytes` emits the 187 canonical bytes, served both inside `cavebro.win/terms` and as `cavebro.win/cave-sale-terms-v1.hex` so a verifier can `curl` them rather than scrape a page. A digest nobody can recompute is a number, not a commitment | ✅ |
| 125.2 | **The recipe is on the page**, six lines of standard-library Python. The domain and payload are each length-prefixed before hashing, which is why no two different documents can collide — stated, because a reader should not have to infer the framing from a hash that happens to match | ✅ |
| 125.3 | **Verified end to end against the live site**: fetching `cave-sale-terms-v1.hex` from cavebro.win and running the published recipe reproduces `159b454e…`. A test does the same from the page's own bytes, so the page cannot drift from the digest it claims, and a second asserts the downloadable file equals the bytes shown | ✅ |
| 125.4 | **The page says what it does not prove.** It establishes *what* the terms are, not *when* they were written — it is our own web server, and nothing inside a document dates itself. The Zcash stamp that would date it has not happened, and the page says so in those words rather than leaving a careful reader to work it out | ✅ |
| 125.5 | **`terms` sits in the landing navbar**, beside `deposit`. Terms reachable only by URL are published in the same sense a notice in a locked drawer is | ✅ |
| 125.6 | **The remaining half of A96 is blocked behind A59.** Stamping the digest into a `ZYP` memo is a vault spend, and the vault is unfunded — the same gate as anchoring. Until then the timing rests on our word | ⏳ |

## 126. The settle crash window is recoverable (19 Sep 2026)

Closing §114.8. A paid entry funds the collection's pool in a *second* chain
operation by a *different* signer. A failure between the two was silent, and
`pay()` doing both meant the order rolled back to `Reserved` — losing the record
that the buyer had already paid.

| # | Decision | |
|---|---|---|
| 126.1 | **The payment is written down before the pool is funded**, and that order is the whole point. `settle` now takes `fund_pool` as its own step, called only after `State::Paid` is persisted. A failure after that line leaves a record saying the buyer paid, instead of a rollback that says they did not | ✅ |
| 126.2 | **`Order::pool_funded` records the second half separately from `state`**, because it is a different operation by a different signer and collapsing them into one enum would make the half-done case unrepresentable | ✅ |
| 126.3 | **Six fields still decode, and read as unfunded.** A record written before this field existed is checked rather than assumed paid out — the conservative direction, since the wrong guess here silently shorts the floor | ✅ |
| 126.4 | **`unfunded` makes the shortfall findable and `reconcile` finishes it**, idempotently: an order already marked funded is left alone, so running it twice does not pay twice. Tested by failing the pool deliberately, then reconciling, and asserting the buyer paid **once** and the pool received **once** | ✅ |
| 126.5 | **An unpaid reservation is not a shortfall.** Funding its share would pay the pool out of the treasury for a sale that never happened, so `reconcile` refuses it | ✅ |
| 126.6 | **`zyn-sale reconcile` stops at the first failure** rather than carrying on. If the treasury cannot fund one share it will not fund the next, and a long list of identical errors buries the reason | ✅ |
| 126.7 | **The buyer is told the truth.** A pool failure now answers *"your payment landed; the collection's share has not, and an operator will finish it"* — not a generic refusal that would read as "your money is gone" | ✅ |

## 127. A node says what it can do (19 Sep 2026)

Closing A104. §120 was a one-day skew: the browser wallet asked for op 28, a
node built the day before did not know it, and the refusal surfaced as a
*wallet* error. Nothing reported the node's build, so the failure pointed at the
wrong component.

| # | Decision | |
|---|---|---|
| 127.1 | **`PROTOCOL_VERSION` is the highest op code the node answers**, so a caller's check is `protocol >= OP_THEY_NEED` rather than a table of versions to features. It must be bumped whenever an op is added, and the constant says so | ✅ |
| 127.2 | **Appended to `OP_STATUS`, which is append-only by design.** An older client that reads the fixed fields and stops is unaffected; a newer client reading an older node gets `protocol: 0` — "too old to say", not "supports nothing" | ✅ |
| 127.3 | **Surfaced in the JSON-RPC `zyn_status`**, because the browser wallet is the caller that was blamed and the one that cannot inspect a binary | ✅ |
| 127.4 | **A test that sliced the status body from the end had to learn about the suffix.** Fixed by having it strip the version block rather than by loosening the assertion — the tail offsets are the thing it exists to check, and appending is the documented pattern here | ✅ |

## 128. Both chains on the current build (19 Sep 2026)

| # | Decision | |
|---|---|---|
| 128.1 | **Mainnet and testnet nodes and gateways all deployed**, state tarred first on both hosts, `STATE_VERSION` confirmed still 12 against each data directory before anything stopped. Binaries renamed into place and the services restarted, so a running process was never asked to overwrite itself | ✅ |
| 128.2 | **The version reporting found a second skew on its first use.** After upgrading the mainnet node, `zyn_status` still said nothing — because the *gateway* was the older binary. Upgrading that surfaced a third: testnet reported `protocol: 0`, exactly as designed for a node too old to say, and its node was the 16 Sep build. All four are now `protocol 28, build 0.1.0` | ✅ |
| 128.3 | **Nothing regressed**: testnet advancing at epoch 4,671, and signer1 verified an anchor through epoch 4,667 and endorsed it against the upgraded node | ✅ |
| 128.4 | **`zyn-sale` was not deployed, and the blockers are not code.** Contabo holds none of what it needs — no treasury `public.bin`, no 4,444-entry manifest, no whitelist registry — and **no invitation codes have been issued**, so a deployed service would refuse every quote. Custody is also unreachable: Contabo forwards both *vault* custodians (8212, 8213) and neither *treasury* one, and Hetzner's key pins `permitopen` to 8210, not 8211 | ⏳ |

## 129. A sequencer restart no longer deadlocks anchoring (19 Sep 2026)

Reported as "testnet signer down". It was not: both signers were running and
had endorsed. Restarting the sequencer during the §128 deploy discarded the
signatures it had already been given, and no signer would send them again.

| # | Decision | |
|---|---|---|
| 129.1 | **The alert named the wrong component, and the logs said so plainly**: `anchor cdc63d82… is on Zcash and confirmed; awaiting signatures (120 pass(es)) before releasing its deposits`, against signers whose own logs showed them endorsing that exact anchor an hour earlier. "Signer down" meant "the sequencer has not heard from a signer", which after a restart is also what a healthy, finished signer looks like | ✅ |
| 129.2 | **§81 already cured the neighbouring case and could not cure this one.** A signer re-offers until it sees the anchor settle *in its own ledger* — which covers a sequencer restart mid-flight. Here the anchor had settled, so every signer had correctly stopped, while the sequencer still needed the certificate to release deposits. One step past where the existing cure reaches | ✅ |
| 129.3 | **`pending_certificates` was an in-memory `Vec` and is now written beside the ledger.** Saved on shutdown, restored at startup through the `submit_certificate` path that already existed. Keeping them is the only end of this the sequencer controls — it cannot make a finished signer speak again | ✅ |
| 129.4 | **A damaged certificate file is dropped, not fatal.** They are an optimisation over asking the set again; refusing to boot over them would turn a recoverable restart into an outage. Tested with garbage in the file | ✅ |
| 129.5 | **Testnet was unstuck by rewinding both signers' state** so they re-verified from the base root and re-offered. `anchored_epoch` moved 4662 → 4667 and the anchors component went green. That is the manual repair this fix exists to remove | ✅ |
| 129.6 | **Mainnet got the fix before it could ever hit this.** It cannot anchor yet, so there were no certificates in flight to lose in the transition — the cheapest possible moment to change how they are stored | ✅ |
| 129.7 | **The deploy caused it, and the deploy is how it was found.** Worth stating: every restart until today carried this hazard silently, and mainnet anchoring gates the sale | ✅ |

---

## 130. The private object model becomes the plan (20 Sep 2026)

`zyn-private-object-model.md` described assets as shielded state objects —
commitments, nullifiers, values nobody but the holder can read — and §70.9
deferred it as "a separate later scope" with "no bespoke proof system
authorized". That deferral is withdrawn. The model is adopted, and the
operator-blind tier is now a **release gate rather than a destination**: no
deposit opens until a holder's amounts and ownership are hidden from us too.

The reason is not that the old tier was dishonest. §92.3 is a defensible
sentence and it was always said out loud. The reason is that the product claim
is "Zcash privacy should not end where programmability begins", and on the
operator-trusted tier that claim is aspirational at exactly the moment a
stranger's money arrives. Taking deposits first and closing the gap afterwards
would mean the first buyers are the ones who never got the property.

| # | Decision | |
|---|---|---|
| 130.1 | **Adopted: assets on Zyn are private state objects, not account balances.** An object is consumed and its successors created; a nullifier retires it; the public sees commitments. This replaces the account-and-balance shape of the ledger, not the account *identity* — `account_of` still names who may spend, and the wallet-controlled schemes (§2) are unaffected | 🔒 direction |
| 130.2 | **Supersedes §70.9 and the sequencing half of §93.9.** Notes, nullifiers and zero-knowledge transfers were "the destination"; they are now the plan, and a bespoke proof system **is** authorized by this decision. §70.1's tier statement remains correct about *today* and stays until the gate is met | 🔒 |
| 130.3 | **Now is the cheapest moment this will ever be.** Mainnet holds 2 accounts and has never anchored; nothing has been sold. The same change after the 4,444 sell is a re-mint of real holdings. §129.6 made the identical argument about certificate storage a day earlier — the transition is free precisely while the chain is empty | ✅ |
| 130.4 | **Four tiers, each shippable and each honest on its own.** The ladder is the roadmap; a tier is not "done" until the sentence it licenses is true. See the privacy ladder in BACKLOG | 🔒 |
| 130.5 | **Tier 2 is a `STATE_VERSION` bump and retires the balance-shaped intents.** `STATE_VERSION` hard-rejects mismatches with no migration (§71.34), which is the correct behaviour and means this is a coordinated replacement, not a rollout. Plan it as one | 🔒 |
| 130.6 | **Tier 3 buys hidden amounts without a circuit, and is not to be skipped.** Pedersen value commitments summing to zero prove conservation; Bulletproofs range proofs stop negative-value overflow and need no trusted setup; ownership is a signature over the nullifier; note contents are encrypted to the recipient. The operator still sees the graph. This is a real property, reachable without a cryptographer, and it is the fallback if Tier 4 slips | 🔒 |
| 130.7 | **Only Tier 4 makes the operator blind**, because the remaining leak is *which* commitment was spent, and hiding that is a Merkle-membership proof in zero knowledge. This is the rung that needs a cryptographer and an audit, and it is the gate on deposits | 🔒 gate |
| 130.8 | **The gate is Zyn-as-private-money, not every product that touches the vault.** What waits for tier 4 is holding a spendable balance in Zyn *under a privacy claim* — the thing the brief is about. ZEC still moves into the vault before then for a purchase; that is a sale, not a custody promise | 🔒 gate |
| 130.8a | **Cave is gated at tier 1, not tier 4, and that is §70.1 unchanged.** §70.1 selected the operator-trusted tier *for the NFT product* and it was always the honest answer there: a buyer acquires a collectible whose allocation record is already blind (§93.2). Cave opens on tier 1 plus Gate 2's security items. §130 raises the bar for Zyn's money product; it does not retroactively raise Cave's | 🔒 scope |
| 130.8b | **ZynZap and the launchpad are not tier-4 gated either — but their copy is.** The live site at `zynzap.pages.dev` says "private balances", "private LP positions" and "spendable here, privately". Those are false on every tier below 3. Whatever ships there states the tier it is actually on, or does not use the word | 🔒 invariant; 🔨 copy is wrong today |
| 130.8c | **A59 and A100 are therefore not blocked by this decision.** The funding drill proceeds; the sale opens on tier 1 + Gate 2. An earlier draft of §130 read the gate as covering every deposit and would have parked the only revenue behind a proving system | ✅ corrected same day |
| 130.8d | **Nap straddles the boundary, and its layout makes a claim its copy does not.** The Zcash half — shielded notes, keys on the holder's machine — is private by Zcash's own guarantee and is gated by nothing here. The Zyn tab inherits whatever tier Zyn is on. But the wallet's design system says `--private` tints "what only this key can see" and calls keeping that apart from `--record` "the one thing this wallet's layout is *about*" — and applies it to `.nft-send-panel`, which is Zyn state the operator reads today. The wallet has two categories and the truth needs three: private to you, visible to the operator, publicly verifiable | 🔨 A109 |
| 130.8e | **A wallet is the worst surface on which to blur this**, because it shows shielded ZEC and Zyn holdings side by side and invites the reader to assume one property covers both. It is also the surface going to a store listing. Nap is not tier-gated; its *visual vocabulary* is | 🔒 invariant |
| 130.9 | **Tier 1 ships first and immediately.** A90, A94 and P2 are built or nearly so and move ownership from public to operator-trusted. They are not made redundant by Tier 4: sealing protects the journals that exist today, and today's plaintext mirror contents stay plaintext forever (§70.8) | 🔨 |
| 130.9a | **Tier 1 survives tier 2, item by item, and the reasoning differs for each.** A90 protects the epochs being published *now*, which stay plaintext forever if it is not on (§70.8), and is still required at tiers 2 and 3 because the sequencer validates in the clear there. A94 is orthogonal at every tier — it governs who reads the Zcash vault, not Zyn's state shape. P2 is the one tier 2 partially subsumes, since a commitment hides ownership without fresh accounts; its derivation/backup/restore machinery survives, and Cave needs it under §130.8a regardless | 🔒 |
| 130.13 | **The external solvency check quietly depends on plaintext deposit amounts, and §130 had not noticed.** `ZYN_BACKING_SIGNERS` is the only check on the system performed against *Zcash* rather than against ourselves: `backing.rs:158` sums the note values a txid actually paid the vault, decrypted with the vault FVK, and refuses if `paid < amount` — where `amount` comes from the `CreditDeposit` intent in the clear. Commit deposit amounts and that comparison has nothing on one side: the backing signer set becomes decorative while still appearing to pass. **Tier 3 must keep a checkable quantity on the deposit path, or replace this check before hiding it.** It is the sharpest constraint found and it is not the one we expected | 🔒 invariant |
| 130.14 | **The precedent is already in the tree, and it is not the accounts section.** `zyn-vm/src/collection.rs` already commits items to their own tree with their own proof path, and says so: *"'I own item X' and 'I hold N xZEC' are two different proof paths"*. swapvm carries it as a **conditional** section (`merkle.rs:322-324`), which is exactly how a commitments/nullifiers section gets added without disturbing existing roots. Tier 2 is closer to extending a shipped pattern than to inventing one | ✅ observed |
| 130.15 | **Build the commitment tree on `commit.rs`, not on bridgetree.** `ProofIndex` (`commit.rs:192-264`) is the append-structure we want, is `no_std`, and already lives inside the guest boundary — `zyn-vm` builds for `riscv32im-unknown-none-elf`. The vendored `bridgetree`/`incrementalmerkletree` in `zyn-custody` hash with Orchard's `MerkleHashOrchard` and would drag Pallas into the guest. Same shape, wrong side of the line | 🔒 |
| 130.16 | **Conformance forbids two things tier 2 would do naively.** S6 requires account ids strictly ascending and `account_ids().len() == account_leaves().len()`; S3 requires `account_leaves()` byte-identical after a *rejected* batch (`conformance.rs:398,413`); and the suite refuses outright a VM whose fixture has no accounts (`:292-299`). A commitment appended eagerly — before a batch is known to be accepted — violates S3. Commitments must be appended on commit, not on submission | 🔒 |
| 130.17 | **The verifier never enumerated accounts, and that is the good news.** `verify_record` needs the record bytes, the path and the anchored root: `O(log n)`, no account list (`da.rs:421`, `commit.rs:268`). Enumeration is entirely a *publisher* burden — `Snapshot::of` walks every account every seal (`node.rs:591`). So an object ledger does not change what a holder's counterparty must do; it changes what we must do each epoch | ✅ observed |
| 130.18 | **The escape hatch stops at "VALID".** Nothing turns a verified `ExitProof` into a payout: `zyn-cli verify-exit` prints VALID and tells the holder to cross-check the root against the mirror. The redemption step is a custodian procedure, not code. This makes the anchor-cadence tradeoff smaller than stated when daily anchoring was set — the hatch is already operator-mediated — and makes writing that redemption path a prerequisite for claiming a hatch at all | ✅ observed; ⏳ |
| 130.10 | **Publishing commitments and nullifiers in the clear returns some of what §70.4 gave up.** Sealing is blunt — encrypt the journal and permissionless replay dies. A stranger holding no key can still check that no nullifier appeared twice, which is the double-spend argument, without learning an amount or an owner | 🔒 |
| 130.11 | **Explicitly not authorized by this decision**: a new language or VM (§2's rejections stand, including Move), fully private pool reserves in the first version, private lending, a ZYN token, or custom AMM mathematics. The economics stay boring and proven; the novelty is the state model. Sui is named nowhere here — its objects are public, and everything private in the brief is Zcash's own construction | 🔒 scope |
| 130.12 | **The honest sentence changes per tier and must be checked before it is said.** Tier 1: "private from the public, operator-trusted." Tier 3: "we cannot see what you hold." Tier 4: the brief's one-sentence definition. §92.3's discipline — name the tier you are actually on — survives this decision unchanged and is the reason it is safe to state any of them | 🔒 invariant |

---

## 131. No zkVM for v1, and the circuit count is one (21 Sep 2026)

The private-object brief's author was asked whether Zyn should prove execution
in a zkVM. The answer was no for the first version, design for one later, and
it is right — with one correction that makes v1 materially cheaper than the
brief itself claims.

| # | Decision | |
|---|---|---|
| 131.1 | **No zkVM in v1. Fixed circuits above the object layer.** A zkVM is what buys *permissionless* programmability, and nothing about the hard problems — vault security, object discovery, nullifiers, state commitments, batch AMM design, shared-state concurrency, withdrawal integrity, data availability — is a proving problem. Today was an unplanned demonstration: the hours went to a tunnel whose restart loop was its own cause, a note-tree rebuild that looked like a hang, and a signer that exited on one refused fetch. None of that is a circuit | 🔒 direction |
| 131.2 | **The operation list is one circuit, not six.** Transfer, split, merge, deposit claim and withdrawal are the same shape — *prove ownership of an unspent commitment without revealing which*. That is a single membership circuit. The brief lists them separately and concludes custom circuits are the cheaper path; they are cheaper than it says | ✅ |
| 131.3 | **Conservation and range are not circuit problems at all.** §130 built the first half today: Pedersen value commitments sum to the identity, so conservation is a group operation with no circuit, no trusted setup and no proving key. Non-negativity is Bulletproofs, same. What is left needing zero knowledge is *only* membership — which is the precise definition of tier 4, and it is narrower than "prove the transition" | ✅ `zyn-vm/src/value.rs` |
| 131.4 | **Put a `program_id` in the transaction, not only on the VM.** `VM_NAME`/`VM_VERSION` and `vm_id = H(name, version)` (§2.4) already discriminate per *chain*. The brief asks for a discriminant per *transaction* — `PRIVATE_TRANSFER`, `AMM_SWAP`, `ADD_LIQUIDITY` mapping to fixed circuits now, and to a zkVM program hash later. That is a different and better thing, it costs one field today, and adding it after a format is deployed costs a `STATE_VERSION` bump | 🔒 do now |
| 131.5 | **The brief's audit column is backwards and should not be inherited.** It scores custom circuits as a "narrow surface". Narrow *each*, wide in total: six hand-written circuits is six audits, and a circuit bug is silent and catastrophic in a way a VM bug is not, because a zkVM is one engine audited once and reused. Zcash's own 2018 counterfeiting vulnerability was in the circuit. 131.2 is what actually reduces this surface — fewer circuits, not a different technology | 🔒 |
| 131.6 | **This is §2.5 restated, not a new direction.** *Stage 0 compiled-in → Stage 1 rv32im ELF → Stage 2 permissionless* is already a staged path to proven execution that defers committing to one, and rv32im was chosen partly because it is where the zkVM ecosystem is (SP1, RISC Zero). `swapvm::guest` exists for exactly this and says so. The brief and §2 are the same position reached twice, which is worth more than either alone | ✅ |
| 131.7 | **Keep the tier-4 path open in every dependency decision.** `curve25519-dalek` was added for §130's value commitments only after checking `zyn-vm` still builds for `riscv32im-unknown-none-elf --no-default-features`. Anything that breaks that target forecloses proving later, and the check is one command | 🔒 invariant |
| 131.8 | **The demand thesis, stated as a thesis.** The bet is that a collectible whose ownership is *verifiably* committed to Zcash — and private — is worth more than one that is not, and that the object layer is what makes both true at once. It is a bet, not an observation: the register holds four whitelist registrations, and nothing is sold. Claims about whether *other* projects are genuinely on-chain were looked at in September and **not established either way**; they are not evidence for this and must not be cited as though they were | ⏳ unproven |
| 131.9 | **So the order stands: Cave ships on tier 1, the object layer matures behind it.** §130.8a already scoped Cave to the operator-trusted tier. 131.8 is the reason not to invert that — gating the only revenue on a proof system would spend a year answering a question four registrations cannot yet ask | 🔒 |

---

## 132. Listings are encumbrances, and the buyer claims (21 Sep 2026)

Today `PlaceOffer` moves the offered asset into `POT_OFFERS` (`state.rs:75`) —
escrow, protocol-held rather than operator-held, which is the safer of the two
custodial designs. Under the object model it can stop being custodial at all.

| # | Decision | |
|---|---|---|
| 132.1 | **A listing encumbers an object; it does not move it.** The item stays the seller's, and the VM refuses any spend of it except the offer's settlement. Strictly better than escrow on three counts: nobody takes custody, the seller can prove ownership throughout, and cancelling transfers nothing back because nothing was transferred | 🔒 direction |
| 132.2 | **The claim is one atomic transaction.** Buyer's payment objects and the encumbered item go in; payment to the seller and the item to the buyer come out. Nothing is staged, so a buyer cannot pay without receiving, and a cancel that lands first makes the claim fail with the payment still the buyer's. The sequencer's ordering settles the race and no lock is needed for it | 🔒 |
| 132.3 | **The store lights a button; the VM decides a sale.** The store watches for the buyer's credit and enables the claim in the frontend. That is convenience, not authority: it never holds the item, cannot fabricate a sale the VM would reject, and cannot permanently censor one — a buyer able to construct the transaction claims without the button, with forced inclusion (§33) behind that | 🔒 invariant |
| 132.4 | **Listings are attributed to the store, which is the privacy gain.** Every listing reads as the store's, so sellers are indistinguishable from one another; the anonymity set is the number of concurrent sellers. The item id being public costs nothing, because under the object model nobody ever knew who owned it. **This is store-trusted, not anonymous** — the store knows every seller, and that does not improve at tier 3. It improves at tier 4. Nobody may describe it as anonymous before then | 🔒 scope |
| 132.5 | **An encumbered object's commitment-to-nullifier link becomes public at settlement**, because the claim must name the commitment it is entitled to spend. That is the one thing the object model otherwise hides, and it is accepted here precisely scoped: the item was publicly listed and publicly sold, so its trade being traceable is expected. The seller's *other* objects keep unlinkable nullifiers. Wherever "commitments are unlinkable" is written down, this exception belongs beside it | 🔒 invariant |
| 132.6 | **A public listing is first-come.** Anyone who pays the asked price may claim. That is what a listing *is*; a sale to a named buyer is a different instrument and is not this one | 🔒 |
| 132.7 | **The claim hangs on `program_id` (§131.4).** Settlement is a transaction naming `take-offer` as its rule, not a transfer with special-case logic. The VM refuses unknown rules and refuses known rules it does not implement, so supporting this is an act someone takes in a diff | ✅ mechanism exists |

| # | Decision | |
|---|---|---|
| 132.8 | **The AMM waits, and does not gate Cave.** A market for items is an order book, not a pool: §132's encumbrance and claim need no batching, no hidden reserves and no circuit. The pool questions — what the coarsest publication is that still lets someone trade, and whether reserves move per trade or per batch — stay open and stop being on any critical path. Worth noting one finding so it is not re-proposed: **publishing slippage or APR publishes the reserves.** Under `x·y = k` the slippage on a known trade size solves for `x` in one step, and APR over a public fee rate gives the ratio directly. Hiding reserves while showing either is arithmetic, not an implementation detail; what works is coarser granularity — per batch, daily averages, banded slippage — which is batching reached from the trader's side rather than the cryptographer's | 🔒 deferred |

---

## 133. Cave launches on the object ledger (21 Sep 2026)

§130.8a said Cave ships at tier 1 rather than waiting for tier 4, and that
stands. What changes is *which VM it ships on*: `objectvm`, not swapvm — so
items are objects from the first mint and nothing is ever re-minted.

| # | Decision | |
|---|---|---|
| 133.1 | **Cave mints onto `objectvm`.** The alternative was shipping on the account ledger and moving 4,444 items later, which is a re-mint of real holdings with real owners. Doing it before anything is sold costs nothing; doing it after is the expensive version of the same change (§130.3 made this argument about certificates a day earlier and it holds here) | 🔒 |
| 133.2 | **The sale machinery did not have to move.** `sale.rs` holds two references to swapvm and both are type imports; `draw.rs` and `cave-randomness` hold none. The draw, allocator, whitelist and settlement reconciliation were already VM-agnostic, which is why this decision is affordable at all | ✅ measured |
| 133.3 | **`collection.rs` is not needed here, because objects are what it was invented to be.** It compresses items into a Merkle tree since "a distinct asset costs a token entry… a million-item collection is not survivable". An object *is* a leaf in a tree. On the object ledger an item is `asset = collection, amount = 1` and the compression is structural rather than bolted on | ✅ |
| 133.4 | **This ships at tier 2, not tier 1** — better than §130.8a assumed, and still not tier 4. Nobody holding a mirror can build the owner map, because the published sections carry commitments and nullifiers and no owner. The operator still can. The sentence Cave may use is "private from the public; we can see them", and not one word more (§130.12) | 🔒 invariant |
| 133.5 | **Nap must read the object ledger before deposits open.** A store whose buyers cannot see what they bought is not shippable. Today a wallet asks the node what an account holds; against an object ledger that query does not exist, and discovery becomes scan-and-trial-decrypt — the Zcash sync problem arriving inside Zyn | 🔒 gate |
| 133.6 | **A ZEC deposit must create an object, not credit an account** — and that is what makes note encryption the keystone rather than a later nicety. The sequencer creates the note; the depositor has to be able to find it. Deriving the nonce publicly from the txid would let an observer confirm a guessed recipient by recomputing the commitment, so the note is encrypted to the recipient instead and discovered by trial decryption. The same mechanism serves 133.5, which is why A114 comes before the deposit path rather than after | 🔒 |

---

## 134. Tiers 3 and 4 are one step, and the range check is a circuit constraint (21 Sep 2026)

§130 split hiding *amounts* from hiding *which object was spent*, on the
grounds that the first needed no circuit. Attempting it showed that it does.

| # | Decision | |
|---|---|---|
| 134.1 | **Pedersen commitments hide amounts and do not constrain them.** An amount is a scalar modulo the group order, so a negative one is an enormous positive one and a spender may balance `10 = 11 + (−1)` and mint value. `value.rs` carries a test that does exactly that and passes the balance check. The fix is a range proof, and without one tier 3 is not a weaker privacy tier — it is unsound | ✅ demonstrated |
| 134.2 | **The obvious library does not build for the target.** `bulletproofs 5.0.0` depends directly on `clear_on_drop`, whose build script fails on `riscv32im-unknown-none-elf`. Structural to the crate, not a feature flag, and found by running §131.7's check before wiring anything in rather than after | ✅ measured |
| 134.3 | **Zcash does not attach range proofs; it constrains values inside its circuit.** Orchard's Halo 2 circuit checks that a note's value is in range as part of the same proof that checks everything else. Aztec's Noir does the same — a `u64` cast *induces* a range constraint. Range is a constraint, not a proof, and Bulletproofs exist for the case where you have commitments and no circuit | ✅ |
| 134.4 | **So tier 3 folds into tier 4.** Hiding amounts safely needs the circuit that hiding the spend needs, and once that circuit exists the range check is inside it at no extra cost. There is no cheap intermediate rung, and pretending there was cost a day to discover | 🔒 |
| 134.5 | **Build it on Halo 2, which is already vendored.** `halo2_proofs 0.3.5`, `halo2_gadgets 0.5.0` and `pasta_curves` arrive through `orchard` via `zcash_primitives` — the same construction Zcash ships and has had audited. The alternative was hand-writing a verifier, which §131.5 warns is the failure mode that is silent when wrong, and which the circuit would have made redundant within months | 🔒 direction |
| 134.6 | **`value.rs` stays, and so does its refusing verifier.** Pedersen conservation is correct and will be used; only the claim that it is *sufficient* was wrong. `RangeProof::verify` returning `RangeProofsNotImplemented` remains the honest state, and it is what stops a VM wiring this in and quietly accepting spends that mint value | 🔒 invariant |
| 134.7 | **I overstated §131.7 while finding this.** A std-only dependency does not foreclose tier 4 — §131.1 chose fixed circuits, so the `riscv32im` target belongs to the *zkVM* path that v1 defers. Keeping it building preserves an option; it is not a hard gate. The rule is good and was applied as a harder constraint than it is | ✅ corrected |
| 134.8 | **Nothing below this moves.** The object ledger — deposits, transfers, listings, claims, exits, the registry, the wallet — is the layer underneath and is unaffected. Cave still ships at tier 2 (§133.4), where nobody holding a mirror can build the owner map | 🔒 |

**A note on the name.** Two unrelated things are called Noir. Aztec's Noir is
the ZK language referenced in 134.3. **Noir Wallet** (`zknoir.com`) is a Zcash
browser extension by the RHEA Finance team — a wallet over Zcash rather than
an execution layer, per its own SDK docs, and a peer of *Nap* rather than of
Zyn. What its "swaps and lending" do concretely is not stated in its public
docs and is not assumed here.

## §135 — The deposit bridge derives its nonces (21 Sep 2026)

A confirmed Zcash deposit becomes an object whose commitment includes a
nonce. That nonce has two requirements that pull against each other, and the
obvious implementation satisfies only one.

**It must be unguessable.** Everything else about a deposit is public: the
account comes from the memo, the amount from the Zcash transaction. If the
nonce were predictable, a stranger would recompute the commitment and learn
which object on the ledger is that deposit — which is the whole privacy claim.

**It must be the same on every attempt.** A bridge can die between building an
intent and learning whether it landed. This is not a hypothetical: it is the
ordinary case of a process restart during a network call.

Random satisfies the first and fails the second, and the failure is worse than
a retry that doesn't work. The VM dedupes on the Zcash txid, so a second
intent with a fresh nonce is refused as `AlreadyCredited` — correct, and
useless: the object from the *first* attempt is already published, and the
bridge has forgotten the nonce that made it. The holder's wallet scans for
that commitment and finds nothing. The money is in the vault, the credit is on
the ledger, and no party can connect them.

So both secrets are derived (§A116):

```text
  nonce      HKDF(bridge_seed, "zyn.ingress.nonce.v1",     "nonce"     ‖ txid)
  ephemeral  HKDF(bridge_seed, "zyn.ingress.ephemeral.v1", "ephemeral" ‖ txid)
```

Deterministic given the seed, unguessable without it. A restarted bridge
rebuilds a byte-identical intent, which makes resubmitting free and makes
`AlreadyCredited` a proof the work was done rather than a report of loss.

**Keyed on the txid alone.** Adding the account or amount would let one
deposit have two valid nonces if either were ever re-read differently — which
is the failure this exists to prevent.

**The bridge seed is not a wallet seed.** It cannot spend anything; spending
needs the holder's key. It can recompute every deposit commitment the bridge
has ever made and link each to its Zcash transaction, which undoes the privacy
of every deposit at once. It belongs where the vault's other secrets are.

### §135.1 — What this does not yet solve

A deposit for an account that has never registered a viewing key cannot be
credited: the object would be one nobody can open. It is reported as pending
and the ZEC stays in the vault, which is right. But the watcher marks that
height scanned and will not report the deposit again, so the pending list is
the only record the vault owes it — and nothing persists that list. A116's
test found this; A117 fixes it, and it gates opening deposits independently of
the privacy ladder.

## §136 — Getting out of an object ledger (22 Sep 2026)

The account ledger's escape hatch enumerates every balance and proves the list
hashes into a root anchored on Zcash. An object ledger has no enumerable
balance, so that proof has no translation, and a holder would have nothing to
show a custodian if the sequencer went away. A106 makes designing the
replacement a precondition of switching chain 26460 over, not a follow-up.

A holder must establish two things:

1. **The note existed** — its commitment is a leaf of the commitments tree,
   whose root is a section of an anchored root.
2. **It was not spent** — its nullifier is absent from the spent set at that
   same root.

### §136.1 — Unspentness comes from the live set, not from the nullifiers

The first version of this design proved unspentness by showing the note's
nullifier was absent from the spent set. **That is unsound at this tier, and
the audit that found it also demonstrated it.**

A spend carries `(commitment, nullifier)` pairs chosen by the spender. The VM
cannot check that a nullifier derives from the spender's key: it does not hold
the key, and there is no circuit yet to constrain one. So a spender publishes
any nullifier they like. `live` still stops them spending the object twice, so
this is not a double-spend — it is worse. The object leaves the live set while
its real nullifier is never published, and an exit proof that tests absence
then says the note is unspent. **The vault pays for a note that was already
sold.** `a_spend_with_an_invented_nullifier_must_not_leave_an_exitable_note`
reproduces it.

The fix is to prove membership in the **live set**, which is the VM's own
record of what remains unspent. It needs no key, so nothing about it can be
forged by a spender, and it is now a committed section (`SECTION_LIVE`).

Three things fall out of it, all improvements:

- **No nullifier key is revealed.** The earlier design leaked it by necessity;
  this one never asks for it, so exiting no longer costs the holder the
  linkability of their past spends.
- **Registration is no longer a precondition of exiting.** A119 is dissolved
  rather than deferred: it existed only because the proof was bound to a
  registered key.
- **`Op::Register` no longer carries a nullifier-key commitment.** It was
  added for the earlier design and nothing uses it now. An unused consensus
  field that looks like a security check is worse than no field, because the
  next reader assumes it is enforced.

### §136.2 — What the nullifier set is still for

It is how a *wallet* recognises its own spends (`scan.rs`). A spender who
publishes a nullifier other than their own only confuses their own wallet, so
nothing else rests on it.

It is **not** a double-spend audit trail, and §130.10's claim that "anyone can
check we did not double-spend" has to be read against the live set instead: an
object leaves `live` exactly once, and that transition is committed. Tier 3+4
makes the nullifier sound by constraining its derivation inside the circuit,
at which point the two records agree and either can be checked.

### §136.3 — One encoding for the account leaf

`account_record` and `account_leaf` encoded the same thing separately, and
when the leaf grew a field the record did not follow. The conformance suite
caught it (rule S6). They now share one function. Two encodings of one fact is
a drift waiting to happen.

### §136.4 — Who gets paid

The proof names the note's **owner**, and a custodian pays that account's
bound destination rather than whoever presented the proof. That is what makes
an exit proof safe to publish: holding a copy of someone else's buys nothing.

It also means the proof needs no signature. What it proves is a statement
about an account, not a request from a person.

### §136.5 — What a verifier still has to supply

That `anchored` is a root Zcash actually saw. The proof shows its sections
hash to that root; it cannot show the root was witnessed, and nothing
self-contained could. A custodian looks the anchor up.

### §136.6 — Conservation is a sum, so every term must be positive

The audit found three ways to create value or halt the VM, all from the same
root: amounts were checked against *zero* rather than against *positive*.

**A negative output pays for an inflated one.** Conservation is a per-asset
sum of inputs minus outputs. Spend 10, create 1,000 for yourself and −990 for
anywhere, and the sum is zero. The books balance and the spender holds a
hundred times what they put in — spendable, sellable, and exitable against the
vault. `commitment()` refused `Fixed::ZERO` and nothing else, so the note was
committable and entered every tree.

Fixed at that chokepoint: `commitment()` now refuses any amount `<= 0`, and a
note that cannot be committed cannot enter any tree, which covers every path
at once rather than each call site separately.

**A negative withdrawal raises the backed supply.** `apply_withdraw` subtracts
the amount from `issued`, so a negative one adds. It also records a debt of
less than nothing. One of those and the chain will let you withdraw the
difference for real.

**A negative asking price is met by any payment, including none**, because the
claim path checks the seller was paid *at least* the asking price.

**An unchecked `sum()` over attacker-chosen outputs panics.** The claim path
totalled the seller's payment with `.sum()`, which overflows on two outputs of
`i128::MAX`. A panic inside the VM is a halt, not a refusal, and the input
that causes one is free to construct. Now a checked fold that refuses.

The general rule this leaves: **a value that participates in a sum must be
validated as positive at the point it enters the system, not where it is
used.** Zero-checks are what you write when you are thinking about empty
inputs rather than about hostile ones.

### §136.7 — What "anyone can check we did not double-spend" actually means

§130.10 said publishing commitments and nullifiers in the clear lets a
stranger with no key check that no nullifier appeared twice. **That check does
not work**, and finding out why took a wrong turn worth recording.

A nullifier is chosen by the spender. The VM cannot verify it derives from
their key — it holds no key, and there is no circuit yet to constrain the
derivation. An operator retiring the same object twice would simply publish
two unrelated nullifiers, and "no nullifier appeared twice" would pass.

**The wrong fix** was to derive the nullifier in the VM from the commitment,
making it verifiable by anyone. It works, and it destroys a property the
design holds deliberately: `a_published_nullifier_does_not_name_its_commitment`
asserts that a nullifier must not identify which commitment it retired, and
without the owner's key there is no path between them. A VM-derived nullifier
is that path, published for everyone. The test caught it, which is the whole
reason to write a test for a property rather than a behaviour.

**The right check is counting.** Every commitment ever appended is either
still live or has been retired, and retiring one publishes exactly one
nullifier:

```text
    commitments  ==  live  +  nullifiers
```

An operator who retires an object quietly fails it, and so does one who
invents spends that retired nothing. It needs no key, it does not reveal which
nullifier belongs to which commitment, and the counts are in the node's status
output so anyone can run it. `ObjectVm::reconciled()`.

This is weaker than the claim §130.10 made: it proves the books balance, not
that a particular object was spent once. The stronger statement needs the
derivation constrained inside a proof, which is A108. Until then the honest
sentence is "anyone can check the ledger accounts for every object it
created", and that is what the docs should say.

## §137 — A market on the object ledger, hidden from the public (23 Sep 2026)

The §130.11 scope lock said the object ledger would carry transfers and
listings and nothing else until tier 3. It is lifted by direction: A108 waits
for funding, and funding waits on something people use — NFTs first, and if
they do not carry it, launches. So the market is built at tier 2, with the
same honesty line as everything else: **private from the public, visible to
the operator.**

### §137.1 — Batches, at the FM-AMM price

A sequencer that sees orders one at a time can reorder them, and on a
constant-product pool reordering is profit. So a swap is escrowed in the epoch
it arrives in and every order in a pool clears together at the seal, at
`p = (y + 2b′)/(x + 2a′)` — the function-maximising AMM (Canidio & Fritsch
2023; CoW AMM), the price at which the pool's post-trade marginal price equals
the price it traded at. Order within the batch cannot matter, so there is
nothing to sandwich; `x′y′ ≥ xy` for every batch (the gap is
`(x·b′ − y·a′)²`), and each reserve stays above half. The cost is honest and
visible: a lone order pays roughly twice the constant-product impact, and the
difference stays with liquidity providers rather than an arbitrageur.

Minimums are refunds, not partial fills: an order the batch price does not
satisfy is returned whole and the batch repriced without it.

### §137.2 — Claims, because the VM cannot encrypt

A batch settles after the trader has signed and left, and the VM holds no
randomness and no reason to encrypt a note for anyone. So what the ledger owes
is a claim — owner, asset, amount — redeemed by its owner with notes they
encrypted themselves. One redemption path for swaps, refunds, pool shares and
sale allocations.

A claim's id derives from a nonce in the intent that created it, and the
wallet derives its nonces from its key and a counter. That is the whole
discovery mechanism: there is deliberately no "claims for this account",
because that query would answer anyone who asked.

### §137.3 — Launches sell at one price

The whole supply is issued into escrow. Commitments are private until the
close; at the close everyone pays the same price, oversubscription is filled
pro rata and the rest refunded, and under the soft cap everything is refunded
and the token burned — the creator's share with it. On success part of the
raise and a token reserve seed a pool whose shares are **all** locked: nobody
holds them, so nobody can pull the liquidity from under the buyers.

A launch's name is its 32-byte `meta`, read by the wallet as an 8-byte symbol
and a 24-byte name. On chain and public, so every wallet shows the same name
without a server; decoded strictly, so a copycat cannot pad their way into
someone else's.

### §137.4 — What is public, and what the root commits

Public: pool reserves, shares and fee; sale terms and total raised. Committed
only as blinded leaves: orders, claims, commitments, and who created a sale.
The market joins `sections()` only once used, so every root and saved state
from before is byte-identical. Pool shares and launched tokens exist only on
the ledger and cannot be withdrawn to Zcash.

### §137.5 — Sales need the clock

Epochs advance with traffic, and a sale closes at an epoch — so on a quiet
chain a sale could stay open forever. While any sale is open, the node seals
on the clock even with nothing to seal. With none open, it does not, and an
idle chain does not fill its history with empty epochs.

## §138 — The bridge fee and the NFT resale fee feed the ZYN pot (23 Sep 2026)

The object ledger (§130) carried value but none of the fee rail the account
ledger had (§68.6, §69.9). The two parts of it that are not trading fees are
brought across unchanged in shape; the launchpad and pool fees wait for the
product decision on launches (§137).

| # | Decision | |
|---|---|---|
| 138.1 | **Nothing is charged until the operator sets it.** `SetFees` and `RegisterCollection` are operator intents, journalled like any other, so every replica charges exactly what the chain charged — and a chain that never set them replays byte-identically | 🔒 |
| 138.2 | **The bridge fee is `bridge_bps` on a deposit, taken from it, and on a withdrawal, added on top** (§30's 50 bps is the starting rate). The fee is ZEC.zy that stays issued and backed and belongs to the ZYN pot. A deposit credited without the fee is refused once one is set | ✅ |
| 138.3 | **The ZYN pot is keyless and public** (§71.23): no intent spends from it, and `zyn_market` reports it beside the schedule so the fee can be checked against what was kept. It is the object ledger's future-ZYN-POL pot (§68.6); its release is §71.22's open question, unchanged | 🔒 |
| 138.4 | **A registered collection charges `fee_bps` on each side of a resale, in its quote asset** — the buyer on top, the seller out of the proceeds, so the listed price is the price — split 50% ZYN pot, 40% the collection's redemption floor, 10% creator (§69.9), remainder to the pot. Cave collections: 100 bps a side, creator Treasury 26460 | ✅ |
| 138.5 | **Terms are set once.** A holder prices an item against its collection's terms, so they do not move under them; `RegisterCollection` on an existing collection is refused | 🔒 |
| 138.6 | **The floor is redeemable, or it is not a floor.** `Redeem` burns items for `floor × burned / outstanding`, as a claim | ✅ |
| 138.7 | **Ceilings in consensus:** 2% bridge, 10% a side on resale | 🔒 |
| 138.8 | **An item is its own asset, `item_address(collection, serial)`** — the address the manifest commits to (§86.2). §133.3's `asset = collection, amount = 1` made items fungible within a collection: after one sale or send nothing, not even the VM, knew *which* item a holder had. With one asset per item, conservation carries identity through every transfer. Supersedes §133.3 | 🔒 |
| 138.9 | **The primary sale delivers with `MintItem`**, operator only, sealed to the buyer's registered viewing key (`primary::mint_item`, `zyn_mintItem` on the operator's token). A serial mints once, and never past the collection's **cap**, fixed at registration: minting past what holders were promised dilutes their share of the floor | 🔒 |
| 138.10 | **Collection items sell through `List`, paid by claim.** The listing publishes the item's nullifier (§132.5) and names no seller; the buyer's outputs are the item and their own change; the seller's proceeds (price less their side of the fee) wait as a claim derived from a nonce only they hold. `PlaceOffer` refuses a collection's items so there is one sale path to get right | ✅ |
| 138.11 | Items cannot leave for Zcash: they are made on the ledger and nothing in the vault backs them | 🔒 |

## §139 — Nap holds ZEC in the browser (23 Sep 2026)

Browser Nap held Zyn tokens and items and sent ZEC holders to desktop Nap for
Zcash itself. It now holds shielded ZEC the way any Zcash wallet does: sync,
balance, receive, send.

| # | Decision | |
|---|---|---|
| 139.1 | **One wallet core, two builds.** The browser runs the desktop's own `zyn_custody` scanning, note trees and payout builder, compiled to WebAssembly (`apps/nap-web/zec-core`, crate `nap-zec`). No second Zcash implementation in JavaScript. `zyn-custody` gains a default `multicore` feature; the browser build turns it off and proves on one thread | 🔒 |
| 139.2 | **The page carries the frames; the core decides.** The core builds every `zyn-lightd` request and checks every reply (`lightd::request`/`lightd::decode`, shared with the desktop's TCP client). The page only posts bytes. Nothing it sends names a key or a shielded address | 🔒 |
| 139.3 | **`zyn-lightd` gets an HTTP door** (`ZYN_LIGHTD_HTTP`): the same request frame as a POST body, the same reply frame back. Loopback only, behind the gateway's TLS, CORS and rate limit; the page reaches it through the same-origin `/zec/<network>` Pages Function, as it reaches `/rpc` | ✅ |
| 139.4 | **Same phrase, same keys, same state format.** ZIP-32 Orchard account from the BIP-39 seed, exactly as `KeyMaterial::spending_key`; tests on both sides pin the same vectors. The scan state is the desktop's `ZYNWAL01` bytes | 🔒 |
| 139.5 | **The spending keys are sealed with the rest, never the phrase.** A kept wallet adds both networks' Zcash keys to its sealed record; one sealed before this asks for the phrase once, checks it opens *this* Zyn account, and reseals. The scan state sits in IndexedDB under AES-GCM keyed from the spending key, under a derived name: a locked device shows nothing about its ZEC | 🔒 |
| 139.6 | **A Zcash address gets a Zcash transaction; a Zyn account gets Zyn tokens and items.** The Zcash section sends only to Zcash addresses and turns a Zyn account id away to Coins/Items. Moving ZEC into Zyn is an ordinary payment to the account's own deposit address ("Pay it from my ZEC") | ✅ |
| 139.7 | **Proving runs in a Web Worker,** about 21 s for a two-action payment including building the key (measured on testnet, Node, one thread). The page stays live; the send says to keep the tab open | ✅ |
| 139.8 | **Where a scan starts.** A phrase created in the browser starts at the tip. A restored one starts at the network's `zecBirthday` (config: mainnet 3,440,000, testnet 4,320,000); "Scan from an earlier block" reseeds from any height | ✅ |

## §140 — Nap connects to sites: Nap Connect (23 Sep 2026)

Sites on Zyn and Zcash can find Nap and ask it for things. Surveyed first:
Ethereum wallets announce themselves with EIP-6963 events and gate accounts
behind `eth_requestAccounts`; web-only wallets (Coinbase Smart Wallet's
keys.coinbase.com and the like) open their own origin in a popup and talk by
`postMessage`; Zcash has no connection standard — MetaMask's Zcash snap
serves only its own site (`getViewingKey`, `signPczt`), and a
WalletConnect-for-Zcash grant was declined in May 2026 — but Zcash wallets do
share ZIP-321 payment requests.

| # | Decision | |
|---|---|---|
| 140.1 | **One provider interface for every Nap:** `request({method, params})`, EIP-1193 error codes, one method set (`apps/nap-connect/README.md`). A site written against the extension works against the browser wallet unchanged | 🔒 |
| 140.2 | **Discovery is EIP-6963's shape under our own names:** the extension answers `nap:requestProvider` with `nap:announceProvider {info, provider}`; `window.zyn` stays for older pages | ✅ |
| 140.3 | **Nap in the browser connects through its own window** (`/connect.html`, popup + `postMessage`). Nap answers only its opener, at the origin of the opener's first message; the site listens only to Nap's window at Nap's origin; the page refuses to run framed, and `_headers` forbids framing everywhere | 🔒 |
| 140.4 | **Connect before anything about the holder.** `nap_connect` is approved once per site and shows the Zyn account and a Zcash address — never balances, notes or keys. Any read of the holder's data, or any write, from an unconnected site prompts the connection first (as `eth_requestAccounts` does). Public order-book reads need nothing. This closes the extension's old gap, where any page could read `zyn_account` | 🔒 |
| 140.5 | **Writes are prepared from the chain, not the site.** An approval shows the listed price, the item, the ZIP-321 amount and memo as Nap reads them; a site's `pay` that disagrees with the listing is refused | 🔒 |
| 140.6 | **ZEC requests are ZIP-321** (`zec_pay {uri}`), single-recipient; a multi-payment or unknown `req-` request is refused whole. One parser (`nap-web/src/zip321.js`) serves the browser wallet, its Send field and the extension build | ✅ |
| 140.7 | **The Cave store connects through Nap Connect:** a header button and picker (extension, Nap in the browser). It reads only the public order book itself (`zyn_offers`, no credentials) so anyone can browse; everything about the holder goes through their Nap. A localhost-only `?nap=` override exists for development and is not read on the real domain | ✅ |

## §141 — Pump.fun-style launches on the object ledger (23 Sep 2026)

The user chose a pump.fun-style launchpad and adopted the fee package. Studied
first: pump.fun's virtual constant product, §69's linear curve, and shld.fun,
a live launchpad on Zcash. Its "ZSA" tokens are not native assets but an
operator-kept ledger whose events ride ordinary Ironwood payments — 332
tokens in 16 days, 18 graduated, 244 of the rest under 1% sold, 24 h volume
7.5 ZEC (99% one token), no creator rewards, 3% buy fee.

| # | Decision | |
|---|---|---|
| 141.1 | **One curve for every token, in consensus** (`objectvm::curve`): 1B supply in 80,000 lots of 12,500; lot i costs 7,721 + i zatoshi; graduation at lot 51,098 — 17.00005 ZEC raised, 47 ZEC FDV, 7.62× from the start, pump.fun's shape in dollars at today's ZEC. A creator cannot tune it against buyers | 🔒 |
| 141.2 | **Linear, not virtual constant product, because of batches.** With each epoch's buys at the average price over the lots they take, the raise to graduate is exactly the integral however the flow arrives, so the pool (the unsold 28,902 lots and everything raised) opens within 0.0011% of the curve's last price. Virtual constant product under FM-AMM batching makes the raise path-dependent (20–42 ZEC for a 20 ZEC target) | 🔒 |
| 141.3 | **Each epoch settles at the seal, sells then buys, each side at one average price, buyers pro rata by budget.** Order is the VM's rule, never arrival, so neither the sequencer nor a bot gains by ordering; one epoch can never buy past graduation, and the crossing epoch fills to the end and refunds the rest. Every rounding favours the curve: the reserve always covers buying back every lot sold | 🔒 |
| 141.4 | **Tokens exist only as bought:** issued on a buy, burned on a sell, the remainder issued into the pool at graduation. No creator allocation; the creator buys like anyone | ✅ |
| 141.5 | **Fees (adopted package), operator-set with `SetCurveTerms`, fixed per token at creation:** a fee on the quote side of every curve trade (on top of a buy, out of a sell), split 50% ZYN pot / 40% the token's graduation liquidity / 10% creator; a creation bond. Creator share and bond are paid only at graduation. Ceilings 2% and 1 ZEC.zy. Graduated pools 30 bps, kept by the pool. **Testnet 212: 100 bps, no bond (23 Sep). Mainnet: not set** | ✅ |
| 141.6 | **Graduation is atomic:** a locked pool (every share, nobody holds any), the creator's escrow and bond paid, the curve closed. The graduation pot joins the pool, which opens slightly above the curve's last price — buyers' fees deepen the market rather than the operator | 🔒 |
| 141.7 | **A curve token cannot be pooled before graduation** — the pool it graduates into cannot be squatted — **nor withdrawn** (it is derived) | 🔒 |
| 141.8 | **Nap-only.** Tokens live on Zyn and no other Zcash wallet can show them, so there is no "buy from any wallet" path. Sites reach Nap through Nap Connect: `zyn_curves` (public), `zyn_curveHoldings`, `zyn_curveCreate`, `zyn_curveBuy`, `zyn_curveSell`, `zyn_swap`, `zyn_collect` | ✅ |
| 141.9 | **Cave Jade is the launchpad's site:** the public yard read straight from the chain (anyone can browse), every write through Nap. Its COOP becomes `same-origin-allow-popups` so Nap's window can answer; `connect-src` admits only the chain's RPC. §69.35's provider-only rule is superseded for public reads | ✅ |
| 141.10 | **Charts come from the node, derived, not consensus** (`zynzapd::market_history`): after each epoch the node records a point per token whose curve or pool moved — lots sold, price, ZEC set in it, and the epoch's net change — to `chain-<id>.history`; `zyn_history {token, since}` and `zyn_curveStats` serve it. Batches clear at one price, so the finest grain is an epoch's net movement and no wallet ever appears. A token younger than a window reports its change since it was made. History starts when a node first runs this code | ✅ |
| 141.11 | **Market cap is the headline** (price × 1B, in ZEC and in USD); candles of market cap at 1m–1d; a bonding-curve progress bar; age; sorts Movers (default — never the empty tab), New, Market cap, Last trade, About to graduate, Graduated. Pump.fun without its social layer: no comments, no holder list — holders are private and the page says so | ✅ |
| 141.12 | **Pictures and descriptions are off chain, attached once by the creator with a proof.** Nap returns, with a new token, an ed25519 signature over `"cave.jade.content.v1" ‖ token ‖ sha256({description, image, v:1})`; Jade's store (Pages Function + KV) checks that the key's account and nonce derive the token, keeps the first attachment only, limits size and type (data-URL PNG/JPEG/WebP/GIF, ≤300 KB after shrinking in the browser; 400 chars), rate-limits by IP, and has an operator hide list. The chain carries only the ticker and name | ✅ |
| 141.13 | **Trades carry floors and settle themselves.** Buys send `minTokens`, sells `minZec` (slippage 1–10%); after the seal the page calls `zyn_collect`, which Nap now serves to a connected site without a prompt — it only moves what the ledger already owes the holder. `zyn_curveHoldings` adds the ZEC.zy balance and, for its holder only, the tokens they made | ✅ |

## §142 — Rarity classes for Collection Z (23 Sep 2026)

The 4,444 had no rarity (`collection.json` said "none; controlled family distribution only"). The user asked for classes where the traits are the clue, and approved these after seeing each class as a grid.

| # | Decision | |
|---|---|---|
| 142.1 | **Scored from traits alone:** an item's score is Σ −log₂(share of the 4,444 with that value) over Species, Background, Body, Eyes, Mouth/Beak (one slot), Headgear, Interactive Light. Visual Family is left out — it names a production batch, not something a holder sees | ✅ |
| 142.2 | **Classes by rank, fixed sizes:** Mythic 44 (1%), Legendary 178 (4%), Epic 445 (10%), Rare 888 (20%), Uncommon 1,334 (30%), Common 1,555 (35%); ties by collection index. Species spread: Mythic 22 Mole / 20 Owl / 2 Moth | ✅ |
| 142.3 | **Moths are not rarer by traits.** Moths have far fewer variants per slot (9 bodies against the moles' 22), so each moth trait is widely shared and cancels the species' scarcity — 45% of moths land in Common against 34% of moles. Accepted as what the traits say | ✅ |
| 142.4 | **Rarity is in the committed metadata:** `Rarity` and `Rarity Rank` attributes enter each manifest preimage. With the re-cut images of 142.9 and the replacements of 142.10–142.11 the manifest root moves `02b01b16…` → `0c56593e1b16bd513a4c88baf31d42602acd89db85aef2b0dcbc4e4bce108426`; policy digest → `a572a747…0b13`; terms digest `159b454e…` → `f2c3242060baf60e0c193b41dc726b6677d85c158a62ba200c948d48a177a59e`. Item addresses and the collection id do not move (they derive from the index). Done before any entry existed; the terms page says so | ✅ |
| 142.5 | **Rarity cannot go stale silently:** `tools/assign_rarity.py` derives it; the store validator refuses metadata whose rarity no longer matches its tags; `RARITY.json` is the copy kept in git | ✅ |
| 142.6 | **Known weak spot:** 34 items hold a trait seen on ≤3 items, several tagged at low confidence (mole mouths "Friendly Smile", "Glowing Grin"; "Zero Field Halo"), and they dominate Mythic. A human look at those 34 before the sale would stop a mislabel from minting a Mythic | ⏳ |
| 142.7 | **1,373 images had slicing defects repaired at their edges** (`tools/repair_slicing.py`, every change in `IMAGE-REPAIRS.json` with old and new hashes): 444 with a dark gutter line (mostly the bottom 1–3 px) extended from the first real line; 1,018 with streak bands — ≥8 px of one repeated edge column that has detail along it — inpainted from the surrounding pixels. Mirroring was rejected (it made mirrored half-characters). Nothing inside the art changed, so traits and reviews hold; checksums and catalog hashes follow the new bytes. Moth #0668, overwritten by a thumbnail sheet on 23 Sep, was restored from its manifest blob | ✅ |
| 142.8 | ~~**209 wide bands (≥16 px) are improved, not clean**~~ superseded by 142.9 (re-cut, no bands). — Mole 0598–0747 and 1347–1356 are almost whole source boards cut too narrow, plus 32 owls. At full size their sides read as soft smears. The clean fix is new art or a swap for a reserve cell (mole 68, owl and moth reserves exist), which changes traits and rarity. Also open: near-twin moles 1624/1649/1674, 2097/2117/2134, 2179/2209, 1690/1727, 2163/2189, 2176/2206, 0452/0457 | ⏳ |
| 142.9 | **The Moles and Owls are re-cut from their source canvases, square by cropping** (`tools/reslice_square.py`, `RESLICE.json`), replacing 142.7's edge repairs for those species. The same measured cells (proven: each rebuilds its original image exactly with the old exporter); flat gutter slivers trimmed; a tall cell loses its extra height — Moles half from the top, Owls 15% (their ear tufts sit high) — instead of being padded with repeated edge pixels. No streaks remain anywhere. Same portraits, same traits; framing slightly tighter. Moths were never padded and keep 142.7's 12 frame-line fixes. The user also saw a code-drawn layered alternative and preferred the original art | ✅ |
| 142.10 | **Nine near-duplicate Moles replaced from a new 3×3 board** (`canvas-34`, the user's; builder `DUPLICATE_REPLACEMENTS`): 0457, 1649, 1674, 1727, 2117, 2134, 2189, 2206, 2209 — the later members of 0452/0457, 1624/1649/1674, 1690/1727, 2097/2117/2134, 2163/2189, 2176/2206, 2179/2209; the first of each group stays. Each board slot keeps its group's family (geode, rail, root, cipher, moon, prism, shielded, gold vein, node) with directly reviewed tags. Rarity from those tags: seven Mythic (ranks 1, 2, 3, 4, 6, 8, 19), 2206 Legendary 49, 2209 Legendary 158. Both builders now finish with the square re-cut and rarity, because a plain rebuild had silently restored the padded slices | ✅ |
| 142.11 | **Five more near-twins replaced from a second board** (`canvas-35`, "Duplicate Replacement Board B"): 1943 (twin of 1887), 2133 (2110), 2219 (2095), 2111 and 2128 (2097). Rarity from their tags: 2111 Mythic 3, 1943 Mythic 4, 2219 Mythic 7, 2133 Mythic 12, 2128 Legendary 53. The near-duplicate scan finds no same-template, same-colour pairs left | ✅ |
| 142.12 | **Four wild cards held outside the 4,444, for the team** — board 35's remaining cells, cut by the same rules into `mole-wild-cards/` (tracked record `wild-cards.json`; metadata `Edition: Wild Card`). CAVEZ's cap is 4,444 on chain and in the terms, so they are not CAVEZ items: delivering them means their own small collection | ✅ |
| 142.13 | **Published 24 Sep 2026: the art and the revised terms.** All 8,888 Collection Z blobs (4,444 images, 4,444 manifests) at `cavebro.win/blobs/<hash>`, each checked against its name before staging and all 8,888 answering live; `cavebro.win/terms` and `cave-sale-terms-v1.hex` carry root `0c56593e…8426` and terms digest `f2c32420…a59e`, recomputed from the live bytes. **The collection is now frozen**: any change to an image, trait or rarity is a different sale. Still to do before the door opens: register CAVEZ on 26460 (operator, the user's activation script), stamp the digest (A96), whitelist codes and the sale service (A100) | ✅ |

## §143 — ZynScan: the ledger replayed, in public (24 Sep 2026)

The user: transactions should at least be visibly on chain, even before Zcash carries them — still more than other Zcash projects show.

| # | Decision | |
|---|---|---|
| 143.1 | **`zynscan` is rewritten for the object ledger** (the old binary served the retired AMM chain's anchors). It replays the node's own journal files through `ObjectVm` — the replica's rules, nothing interpreted by hand — incrementally, and serves a read-only GET API: summary, epochs, one epoch, transactions, one transaction, the public market | ✅ |
| 143.2 | **It checks, not just shows:** the summary carries the root it replayed and the node's `state_root`, and `agrees` when both are at the same sequence. Verified on the devnet, testnet 212 and mainnet 26460 | ✅ |
| 143.3 | **Tier 2 holds (§130.12, §133.4).** For a holder's operation it publishes the kind, accepted or refused, notes created and spent, and at most a public object id (a coin, a pool, a collection) — never an account, an amount, an owner or a destination. Operator operations (fees, collection registration, curve terms, item mints' collection and serial, exits' Zcash txid) are public by nature and shown whole. A test pins that a curve trade's trader and amount never appear. Account pages were not built: collecting one account's history is the thing tier 2 exists to prevent | 🔒 |
| 143.4 | **Settlement is stated as it is.** An epoch shows "on Zyn" until an anchor carries it, then the Zcash height and txid. Testnet 212 is anchored through epoch 22; mainnet waits on a funded vault | ✅ |
| 143.5 | **Where it runs:** `zynscan-mainnet` on Contabo (127.0.0.1:8093), `zynscan` on Hetzner (127.0.0.1:8093, forwarded by the object tunnel to Contabo :8593); nginx `zyn-rpc.cavebro.win/scan/` and `/scan-testnet/`, GET only, CORS from the server block. Site: `zynscan.pages.dev` (`apps/zynscan`), CSP pinned to the gateway | ✅ |

## §144 — Collection Z's sale on the object ledger (24 Sep 2026)

The user: rebuild the sale for the new chain — buyers do not pay a separate Zcash entry; they deposit ZEC, it is bridged, and they buy with it. Keep the lottery. Claims must not depend on trusting the operator.

| # | Decision | |
|---|---|---|
| 144.1 | **Tickets are bought on Zyn, in ZEC.zy** (`BuyTickets`, tag 27): 1–20 per buy, at the terms' entry price (0.01). No Zcash payment to watch, no confirmations, no reservation codes — a ticket exists the moment its intent is accepted. The whitelist page stays a sign-up list; it gates nothing | ✅ |
| 144.2 | **A ticket id is `H(buyer, nonce, k)`** from a market nonce only the buyer's key derives, so the chain's ticket list (`zyn_tickets`) names no one, and a restored wallet finds its own tickets by asking for the ids it can derive (both Naps: first ticket per nonce, then the rest of each buy) | 🔒 |
| 144.3 | **Money:** 10% of every sale payment (ticket, claim, open mint) is a claim to the collection's creator (the treasury); 90% goes to the collection floor that redemption pays out of. The sale object holds no value itself — conservation counts floors and claims | ✅ |
| 144.4 | **The lottery is the published one** (`cave-randomness`, policy `a572a747…0b13`, weights 1:80 / 2:19 / 3:1, concealed shuffle over the frozen manifest `0c56593e…8426`). Each Zyn ticket stands in the engine's ticket where a Zcash deposit stood: its id is the deposit txid and recipient, its place is the tx index, and its height is the close. `zynzapd::sale_draw` is the whole mapping | ✅ |
| 144.5 | **The close is a Zcash height announced in advance** (§104). `zyn-sale-draw close` posts `CloseSale` with that block's hash once Zcash reaches it; the chain refuses tickets after it. **It also refuses to close if the reveal's drand round is already public** — the engine fixes that round from the announcement, not from the close, so an announcement naming too early a round would leave only the Zcash block hidden while tickets were still on sale | 🔒 |
| 144.6 | **The draw is posted as a Merkle root** (`PostDraw`): one leaf per winning ticket, `H("objectvm.draw-leaf.v1", ticket, serials)`, in ticket order. The full result is served beside the chain (`zyn_saleDraw`); both Naps rebuild the root from it and use it only if it matches the chain's. Anyone with the tickets, the announcement, the drand beacon and the Zcash reveal block can re-run the draw and get the same root | ✅ |
| 144.7 | **Claims are trustless:** `ClaimItems` names the ticket (by nonce and k, so only its buyer can), the serials and a proof against the posted root; the chain checks the proof, takes the claim price (0.005 each), and mints each item straight to the buyer as a sealed note. The operator cannot claim for anyone, and cannot withhold a claim once the root is posted | 🔒 |
| 144.8 | **The claim window is the terms' 16,128 blocks after the close** (as published; the first build counted from the reveal block and was corrected 24 Sep). After it, `zyn-sale-draw open-mint` posts the order the rest sell in: what the draw handed nobody, in the engine's revealed order, then unclaimed wins in draw order — both fixed by the reveal, so nobody chooses what sells next. `MintOpen` takes the next place with a proof against that root, at the open price (0.0133) | ✅ |
| 144.9 | **Where it lives:** ops 26–32 in `objectvm::sale`; the operator's runner `zyn-sale-draw` (status/open/close/draw/open-mint; token read from the node's env, never printed); node RPCs `zyn_market.sales`, `zyn_tickets`, `zyn_saleDraw`; Nap Connect `zyn_saleInfo`, `zyn_saleTickets`, `zyn_buyTickets`, `zyn_claimItems`, `zyn_mintOpen` in Nap in the browser and (through the desktop app) the extension; `cavebro.win/mint` rebuilt on them; ZynScan names each step. The retired checkout (`/api/sale` relay, receiving accounts) is no longer used by any page | ✅ |
| 144.10 | **A rehearsal policy** (`cave_z::rehearsal_policy`) keeps the published odds, shuffle, manifest and prices with minutes of notice instead of a day. Its digest differs, so nothing is ever sold under it. Rehearsed end to end on a local devnet with real testnet Zcash blocks and real drand rounds (24 Sep) | ✅ |
| 144.11 | **Consensus change, deployed 24 Sep 2026** (commit `09f37ac`): tags 26–32 and the sale section of the state root, present only once a sale exists. Upgraded in replay-first order — mainnet replica (Windows WSL), testnet replica and mainnet ZynScan (Contabo), testnet ZynScan (Hetzner), then the testnet node and the mainnet node. Both nodes restarted at the same sequence with the same root (212: seq 97 `b9a9498e…0d80`; 26460: seq 7 `bfcec476…646b`), both ZynScans agree, `zyn_tickets`/`zyn_saleDraw` answer on both gateways. `zyn-sale-draw` is installed on Hetzner and Contabo. Old binaries kept as `*.bak-20260924-sale`. **Opening a sale is the user's call** | ✅ |

## §145 — Burn a Solana token for Collection Z tickets (24 Sep 2026)

The user: launch a memecoin on Solana (pump.fun, paired with ZEC) for attention, and connect it to Zyn so the attention becomes activity on Zyn — burning the coin should get people into the draw, beside paying ZEC.zy.

| # | Decision | |
|---|---|---|
| 145.1 | **Two ways onto one ticket list:** pay ZEC.zy (`BuyTickets`) or burn the token on Solana. A burn becomes ordinary tickets in the same order, drawn at the same odds; nothing in the lottery, the terms digest or the draw changes | ✅ |
| 145.2 | **The burn's memo carries `zyn:<commitment>`**, `H("objectvm.voucher.v1", account, secret)` with the secret one of the wallet's market nonces. It names no Zyn account. The claim reveals the secret *from that account*, so a revealed secret lets nobody else claim the burn's other tickets (tested) | 🔒 |
| 145.3 | **Tickets exist when the burn is posted, not when it is redeemed** (`PostVoucher`, tag 34, operator): nobody burns and then misses the close by forgetting a step. Ids are `H("objectvm.voucher-ticket.v1", commitment, k)`; Nap finds them by the same scan of its nonces that finds bought tickets | ✅ |
| 145.4 | **A burn counts once** — the chain records every burn id, `H("objectvm.solana-burn.v1", signature)`, and refuses a second post; a commitment counts once too. Anyone can recompute the ids from Solana, so "Zyn burn tickets = Solana burns ÷ rate" is checkable | ✅ |
| 145.5 | **Burn tickets pay nothing into the floor, and a sale caps them** (`SetVoucherCap`, tag 33; none until set; never below what is posted). The user did not choose between treasury-funded and free-and-capped, so the default is free, capped and stated on the page. A winning burn ticket still pays the claim price per item — which is also what brings its holder's ZEC into Nap | ✅ |
| 145.6 | **Trust, stated plainly:** Zyn cannot verify Solana, so the operator's `zyn-burn-watch` decides which burns are posted (finalized, successful, the one mint, a `zyn:` memo, whole tickets at the published rate, at most 20 per burn). The draw and the claims stay trustless (§144) | ✅ |
| 145.7 | **Burns after the close, past the cap, or under one ticket earn nothing** and are not refunded — the watcher logs them, the page says so before anyone burns, and the operator runs the watcher once right before `close` so a burn that landed in time is counted | ✅ |
| 145.8 | **The burn is built by the store, signed by the holder's wallet:** `solana-burn.js` lays out one legacy transaction (`burnChecked` + memo) and hands the bytes to any Wallet Standard wallet (7e.7) — no Solana SDK. The page reads only `getLatestBlockhash` and the holder's `getTokenAccountsByOwner` for the one mint, straight from Solana (the burn config's `rpc`, or the public endpoint — which refuses Cloudflare's workers, so a relay was tried and dropped); the policy's `connect-src` names exactly those origins, built from the configs. The layout was accepted and finalized by Solana devnet (24 Sep) and is pinned in a test | ✅ |
| 145.9 | **Where it lives:** ops 33–35 in `objectvm::sale` (snapshot `MARKET06`; `MARKET05` still loads; the sale section of the root grows a burns root — no live chain has a sale yet, so no root changes); `zynzapd::burn_watch` + `zyn-burn-watch` (status/cap/once/run); Nap Connect `zyn_burnMemo` (web and desktop/extension), burn tickets in `zyn_saleTickets` and `zyn_claimItems`; `cavebro.win/mint` shows the burn button only when `/burn.json` (mainnet) or `/burn-testnet.json` exists; ZynScan shows "Burn posted" | ✅ |
| 145.10 | **Consensus change, deployed 24 Sep 2026** (commit `83688f2`; tags 33–35, `MARKET06` state): replay-first order again — mainnet replica (Windows WSL), testnet replica and mainnet ZynScan (Contabo), testnet ZynScan (Hetzner), then both nodes. Both restarted at the same sequence and root (212: seq 97 `b9a9498e…0d80`; 26460: seq 7 `bfcec476…646b`); both ZynScans agree. `zyn-burn-watch` is on both boxes, idle until a mint is set. Each node's pre-upgrade state file is kept as `*.object-state.bak-20260924-burn` — the previous binary cannot read `MARKET06`, so a rollback restores both. **Still the user's:** the token, its rate per ticket and the cap | ✅ |

## §146 — SOL.zy and ETH.zy on the object ledger, testnet first (24 Sep 2026)

The user: for the Colosseum narrative, bridge SOL and ETH (Ethereum L1) as well as ZEC; judges need testnet/devnet (Solana devnet, Ethereum Sepolia); about three weeks; "as long as there's some proof would be enough".

| # | Decision | |
|---|---|---|
| 146.1 | **Bridged assets are private notes, like ZEC.zy**, named by §66.13 from public facts: SOL.zy (devnet) = `bridged("solana-devnet", "")` = `9204324e…2ad1`; ETH.zy (Sepolia) = `bridged("eip155:11155111", "")`. Mainnet SOL and ETH get their own ids later; a test coin can never pass for a real one | 🔒 |
| 146.2 | **`RegisterBridged` (tag 36, operator)** records an origin from the frozen registry (ported from the AMM chain's validator) with its decimals. Only registered assets take committed deposits; ZEC.zy keeps its own path | ✅ |
| 146.3 | **Deposits carry a commitment, not an account.** Other chains have no per-account shielded address, so a deposit carries `H("objectvm.deposit.v1", account, secret)` (the secret one of the wallet's market nonces) — a Solana memo `zyn:<hex>`, or the `bytes32` `ZynVault.depositNative` already takes. `DepositCommitted` (37, operator) records it once by its reference, issues the gross, sends the bridge fee to the pot and keeps the rest waiting under `H(commitment, asset)` (counted in `holdings`, so conservation holds); `ClaimDeposit` (38) takes it as a note by revealing the secret *from the committed account* | 🔒 |
| 146.4 | **A fresh code per deposit:** Nap reuses a code only until a deposit with it lands, so two deposits are not linked by their memo. Waiting deposits are found by the same nonce scan as claims and are claimed on their own (Nap Bridge, and `zyn_collect`) | ✅ |
| 146.5 | **Exits are the ordinary `Withdraw`**, committing to `keccak(address ‖ salt)` on the destination chain; the holder reveals the address with `zyn_bridgeReveal` (accepted only when it reproduces an unpaid exit of that asset) and the chain's bridge reads it through `zyn_bridgeExits` (operator token). Paid exits are recorded with `SettleExit`, the Solana signature's hash or the Ethereum transaction hash | ✅ |
| 146.6 | **Solana: `zyn-sol-bridge`** reuses the devnet vault `7KacSY…KZmA` — a FROST ed25519 2-of-3 key — and its durable nonce `BrRELA…d61H`. Payouts are built and signed through `custody::prepare_with` by the custodians, rebroadcastable and never repeated; the network fee comes from the exit. Rehearsed 24 Sep on real devnet: 0.05 SOL in by Nap's code, claimed; 0.02 out, signed with custodian 3 offline, finalized, settled; vault and issued agree | ✅ |
| 146.7 | **Ethereum: `ZynVault` unchanged, `ZynSigners` as its signer** — a fixed 2-of-3 EIP-1271 contract (threshold signatures ascending by owner; 3 Foundry tests beside the vault's 20). `zyn-eth-bridge` credits finalized `Deposited` logs by the vault's per-asset index and pays revealed exits; it computes the digest in the contract's encoding and **refuses to sign unless the vault's own `withdrawalDigest` agrees** (the old Rust digest typed the asset `bytes32`, the contract `uint256`). Rehearsed on anvil: 1 ETH in, claimed; 0.4 out, paid exactly, settled | ✅ |
| 146.8 | **"Some proof" custody, stated:** the SOL shares are the existing devnet custodians (two on Hetzner, one reachable by tunnel — §36.7's concentration is unchanged for devnet); the three ETH owner keys and the gas payer are files on one box. Both vaults are threshold by construction, so spreading the keys later changes no code and no address — ETH needs a new `ZynSigners` and vault, SOL none | ⚠️ |
| 146.9 | **Nap Bridge** (`/bridge.html`): deposit from a Solana wallet (Wallet Standard, a transfer + memo laid out by `src/solana.js`, accepted by devnet and pinned in a test) or an Ethereum wallet (EIP-1193, `depositNative` with the code); send out to a Solana or `0x` address. Its CSP adds only Solana devnet's RPC. ZynScan shows "Bridge deposit" (asset only) and "Bridged asset registered" | ✅ |
| 146.10 | **Mainnet waits:** both bridge binaries refuse mainnet (`zyn-sol-bridge` through the custody RPC's own guard; `zyn-eth-bridge` refuses chain 1) until custody is federated | 🔒 |
| 146.11 | **Consensus change deployed 24 Sep 2026** (tags 36–38, `MARKET07`): nodes, replicas and ZynScans upgraded in replay-first order, both roots unchanged (212 seq 97, 26460 seq 7). **SOL.zy (devnet) is live on testnet 212**: registered, `zyn-sol-bridge.service` on Hetzner. Public round trip through the live services: 0.02 SOL in by a fresh code, claimed as 0.0199 (50 bps bridge fee); 0.01 out, threshold-signed, paid 0.009995 (network fee from the exit), settled; vault +0.02 −0.01, issued 0.01, testnet ZynScan agrees. **ETH.zy (Sepolia) is live too** (24 Sep): `ZynSigners` `0x95cb…fa41` (2 of 3), `ZynVault` `0xe17b…ef27`, `zyn-eth-bridge.service` on Hetzner. Public round trip: 0.01 ETH in by a fresh code (Sepolia block 11772911), claimed as 0.00995; 0.004 out, signed 2-of-3, paid exactly in `0xcde64afb…b558678`, finalized and settled; vault 0.006 = issued 0.006; ZynScan agrees. Lesson: never `source` the node's env file in a shell — `ZYN_DA_MIRRORS` holds `url|token`; systemd parses it, a shell pipes it | ✅ |
| 146.12 | **Operations, 24 Sep:** the node no longer reads the vault's note stores under its lock (A162; deployed to both nodes, 0 slow calls of 60 on testnet during catch-up, roots unchanged). The DA mirror token was rotated on the mirror and in all four env files after being printed to a terminal; both nodes hold the new one. Testnet anchoring resumes by itself once its note trees reach the tip (16k blocks behind at 16:41 UTC) | ✅ |
| 144.12 | **A sale that closed with no tickets may be set again** (24 Sep, after the first live testnet rehearsal closed empty: nothing could be drawn and `SetSale` refused a closed sale, so the collection's sale was stuck for good). Once any ticket exists, prices and the sale are fixed as before; `zyn-sale-draw draw` over no tickets now says to `open` again or go to `open-mint` | ✅ |
| 144.13 | **Live rehearsal on testnet 212, 24 Sep** — the whole cycle through the public site and services: the empty sale set again (144.12); 3 tickets bought on cavebro.win/mint through the Nap popup and 1 burned on Solana devnet (posted by `zyn-burn-watch`); an announced close on real testnet heights; the draw over real Zcash block 4388691 and drand round 32492801 (4 tickets, 4 items); all 4 claimed on the page with their art; the open mint posted over 4,437 serials, leaving out the 3 test items that already existed; one minted on the page (Zyn Owl #0457). ZEC.zy spent matches to the zatoshi (3 × 0.01 + 4 × 0.005 + 0.0133); testnet ZynScan agrees. Browser test `apps/nap-connect/tests/sale.e2e.mjs`, phases buy/claim/mint | ✅ |

## §147 — ZYN release: a milestone ladder, and an airdrop to NFT holders at genesis (25 Sep 2026)

The user: airdrop a small portion to Cave and Nap NFT holders; release ZYN at milestones — genesis at 420 ZEC in the pot, later 1,000 ZEC, later more — but do not anchor every rule now, because ZYN's value should not depend on Zcash alone in the future. Snapshot, and the 333 Nap Mole holders.

| # | Decision | |
|---|---|---|
| 147.1 | **§71 stands:** 21,000,000 ZYN, created once, never minted again, never burned; 18,900,000 in the keyless reserve; 2,100,000 opening the ZYN:ZEC.zy market. **Genesis is the first milestone and the only one fixed now:** 420 ZEC.zy in the future-ZYN-POL pot opens the market at 0.0002 | 🔒 |
| 147.2 | **Later milestones are committed by rule, not by number.** Each is published — its trigger and what it releases from the reserve — a fixed notice period before it can fire, and cannot fire otherwise. "1,000 ZEC.zy in the pot" is the expected second; its size and every later step are decided when announced. Nothing is released by time alone (§71.6) | 🔒 rule; ⏳ notice period, milestone 2 |
| 147.3 | **Only genesis is measured in ZEC.** Later triggers may be measured in whatever ZYN's own market can price — liquidity across ZEC.zy, SOL.zy, ETH.zy and later assets, or ZYN pool depth — so ZYN's value is not tied to Zcash alone. This is §71.8's transition, made concrete | 🔒 direction |
| 147.4 | **An airdrop to NFT holders at genesis, by snapshot.** At the genesis epoch the chain itself — not an operator-supplied list — gives every minted item of Cave Collection Z and of the 333 Nap Moles held by an account other than the treasury a ZYN claim owed to that item's holder, one per item. Holders stay private (claim ids derive from the item; the ledger's claim record names no one publicly) and collect it like any other claim. Items the treasury still holds, unminted serials and items redeemed against the floor get nothing. **This amends §71.13** — "nobody receives ZYN" — for this one, bounded allocation | 🔒 |
| 147.5 | **Ten releases of 2,100,000 ZYN each** (25 Sep): genesis is the first, the 18,900,000 reserve is the other nine. **Each release: 10% to the community, 90% to the market.** Genesis's 10% is the NFT-holder airdrop — 210,000 ZYN (confirmed); later releases' 10% is named in their announcements. **Notice period: 30 days** (confirmed). At genesis the 90% (1,890,000) pairs with the pot's 420 ZEC.zy as protocol-owned liquidity — opening near 0.000222 ZEC per ZYN; the airdrop is the only float | 🔒 |
| 147.5a | **Later releases' 90% (proposed): sold in a one-price sale** (the ledger's commit-and-clear launch — one clearing price, no allocation, a floor at the last market price, window and floor named in the 30-day announcement), **and what it raises becomes ZYN pool liquidity**, so each release deepens the market instead of flooding it. Open: whether a fixed treasury share of the proceeds is taken, stated per announcement | ⏳ user |
| 147.6 | **The Nap Mole half needs the 333 on this ledger first:** they retired with the AMM chain, all still held by the treasury (RUNBOOK). They are re-minted on the object ledger and given out through the claim-code gift path before the snapshot; any the treasury still holds at genesis are excluded | ⏳ re-mint + gift path |
| 147.7 | **To build, on the object ledger:** ZYN issuance at genesis into keyless pots (§71.19–21 were built on the AMM chain), the opening pool, the milestone release operation with its notice period, and the snapshot airdrop operation | ⏳ |

## §148 — The Nap extension is a wallet by itself (25 Sep 2026)

The extension was a window onto desktop Nap (`127.0.0.1:8977`), and desktop
Nap has no public download: from a store it would have opened onto "Nap isn't
running". The holder's call: "Extension wallet should be able to work by
itself."

| # | Decision | |
|---|---|---|
| 148.1 | **The extension is Nap in the browser, packaged.** Its pages are apps/nap-web built in extension mode; one wallet codebase serves nap.cavebro.win and the extension. Keys live on the extension's origin, sealed under the holder's password when kept; the phrase is never stored | ✅ |
| 148.2 | **One server.** Every endpoint is on nap.cavebro.win (the only host permission); on the web every endpoint stays a same-origin path. `src/origin.js` enforces both | 🔒 |
| 148.3 | **Sites reach Nap's connect window directly**, over a runtime port from the content script; the browser names the port's origin, so a page cannot speak for another. The background only opens the window; nothing a site waits on lives in the service worker | 🔒 |
| 148.4 | **The provider goes to https pages** (and localhost for development), not every URL | ✅ |
| 148.5 | **Desktop Nap is no longer behind the extension.** Its own UI stays; whether the extension should also reach it (when it runs) is open | ⏳ |
| 148.6 | **Stores:** Chrome Web Store and Firefox Add-ons, from `apps/extension/release/`. Firefox gets the source package (rebuilds byte for byte) and declares `financialAndPaymentInfo` — the signed transactions that leave the device | ✅ |

## §149 — Nap on phones is the web wallet in a native shell (25 Sep 2026)

"Also prepare for the nap mobile app." The desktop project (`apps/nap`) wraps
the Rust wallet core and had a phone plan that stopped at halo2 on Android;
the web wallet already runs Zcash in WebAssembly and is laid out for phones.

| # | Decision | |
|---|---|---|
| 149.1 | **One wallet codebase for web, extension and phones.** `apps/nap-mobile` is a Tauri 2 shell with no commands or plugins around `apps/nap-web` built with `--mode mobile`; keys are made and sealed on the phone | ✅ |
| 149.2 | **Nap's server answers the app's origins only** (`tauri://localhost`, `http(s)://tauri.localhost`) via a Pages middleware; other cross-origin pages still get no readable reply | 🔒 |
| 149.3 | **Identifier `io.infinityteam.napwallet`** on both stores (the one the desktop app already uses); iPhone-only and iOS 15+ for the first version; Android native code aligned for 16 KB pages | ✅ |
| 149.4 | **Publishing needs the holder:** Apple Developer Program as an organization (3.1.5), Google Play account (12 testers × 14 days if personal), the upload key (`scripts/make-upload-key.sh`) | ⏳ |
| 149.5 | **Next, before review:** biometric unlock through Keychain/Keystore (4.2), and a decision on whether iOS ships Nap Market (3.1.5(iii)) | ⏳ |

## §150 — Attendance certificates: tap an event's NFC tag, get a private certificate (25 Sep 2026)

The holder's direction: fun features for the app — NFC at crypto events,
"more like attend certificate", and "prepare the tech part so future
integration will be possible" without yet deciding who issues them.

| # | Decision | |
|---|---|---|
| 150.1 | **An event is a collection; a certificate is one of its items.** The event's manifest (name, date, place, issuer, art hash, cap, mode) is canonicalised and hashed, and the collection address is derived from the hash (`objectvm.attend.event.v1`), so an event cannot be reworded after it is registered | 🔒 |
| 150.2 | **Taps are NTAG 424 DNA SUN reads** (NXP AN12196): encrypted UID and counter plus an 8-byte CMAC, checked in `zynzapd::tap` (RFC 4493 CMAC; NXP's example tap is a test vector). A copied link is worth one tap | 🔒 |
| 150.3 | **Two modes.** `badge`: one certificate per tag (badges, wristbands), the first account to claim a tag keeps it; `door`: one shared tag, one certificate per account, each tap good once. Either way one per account, a claim binds once and a retry reaches the same serial | ✅ |
| 150.4 | **The mint is the operator's** (`zyn_mintItem`), from `zyn-attend`, like the Cave sale's delivery — trusted and said so. Opening this to any organiser is one consensus step: a mint signed by the collection's own issuer. Not built until that is decided | ⏳ |
| 150.5 | **Nap:** a tag opens `nap.cavebro.win/tap?e=…&picc=…&cmac=…`; the wallet keeps the tap through unlock or creation, publishes its receiving key if needed, collects, and names certificates by their event. `/attend[/testnet]` relays to the network's service (503 until configured) | ✅ |
| 150.6 | **Open:** phone-app deep links (need the Apple team id and Play signing key), soulbound certificates, and proving attendance privately (a signed read of holding one item of a collection) | ⏳ |

## §151 — Face ID / fingerprint unlock in the phone app (25 Sep 2026)

Before the first App Store review (guideline 4.2 reads thin web wrappers as
"just a website") and because typing a password on a phone is the slow path.

| # | Decision | |
|---|---|---|
| 151.1 | **What biometrics release is the kept wallet's password**, not a key: the sealed wallet stays exactly as on the web, the password still opens it, the phrase still restores it | 🔒 |
| 151.2 | **The password lives in the platform's secure store**, released only to this device's current biometrics — Keychain `.biometryCurrentSet` (+ an explicit `evaluatePolicy`), Android Keystore key with per-use strong-biometric auth. New faces/fingers invalidate it; it never leaves the device | 🔒 |
| 151.3 | **Offered, not imposed:** once, after the password opens the wallet; "Not now" is final for the session. Forgetting the device removes it. Web and extension builds carry none of this | ✅ |
| 151.4 | **One native plugin, no third-party code:** `apps/nap-mobile/plugins/napvault` (Swift + Kotlin + Rust), exposed only to the app's own pages through a capability | ✅ |

## §152 — The Nap browser in the phone app (25 Sep 2026)

"Why can't iphone app open link. Phantom and jupiter has some browser
features" → "so build it". The audit (same day) found outside links did
nothing on iOS and loaded inside the wallet on Android.

| # | Decision | |
|---|---|---|
| 152.1 | **Sites open in the Nap browser, a separate native web view** with an address bar and a start page of Nap's sites (Cave store, Jade, Collection Z, ZynScan). It has no Tauri IPC and no access to the wallet's origin, storage or Face ID | 🔒 |
| 152.2 | **Every https site in it can reach Nap** through the same provider as the extension (Nap Connect), named by the origin the platform reports for the main frame | 🔒 |
| 152.3 | **The wallet serves those requests with connect-ops' methods**: public at once, the holder's data after connecting, every write approved in a native sheet (Approve armed after 0.8 s). The connect sheet says holdings are visible when asked — the audit's H1 wording fix, here first | ✅ |
| 152.4 | **The wallet's own view never leaves the app's pages**; outside links and new windows open in the Nap browser | 🔒 |
| 152.5 | **One Zcash account per wallet page:** sites use the wallet's own (its address once open; payments wait for its sync), never a second one in the same worker | ✅ |

## §153 — The wallet audit's four highs, fixed (25 Sep 2026)

The 25 Sep audit of every Nap build (keys, web surface, transactions, site
connections, phone app) found no critical issue and four high ones. "do them":

| # | Decision | |
|---|---|---|
| 153.1 | **Amounts that do not fit are refused, never wrapped.** `parseAmount` stops below 2^127; the one 128-bit encoder every intent uses throws outside the signed range. A site could have had a listing shown at 340 quintillion ZEC.zy and signed at 5 wei | 🔒 |
| 153.2 | **The connect screens say what a connected site can read**: account and Zcash address, and — when it asks — balances, items, what is owed, sale tickets and coins made. The privacy policy says the same. (Jade's portfolio uses the coins-made list, so it stays; honesty, not removal) | ✅ |
| 153.3 | **A restored wallet finds its count.** A device that never counted for a wallet walks its nonces in pages of 64 for anything the chain still holds under them (claims, tickets, waiting deposits, coins made), stopping after 256 empty in a row, and keeps the count. Once per device; ~1.6 s on testnet. A run of more than 256 fully collected trades is the one case it cannot see past | ✅ |
| 153.4 | **iOS 16 is the phone app's minimum** (AbortSignal.timeout and wasm-unsafe-eval arrived in Safari 16); the web build targets safari16 / chrome103 / firefox114 so newer syntax fails the build, not the phone. Supersedes §149.3's iOS 15 | ✅ |
| 153.5 | **Open from the audit:** the kept wallet's KDF and password rule (Argon2, 12+ characters, phone backups), one connect window with a rate limit, per-asset approval labels and token ids, exit salts that survive lost storage, /rpc allowlist, app privacy screen and auto-lock, Firefox 127 minimum, broken `href="/"` links in the extension | ⏳ |


## §154 — BTC.zy: Bitcoin signet on the bridge (25 Sep 2026)

"is there a possibility to bridge monero on Zyn" → "or get BTC on the bridge
now, easier" → "build it". Monero waits: no threshold-signing library we would
trust, and view keys that break its privacy model. Bitcoin has both.

| # | Decision | |
|---|---|---|
| 154.1 | **One Taproot key, 2-of-3 FROST** (`frost-secp256k1-tr`, BIP-340/341 compatible), made by `zyn-ceremony bitcoin`; the custodians sign it on their own ops (7 and 8) beside the Solana and Zcash ones. Testnet runs all three shares on one box, as SOL does — stated, not hidden | 🔒 |
| 154.2 | **The deposit code is the address.** Bitcoin wallets cannot attach a memo, so a deposit goes to the vault's internal key tweaked with `taggedHash("Zyn/BtcDeposit", commitment)` standing where a script tree's root would be. Only the key path spends it; no script is ever revealed. Nap derives the address itself (`bitcoin.js`, vectors pinned against `zyn-custody`) and never takes one from a server | 🔒 |
| 154.3 | **The bridge watches only codes it was told about**: Nap calls `zyn_depositWatch {network, commitment}` (public, registered bitcoin networks only, capped at 200,000) when it shows an address. The node already learns the commitment when the deposit is credited, so this tells it nothing new except earlier | ✅ |
| 154.4 | **Credited at 2 confirmations, once per output** (`DepositCommitted` with the outpoint as reference); several sends to one address add up and are claimed together. Change and fee buffer sit at the vault's key-path address (BIP-86 tweak) | ✅ |
| 154.5 | **Exits pay segwit and Taproot addresses only** (the revealed destination is the output script, 22–42 bytes, never OP_RETURN); legacy addresses are refused in Nap before anything is burned. Whole satoshi, at least 0.0001 BTC | 🔒 |
| 154.6 | **The network fee comes out of the payouts**, shared equally across the exits one transaction pays; an exit too small to carry its share waits. What leaves the vault is what Zyn burned. The signed transaction is written to `inflight.json` before it is broadcast, so a crash rebroadcasts rather than pays twice | 🔒 |
| 154.7 | **Signet, not testnet3 or testnet4, and not mainnet.** `bitcoin-signet` is the registered network; `ZYN_BTC_NETWORK=testnet4` is supported but unused. Mainnet needs custodians on separate boxes first | ⏳ |

## §155 — ETH.zy on mainnet: a capped pilot whose vault answers to the team's Safe (26 Sep 2026)

"should we test the sol and eth bridge on zyn mainnet now" → "make it. Eth I
have Safe" → "Safe: 0xEE74…b983, 2-3. Solana can wait, build eth now" → cap
"0.01". This relaxes §146.10 and §69.1 for one asset, on purpose and within a
cap: the ETH boundary opens on mainnet before Phase 2, as a pilot.

| # | Decision | |
|---|---|---|
| 155.1 | **The vault's signer is the team's Safe** (`0xEE741530abB9eaD1C7d6E9C58E5Df0D43F39b983`, Safe v1.4.1, 2 of 3 owners). `ZynVault` `0x235730d7738CC16d3b94d0e3D7097F9ddE22A459` on Ethereum mainnet, deployed block 26062420; asset 1 = native ETH; admin handed to the same Safe, so the deployer keeps no role. No bridge key can move funds | ✅ |
| 155.2 | **`withdrawAsSigner`**: the signer may call the vault directly with the same payouts and nonce a signed `withdraw` would carry — how a Safe executes a payout. One nonce for both paths, so neither can replay the other | ✅ |
| 155.3 | **The bridge only proposes.** In Safe mode `zyn-eth-bridge` holds a proposer key (`0x11C34dD3B2698a02975FDE9241A0E418b68ADC9B`, listed as a proposer on the Safe) and no gas money. Each batch (≤ 8 exits) is simulated from the Safe, hashed by the Safe's own `getTransactionHash`, recorded in `proposal.json`, then posted to the Safe transaction service. Two owners approve and execute in Safe{Wallet}; the bridge settles the exits once that execution is finalized. If the Safe spends the proposal's nonce on anything else, the bridge pays nothing and waits for the operator to delete the record. On chain 1 the bridge refuses to run without a Safe | ✅ |
| 155.4 | **Capped at 0.01 ETH issued** (`ZYN_ETH_CAP_WEI`). A deposit that would pass the cap is not credited; it is appended to `over-cap.jsonl` (index, amount, sender, tx, commitment) for the Safe to refund by hand. Nap checks the room left before a deposit and says so | ✅ |
| 155.5 | **Stated where users decide**: Nap's ETH card and the landing page say pilot, the cap, who holds the Safe, that its owners could move what the vault holds, that payouts wait on them, and that the contract is unaudited | ✅ |
| 155.6 | **Rehearsed before mainnet** on an anvil fork of mainnet against the real Safe (owners impersonated): deposit, over-cap deposit, withdraw, proposal, `approveHash` × 2 + `execTransaction`, settle; and a rejected proposal re-proposed at the next nonce. The first mainnet run found what the mock did not: the Safe service refuses lowercase addresses (400/422); everything now goes out EIP-55 | ✅ |
| 155.7 | **Not yet**: SOL on mainnet (a Squads multisig would play the Safe's part), an audit, a higher cap. Raising the cap is a new decision here, not an env edit | ⏳ |

## §156 — NU7: nothing signs for an upgrade this build does not know (27 Sep 2026)

"Zcash is upgrading NU7 soon, check if we need to change anything" → "make our
infra NU7 compatible". NU7 activates on testnet 6 Oct 2026; the mainnet target
is 5 Nov (go/no-go 20 Oct). It brings a new consensus branch (`0x77190AD9`, ZIP 259),
25-second blocks (ZIP 218), no v4 transactions (ZIP 2003) and the NSM. The
libraries we build on (zcash_protocol 0.10.6 the newest) do not know NU7's
branch or heights yet, so the switch itself waits for them. What can be done
now is below; §156.6 is the switch.

| # | Decision | |
|---|---|---|
| 156.1 | **Fail closed on the branch.** Every Zcash transaction we sign — vault exits (`object_exit`, `settle`), anchors, desktop Nap, browser Nap — is built with `Envelope::next(network, tip, branch)`, where `branch` is what Zebra says the *next* block is under (`getblockchaininfo` → `consensus.nextblock`). If this build would sign for any other branch, it refuses and says so ("That is NU7: this build predates it…"). A transaction signed under the wrong branch never lands, so a stale build would otherwise leave exits in flight until they expire and ask the custodians again and again for nothing. The browser and desktop wallets get the branch through `zyn-lightd`'s INFO, which now passes on Zebra's answer instead of computing its own | ✅ |
| 156.2 | **Signed for the next block, not the tip.** The envelope used the tip's branch; at an upgrade's first block that was off by one. Now height = tip + 1 | ✅ |
| 156.3 | **Waits keep their length in time.** Expiry is 40 blocks before NU7 and 120 after it (both about 50 minutes, ZIP 218). The wallets' scan depth of 3 blocks becomes 9 when the next block is NU7 (`blocks_per_old_block`). The object nodes' and replica's `ZYN_CONFIRMATIONS` go ×3 at the switch (6→18, 10→30) — an env change made with the switch's deploy, erring towards waiting longer | ✅ / at the switch |
| 156.4 | **Custodians need no NU7 rebuild.** They sign a bare sighash with re-randomized FROST, which NU7 does not change; the coordinator builds the transaction. The Hetzner, Air and Windows shares stay as they are | ✅ |
| 156.5 | **Zebra to 6.4.2 now** (`ops/zebra-upgrade.sh`, both hosts): the security fix for malformed v6 transactions. It moves Zebra's own end of support to about 2 Nov 2026, so the NU7 Zebra release must be in before then on mainnet, and before 6 Oct on testnet | the user runs it |
| 156.6 | **The switch**, when librustzcash and Zebra ship NU7: bump the crates (workspace and `zec-core`), check `BranchId::Nu7` is `0x77190AD9` and `Envelope::next` accepts it, upgrade Zebra, redeploy `zyn-object-node` and `zyn-lightd` on both hosts with confirmations ×3, rebuild the wasm and deploy Nap web, resubmit the extension and phone apps early (reviews take days). RUNBOOK "NU7" | ⏳ |
| 156.8 | **The branch id is ZIP 259's `0x77190AD9`** (found 28 Sep while reading Zakura). ZIP 254 (`0x77190AD8`) was withdrawn; ZIP 259 (draft, 15 Sep 2026) replaces it and sets the heights on 5 Oct (testnet) and 20 Oct (mainnet). The check in 156.1 was never affected — it compares the libraries' branch with Zebra's — but the constant turns on the 120-block expiry, the ×3 wallet depth and the "That is NU7" message after the switch. Corrected in source and in Nap web's wasm; the hosts pick it up with the switch's redeploy | ✅ |
| 156.7 | **Not decided here**: Collection Z's sale terms count blocks (claim window 16,128, notice 1,152). After NU7 they would last a third as long. Keeping their length in time means republishing the terms ×3 before the sale opens — the user's call | open |

## §157 — Zyn Lend: private lending on the object ledger (30 Sep 2026)

"what if a lending protocol" → "the pairs will be ZEC-ETH-SOL, protocol venue
on Zyn, collateral one borrow another, lend all 3" → "make new repo … it's ok to
be open source, as long as it enforce running on Zyn" → "build it". For the
Zecathon (Private Markets, due 28 Oct) and Colosseum (~15 Oct).

| # | Decision | |
|---|---|---|
| 157.1 | **The rules are open source, the venue is Zyn.** `zyn-io/zyn-lend` (MIT/Apache) holds the lending crate and the app; `objectvm/src/lend.rs` holds the state and joins it to notes, claims and the seal. What ties it to Zyn is not a licence but what it stands on: private notes, the batch, the bridged assets, the pools the price check uses | ✅ |
| 157.2 | **Three pools, six directions.** ZEC.zy, ETH.zy, SOL.zy; one collateral and one borrowed asset per position. Risk: ZEC 55/60/8, ETH 75/80/5, SOL 65/70/7 (max LTV / liquidated at / penalty, %). Rate: two-slope utilization curve, 0% → 4%/yr at 80% used → +75%/yr; 10% of interest to the ZYN pot | ✅ |
| 157.3 | **Liquidation is a Dutch auction in batches (§12f), not a sale into a pool.** A lot opens at 102% of the posted rate and falls a point per epoch to 85%; every bid in an epoch pays one price, pro rata when oversubscribed; a lot sells only what covers debt and penalty and returns the rest; bidders never see whose position it was | ✅ |
| 157.4 | **The operator posts prices and the time** (`LendPrices`, Coinbase + CoinGecko median, 2% spread, every 30 s). Interest accrues to that timestamp — the ledger has no clock. New borrowing in a pair pauses when Zyn's pool disagrees with the posted prices by more than 5%. A stated trust assumption | ✅ |
| 157.5 | **Everything lending pays is a claim**, found by ids only the holder derives (`lend_source`, `lot_source`) and collected by Nap. Positions are private from the public, visible to the sequencer (tier 2) | ✅ |
| 157.6 | **Live on testnet 212 (30 Sep):** pools listed at node start (`ZYN_LEND_ASSETS`), prices posting, anchored and replica-verified; first real loans (SOL.zy against ETH.zy) opened through Nap from zyn-lend.pages.dev (`tests/lend.e2e.mjs`, 9/9) | ✅ |
| 157.7 | **Not yet:** a ZEC.zy pool with depth (needs TAZ), ZEC pairs' Zyn pools, mainnet | ⏳ |

"The real thing is the engine to make sure 1. economy lowest risk possible
2. privacy 3. we have revenue" → "protocol owned is ok for now. close economic
risk and privacy gap".

| # | Decision | |
|---|---|---|
| 157.8 | **Guarded mode, switched on by the first `LendLimits` (op 48, operator).** Journals from before it replay to the same roots; the state root's lend section grows (`objectvm.lend.v2`) only once it is on. Saved state is MARKET09 | ✅ |
| 157.9 | **Caps and a floor.** Per asset: a collateral cap and a borrow cap (whole tokens) and a minimum loan in USD, so no pool takes more than its tail can sell and no loan is too small to be worth liquidating. Testnet: SOL 100/50/$1, ETH 2/1/$1, ZEC 5/2/$1 (`ZYN_LEND_LIMITS`) | ✅ |
| 157.10 | **Prices must be fresh and confirmed.** Borrowing and collateral removal stop when the last posted price is over 12 epochs old; a move over 15% is held, not used, until a second post within 2% confirms it. Liquidation waits on a held price too, so one bad feed tick neither opens nor closes positions | ✅ |
| 157.11 | **Losses are booked when they become likely, not when they land.** At lot open the shortfall the floor price implies is taken from insurance first, then written off the pool; whatever the lot recovers goes back to lenders, then insurance, then the owner | ✅ |
| 157.12 | **Nothing sits unsold.** A lot still open after 37 epochs goes into its pair's Zyn pool batch as a protocol order, minimum 70% of the posted rate; what it fetches settles the lot like any bid | ✅ |
| 157.13 | **Revenue is protocol-owned for now.** The 10% reserve fills insurance up to 5% of each pool's debt; only above that does it go to the ZYN pot | ✅ |
| 157.14 | **What the public sees is a snapshot, rounded.** `zyn_lendMarkets` answers from the state at the last seal (the node seals on its clock while lending is listed), with pool sizes to 3 significant figures, utilization to 0.1% and rates computed from that, a share price instead of a share count, and no count of positions. ZynScan names every holder's lending intent just "Lending". One supply or loan can no longer be read off the next answer | ✅ |
| 157.15 | **The object node keeps anchoring across a restart.** It never saved its pending epochs, proposal or lineage; with lending listed it seals on its clock, so anchors run back to back and a restart (the guarded deploy, 30 Sep) landed with one in flight: the node rebuilt a proposal over every pending epoch, which is not the anchor Zcash confirmed, and stopped anchoring. Now the node saves all four (`chain-N.anchors`, `pending-N-*.sealed`, the proposal, certificates) and restores them; a lost proposal is found by id among the oldest pending epochs; and an anchor sent *before* a restart whose epochs are gone is settled as a transaction, not accepted — it stays on Zcash for anyone to check, and the next anchor finalises past it. One sent by the running process still fails closed | ✅ |

"the main thing is the privacy on both lending and borrowing side. make sure of
it before making liquidation public" — every public surface audited against the
code, 30 Sep.

| # | Decision | |
|---|---|---|
| 157.16 | **Held already:** intents are never public (the journal reaches only the operator, the replica and the explorer's replay; DA bundles are sealed); ZynScan shows a holder's lending intent as "Lending" with no account, amount or position; position and claim ids derive from nonces keyed by the wallet's secret, so nobody else can look them up; bidders name a lot by `lot_source`, never the position; spent inputs publish nullifiers only; the RPC goes straight to our host, no proxy in between | ✅ |
| 157.17 | **Fixed:** (1) the leftovers of a liquidation were claimable under the lot's *public* id, so anyone could watch them collected and tie together the lots one owner collects at once — now under `leftover_source(position)`, which only the owner derives (no lot has ever existed, so every journal replays unchanged); (2) the published share price and borrow index were exact, which gives exact utilization back — now six decimals; (3) pool figures to two significant figures with borrowed and cash on the same step, and utilization to 1%, so a loan small beside its pool leaves no trace (was three and 0.1%, which showed a 0.02 SOL loan in a 2.6 SOL pool); (4) the ZYN pot, which lending's reserve feeds, to three significant figures (it was exact, for fee checks; the fee can still be checked to that precision); (5) ZynScan drops the refusal reason and the note counts for holder lending, which said what was attempted | ✅ |
| 157.18 | **Still true, and said:** the operator sees every position (tier 2); a lot in auction shows its collateral, debt and opening epoch, because bidders must — never whose it was; a loan that is a large share of a thin pool still moves the rounded figures. Privacy of amounts grows with the pools | ✅ |
