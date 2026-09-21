# Zyn Settlement Lab

Standalone frontend for the ZecHub hackathon flow. Independent of Cave Market,
the public landing page and the daemon's bundled UI. No deployment is required.

## Run

```sh
npm install
npm run dev
```

Open http://127.0.0.1:4177. Use `npm run build` for a static `dist/` bundle,
`npm run preview` to preview it, and `npm test` to verify rehearsal accounting,
the live state machine, the bridge allowlist, and timeout recovery.

## What works

- Standalone settlement demo only. Cave's store lives in `../cave-market`,
  independently of this application and its deployment.
- One Shielded Byte card from `examples/zcash-pixel-nft`, followed through the full run.
- User-supplied gold Zyn symbol and wordmark, with the symbol also used as favicon.
- Persistent Wallet A and B balances and current ownership.
- Ordered deposit, paid claim, sale, censored return sale, forced retry,
  redemption, batch anchor and shielded withdrawal.
- Manual progression or Play walkthrough / Pause.
- Exact zatoshi arithmetic, equal reserve/creator fee splits, floor chart.
- Inspectable simulated receipts, filterable trail and anchor coverage.
- Deterministic reset, local session restoration and responsive layout.
- Reduced-motion support, semantic buttons and keyboard-accessible dialogs.
- An explicit rehearsal/live mode boundary in the header.
- Live reads and guarded writes through three independent local daemons.
- Real `seq`, `epoch`, and `txid` receipts with scheduled-anchor read-back.

## Rehearsal boundary

All transaction buttons run a local model. SIM identifiers are deliberately not
hex txids; no block explorer or cryptographic proof is fabricated. No private keys
are requested, and no transaction API is called. Mainnet fees are excluded.

Seed: Wallet A has 0.02 external ZEC; Wallet B has 0.02 ZEC.zy; the reserve holds
0.03 ZEC with five pieces already minted. Claiming the selected sixth piece adds
0.005 ZEC to sale proceeds, not the reserve. Each 0.006 trade charges each side
1%, splitting the combined fee equally between creator and reserve. The floor
after claim is 0.005, then 0.00501 and 0.00502 after the two trades. Redemption
pays 0.00502 and burns the selected piece. Withdrawal returns that amount to A.

## Live testnet mode

```sh
ZYN_DAEMON_CREATOR=http://127.0.0.1:8977 \
ZYN_DAEMON_A=http://127.0.0.1:8978 \
ZYN_DAEMON_B=http://127.0.0.1:8979 \
VITE_ZYN_COLLECTION_ID=<live-collection-id> \
npm run dev
```

The listed localhost ports are defaults, so only `VITE_ZYN_COLLECTION_ID` is
required when those ports are used. Click **Rehearsal** in the header and confirm
to enter live mode. Connection performs reads only. Each subsequent gold control
states the live intent it may submit.

The Vite bridge has separate `creator`, `a`, and `b` targets. It admits only the
documented four GET and nine POST routes; arbitrary roles, methods, paths, nested
paths, and oversized bodies are refused. It is deliberately a local demo bridge,
not a public backend.

A real ZIP-321 request and QR are produced only from Wallet A's daemon-issued
address or `deposit_uri`; a shared-vault memo is preserved. Amounts remain decimal
strings and all comparisons use integer fixed-point arithmetic. The frontend
never auto-retries a write. A timeout is labelled **unknown** and the next action
only reads all wallets back before deciding whether it landed.

The live path is: deposit check, buyer `/fund` + creator `/mint`, A `/offer` + B
`/take`, B `/force` returning the item to A, A `/redeem`, optional `/bind` +
`/withdraw`, then scheduled `/anchors` polling. The current daemon `/force` route
can force only a transfer, not an offer or take. The interface therefore labels
the beat **forced return transfer** and never implies a paid return sale.

`GET /api/anchors` is observational. The UI waits for a scheduled anchor covering
the largest returned receipt epoch and does not create or fabricate one. An
explorer link is intentionally omitted until a verified testnet explorer base is
configured; the complete txid remains inspectable in the receipt JSON.

The demo uses the existing Shielded Byte hero render from
`examples/zcash-pixel-nft/renders/hero.png`. It is a Zyn Privacy Relics card
concept, independent of Collection Z. Only one card is displayed; the five
other seeded pieces exist solely in the rehearsal accounting model.
The original supplied branding images are displayed using CSS framing without
altering the source artwork. Display lighting is not authoritative NFT metadata.
Fonts are local DM Sans and Syne, with their licenses in `public/assets/fonts`.
