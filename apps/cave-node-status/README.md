# Cave node status

The public face of the mainnet Zebra node: a page anyone can open, backed by a
node nobody is handed. Live at **https://cave-node-status.pages.dev**.

The original generated background is `assets/cave-node-chamber.png`; the page
ships its optimized `assets/cave-node-chamber.webp` derivative.

```sh
npm test
npm run build
npm start
# http://127.0.0.1:4182 — static only, preview record

npx wrangler pages dev --port 4183 --binding ZEC_RPC_URL=https://zec-rpc.cavebro.win/
# http://127.0.0.1:4183 — the real endpoint against the real node
```

## What the browser can reach

`GET /api/status` on the page's own origin, and nothing else. It answers with
six fields and no way to ask a seventh:

```json
{
  "network": "main",
  "height": 3487500,
  "estimatedHeight": 3487502,
  "tip": "000000000058484dc035149f002f4b56b4831c9a724191de454be6ded1f466cb",
  "sampledAt": "2026-09-18T08:38:59.002Z",
  "latencyMs": 473
}
```

On localhost the page shows a clearly marked, in-memory preview record. On any
other hostname it requests only `GET /api/status`; it never calls Zebra
JSON-RPC. The UI rejects wrong-network, malformed, or older-than-two-minute
evidence.

## How the node stays out of it

`functions/api/status.js` is a Pages Function — it runs on Cloudflare, not in
the browser. It reads the upstream from **`ZEC_RPC_URL`**, a project secret, so
the address is not in this repo, not in a built asset, and not in a response.

- The shape *is* the allowlist. `getblockchaininfo` returns chainwork, disk
  size, value pools and more; `shapeStatus` names six fields and the rest never
  leaves the Function.
- Errors say `node unreachable`, never what upstream said. An echoed error is
  how an address gets out.
- **No CORS header**, matching the page's `connect-src 'self'`. One would make
  this an open RPC proxy for anyone who wanted one.
- `POST` is refused, including a JSON-RPC body. There is no tunnel here.
- Readings are coalesced for 5s, so a crowd cannot spend the node's 8 r/s
  (DECISIONS §53.4) asking one question. Well inside the 120s the page treats
  as fresh.

This is not a claim that the RPC is secret — DECISIONS §53 made
`zec-rpc.cavebro.win` public on purpose, default-deny by method. What this
withholds is a working endpoint handed to every visitor of a public page, plus
the traffic that follows one.

`tests/status-endpoint.test.js` holds the line: it fails if a built asset ever
names a node again, if the upstream becomes a literal, or if the Function's
output stops satisfying the page's own validator.

This directory does not deploy or modify nginx. Zebra stays on loopback
(§53.1); the raw RPC is filtered there, separately.
