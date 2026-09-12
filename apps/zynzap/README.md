# ZynZap web

The standalone, keyless front for ZynZap. Its first live path is the Zcash
mainnet bridge: ZEC ↔ ZEC.zy on Zyn chain 26460.

The hosted page never calls the Zyn node or the local wallet HTTP API. It uses
the approval-gated `window.zyn` provider injected by the Nap Wallet extension.
Nap Wallet remains responsible for keys, network selection, transaction
construction, signing and submission. The hosted app refuses to enable when
Nap reports anything other than Zcash mainnet and Zyn chain 26460.

When the same file is served by Nap Wallet at `http://127.0.0.1:8977/zynzap`,
it retains the documented direct same-origin development path and may be used
with the wallet's configured network.

## Run and verify

```sh
npm test
npm start
```

Open <http://127.0.0.1:4179>. The static build is in `dist/` and can be
deployed directly to Cloudflare Pages. Fonts are copied from the audited,
self-hosted Nap assets; the page makes no third-party requests.

## Production boundary

- Expected wallet network: `mainnet`
- Expected Zyn chain: `26460`
- Signing transport: Nap Wallet extension only
- Public Zyn RPC: not required and not contacted
- Website-held keys: none
