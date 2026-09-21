# What Zyn may be said to be

One page, because every overclaim so far came from translating an accurate
sentence into a friendlier one and gaining a guarantee on the way (§103.8).
This exists so the friendly version is a lookup, not a fresh translation.

**Rule: if a claim is not on this page, it has not been checked. Check it
against the code, then add it here.**

## The canonical description

> **Zyn is an application execution layer built for Zcash. It runs deterministic
> application state machines, batches their activity, and anchors state
> commitments to Zcash.**

Compact, for a headline:

> **ZEC has private money. Zyn gives it applications.**
> Cave is the first consumer app powered by Zyn.

The hierarchy, which keeps getting flattened:

> **Cave** is the first consumer application powered by Zyn.
> **ZynZap/SwapVM** is Zyn's first execution environment (`SPEC.md`: "application #1").

## The trust model, stated whenever the pitch is

> Today Zyn is operator-sequenced. Bridged ZEC is backed by ZEC held in a 2-of-3
> threshold vault operated by the Zyn team. Zcash records deposits, withdrawals
> and anchor commitments, but does not validate Zyn state transitions.

## Say / do not say

| Say | Not | Why |
|---|---|---|
| an application execution layer **built for** Zcash | the execution layer **of** Zcash | "of" implies protocol recognition and inherited security; Zcash does not know Zyn exists |
| Zcash-anchored; anchor-transaction finality comes from Zcash | settled on Zcash; inherits Zcash security/finality | Zcash finalises the anchor tx and its 32-byte commitment, not the state behind it |
| users can publish signed intents through Zcash, making censorship **detectable** | you can always get a transaction in; censorship-resistant execution | a `Censored` record evidences a refusal; it does not compel execution |
| users hold a **verifiable account-state proof** against an anchored root | trustless exit; you can always leave; nobody can trap your assets | nothing on Zcash consumes an exit proof; `confirm_withdrawal` needs a settlement signer |
| **withdrawals depend on the Zyn-operated threshold custodians** | trustless bridge | say it plainly; it is the single most important disclosure |
| account reads require the holder's signature | private by default; private to your key; known only to your wallet | the operator and authorised verifiers see the journal |
| operator-sequenced; progressively minimising trust | decentralised; L2; rollup | one operator, and no fraud or validity proofs |
| a **programmable architecture** for ZEC applications | anyone can deploy; permissionless applications; Zcash smart contracts | `host.rs`: applications compile into the node |
| threshold-vault-backed ZEC | fully backed / 1:1 without qualification | backed, by a vault two of three of our machines can move |
| Zyn, the execution layer | the chain, our chain, a chain we built | "chain" invites *which consensus? who validates?* — the same questions "L2" invites, and it slipped through four drafts because it sounds harmless |
| how the VM clears / what the mechanism does | how our users trade / what traders get | **built and enabled is not in use.** Batch clearing is switched on in production and nothing has ever traded through it; describe the mechanism, never the usage |
| Cave: a **ZEC-settled object market** powered by Zyn | private object market (as a description of today) | privacy is the destination; keep it in the future tense |

## The privacy sentence

Approved, and the strongest form currently true:

> The collection, the floor and the art are public by design. Who holds what is
> not visible to the public — the ownership journal is encrypted, and items are
> received into fresh per-collection accounts. It is visible to us and to
> authorised verifiers today; we are working toward a version where it is not
> visible to anyone.

Note the two halves. Dropping the second is the overclaim.

## Forum framing

> **Zcash has private money. We're building Zyn so that money has somewhere to
> do things.**
>
> Zyn is an application execution layer built for Zcash, beginning with Cave, a
> ZEC-settled consumer marketplace. Zyn currently uses an operator sequencer and
> a threshold vault, while committing application state to Zcash for public
> timestamping and verification.

## What is genuinely strong, and worth leading with

These survive scrutiny because they are receipts rather than adjectives:

- **`backing.rs`** — a signer refuses to endorse a credit its *own* Zcash node
  cannot see. Proof of reserves by construction, not an attestation.
- **`exitproof.rs`** — a holder keeps evidence of what they held, checkable
  against an anchored root without our cooperation. *(Evidence, not a
  withdrawal. Say which.)*
- **Forced inclusion** — a refusal to sequence becomes a record.
- **Per-signer liveness and the backing requirement** — a threshold that clears
  cannot hide a signer who checked nothing.

A custodian cannot produce any of these. That is the pitch, and it does not
need to be inflated to work.
