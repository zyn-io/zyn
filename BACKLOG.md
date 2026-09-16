# Backlog

## NFT privacy from the public — operator-trusted tier (13 September 2026)

Selected in DECISIONS §70; **planned, not implemented or verified live**.
Operators/authorized replay verifiers remain trusted with plaintext. Counterparty
disclosure, public listings, timing and historical exposure remain explicit
limits. Aliases and operator-private zero-knowledge transfers are deferred.
Do not restrict live DA or advertise private NFT ownership merely because
account RPCs require signatures or snapshots contain hashes.

| ID | Work / GitHub issue | State and dependencies | How to check before closing |
|---|---|---|---|
| P0 | **Disclosure audit and threat model** — [#21](https://github.com/zyn-io/nap-wallet/issues/21) | 🔒 planned; first | Reviewed artifact/reader/metadata matrix; synthetic mint/transfer/Reblind fixture reproduces journal exposure; read-only deployment audit distinguishes code capability from actual access; corrected privacy claims |
| P1 | **Public commitments versus restricted replay data, preserving verification and recovery** — [#22](https://github.com/zyn-io/nap-wallet/issues/22) | 🔒 direction; mechanism open; depends on P0 | Unauthenticated readers cannot retrieve protected journals/base states/records/blinds; authorized verifiers replay identical roots and reject tampering; holder proof export/restore and operator-outage/exit drills pass; reduced unrestricted public replay is explicitly documented |
| P2 | **Recoverable fresh receiving accounts and confirmed privacy blinding** — [#23](https://github.com/zyn-io/nap-wallet/issues/23) | 🔒 planned; depends on P1 and Nap #15/#16 | Create/receive/blind/rotate/backup/clear/restore recovers exact fixture accounts and items; initialization ordering is tested; pending or failed Reblind never enables a privacy badge; old authorities remain recoverable |
| P3 | **Secondary disclosure controls: listings, claims, diagnostics and artwork** — [#24](https://github.com/zyn-io/nap-wallet/issues/24) | 🔒 planned; depends on P0; integrates existing #6/#8/#13/#16/#17 | Network/log/public-export inspection of receive/view/claim/list/sell fixtures finds no involuntary holder map; explicit listing opt-in states its disclosure; unauthorized claim enumeration fails; existing #17 media-privacy checks pass |
| P4 | **Integrated privacy/recovery release gate and future-epoch rollout** — [#25](https://github.com/zyn-io/nap-wallet/issues/25) | 🔒 planned; blocked by P0–P3 | Public-observer leakage tests plus authorized replay, quorum/backing, holder recovery, outage/exit and revocation tests pass; disposable-testnet rollout/rollback evidence and chain/epoch cutoff recorded; no retroactive privacy claim |

Order: P0 → P1 → P2; P3 can proceed after P0; P4 gates release of the combined
result. These are required acceptance tests to implement, **not tests already
passing**. Full issue scopes and the public decision copy are in
`docs/NFT-PUBLIC-PRIVACY.md`. Existing media and claim issues retain ownership of
their implementation; P3 coordinates their disclosure boundary.

## Nap #15/#16 release gates — 12 September 2026

Implementation is on nap-wallet PR #19; both issues remain open. Remaining:
verified resumable legacy migration, full NFT restore/export drill,
#13 claim coordinator/lifecycle integration, and current
mobile Safari/Chrome acceptance. See `docs/NAP-RELEASE.md` for implemented work,
test evidence and the detailed backlog. Do not use this build for funded gifts.

**Every item carries how to check it.** Memory drifts and end-of-session summaries
flatter; a command that either prints the expected thing or does not, does not. If
a status here disagrees with its check, the check is right.

Audited against `DECISIONS.md` (§1–53), `SPEC.md`, `RUNBOOK.md` and the specs under
`docs/superpowers/` on **11 Sep 2026**.

`DECISIONS.md` now stands at **244 ✅ · 11 🔨 · 23 🔒 · 5 ❓**.

---

## Milestones: the rung ladder

Zyn's own framing for decentralising the sequencer (§32–34). This is the roadmap;
everything else is detail hanging off it.

| Rung | What it means | State |
|---|---|---|
| **0** | Anchors on Zcash, DA off the box, an independent verifying replica | ✅ §32 — anchors confirmed on Zcash, DA mirrored, replicas reproduce roots |
| **1** | Forced inclusion — the sequencer cannot silently exclude you | ✅ §33, §44 — detection proven 7 Sep; **delivery proven 10 Sep**, unaided, after fixing the range gap that had made it impossible |
| **2** | k-of-n signers: a deposit releases only once k replicas reproduce its root | ✅ §34, §36 — 2-of-3 live on three machines since 9 Sep |
| **3** | Sequencers that verify before signing; standby sequencer from a replica's state | 🔒 §32.8, §33.6 — not started |
| **4** | Proof-carrying settlement — a verifier checks a proof instead of re-running the VM | 🔒 — explicitly out of scope for now (`signer-federation-design.md`) |

**We are at rung 2, with rungs 0–2 all proven in both directions.** Rung 3
(verify-before-signing sequencers, a standby from a replica's state) is the next
milestone and is not started.

---

## Verified working

| # | Item | How to check |
|---|---|---|
| V1 | Escrowed offers: `PlaceOffer`/`TakeOffer`/`CancelOffer`, VM → RPC → client → HTTP (§39) | `cargo test --workspace` → 685; `cargo test -p swapvm offer_tests` → 10 |
| V2 | Offers reachable end-to-end over a socket | `cargo test -p zynzapd --test demo` → 2 |
| V3 | `OP_ANCHORS` (22), `/api/anchors`, `settlement` in overview (§40) | `cargo test -p zynzapd anchors_map` |
| V4 | Collection lifecycle over HTTP | `grep -c '("POST", "/api/' zynzapd/src/app.rs` |
| V5 | Three signer keys on three machines (§36.6, closed 9 Sep) | signer2 `disabled` on Windows; running on the Air |
| V6 | Alerts reach Telegram from all four hosts (§41) | `grep -c ZYN_ALERT_URL` per host |
| V7 | `zyn-tgbot`: chain, signers, anchors, offers, zcash nodes | `systemctl is-active zyn-tgbot` on Hetzner |
| V8 | Censorship alerts reflect current state, not the permanent record (§41.3) | `grep CENSORSHIP_CURRENT zynzapd/src/replica.rs` |
| V9 | WSL + 280 GB Zebra state on the SanDisk | BasePath `\\?\E:\wsl\zyn` |
| V10 | Mainnet vault DKG'd, sequencer holds no share (§42) | `zyn-dkg address <dir> mainnet` on all three — must agree |
| V11 | Mainnet Zebra reached tip (§38.6, closed) | `/nodes` in the bot; lag ≤ 2 |
| V12 | **signer1 reads its own Zebra, not the sequencer's** (§36.4, half) | log line: `Zebra at 127.0.0.1:18242` |
| V13 | PR #216 open on ZecHub/zec-bounties | `gh pr view 216 --repo ZecHub/zec-bounties` |
| V14 | **Nap agent mode, locally verified; not deployed** (§51, AG0): swap-only default, signed asset/pool/amount/slippage/epoch constraints, committed verification in sequencer/replica/proof paths, delegated RPC, persisted Nap sessions, idempotent actions, restricted `nap-agent` CLI and human mandate UI | `cargo test -p zyn-vm a_default_session_may_trade_and_nothing_else`; `cargo test -p swapvm an_agent_policy_binds_assets_pools_amount_and_slippage`; `cargo test -p swapvm the_guest_reverifies_a_delegated_action_from_committed_bytes`; `cargo test -p zynzapd delegated_submit_allows_swaps_and_refuses_transfers`; `cargo test -p zynzapd --bin nap-wallet the_agent_listener_has_no_wallet_or_mandate_creation_escape_hatch`; `cargo test -p zynzapd --bin nap-agent swap_arguments_are_structured_and_do_not_include_an_account` |
| V15 | **Nap runtime fonts are self-hosted** (§51.9): the daemon, Tauri bundle, extension popup and approval window load only packaged OFL font files; Tauri's CSP permits no Google font origin | `cargo test -p zynzapd --bin nap-wallet the_wallet_ui_loads_fonts_only_from_its_own_origin`; rebuild from each app directory with `node build-ui.mjs` and `node build.mjs`; `rg 'fonts\.g[a-z]+\.com' zynzapd/ui/index.html apps/nap/dist/index.html apps/extension/dist/popup.html apps/extension/dist/confirm.css apps/nap/src-tauri/tauri.conf.json` must print nothing |
| V16 | **Nap recovery is BIP-39/ZIP-32 and portable** (§56): new wallets use 24 words; phrase, passphrase and account restore deterministically on both networks; a versioned JSON backup carries the birthday and network; legacy 32-byte wallets still open | `cargo test -p zynzapd recovery_tests`; `cargo check -p zynzapd --all-targets`; `cmp -s zynzapd/ui/index.html apps/nap/dist/index.html`; `rg -q 'download-backup' apps/extension/dist/popup.js` |
| V17 | **Nap can receive, review and send NFT items from the Items tab** (§65; public `main` `ddafa6616f21`): receive exposes the exact Zyn account id as QR + copy; review shows the committed identity and collection backing; send fixes the amount to one whole item, validates the account id, names the consequence, and retains forced inclusion as an explicit recovery path | `cargo test -p zynzapd --bin nap-wallet`; `cargo test -p swapvm --test chain an_unwrapped_asset_cannot_be_fractionalised -- --exact`; `cargo test -p swapvm --test chain an_item_cannot_be_ended_by_a_partial_holder -- --exact`; `sed -n '/<script>/,/<\/script>/p' zynzapd/ui/index.html \| sed '1d;$d' \| node --check -`; `(cd apps/nap && npm run ui)`; `cmp -s zynzapd/ui/index.html apps/nap/dist/index.html` |

## Open agent work

| # | Item | State | How to check |
|---|---|---|---|
| AG0 | **Deploy the v8 / `ZYNAUTH1` format to chain 11 verifier-first** (§51.8): signer1, then signer3, then the sequencer. Do not let the sequencer emit v8 state until both live signers accept it | 🔨 code ready; deployed marker is 0 on the sequencer, 0 on signer1, and reported 0 on signer3 | Signer1: `strings ~/.zyn/bin/zyn-replica \| awk '$0 ~ /ZYNAUTH1/ {n++} END {print n+0}'` → at least 1. Signer3: use `systemctl show -p ExecStart --value <signer3-service>` inside WSL, then run the same marker check on that path. Sequencer last: check its active `zynzapd` path the same way. After each verifier restart, its verified epoch must catch the current anchored epoch before proceeding |
| AG1 | Dedicated agent accounts with an owner-controlled close path, so the funded balance is a hard cumulative loss bound (§51.7) | 🔒 settled, not built | `cargo test --workspace -- --list | rg -q 'dedicated_agent_account_caps_total_loss: test$' && cargo test --workspace dedicated_agent_account_caps_total_loss` |
| AG2 | Advisor approval inbox: an agent can submit an exact draft for human approval without receiving autonomous authority | 🔒 designed, not built | `cargo test --workspace -- --list | rg -q 'advisor_draft_requires_human_approval: test$' && cargo test --workspace advisor_draft_requires_human_approval` |
| AG3 | Full action lifecycle from accepted intent through seal and Zcash anchor, exposed through `nap-agent` with notifications | 🔒 designed, not built | `cargo test --workspace -- --list | rg -q 'agent_action_reaches_anchored_lifecycle: test$' && cargo test --workspace agent_action_reaches_anchored_lifecycle` |
| AG4 | Stateful per-mandate cumulative accounting and immediate on-chain revocation, if dedicated accounts prove insufficient | ❓ deferred design choice | `cargo test --workspace -- --list | rg -q 'revoked_mandate_is_rejected_during_reexecution: test$' && cargo test --workspace revoked_mandate_is_rejected_during_reexecution` |

## Frontends

Audited 12 Sep. ZynZap is now a standalone hosted product surface; it still
routes every read and write through the visitor's Nap Wallet and does not open
the private Zyn RPC.

**Phase-1 scope changed 13 Sep (§69):** after Cave NFT secondary trading, the
next growth surface is Cave's native-token bonding-curve launchpad and atomic
graduation into ZynZap. Production SOL.zy and EVM/BOLD bridges are deferred;
their test infrastructure stays maintained, and EVM/BOLD returns before Phase 2.

| # | Item | State | How to check |
|---|---|---|---|
| F1 | ~~**ZynZap has no standalone site.**~~ **CLOSED 12 Sep.** The same real swap / earn / bridge app is live at `zynzap.pages.dev`, built from `apps/zynzap`, while remaining `include_str!`'d into `nap-wallet` for same-origin development | ✅ live | `curl -fsS https://zynzap.pages.dev/ \| grep -c 'MAINNET_CHAIN = 26460'` → 1; `cd apps/zynzap && npm test && npm run build` |
| F2 | **Cave's NFT market and NFT creator launch surface are built, but cannot transact.** The repo now has Collection Z, Market, Launch and Portfolio views. `#launch` configures fixed, weighted or buyer-selected quantities independently from identity assignment, previews capacity and simulates weighted outcomes locally. No wallet, deposit, allocation or publish call exists, and the deployed Pages build intentionally remains unchanged until requested | 🔨 frontend built, backend unwired | `cd apps/cave-market && npm test && npm run build`; `rg -c 'fetch\(|window\.zyn' app.js` → 0 |
| F3 | **zynscan is unpublished.** Built and running locally (§48); no host, no domain. The Gate 1 "nice to have" that makes the settlement claim checkable by a voter | 🔨 built, unpublished | `curl -s -o /dev/null -w '%{http_code}' https://scan.<domain>/` |
| F4 | **A hosted ZynZap needs nothing public.** The extension injects `window.zyn` with an EIP-1193 `request({method,params})`, so a static site signs and reads through the visitor's own wallet — the MetaMask model. This decouples shipping ZynZap from the public-`zyn-rpc` decision entirely | ✅ mechanism exists | `grep -c 'isZynZap' apps/extension/src/inpage.js` → 1 |
| F5 | ~~**What a visitor without the extension sees.**~~ **CLOSED 12 Sep.** The complete Bridge / Swap / Earn product stays navigable in read-only preview, beneath a compact setup panel explaining Nap, the 12-confirmation bridge path and the exact mainnet/chain boundary. Only balances, quotes and actions wait for a wallet. It deliberately shows no invented live data and makes no public RPC request | ✅ | Open `zynzap.pages.dev` without the extension and switch all three tabs; `curl -fsSI https://zynzap.pages.dev/ \| grep -i "content-security-policy.*connect-src 'none'"` |
| F6 | ~~**NFT custody was visible but not actionable.**~~ **CLOSED + PUBLISHED 12 Sep in PR #9 (`ddafa6616f21`).** Items now pairs the private holding review with a receive account QR/copy surface and a selected-item send flow. The send has no editable amount, reviews one whole item and the full destination, and does not pretend a content hash is remotely hosted artwork (§65) | ✅ public | Open Items in an isolated Nap instance; verify one QR and a 64-character `it-account` value; select a funded item and review “Send this NFT”; run the V17 checks above |
| F7 | **Cave's fungible launchpad has an end-to-end product path and a standalone Jade deployment.** The VM has permissionless persistent curves, buy/sell, exact quotes, fee accounting and atomic locked-LP graduation into the canonical ZynZap pool. The dark-jade gallery root, `/launch` and immutable `/coin/<asset-address>` routes use committed state through Nap Wallet; all writes are approval-gated and `zynscan` exposes launch evidence. The independent `cave-jade.pages.dev` Pages project is live. `launch.cavebro.win` is registered on that project but its DNS/hostname activation is an operator step. Receipt-derived history and funded v4/two-wallet rehearsal remain. Nap Mole ownership is not a gate; its multiplier belongs to the separate credit system | 🔨 live product path; DNS/history/rehearsal open | `cargo test -p swapvm --test cave_launch`; `cargo test -p zynzapd cave_gallery_and_quote_are_public_and_exact`; `cd apps/cave-market && npm test && npm run build:launchpad`; `curl -fsSI https://cave-jade.pages.dev/` |

---

## Critical path to 2 October

| # | Item | State | Next action |
|---|---|---|---|
| O1 | ~~Rung 1~~ **CLOSED 10 Sep** | Fix deployed and verified: a forced intent landed through the normal sweep, unaided (§44.9) | — |
| O2 | ~~Point the mainnet daemon at the DKG'd vault~~ **DONE 11 Sep** — chain **26460** live on Contabo, watching the DKG'd vault, seq 0 epoch 0 | `zyn-cli status 127.0.0.1:8099 26460` on Contabo |
| O2b | ~~Wire custody + DA mirrors~~ **DONE 11 Sep** — 2-of-3 remote custody, DA mirrored to Hetzner (§54) | `journalctl -u zynzapd-mainnet | grep 'anchors ON ZCASH'`; `curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8181/chain-26460/base.state` on Hetzner → 200 |
| O2d | **Offline backup of each mainnet share** (§55.7) — share 01 exists only on Hetzner; a lost VPS costs it permanently | Before the vault is funded |
| O2e | Convert the Air's LaunchAgents to LaunchDaemons so custody survives a reboot with nobody logged in (§56.4) | Needs sudo at the keyboard |
| O2f | **Rotate signer2's endorsement key (chain 11, set slot 2).** The seed was printed into an assistant transcript on 11 Sep while reading its plist. Testnet endorsement key only — not a spending key, and mainnet 26460 has a separate set — but with threshold 2 it is half a forged certificate. Rotate the seed, update `ZYN_SIGNER_SET` slot 2 on the sequencer, restart both | Blocked on approval; signer2 currently runs the old key |
| O2c-done | ~~Wire share 02 (the Air)~~ **DONE 11 Sep** — `anchors signed by 3 custodian(s), threshold 2` (§56) | `journalctl -u zynzapd-mainnet \| grep 'signed by 3 custodian'` |
| O2c-old | **Wire share 02 (the Air) as a third mainnet custodian** (§54.3) — 2 of 3 with only two reachable means either one going down stops anchoring | Air was unreachable by key on 11 Sep; needs `ssh-copy-id` then a custodian + tunnel like the other two |
| O2-old | Point the mainnet daemon at the DKG'd vault | Verified `grep -c u1kc38pww /etc/zyn/zynzapd.env` → **0** | Cut over when ready; nothing has funded it |
| O3 | Storefront live integration | Spec written and reviewed | Colleague: dual-daemon reads, then offer/take |
| O4 | **Second independent Zebra → close §36.4/§36.5** | signer1 done; **Air's testnet Zebra syncing since 10 Sep 11:10** (from height 0, a few hours) | When synced, point signer2 at `127.0.0.1:18242` on the Air |
| O5 | Strict deposit verification (§37) | `ZcashBacking::strict()` written, unused | Depends on O4 |
| O6 | Film the demo | Blocked on O1, O3 | — |

### O1 evidence — the forced-inclusion experiment, 10 Sep 2026

A real forced intent was sent end to end and **was never applied**. What is
established, so the next person does not repeat it:

| Fact | Value |
|---|---|
| Intent | `Transfer` 0.0001 xZEC to account `0202…02`, signed by `ea329c6c…` |
| Sent via | `POST /api/force` on `nap-wallet` — builds the frame, wraps it in a `ZYF` memo, pays 10,000 zat to the vault |
| Zcash txid | `1c7ad023a9be14e5ee5fb06b00b2a8904cc7a205be7a15b127447a0f646be0d7` |
| Confirmed | height **4,337,728**, **12 confirmations** (6 required) |
| Sequencer scanned to | **4,337,734** — past it, reporting `ok` |
| Result | **nothing.** No deposit, no forced sighting, no `Unattributable` error, `seq` unchanged at 11548, `accounts` unchanged at 10 |

**Ruled out**

- The memo layer: `cargo test -p zyn-custody memo` passes, including
  `a_forced_memo_is_neither_a_deposit_nor_an_anchor_and_round_trips_its_frame`
- `ZYN_IGNORE_FORCED`: not set on the sequencer
- The call path: `bridge.rs:456` does call `apply_forced` from the observe loop
- Depth: 12 confirmations against a requirement of 6
- Signers: unrelated — both endorse normally and certificates clear

**Where it must be.** The scanner produced no `ForcedSighting`, so execution never
reached `apply_forced`. In `shielded.rs:250` the sighting requires *both*
`memo::forced_frame(&memo_bytes)` **and** `memo::forced_account(frame)` to return
`Some`, on a note that decrypted as ours. One of those three is failing silently —
and silently is the problem: every other branch there logs or raises.

**Narrowed further, 10 Sep.** A test now drives a *real* submission frame through
the memo round trip — `client.rs::forced_frame_tests` — and it **passes**: the frame
survives `encode_forced` → `forced_frame` unchanged, and `forced_account` recovers
the signer. So the frame layout, the memo wrapper and the signer recovery are all
sound. That leaves exactly two candidates:

1. the note did not decrypt as the vault's (wrong receiver, or wrong pool — the
   sending wallet held **Ironwood** only: `ironwood: 90000, orchard: 0`), or
2. the scanner's range never covered that block in a pass where a sighting could be
   made.

**In flight.** `ZYN_FORCED_RESCAN_FROM=4337720` is set on the sequencer, which makes
`rescan_forced` re-read the range on boot and log how many it queued. Its answer
decides between the two: a non-zero count means the scanner *can* see the memo and
the live pass missed it; zero means the note is not being recognised as the vault's
at all. **Remove that variable once the answer is in** — it re-reads on every boot.

**Why it matters twice.** Rung 1 has only ever been demonstrated *backwards* — the
7 Sep run proved a replica notices censorship. Nothing has ever proved the happy
path. A censorship-resistance mechanism that detects censorship but cannot deliver
an intent is half a mechanism, and it is beat 5 of the demo.

## After 2 October

| # | Item | Source |
|---|---|---|
| A1 | **Custodian policy verification** — a custodian signs a bare 32-byte sighash (`custody_net.rs:171`) and cannot tell an anchor from a payout. Largest known security gap | this audit |
| A2 | **Finish the sale service behind the creator UI.** Built in `zyn-io/cave`: verified allocation; atomically persisted reservations/receipts; strict `ZYC` entry/claim memo codecs; a canonical Zcash payment view excluded from ordinary buyer bridge credit; and the entry settlement state machine. Each accepted entry credits its exact ZEC.zy once to Treasury 26460, waits for anchor finality, funds 90% into the collection, retains 10% in Treasury, verifies VM receipts and gates mint plans on funding. Core Zyn mints the explicit receipt serial while replaying historical tag 31 unchanged. Remaining, in order: canonical `cave-sale-terms/v1` bytes and migration of entry/ticket references from the allocation-policy digest; external publication/timestamp adapter; continuous Zcash/drand service; public ledger/receipt HTTP; Treasury 26460 automated signer; claim-payment admission/settlement; open-mint settlement; explicit team-reserve and chain-height close parameters; and storefront wiring | 🔨 entry money rail + engine + durable ledger + VM boundary built; complete sale commitment/service/signing/product open; §64; `docs/2026-09-09-genesis-collection.md` |
| A3 | ~~zynscan, served from a **replica**~~ **BUILT 10 Sep** — anchor index over HTTP, reads a replica by default (§48). Backfill of ~170 stale `height 0` entries still owed | §48 |
| A4 | Rung 3: verify-before-signing sequencers; standby from a replica's `Saved` | §32.8, §33.6 |
| A5 | **DEFERRED from Phase 1:** Solana custody still has 2-of-3 shares on the sequencer's box. Keep devnet regression coverage; do not promote SOL.zy to production until user demand returns it to scope | §36.7; §69.1 |
| A6 | Storage for 4,444 × 35 KB (154 MB); on-chain is a 32-byte hash per item | `COLLECTION-SPEC.md` |
| A7 | ZCG grant (rolling). Forum post is part of the process — post first, apply after | this audit |
| A8 | **DEFERRED bridge expansion:** BTC via Taproot (`frost-secp256k1`) and production EVM deployment are outside Phase 1. EVM/BOLD must return before Phase 2; **ETH is not a FROST target** | this audit; §69.1 |
| A9 | Ops panel | lower priority than alerts, which now work |
| A10 | **Split DA publication from anchoring** (§43.5) — publish intents + snapshot continuously, the Zcash witness when available. Today an outage stops verification entirely, and the unaudited window is unbounded | §43 |
| A11 | Bound the unanchored window: alert past N epochs, hard cap refusing new intents while queued exits still settle (§43.7) | §43 |
| A13 | ~~Raise `ZYN_CONFIRMATIONS`, re-check confirmed anchors, refuse to extend a broken lineage~~ **DONE 11 Sep** — depth 12, `reorged()` each pass, `send_next` gated, folded into the `anchors` alert (§50.10–12) | §50 |
| A17 | **Re-basing a signer forfeits its vote on everything older.** Done 11 Sep: signer3 re-based to epoch 5464 to clear the halt, and can no longer endorse the pending anchor at 5397. With threshold 2 of 3 and signer2 stale at epoch 2248, that anchor is stuck at **1 endorsement** and its deposits cannot become spendable. Either bring a second signer current past the gap (needs A16), or lower the threshold deliberately. **Re-basing is not free — it costs the signer's vote on all prior history** | §50 |
| A20 | ~~Re-offer endorsements until the certificate clears~~ **BUILT + DEPLOYED 12 Sep** (§63) — signers re-send every pass until the sequencer confirms threshold, and seed the list at startup with their head anchor. Recovered chain 11 at 08:55 UTC after 2810 passes; epoch 5397 settled and the chain resumed at 6692 | §63 |
| A24 | ~~Define the address standard and fix its three gaps~~ **DONE 12 Sep** (§66, `docs/2026-09-12-address-standard.md`) — fixed `derive_for` so the VM version is actually hashed; added `native_address`/`bridged_address`/`asset_address`/`collection_address`/`item_address` and non-generic `program_scope_of`; bridged assets now key on origin *network* (`zcash` vs `zcash-test`) and a chain's own token carries its chain id. Genesis table pinned by assertion. 8 new tests, 733 passing | §66 |
| A39 | ~~Decide the 2,100,000 ZYN remainder~~ **DESTINATION SETTLED 13 Sep** (§71.9) — it is the ZYN side of the opening ZYN:ZEC.zy market, paired against accrued ZEC.zy. **Still open: the opening price.** §68.7's `0.0002 ZEC` is the Phase-2 wind-down trigger, so opening there fires it at launch; opening at `0.002` launches ten times past it. Needs two numbers, threshold above opening | Open; blocks any ZYN market |
| A42 | **Bound the reserve release rate against pool depth** (§71.6). Nothing circulates at open (§71.10), so this binds from the *first reserve release*, not launch. The pool moves 8.9% on a 100,000-ZYN sell with 18.9M queued behind it; unlock rules need a depth constraint, not only an activity test | Open; §71 |
| A43 | ~~Name ZYN's utility~~ **FOLDED INTO A38** — the credit system with a Nap Mole multiplier is already tracked there. Recorded here only as the dependency: nobody receives ZYN today (§71.13), and **A40's release rules cannot be written until A38 defines what earns credits** | Merged into A38 |
| A44 | ~~Implement §69.9's NFT secondary split~~ **DONE 14 Sep** — `charge_collection_fee` now routes `40%` to the collection floor, `10%` to the creator and the remainder to `ZYN_POL`, so rounding dust favours protocol liquidity rather than the creator (§69.27's principle) | §69.9 |
| A50 | ~~Wallet sent numeric asset ids after A25~~ **DONE 14 Sep** — the page coerced with `+`/`Number()`, giving `NaN` and a JSON `null`, so the manual Zyn transfer, the agent mandate asset field and `ids()` for allowed assets/pools were all dead. They fail closed, so nothing was at risk, but nothing worked either. Now validated as 64-hex and the item detail shows a short id (§66.15) | §66.9 |
| A51 | ~~Cave store shipped with no security headers~~ **DONE 14 Sep** — `scripts/build.mjs` emitted no `_headers` while the launchpad build emitted a full policy, so the deployed store had no CSP, no `frame-ancestors`, no referrer policy. `jade-headers` is now `headers`, both builds copy it, and a test asserts both emit it. §63.2's `connect-src 'none'` guarantee is worth nothing if the header never reaches the deploy | §63.2 |
| A52 | **Wire `cave-market/app.js` to `window.zyn`.** `jade.js` already uses the provider; the store page has zero provider calls and renders nothing live. Needs a collection on chain to show, so it follows Treasury 26460 | Open; after Treasury |
| A53 | **Set `exit_timeout_epochs` for mainnet against worst-case settlement** (§77.7). `may_cancel` checks elapsed epochs only and cannot see an in-flight payout, so the deadline is the sole defence against a user cancelling while the ZEC leaves the vault. Needs a number that exceeds settlement + confirmations + the longest unattended outage; the ~34-hour posture in the code comment is generous against continuous settlement but was never fixed for mainnet | Open; §77.7 |
| A54 | **Decide who absorbs the withdrawal payout fee** (§77.8). `0.0001` ZEC per exit, currently unaccounted — §69.8/§69.9 route trading fees but not this. Protocol-absorbed is the obvious answer at four cents; the point is to write it down rather than leave it implicit | Open; §77.8 |
| A55 | **Submit the extension unlisted to the Chrome Web Store** (§78). Needs a $5 developer registration, a privacy-policy URL (required once `storage` is declared), 128px icon and a screenshot. Narrow `content_scripts.matches` from `<all_urls>` to our own hosts first (§78.4) — it removes the broad-host-permission justification and can widen later. Firefox AMO and Edge take the same MV3 package | Open; §78 |
| A56 | **Split Cave's launch pots per purpose** (§66.14, §79.6). Today one `launch_vault(asset)` holds refundable curve principal, the pair seed, the graduation pot and creator escrow together, so no auditor can tell which is which and no invariant can say the principal was never touched. Each needs its own `pot_address(purpose, [asset])` | Open; §79.6 |
| A57 | ~~Provision Treasury 26460~~ **DONE 14 Sep** (§83) — `b3e4c493…fe99`, 2-of-3, provisioned by the vault's own ceremony over Ed25519 across Mac, Air and the Windows box. All three agreed; one share each; relay held none. Pinned as `TREASURY_26460` with a test against the ceremony's output | §83 |
| A58 | **Mainnet genesis run.** Blocked only on decisions: Treasury ceremony (one command per custodian — the Air is not reachable from here), two collection symbols, `fee_bps` for the 333 (Collection Z is 100), and the per-item symbol. Then: `/api/topup` the vault (§82), 335 intents, one anchor, stop, back up `/var/lib/zyn/mainnet` | Open; needs 5 decisions |
| A59 | **Fund the mainnet vault so anchoring can start.** Chain 26460 is sealed at epoch 0 and stopped; its anchor pass fails with "waiting for a spendable Ironwood note" purely because the vault has never been paid. One ZEC transfer to the vault, restart `zynzapd-mainnet` (it is `disable`d — re-`enable` it), and the genesis root reaches Zcash. |
| A60 | **`reject_name` drifted 15 codes behind the VM.** Fixed: every collection and offer rejection (36-46, and 15-17, 32) reached the wallet as the bare word "rejected", which is the whole NFT surface Nap Wallet and Cave use. A test now reads `wire.rs` and fails if any code lacks a sentence, so it cannot drift again. |
| A61 | **Split the 18.9M reserve into named purpose pots** when the unlock rules exist (§71.21). One pot today because no allocation is designed and §71.13 owes nobody anything. §71.4 permits the later split and §71.3's numbers do not move when ZYN goes pot to pot. Blocked on Phase-2 design, not on code. |
| A62 | **Give the reserve a release path in the Phase-2 chain version** (§71.22). Today nothing can leave `ZYN_RESERVE`, which is correct for Phase 1 and useless afterwards. §71.16 already makes Phase 2 a `STATE_VERSION` change, so the rules ship with it — but they must be designed before that version is cut, or the reserve is stranded again. |
| A63 | **Surface the supply policy in Cave and the extension**, not only the desktop wallet and JSON. §71.18's whole point is that a holder reads the policy where they make the decision. The launchpad and the store both show tokens and neither shows this yet. |
| A64 | ~~Build the Phase-2 open~~ **BUILT 15 Sep** — arms when the pot fills, opens 8,064 Zcash blocks later at exactly 2,100,000 : 420, LP to the treasury (§71.29-71.32). What remains of Phase 2 is separate and still undesigned: the fee rail coming down and ZYN gas (§71.16). |
| A67 | ~~Show the countdown in the wallet~~ **BUILT 15 Sep** — `/api/overview` carries a `market` block and the Zyn tab renders all three states: progress toward the date while the pot fills, a countdown in blocks and days once it is set, and the opening price after. |
| A68 | **Show the countdown in Cave and zynscan too.** The store reaches the chain only through the extension (§63.2), so this waits on A52 wiring `cave-market/app.js` to `window.zyn`. zynscan reads a replica and can show it today. |
| A69 | **Cloudflare bot protection 403s non-browser user agents** on `cavebro.win/blobs/*`. A browser and `curl` are fine; `Python-urllib` is refused. Anything scripted that fetches art — a CLI, a checker, an indexer — needs a real UA or the blobs look missing. Worth a WAF exception for `/blobs/*`, which is public immutable content with nothing to protect. |
| A70 | ~~Prove the deposit path against real Zcash~~ **DONE 16 Sep** — 0.1 TAZ sent to the testnet vault, seen at height 4354607, credited, and chain 11 anchored to Zcash for the first time since the reorg (`e6cd02ed…` at epoch 0). It also found the thing tests could not: a deposit with no memo is not credited at all. Originally: `deposits.rs` covers it end to end — JSON on a socket to a credited balance, 5 tests — but against a *fake* Zebra, and in 30 days of live testnet no deposit has ever been credited. Send TAZ to the testnet vault and watch it credit before any whitelist is announced: first contact with real Zcash should not happen under launch load. |
| A71 | **Build the Cave sale service** — the piece that joins whitelist, payment and `cave_randomness::allocate` into a mint. The registry (A-this) and the allocator exist; nothing wires them to a paid `MintCollectionItem`. This is the last large unbuilt piece for the 4444. |
| A72 | **Build the 4444's Zyn manifests.** Proven possible: the 333's `build_manifests.py` runs against `examples/zyn-pfp-concepts/*-collection-*` with a ~20-line adapter — the hash lives in `checksums.sha256` rather than inline, and integer attribute values need stringifying. Produced 2,222 manifests / 4,444 blobs / 77.8 MB for the moles; all three species land near 8,888 blobs and ~155 MB. |
| A73 | ~~Fund the testnet vault~~ **DONE 16 Sep** — funded, attributed by hand, anchoring. Originally: The fresh chain is up, serving, publishing DA and endorsed by both signers, but every anchor is refused: `ironwood double-spend, in finalized state: true` — the note it picks is already spent. A small TAZ transfer creates an unambiguous unspent note and, in the same act, proves the deposit path against real Zcash (A70). |
| A74 | **`ZYN_BRIDGE_FROM_HEIGHT` should default forward on a chain reset.** Nothing warns that a replica pointed below the previous incarnation's last anchor will find those anchors on Zcash and halt demanding bundles that never existed (§71.37). Either refuse to start when the scan start predates the base state, or say so loudly. |
| A75 | **A deposit with no memo is not credited — it waits for a human.** The node requires `ZYN1:<32-byte account, hex>`; without it the funds reach the vault and sit unattributed until someone adds a line to `/etc/zyn/zcash-attributions`. A faucet will never set that memo, and neither will most exchanges. Before Cave's paid mint opens, decide what a buyer sending from an exchange gets: a per-account address that needs no memo, a claim flow that attributes by amount and timing, or an explicit "memo required" that the checkout enforces. |
| A76 | **Say the memo requirement everywhere a deposit address is shown.** Nap Wallet's deposit panel, cavebro.win, and any doc that prints a vault address. Handing someone a bare address is handing them a deposit that will not credit. |
| A65 | **Decide whether post-open buybacks are automatic or discretionary** (§71.25). Automatic on accrual is consistent with §71.5 and needs no key; discretionary needs the treasury and is therefore a market operation someone performs. Affects whether the ZYN price path is a protocol property or a decision. |
| A66 | **Disclose that ZYN market liquidity is treasury-withdrawable** (§71.27) wherever the trust claim appears — the Colosseum copy, the site, the docs. The claim is true of deposits, mint proceeds and the reserve; it is not true of the ZYN pool, and a reader who checks will find that out. Better said than discovered. |
| A46 | ~~`ADDRESS_SCOPE_V1` declared twice~~ **DONE 14 Sep** — `swapvm` re-exports `zyn_vm::derive::ADDRESS_SCOPE_V1` instead of redeclaring the literal. Two copies of a frozen value are two things that can drift, and §66.10 exists so this one cannot | §66.10 |
| A47 | ~~Duplicate decision and correction ids~~ **DONE 14 Sep** (nap-wallet#18) — `§63`/`§64` each named two sections and `C17`/`C18`/`C19` each named two corrections. The endorsement and replica-base sections moved to **§72/§73** (the Cave sections are newer, larger and actively built against), and the later corrections to **C22–C24**. References repointed: A21 to §72.6, and the font-leak citations in §67.6 and the wallet to C23 | nap-wallet#18 |
| A48 | ~~Build the 333 claim service~~ **BUILT 15 Sep** — `zyn-claim issue` prints codes once and keeps only fingerprints; `zyn-claim serve` binds a code to the first account that redeems it, is idempotent on retry and refuses a rebind (§74.8-74.11). Exercised live: claim, retry, rebind refused, unknown code. Originally: (§74.5-74.6). One-time codes bound to serials, recipient supplies the account, idempotent on retry and refused on rebinding, marks a code void once used. The chain side needs nothing (§74.2) — this is a small operator service plus a claim page. `nap-web` in nap-wallet#19 makes the browser route viable, which removes the app-store dependency that motivated printed cards | Open; §74 |
| A49 | **Publish the 333 manifest commitment before any mint** (§74.3). `out/content.json` has all 333 content hashes; the serial-to-content binding must be committed first so a recipient can check which mole they received and nobody can be given the same serial twice | Open; §74 |
| A40 | **Make pot release a committed predicate, not a judgement** (§71.6–71.7). Unlock rules must be evaluated by the VM over chain state, must not be per-trade volume (§69.15), and must be committed before a pot can release. If a person decides, the pots are a treasury and 'keyless' is decoration | Open; §71 |
| A41 | **Publish the three supply numbers from chain state** (§71.3) — `total` 21M constant, `unlocked`, `circulating` — derivable by an explorer without trusting us. They only answer the 90%-reserve criticism if published together with the release rules, from day one | Open; read API |
| A26 | ~~Define where NFT media lives and how a wallet trusts it~~ **DONE 12 Sep** (§67, `zyn-vm/src/manifest.rs`) — `zyn.manifest.v1`: media named by SHA-256 with **no URL**, location a replaceable hint outside consensus, gateway is a hash-keyed blob store, wallet verifies before render with an IndexedDB cache and fetching **off by default**. Size never enters the chain, so GIF and video need no protocol change. 10 tests; Rust, Python and JS pinned to one vector | §67 |
| A27 | ~~Serve the 666 NFT blobs~~ **DONE 15 Sep** — staged by `apps/cave-collection-z/scripts/build.mjs`, live at `https://cavebro.win/blobs/<hash>`: immutable, CORS-open, real 404 on a hash that is not there. All 333 items' media and manifests verified over the live domain. Originally: `build_manifests.py` emits `out/blobs/<sha256>` with 333 image objects plus 333 canonical manifest objects (13.6 MB total), ready to upload as immutable hash-named objects, plus `out/content.json` mapping serial to the `content` to mint. Needs: the bucket, a privacy-safe `nap_media_sources` default, independent verification, and a mirror | Open; GitHub issue #17 |
| A25 | **Finish the frozen address-standard rollout.** Core `AssetId`/`PoolId`/`CollectionId`, UTF-8 symbols, deterministic creation, wire/state/RPC/wallet/bridge paths and published native/bridged/pool vectors are migrated under `ADDRESS_SCOPE_V1 = b"zynzap\x00\x02"`. Bridge creation now takes exact `origin_network` and raw `origin_asset`, reproducing SOL.zy/BOLD.zy. Remaining: move actual pool reserves to `vault(pool)` balances, add Bech32m public rendering, remove developer-only legacy-id fixtures, and rehearse the verifier-first state/wire fork | 🔨 core migrated; storage/presentation/rollout open |
| A22 | **signer3 cannot start from the published base state, cause unresolved.** Its halt at 6692 was genuine (the §A17 re-base left its base root off the lineage). New binary installed and hash-verified; env file correct and proven readable by systemd (a transient unit running `printenv` returned the set value). But `zyn-replica` reports `is not the attested root c8c503e6…` regardless of `ZYN_BASE_ROOT`, including an explicit `--setenv=1111…`. At `replica.rs:249` that printed value *is* the env var, and it is the only such site in the workspace — so one of these measurements is wrong and I have not found which. signer3 left **stopped**; chain runs 2-of-2 on signer1+signer2 | Open, **no longer blocking** — A23 sidesteps it by never leaving the data dir empty. Still worth resolving: any future replica built from scratch hits it |
| A23 | ~~Seed signer3 from signer2's verified state~~ **DONE 12 Sep** — hot-copied signer2's data dir (28 MB, 1.1 MB gzipped, hash-verified) while it kept running, so no signer ever stopped. signer3 resumed at epoch 6821 and its first endorsement cleared a certificate. It signs as `cc158dc4…` (set slot 3) — **only the chain state came from signer2; the signing identity is its own**. Set is 3-of-3 healthy again, threshold 2 with margin | §A22 |
| A21 | **Persist the in-flight anchor's certificate on the sequencer** (§72.6). A20 heals this from the signer side, but the sequencer still loses collected signatures on restart — which also means a restart discards work the set already did. Write partial certificates alongside the anchor and reload them at startup | Open; §63 |
| A19 | ~~Defer a lineage break while the scan is behind the tip~~ **BUILT 11 Sep** (§62) — `apply_sightings_scanned(.., scan_complete)`; the replica holds sightings it cannot yet place instead of latching, and only halts on a break that survives a complete scan. Two tests: one holds and clears when the repair arrives, one still halts on a complete view | §62 |
| A18 | ~~Upgrade signer1 to the §62 binary~~ **DONE 12 Sep** — repointed at `zyn-replica-a16` and restarted; it resumed from its persisted ledger at epoch 5397, re-offered its lost endorsement, and the certificate cleared. The shared `~/.zyn/bin/zyn-replica` was left untouched (§59.4) | §62, §63 |
| A16 | ~~Order sightings by lineage, not height~~ **BUILT 11 Sep** (§61) — `in_lineage_order()` now sorts `(epoch, height)`, extracted as a pure function with three tests including the real 4321/4330/4336/4474 inversion. `previous_root` still validates every step, so the sort decides what is *offered*, never what is *accepted*. Deployed to signer2 as `zyn-replica-a16`; signer1 still runs the old binary and is healthy past the gap | §61 |
| A15 | **Re-anchor the tail** (§50.14, option 2) — re-anchor every anchor after a gap so chain order is restored. Correct and needs no protocol change, but each one waits on the vault's single note, so it is slow and costs a note per anchor. The fallback if A16 is judged too invasive | §50 |
| A14 | ~~Re-anchor a historical anchor~~ **BUILT 11 Sep** — queued repair, retried until the vault has a note; 4330 is back on Zcash at height 4338929 (§50.9). **Does not heal the replica**: it applies anchors in chain order and the repair landed after 4336, so a decision is owed on §50.14 | §50 |
| A12 | ~~Nap Wallet transparent receive~~ **DONE 10 Sep** — derivation, UTXO lookup and t→z sweep, proven end to end, and surfaced in the wallet UI as the "Coming from an exchange?" drawer (§45) | §45 |
| A28 | **Verified NFT media resolution and funded two-wallet QA.** Resolve content only through a privacy-reviewed path, hash the fetched bytes against the committed 32-byte content id before display, and record a live Nap A → Nap B transfer where the item disappears from A and appears in B. The current identicon + commitment is the honest fallback, not a broken image (§65.5) | §65; GitHub issue #8 |
| A29 | **Finish the launchpad state fork.** The 32-byte identifiers, Cave launch records and v4 VM identity are implemented. Current Cave, NFT and graduated-pool POL credits use the pinned derived `zyn-pol` account; the old tokenomics/bridge-fee UI is removed, `ZYN_LAUNCH*` now fails startup, and wallet signatures can no longer authorize the immediate-pool `CreateToken` shortcut. Historical §30 structs, legacy pot and RPC reads remain so old journals/state can be audited and migrated. Still rehearse the v4 state/wire rollout, prove live state has no active legacy launch or balance at the legacy POL placeholder, then remove compatibility state during the planned migration and upgrade replicas/signers before the sequencer | 🔨 activation/current POL migration built; rehearsal/legacy-state migration open; §68; §69.17–18; RUNBOOK §5j |
| A30 | **Harden the built Cave curve state and exact math.** One-billion supply/eight decimals, 660M/340M allocation, exact interval math with division last, curve-favouring atomic rounding, no expiry, reversible trading and the live-curve `genesis_pool` exception are implemented. Boundary vectors, 1,000-step deterministic buy/sell churn and state restart are tested. Remaining: extended fuzzing and separate-account reconciliation if A25's vault-balance migration retains that model | 🔨 core hardened; fuzz/storage follow-up open |
| A31 | **Finish fee-account storage separation.** Built: creation `0.009/0.001`, immutable `50–300 bps`, curve `50/40/10`, NFT `50/40/10`, graduated two-way `45/45/10` ZEC-leg routing and protocol-favouring remainder rules. Current Cave/NFT/graduated-pool POL credits land at pinned `derive(P,"zyn-pol",[ZEC.zy]) = f40435d1…7e61`; receipts expose the fee asset, total fee and all three shares. Current Cave custody is otherwise one derived launch vault with principal/seed/graduation/creator amounts separated by committed fields; pools still keep reserves/lock inline and curve trade receipts omit individual shares. Remaining: migrate existing balance from legacy `[0xF3;32]` if live state contains one, split launch balances into the separately derived accounts in §66.14, and expose reconciliable reads when A25 moves pool balances to derived vaults | 🔨 derived POL/current routing built; storage/read migration open; §66.14; §69.7–9, §69.14, §69.19–20, §69.25–28 |
| A32 | ~~**Build atomic curve graduation on ZynZap primitives.**~~ **BUILT 14 Sep.** The terminal buy sizes both reserves at exact `Pg`, rounds token quantity to its eight-decimal unit, routes cap/rounding overflow to future ZYN POL, installs one canonical no-reference pool, records immutable opening evidence, sets `genesis_pool`, releases creator escrow and locks every initial LP share on scratch state | ✅; `cargo test -p swapvm --test cave_launch` |
| A33 | ~~**Validate the fixed persistent curve before code.**~~ **CORE VECTORS BUILT 14 Sep.** Exact endpoint/reserve vectors, division-last interval math, buy-up/sell-down atomic principal rounding, 1,000-step reversible churn, restart, graduation cap/price continuity and locked-LP evidence are encoded in the VM/integration tests. The cap/overflow branch exists, including atomic rounding residual to POL, but exhaustive forced-failure and synthetic above-cap coverage remains in A30; valid v1 fee parameters normally keep candidate liquidity below 34 ZEC.zy | 🔨 core vectors pass; adversarial coverage open; §69.5–6, §69.10–13, §69.28; `cargo test -p swapvm --test cave_launch` |
| A34 | **Finish launchpad exposure.** Built: three VM intents (graduation is atomic, so no fourth advance intent), typed receipts, Unicode validation, launch rate, capped dev buy, wire/state replay, public list/single-launch/exact-quote RPC, and signed Rust client writes. Cave Jade is the separate no-landing-page artifact: live provider-only gallery root, `/launch`, immutable `/coin/<asset-address>` pages, deterministic curve-position chart, exact curve and graduated-pool quotes, wallet approvals, official-address badges, explicit `zec_per_token`, current reserve and full fee-hop evidence. `zynscan` now publishes reserve, creator/graduation fees, overflow, opening liquidity and locked-LP state. The obsolete hidden Signal UI was removed from the NFT app. The independent `cave-jade.pages.dev` production deployment is live; `launch.cavebro.win` is attached but DNS activation is left to the operator. Open: receipt-derived historical OHLCV under A35, final editable copy and v4/two-wallet rehearsal under A36 | 🔨 product path/deployment built; DNS/projection/rehearsal open |
| A35 | **Build the non-authoritative market projection:** derive launch and graduated-pool trades from accepted receipts into OHLCV keyed by launch/asset/pool; make it replayable and prevent double-counting at graduation. Gallery/chart failure must never block consensus reads, trades, curve sells or exits | §69.23; RUNBOOK §5j |
| A36 | **Staged permissionless launch:** NFT secondary trading first, then two-wallet launchpad testnet rehearsal → public read-only markets → public alpha with the fixed per-creator two-per-60-epoch launch rate and no NFT/manual/value/concurrency gate. Raise general infrastructure capacity only after a long-lived curve and atomic graduation survive restart and replica replay. Check launch-fee routing, dev buy, uncapped buys, curve reserve, both graduated fee directions, creator/POL credits, overflow, price orientation and locked LP | §69.16–34; RUNBOOK §5j |
| A37 | **Implement address-based authenticity and Unicode presentation.** Accept the exact symbol/display-name bounds in §66.11 and §69.29; show short addresses for duplicate/confusable labels; badge only pinned official addresses; and let reports warn or de-index without affecting settlement, curve sells or exits. Provision the threshold `Treasury 26460` AccountId, then regenerate and pin the official Nap Mole and Cave collection/item vectors before minting | §66.16–17; §69.29; RUNBOOK §5j |
| A38 | **Design the separate credit system and Nap Mole multiplier.** Define what earns credits, the multiplier value, ownership snapshot/live-check rule, transfer behavior, caps and Sybil policy. Prove the credit system cannot gate, discount, prioritize or otherwise alter Cave launchpad execution | §69.16, §69.24; outside launchpad consensus |
| A45 | ~~**Finish graduated pools' ZEC-leg fee contract before public writes.**~~ **BUILT 14 Sep.** ZEC.zy → token charges the ZEC input; token → ZEC.zy prices the full token input, computes the fee on gross ZEC output and pays net ZEC. Both directions route `45% pool / 45% POL / 10% creator`, give atomic remainder to POL and expose `fee_asset`, total, pool, creator and POL shares per hop. Exact-in, exact-out and sealed-batch tests reconcile balances/reserves and prove no launched-token fee reaches creator or POL. V3 introduced the receipt contract; current v4 additionally pins the derived POL destination while persistent asset/pool addresses remain frozen | ✅ blocker resolved; §69.14, §69.19; RUNBOOK §5j |

---

## Gate 1 — hackathon submission (2 October, 22 days)

**Whose money is at risk: yours, in dust.** That is what makes this gate short. The
demo can run on 0.001-scale amounts in a vault you control, so most of the
hardening below belongs to Gate 2, not here.

**Must have**

| # | Item | Why it blocks |
|---|---|---|
| G1.1 | **A forced intent applied successfully** (O1) | Demo beat 5, and rung 1's unproven half. Only the failure case has ever been shown |
| G1.2 | Mainnet daemon pointed at the DKG'd vault (O2) | Beats 1, 7 and 8 are mainnet transactions; they need a vault the daemon watches |
| G1.3 | Storefront reaching live routes (O3) | Beats 2–6 are filmed through it |
| G1.4 | A small amount of real ZEC in hand | Deposits, exit and fees. Tens of dollars |
| G1.5 | Mainnet Zebra stable through a filming window | K1: a reboot costs 30–60 min. Don't reboot |
| G1.6 | Video recorded and posted in `#zechub` | The submission itself |
| G1.7 | Source + docs in the ZecHub repo | Required by the rules |

**Deliberately not required**

The sale backend, 4,444-image storage, strict deposit verification, custodian policy
verification, a second independent Zebra, zynscan. None of them change what the video
shows, and none of them protect anyone but you at dust scale.

**Nice to have, in priority order:** zynscan's anchor page (makes the settlement claim
checkable by a voter), a second independent Zebra (a stronger decentralisation claim
in the write-up).

---

## Gate 2 — NFT deposit opening

**Whose money is at risk: the public's.** At 4,444 items around 0.0133 ZEC that is
roughly **59 ZEC** of other people's money flowing into a vault, held for as long as
the collection lives. Every item here exists because that sentence is true; none of
them were required at Gate 1.

**Must have — security**

| # | Item | What goes wrong without it |
|---|---|---|
| G2.1 | **Custodian policy verification** (A1) | A custodian receives a bare 32-byte sighash (`custody_net.rs:171`) and cannot tell an anchor from a payout. A compromised coordinator can collect a quorum's signatures over **an arbitrary payout of the whole vault**. This is the single largest known risk and it is not theoretical |
| G2.2 | **Two independent Zebras** (O4, §36.4/36.5) | At 2-of-3 one honest view cannot block a bad root; the others still reach threshold. Deposit crediting rests on the sequencer's own node |
| G2.3 | **Strict deposit verification on** (O5, §37) | Credits are the sequencer's word. With per-account deposit addresses plus strict mode, a verifier checks each txid itself |
| G2.4 | Close the memo-less attribution gap (K6, §37.6) | An unmemoed deposit is credited from an operator-held file no verifier can see. Wrong-account credits become other people's losses |
| G2.5 | Split the Windows box's roles (K4) | It holds signer3 + custody3 + the mainnet node. One machine, three jobs, real money |
| G2.6 | A funded-vault drill on mainnet at small scale | Deposit, trade, redeem and **exit** with real ZEC before strangers do. Exits are the path never exercised under load |

**Must have — product**

| # | Item |
|---|---|
| G2.7 | Sale backend behind `apps/cave-market/#launch`: immutable policy publication, draw, allocation ledger, claim, team reserve, open mint, treasury split (A2, §64) |
| G2.8 | Storage for 4,444 × 35 KB, with the on-chain 32-byte hash matching what is served (A6) |
| G2.9 | The real storefront on cavebro.win — not the settlement lab, which is demo scaffolding |
| G2.10 | Verifiable allocation service: the canonical policy, public-entropy engine, replayable batch and per-item proofs are built in `cave-randomness`; publish policy + manifest before deposits, precommit targets, verify Zcash chain facts, and expose the durable ledger/receipts (§64) |
| G2.11 | Stated terms: what a buyer is buying, what the floor is, what happens if the sale does not fill |

**Should have**

zynscan (buyers checking the floor themselves is the whole pitch), Solana custody
federated off the sequencer's box (§36.7), an ops panel.

**The ordering that matters:** G2.1 before any public deposit. Everything else can be
staged; a vault whose custodians cannot see what they are signing should not be given
other people's money.

## Known operating characteristics

| # | Behaviour | Consequence |
|---|---|---|
| K1 | Zebra cold-opens 280 GB of RocksDB in 30–60 min | A reboot costs that much mainnet downtime. **Do not reboot near filming** |
| K2 | `zyn-tgbot` logs 409s for ~50 s after restart | Telegram's long-poll window. Self-resolving |
| K3 | Ad-hoc `nohup`/`ssh -f` forwards die with their SSH session | Use managed units. Cost three failed DKG attempts |
| K4 | Windows holds signer3 + custody3 + the mainnet node | Concentration; split before real money |
| K5 | Testnet vault is dealer-dealt with copied shares | Deliberate (§42.8) — it holds only TAZ |
| K6 | Deposits with no memo are credited from an operator-held attribution file | §37.6 — no verifier can check that mapping |
| K7 | **Restarting the sequencer costs ~25 min** of note-tree rebuild before the bridge starts | It seals and anchors nothing meanwhile. Measured 10 Sep: ~400 blocks/min falling to ~100. Treat a `zynzapd` restart as an outage, not a blip |

## Corrections — asserted, then disproved

Kept so they are not re-litigated.

| # | Claimed | Actually |
|---|---|---|
| C1 | The SanDisk was faulty; return it | The benchmark used `WriteThrough` with write caching disabled. The fault was a **USB 2.0 port**; on SuperSpeed it reads 756 MB/s and synced at 6,669 blocks/min |
| C2 | The signer halt was not version skew | It was. I re-tested 45 s after restart, before the replica had replayed 2546→3117 |
| C3 | The DA store had a 930-epoch gap | It did not. Bundle 3475 spans 929 epochs because it followed a stall |
| C4 | Something else was polling the bot token | My own `curl` was |
| C5 | ZF Minor Grants round 2 was open | That page is from **2023**. ZCG is the live programme |
| C6 | Zebra was crash-looping | `restarts=0`, `exit=0` — it was opening its database slowly |
| C7 | §36.6, §38.6 were open | Both closed by this session's work; `DECISIONS.md` markers were stale until this audit |
| C8 | The transparent work was "done" | The daemon routes were done; **the UI had none of it**, and still told users "Transparent senders cannot pay it." Built ≠ reachable |
| C9 | `/api/transparent` matched the rest of the API | It returned **zatoshi** where every other endpoint returns ZEC. No consumers had caught it; the first one would have shown every figure 100,000,000× too large |
| C19 | A hosted ZynZap would need the public JSON-RPC gateway | It does not. The extension already injects `window.zyn`, so a static site reads and signs through the visitor's own wallet. Shipping ZynZap and opening `zyn-rpc` are independent decisions, not one |
| C20 | Ordering sightings by lineage (A16/§61) would clear the chain-11 endorsement stall | It was necessary but not sufficient, and signer2 halted at 4336 proving it. A sort can only order what has been *seen*; the repaired 4330 was 1105 blocks ahead of the replica's scan cursor, so it was never a candidate for ordering at all. The real fault was applying each 200-block window as a complete view of the chain (§62) |
| C21 | `zyn_vm::derive` gave each VM version its own address space, as §4 promises | It did not. `derive_for` wrote the version at bytes `[32..34]` of a 34-byte buffer and hashed only `[..name.len()+2]`, so for any name under 32 bytes the version was never in the preimage — `zynzap` v1 and v2 derived **identical** addresses. Every existing test asked whether derivation was *deterministic*, which the bug preserves; it is distinctness that was lost (§66.2) |
| C18 | The agent work's version bump was a future risk | It landed the same day, on mainnet. The Sep 10 replica could not decode the Sep 11 sequencer's v8 base state. The warning was right and still needed a concrete failure to be acted on — the deploy-order rule belongs in the RUNBOOK, not in a review comment |
| C17 | The Air was powered off or asleep | It was up the whole time, `sleep 0`, SSH open. The scan that "proved" it down never ran — WSL2's NAT swallowed one, a path error killed the other, and ARP absence only means *that* host had not spoken to it recently. The account was `zhang`, not `tobiasd`; SSH gives an identical error for a wrong username and a wrong password |
| C16 | An nginx `map` on `$request_body` could enforce a method allowlist | It cannot: `$request_body` is unpopulated during the rewrite phase, so the map matched nothing and denied **every** request — including the permitted ones. It failed safe, which is the only reason it was obvious. A regex also cannot check a JSON-RPC batch honestly |
| C15 | The mainnet guard blocked only the transparent scaffold | It matched **every** `Custody::Zcash`, shielded included — so the vault mainnet exists for was refused too and the chain could never start. The message named the scaffold while the pattern caught everything |
| C12 | The ~170 `height 0` anchors were all confirmed-but-unrecorded | Most are. **4330 was reorged out after being marked confirmed at depth 6**; 4336 settled under a *different* txid than the index names. `height 0` and a stale txid are separate faults, and neither means what it looks like |
| C11 | Anchors above epoch ~1700 looked unsettled | They settled. `write_local` never replaced the height-0 line written at broadcast, so 170 confirmed anchors were **published as though they never landed**. Four sampled, four confirmed on chain |
| C10 | The extension shipped the wallet page | It shipped the page's *markup*. `build.mjs` copied `src/` and `icons/` but never `nap-mark.png`, so the popup navbar rendered an empty frame. The daemon serves that file from its own route, which is why it only broke in the extension |
| C13 | `CAP_OPERATE` was a safe capability for an AI session because it could not take value off Zyn | That was a coherent **warm-key** threat model. An agent can transfer the whole balance to an attacker's Zyn account, which is a total loss without crossing the bridge. Agent sessions now default to `CAP_SWAP`, and `Transfer` has its own capability (§51.1) |
| C14 | Adding mandate support was principally a new RPC operation | Replicas and the proof guest re-execute intents, so an RPC-only or Nap-only decision is not reproducible. The signed mandate and credentials had to enter the committed encoding, moving the VM/state/journal versions together (§51.3–51.4) |
| C22 | Passing the agent-mode tests made the feature ready to enable on the running sequencer | The code is ready; the deployment is a coordinated chain-format migration. Every deployed chain-11 binary is still old, and a sequencer-first rollout would halt the old signers exactly when signer2 is already unavailable (§51.8, AG0) |
| C23 | Opening Nap disclosed nothing beyond the local wallet process | The shared page fetched three font families from Google, so every desktop or extension popup open could disclose that the wallet was active. The fonts and OFL licences are now bundled, served locally, and copied into both runtime packages (§51.9) |
| C24 | Putting the transparent receiver at the standard BIP-44 path made the old raw-key wallet interoperable with mnemonic wallets | The path was standard, but its master seed was the independently generated Orchard spending key. A standard BIP-39 restore would derive a different transparent receiver. New wallets now retain the BIP-39 seed and derive both ZIP-32 Orchard and BIP-44 transparent account 0 from it; legacy wallets keep their old derivation (§56.4) |
| C25 | `cave-market` making no network calls was a blocker — the store was unwired | It was the architecture. §63.2 ships hosted Zyn frontends with `connect-src 'none'`: the page reaches the chain only through the extension's injected `window.zyn`, precisely so a public page cannot acquire a direct dependency on the node. `jade.js` already does this correctly. The real gaps were narrower — `app.js` has no provider calls, and the **store build emitted no `_headers` at all** while the launchpad build emitted a full policy |
| C26 | Batching settlement daily would mean ZEC arrives within 24 hours | It would not. Anchor cadence and bridge speed are independent: a payout is its own Zcash transaction, and "final" in the exit path means *that* transaction confirming, not an anchor covering it. `settle.rs` already groups exits canonically and continuously — twenty-odd per finality window. I read "batch settle once a day" as batching payouts, then reasoned forward from my own misreading instead of opening the settlement pipeline first (§77.4-77.5) |
| C27 | §63's re-offer cured the lost-endorsement class | It cured sending once and cut the wound one level deeper. Stopping on `clears` trusts a claim about the sequencer's *memory*; a restart drops those signatures and the signer never offers again. Chain 11 stalled eight hours with every signer visibly working — the logging made a dead chain look busy (§81) |
| C28 | The 333 would mint at serials 1..333 | They mint at 0..332. The VM requires `serial < cap` (`vm.rs:1687`), so a cap of 333 admits serials 0..=332 and serial 333 is `CollectionFull`. Found by the rehearsal, which rejected `MOLE333` after 332 successes. Collectors still count from one: `MOLE001` is serial 0. `zyn-genesis` binds number to serial in one struct so the off-by-one cannot spread. |
