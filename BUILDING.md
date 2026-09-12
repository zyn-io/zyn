# Building on Zyn

An application on Zyn is a Rust type that implements `MicrochainVm`. There is
no new language, no bytecode to target, and nothing to deploy — a VM is a crate
you compile, and the infrastructure around it (sequencing, epochs, anchoring,
data availability, exits, wallet signing) is the same for every application and
knows nothing about any of them.

The shortest complete example is **`zyn/tests/tally/mod.rs`** — about 250 lines
that count things. No swap, no bridge, no assets. Everything in it is there
because the specification requires it of *everything*, which makes it the
honest answer to "how much do I have to write".

## What you implement

Thirty methods, twenty-four of which have defaults. The six that do not are the
ones only you can answer: what an intent is, what applying one does, what your
state commits to, and what it conserves.

The mechanical ones are where mistakes happen, and they are all the same
mistake — **a byte string that is not a pure function of state**. An unordered
map in `encode`, a `HashMap` iterated in `sections`, a length written after the
thing it measures. Each produces a chain two nodes disagree about, and none
produces an error.

## Start by making it fail

Before writing an intent handler, wire up the conformance suite:

```rust
#[test]
fn it_conforms() {
    zyn_vm::conformance::assert_conforms(Fixture {
        state: seeded(),                    // a chain with history
        accepted: /* an intent that works */,
        rejected: /* one that must not */,
        sequence: vec![/* a run with variety */],
    });
}
```

It checks the numbered rules in `SPEC.md`:

| | |
|---|---|
| **S1** | the same intents twice produce the same root *and the same bytes* |
| **S2** | only `seq + 1` applies; everything else is inert |
| **S3** | a rejection advances history without moving application state |
| **S5** | the root moves when committed state moves |
| **S6** | the section layout, and that an exit proof actually verifies |
| **S7** | sealing, and that the sealed view is recoverable |
| **S8** | encode/decode round-trips to the same root and resumes |
| **S10** | no decoder panics — on random bytes, every truncation, and bit flips |
| **S11** | the books balance after *every* intent, not just at the end |

`sequence` is the field worth filling in. S1 and S11 are only as convincing as
the variety of state changes they see, and you are the only one who knows what
your machine's interesting runs are.

The suite is checked against itself: `conformance::catches_violations` contains
a VM that breaks determinism and one that mints money, and asserts the suite
names the right rule. A suite nobody has watched fail is decoration.

## What it cannot check

S4 (outcomes emitted, never accepted), S9 (checked arithmetic), S12–S14
(signing, presentation, entitlement) are properties of how you write the code.
The suite can only fail to disprove them.

Two are worth stating as habits rather than rules:

- **Never trust an input you did not produce.** Decoders return `Option`, not
  panics. The bytes come off a socket and an attacker picks all of them.
- **`intent_authorities` is not optional.** It declares which accounts must
  have signed. Its default is *empty*, meaning no user signature can authorise
  the intent at all — so forgetting it makes your intent unusable, never
  unguarded. That default is deliberate (**S14**).

## Running alongside others

`zyn::host::Registry` holds applications whose `Intent` types have nothing in
common, addressed by chain id. The boundary is bytes, because
`MicrochainVm` cannot be a trait object — associated types are how the compiler
stops one application's intents reaching another's state, and that is worth
keeping.

```rust
let mut apps = Registry::new();
apps.register(Box::new(Node::new(21, /* your params */, policy, economics)))?;
apps.register(Box::new(Node::new(22, /* another app */, policy, economics)))?;
apps.submit_signed(21, &frame, now)?;   // routed, verified, applied
```

A signature is bound to a chain id (**S12**), so a frame aimed at the wrong
application cannot verify even when both run in one process. Routing is a
convenience; the binding is the security.

## What users sign

You get wallet support without building a wallet. `typed_intent` renders your
intent as EIP-712 typed data, which MetaMask and every EVM wallet already sign,
and Solana wallets sign the same intents through `signMessage`. The default
renders an opaque hash — honest, and worse than what you can do in ten minutes
by naming your fields.

Every field that changes the outcome must appear. The one that catches people
is routing: a trade through a different pool at identical amounts is a
different trade, and if the route is not in the signature a compromised front
end can change it.

## What is not here yet

- **Loading a program the node has never seen.** The set of applications is
  fixed when the binary is built. Dynamic loading needs an rv32im interpreter
  (`ZVM.md`, Stage 1) and metering — and nothing permissionless ships without
  metering (`DECISIONS` §8.1).
- **Composing two applications atomically.** `DECISIONS` §8.5. The shape is
  decided (bundles over scratch state, Sui's Programmable Transaction Blocks as
  the model — deliberately *not* synchronous calls), and it is not built.
