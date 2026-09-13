# The Zyn address standard

12 September 2026 · defines `zyn_vm::derive` · status: **specified, not yet live**

The VM still uses `AssetId = PoolId = CollectionId = u32`. This document defines
what replaces them. Adopting it changes the committed encoding, so it is a fork
(DECISIONS §4.4, §66.8).

## Why derived addresses

A monotonic `u32` depends on the order intents happened to be sequenced. Three
things follow, and all three are true of the chain today:

1. **Nothing can be computed offline.** A client cannot name a pool before it
   exists, so a swap UI cannot build a route without a round trip.
2. **Names are not content-addressed.** Two chains holding the same pools
   disagree about what they are called.
3. **Lookups scan.** `find_pool` (`swapvm/src/state.rs:731`) walks every pool,
   because a maintained index is a second thing that can disagree with the data
   it came from.

A derivation cannot disagree with anything.

## The derivation

```
addr = SHA256( "zyn.derive.v1"
             ‖ len(program)   ‖ program
             ‖ len(namespace) ‖ namespace
             ‖ count(seeds)
             ‖ (len(seed) ‖ seed)* )
```

Every component is length-prefixed, so no regrouping of the same bytes produces
the same preimage — `["ab","c"]` and `["a","bc"]` cannot collide. Solana
concatenates seeds and survives it only because seeds are length-capped; that is
a sharp edge inherited rather than chosen.

Persistent ZynZap identities use the frozen address scope
`ADDRESS_SCOPE_V1 = b"zynzap\x00\x02"`. It is the eight-byte scope under which
the published vectors were derived. An ordinary execution `VM_VERSION` or
`STATE_VERSION` bump must not move assets, pools or collections. A new address
scope is allowed only for an explicitly incompatible address-schema fork.

The current generic `derive_for<V>` still builds its scope from `VM_VERSION`.
A25 must therefore introduce the frozen constant and use it for persistent
identities before the next execution-version bump. `program_scope_of` must also
reject names longer than 32 bytes instead of silently truncating them.

There is **no bump seed**. Solana grinds one because its addresses are ed25519
public keys and a program-owned address must be proved to have no private key.
Zyn's `AccountId` is an opaque 32 bytes with no curve behind it, so there is
nothing to be off (§4.2).

All identifiers are 32 bytes, but their roles remain distinct. An asset address
is an `AssetId`, a pool address is a `PoolId`, and a derived vault is an
`AccountId`. A pool's two reserves live in `vault(pool)`; its pool record commits
the invariant, fee policy, reserve assets, LP asset and vault address. No user
key can authorize a derived vault.

## Namespaces

| kind | derivation |
|---|---|
| native | `derive(P, "native", [chain_id, symbol])` |
| bridged | `derive(P, "bridged", [origin_network, origin_asset])` |
| user token | `derive(P, "asset", [creator, symbol])` |
| pool | `derive(P, "pool", ordered_pair(a, b))` |
| LP asset | `derive(P, "lp", [pool])` |
| pool vault | `derive(P, "vault", [pool])` |
| collection | `derive(P, "collection", [creator, symbol])` |
| item | `derive(P, "item", [collection, serial])` |
| curve inventory | `derive(P, "curve-inventory", [asset])` |
| graduation inventory | `derive(P, "graduation-inventory", [asset])` |
| refundable curve reserve | `derive(P, "curve-reserve", [asset])` |
| launch pair seed | `derive(P, "pair-seed", [asset])` |
| graduation-fee pot | `derive(P, "graduation-fees", [asset])` |
| creator escrow | `derive(P, "creator-escrow", [asset])` |
| graduation LP lock | `derive(P, "lp-lock", [pool])` |
| future ZYN POL pot | `derive(P, "zyn-pol", [ZEC.zy])` |

The namespace is what keeps `["pool", x]` and `["vault", x]` unrelated. Cave
uses separate keyless accounts for amounts with different movement rules; the
refundable reserve, pair seed, graduation fees and creator escrow never share a
balance even though they all eventually participate in one graduation.

### Canonical seed bytes

User-token symbols are 1–32 bytes of valid UTF-8. CJK, emoji, mixed case and
punctuation are allowed. Unicode whitespace, NUL/C0/C1 controls, line separators
and bidi override/isolate characters are rejected. The exact UTF-8 bytes enter
derivation: the VM does not case-fold, normalize, trim or truncate after
signature. NFC and non-NFC spellings may therefore be different assets even
when a font renders them alike. Wallets must show a short address beside the
symbol whenever identity matters.

The `creator` seed is the 32-byte owner account named by the creation intent and
credited with the creator fee. It is not a browser session key, delegation key,
relayer or HTTP submitter. Different creators may use exactly the same symbol,
including an official ticker such as `ZYN`. One creator may never reuse the same
exact symbol bytes because they would derive the same asset address; the second
creation is rejected and an asset address is never recycled.

Integers are unsigned big-endian: `chain_id` and NFT `serial` are `u32`. Symbols
enter derivation without fixed-width NUL padding.

### Two scoping rules, and why they differ

**A bridged asset names its origin network, not its chain family.** The first
registry entries are exact lowercase ASCII: `zcash`, `zcash-test`, `solana`,
`solana-devnet`, `bitcoin`, `bitcoin-test`, `bitcoin-signet`, and EVM networks
as `eip155:<chain-id>` with canonical decimal digits and no leading zero. Aliases
are invalid: `eth`, `ethereum`, and `eip155:01` must not name Ethereum mainnet.

`origin_asset` is empty for a network's own coin. A Solana token uses the raw
32 bytes obtained by Base58-decoding its mint; an EVM token uses the raw 20-byte
contract address. Display strings, Base58 text, `0x`, checksum capitalization and
ABI padding never enter the seed. Other origin families require an address-byte
rule in this registry before their first asset is accepted.

**A chain's own token carries the Zyn chain id; a bridged asset does not.**
Without it, mainnet ZYN and testnet ZYN are the same 32 bytes and an address
shown on its own cannot tell a holder which they have. Bridged assets need no
such seed because their origin network already separates them. The scoping goes
exactly where the ambiguity is, and nowhere else.

### Symbols are labels; addresses are identity

Two creators may both issue `CAT`, `玉` or `ZYN` and get different addresses;
one creator cannot issue the same exact bytes twice. Native, bridged and user
assets already occupy different namespaces, so a user token named `ZYN` cannot
take the official native ZYN address.

Official status is an address property. Wallets and Cave ship the protocol's
pinned set of official native, bridged and collection addresses and render a
badge only for an exact match. Symbols, display names, images and manifests can
never grant the badge. Official addresses are derived and inserted before their
public use; no ticker is reserved from user launches.

### Text representation

Consensus and state use the raw 32 bytes. Wallets, public HTTP/RPC responses and
URLs use lowercase Bech32m with a type- and network-specific HRP: `zyna`, `zynp`,
`zync`, `zyni`, `zynv` on mainnet and the corresponding `tzyna`, `tzynp`,
`tzync`, `tzyni`, `tzynv` on testnet. These represent asset, pool, collection,
item and program-vault addresses. CLI/debug output may additionally accept and
show 64-character lowercase hex, but hex is not the primary copy/paste format.

## The genesis table

ZynZap v2. Asserted in `zyn-vm/src/derive.rs::the_genesis_addresses_are_what_was_published`
— if that test fails, every published address moved, which is a fork and not a
refactor.

| asset | seeds | address |
|---|---|---|
| ZYN (mainnet) | `native`, `26460`, `ZYN` | `49eb9241e2a0b1163a1eed09bb91a575c93bbc6e76cd7afeb65263640fd169d5` |
| ZYN (testnet) | `native`, `11`, `ZYN` | `893e745e33b40310003011a14651dc3cf173f71b92ee6e77ef62b7ba5a84e56e` |
| ZEC.zy | `bridged`, `zcash`, `∅` | `0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201` |
| TAZ.zy | `bridged`, `zcash-test`, `∅` | `bf244a27ad73fea5128fead691c647fe1c5a5cddde0a847b24ca20041250da48` |
| SOL.zy | `bridged`, `solana`, `∅` | `0f61f4340cd6a910ec8d7e73ef239569b83f16099e620786e71e7e7395c9f6df` |
| BTC.zy | `bridged`, `bitcoin`, `∅` | `0e7faa55f3bdb12e80781da88fb5ac4eb7e24d1935494b9aabd84c4565bbffca` |
| USDC.zy on Solana (address example, not the selected stable) | `bridged`, `solana`, raw mint `EPjF…Dt1v` | `83e66d694af68a844f4ea9a61d73bce7ffa8e53fc780023868f825c2e753c79a` |
| BOLD.zy on Ethereum mainnet | `bridged`, `eip155:1`, raw contract `6440…b01d` | `ddf2b7ecc2634e39a891a19020205ae26a87e9f753e492e03b84e7dc627f250f` |

### Pools over them

A pool address follows from its assets alone, so these need no registry.

| pool | address |
|---|---|
| ZEC.zy / ZYN | `4d9b08dbee9c82781b59dfa144730399c2b23ad320a2610601702ff305ddd999` |
| SOL.zy / ZEC.zy | `142639c35adc1be4183c2268146f069d3b188988cb143b89be7ba1a67b2b68d7` |
| BTC.zy / ZEC.zy | `ed3fef337e34a8c7b3343fc832f9ae4c289226cb51cebd5813b53746df18cab8` |

For the current ZEC.zy/ZYN pool:

```text
LP asset    de3a8a633ceccc69aef8069fab4d16b75c5410a5b3838c3202b88e4a854eb9f5
pool vault  563aba537ce9c65bae3131edad46815edd9240f66b3c70f03173f05e7fc67211
```

`ordered_pair` sorts the seeds, so a pool over `(A,B)` and one over `(B,A)` are
the same address. The split-liquidity bug is unrepresentable rather than
forbidden by convention.

## Official NFT collections

A collection is one address; its items are derived from it. Both official NFT
collections use the same protocol treasury creator account, named **Treasury
26460**:

```text
Nap Mole (333):  collection_address(P, Treasury 26460, "NAP")
Cave (4,444):    collection_address(P, Treasury 26460, "CAVE")
```

`Treasury 26460` is the stable name for the spendable threshold treasury account
stored in chain parameters. Its exact 32-byte `AccountId` must be provisioned
and pinned before either collection is created. The old `0xca7e…ca7e` examples
were placeholder vectors and are not mint addresses.

Deriving item ids from the serial means every item's identity is computable
before the collection mints, which is what a reveal schedule or an allowlist
needs. `collection::item_leaf` binds the collection address into the leaf, so an
item proved against one collection cannot be replayed against another.

## Presentation

Public addresses are typed, network-aware lowercase **Bech32m** (§66.15). Mainnet
HRPs are `zyna` / `zynp` / `zync` / `zyni` / `zynv`; testnet prefixes each with
`t`. Consensus retains the raw 32 bytes; the lowercase hex used throughout this
document is a CLI and debugging form, not the wallet or URL format.

## What this does not settle

- **Adoption.** `AssetId` is still `u32` and `zyn_vm::derive` still has no
  callers in `swapvm`. §4.4 remains open. A25 must replace ids and counters,
  move pool reserves to their derived vaults, add Cave's keyless accounts, and
  update state, wire, Merkle, receipts, RPC, wallet, bridge, explorer and replay.
- **Ordering against compressed collections.** If `zyn_vm::collection` lands
  first, items leave the token map entirely and the address migration has 4,444
  fewer entries to widen. Compression first, then addressing — or decide both
  together.
- **Metadata.** Settled separately — see `zyn_vm::manifest` and DECISIONS §67.
  `content` is `H(manifest)`; the manifest names media by hash with no URL, the
  location is a hint outside consensus, and the wallet verifies bytes before it
  renders them.
- **Treasury provisioning.** The creator choice and name are settled: both
  collections use Treasury 26460. Pin its threshold `AccountId`, then publish
  exact Nap Mole/Cave collection and item vectors before minting.
