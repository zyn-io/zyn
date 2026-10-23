# ZEC Pay: accept shielded ZEC on a website

Design, 23 Oct 2026. Status: for review, not built.

## Goal

A web developer can accept shielded ZEC with one small browser script and one
server package, and know for certain when each order is paid, without running a
Zcash node or a wallet and without handing anyone a spending key.

Who it is for: developers adding ZEC payments to a shop, a paywall, a donation
page or an app. Not for wallet builders.

What success looks like:
- the "accept ZEC" quick start is under 20 lines of code on each side;
- any shielded Zcash wallet can pay (Zashi/Zodl, Ywallet, Zingo, Noir, Nap);
- the merchant's ZEC lands in the merchant's own wallet, never with us;
- a working example on onboard.cavebro.win that anyone can pay.

## Why this, and not a wallet SDK

Noir Wallet (RHEA team) already has a typed dApp SDK, a multi-wallet adapter
and a WalletConnect transport; racing them on "connect a wallet" is their home
ground. Nobody covers the merchant's half lightly: BTCPay's Zcash plugin needs a
whole BTCPay install and a node; Noir's SDK is payer-side only and requires
Noir. This package is wallet-neutral and covers the merchant side.

## Scope

**v1 (this spec)**
1. `@zyn-io/zec-pay/watch` — merchant side: invoices with one address each,
   payment detection with a viewing key, confirmations, status, webhooks.
2. `@zyn-io/zec-pay/pay` — buyer side: ZIP-321 link and QR, optional one-click
   buttons for Nap and Noir, live status.
3. `zec-pay-core` — a small Rust crate compiled to wasm that does the
   cryptography for `watch`.
4. A "Build" page on onboard.cavebro.win with the quick start and a live
   example.
5. `zec-pay serve` — the same engine as a small HTTP service for one merchant
   (REST API + webhooks), for stacks that are not Node.

Built gateway-ready from the start (see "Ready for payment gateways"): every
record carries a merchant id, one scan serves many viewing keys, the API and
events are versioned, storage is an interface.

**v1.x (gateway features, on the same interfaces)**
- Many merchants per `serve` process, each with its own API key and webhook.
- A Postgres store.
- Adapters for existing shop plugins (see below).

**v2 (second milestone, same package)**
- Settle in USDC: the buyer pays ZEC, NEAR's 1Click swaps it and delivers USDC
  to the merchant's address, with our app fee on the quote.

**Not in scope**
- Transparent-address payments (exchange withdrawals). Shielded only.
- Unconfirmed (mempool) detection. A payment counts once it is in a block.
- Fiat conversion (a gateway passes its own quoted price through; see below),
  refunds, the fee-in-payment model.
- The public developer endpoints (separate piece; access likely gated by
  staking ZEC — to be designed later).
- Zakura PIR.

## How it works

```
 buyer's browser                    merchant's server                block server
 ───────────────                    ─────────────────                ────────────
 pay.render(invoice)  ── poll ──▶   watch.handler  ◀── status ──┐
   QR / zcash: link                 watch.invoice(amount)       │
   [Nap] [Noir] buttons             watch scanner ── BLOCKS ──▶ zyn-lightd HTTP door
        │                                (viewing key only)       (zyn-rpc.cavebro.win/zec)
        ▼
 buyer's wallet ── shielded tx ──▶ Zcash ──────────────────────────▶ (compact blocks)
                                        funds land in the merchant's own wallet
```

### Invoices: one address per order

The merchant configures `watch` with a **unified full viewing key** (UFVK) and
the height it starts from. Every Zcash wallet that can export a UFVK works;
the merchant keeps spending in that wallet as usual.

`invoice({ amount, expiresIn, metadata })` returns:
- `id`
- `address` — a unified address with one Orchard receiver at a fresh
  diversifier index, the same mechanism the Zyn vault uses for per-account
  deposit addresses (`VaultKeys::deposit_address`);
- `uri` — `zcash:<address>?amount=<ZEC>` (ZIP-321), plus `message` when given;
- `amountZat`, `expiresAt`, `status`.

Indices start at `2^40 + n` (n = invoice counter, persisted), far above the
indices a wallet hands out for itself, so a payment to the merchant's ordinary
wallet address is never mistaken for an invoice payment.

Because the address identifies the order, the buyer needs **no memo** and the
scanner fetches **no full transaction**: compact blocks are enough. The block
server learns that someone is reading blocks from height H onwards, and nothing
else (DECISIONS §24).

The merchant's normal wallet sees these payments too: diversified addresses are
all the same key. Funds never pass through us.

### Detection

`zec-pay-core` (wasm) holds the viewing key and exposes:
- `Watcher::new(ufvk, network)` — decode the UFVK's Orchard item; Ironwood
  uses the same key (one viewing key, two pools since NU6.3);
- `addressAt(index)`;
- `scan(blocksReply)` → hits `[{ height, blockHash, txid, pool, index, valueZat }]`,
  where `index` is the diversifier index recovered from the decrypted note's
  recipient (`IncomingViewingKey::diversifier_index`);
- request builders for INFO and BLOCKS (`zyn_custody::lightd::request`).

It reuses `zyn_custody::compact` and `VaultKeys`; the only change there is a
scan that keeps the recipient address the compact decryption already returns
(today `compact::scan` drops it). No proving keys, no spending code: the wasm
stays small.

`watch` (JavaScript, runs in Node ≥ 20 and in the browser):
1. every `pollSeconds` (default 20): INFO → tip; BLOCKS from the next height
   in batches;
2. pass blocks to `scan`; keep only hits whose index belongs to an open
   invoice;
3. per invoice, sum the hits; status follows:

| status | when |
|---|---|
| `open` | no payment seen |
| `seen` | payments in blocks, total ≥ amount, fewer than N confirmations |
| `partial` | payments in blocks, total < amount |
| `paid` | total ≥ amount with N confirmations |
| `overpaid` | as `paid`, total > amount (reported, not refunded) |
| `expired` | past `expiresAt` with nothing seen |
| `late` | a payment arrived after `expiresAt`; reported for the merchant to decide |

4. on each status change: call `onStatus(invoice)` and POST the merchant's
   webhook (HMAC-SHA256 signed body, retried with backoff).

**Confirmations.** N is stated in blocks per network upgrade: 3 before NU7, 9
after it (25-second blocks, ZIP 218), taken from the activation height
(mainnet 3,543,405; testnet 4,465,026), so it changes on its own at NU7.
Merchants can override.

**Reorgs.** Each poll re-reads the block hash at the last scanned height. On a
mismatch, rewind 20 blocks, drop hits above the rewind point and rescan.
`paid` is only set at depth N, so a reorg shallower than N can move an invoice
back from `seen` but not from `paid`. A deeper reorg is logged loudly and the
invoice re-evaluated; on Zcash that is rare.

**Store.** A small interface (`get`, `put`, `list open`, `cursor`) with two
implementations: SQLite (Node, default) and in-memory (browser, tests).

### Buyer side

`pay.render(element, { invoiceUrl })`:
- fetches the invoice from the merchant's endpoint, draws the QR of `uri` and
  the address with a copy button;
- shows one-click buttons when a wallet is present:
  - Nap: Nap Connect `zec_pay { uri }`;
  - Noir: `window.noirwallet.zcash.sendTransaction(...)` with the address and
    amount (field names per Noir's SDK at build time);
- polls the invoice and shows the status in plain labels ("Waiting for
  payment", "Payment seen — confirming", "Paid").

`watch.handler()` gives the merchant a request handler (Node `http` signature,
usable from Express, Hono, Fastify) that serves `GET /invoice/:id` with the
public fields only — never the viewing key, never other invoices.

### Block source

Default: our block server's HTTP door, `https://zyn-rpc.cavebro.win/zec`
(mainnet) and `/zec-testnet`, rate-limited at 20 req/s per IP. A merchant can
point `watch` at their own `zyn-lightd` (public repo `zyn-io/zyn`). The source
is an interface, so a standard lightwalletd adapter can be added later.

## Ready for payment gateways

A gateway (ours later as the hosted watcher, or someone else's) runs the same
engine for many merchants. v1 does not ship gateway features, but nothing in
it has to be rewritten to add them:

- **Merchants as a first-class id.** Invoices, cursors, events and webhook
  settings are keyed by `merchantId`. The library's single-merchant mode uses
  one fixed id.
- **One scan, many keys.** `zec-pay-core` holds a set of viewing keys; each
  block is fetched once and trial-decrypted with every key. A merchant added
  later gets its own catch-up cursor from its starting height, then joins the
  shared scan at the tip. Cost grows with the number of keys per block; the
  plan measures how many keys one core keeps up with.
- **A stable API.** REST under `/v1`:
  - `POST /v1/invoices` (with an `Idempotency-Key` header);
  - `GET /v1/invoices/:id`;
  - `GET /v1/invoices?status=&since=`;
  - `POST /v1/invoices/:id/cancel`.
  The JSON is the library's invoice object; the library and the service never
  drift.
- **Versioned events.** `invoice.created`, `invoice.seen`, `invoice.partial`,
  `invoice.paid`, `invoice.overpaid`, `invoice.expired`, `invoice.late`,
  `invoice.cancelled`. Each has an event id (delivered at least once; receivers
  dedupe by id), a timestamp and an `apiVersion`. The signature header carries
  a timestamp and an HMAC over `timestamp.body`, so receivers can refuse
  replays.
- **Pass-through fields.** `orderId`, `metadata` (string map) and an optional
  `price { currency, amount }`: the gateway's own fiat quote, stored and
  returned, never computed by us.
- **Pluggable parts.** Store (SQLite, memory; Postgres in v1.x), block source
  (zyn-lightd; lightwalletd later), clock (for tests).
- **Plugin adapters, later.** Existing shop plugins (WooCommerce, Shopify
  apps) mostly speak BTCPay's Greenfield API. A Greenfield-compatible subset
  in v1.x would let those plugins use ZEC Pay unchanged. Which subset they
  actually call is to be checked before committing to it.

## Disclosures (shown inline in the docs, not behind an "i")

- The merchant's server holds a viewing key: it can see every payment to that
  wallet, and whoever gets that server can too. It cannot spend.
- By default the block server is ours. It sees which block ranges are read and
  from what IP, not which payments are yours.
- Payments count only once they are in a block; there is no instant
  confirmation.
- Shielded only: buyers paying from an exchange (transparent address) need to
  pay from a shielded wallet.

## The Build page and example

`onboard.cavebro.win/build` (in `apps/zec-onboard`), general Zcash like the
rest of the guide:
- the quick start, both halves;
- a live tip jar on mainnet: an invoice for 0.001 ZEC, scanned in the browser
  with a dedicated demo viewing key. The page says plainly that payments to
  this demo are visible to anyone who reads the page source.
- the disclosures above, and the list of wallets that can pay.

## Layout

```
apps/zec-pay/
  core/            Rust crate zec-pay-core → wasm (wasm-pack, targets nodejs + web)
  package/         @zyn-io/zec-pay: watch.js, pay.js, serve.js (+ `zec-pay serve` bin),
                   store-sqlite.js, store-memory.js, the built wasm, README
  tests/
apps/zec-onboard/public/build/   the Build page and example
zyn-custody/src/compact.rs       scan that keeps the recipient (small change)
```

Licence MIT, like Nap Connect.

## Error handling

- Block server down or rate-limited: keep the cursor, back off (5 s → 5 min),
  report `stalled` through `onError`; never mark anything paid on missing data.
- Server tip behind the chain (`estimated > tip + 20`): wait; say "block
  server syncing".
- UFVK without an Orchard item: refuse at start with a plain message.
- Network mismatch (mainnet key, testnet server): refuse at start — INFO
  carries the network byte.
- Webhook failures: retry up to 24 h, then mark `webhook_failed`; status is
  still readable by polling.

## Testing

- `zec-pay-core` unit tests: UFVK decode, `addressAt`, index recovery from a
  built note in both pools, scan of fixture compact blocks.
- `watch` tests with a fake block source: partial, paid, overpaid, expired,
  late, reorg rewind, the NU7 change of N, webhook signing and retry.
- Two merchants in one scan: each sees only its own payments; a merchant added
  mid-chain catches up and joins.
- `serve` API tests: idempotent create, status reads, event ids stable across
  retries, signature check.
- A throughput measurement: viewing keys per core at mainnet block sizes.
- One live test on mainnet, run by hand: pay the demo invoice from Nap and from
  one other wallet (Zashi/Zodl), watch it reach `paid`.
- The Build page's e2e in `apps/zec-onboard/tests` follows the existing
  pattern.

## v2: settle in USDC (outline)

- `invoice({ amount, settle: { asset: 'USDC', chain, address } })` asks 1Click
  for a quote with our app fee; the invoice's address is the 1Click deposit
  address and its `uri` pays it.
- Status comes from 1Click's quote status instead of the scanner.
- Disclosed inline: the 1Click deposit address is transparent, so this path is
  public on Zcash; shielded ZEC to the merchant stays the private default.
- Fee: an app fee above 1Click's 20 bps minimum, split 50/50 with 1Click
  (docs.near-intents.org/resources/fees). Our share goes to the team treasury.

## Open points

- npm scope: `@zyn-io` — check we own it before publishing.
- Which wallets export a UFVK today (Ywallet, Zingo, Zashi/Zodl, Nap). If Nap
  cannot, add "Export viewing key" to Nap as part of this work.
- CORS: the block server's HTTP door must allow `onboard.cavebro.win` for the
  in-browser example; merchants' servers are not browsers and need nothing.
- Demo viewing key: a fresh wallet made for the tip jar; its UFVK is public by
  design, its phrase is kept like other team keys.
- After the NU7 crate switch, `zec-pay-core` builds on the same Zcash crate
  versions as Nap's core.
