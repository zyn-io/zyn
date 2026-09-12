# Escrowed offers — design

**Goal:** a chain-native primitive for a resting, binding, non-custodial offer:
one party commits an asset at a price, goes offline, and anyone may take it.

## Why this is protocol and not application

An app can already build a marketplace on `AcceptOffer`, which is co-signed by
maker and taker. What it cannot build is a *passive* one. `AcceptOffer` names the
taker in the intent the maker signs, so the maker must be online to sign for each
buyer. To make an offer that binds while its maker is offline, something has to
hold the asset — and if that something is the app, the app is a custodian: it can
steal, it must stay up, and every buyer trusts it.

So the test is not "does this need to be on-chain" but "can the app build it with
what Zyn already exposes". Passive non-custodial offers, it cannot. That makes
them infrastructure.

They are also asset-agnostic and unlock a class rather than a product —
marketplaces, OTC desks, resting bids. Nothing in this design mentions
collections, items, or listings; the collection fee attaches itself because the
existing fee logic keys off whether either leg belongs to a collection.

## Why escrow rather than a signed offer carried in the intent

The alternative was a maker signature over the offer terms, verified by the VM,
with the taker unbound at signing time. It was rejected:

- The existing replay guard keys on the *credential* (`rpc.rs:538`), so it would
  protect the taker's signature but not the maker's offer — a second taker could
  submit the same offer naming themselves. Preventing that needs a set of spent
  offer digests in state, pruned by expiry.
- It needs a second authorization path alongside the credential model.
- Cancellation has no natural expression.

Escrow needs none of that. The asset moves when the offer is placed, so a second
take is impossible by construction, cancellation is a withdrawal, and all three
intents are ordinary single-signer ones under the existing model. The order book
is then chain state, which means a storefront or explorer reads it from the node
with nothing extra to trust and no listings service to run.

**Note the deliberate difference from the AMM.** `state.rs:562` records that a
swap escrows nothing — the input stays in the account until the seal. That is
right for batch-cleared swaps, which live for one epoch while the account keeps
its funds. An offer is the opposite case: it must outlive its maker's session, so
the asset has to be committed. Same chain, opposite answer, for a reason.

## State

```rust
pub type OfferId = u64;

pub struct Offer {
    pub maker: AccountId,
    pub offer_asset: AssetId,
    pub offer_amount: Fixed,
    pub want_asset: AssetId,
    pub want_amount: Fixed,
    /// Inclusive. `u64::MAX` means no expiry.
    pub expires_at_epoch: u64,
}
```

On `SwapState`: `offers: BTreeMap<OfferId, Offer>` and `next_offer_id: OfferId`.

Escrow is one keyless account, `POT_OFFERS = [0xF6; 32]`, added to `is_pot`. The
map is the ledger of who committed what; the account is only where it sits.

## Intents

| Intent | Signed by | Capability |
|---|---|---|
| `PlaceOffer { maker, offer_asset, offer_amount, want_asset, want_amount, expires_at_epoch }` | maker | `CAP_OPERATE` |
| `TakeOffer { taker, offer }` | taker | `CAP_OPERATE` |
| `CancelOffer { maker, offer }` | maker | `CAP_OPERATE` |

All three are trades or the undoing of one: a warm session key can waste value
this way but cannot take it off the chain, which is the line `CAP_OPERATE` draws.

### PlaceOffer

Rejects unless both amounts are positive, the two assets differ, and the maker
holds `offer_amount`. Debits the maker, credits `POT_OFFERS`, assigns
`next_offer_id`. → `Receipt::OfferPlaced`

### TakeOffer

Rejects if the offer is unknown (`NoSuchOffer`), `state.epoch >
expires_at_epoch` (`OfferExpired`), the taker is the maker
(`CannotTakeOwnOffer`), or the taker cannot pay `want_amount` plus their side of
any collection fee.

Both legs land together: `POT_OFFERS` → taker for the offered asset, taker →
maker for the wanted one. The offer is removed. → `Receipt::OfferTaken`

The collection fee is the same charge `AcceptOffer` takes — each side pays
`fee_bps`, half to the pool and half to the creator, and a collection item priced
in anything but xZEC is refused with `ItemNeedsAPrice`. The computation is
extracted from `accept_offer` into one function used by both, so the two paths
cannot drift.

### CancelOffer

Rejects unless the offer exists and `maker` matches (`NotTheMaker`). Returns the
escrowed asset. → `Receipt::OfferCancelled`

An expired offer is still cancellable — expiry blocks taking, not reclaiming.
Nothing sweeps expired offers automatically; leaving them for the maker keeps the
transition free of a caller nobody authorised.

## New rejections

`NoSuchOffer`, `OfferExpired`, `CannotTakeOwnOffer`, `NotTheMaker`.

## Commitment and versioning

`STATE_VERSION` 6 → **7**. Decode reads offers only `if version >= 7`, the
established pattern from when collections were added at 6.

The root gains an offers section, appended only while `!offers.is_empty()` —
matching collections, orders and the launch. **A chain that has never had an
offer keeps exactly the roots it has now**, so live chain 11 needs no migration
and no re-anchoring.

Leaf domain: `swapvm.offer.v1`, one per offer, covering every field. An offer is
state a replayer must reproduce: it holds an asset, and an escrow nobody can
check is not one.

## Wire

Intent tags 35 (`PlaceOffer`), 36 (`TakeOffer`), 37 (`CancelOffer`); receipt tags
46 (`OfferPlaced`), 47 (`OfferTaken`), 48 (`OfferCancelled`). Typed-data entries
in `typed.rs` so EVM and Solana wallets can sign all three.

## Out of scope

Partial fills, offers against a basket, on-chain expiry sweeping, and any notion
of a listing, a store or a price feed. Discovery, browsing, filtering, metadata
and images stay in the application, which reads the order book from the node.
