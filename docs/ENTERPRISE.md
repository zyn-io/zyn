# Zyn as an enterprise-friendly layer

Status: **direction, not a claim.** Nothing here may be said to an institution
until the row that carries it is marked built. The whole value of this position
is that it can survive diligence; asserting it early is the one way to lose it.

## The sentence

**Confidential settlement with provable reserves and an exit no counterparty
can block.**

An institution cannot use a transparent chain: positions, counterparties and
flows are trade secrets, and frequently protected by law rather than
preference. It cannot use an opaque one either, because auditors, regulators
and its own risk desk need proof. Almost nothing is allowed to be both. Zyn is
shaped to be — Zcash's shielded pool underneath, an execution layer above it,
and disclosure as an explicit act rather than a default.

This is not a pivot. It is the same system with the operational bar raised and
three primitives added. Consumer surfaces (Cave, Nap Wallet) stay: they are the
proof the thing runs, and the work below makes them more trustworthy too.

## What already argues for it

These exist, are tested, and are rare. They are the reason the position is
available at all.

| | What it is | Why an institution cares |
|---|---|---|
| `zynzapd/src/backing.rs` | A signer refuses to endorse a credit its **own** Zcash node cannot see | Proof of reserves by construction, not an attestation letter. The failure mode it prevents — a fabricated credit that replays identically on every signer — is the one an auditor would ask about |
| `zynzapd/src/exitproof.rs` | A holder's record, leaf index and path to the anchored root. Kilobytes, refreshed each anchor | Exit "with the sequencer gone and every mirror dark". *Nobody can trap my assets* outranks throughput in every institutional risk review |
| Vault FVK held by verifiers | Spend authority and visibility already separated | The seed of selective disclosure: a party can be given sight without power |
| Forced inclusion (`ZYF` memos, §33) | An intent that reaches Zcash must be sequenced | Censorship by the operator is bounded, and the bound is checkable |
| Anchors every ~3 min to Zcash | Settlement finality inherited from a public chain | A finality story that does not rest on the operator's word |

## What is missing

Ordered by what a first diligence call actually kills a deal over.

### 1. Operations — the real gap, and not a cryptographic one

On 15–16 Sep: a signer was dead for 36 hours and nothing reported it; mainnet
custody ran at bare 2-of-3 because a mini-PC entered Modern Standby; a vault
share sat on a laptop that sleeps; a fourth machine holding a role could not be
reached from the host that builds the binaries (§89). Any competent reviewer
finds all four in week one.

Institutions buy operations, not cryptography. So:

- **Per-signer liveness** — reporting *who* endorsed, not only that the
  certificate cleared. A set of three with one silently gone is a set of two
  with no margin (§89.2). **Built 16 Sep**: `Node::signer_liveness`, one alert
  component per signer, plus a TCP probe per custodian.
- **Shares off personal machines.** Named, independent operators on
  infrastructure that does not sleep. Three machines in one house is not three
  operators, and diligence will say so.
- **An incident log.** Published, with causes. Institutions do not expect zero
  incidents; they expect them to be found, explained and closed. `DECISIONS`
  already is this — it needs an outward-facing summary.
- **Documented finality tiers**: accepted → sealed → anchored → Zcash-confirmed
  at N. Each with what it guarantees and against whom.

### 2. Selective disclosure as a product

The single most differentiating thing available, and weeks rather than months.

Per-account viewing keys, **scoped and time-boxed**: this account, this period,
to this auditor, with no spend authority and no ability to widen the window.
The primitive exists (the vault's FVK is already handed to verifiers); the
product does not. What is needed:

- derivation of a disclosure key per account, independent of the spend key
- a scope: account set, epoch range, and which fields
- an artefact an auditor can verify offline against an anchored root — so the
  disclosure proves itself rather than requiring trust in whoever exported it
- revocation that is honest about being expiry, not an instant switch (§ the
  same trade recorded for reveals)

### 3. Issuer-controlled assets

Regulated instruments need a named issuer, transfer restrictions, and — the
uncomfortable one — freeze or seizure under court order. The VM has none of it:
no allowlist, no freeze, no issuer registry.

**The resolution is per-asset, never protocol-level.** `SupplyPolicy`'s
`authority()` (§74 work, 16 Sep) is already the right shape: the issuer holds
that power over *their own* asset and the protocol holds none. A chain with a
freeze key is not credibly neutral; an asset whose issuer has one is a
regulated security behaving correctly. Keep that line exactly where it is — it
is the difference between serving institutions and becoming one.

### 4. Permissioning at the gateway, never in consensus

KYC on who may deposit, and an allowlist of issuers, enforced at the edge. The
chain stays neutral and sees none of it. Put identity into consensus and the
only property worth buying is gone.

### 5. The legal wrapper

Not code, and not optional: an entity, a jurisdiction, terms for the vault
operators, and a stated position on what the operator can and cannot do. The
§84 frame — "progressively minimising trust", with today's trust stated
plainly — is the right voice for it.

## The tension worth naming early

Everything above is a request to hold two lines at once: enough control for a
regulated issuer, none of it for the protocol. Every institutional chain that
failed did so by resolving that tension in the protocol's favour, one
reasonable exception at a time. The per-asset boundary is the whole design, and
it is worth refusing revenue to keep.

## Sequencing

1. Close the operational gaps (A82 done, A83/A84 outstanding, shares off
   laptops).
2. Ship selective disclosure — it improves the consumer product too.
3. Then talk to one institution and let their diligence write the rest of this
   file.

Nothing here justifies leading with an institutional pitch now. It justifies
building so that the pitch is available later, and earned when it is made.
