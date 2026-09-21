# Settlement Lab — live integration handoff

**For:** whoever takes the frontend while the mainnet node is being set up.
**Goal:** replace the rehearsal model in `apps/zyn-settlement-lab` with real calls
to a live Zyn daemon, one beat at a time.

Nothing here needs the mainnet node, the Windows box, or any of the hardware
currently being worked on. It is all developed against **testnet chain 11**,
which is live now.

## Why this is the right piece to parallelise

The lab's README says the daemon "does not provide the public paid-claim or
AcceptOffer HTTP path". As of 9 Sep it does — the VM primitive, the RPC op, the
client methods and the HTTP routes all exist and are tested (685 tests green).
So the blocker the lab was written around is gone, and the work is now pure
integration.

Keep the lab's existing discipline: **it must not substitute simulated success
for a missing live route.** Convert beats one at a time, and leave the rest on
the local model until their route is proven.

## What is live

- Sequencer / testnet chain 11: `168.119.53.39:8099`
- Existing assets: xZEC (ZEC.zy) is asset 1, ZYN is asset 7, SOL.zy is asset 2
- Anchors are produced every 30 epochs or 1800 s, whichever comes first

## Three daemons, not two

`zynzapd`'s app holds **one key**, so one daemon is one wallet. There is no
multi-account mode and adding one is not in scope.

The demo needs **three**: wallet A, wallet B, and the **creator** — because
`/api/mint` is creator-only, so the account that hands over an item cannot be
the account that buys it. Each is its own instance on its own port with its own
key and data dir.

You *can* collapse to two by making A the collection creator, and the chain will
not object. Don't: it makes the claim read as the seller minting to themselves,
which is the opposite of what the beat is supposed to show. A third instance
costs a port and a key.

Suggested: `creator` 8977, `A` 8978, `B` 8979.

The implemented Vite bridge has three targets: `ZYN_DAEMON_CREATOR`,
`ZYN_DAEMON_A`, and `ZYN_DAEMON_B`. They default to ports 8977, 8978, and 8979,
respectively. The route prefix selects a wallet explicitly (`/daemon/a/...`),
and the allowlist still rejects arbitrary methods and paths.

## The bridge allowlist is a deliberate decision, not an oversight

`vite.config.js` now forwards the four reads and nine writes listed below and
refuses everything else. Keep this explicit list — **not** a wildcard, and not
arbitrary paths. The bridge also rejects unknown wallet roles, nested paths,
wrong methods, and request bodies above 64 KiB.

## Endpoints

All are JSON. Amounts are decimal strings in ZEC.zy ("0.005"), never floats.

### Reads

```
GET /api/overview     existing, now also carries:
                        offers[]      the order book
                        settlement    { anchors, epochs_anchored, last{epoch,txid,height,root} }
                        collections[] cap, minted, outstanding, pool, phase, redeem_price
                        items[]       what this wallet holds
GET /api/offers       { offers: [ { id, maker, mine, offer_asset, offer_amount,
                                    want_asset, want_amount, expires_at_epoch } ] }
GET /api/anchors      { anchors: [ { epoch, root, anchor_id, txid, height } ] }
GET /api/deposit-address   existing
```

`settlement` is what the status strip should show. `anchors` is the epoch → Zcash
transaction mapping — that is what turns "settled in batches" from a claim into
something a viewer can check on a chain they already trust.

### Writes

```
POST /api/fund          { collection, amount }                  -> { funded, seq, epoch }
POST /api/mint          { collection, serial, to, content, symbol? } -> { minted, serial, asset, seq, epoch }
POST /api/offer         { asset, price, amount?, expires_at_epoch? }
                                                                -> { placed, seq, epoch }
POST /api/take          { offer }                               -> { taken, seq, epoch }
POST /api/cancel-offer  { offer }                               -> { cancelled, seq, epoch }
POST /api/redeem        { asset }                               -> { redeemed, seq }
POST /api/force         { to, asset, amount } — forced Transfer via Zcash memo
POST /api/bind          {} — bind this daemon's Zcash address before withdrawal
POST /api/withdraw      existing — exit to shielded ZEC
```

Notes that will save an afternoon:

- `/api/offer` takes `asset` (the item) and `price` (in ZEC.zy). `amount`
  defaults to one whole unit, which is what an item is. Omitting
  `expires_at_epoch` means the offer rests until cancelled.
- `/api/mint` is **creator-only** and needs `serial` (the allocation's canonical
  collection index) plus `content` (32-byte hex). It rejects
  anything else with `NotTheCreator`.
- `to` and `content` are 32-byte hex strings, no `0x`.
- `/fund`, `/mint`, `/offer`, and `/take` return `seq` and `epoch`; `/redeem` and
  `/withdraw` return `seq`; `/force` returns the Zcash `txid`; `/bind` returns
  `bound`. Use exactly the fields returned as receipts rather than inventing
  missing epochs.

## The paid claim is two intents

There is no single "buy" endpoint, deliberately. A claim is:

1. the buyer calls `POST /api/fund` — their money goes into the collection pool,
   signed by them
2. the creator calls `POST /api/mint` with `serial` from the verified allocation
   receipt and `to` = that receipt's buyer — the exact item is handed over

`FundCollection` is authorised by whoever pays, so the buyer pays themselves and
the creator never moves a buyer's money. Two daemons, two calls, in that order.

## The demo sequence, as API calls

| Beat | Call | Wallet |
|---|---|---|
| 1. Deposit | send real ZEC to `/api/deposit-address`, then poll `/api/overview` | A |
| 2. Claim | `/api/fund` then creator's `/api/mint` | A, creator |
| 3. Sell | `/api/offer` then B's `/api/take` | A, B |
| 4. Forced return | `/api/force {to:A, asset:item, amount:"1"}` — the transfer rides a `ZYF` Zcash memo | B |
| 5. Redeem | `/api/redeem` | A |
| 6. Withdraw | `/api/bind` if needed, then `/api/withdraw` — back to shielded ZEC | A |
| 7. Anchor | poll `/api/anchors` for an epoch covering the run | — |

The current `/api/force` implementation constructs only `Intent::Transfer`. It
cannot force an offer or a take, so a paid forced return sale would be a false
claim. The live UI calls this beat **forced return transfer** and says explicitly
that there is no price leg. A future forced-offer/take route can restore the
eight-beat sale-back choreography without changing that distinction.

The floor rises after beat 3 from its share of trade fees alone — each side pays
`fee_bps`, of which `40%` now goes to the collection floor reserve, `50%` to ZYN
POL and `10%` to the collection creator. This supersedes the demo's historical
50/50 pool/creator wording. Show `redeem_price` from `/api/overview` on screen
and let it tick up; that is the most persuasive number in the demo and it needs
no narration.

## Anchors are scheduled — there is no manual trigger

Confirmed: no endpoint creates an anchor on demand. The sequencer anchors every
`ZYN_ANCHOR_EVERY` epochs (default **30**) or after `ZYN_ANCHOR_AFTER_SECS`
(default **1800**), whichever comes first, and the live node sets neither — so
the defaults apply and an anchor can be **up to 30 minutes away**.

The UI must therefore show **"waiting for anchor"** honestly, driven by matching
the largest observed run epoch against `/api/anchors`. Do not fake it and do not
pretend that clicking the check control triggers an anchor.

For filming this is a non-issue: the anchor beat is recorded separately once one
lands, and the video cuts. It only matters for anyone operating the lab live.

## Idempotency: what the chain already gives you, and what it does not

The RPC layer records every credential and refuses a second submission of the
same signature (`rpc.rs:591`, "already applied: this signature was seen before").
So replaying an identical signed intent is safe.

That does **not** make the HTTP routes idempotent, because the app signs a fresh
intent per call. A retried POST is a new signature and a new intent. The split:

- **Naturally single-shot** — `/api/take`, `/api/redeem`, `/api/cancel-offer`.
  The second attempt fails on its own (`NoSuchOffer`, `NotACollectionItem`),
  because the thing it acts on is gone.
- **Not protected** — `/api/fund`, `/api/mint`, `/api/offer`. A double POST
  funds twice, mints twice, or creates two offers.

So client-side guards belong on those three: disable the control while in
flight, and treat a timeout as *unknown* rather than failed — re-read
`/api/overview` before retrying. Do not add a retry loop that assumes failure.

## Deployment: local is the deliverable

The hackathon submission is a **recorded video** posted in `#zechub`, plus
source and docs in the ZecHub repo. There is no hosted-site requirement, and
nothing needs to run on anyone else's machine before 2 October.

The vite bridge is a dev-server middleware and is entirely adequate for that. It
is correct that it is not a production backend — it should not become one. If a
public storefront follows later it belongs on its own front (cavebro.win) with a
real proxy, and that is a separate piece of work that does not serve this
deadline.

## Suggested order of work

1. Done: explicit bridge allowlist with three daemon targets.
2. Done: live balances, ownership, collection floor, receipt trail, and anchor
   status are read from the daemon.
3. Done: guarded `/fund`, `/mint`, `/offer`, `/take`, `/force`, `/redeem`,
   `/bind`, and `/withdraw` actions.
4. Done: in-flight controls are disabled and a write timeout becomes unknown;
   read-back occurs before any retry can be considered.
5. Operator work remains: start the three keyed daemons, set the live collection
   id, fund testnet wallets, and record once the scheduled anchor lands.

## Out of scope

The lottery: draw, odds, allocation ledger, team reserve, open mint, treasury
split. None of it is needed for the demo, because the demo's claim is the two
intents above. It is the launch mechanic, and it is specified in
`docs/2026-09-09-genesis-collection.md` when someone gets to it.
