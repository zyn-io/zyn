# ZynRouter — private payments for AI and media APIs — design

Date: 5 Oct 2026. Draft for review; nothing built.

## Why

The Ethereum Foundation's zkAPI (live 1 Oct 2026) pays for API usage without a
billing identity: deposit once into a vault, prove "a funded note covers this
and is unspent" per lease, get a short-lived capped key. It covers one provider
(OpenRouter, `/v2/openrouter/leases`), bills in native ETH ("not a stable-dollar
deposit") or USDC, is funded from a public Ethereum address, has one operator
and an escape challenger, and has no budgets for agents. Its repository has no
licence: we take the design (Crapis & Buterin, ethresear.ch), not the code.

ZynRouter routes a paid request to a provider and pays for it privately on Zyn.
Agents are one use, and the one that carries the trend.

## What it does that zkAPI does not

| zkAPI | ZynRouter |
|---|---|
| OpenRouter only (providers that mint capped sub-keys) | Any provider: capped keys where the provider offers them, a broker relay where it does not |
| Metered leases; long jobs fit badly | **Exact quotes** for deterministically priced work — video per second, image per picture, voice per minute — paid once, no meter, no refund, however long the job runs |
| ETH (volatile) or USDC (freezable) | **BOLD.zy** (stable, no issuer freeze, §68) or ZEC.zy |
| Funded from a public Ethereum address | Funded through shielded ZEC or Zyn's private channel |
| One person, one local gateway | **Agent accounts** with a hard budget; later, chain-enforced mandates |
| One operator; providers paid off-chain | Several providers and brokers listed on Zyn, paid directly in BOLD.zy / ZEC.zy |
| Ethereum gas per deposit and settlement | Zyn epoch batches |

Same as zkAPI, stated plainly: the provider sees the prompt; IP address and
timing leak unless the client uses a relay. Weaker than zkAPI until phase 3:
Zyn's operator can see payments (tier 2), where zkAPI's server cannot link a
lease to a deposit.

## 1. Exact-quote jobs (phase 1 — the demo)

For work whose price is known up front.

1. The client asks a broker for a quote: `{model, params} → {price, asset, quote_id, expires}` (signed by the broker).
2. The client pays exactly `price` to the broker's Zyn account with a private
   note, the memo carrying `quote_id`. On Zyn this is an ordinary payment: other
   holders see nothing; the broker sees a payment, not who made it.
3. The broker sees the payment at the next seal (about 10 s on testnet), runs the job against the
   provider with its own key, and returns a job handle; the result is fetched by
   handle (video, image, audio) and kept for a short time, then deleted.
4. Failure refunds: a job the provider rejects is refunded to a one-time return
   commitment the client put in the memo, never to an account.

What the broker learns: the prompt and the job, not the payer. What the
provider learns: the prompt, under the broker's identity.

First provider: a video API with per-second pricing (fal or Replicate), so the
demo is "an agent pays 0.5 BOLD for a 5-second clip — no account, no card".

## 2. Capped keys (phase 2 — zkAPI-compatible)

For metered text models. The client proves funds, the broker mints a short-lived
provider key capped in dollars (as zkAPI does with OpenRouter), the provider
reports usage at expiry, the broker settles it and returns the remainder as a
new private note. ZynRouter exposes the same local OpenAI/Ollama-compatible
gateway as zkAPI's client, so apps and agents point at `localhost` unchanged.

## 3. Unlinkable spends (phase 3)

To hide the payer from Zyn's operator as well, a spend presents a proof of
membership plus a nullifier, as zkAPI's credits do. Zyn's private channel
already proves note membership and nullifiers (§158); the credit check is an
adaptation of that circuit, not new research.

## Agents

- **Phase 1–2: a dedicated agent account** (§51.7 — its balance is the hard
  cumulative loss bound). The owner funds it; the agent holds only its key;
  ZynRouter pays from it. Top-ups and withdrawals are the owner's.
- **Later: mandates on the object ledger** (per day, per provider, per asset,
  expiry), verified by the ledger as §51.3 requires. Mandates today exist only
  in the zyn-vm (swapvm-era) format, not on the object ledger.
- Providers are addressed by name (`fal.zyn`) once Zyn names exist.

## Reuse

Payments, private notes, memos, epoch batching, BOLD.zy/ZEC.zy, Nap Connect,
the private channel's circuit (phase 3). New: the broker service (quote, watch
payments, run jobs, refunds), a provider adapter per API, the local gateway
(phase 2), a ZynRouter page in the app for providers and prices.

## Risks

- **Content.** Brokers relay generated media; each broker keeps the provider's
  content policy and the right to refuse. ZynRouter lists brokers; it does not
  run content through itself (§91.7: permissioning at the gateway).
- **Provider terms.** Reselling an API under one key must be allowed by that
  provider's terms; check per provider before listing.
- **Broker honesty.** Phase 1 trusts the broker to run the job it was paid for;
  quotes are signed, so a short-changed client can prove the quote and the
  payment.

## Open questions for the user

1. First provider for the demo: fal or Replicate (video), or both.
2. Who runs the first broker — us, as the reference — and its fee.
3. Phase order after the demo: capped keys (zkAPI compatibility) or unlinkable spends.
