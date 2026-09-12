# Zyn Underground, Collection 01 — the genesis sale

The first dapp on Zyn, and deliberately a trojan for it: people come for the
artwork and leave having used a shielded bridge, a batch-settled trading layer and
a redeemable floor without being asked to care about any of them.

Front-end home: **cavebro.win**, kept separate from the Zyn/ZynZap brand.

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
| Trade fee | **1% per side** | half to the pool, half to the creator |
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

- **Allocation happens at deposit, not at claim.** A confirmed deposit assigns the
  pieces immediately; the claim is only when they are handed over. Without claiming,
  the deposit cannot be recovered — the pool is what backs the floor.
- **Stop deposits when fewer than 3 items remain**, since a ticket can win 3. The
  final 1 or 2 are reserved for the team.
- **Unclaimed at the end of the deposit period rolls into open mint** at 0.0133.
- **Treasury takes 10%** before the remainder is moved into the pool with
  `fund_collection`.

## What exists in code

Verified 9 Sep 2026.

**Complete** — the collection primitive is done and tested:
`create_collection`, `mint_collection_item`, `fund_collection`,
`redeem_collection_item`, `advance_collection`; the fee split in `accept_offer`
(each side pays `fee_bps`, half to pool, half to creator, `ItemNeedsAPrice` if not
priced in xZEC); `redeem_price()`; intent authorities; `OP_COLLECTIONS`,
`OP_DEPOSIT_ADDRESS`; the five client methods; `/api/redeem`,
`/api/deposit-address`; per-account deposit addresses.

**Not built — the sale backend, in full:**

1. Draw, allocation ledger, claim, team reserve, open mint
2. Treasury split and the `fund_collection` calls that follow it
3. A public claim path — `mint_collection_item` is creator-only, so every mint is
   signed by the collection creator on the buyer's behalf
4. **The buy/sell path**: `AcceptOffer` exists in the VM but is exposed nowhere —
   only a test at `zynzapd/src/rpc.rs:1247`. No client method, no endpoint. Nothing
   can be traded from outside the VM today. *Blocks both the sale and the demo.*
5. Storefront UI
6. No `DECISIONS.md` section — the record stops at §38

## Open questions

**Where the images live.** On-chain is a 32-byte content hash per item
(`content: [u8; 32]`), so 4,444 items cost 142 KB of state and the 154 MB of PNGs
sits off-chain with integrity by hash. Candidates: served by the storefront, IPFS,
or the existing DA mirror. Undecided — and not on the hackathon's critical path,
since the video needs ~30 pieces.

**Whether the draw should be verifiable.** The deposits are on mainnet and anyone
can audit them; the lottery result is backend-only, so buyers trust us on the one
step where trust is doing real work. A commit-reveal — publish a seed commitment
before the sale, reveal at the close, derive the draw from it — would close that,
and it is the kind of thing this audience checks for. Worth deciding before launch,
not after.

**Whether the team reserve is 1–2 pieces or a stated number.** "Whatever is left
under 3" is a consequence of the stop rule rather than a policy; stating a fixed
number up front reads better.
