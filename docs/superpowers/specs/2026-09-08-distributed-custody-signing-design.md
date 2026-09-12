# Distributed custody signing: no one machine can pay the vault alone

Status: approved 8 Sep 2026 (custody, step 1 of 2; DKG is step 2). Zcash
(RedPallas) custody first — the key that signs anchors and Zcash exits.

## The property

A vault payout — an exit, or the anchor self-send — is signed by a threshold
of share-holders each running on its own machine, none of which ever holds
more than one share. A compromised or seized signer machine cannot move funds;
it can only decline to. Today all shares sit in one directory and the settler
signs in-process — the exact single point of compromise this removes.

## What already exists

`signing::Session` (round-1 commit, round-2 `sign` consuming `self` so a nonce
is single-use), `signing::Coordinator`/`Aggregator` ("untrusted by
construction: it sees commitments, a message, and shares, none of which reveal
a key"), and the rerandomized helpers `sign_share`/`params_for`/`aggregate`.
`payout::sign_all` drives them in-process. The crate was built to distribute;
this wires it across machines.

## Mechanism

**A `Quorum` abstraction** replaces the `keys: &[(Identifier, &VaultKeys)]`
argument to `sign_all`. Two rounds, per payout (not per action — a payout with
k actions still costs two round trips):

```
trait Quorum {
    fn identifiers(&self) -> Vec<Identifier>;
    // round 1: each participant commits, one commitment per action alpha
    fn round1(&mut self, sighash, alphas) -> Vec<(Identifier, Vec<SigningCommitments>)>;
    // round 2: each participant signs, one share per action package
    fn round2(&mut self, packages) -> Vec<(Identifier, Vec<SignatureShare>)>;
}
```

`sign_all` becomes: gather `needs_signatures` → `round1` → build each action's
`SigningPackage` from the i-th commitments of a threshold of participants →
`round2` → `aggregate` per action with `params_for(alpha)` → `apply`. The
existing in-process behaviour is `LocalQuorum` (holds the shares), used by the
V0 settler and every test. `RemoteQuorum` speaks to custodian daemons.

**The custodian daemon** (`zyn-custodian`) holds exactly one share
(`ZYN_CUSTODY_SHARE`, a share dir with one `share-*.bin` + `public.bin`) and
serves two ops on a framed socket:
- `ROUND1 { request_id, sighash, alphas[] }` → begins one `Session` per alpha,
  stores them under `request_id` in memory, returns `identifier` and the
  commitments. A second `ROUND1` for a live `request_id` is refused (a nonce
  is committed once).
- `ROUND2 { request_id, packages[] }` → consumes each stored session,
  `sign_share` under the action's alpha, returns the shares, drops the
  request. An unknown `request_id` (e.g. after a restart) is refused: the
  coordinator retries the whole payout with fresh nonces. **Fail closed** — a
  lost signing attempt is nothing; a reused nonce is the share.

Requests live only in memory; a restart forgets them, which is the safe
direction. Each carries a deadline so an abandoned request cannot pin nonces
forever.

**The coordinator** (`RemoteQuorum`, in the settler) holds no share — only the
custodian addresses, the public package, and the group verifying key, all
public. It runs round 1 against all custodians, keeps whichever ≥ threshold
answered, runs round 2 against exactly those, and aggregates. A custodian that
drops between rounds fails the payout; the settler retries, and one down out of
n>threshold is tolerated on the retry.

**The settler**: `ZYN_CUSTODIANS=host:port,…` and `ZYN_CUSTODY_THRESHOLD`.
When set, `ZcashSettler` and the anchor `ZcashBuilder` sign through a
`RemoteQuorum` and hold no shares; the viewing key and public package stay (to
build and aggregate). Absent, the in-process `LocalQuorum` runs exactly as now.

## What this is not

Not distributed key generation — the shares were still born together in one
ceremony (step 2). Not Solana custody yet (same abstraction, a later pass).
Not a change to what is signed or to the exit/anchor logic: only *where the
shares live* at signing time.

## The honest limit of the demo

Three custodians on the three signer machines (Mac, Windows, Hetzner), one
share each, threshold two — a devnet key on testnet. No machine holds a
quorum, which is the real property; true independence is separate operators,
which is operational. And the shares were generated together until step 2
(DKG) lands.

## Tests

`custodian`: round1 then round2 reproduces exactly the signature `sign_all`
makes in-process (drive both, compare bytes on a real Orchard action, reusing
the `orchard_signing_tests` fixture); a second round1 for a live request is
refused; round2 for an unknown request is refused; a dropped request frees its
nonces. `payout`: `sign_all` over a `LocalQuorum` equals the old path; over a
fake in-memory `RemoteQuorum` (channels, not sockets) equals it too; a quorum
one short fails, threshold exactly succeeds. `zyn-custodian`: round-trips over
a real socket. Live: three custodians, threshold two; an anchor self-send
signs with no machine holding two shares; stop one custodian and the next
still signs on the other two; the settler holds no share file.
