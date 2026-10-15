# Access with $EPEAKY Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A visitor without an invite connects a Solana wallet on our access page, buys EPEAKY with 0.1, 0.25 or 0.5 SOL, and that buy lets their device into the test den.

**Architecture:** The den's existing `external` way in is the door: the den's page opens the issuer popup with the device's pass hash, the issuer registers it (`POST /door/passes`, bearer `DEN_ISSUER_TOKEN`) and posts back. A one-change core addition lets the stock image name the issuer from the environment. The issuer is a Cloudflare Pages project (`apps/epeaky-access`, not public) whose functions quote and build the Jupiter swap and check the finished buy before registering.

**Tech Stack:** Elixir/Phoenix (core), plain ES modules (page, Wallet Standard), Cloudflare Pages Functions + D1, Jupiter lite API, Solana JSON-RPC, WebCrypto Ed25519, `node --test`.

**Spec:** apps/cave-den/docs/specs/2026-10-15-epeaky-access.md

## Global Constraints

- Mint `6dkcEZs83NpCJmWveasCMu3Kmd2XkCzSVmNJJhQBLtx1`, 6 decimals, Token-2022.
- Amounts exactly `0.1,0.25,0.5` SOL; the smallest is the minimum the claim check accepts.
- Plain product copy (no slogans; title "Epeaky access"); risk notes inline before buying; nothing says whose token it is.
- No web3.js or wallet-adapter bundle: Wallet Standard raw bytes, own base58.
- The access page is ours only: not in `scripts/den/sync.sh`'s tree (it lives in `apps/epeaky-access`).
- Core change is generic (no token, no Solana).

## Review Focus

- A buy someone else made (seen on chain) submitted with a different pass → refused (message signature binds pass + tx to the payer).
- The same buy claimed twice with two passes → second refused; same pass retried after a den failure → allowed.
- The transaction not yet visible at `confirmed` right after sending → "not found yet", page retries a few times before showing Check again.
- No `den-pass` in the address (page opened directly) → page says to open it from the den and offers no buy.
- Wallet without `solana:signMessage` or rejects a prompt → plain message, nothing registered.

---

### Task 1: Core — issuer from the environment

**Files:**
- Modify: `apps/cave-den/server/config/runtime.exs` (prod `config :den` block)
- Modify: `apps/cave-den/server/lib/den_web/page.ex` (serve `/config.json`)
- Modify: `apps/cave-den/web/app.js` (button label)
- Modify: `scripts/den/root/README.md` (settings table, "Who gets in")
- Test: `apps/cave-den/server/test/den_web/page_test.exs`

**Interfaces:** Produces `config.json` keys `issuer` (string URL) and `issuerLabel` (string) when `:den, :issuer` / `:issuer_label` are set.

- [ ] Test: with `Application.put_env(:den, :issuer, "https://access.example.org/")` and `:issuer_label "Get access"`, `get(conn, "/config.json")` is JSON with those two values and the built file's other keys; without them the file is served unchanged.
- [ ] Run `cd apps/cave-den/server && mix test test/den_web/page_test.exs` → fails.
- [ ] `Page.call` clause for `path_info: ["config.json"]`, GET/HEAD: read `priv/page/config.json` (or `%{}`), `Map.merge` the set keys, `json`, CSP headers, `cache-control: no-cache`.
- [ ] runtime.exs: `DEN_ISSUER` must be `https://…` (raise otherwise) → `issuer`; `DEN_ISSUER_LABEL` (≤ 40 chars) → `issuer_label`.
- [ ] app.js: `if (enter && config.issuerLabel) enter.textContent = config.issuerLabel;`
- [ ] README: two table rows; one sentence in `external` ("…or set `DEN_ISSUER` and `DEN_ISSUER_LABEL` with the stock image").
- [ ] `mix test` all pass; `web/build.sh`; commit.

### Task 2: Access check library

**Files:**
- Create: `apps/epeaky-access/lib/b58.mjs` — `decode(str) → Uint8Array`, `encode(bytes) → string`
- Create: `apps/epeaky-access/lib/check.mjs`
- Test: `apps/epeaky-access/tests/check.test.mjs`, fixtures in `tests/fixtures/*.json` (recorded `getTransaction` answers, edited copies for the failure cases)

**Interfaces:**
- `message(pass, signature) → string` = `"Epeaky access\npass: " + pass + "\ntx: " + signature`
- `async verifySigned(wallet, msg, sigB58) → boolean` (Ed25519, WebCrypto)
- `judge(tx, {wallet, mint, minLamports, now, maxAge}) → {ok:true} | {ok:false, why}` where `why ∈ 'missing','failed','old','payer','small','token'`
- `async claim({pass, signature, wallet, signed}, deps) → {ok:true} | {ok:false, why, status}`; `deps = {getTx(sig), record(sig, pass, wallet) → 'new'|'same'|'other', register(pass) → {ok}|{error}, mint, minLamports, now}`; extra `why ∈ 'pass','signed','used','den'`.

- [ ] Tests: real buy passes; meta.err → failed; blockTime 20 min old → old; another first signer → payer; lamport drop 0.05 SOL → small; no EPEAKY increase → token; bad message signature → signed; record 'other' → used; record 'same' → registers again; pass not 64 hex → pass. Ed25519 test key made with WebCrypto in the test.
- [ ] Run `node --test apps/epeaky-access/tests` → fails; implement; passes; commit.

### Task 3: Functions and storage

**Files:**
- Create: `apps/epeaky-access/functions/api/quote.js` (`GET ?sol=0.1` → Jupiter quote summary + the quote)
- Create: `apps/epeaky-access/functions/api/swap.js` (`POST {sol, wallet}` → re-quotes server-side for the fixed amount, returns `{tx}` base64 from Jupiter `/swap`)
- Create: `apps/epeaky-access/functions/api/claim.js` (`POST` → `claim()` with D1 + RPC + den deps)
- Create: `apps/epeaky-access/schema.sql` (`claims(signature TEXT PRIMARY KEY, pass TEXT NOT NULL, wallet TEXT NOT NULL, at INTEGER NOT NULL)`)
- Create: `apps/epeaky-access/wrangler.toml` (`pages_build_output_dir = "public"`, `[[d1_databases]] binding = "CLAIMS"`, vars `MINT`, `AMOUNTS`, `DEN_URL`, `SOLANA_RPC`)

- [ ] `sol` not in `AMOUNTS` → 400. Slippage 300 bps. `dynamicComputeUnitLimit: true`, `prioritizationFeeLamports: "auto"`.
- [ ] record: `INSERT … ON CONFLICT DO NOTHING`, then select the row: same pass → 'same', other → 'other', inserted → 'new'.
- [ ] register: `fetch(DEN_URL + "/door/passes", {method:"POST", headers:{authorization:"Bearer " + env.DEN_ISSUER_TOKEN, "content-type":"application/json"}, body: JSON.stringify({pass})})`.
- [ ] getTx: RPC `getTransaction` with `{encoding:"jsonParsed", maxSupportedTransactionVersion:0, commitment:"confirmed"}`.
- [ ] `npx wrangler pages dev public` smoke: quote answers; commit.

### Task 4: The page

**Files:**
- Create: `apps/epeaky-access/public/index.html`, `public/access.js`, `public/access.css`, `public/_headers` (CSP: self + connect self; frame-ancestors none)

- [ ] Reads `den-pass` from the hash; none → "Open this from the den's page." and no buttons.
- [ ] Wallet Standard discovery (`wallet-standard:app-ready` / `register-wallet`), only wallets with `standard:connect`, `solana:signAndSendTransaction`, `solana:signMessage`.
- [ ] Three buttons, each filled from `/api/quote` (EPEAKY out, price impact). Risk notes inline above them.
- [ ] Buy: `/api/swap` → `signAndSendTransaction({account, chain:"solana:mainnet", transaction})` → `signMessage(message(pass, sig))` → `/api/claim`, retrying "missing" every 3 s up to 60 s, then a Check again button.
- [ ] Success: `opener.postMessage({den:"pass", registered:true}, DEN_ORIGIN)`, "You're in.", close. Cancel button → `{den:"pass", error:true, cancelled:true}`. `DEN_ORIGIN` from `/api/config` (from `DEN_URL`).
- [ ] Browser check with Playwright on `wrangler pages dev`: no pass → message; with pass → three quoted buttons; commit.

### Task 5: Release and deploy

- [ ] Core: `scripts/den/sync.sh --push`; wait for CI + image.
- [ ] `wrangler d1 create epeaky-access`, apply schema, id into wrangler.toml; `wrangler pages project create epeaky-access`; deploy; `DEN_ISSUER_TOKEN` secret from a 0600 scratch file (never printed).
- [ ] Test den (user runs; remote writes are theirs): `DEN_ENTRY=invite,approval,external`, `DEN_ISSUER=https://epeaky-access.pages.dev/`, `DEN_ISSUER_LABEL=Get access with $EPEAKY`, `DEN_ISSUER_TOKEN` from the same file, `epeaky update`.
- [ ] Check: door shows the button; popup opens with the pass; a forged claim is refused end to end (`/api/claim` with a random signature → missing). The real 0.1 SOL buy is the user's.
