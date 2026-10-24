# Zyn Pay — Plan 2: Nap (payer side) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A person can approve a scheduled payment or a one-off payment that a site asks for through Nap Connect, see their schedules in Nap, top them up and cancel them, and get back what a cancel, the end of a term or a refund returns, all in ZEC.zy, ETH.zy, SOL.zy or BOLD.zy.

**Architecture:** Nap builds object-ledger intents in JS (`apps/nap-web/src/market.js`), funds them from held notes (`fund`), signs the raw intent bytes with Ed25519 and submits through `submitFresh` (`src/ledger.js`). Zyn Pay adds:
- the five holder pay encoders and their id helpers, checked byte for byte against vectors the Rust side generates;
- pay reads through the node's `zyn_paySchedules` and `zyn_payPayments`;
- pay claim sources so `collectOwed` picks up what comes back;
- five Nap Connect methods whose `prepare` rows are the consent cards;
- a Payments view in the web app.

**Tech Stack:** JavaScript (ES modules, `node --test`), Rust for the vectors (`zyn-object-vectors`), Vite builds (web, extension, mobile share `src/`).

**Spec:** `docs/superpowers/specs/2026-10-23-zyn-scheduled-payments-design.md` (section "Nap"), DECISIONS §188.
**Builds on:** plan 1 (ledger + node), PR zyn-io/zyn#1. This branch (`zyn-pay-nap`) stacks on it.

## Global Constraints

**Op codes and programs.**
- Codes: PayCreate 69, PayTopUp 70, PayCancel 71, PayOnce 72, PayRefund 73.
- Programs `pay-create`, `pay-manage` (top-up and cancel), `pay-once`, `pay-refund`.
- Nap never builds PayClock (74).

**Byte layout.** These match `ObjectVm::encode_intent` exactly. `i128` is the existing `Fixed` encoder, `u32`/`u64` are the existing big-endian helpers, and `inputs()`/`outputs()` are the existing list encoders.
- **69 PayCreate:** `u8 69, payer, program, payee, asset, i128 amount, every (u8 0=Days|1=Months, u32 n), u64 firstDue, periods (u8 0 | u8 1 + u32), u32 fundPeriods, reference, nonce, inputs, outputs`
- **70 PayTopUp:** `u8 70, payer, program, schedule, u32 periods, inputs, outputs`
- **71 PayCancel:** `u8 71, payer, program, schedule`
- **72 PayOnce:** `u8 72, payer, program, payee, asset, i128 amount, reference, nonce, inputs, outputs`
- **73 PayRefund:** `u8 73, payee, program, payment, i128 amount, inputs, outputs`

The vectors are the authority: if this text and the vectors disagree, the vectors win.

**Ids (same leaf construction as Rust `pay.rs`):**
- `scheduleId(payer, nonce) = id('objectvm.pay.schedule.v1', payer, nonce)`
- `payPaymentId(schedule, period) = id('objectvm.pay.payment.v1', schedule, u32be(period))`
- `payOnceId(payer, nonce) = id('objectvm.pay.once.v1', payer, nonce)`
- `refundSource(payment, n) = id('objectvm.pay.refund.v1', payment, u32be(n))`
- `cancelSource(schedule) = id('objectvm.pay.cancel.v1', schedule)`
- Claims are `claimId(source, 0)`.

**Ledger rules Nap must not fight:**
- **Minimum amount:** `MIN_PAY_AMOUNT` = 0.001 token (1e15 base units) per period or per one-off payment.
- **Clock start:** pay is refused while the pay clock has not started ("the pay clock has not started"). Show that sentence as it is.
- **No items:** collection items are refused as pay assets.

**Copy** (~/.claude/CLAUDE.md, docs/MESSAGING.md):
- **Labels:** "Payments", "Top up", "Cancel", "Pay".
- **Wording:** "sets aside", never "auto-debit".
- **Disclosure in every consent card:** "Who pays whom is not shown to the public; Zyn's operator sees it." It stays visible inline, not behind an "i".
- **Every create card states:**
  - the amount set aside now;
  - the schedule ("10 BOLD every month, 12 times, from 1 Nov 2026");
  - that cancelling returns what has not been paid.

**Privacy:** `zyn_paySchedules` reveals schedule ids to a site only under a new grant scope `payments`, given once per site and network (like `positions`). Schedule and payment ids are access to the record (§188.11), so Nap never logs them and never sends them anywhere except the node and the requesting site's own schedules.

**Payee side is not in this plan.** A Nap user who is a payee would need the payer's schedule id to find the claims. Merchants collect through plan 3's package. The Payments view shows schedules the user pays.

## Review Focus

1. **Locale and time zone.** The schedule text shown in a card must be computed in UTC from `firstDue`, exactly as the ledger does. A user in UTC+8 approving "from 1 Nov" must not see 31 Oct. Test: `schedule_text_is_utc` (Task 4).
2. **Fewer funds than the request.** A site asks `fundPeriods: 12`, but the wallet holds enough for 3. The card must refuse up front with the shortfall, not fail at submit. Test: `create_refuses_when_holdings_cannot_fund` (Task 4).
3. **Wallet restored from its phrase.** Schedules made before a restore must reappear: nonce recovery covers pay nonces. Test: `restored_wallet_finds_its_schedules` (Task 3).
4. **Cancel racing a due period.** The card says what comes back "as of now". The view refreshes after submit and shows the actual returned claim. Test: `cancel_shows_what_came_back` (Task 5).
5. **Malformed site requests.** Bad hex, negative or fractional periods, `every: {months: 0}`, a `firstDue` in the past, an unknown asset, or an item asset. Each must get `bad()` with a plain message before any card shows. Test: `bad_requests_never_reach_a_card` (Task 4).

---

## File Structure

- Modify `zynzapd/src/market_vectors.rs`: a `pay_vectors()` block (programs, ids, five intents). Regenerate `apps/nap-web/tests/object-vectors.json` with `cargo run -p zynzapd --bin zyn-object-vectors`, using the command its doc comment gives.
- Modify `apps/nap-web/src/market.js`: `MARKET_PROGRAM` pay entries, `TAG` 69–73, the five `pay*Intent` encoders and the id helpers.
- Modify `apps/nap-web/src/ledger.js`:
  - `loadSchedules(config, identity)` and `loadPayments(config, ids)`;
  - pay claim sources in `claimSources`/`claimKinds`/`CLAIM_KINDS`;
  - pay nonces in `recoverNonces`.
- Create `apps/nap-web/src/pay.js`: pure helpers.
  - `scheduleText({amount, asset, every, periods, firstDue})` (UTC)
  - `fundedUntil(schedule)`
  - `runsOutSoon(schedule, now)` (7 days, or one period if shorter)
  - `parsePayRequest(params)` (validation)
- Modify `apps/nap-web/src/connect-ops.js`: methods `zyn_payCreate`, `zyn_payOnce`, `zyn_payTopUp`, `zyn_payCancel` (write) and `zyn_paySchedules` (read, scope `payments`).
- Modify `apps/nap-web/src/connect-grants.js`: scope `payments`.
- Modify `apps/nap-connect/nap-connect.js`, `apps/jade/public/nap-connect.js`, `apps/cave-market/nap-connect.js`: the method list in the doc comment. Keep the copies as their sync rule says.
- Modify `apps/nap-web/src/app.js`:
  - the `payments` view: list, Top up, Cancel, past payments;
  - the runs-out notice;
  - in-app review dialogs, modelled on `openObjectSend`.
- Tests:
  - `apps/nap-web/tests/pay.test.mjs` (encoders against vectors, ids, `pay.js` helpers);
  - `apps/nap-web/tests/pay-ops.test.mjs` (METHODS, modelled on `lend-ops.test.mjs`);
  - update `tests/market.test.mjs`, `tests/connect-grants.test.mjs` and `tests/idle-lock.test.mjs` only if their lists must grow.
- E2E:
  - `apps/nap-connect/tests/pay.e2e.mjs` plus a tiny local page `apps/nap-connect/tests/pay-page.html` that calls the methods. Modelled on `lend.e2e.mjs`.
  - It runs live only after the testnet rollout with `ZYN_PAY=on`.
- Docs: DECISIONS §188 rows for Nap, and the `apps/nap-connect/README.md` methods table.

---

### Task 1: Pay vectors from the Rust side

**Files:**
- Modify: `zynzapd/src/market_vectors.rs`. Add `pay_vectors(&input)` and chain it like `lend_vectors`.
- Regenerate: `apps/nap-web/tests/object-vectors.json`.

**Interfaces:**
- **Produces:** vector keys
  - programs: `program_pay_create`, `program_pay_manage`, `program_pay_once`, `program_pay_refund`;
  - ids: `pay_schedule_id`, `pay_payment_id_0`, `pay_payment_id_1`, `pay_once_id`, `pay_refund_source_0`, `pay_cancel_source`;
  - intents: `pay_create`, `pay_create_until_cancelled`, `pay_top_up`, `pay_cancel`, `pay_once`, `pay_refund`.

- [ ] **Step 1: Add the vectors.** Follow `lend_vectors`:

```rust
/// Zyn Pay: its programs, derived ids and the five intents a holder signs.
fn pay_vectors(input: &[([u8; 32], [u8; 32])]) -> Vec<(&'static str, String)> {
    use objectvm::pay::{cancel_source, pay_once_id, pay_payment_id, refund_source, schedule_id, Every};
    let enc = |op: &Op| hex(&ObjectVm::encode_intent(op));
    let schedule = schedule_id(&OWNER, &NONCE);
    let payment = pay_payment_id(&schedule, 0);
    let create = |periods: Option<u32>| Op::PayCreate {
        payer: OWNER, program: program::pay_create(), payee: PAYEE, asset: ZEC,
        amount: Fixed::whole(2), every: Every::Months(1), first_due: 1_793_491_200,
        periods, fund_periods: 3, reference: [0x5e; 32], nonce: NONCE,
        inputs: input.to_vec(), change: vec![change()],
    };
    vec![
        ("program_pay_create", hex(&program::pay_create().0)),
        ("program_pay_manage", hex(&program::pay_manage().0)),
        ("program_pay_once", hex(&program::pay_once().0)),
        ("program_pay_refund", hex(&program::pay_refund().0)),
        ("pay_schedule_id", hex(&schedule)),
        ("pay_payment_id_0", hex(&payment)),
        ("pay_payment_id_1", hex(&pay_payment_id(&schedule, 1))),
        ("pay_once_id", hex(&pay_once_id(&OWNER, &NONCE))),
        ("pay_refund_source_0", hex(&refund_source(&payment, 0))),
        ("pay_cancel_source", hex(&cancel_source(&schedule))),
        ("pay_create", enc(&create(Some(12)))),
        ("pay_create_until_cancelled", enc(&create(None))),
        ("pay_top_up", enc(&Op::PayTopUp { payer: OWNER, program: program::pay_manage(), schedule, periods: 2, inputs: input.to_vec(), change: vec![change()] })),
        ("pay_cancel", enc(&Op::PayCancel { payer: OWNER, program: program::pay_manage(), schedule })),
        ("pay_once", enc(&Op::PayOnce { payer: OWNER, program: program::pay_once(), payee: PAYEE, asset: ZEC, amount: Fixed::whole(1), reference: [0x5e; 32], nonce: NONCE, inputs: input.to_vec(), change: vec![change()] })),
        ("pay_refund", enc(&Op::PayRefund { payee: PAYEE, program: program::pay_refund(), payment, amount: Fixed::whole(1), inputs: input.to_vec(), change: vec![change()] })),
    ]
}
```

Define `const PAYEE: [u8; 32] = [0x9a; 32];` next to `OWNER` if no second account constant exists. Use the file's existing `OWNER`, `NONCE`, `ZEC`, `change()` and `hex()`.

- [ ] **Step 2: Regenerate and check.** Run the generator exactly as its doc comment says, then `git diff --stat apps/nap-web/tests/object-vectors.json`. Expected: only added keys, no existing vector changed.
- [ ] **Step 3: Run the Rust tests that read the vectors.** `cargo test -p zynzapd market_vectors`, or whatever test the file names, must pass.
- [ ] **Step 4: Commit.** Commit `zynzapd/src/market_vectors.rs` and `apps/nap-web/tests/object-vectors.json`.

### Task 2: JS encoders and ids

**Files:**
- Modify: `apps/nap-web/src/market.js`.
- Test: `apps/nap-web/tests/pay.test.mjs`.

**Interfaces:**
- **Produces** these exports from `market.js`:
  - `payCreateIntent({payer, payee, asset, amount, every, firstDue, periods, fundPeriods, reference, nonce, funding})`
  - `payTopUpIntent({payer, schedule, periods, funding})`
  - `payCancelIntent({payer, schedule})`
  - `payOnceIntent({payer, payee, asset, amount, reference, nonce, funding})`
  - `payRefundIntent({payee, payment, amount, funding})`
  - `scheduleId`, `payPaymentId`, `payOnceId`, `refundSource`, `cancelSource`
  - `MIN_PAY_AMOUNT = 10n ** 15n`
- **Argument shapes:**
  - `every` is `{days: n}` or `{months: n}`;
  - `periods` is a number, or `null` for until cancelled;
  - `funding` is what `fund()` returns, with the same shape `lendSupplyIntent` takes.
- **Consumes:** the existing `id`, `u32be`, `u64be`, `i128`, `inputs`, `outputs`, `MARKET_PROGRAM`, `TAG`.

- [ ] **Step 1: Write the failing test.** `tests/pay.test.mjs` loads `object-vectors.json` the way `market.test.mjs` does (lines 18 and 240), builds each intent from the same constants the Rust side used, and asserts hex equality for:
  - all six intents;
  - all six ids;
  - all four programs.

  It also asserts that `MIN_PAY_AMOUNT` equals 1e15.
- [ ] **Step 2: Run it and see it fail.** `cd apps/nap-web && node --test tests/pay.test.mjs`. Expected: failures on the missing exports.
- [ ] **Step 3: Implement.**
  - Add `payCreate: 'pay-create', payManage: 'pay-manage', payOnce: 'pay-once', payRefund: 'pay-refund'` to `MARKET_PROGRAM`, and `payCreate: 69, payTopUp: 70, payCancel: 71, payOnce: 72, payRefund: 73` to `TAG`.
  - Write each encoder in the layout from Global Constraints, in the style of `lendSupplyIntent` (market.js:481).
  - Write the id helpers with `id(domain, …)`.
- [ ] **Step 4: Run the test** and the whole suite: `npm test`. Expected: pass.
- [ ] **Step 5: Commit.**

### Task 3: Reads, recovery and collecting what comes back

**Files:**
- Modify: `apps/nap-web/src/ledger.js`.
- Test: `apps/nap-web/tests/pay.test.mjs` (or a new `tests/pay-ledger.test.mjs`, stubbing `fetch` like `lend-ops.test.mjs`).

**Interfaces:**
- **Produces:**
  - `loadSchedules(config, identity) -> [{id, payee, asset, amount, every, firstDue, nextDue, index, remaining, pot, reference, state}]`. It derives schedule ids from the wallet's pay nonces and queries `zyn_paySchedules` in chunks of 240, like `loadPositions`.
  - `loadPayments(config, schedule) -> [{id, amount, fee, refunded, at, reference}]`. It covers `payPaymentId(schedule.id, k)` for `k < schedule.index` and queries `zyn_payPayments`.
- **Claims:**
  - Pay claim sources are `cancelSource(schedule)` and `refundSource(payment, n)` for the user's own payments, with n from 0 to `refunds` from the record.
  - They are added to `claimSources`/`claimKinds`/`CLAIM_KINDS`, so `collectOwed` and `collectWaiting` pick them up.
- **Recovery:** `recoverNonces` (ledger.js:104-131) also scans the pay nonce kind. Nonces used for PayCreate and PayOnce are recorded where lend nonces are, and recovered the same way after a restore.

- [ ] **Step 1: Write the failing tests.**
  - `restored_wallet_finds_its_schedules`: with the `fetch` stub, a schedule created at nonce counter k is found after a fresh `deriveIdentity` from the same phrase.
  - `cancel_returns_are_collected`: a claim at `claimId(cancelSource(s), 0)` appears in `loadOwed`.
  - `refunds_are_collected`.
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement**, following `loadPositions` (ledger.js:570) and the `lendSource` claim pattern.
- [ ] **Step 4: Run** `npm test`. Expected: pass.
- [ ] **Step 5: Commit.**

### Task 4: Pure helpers and the Nap Connect methods (the consent cards)

**Files:**
- Create: `apps/nap-web/src/pay.js`.
- Modify: `apps/nap-web/src/connect-ops.js` (`METHODS`) and `apps/nap-web/src/connect-grants.js` (scope `payments`).
- Modify: the method doc comments in the three nap-connect.js copies, and `apps/nap-connect/README.md`.
- Test: `apps/nap-web/tests/pay-ops.test.mjs`. Extend `tests/connect-grants.test.mjs`.

**Interfaces.**

Requests to the five methods:
- `zyn_payCreate {payee, asset, amount, every, firstDue, periods?, fundPeriods, reference}` → `{schedule, payment: null}`. `asset` is a symbol (ZEC/ETH/SOL/BOLD) or a hex asset id. `amount` is a decimal string. `every` is `{days}` or `{months}`. `firstDue` is unix seconds. `reference` is 32-byte hex.
- `zyn_payOnce {payee, asset, amount, reference}` → `{payment}`.
- `zyn_payTopUp {schedule, periods}` → `{schedule}`.
- `zyn_payCancel {schedule}` → `{schedule}`.
- `zyn_paySchedules {}`: returns **the schedules this site created**. Nap remembers which site asked for which schedule id, in the per-site grant store, and only those are returned. That keeps one site from seeing another's.

`pay.js`:
- `scheduleText(...)` gives "2 BOLD every month, 12 times, from 1 Nov 2026" (UTC, en-GB date) and "…, until cancelled".
- `fundedUntil(s)` is the date of the last funded period.
- `runsOutSoon(s, now)` follows Global Constraints.
- `parsePayRequest(p)` returns a clean request or throws `bad(...)` with a plain message.

**Cards.** These are `prepare` rows, so the generic `approvalCard` renders them.
- **Create.** Title "Scheduled payment". Rows:
  - "To" (payee, short);
  - "Amount" (per period);
  - "Schedule" (scheduleText);
  - "Set aside now" (amount × fundPeriods, asset);
  - "Funded until" (date);
  - "Fee" ("0.5%, paid by the recipient").

  Note: "Cancel any time in Nap; what has not been paid comes back. Who pays whom is not shown to the public; Zyn's operator sees it."
- **One-off.** Title "Payment". Rows: "To", "Amount". Note: the privacy sentence.
- **Top up.** Title "Top up payment". Rows: "To", "Adds", "Funded until (after)".
- **Cancel.** Title "Cancel payment". Rows: "To", "Comes back now" (pot). Note: "Nothing more will be paid on this schedule."

`run`:
- funds with `fund(identity, held, asset, total)`;
- submits with `submitFresh` and the matching intent;
- returns the ids;
- records the schedule id in the site's grant entry so `zyn_paySchedules` can return it.

Errors:
- Map the ledger's refusal strings to the same sentences: "the pay clock has not started", "an amount below the minimum", "items cannot be paid by schedule".
- Use `mustHold` for the shortfall before showing a card.

- [ ] **Step 1: Write the failing tests** in `pay-ops.test.mjs`, following `lend-ops.test.mjs`:
  - `schedule_text_is_utc`: run with `process.env.TZ='Asia/Shanghai'` set before importing.
  - `create_refuses_when_holdings_cannot_fund`.
  - `bad_requests_never_reach_a_card`: a table of bad inputs, each expecting a `bad()` error.
  - `create_card_states_set_aside_schedule_and_privacy`: the row labels and the note text.
  - `create_run_submits_the_vector_bytes`: the stubbed `zyn_submitSigned` receives a frame whose intent bytes equal `payCreateIntent(...)` for the same inputs.
  - `pay_schedules_needs_the_payments_scope_and_returns_only_this_sites_schedules`.
  - `cancel_card_shows_what_comes_back`.
  - `ledger_refusals_show_their_sentence`.
  - In `connect-grants.test.mjs`: the `payments` scope.
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement** `pay.js`, the five METHODS entries and the scope, using the patterns at connect-ops.js:653-684 (`zyn_lendSupply`) and connect-grants.js:21.
- [ ] **Step 4: Run** `npm test`. Expected: pass. No method is public, so `apps/extension/src/content.js` `PUBLIC` and `tests/idle-lock.test.mjs` stay unchanged. Check that this holds.
- [ ] **Step 5: Update docs.** Add the method rows to `apps/nap-connect/README.md`, and add the doc-comment list to the three nap-connect.js copies. Respect the copy-sync test, if one exists: grep `nap-connect.js` in tests.
- [ ] **Step 6: Commit.**

### Task 5: The Payments view

**Files:**
- Modify: `apps/nap-web/src/app.js`:
  - `VIEWS` (:437) gains `['payments', 'Payments']`;
  - a `data-view="payments"` div in `wallet()`;
  - a loader;
  - the review dialogs.
- Test: `apps/nap-web/tests/pay-view.test.mjs`, for the DOM-free parts: row models and the notice. If `app.js` can't be imported in node, test helpers exported from `pay.js`.

**Behaviour:**
- **One row per schedule**, from `loadSchedules`:
  - "To" (payee short);
  - "Amount" (per period + asset);
  - "Next" (date, or "Ended" / "Cancelled" / "Waiting for funds");
  - "Funded until";
  - buttons "Top up" and "Cancel" (for active and unfunded schedules).
- **Past payments** for a schedule appear when it is expanded (`loadPayments`): date, amount, "refunded" if any.
- **Notice:**
  - At the top of the view, and as a badge on the nav item, when any schedule `runsOutSoon`: "Payment to X runs out on D — Top up".
  - When a schedule is Unfunded: "Payment to X is waiting for funds".
- **Top up and Cancel** open a review dialog modelled on `openObjectSend` (app.js:1213). It shows the same rows as the Connect cards, then submits with the same functions. Put the rows in `pay.js` so both share them.
- **After submit:** reload schedules and run `collectWaiting`. For a cancel, show the returned claim (`cancel_shows_what_came_back`).
- **Empty state:** "No payments yet."

- [ ] **Step 1: Write the failing tests** for the row model, the notice and `cancel_shows_what_came_back`. The notice covers 8 days before (no notice), 6 days before (notice) and unfunded (waiting).
- [ ] **Step 2: Run them and see them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `npm test` and `npm run build`, plus `npm run build -- --mode extension` and `--mode mobile`, or the scripts in `package.json`. Expected: all builds succeed.
- [ ] **Step 5: Smoke-check the view in the browser.** Run `npm run dev` and screenshot the Payments view with the stubbed RPC, or against testnet once Pay is on. Then follow the project memory rule: run the existing e2e (`cave.e2e.mjs`, `lend.e2e.mjs` where they need no funded wallet) and update any they break.
- [ ] **Step 6: Commit.**

### Task 6: End-to-end on testnet (runs live after the rollout)

**Files:**
- Create: `apps/nap-connect/tests/pay-page.html`, a plain page with the nap-connect.js copy. Buttons: Subscribe (2 BOLD daily, 2 periods, fund 2), Pay once, Top up, Cancel, Schedules. It prints results into `<pre>`.
- Create: `apps/nap-connect/tests/pay.e2e.mjs`, modelled on `lend.e2e.mjs` (`PHRASE_FILE`, `NAP_LOCAL`, unlock):
  1. serve `pay-page.html` locally;
  2. click Subscribe, approve the card (assert its rows), and assert that `zyn_paySchedules` lists the schedule with `pot` = 2 periods;
  3. wait until `nextDue` passes and the pay clock pays period 0, then assert `index` 1 and `pot` 1 period;
  4. Cancel, and assert that the returned claim is collected in Nap;
  5. Pay once, and assert that the payment record carries the reference.
- If `ZYN_PAY` is off on the target node, the test stops at step 2 with "the pay clock has not started". That counts as a skip with a message, not a pass.

- [ ] **Step 1:** Write both files.
- [ ] **Step 2:** `node --check apps/nap-connect/tests/pay.e2e.mjs`.
- [ ] **Step 3:** Commit. The live run is the user's step after `ops/zyn-pay-rollout.sh testnet`. Its command goes in the report and in RUNBOOK's Zyn Pay section:
  `PHRASE_FILE=… NAP_LOCAL=apps/nap-web/dist node apps/nap-connect/tests/pay.e2e.mjs`

### Task 7: Record it

**Files:**
- `DECISIONS.md` §188: add rows.
  - Nap side built.
  - `payments` scope; a site sees only the schedules it asked for.
  - The payee side is not in Nap.
- `RUNBOOK.md` Zyn Pay section: the e2e command.
- `docs/superpowers/specs/2026-10-23-zyn-scheduled-payments-design.md` status line: "ledger + node + Nap built; merchant package not built".

- [ ] **Step 1:** Edit.
- [ ] **Step 2:** Commit.

## Self-Review

- **Spec "Nap" section:**
  - consent cards → Task 4;
  - Payments list with Top up and Cancel → Task 5;
  - notice → Task 5;
  - Nap Connect methods → Task 4 (`zyn_paySchedules` scoped per site, a privacy tightening).
- **Ledger rules** (minimum amount, clock start, items refused) are surfaced through Task 4's error mapping.
- **Review Focus** tests are placed in Tasks 3, 4 and 5.
- **Names used across tasks:** `payCreateIntent` etc., `scheduleId` etc., `loadSchedules`, `loadPayments`, `scheduleText`, `fundedUntil`, `runsOutSoon`, `parsePayRequest`.
