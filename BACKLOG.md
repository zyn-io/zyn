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
| F7 | **Cave fungible launchpad does not exist yet.** Add permissionless token discovery and creation, the versioned persistent curve, buy/sell review, reserve and fee breakdown, graduation progress and the final ZynZap market link. Nap Mole ownership is not a gate; its multiplier belongs to the separate credit system. No production write is enabled until the §69 invariants and RUNBOOK §5j verification gate pass | 🔒 specified, not built | `cargo test -p swapvm cave_curve`; `cargo test -p zynzapd cave_launch`; `cd apps/cave-market && npm test`; `rg -q 'window\.zyn' apps/cave-market/app.js` |

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
| A2 | **Sale policy + allocation backend behind the finished creator UI:** canonicalize and commit `cave-allocation/v1`; enforce fixed/weighted/buyer-selected quantity policies; reserve worst-case capacity; derive public-beacon draws and identity shuffles; publish allocation ledger and receipts; claim, team reserve, open mint and treasury split | §64; `docs/2026-09-09-genesis-collection.md` |
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
| A28 | ~~Decide the 2,100,000 ZYN remainder~~ **DESTINATION SETTLED 13 Sep** (§71.9) — it is the ZYN side of the opening ZYN:ZEC.zy market, paired against accrued ZEC.zy. **Still open: the opening price.** §68.7's `0.0002 ZEC` is the Phase-2 wind-down trigger, so opening there fires it at launch; opening at `0.002` launches ten times past it. Needs two numbers, threshold above opening | Open; blocks any ZYN market |
| A31 | **Bound the reserve release rate against pool depth** (§71.6). Nothing circulates at open (§71.10), so this binds from the *first reserve release*, not launch. The pool moves 8.9% on a 100,000-ZYN sell with 18.9M queued behind it; unlock rules need a depth constraint, not only an activity test | Open; §71 |
| A32 | **Name ZYN's utility.** Not gas before Phase 2 (§68.5/§71.14), not redeemable (§68.9), not per-trade rewards (§69.15), not cashback (§71.12). Intended direction is a **credit system with a ZYN airdrop**, not yet designed; nobody is owed anything today (§71.13). **Still blocks A29** — release rules cannot name the unlocking activity until the credit system defines it | Open; blocks A29 |
| A29 | **Make pot release a committed predicate, not a judgement** (§71.6–71.7). Unlock rules must be evaluated by the VM over chain state, must not be per-trade volume (§69.15), and must be committed before a pot can release. If a person decides, the pots are a treasury and 'keyless' is decoration | Open; §71 |
| A30 | **Publish the three supply numbers from chain state** (§71.3) — `total` 21M constant, `unlocked`, `circulating` — derivable by an explorer without trusting us. They only answer the 90%-reserve criticism if published together with the release rules, from day one | Open; read API |
| A26 | ~~Define where NFT media lives and how a wallet trusts it~~ **DONE 12 Sep** (§67, `zyn-vm/src/manifest.rs`) — `zyn.manifest.v1`: media named by SHA-256 with **no URL**, location a replaceable hint outside consensus, gateway is a hash-keyed blob store, wallet verifies before render with an IndexedDB cache and fetching **off by default**. Size never enters the chain, so GIF and video need no protocol change. 10 tests; Rust, Python and JS pinned to one vector | §67 |
| A27 | **Serve the 666 NFT blobs.** `build_manifests.py` emits `out/blobs/<sha256>` with 333 image objects plus 333 canonical manifest objects (13.6 MB total), ready to upload as immutable hash-named objects, plus `out/content.json` mapping serial to the `content` to mint. Needs: the bucket, a privacy-safe `nap_media_sources` default, independent verification, and a mirror | Open; GitHub issue #17 |
| A25 | **Adopt the frozen address standard:** migrate `AssetId`/`PoolId`/`CollectionId` from `u32` to 32 bytes and replace fixed `[u8; 8]` symbols with length-delimited 1–32-byte UTF-8 under `ADDRESS_SCOPE_V1 = b"zynzap\x00\x02"` (§4.4, §66). Enforce exact symbol/origin bytes, reject duplicate derived ids, delete the three `next_*_id` counters and `find_pool`, move pool reserves to `vault(pool)`, and wire derived identities through state, wire, codec, Merkle, receipts, RPC, wallet, bridge, explorer and replay. Add typed Bech32m and address-keyed official badges at public boundaries plus exact Rust/JS vectors. This is one `STATE_VERSION`/wire fork; upgrade replicas and signers before or alongside the sequencer (§59.5). **Do compressed collections first** so 4,444 Cave items are not widened and then deleted | Open; consensus choices locked, implementation required while mainnet has no assets |
| A22 | **signer3 cannot start from the published base state, cause unresolved.** Its halt at 6692 was genuine (the §A17 re-base left its base root off the lineage). New binary installed and hash-verified; env file correct and proven readable by systemd (a transient unit running `printenv` returned the set value). But `zyn-replica` reports `is not the attested root c8c503e6…` regardless of `ZYN_BASE_ROOT`, including an explicit `--setenv=1111…`. At `replica.rs:249` that printed value *is* the env var, and it is the only such site in the workspace — so one of these measurements is wrong and I have not found which. signer3 left **stopped**; chain runs 2-of-2 on signer1+signer2 | Open, **no longer blocking** — A23 sidesteps it by never leaving the data dir empty. Still worth resolving: any future replica built from scratch hits it |
| A23 | ~~Seed signer3 from signer2's verified state~~ **DONE 12 Sep** — hot-copied signer2's data dir (28 MB, 1.1 MB gzipped, hash-verified) while it kept running, so no signer ever stopped. signer3 resumed at epoch 6821 and its first endorsement cleared a certificate. It signs as `cc158dc4…` (set slot 3) — **only the chain state came from signer2; the signing identity is its own**. Set is 3-of-3 healthy again, threshold 2 with margin | §A22 |
| A21 | **Persist the in-flight anchor's certificate on the sequencer** (§63.6). A20 heals this from the signer side, but the sequencer still loses collected signatures on restart — which also means a restart discards work the set already did. Write partial certificates alongside the anchor and reload them at startup | Open; §63 |
| A19 | ~~Defer a lineage break while the scan is behind the tip~~ **BUILT 11 Sep** (§62) — `apply_sightings_scanned(.., scan_complete)`; the replica holds sightings it cannot yet place instead of latching, and only halts on a break that survives a complete scan. Two tests: one holds and clears when the repair arrives, one still halts on a complete view | §62 |
| A18 | ~~Upgrade signer1 to the §62 binary~~ **DONE 12 Sep** — repointed at `zyn-replica-a16` and restarted; it resumed from its persisted ledger at epoch 5397, re-offered its lost endorsement, and the certificate cleared. The shared `~/.zyn/bin/zyn-replica` was left untouched (§59.4) | §62, §63 |
| A16 | ~~Order sightings by lineage, not height~~ **BUILT 11 Sep** (§61) — `in_lineage_order()` now sorts `(epoch, height)`, extracted as a pure function with three tests including the real 4321/4330/4336/4474 inversion. `previous_root` still validates every step, so the sort decides what is *offered*, never what is *accepted*. Deployed to signer2 as `zyn-replica-a16`; signer1 still runs the old binary and is healthy past the gap | §61 |
| A15 | **Re-anchor the tail** (§50.14, option 2) — re-anchor every anchor after a gap so chain order is restored. Correct and needs no protocol change, but each one waits on the vault's single note, so it is slow and costs a note per anchor. The fallback if A16 is judged too invasive | §50 |
| A14 | ~~Re-anchor a historical anchor~~ **BUILT 11 Sep** — queued repair, retried until the vault has a note; 4330 is back on Zcash at height 4338929 (§50.9). **Does not heal the replica**: it applies anchors in chain order and the repair landed after 4336, so a decision is owed on §50.14 | §50 |
| A12 | ~~Nap Wallet transparent receive~~ **DONE 10 Sep** — derivation, UTXO lookup and t→z sweep, proven end to end, and surfaced in the wallet UI as the "Coming from an exchange?" drawer (§45) | §45 |
| A28 | **Verified NFT media resolution and funded two-wallet QA.** Resolve content only through a privacy-reviewed path, hash the fetched bytes against the committed 32-byte content id before display, and record a live Nap A → Nap B transfer where the item disappears from A and appears in B. The current identicon + commitment is the honest fallback, not a broken image (§65.5) | §65; GitHub issue #8 |
| A29 | **Prepare the one launchpad state fork.** Finish A25's 32-byte identifiers first. Remove the superseded ZYN per-block mint, halving, burn and bridge-fee launch state plus the old ZynZap tokenomics panel; add a derived keyless ZEC.zy pot for future ZYN POL without issuing ZYN. Specify the `STATE_VERSION`/VM-version migration and upgrade replicas/signers before or alongside the sequencer | §68; §69.17–18; RUNBOOK §5j |
| A30 | **Build Cave curve state and exact math:** one-billion supply with eight decimals, 660M curve/340M graduation inventory and no free creator allocation; exact `p0 = 0.009/1B`, `Pg = 100/1B`, derived linear slope, no expiry, fixed-point integrals, rounding, refundable reserve and monotonic `Curve → AMM`. Derive and reconcile separate curve inventory, graduation inventory and refundable-reserve accounts. Permit a missing `genesis_pool` only with a matching live curve. Property-test against `reserve == R(q)` | §66.14; §69.4–7, §69.10, §69.18, §69.27–28; RUNBOOK §5j |
| A31 | **Extend fee accounting end to end.** At creation route `0.009 ZEC.zy` to the derived pair-seed account and `0.001` to treasury. Commit creator-selected `50–300 bps` trading fee, default `100`, immutable through graduation. Use separate derived graduation-fee, creator-escrow and global future-ZYN-POL accounts; implement curve `50/40/10`, NFT `50/40/10`, and graduated `45/45/10`, including exact remainder rules in quote, hop, batch, receipt and replay | §66.14; §69.8–9, §69.14, §69.19, §69.25–28; RUNBOOK §5j |
| A32 | **Build atomic curve graduation on ZynZap primitives:** stop curve pricing, derive the terminal pool ratio in `ZEC.zy per token`, move reserve plus graduation fees, the `0.009 ZEC.zy` pair seed and reserved inventory, cap pool ZEC at 34, route overflow to future ZYN POL, create exactly one canonical ZEC.zy AMM with no external reference, set `genesis_pool`, lock every LP share, release creator escrow, or roll everything back | §69.10–12, §69.20–21, §69.25, §69.28; RUNBOOK §5j |
| A33 | **Validate the fixed persistent curve before code:** encode exact rational/fixed-point vectors for `p0`, `Pg`, slope, 660M/340M allocation and `R(Qg) = 33.00297 ZEC.zy`. Simulate every allowed fee, heavy buy/sell churn, long inactivity, restart and atomic rounding. Cap graduated pool ZEC at `Pg × 340M = 34 ZEC.zy`; prove excess reaches future ZYN POL and no healthy launch can fail from inventory overflow | §69.5–6, §69.10–13, §69.28; RUNBOOK §5j |
| A34 | **Expose launchpad end to end:** four VM intents and typed receipts; exact Unicode symbol/display validation; two launches per creator per rolling 60 chain epochs; atomic same-price dev buy capped at 5%; wire/state/replica/proof replay; list/get/quote plus write RPC/client/HTTP/provider methods; explicit `zec_per_token`; permissionless Nap approval text; and zynscan reserve, fee-pot, overflow, phase and locked-LP evidence | §66.11–12; §69.16–29; RUNBOOK §5j |
| A35 | **Build the non-authoritative market projection:** derive launch and graduated-pool trades from accepted receipts into OHLCV keyed by launch/asset/pool; make it replayable and prevent double-counting at graduation. Gallery/chart failure must never block consensus reads, trades, curve sells or exits | §69.23; RUNBOOK §5j |
| A36 | **Staged permissionless launch:** two-wallet testnet rehearsal → public read-only markets → public alpha with the fixed per-creator two-per-60-epoch launch rate and no NFT/manual/value/concurrency gate. Raise general infrastructure capacity only after a long-lived curve and atomic graduation survive restart and replica replay. Check launch-fee routing, dev buy, uncapped buys, curve reserve, both graduated fee directions, creator/POL credits, overflow, price orientation and locked LP | §69.16–29; RUNBOOK §5j |
| A37 | **Implement address-based authenticity and Unicode presentation.** Accept the exact symbol/display-name bounds in §66.11 and §69.29; show short addresses for duplicate/confusable labels; badge only pinned official addresses; and let reports warn or de-index without affecting settlement, curve sells or exits. Provision the threshold `Treasury 26460` AccountId, then regenerate and pin the official Nap Mole and Cave collection/item vectors before minting | §66.16–17; §69.29; RUNBOOK §5j |
| A38 | **Design the separate credit system and Nap Mole multiplier.** Define what earns credits, the multiplier value, ownership snapshot/live-check rule, transfer behavior, caps and Sybil policy. Prove the credit system cannot gate, discount, prioritize or otherwise alter Cave launchpad execution | §69.16, §69.24; outside launchpad consensus |

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
| G2.10 | Verifiable allocation: publish canonical policy + manifest commitment before deposits, predeclare future public entropy and exact algorithm, then publish a replayable ledger and per-entry receipts. A private seed reveal or browser preview is insufficient (§64) |
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
| C17 | Passing the agent-mode tests made the feature ready to enable on the running sequencer | The code is ready; the deployment is a coordinated chain-format migration. Every deployed chain-11 binary is still old, and a sequencer-first rollout would halt the old signers exactly when signer2 is already unavailable (§51.8, AG0) |
| C18 | Opening Nap disclosed nothing beyond the local wallet process | The shared page fetched three font families from Google, so every desktop or extension popup open could disclose that the wallet was active. The fonts and OFL licences are now bundled, served locally, and copied into both runtime packages (§51.9) |
| C19 | Putting the transparent receiver at the standard BIP-44 path made the old raw-key wallet interoperable with mnemonic wallets | The path was standard, but its master seed was the independently generated Orchard spending key. A standard BIP-39 restore would derive a different transparent receiver. New wallets now retain the BIP-39 seed and derive both ZIP-32 Orchard and BIP-44 transparent account 0 from it; legacy wallets keep their old derivation (§56.4) |
