# NFT ownership privacy: public readers versus trusted operators

Decision date: 13 September 2026. Implemented 16–17 September.
Local source: DECISIONS §70, §92–§98; BACKLOG P0–P4, A90–A95.

## Where this stands

**Ownership is hidden from public readers. It is not hidden from the operator,
and saying otherwise is the claim §70.2 already retracted.** That is the tier
§70.1 chose — operator-trusted, not operator-private — and it is defensible
stated plainly. Overclaiming here is the one thing that cannot be walked back.

What each observer sees today:

| Observer | Sees |
|---|---|
| Anyone with a mirror copy | Roots, leaves, anchors, certificates. Leaves are `H(record)` over a record carrying the holder's blind, so nothing about balances or items |
| Anyone watching Zcash | A shielded payment to a diversified vault address. No link to a Zyn account |
| A keyed verifier / the operator | The journal, and so the whole picture |

### Built

- **The journal and base state are encrypted** (§93). They are the only two
  files in a bundle that carry ownership, and they carry it because they are
  what a verifier replays. Encrypted rather than withheld, so a mirror that
  cannot read a file can still keep it: durability and permissionless *hosting*
  survive, permissionless *replay* does not (§70.4's trade, stated).
- **Items are received into per-collection accounts** (§95, §97.5), so what the
  journal names is an account with no other history.
- **Item operations are signed by the account that holds the item** (§98.1) —
  a correctness requirement once items live there, with unlinkability as the
  consequence.
- **The browser wallet no longer fetches art for every holding** (§96). It did,
  unconditionally, which handed the gateway a complete owner map keyed by IP —
  off the chain entirely, and untouched by any encryption.
- **The whitelist registry stores a commitment, not the account** (§98.2), so
  that file alone is not an invitation-to-account map.
- **Backing verification is required of enough endorsers** (§97.2), which is
  what makes it *safe* to narrow the vault viewing key to fewer parties.

### Not built, and honest about it

- **The operator still sees execution.** Only notes, nullifiers and proofs
  change that, and §70.9 defers them. The nearer path is reducing *who the
  operator is* — threshold sequencing, independent operators (A88) — which
  costs no latency, rather than putting a prover on a user's device, which is
  Zcash's own UX tax imported into the thing Zyn differentiates from.
- **Nothing is retroactive** (§70.8). Everything published before the cutoff is
  plaintext and stays so.
- **Deposit attribution is derivable by anyone holding the vault FVK** (§97.1),
  deliberately: it is what makes a credit checkable rather than the operator's
  word. A95 is the upgrade that keeps both.
- **Deployment is pending**: A90 (the DA key), A92 (wire the mint to receiving
  accounts), A94 (narrow the FVK, backing set named *first*).

### What may be said

> The collection, the floor and the art are public by design. Who holds what is
> not visible to the public — the ownership journal is encrypted, and items are
> received into fresh per-collection accounts. It is visible to us and to
> authorised verifiers today; we are working toward a version where it is not
> visible to anyone.

Nothing stronger until the row carrying it is built.

## Architecture constraints

- Separate allowlisted public commitments/anchors/certificates and reviewed
  aggregate/catalog information from confidential full journals, base states,
  private records and ownership-bearing proofs. Review hash/index/order/timing
  inference too; absence of plaintext is not a proof of unlinkability.
- Choose and test confidential authenticated distribution, reader authorization,
  provisioning, rotation/revocation, cache policy and diagnostic redaction.
  Exact mechanisms remain design work; hiding only NFT opcodes is insufficient
  in a shared replay journal.
- Authorized independent verifiers must continue to replay and reject invalid
  state, preserving backing/quorum checks. Restricting their inputs reduces
  unrestricted public replay. Public roots or operator certificates are not an
  equivalent substitute for permissionless execution verification.
- Preserve authenticated holder access and durable export of anchored records
  and inclusion/exit witnesses. Test recovery, outage and forced-exit behavior;
  state any dependence on previously retained witnesses. Do not change live
  publication until this recovery contract and its tests pass.
- Fresh receiving accounts require recoverable, versioned derivation and tested
  account discovery. Blinding requires confirmed execution and a confidential
  journal path. A disclosed blind is not secret; future rotation cannot undo
  already public ownership history.
- Review offers, public listings, claims, explorers, logs and media requests so
  these surfaces do not publish an alternate owner map. Public listings must
  require informed opt-in where they disclose ownership.
- Set a future chain/epoch privacy cutoff. Do not promise to erase previously
  distributed records. A rollback to plaintext publishing must not silently
  expose transactions submitted under a privacy expectation.

Aliases and operator-private NFT notes/nullifiers/zero-knowledge proofs are
explicitly deferred; neither is implemented by this workstream.

## Backlog and acceptance

| Local ID | GitHub issue | Dependencies | Required evidence |
|---|---|---|---|
| P0 | [#21 — Disclosure audit and threat model](https://github.com/zyn-io/nap-wallet/issues/21) | First | Artifact/reader/metadata matrix, synthetic leakage fixture, separately documented live exposure and corrected claims |
| P1 | [#22 — Restricted replay with verification and recovery](https://github.com/zyn-io/nap-wallet/issues/22) | P0; coordinate #1/#15 | Unauthorized access denial; authorized root reproduction/tamper rejection; holder proof export, restore and outage/exit drills; explicit public-replay tradeoff |
| P2 | [#23 — Recoverable receiving accounts and blinding](https://github.com/zyn-io/nap-wallet/issues/23) | P1; #15/#16 | Exact account/item recovery after clearing browser state; initialization/rotation ordering; no false privacy status; preserved legacy authority |
| P3 | [#24 — Secondary ownership disclosures](https://github.com/zyn-io/nap-wallet/issues/24) | P0; integrates #6/#8/#13/#16/#17 | Public/network/log inspection through receive/view/claim/list/sell; explicit listing disclosures; private claim allocations; existing media-privacy checks |
| P4 | [#25 — Integrated rollout gate](https://github.com/zyn-io/nap-wallet/issues/25) | P0–P3 | Leakage plus replay/backing/quorum/recovery/outage/exit/revocation tests; disposable-testnet rollout and rollback; documented chain/epoch cutoff |

These acceptance tests are requirements to implement, not claims of completed
testing. Existing claim and media issues keep their implementation scope; P3
coordinates them instead of creating another claim service or gateway.

No mainnet action, live mirror restriction, cache purge, history deletion,
automatic wallet migration or funded event launch is authorized by these issues.
