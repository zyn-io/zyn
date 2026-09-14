# ZecHub hackathon — demo flow

**Deadline:** DAO vote 2 October 2026, 12:00 UTC.  
**Prizes:** 50 / 35 / 15 ZEC.  
**Requirements:** interact with Zcash **mainnet** through transactions, encrypted
memos or RPC metrics; post a video demo in `#zechub` on Discord; publish source
and documentation in the ZecHub repo.

The submission is a recorded demonstration. Mainnet needs to work reliably for
the rehearsal and recording; it does not need to remain continuously available
through the vote. The Windows box is sufficient, so no additional server is
required.

## What the video argues

One sentence:

**Fund on Zcash. Trade many times on Zyn. Verify the resulting state in one
Zcash anchor — even when the sequencer refuses you.**

The Shielded Byte card from `examples/zcash-pixel-nft` makes the mechanism
visible; it is not the pitch. Keep this demo independent of Collection Z.
Open with the transaction flow, then use the single card to make ownership,
price and redemption easy to follow.

Each piece has a claim on a transparent ZEC redemption reserve. The reserve gives
the artwork a verifiable floor without presenting the project as speculation. In
the video and submission, describe it as a **collectible with a verifiable ZEC
redemption reserve**, not a “ZEC-backed instrument.”

Zyn does not replace Zcash settlement. It accepts funded value, executes several
state transitions, gives users a Zcash path around sequencer censorship, and
anchors the resulting state back to Zcash in a batch.

## The seven beats

Keep two wallets visible throughout. A viewer should always be able to see both
balances, the current owner, the reserve floor and the latest state transition.

| # | Beat | Layer | What the viewer sees |
|---|------|-------|----------------------|
| 1 | Wallet A sends 0.01 ZEC to its vault deposit address | **mainnet** | ZIP-321 request, explorer txid, then the ZEC.zy balance increases |
| 2 | Wallet A pays 0.005 ZEC to claim one piece | **mainnet** | Second explorer txid, then the selected piece moves into Wallet A |
| 3 | Wallet A sells the piece to Wallet B | Zyn | Owner and balances change immediately; the fee enters the reserve and the floor rises |
| 4 | Wallet B tries to sell it back; the sequencer refuses; Wallet B forces the same sale through an encrypted `ZYF` memo | **mainnet + Zyn** | Visible rejection, memo txid, replica acceptance, then ownership returns to Wallet A |
| 5 | Wallet A redeems the piece | Zyn | The piece is burned, the reserve and outstanding count update, and Wallet A receives the displayed floor |
| 6 | One `ZYA` anchor covers beats 3–5 | **mainnet** | Settlement strip links the included Zyn transitions to one explorer txid |
| 7 | Wallet A withdraws the redeemed value to shielded ZEC | **mainnet** | ZEC.zy decreases and the shielded wallet receives the withdrawal |

Beat 4 is the centrepiece. Use `ZYN_IGNORE_FORCED=1` to simulate a sequencer that
refuses the return sale. Label this state clearly as a demo fault injection. The
sale submitted through the shielded Zcash memo must be the same sale the
sequencer rejected, so the recovery is obvious rather than a separate technical
example.

The flow touches mainnet at beats 1, 2, 4, 6 and 7. More importantly, each touch
has a distinct purpose: fund, claim, force, anchor and exit.

## Why the mint has two deposits

`mint_collection_item` in `swapvm/src/vm.rs:1000` charges nothing. It is
creator-only and debits no buyer balance. The 0.005 price does not exist in the
VM; it is a sale-backend concept.

For the hackathon flow, the claim payment is a second mainnet deposit. This keeps
primary-sale proceeds in the vault as auditable ZEC and gives the sale backend a
confirmed payment to bind to the claim. Do not justify the second deposit as a
way to satisfy the hackathon requirement; forced inclusion and anchoring already
demonstrate meaningful mainnet use.

The pitch therefore does not claim “one mainnet transaction total.” It claims
that funded users can perform several trades on Zyn and verify their combined
result in one Zcash anchor. A later buyer-initiated mint could debit an existing
ZEC.zy balance, but that requires a VM-level paid-claim path and is outside this
demo decision.

## Economics to show on camera

Seed the reserve with `fund_collection` before recording. The pool otherwise
starts empty, and trade fees alone will produce numbers too small to read at the
demo’s transaction sizes.

Keep this accounting visible:

1. Opening reserve, outstanding supply and floor (`pool / outstanding`)
2. Fee from the Wallet A → Wallet B sale and its reserve/creator split
3. Fee from the forced Wallet B → Wallet A sale and its split
4. Updated reserve and floor
5. Redemption amount, followed by the shielded withdrawal

Say that trading increased the floor from the displayed opening value to the
displayed closing value. Do not claim that redemption pays more than all deposits
or total spending; the reserve is deliberately seeded, so that comparison would
hide where the value came from.

## What exists, and what has to be built

Verified in the tree on 9 September 2026.

**Built — VM** (`swapvm/src/vm.rs`, `state.rs`, `microchain.rs`)

- `Collection { creator, symbol, cap, minted, outstanding, pool, fee_bps, phase }`
- `Phase: Depositing → Minting → Closed → Live`, forward only
- `redeem_price() = pool / outstanding`, zero unless `Live`
- `create_collection`, `mint_collection_item`, `fund_collection`,
  `redeem_collection_item`, `advance_collection`
- Trade fee in `accept_offer`: each side pays `fee_bps`; the current split is
  `50%` to ZYN POL, `40%` to the collection floor reserve and `10%` to the
  collection creator (protocol Treasury 26460 for Cave/Nap); one leg must be xZEC or
  `Reject::ItemNeedsAPrice`
- Authorities: redeem → holder, fund → source account, remaining collection
  operations → creator

**Built — daemon** (`zynzapd`)

- `OP_DEPOSIT_ADDRESS` (19), `OP_COLLECTIONS` (20)
- Client methods for all five collection intents, plus `collections`,
  `collection`, `is_item`, `phase_name`, `remaining`, `deposit_address`
- `/api/deposit-address`, `/api/deposit`, `/api/redeem`, `/api/withdraw`,
  `/api/force`, `/api/swap`, `/api/quote`, `/api/transfer`
- Per-account deposit addresses: `deposit_index(account)` = SHA-256 of
  `"zyn.deposit.index.v1" ‖ account`, 11 bytes, never the vault’s own index
- Anchors: `ZYN_ANCHOR_EVERY` = 30 epochs, `ZYN_ANCHOR_AFTER_SECS` = 1800

**Built — art** (`examples/zyn-pfp-concepts`)

- 4,444 images, 192×192 PNG, approximately 35 KB each and 154 MB total
- Mole 2,222 / Owl 1,333 / Moth 889
- All three collections validate `status: pass`, with unique image hashes,
  per-token traits and the `zyn-light-v1` viewer contract

**Missing — build in this order**

1. **Reachable buy/sell path.** `AcceptOffer` appears only in a test at
   `zynzapd/src/rpc.rs:1247`. Add the client method, HTTP endpoint, validation and
   an end-to-end test. Beats 3 and 4 depend on it.
2. **Paid public claim path.** `mint_collection_item` rejects anyone except the
   creator. The sale backend must bind a confirmed claim deposit to one buyer and
   one unminted item, then invoke the creator-authorized mint exactly once.
3. **Deterministic demo reset and seed.** One command should create the
   collection, fund the reserve, assign Wallet A and Wallet B, select stable item
   IDs and return the system to the opening state after every rehearsal.
4. **Forced-inclusion observability.** Surface the rejected operation, `ZYF`
   transaction, replica acceptance and resulting state transition in one event
   trail.
5. **Anchor control and evidence.** Add a guarded demo command to anchor now, and
   expose the included epoch/state root and `ZYA` txid through the overview data.
6. **Demo cockpit UI.** Build the focused filming surface described below.
7. **Decision record.** Add the collection and demo decisions after §38 in
   `DECISIONS.md`.

## Demo cockpit scope

Build a standalone app in `apps/zyn-settlement-lab`, separate from `zynzapd`
and the Cave landing page and marketplace. Keep it in the same repository for
review, with its own development server and production build. Live integration
should reuse daemon RPC, deposit endpoints, collection reads and authentication
through a narrowly scoped adapter rather than bundling the UI into the daemon.

The initial frontend implements the complete sequence in explicitly labeled
rehearsal mode, with local simulated balances and receipts. Its optional daemon
bridge is read-only (overview and account-specific deposit address). Claim,
trade, forced inclusion, anchor and withdrawal execution still need live
integration before this interface can be used for the mainnet recording.

The page is a filming cockpit rather than a general marketplace:

1. **Persistent wallet rail:** Wallet A and Wallet B balances, current roles and
   the selected piece’s owner, visible at the same time
2. **Selected-piece stage:** artwork, token identifier, price, reserve floor and
   available actions
3. **Fund and claim panel:** ZIP-321 URI, QR code, confirmation state and ZEC.zy
   balance update
4. **Action trail:** normal sale, sequencer rejection, forced memo acceptance,
   redemption and withdrawal in order
5. **Reserve panel:** pool, outstanding count, floor and fee contribution, with
   before/after values during trades
6. **Settlement strip:** `N state transitions · anchored to Zcash in tx…`, linked
   to the explorer and expandable to show the included epoch or state root

Use only the Shielded Byte card, with no selectable gallery. Follow the same
piece throughout the seven beats. Use the supplied gold Zyn symbol and wordmark
for the app branding. Other seeded pieces are accounting fixtures, not a
catalogue to display.

Do not hide Wallet A and Wallet B behind an account selector. The ownership
change is much easier to understand when both sides remain visible.

## Build and rehearsal order

The application can be developed against chain 11 while the Windows Zebra node
syncs. Mainnet is needed for rehearsals and the final recording, not for routine
UI work.

- **9–12 Sep:** buy/sell client and endpoint, validation, paid claim path
- **12–15 Sep:** deterministic reset/seed, forced-inclusion event trail, manual
  demo anchor
- **15–18 Sep:** cockpit UI and the seven complete flows on testnet
- **18–24 Sep:** mainnet rehearsal, seed the mainnet collection, record clean
  takes
- **24–30 Sep:** edit the video, write documentation and open the ZecHub PR
- **2 Oct, 12:00 UTC:** DAO vote

Do not wait for mainnet sync before building the storefront or transaction
flows. Treat the sync and application work as parallel tracks.

## Filming notes

- Open with the one-sentence claim and the settlement strip; establish the proof
  before introducing the collection.
- Keep both wallet balances, current owner, reserve and floor on screen.
- Show each meaningful mainnet txid on the explorer. Cut across confirmation
  time, then return to the confirmed state.
- Label `ZYN_IGNORE_FORCED=1` as a deliberate demo fault injection before the
  sequencer rejects the return sale.
- Keep the rejected sale and accepted forced sale adjacent in the action trail.
- Show the floor before and after each fee-producing trade.
- Expand the anchor strip long enough to show which transitions its state root
  covers.
- Complete the withdrawal to shielded ZEC. It proves users can exit the system.
- Keep real amounts in the 0.001–0.01 ZEC range.
- Aim for a two-and-a-half to three-and-a-half minute final video.

## Custody caveat for the documentation

The Windows box holds **two of the three signer keys** and one of three vault
FROST shares. It can certify alone. This is acceptable for a demonstration using
dust amounts and is not an acceptable production custody configuration. State
that limitation directly in the submission, together with the intended
production separation of signers.
