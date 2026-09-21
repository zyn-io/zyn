# EVM, SVM, Sui Move — and what Zyn should take from each

Three production VMs, three different bets about **how much the machine knows
about what it is running**. That single axis explains most of their strengths
and nearly all of their bug classes.

```text
   knows nothing                                          knows about assets
   ├──────────────────┬────────────────────────┬────────────────────────────┤
  EVM                SVM                    Sui Move
  (all safety in     (knows isolation and    (knows assets: linear types,
   the application)   data flow, not value)   conservation by construction)
```

Zyn sits off this axis on purpose: it is not a general contract platform, and
the project plan is explicit that the narrower the VM, the easier it is to
build, audit, prove and reason about balances (§7). What follows is what to
take, what to refuse, and one thing we were missing.

---

## 1. EVM

**Model.** 256-bit stack machine, single-threaded, global mutable state in a
Merkle-Patricia Trie. Any contract may call any contract synchronously and
re-enter.

**Pros**
- **Composability is the real one.** Arbitrary atomic synchronous calls between
  contracts is why aggregators, flash loans and the whole DeFi lego stack exist.
  Nothing else on this list matches it.
- Ten years of adversarial pressure. The bug classes are *known* bug classes.
- The deepest tooling and auditor pool in the industry.

**Cons**
- **Reentrancy is the default.** The most expensive bug class in crypto history
  is a direct consequence of synchronous callbacks into mutable state.
- The VM has no idea what a balance is, so it cannot enforce conservation. Every
  safety property is application-level and audit-verified.
- 256-bit words on a machine that mostly does 64-bit arithmetic: expensive to
  interpret, far more expensive to prove.
- Sequential execution; no parallelism.
- MPT is a poor structure to prove — hashing overhead on every state access.
- Per-opcode gas has needed repricing hard forks more than once.

**Take:** composability within one execution environment. **Refuse:** the
callback surface, the word size, and the trie.

---

## 2. SVM (Solana)

**Model.** Stateless eBPF programs; state lives in accounts passed explicitly
into each transaction. **Transactions declare their read/write set upfront**,
which is what lets non-overlapping ones run in parallel.

**Pros**
- **Declared access sets are a security property before they are a performance
  one.** You can see statically what a transaction can touch. No surprise state
  access, no ambient authority.
- Parallelism falls out of that declaration rather than out of speculation.
- Stateless programs plus explicit accounts make data flow legible.
- eBPF is a sane register machine — cheap, and much closer to provable than the
  EVM.

**Cons**
- **Account validation is the entire attack surface and it is manual.** The
  canonical Solana bug is a missing owner / signer / PDA-derivation check.
  Anchor exists because raw SVM needs enormous boilerplate to be safe.
- **Hot-account contention defeats the parallelism exactly where we would want
  it.** Every transaction touching one popular AMM pool serialises on that
  account. For a swap venue, the contended object *is* the pool.
- Declaring the access set up front is awkward for anything dynamic — including
  routing, where the path is the thing being decided.
- CPI depth is capped, which bounds composability.

**Take:** explicit, statically visible data flow. **Refuse:** manual account
validation, and parallelism we would not benefit from.

---

## 3. Sui Move

**Model.** Object-centric state with typed owners. Assets are **linear
resources**: a `Coin<T>` cannot be copied or silently dropped, enforced by the
bytecode verifier at publish time.

**Pros**
- **The strongest security property of any production VM.** Double-spend and
  asset conservation are *type errors*, not audit findings. You cannot write the
  "forgot to decrement the balance" bug.
- The verifier rejects whole bug classes before deployment, not after.
- Owned-object fast path skips consensus entirely — genuinely low latency.
- Move Prover is formal verification that people actually use.

**Cons**
- **Move is a new language**, which is ruled out here.
- The fast path does not apply to us: any AMM pool is a *shared* object, so it
  re-enters consensus and the headline latency win evaporates for a swap.
- Smaller ecosystem and auditor pool.
- No proving story.
- The object model does not map cleanly onto a section-based state root.

**Take:** conservation enforced by the machine. **Refuse:** the language, and an
object model we would have to bend our commitment around.

---

## 4. What Zyn should be

The framing that matters: **Zyn is not competing on generality.** It is a
narrow, application-specific VM behind a fixed spec, so it can afford
guarantees a general platform cannot.

### 4.1 No callback surface at all — *keep*

Zyn has no cross-contract calls. An intent enters, a transition runs, receipts
come out. There is no reentrancy because there is nothing to re-enter. This is
not a limitation to apologise for; it removes the most expensive bug class in
the industry by construction, and it is the direct benefit of a narrow VM.

### 4.2 Composability, scoped correctly — *keep, and name the gap*

EVM's composability is real and we do give something up. What we keep is
composability *within* one VM: ZynZap routes atomically across every pool it
holds, in one intent, priced in one transition (`SPEC.md` §5, plan §3). That
covers the case a swap venue actually needs.

What we lack is *cross-application* atomicity. The eventual answer should not be
synchronous callbacks — that is how the reentrancy surface gets rebuilt. It
should be an **atomic bundle across VMs**: apply to each VM's scratch state,
commit only if all succeed, discard otherwise. `zvm::apply_batch` is already
exactly this shape for one VM. It buys atomic multi-app transactions and
deliberately does not buy flash loans.

### 4.3 Explicit data flow, without declared access sets — *keep the property, skip the mechanism*

An intent already names the accounts and pools it touches (`accounts_named` in
`vm.rs`), so data flow is statically legible — SVM's real benefit. We skip the
declaration because we would not collect the reward: the contended object in an
AMM is the pool, so Sealevel-style parallelism serialises on the hot pool
anyway. Revisit only if Zyn hosts many low-contention applications.

### 4.4 Conservation enforced by the machine — **this was the gap, now closed**

Move's guarantee is that the machine knows what an asset is. Zyn cannot have
linear types without a new language, but the guarantee can move from the type
system into the **specification**:

> **S11 — Conservation.** Every VM declares what it conserves. `transition`
> checks it after every batch and discards the batch if it fails.

Before this, ZynZap's `check_invariants` — supply against units held, ZEC.zy
against confirmed ZEC backing, pool reserves against LP shares — was exhaustive
and **called only from tests**. Nothing stopped a committed root from asserting
that value had been created.

It is now enforced by the infrastructure, for every VM, in the same code path a
proof covers. `zyn/tests/second_app.rs` demonstrates the difference: a VM that
credits a holder without crediting its total accepts the intent happily — as an
EVM contract with a missing decrement would — and then cannot commit a root or
produce a proof.

Coarser than Move: per batch rather than per expression. Unlike an audit, it is
not optional.

**The cost is real.** A full conservation scan per batch is O(state), paid again
inside a guest. The replacement is incremental conservation — track the deltas a
transition makes and check those — which is a meaningful redesign and is logged
in `ZVM.md` rather than hand-waved.

### 4.5 Metering as one number — *better than all three*

EVM prices per opcode and has needed repricing forks. SVM has compute units.
Move has gas. All three maintain an execution cost model *separate from* their
proving cost.

On rv32im, the execution budget and the proving cost are **the same number**:
cycles. One resource limit, not two that must be kept in agreement. (Not built
yet — see `ZVM.md` §5.)

### 4.6 A commitment built to be proved — *better than EVM*

EVM's MPT hashes on every state access. Zyn commits four ordered sections over
`BTreeMap`s in a binary Merkle tree, so a holder's exit proof opens one account
without touching the pools tree at all (`SPEC.md` §S6). That is why one
settlement verifier can serve every application Zyn hosts.

---

## 5. Summary

| | EVM | SVM | Sui Move | Zyn |
|---|---|---|---|---|
| New language | Solidity | no | **yes** | **no** |
| Reentrancy surface | **yes** | limited (CPI) | limited | **none** |
| Conservation enforced by the machine | no | no | **yes (types)** | **yes (S11, per batch)** |
| Composability | **best** | good | good | within one VM; cross-VM bundles planned |
| Parallelism | none | **best** | good | none — contention is the pool |
| Provability | poor | fair | poor | **designed for it** |
| Metering vs proving cost | two models | two models | two models | **one number (cycles)** |
| Ecosystem | **best** | good | fair | none |

The honest reading: Zyn gives up generality, composability across applications,
and an ecosystem. In exchange it removes reentrancy entirely, enforces
conservation the way Move does without Move's language, and is the only one of
the four whose state commitment and cost model were designed to be proved.

That trade is only worth taking because ZynZap is a narrow application. It would
be the wrong trade for a general contract platform, and the plan is right to
postpone becoming one.
