# Zyn Pay: scheduled payments, one-off payments and refunds

Design, 23 Oct 2026. Status: ledger + node + Nap built (branch zyn-pay-nap); merchant package not built; not deployed.

## Goal

A merchant can sell a subscription, a donation plan or any repeating payment
in ZEC, ETH, SOL or BOLD, and get paid every period without the customer
doing anything after the first approval. A merchant can also take one-off
payments and refund them without ever learning who paid.

Zcash itself cannot do this: every shielded spend needs a fresh proof made
with the spending key, so a ZEC subscription needs the customer's wallet open
each period. CipherPay's subscriptions say so plainly ("Zcash requires the
customer to actively send each payment") and send the customer an invoice
link each time. On Zyn the customer signs once and the ledger keeps the
schedule.

Background: the ZEC Pay design (2026-10-23-zec-pay-design.md) is parked —
CipherPay has run that product on mainnet since February.

## What the customer approves

One consent card in Nap (S13: every field that changes the outcome is shown):

> **Pay merchant.example 10 BOLD every month, 12 times, from 1 Nov 2026.**
> Sets aside 120 BOLD now. Cancel any time; what has not been paid comes back.

The customer chooses how many periods to fund now (at least one). Funding
fewer than the total is allowed; Nap reminds them to top up before the
money runs out.

## Why funds are set aside

On the object ledger value lives in notes, and spending a note needs a
nullifier only its owner can derive. The ledger therefore cannot take money
from a customer's notes later. Instead, approval moves the funded amount out
of the customer's notes into the schedule (the same way market orders and
lending pools hold value), and the ledger pays out of that on each due date.

Consequences, stated to the customer:
- the amount set aside is not spendable while the schedule runs;
- cancelling returns the unpaid remainder at once;
- the merchant can never take more than one period's amount per period.

A later phase may let Nap top a schedule up automatically while it is open
(auto top-up, phase 2). The ledger rule stays the same.

## Ledger changes (`objectvm/src/pay.rs`)

### State

`Schedule`:
- `id` — `H(payer ‖ nonce)`;
- `payer`, `payee` (account ids);
- `asset`, `amount` (per period);
- `every` — `Days(n)` or `Months(n)`; months use the UTC calendar (a
  schedule started on the 31st pays on the last day of shorter months);
- `next_due` (operator-clock seconds), `remaining` periods (or unlimited);
- `pot` — amount set aside and not yet paid;
- `reference` — 32 bytes chosen by the merchant (its subscription id);
- `state` — `active`, `unfunded`, `ended`, `cancelled`.

`Payment` (every payment made through `pay`, for refunds):
- `id` — `H(schedule ‖ period)` or `H(payer ‖ nonce)` for one-off;
- `payer`, `payee`, `asset`, `amount`, `fee`, `refunded`, `at`;
- `reference` — the one-off's own reference, or its schedule's (DECISIONS §188.16).

Both live in a new state section as a Merkle root of blinded leaves, like the
market's claims: the public learns nothing; the operator and replicas read
them (tier 2).

### Ops

| Op | Authority (S14) | Does |
|---|---|---|
| `PayCreate { payer, payee, asset, amount, every, first_due, periods, fund_periods, reference, nonce, inputs, change }` | payer | Consumes `inputs`, moves `amount × fund_periods` into the pot, creates the schedule. Refuses a payee with no registered viewing key, `first_due` before the clock's last time, `fund_periods < 1`, an amount below `MIN_PAY_AMOUNT`, a collection item as the asset, and any create before the pay clock has started. |
| `PayTopUp { payer, schedule, periods, inputs, change }` (typed field: `addPeriods`) | payer | Adds to the pot. An `unfunded` schedule becomes `active` at the latest period already due; missed periods are not charged and count against the term, and the next tick pays that one period. |
| `PayCancel { payer, schedule }` | payer | Ends it now; the pot becomes a claim for the payer. |
| `PayOnce { payer, payee, asset, amount, reference, nonce, inputs, change }` | payer | A one-off payment with a refundable record that keeps its `reference`. Same floor, item and clock refusals as `PayCreate`. |
| `PayRefund { payee, payment, amount, inputs, change }` | payee | Pays up to the unrefunded amount back to the original payer as a claim. The payee names the payment, never the payer. |
| `PayClock { at }` | operator | Advances the clock; pays every schedule due at or before `at`, oldest first, at most `MAX_DUE_PER_TICK` (256, S5), the rest at the next tick. |

Each payout is a claim for the payee (`market.pay`), which the payee's
wallet or server collects as usual (`zyn_collect`). Claims do not change. Payment ids derive from the schedule and the period,
and a payee's claim is `claim_id(payment, 0)`, so the merchant matches a
payment to its subscription by id (DECISIONS §188.7).

A due schedule whose pot holds less than one period is marked `unfunded`
and pays nothing; a top-up resumes it at the latest period already due.
Periods skipped while unfunded are never charged and count against
`remaining`, so "N times from date D" keeps its end date; after the term's
last due date has passed, a resume pays at most that one overdue period.
Nothing is partially paid.

A cancel takes effect immediately: periods due but not yet paid (for example
held back by the 256-per-tick cap) are not paid, and their money returns with
the pot. A refused pay op leaves ledger state unchanged.

### The clock

`PayClock` is posted by the operator like `LendTime` and `DaoClock`: forward
only, and by at most an hour per post, because the ledger holds no epoch
timestamp to check it against. The operator switches Pay on with `ZYN_PAY=on`
in the node's env; until then the node never posts and the chain keeps every
root. Once on, the node posts at startup and every `ZYN_PAY_TICK_SECS`
(default 10 s), and catches up an hour per post after downtime. `PayCreate`
and `PayOnce` are refused until the first post has started the clock
(DECISIONS §188.14). A schedule
therefore pays within one tick of its due time, and never early. Until the
clock has caught up, a new schedule's first due date is checked against the
clock's last time, so a past first date can be accepted and paid from the
payer's own pot.
The operator can delay a tick; that is the same trust as every other
operator-sequenced action, and a delay shows in the journal.

### Fee

0.5% of each scheduled or one-off payment, taken from the payee's side and
paid to the team treasury (the fee pot). Refunds carry no fee and do not
return it. The rate is a committed parameter.

### Minimum

A period's amount and a one-off must be at least `MIN_PAY_AMOUNT`, 0.001 of
an 18-decimal token, because each schedule and payment is a record kept for
good. Refunds have no floor. Collection items cannot be paid.

### Rollout

New ops and a new state section are a chain-format change: verifier first
(§51.8). Every signer and replica takes the new build before the sequencer
emits a `Pay*` op. Testnet 212 first; mainnet 26460 after the NU7 switch.

## Nap

- Consent cards for all five holder pay ops (`PayCreate`, `PayTopUp` with
  `addPeriods`, `PayCancel`, `PayOnce`, `PayRefund`), from the typed intent
  (S13). Each typed struct also carries `intent`, the digest of the whole op,
  so an EVM signature binds the notes spent and the change (DECISIONS §188.10).
- A **Payments** list under Zyn: each schedule with payee, amount, next
  payment, funded until, Top up, Cancel; past payments and refunds.
- A notice 7 days (or one period, if shorter) before a pot runs out, and
  when a schedule turns `unfunded`.
- Nap Connect methods: `zyn_payCreate`, `zyn_payOnce`, `zyn_payTopUp`,
  `zyn_payCancel`, `zyn_paySchedules`.

## Merchant side

The merchant is a Zyn account. `@zyn-io/pay` (Node) and the same engine as a
small `zyn-pay serve` service:

- **Checkout:** `checkout({ plan | amount, reference })` returns what the page
  passes to Nap Connect (`zyn_payCreate` / `zyn_payOnce`); a page without Nap
  shows "Pay with Nap" and the Nap web link.
- **Watching:** reads the payee's claims through the node's signed account
  reads and collects them. No viewing key leaves the merchant.
- **Refunds:** `refund(paymentId, amount)` signs `PayRefund` with the merchant
  key.
- **Webhooks**, versioned, each with an event id and a timestamped HMAC
  signature: `subscription.created`, `payment.succeeded`,
  `subscription.unfunded`, `subscription.resumed`, `subscription.cancelled`,
  `subscription.ended`, `payment.refunded`.
- **API** under `/v1`: `POST /checkouts` (Idempotency-Key),
  `GET /subscriptions/:reference`, `GET /payments/:id`,
  `POST /payments/:id/refund`.
- Built gateway-ready as in the parked ZEC Pay design: every record keyed by
  merchant id, storage behind an interface, a hosted multi-merchant service
  possible later.

## What is visible to whom (shown inline in Nap and the docs)

- **Public:** no payer, payee, amount or id. Pay ops appear in ZynScan as
  labelled rows, and the pay clock posts on a fixed cadence (DECISIONS
  §188.12, §188.2).
- **The merchant:** payments to its account, matched by payment id
  (DECISIONS §188.7); it reads the schedule's reference through
  `zyn_paySchedules`; never the payer's account or address. Refunds go back without it.
- **The operator (us), today:** every schedule and payment, including payer
  and payee. The same tier-2 visibility as all of Zyn.
- **Whoever holds an id:** `zyn_paySchedules` and `zyn_payPayments` read by
  unguessable id with no signature. Payer and payee keep schedule and payment
  ids private: a leaked schedule id shows its pot, state and next due date,
  and a leaked payment id its amount, fee and refunds (DECISIONS §188.11).
- **Amounts on a thin fee pot:** each payment's fee goes to the ZYN pot, whose
  figure is published at three significant figures. While the pot is thin, a
  change in that figure can show a payment's approximate size, as for Lend
  (DECISIONS §157.18, §188.17). A separate, coarser pay-fee pot is deferred.
- **Custody:** amounts set aside are on Zyn; withdrawing to Zcash depends on
  the team's threshold custodians.

## Not in this spec

- Merchant pull with caps (variable and usage billing, agents): phase 2, on
  the same `Schedule` with a `cap` instead of a fixed amount.
- Prices in dollars paid in ZEC/ETH/SOL (using posted prices): phase 2. v1
  prices in the payment asset; BOLD is the dollar option.
- Auto top-up from Nap: phase 2.
- Escrow and batch payouts: own specs.
- Shielded ZEC paid straight from a Zcash wallet: that is CipherPay's ground.

## Error handling

- Insufficient inputs, unknown asset, payee without viewing key, `first_due` before the clock's last time, zero amount or periods: refused at sequencing with a reason.
- Before the pay clock has started: `PayCreate` and `PayOnce` refused ("the pay clock has not started").
- An amount below `MIN_PAY_AMOUNT`, a collection item as the asset, a zero period, or a schedule whose next date overflows the calendar: refused, each with its own reason.
- Clock more than an hour ahead of its last time, or backwards: refused.
- More than 256 schedules due in one tick: the rest pay at the next tick, in
  due order; never skipped.
- Refund above the unrefunded amount, or by anyone but the payee: refused.
- Webhook delivery failures: retried with backoff for 24 h; state stays
  readable through the API.

## Testing

- `objectvm` unit tests: create/fund/pay/end; Days and Months schedules
  including month ends; unfunded then top-up; cancel returns exactly the
  pot; refund routing and limits; authority on every op; conservation across
  all of them; the 256-per-tick bound and ordering; clock rules.
- Journal replay and replica determinism on a testnet fixture with schedules
  spanning several clock ticks.
- Nap: consent card fields match the op (S13); Payments list; cave.e2e-style
  end-to-end on testnet 212: subscribe, two periods paid, cancel, remainder
  back.
- `@zyn-io/pay`: checkout, claim matching, refunds, webhooks (signature,
  ids, retries), idempotent API.

## Open points for the user

- The fee: 0.5% of each payment, paid by the merchant — or another rate.
- Whether the first live merchant is one of ours (a Jade or Cave
  subscription, or recurring donations to the team treasury) before inviting
  outside merchants.
