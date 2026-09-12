# ZVM — how a Zyn program runs

**Recommendation: `rv32im`. Rust is the first-class SDK. No new language, now or later.**

This document answers one question: when Zyn opens to applications other than
ZynZap, what does an application *ship*? It assumes `SPEC.md`, which defines
what an application must *be*.

---

## 1. The short version

A Zyn program is a `riscv32im` ELF binary with one entry point:

```text
   bytes in  ──>  run  ──>  bytes out
```

Two operations — read the input tape, write the output. No storage calls, no
network, no clock, no randomness. That is the entire host interface, and it is
already implemented: `zyn_vm::zvm::run`.

You write it in Rust today. TinyGo, Zig and C are possible later without the
platform changing, because the target is an instruction set rather than a
language runtime.

---

## 2. Why rv32im and not the alternatives

The deciding argument is not performance. It is that **there must be exactly one
artefact.**

This codebase has repeatedly refused designs where two implementations are
"supposed to match" — the same reasoning that put the commitment encoding in the
spec rather than in each application, and that keeps `guest.rs` a wrapper rather
than a second copy of the transition. The execution/proving boundary is the
largest instance of that trap available, and rv32im is the only option that
avoids it: the binary the sequencer executes is the binary the prover proves.

| | rv32im | WASM | Move | EVM |
|---|---|---|---|---|
| New language? | no (Rust, Go, C) | no (Rust, Go, C) | **yes** | **yes (Solidity)** |
| Same artefact executes and proves | **yes** | no — needs a WASM zkVM, or two paths | no | no |
| Floats | **absent from the ISA** | present; must be disabled by policy | n/a | n/a |
| Metering | cycles — the same number a proof costs | fuel — a second budget to reconcile | gas | gas |
| Native speed | slower (interpreter/JIT) | **fastest** | fast | slow |
| Non-Rust tooling today | thin | **excellent** | n/a | n/a |
| Already targeted here | **yes** | no | no | no |

**Against WASM.** It is the better *execution* choice and the worse *system*
choice. Proving WASM means either a WASM zkVM — a much thinner ecosystem than
the RISC-V provers — or running WASM for speed and RISC-V for proofs, which is
two implementations that must agree about every edge of every application. That
is the failure mode, not a trade-off around it. WASM's determinism story is also
policy rather than physics: floats exist and must be canonicalised or forbidden.
On rv32im the `F` and `D` extensions are simply absent, so **S1's no-float rule
is enforced by the absence of the instructions**.

**Against Move.** The resource model is genuinely excellent for assets — linear
types make double-spend a type error, which is a real advantage over anything on
this list. But it is a new language, which the brief rules out; there is no
mature proving story; and its global-storage object model does not map cleanly
onto the section-based state root in `SPEC.md` §S6. Worth revisiting if Zyn ever
wants formal verification more than it wants proofs.

**Against EVM.** Explicitly out of MVP scope in the project plan (§24), and it
would import Solidity as the new language the brief rules out.

**The honest cost of rv32im:** native execution is meaningfully slower than
WASM, and the non-Rust toolchains are thinner today than WASM's. Section 5 says
what to do if that cost ever bites.

---

## 3. The interface

Already live in `zyn_vm::zvm`:

```rust
pub fn run<V: MicrochainVm>(tape: &[u8]) -> Result<Vec<u8>, ZvmError>;
```

The statement it establishes:

> Starting from a state whose commitment is `base_root`, applying `batch` in
> order yields a state whose commitment is `final_root`.

`base_root` is an input to the statement, not something the caller is trusted to
have checked — otherwise a proof would say only that *some* state transitioned
correctly, leaving a sequencer free to prove from a state that was never
anchored.

The public output is six fields:

```text
vm_id | chain_id | base_root | final_root | epoch | seq
```

`vm_id` is `H(name, version)`. It is what stops a transition proved for one
application being presented as another's — the failure that matters most the
moment Zyn hosts more than one. `zyn/tests/second_app.rs` asserts two
applications cannot share one.

Because the ABI belongs to the spec rather than to ZynZap, **one settlement
verifier serves every application Zyn will ever host**, instead of a contract
redeployed per app. `swapvm::guest` is now a thin wrapper over it, and the
counting VM in `zyn/tests/second_app.rs` gets the same entry point without
writing a line of it.

---

## 4. The staged path

The point of the layering is that ZynZap does not move as Zyn opens up.

| Stage | How an application ships | Who may deploy | ZynZap changes |
|---|---|---|---|
| **0 — today** | a Rust crate implementing `MicrochainVm`, compiled into the node | operator | — |
| **1** | an `rv32im` ELF, run in an interpreter behind the same ABI | operator admits | **none** — it already builds for the target |
| **2** | same, plus registration, cycle metering, isolation | permissionless | **none** |

Stage 0 → 1 is a change to how ZynZap is *compiled*, not to what it *is*. The
crate already builds for `riscv32im-unknown-none-elf` and is verified to do so
on every run of the test suite.

Stage 1 is the real engineering: an rv32im interpreter (or a wrapper around an
existing one), an ELF loader, and the two-syscall host shim. It is bounded work
against a fixed instruction set, which is why the target being an ISA rather
than a language runtime matters.

Stage 2 is mostly policy, and its hard problems are the ones already logged as
open questions in `SPEC.md` §6: shared versus isolated threshold custody, and
what settlement can verify about an application whose records it cannot
interpret.

---

## 5. What is deliberately unresolved

**State passing.** Today the whole state crosses the boundary on every batch:
O(state) per call. Fine while a chain is small, wrong at scale. The fix is a
merkleised store where a program requests only the sections it touches and the
host supplies them with proofs — the same journal design a zkVM needs anyway. It
is real work and it changes the ABI, so it should be done once, deliberately,
before Stage 2 rather than during it.

**Metering.** `MAX_BATCH` is a decode-time sanity limit, not metering. Real
metering is cycle counting inside the executor, and it does not exist yet.
Nothing permissionless can ship without it.

**If native speed becomes the binding constraint.** Do not add WASM as a second
execution path — that reintroduces the exact problem rv32im was chosen to avoid.
Compile the interpreter's hot path, or JIT rv32im. Same artefact, faster host.

**Cross-application composition.** Out of scope, per `SPEC.md` §5. Two
microchains cannot call each other, and atomicity stops at one VM's transition.
That is why ZynZap holds all its pools in one execution environment rather than
one per pair.
