# Cave Market

The existing Cave storefront for the `cave-market` Cloudflare Pages project.
The established landing page remains the default view. A conventional market
view is available at `#market`, and a private owner portfolio at `#portfolio`.
The creator launch configurator is available at `#launch`.

## Local development

Run `npm start`, then open:

- http://127.0.0.1:4178/ — existing Collection Z page
- http://127.0.0.1:4178/#market — marketplace view
- http://127.0.0.1:4178/#launch — local launch policy configurator
- http://127.0.0.1:4178/#portfolio — owner gallery

No package installation or backend is required. Run `npm test` for storefront
checks.

## Build

Run `npm run build`. Static output is `dist/`, ready for the existing
`cave-market` Pages project. Building does not deploy or change any domains.

`npm run build:launchpad` emits the separate Cave Jade product to `dist/` for
the `launch.cavebro.win` Pages project. Its root is the live coin gallery,
`/launch` is the creation workbench, and `/coin/<asset-address>` is the market
page. The hosted product reads and writes only through the approval-gated
`window.zyn` provider injected by Nap Wallet; it makes no page-originated RPC
requests. The obsolete hidden “Cave Signal” prototype has been removed from the
NFT app.

The standalone artifact is deployed at `https://cave-jade.pages.dev` under the
`cave-jade` Cloudflare Pages project. `launch.cavebro.win` is registered on that
project; final DNS/hostname activation is intentionally left to the operator.

The creation workbench mirrors the frozen v1 envelope: one billion tokens with
eight decimals, an exact Unicode symbol of at most 32 UTF-8 bytes, a 0.5–3%
immutable trading fee, and an optional dev buy capped at 50 million tokens.
Launch, exact curve quote, buy and sell requests are implemented. Every write
requires a separate Nap Wallet approval.

## Market behavior

The market view uses the same Cave background, pixel typography, colors and
navigation as the existing site. It presents familiar search, listing tabs,
sorting, filters, market metrics and wallet placement. Its secondary-market
disclosure mirrors consensus: each side pays the collection fee in ZEC.zy;
`50%` goes to future ZYN POL, `40%` to the collection redemption floor and
`10%` to the creator, with atomic remainder to POL. Treasury 26460 is the
creator recipient for Cave and Nap Mole.

Collection Z is still pre-mint, so the catalog reports zero listings and does
not reveal artwork, invent prices, connect a wallet or accept transactions. The
controls are intentionally disabled until real inventory and backend routes are
available.

Portfolio replaces the redundant Release Status tab. It provides the expected
profile statistics, Collected/Created/Activity tabs, gallery controls, and an
honest disconnected state without showing fabricated owned items. Wallet buttons
open an integration-status dialog; they do not request keys or addresses.

The launch view supports fixed receipts, creator-weighted quantity draws and
buyer-selected quantities, independently from object identity assignment. Its
10,000-entry preview uses browser randomness for illustration only. Publishing
is disabled: the app makes no policy, wallet, deposit or allocation request until
the backend commitment service described in `DECISIONS.md` §64 exists.
