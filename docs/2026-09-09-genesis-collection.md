# Zyn Underground, Collection 01 — the genesis sale

The first dapp on Zyn, and deliberately a trojan for it: people come for the
artwork and leave having used a shielded bridge, a batch-settled trading layer and
a redeemable floor without being asked to care about any of them.

Front-end home: **cavebro.win**, kept separate from the Zyn/ZynZap brand.

> **Current identity and fee note (13 Sep 2026).** The collection creator and
> creator-fee recipient is the threshold protocol account named **Treasury
> 26460**, shared with the 333 Nap Mole collection. The secondary fee described
> below is historical: DECISIONS §69.9 supersedes it with 50% to future ZYN POL,
> 40% to the collection floor and 10% to Treasury 26460. Pin the treasury's real
> 32-byte AccountId and recompute the official collection/item addresses before
> minting.

## The collection

| | |
|---|---|
| Supply | **4,444**, fixed at creation |
| Species | Mole 2,222 (50%) · Owl 1,333 (30%) · Moth 889 (20%) |
| Format | 192×192 PNG, 16-colour indexed palette, ~35 KB each |
| Total size | 154 MB |
| Metadata | `zyn-atomic-components/v1`, traits + `zyn-light-v1` viewer contract |
| Status | all three sets generated and validating `status: pass`, unique image hashes |

Source of truth: `examples/zyn-pfp-concepts/{mole-collection-2222,
owl-collection-1333, moth-collection-889}`, with `COLLECTION-SPEC.md`,
`NFT-METADATA.md` and `STORE-FILTERS.json` alongside.

## Economics

| Stage | Price | Notes |
|---|---|---|
| Deposit | **0.01 ZEC** | one lottery ticket; mainnet deposit to the vault |
| Draw | — | 1 item 80% · 2 items 19% · 3 items 1% |
| Claim (mint) | **0.005 ZEC** | second mainnet deposit; the moment the piece is handed over |
| Open mint | **0.0133 ZEC** | whatever the sale did not allocate |
| Treasury | **10%** of all deposits and mints | taken before the pool is funded |
| Trade fee | **1% per side** | historical `50/50`; superseded by §69.9's `50/40/10` |
| Floor | `pool / outstanding` | rises with every trade and every redemption leaves it unchanged |

**Both payments are mainnet deposits** (decided 9 Sep 2026). The claim is its own
Zcash transaction rather than a Zyn-side transfer of already-bridged value, so
every zatoshi of sale proceeds arrives in the vault as an auditable mainnet
deposit, and the buyer never needs ZEC.zy before they own anything.

### Worked example, full allocation

Expected items per ticket: `1(0.80) + 2(0.19) + 3(0.01)` = **1.21**

```
tickets to allocate 4,444      4,444 / 1.21   ≈ 3,673
deposits                       3,673 × 0.01   ≈ 36.73 ZEC
claims                         4,444 × 0.005  =  22.22 ZEC
gross                                            58.95 ZEC
less 10% treasury                              −  5.90 ZEC
pool                                             53.05 ZEC
floor at listing               53.05 / 4,444   ≈ 0.01194 ZEC
```

Average cost of one item via the lottery is `0.01/1.21 + 0.005` ≈ **0.01326 ZEC**,
so the floor is ~90% of what it cost — the treasury is exactly that gap, which is
the honest way to describe it.

Open mint at 0.0133 contributes `0.0133 × 0.9` ≈ 0.01197 to the pool per item,
marginally **above** the floor, so late buyers are mildly accretive rather than
dilutive and the lottery route stays the cheaper one. That pricing is deliberate;
keep the three numbers in step if any of them moves.

## Phases

The VM's phase machine (`swapvm/src/state.rs:79`) is the sale timeline, and it is
one-way by construction:

| Phase | Sale stage | Rules |
|---|---|---|
| `Depositing` | tickets on sale | money in, **no items exist yet** |
| `Minting` | claims + open mint | items handed out, pool still filling, **nothing redeemable** |
| `Closed` | claims over | denominator final, market not yet open |
| `Live` | listed and trading | redemption open, floor real |

Why the order is a safety property and not bookkeeping: while claims are still
running, `pool / outstanding` is an artefact of how far the sale has got, not a
floor. With 36 ZEC in and one item claimed it would read 36. So redemption cannot
open until the denominator is final, and minting can never reopen once it has, or
a late mint would dilute a claim someone is already redeeming against.

Two consequences that were asked for and are enforced for free:

- **Claiming cannot start until deposits finish** — `mint_collection_item` rejects
  unless `phase == Minting`.
- **Redemption opens with the secondary market**, not with the mint — `redeem_price()`
  returns zero unless `phase == Live`.

## Rules the backend owns

The lottery is off-chain by design; Zyn holds the collection, the pool and the
items, not the draw.

- **A deposit commits a ticket; future public evidence resolves it.** Fixed and
  buyer-chosen quantities are known immediately. A weighted quantity resolves
  only after its precommitted future drand round. Concealed item identities resolve
  after deposits close from the precommitted drand round and a Zcash block height
  strictly after the policy's committed deposit-close height.
  Claim still only hands over the already resolved pieces.
- **Reserve the maximum possible receipt for every unresolved ticket.** Collection
  Z's proposed weighted policy therefore reserves three items until its quantity
  draw resolves. Stop accepting tickets before those reservations can exceed
  supply; do not strand an accidental remainder and call it a team reserve.
- **Unclaimed at the end of the deposit period rolls into open mint** at 0.0133.
- **Treasury takes 10%** before the remainder is moved into the pool with
  `fund_collection`.

## What exists in code

Verified 14 Sep 2026.

**Complete** — the collection primitive is done and tested:
`create_collection`, `mint_collection_item`, `fund_collection`,
`redeem_collection_item`, `advance_collection`; the fee split in `accept_offer`
(each side pays `fee_bps`; current policy is `50%` ZYN POL / `40%` floor reserve /
`10%` creator, with protocol Treasury 26460 as creator of Cave/Nap;
`ItemNeedsAPrice` if not priced in xZEC); `redeem_price()`; intent authorities; `OP_COLLECTIONS`,
`OP_DEPOSIT_ADDRESS`; the five client methods; `/api/redeem`,
`/api/deposit-address`; per-account deposit addresses.

**Complete — the pure allocation engine:** `cave-randomness` canonicalizes and
hashes `cave-allocation/v1`, verifies pinned drand Quicknet BLS beacons, combines
the collection reveal with a predeclared Zcash block hash, samples arbitrary
positive weights without modulo bias, assigns concealed/instant/sequential
identities without replacement, emits Merkle-backed receipts and independently
replays a complete batch. Each ticket is deterministically bound to its policy,
exact Zcash output and amount, and Zyn recipient. Manifest item ids must equal the
canonical Zyn address derived from collection plus serial. Its scale test assigns
all 4,444 indices exactly once.

**Complete — allocation-to-mint identity:** `MintCollectionItem` carries the
allocated serial, rejects a serial outside the immutable cap or one already
minted, and derives the asset id from that serial rather than claim order. The
cross-crate test allocates two receipts and mints them in reverse order while
proving both canonical item ids and recipients remain exact. `POST /api/mint`
therefore requires `serial` and returns the derived asset id.

**Complete — durable sale state:** `zyn-io/cave:cave-sale` locks an externally
published policy/manifest commitment, admits each verified Zcash output once,
reserves worst-case supply before acceptance, closes only after a verified tip
passes the committed height, atomically persists the ledger, replays every
reservation and allocation on restart, and emits idempotent mint plans bound to
the receipt's serial, recipient and content hash.

**Complete — entry-payment isolation and proceeds planning:** `ZYC/v1` binds an
entry to the policy digest and Zyn recipient. The shared vault scanner surfaces
it with canonical block/transaction/output position and excludes it from the
ordinary buyer-credit path before address attribution. Cave admits it only at
the configured confirmation depth, credits the exact amount to Treasury 26460,
waits for that epoch's anchor, then plans 90% to `fund_collection` while 10%
remains in Treasury. Matching VM receipts are required and item mint plans stay
blocked until the ticket's floor funding is recorded.

**Not built — external adapters and product paths:**

1. Publish/timestamp the immutable policy and manifest; run the continuous
   Zcash/drand adapters; expose the persisted ledger and receipts publicly
2. Treasury 26460 threshold signing and submission of the already planned entry
   credit/funding actions
3. Claim-payment admission/settlement, automated claim, team reserve and open mint
4. A public claim endpoint — `mint_collection_item` is creator-only, so every mint is
   signed by the collection creator on the buyer's behalf
5. Storefront wiring. The core client and wallet API already expose place/take/
   cancel offer; Cave still has to connect its secondary-trading UI to them

## Open questions

**Where the images live.** On-chain is a 32-byte content hash per item
(`content: [u8; 32]`), so 4,444 items cost 142 KB of state and the 154 MB of PNGs
sits off-chain with integrity by hash. Candidates: served by the storefront, IPFS,
or the existing DA mirror. Undecided — and not on the hackathon's critical path,
since the video needs ~30 pieces.

**Verifiable draw is decided; publication is not yet wired.** The exact policy and
manifest root are committed before deposits. Ticket quantities use predeclared
future drand rounds. Concealed identity combines a predeclared drand round and
Zcash block height/hash with the final ticket-ledger root. The engine verifies
drand in-process and replays every receipt; the service must independently verify
and publish the Zcash chain evidence rather than asking buyers to trust its seed.

**Whether the team reserve is 1–2 pieces or a stated number.** "Whatever is left
under 3" is a consequence of the stop rule rather than a policy; stating a fixed
number up front reads better.
