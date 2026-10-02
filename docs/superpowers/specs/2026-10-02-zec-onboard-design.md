# ZEC onboarding site + Nap domain move — design

Date: 2 Oct 2026. Approved in chat ("go").

## Why

ZECATHON Wildcard track: $5,000 in ZEC for the best onboarding video series or web
experience — wallet setup, getting ZEC, shielding & unshielding, sending & receiving;
zero to the first shielded transaction. Submission is a tweet tagging @zksnarks_.
Deadline Sunday 4 Oct 2026.

The site is about ZEC. Nap Wallet is the only thing of ours it names — no Zyn, Lend,
bridges, Cave.

## 1. The guide — onboard.cavebro.win

- Static, no build: `apps/zec-onboard/public`, Cloudflare Pages project `zec-onboard`.
- Title "Zcash onboarding"; hero one sentence: "From zero to your first shielded ZEC transaction."
- Six steps, each with a tick saved in localStorage (wrapped in try/catch):
  1. Set up a wallet (Nap web or extension, 24 words, backup warning inline)
  2. Get ZEC (exchange that accepts a `u1…` withdrawal — verified before naming; or testnet TAZ from a faucet)
  3. Shield (exchange withdrawal to the Nap `u1…` address is the transparent → shielded move)
  4. Receive (address, QR, memo)
  5. Send shielded (`u1 → u1`, memo, ≈30 s proving, fee)
  6. Unshield (send to a `t1…` address; public-visibility disclosure inline)
- Tools in the page: address checker (t1/t3/u1/zs/testnet; can Nap send to it), a
  "what's visible" table (t→t, t→z, z→z, z→t), txid → public explorer link.
- Real Nap screenshots from a testnet wallet.
- Plain labels, explanations behind "i" bubbles (house copy rule).

**Decided:** Nap's browser build has no transparent receive, so no Shield button; the
guide uses what Nap does today (option a). Nap only, no other wallets listed.

## 2. Nap moves to nap.zyn-network.com

- Add `nap.zyn-network.com` as a custom domain on Pages project `nap-web`.
- `nap.cavebro.win` stays as an alias, no forced redirect: the 0.3.1 extension zips and
  the TestFlight build call it, and browser wallets are stored per origin. The old origin
  shows a banner pointing at the new one ("restore with your phrase there").
- Repoint `NAP_WEB` (nap-connect, cave-market, zyn-lend), extension host permission,
  `VITE_NAP_API` (`.env.extension`, `.env.mobile`), links in zyn-site, zynscan,
  cave-collection-z, privacy URL in listings, docs.
- Keep the RDNS ids `win.cavebro.nap.*` (protocol identifiers).
- Rebuild extension 0.3.2; redeploy nap-web, cave-market, zyn-lend, zyn-site, zynscan;
  run cave e2e + extension tests against the new domain. Mobile: env changed, no upload.

## User actions

- DNS (the wrangler token is zone:read only): proxied CNAMEs
  `nap.zyn-network.com → nap-web.pages.dev`, `onboard.cavebro.win → zec-onboard.pages.dev`.
- Tweet the submission.

## Testing

Playwright: guide at 1280 and 390 widths, no horizontal scroll, ticks persist, address
checker cases. Nap move: e2e suites against nap.zyn-network.com.
