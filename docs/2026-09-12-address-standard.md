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

`program` is `VM_NAME ‖ VM_VERSION` big-endian — for ZynZap v2, the eight bytes
`zynzap\x00\x02`. A version bump moves every address the VM derives, which is
the point: a version that changes what a record means should change where it
lives.

There is **no bump seed**. Solana grinds one because its addresses are ed25519
public keys and a program-owned address must be proved to have no private key.
Zyn's `AccountId` is an opaque 32 bytes with no curve behind it, so there is
nothing to be off (§4.2).

A derived address **is** an `AccountId`. A pool holds balances through the
ordinary path and is proved through the ordinary exit hatch.

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

The namespace is what keeps `["pool", x]` and `["vault", x]` unrelated.

### Two scoping rules, and why they differ

**A bridged asset names its origin network, not its chain family.** `zcash` and
`zcash-test` are different assets because ZEC and TAZ are different assets. A
testnet bridge minting something addressed as mainnet ZEC would be claiming
backing it does not hold. The same applies to Solana devnet and Bitcoin signet.

`origin_asset` is empty for a network's own coin and carries the origin contract
or mint for a token issued on it — so `USDC.zy` is addressed by Solana's actual
USDC mint and can be recomputed from public facts about that chain.

**A chain's own token carries the Zyn chain id; a bridged asset does not.**
Without it, mainnet ZYN and testnet ZYN are the same 32 bytes and an address
shown on its own cannot tell a holder which they have. Bridged assets need no
such seed because their origin network already separates them. The scoping goes
exactly where the ambiguity is, and nowhere else.

### Symbols are unique per creator

Two creators may both issue `CAT` and get different addresses; one creator
cannot issue `CAT` twice.

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

### Pools over them

A pool address follows from its assets alone, so these need no registry.

| pool | address |
|---|---|
| ZEC.zy / ZYN | `4d9b08dbee9c82781b59dfa144730399c2b23ad320a2610601702ff305ddd999` |
| SOL.zy / ZEC.zy | `142639c35adc1be4183c2268146f069d3b188988cb143b89be7ba1a67b2b68d7` |
| BTC.zy / ZEC.zy | `ed3fef337e34a8c7b3343fc832f9ae4c289226cb51cebd5813b53746df18cab8` |

`ordered_pair` sorts the seeds, so a pool over `(A,B)` and one over `(B,A)` are
the same address. The split-liquidity bug is unrepresentable rather than
forbidden by convention.

## NFTs

A collection is one address; its items are derived from it.

```
CAVE collection   a63c862561ee34f07982d9a2001acee93e39acdc55f6d9aa3760ba2cdfdf8493
  item #0         81b56002d52dd701e0997ab6f1cb244b3d62e8cd750879688b87909996a54378
  item #1         cb878f3491cf90956acf6c823047f517ebd7452f059571628f8db012d049f8d5
  item #4443      1bb22128336748b7d8a0cf656e2792ab55a902b1f9fa2f7edbdf0d638401c688
```

(Creator `0xca7e…ca7e`, symbol `CAVE`; substitute the real creator before
minting — the addresses move with it.)

Deriving item ids from the serial means every item's identity is computable
before the collection mints, which is what a reveal schedule or an allowlist
needs. `collection::item_leaf` binds the collection address into the leaf, so an
item proved against one collection cannot be replayed against another.

## What this does not settle

- **Adoption.** `AssetId` is still `u32` and `zyn_vm::derive` still has no
  callers in `swapvm`. §4.4 remains open.
- **Ordering against compressed collections.** If `zyn_vm::collection` lands
  first, items leave the token map entirely and the address migration has 4,444
  fewer entries to widen. Compression first, then addressing — or decide both
  together.
- **Metadata.** Settled separately — see `zyn_vm::manifest` and DECISIONS §67.
  `content` is `H(manifest)`; the manifest names media by hash with no URL, the
  location is a hint outside consensus, and the wallet verifies bytes before it
  renders them.
