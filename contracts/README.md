# ZynVault — the EVM side of the bridge

One contract, deployed at the **same address on every EVM chain**, holding
value against units issued on Zyn.

Design and the reasoning behind it: `DECISIONS.md` §14d (why a contract) and
§14e (why the microchain decides what it signs).

## Why a contract rather than an address

A deposit has to say who it is for. An ERC-20 `transfer` to an externally owned
account carries no memo and no calldata, so a watcher could only key on the
**sender** — publishing a permanent link from every depositor's EVM identity to
their Zyn account. On a chain whose premise is shielded trading that is the
whole leak, at the front door.

`deposit(bytes32 zynAccount, ...)` puts the destination in calldata, where it
need not relate to `msg.sender` at all.

## The two roles, and what neither can do

| | `signer` | `admin` |
|---|---|---|
| authorise a withdrawal | yes | **no** |
| register a *new* asset | no | yes |
| re-point an existing asset | **no — nobody can** | **no** |
| pause deposits | no | yes |
| pause withdrawals | **no — nobody can** | **no** |
| move funds directly | no | no |

Two asymmetries are deliberate. **Withdrawals cannot be paused**, because the
power to halt exits is the power to trap other people's money. And asset
registration is **add-only**, because re-pointing a live asset id at a different
token would let that role rewrite what deposited value *is*, which is
indistinguishable from being able to spend it.

`signer` is immutable and may be either:

- a **Safe** (k-of-n), verified through EIP-1271 — where to start; its owner set
  rotates without changing the Safe's address, and therefore without touching
  this contract; or
- a **threshold-ECDSA** address, verified through `ecrecover`.

`SignatureChecker` accepts both, so moving between them never redeploys the
vault or changes its address.

## The digest is a cross-language contract

`WITHDRAWAL_TYPEHASH` and `PAYOUT_TYPEHASH` are hard-coded here and generated in
`zyn-bridge/src/evm.rs`. Nothing at runtime would notice them drifting apart: a
mismatched typehash makes every signature fail to verify, on a chain holding
funds, with no error that points at the cause.

So both sides are pinned by tests, and one test checks them against each other:

- Rust: `the_type_strings_are_what_the_contract_expects` prints the vector
- Solidity: `test_digest_matches_the_rust_vector` reproduces it

**Change either encoding and both tests must be regenerated together.**

## Replay

The vault is deployed by CREATE2 at one address everywhere, which is what makes
adding a chain a config row instead of an audit. The direct consequence is that
a digest which does not bind the chain authorises the identical withdrawal on
all of them.

- across chains — the EIP-712 domain binds `chainId` and `verifyingContract`
- on one chain — a strictly increasing `nonce`, consumed before payout

`test_a_signature_does_not_cross_chains` asserts the first;
`test_a_withdrawal_cannot_be_replayed` the second.

## Scaling

Zyn's `Fixed` is WAD-scaled (1e18) precisely to match Solidity, so an
18-decimal token needs no conversion. Anything else is scaled here, and an
amount that does not divide exactly **reverts rather than rounds** — silently
dropped dust is how a bridge's books stop balancing. The VM's exit minimum is
what keeps that unreachable in practice.

## Build and test

```sh
forge build
forge test
```

Dependencies are vendored under `lib/` (OpenZeppelin 5.1.0, forge-std 1.9.6)
rather than installed with `forge install`, because the workspace is not a git
repository.

## Deployment

Deploy through the deterministic CREATE2 proxy at
`0x4e59b44847b379578588920cA78FbF26c0B4956C`, present at that address on most
EVM chains, with a **fixed salt and identical bytecode** — that is what yields
one address everywhere. Identical bytecode means identical compiler settings;
`foundry.toml` pins `solc` and the optimizer for that reason.

Order matters: deploy, then `registerAsset` each asset, then deposit. An asset
cannot be corrected once registered, only added alongside under a new id.

## Not built yet

- The **watcher** — reads `Deposited` and turns it into a credit intent. Same
  shape as `zyn-custody/src/zebra.rs`.
- Choosing and standing up the `signer` (a Safe to begin with).
- A **Level 2** variant holding a state root, so exits are permissionless
  rather than merely permissionless *to submit* (§14e).
