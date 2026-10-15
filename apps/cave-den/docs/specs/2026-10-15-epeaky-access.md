# Access with $EPEAKY

15 October 2026. Approved in chat: a third way into the test den (epeaky-200)
beside invite links and requests. A visitor connects a Solana wallet, buys
EPEAKY with one of three fixed SOL amounts on our page, and a successful buy
lets their device in.

## What exists

The den already has the `external` way in (`Den.Gate.Members.register/1`,
`POST /door/passes` with `DEN_ISSUER_TOKEN`). The den's page, when
`config.json` names an `issuer`, shows an Enter button that opens the issuer
in a popup with `#den-pass=<hash>` (the device makes the code and keeps it),
and waits for `postMessage({den: 'pass', registered: true})` from that
window's origin (`web/gate.js`). Nothing in that flow changes.

## The token, as checked on 15 Oct

- Mint `6dkcEZs83NpCJmWveasCMu3Kmd2XkCzSVmNJJhQBLtx1`, Token-2022, 6 decimals,
  1,000,000,000 supply, launched on stonk.fun / Raydium LaunchLab.
- **3% transfer fee** (transferFeeConfig 300 bps). Its fee authority can
  change it; the page reads the current fee from the mint, not a constant.
- Small: about $436 liquidity, 11 holders. Jupiter routes SOL → EPEAKY
  (0.1 SOL ≈ 1% price impact).

## Parts

### 1. Core: the issuer from the environment (public, zyn-io/epeaky)

Today a host sets `issuer` only by building its own page. Add two settings so
the stock image can use an issuer:

- `DEN_ISSUER`: the issuer page's https URL.
- `DEN_ISSUER_LABEL`: the Enter button's text (default "Enter").

The den serves `/config.json` as the built file with these two keys laid over
it (`issuer`, `issuerLabel`) when set; `app.js` uses `issuerLabel` for the
button. README's settings table and "Who gets in" gain one line each. Server
test: `/config.json` carries the env issuer; page test: the label shows.

### 2. The access page (ours, not in the public repo)

`apps/epeaky-access`: a Cloudflare Pages project `epeaky-access`, served at
`epeaky-access.pages.dev` (a custom name later, when DNS is set). Static page
+ two Pages Functions. No framework, no web3.js: Wallet Standard
(`solana:signAndSendTransaction` and `solana:signMessage` take raw bytes),
a base58 helper, and fetch.

Page, top to bottom (plain labels, no slogans):

- Title "Epeaky access". One line: buying lets this device into the den.
- **Connect wallet**: lists Wallet Standard wallets found (Phantom,
  Solflare, Backpack…); none found → "No Solana wallet found in this
  browser."
- Three buttons: **Buy with 0.1 SOL**, **Buy with 0.25 SOL**,
  **Buy with 0.5 SOL**. Each shows the quoted EPEAKY amount and price impact
  (from Jupiter's quote) before the click. Any of them gives access.
- Shown inline, before buying (the risk a buyer must see):
  - This is a market buy through Jupiter, not a payment to us; no refunds.
  - The token takes a 3% fee on every transfer (read from the mint); you
    receive less than the swap shows.
  - Liquidity is small, so the price can move a lot.
  - This page's server sees your wallet address and which device it let in.
    The den does not see your wallet.
- After the buy: "Checking the buy…", then "You're in." and the window
  closes; or the reason it failed and a way to retry.

Flow:

1. Page reads `den-pass` from the hash (64 hex) and refuses without it
   ("Open this from the den's page.").
2. Connect → the wallet's address.
3. Click → `GET /api/quote?sol=0.1` (the function calls Jupiter's quote with
   the fixed amount; the client never chooses the amount) → `POST /api/swap`
   returns Jupiter's unsigned transaction (base64) for that wallet.
4. Wallet signs and sends (`signAndSendTransaction`) → signature.
5. Wallet signs a message: `Epeaky access\npass: <pass hash>\ntx: <signature>`.
   This binds the buy to this device: anyone can see a buy on chain, but only
   the buyer's wallet can sign for it.
6. `POST /api/claim {pass, signature, message signature, wallet}`.
7. On `{ok}`: `opener.postMessage({den: 'pass', registered: true}, DEN_ORIGIN)`
   and close. Cancel or close → `{den: 'pass', error: true, cancelled: true}`.

### 3. The claim check (Pages Function `/api/claim`)

Registers the pass only if every one holds:

- The message signature verifies (Ed25519, WebCrypto) for `wallet` over the
  exact message with this `pass` and `signature`.
- `getTransaction(signature, finalized … else confirmed)` exists, has no
  error, and its block time is under 15 minutes old.
- `wallet` is the fee payer (first signer).
- The wallet's SOL went down by at least 0.1 SOL (pre − post lamports, so
  fees only add to it).
- The wallet's EPEAKY balance (any token account it owns for the mint, from
  pre/post token balances) went up.
- The signature is new: D1 table `claims(signature PRIMARY KEY, pass, wallet,
  at)`; the insert's unique key settles races. A retry with the same
  signature and the same pass is allowed (the den call may have failed); a
  different pass is refused ("This buy already let a device in.").

Then `POST {DEN_URL}/door/passes {pass}` with `DEN_ISSUER_TOKEN`; the den's
answer is passed on. Errors are plain: "The buy was not found yet. Wait a few
seconds and press Check again.", "The buy was under 0.1 SOL.", "This wallet
did not sign the buy."

Function settings (Pages secrets / vars): `DEN_URL`, `DEN_ISSUER_TOKEN`,
`SOLANA_RPC` (default the public mainnet RPC), `MINT`, `AMOUNTS`
(`0.1,0.25,0.5`), D1 binding `CLAIMS`.

### 4. The test den

On epeaky-200: `DEN_ENTRY=invite,approval,external`,
`DEN_ISSUER=https://epeaky-access.pages.dev/`,
`DEN_ISSUER_LABEL=Get access with $EPEAKY`, and a fresh
`DEN_ISSUER_TOKEN` (shared only with the function). Needs the core change
released (CI image) and `epeaky update`.

## What it does not protect

- The access server links wallet ↔ pass. The den's blind day tokens keep pass
  ↔ device apart, but the same people run both, so the page says so.
- A banned device's owner can buy again; each new pass costs a buy.
- A buy then an immediate sell still counts.
- The public RPC can rate-limit; a paid RPC URL can be set later.

## Tests

- Core: server test for `/config.json` with `DEN_ISSUER`; page e2e (den.e2e)
  shows the label when set.
- Access: unit tests for the claim check against recorded `getTransaction`
  answers (a real buy, too small, wrong payer, failed tx, old tx, reused
  signature, bad message signature) and the base58/Ed25519 helpers.
- End to end on mainnet, by hand with a small real buy (0.1 SOL) from the
  user's wallet, once deployed: the device enters the test den.
