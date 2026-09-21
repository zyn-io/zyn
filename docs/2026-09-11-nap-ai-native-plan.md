# Nap Agent Mode and Zyn Mandates

**Status:** design record; implementation state and proof commands live in `BACKLOG.md` (V14 and “Open agent work”)  
**Date:** September 11, 2026

## 1. Proposal

Make Nap an AI-native wallet by giving software agents a first-class tool interface and a narrowly constrained way to act. The AI plans off-chain. Nap protects keys, translates plans into exact typed intents, simulates their effects, and submits them. Zyn deterministically enforces the authority the user granted.

The product claim becomes:

> **Nap is an AI-native wallet for high-speed execution on Zyn, anchored to Zcash. Users give agents a mandate, not their wallet.**

This is more than adding chat to the wallet. An AI-native wallet needs:

1. A machine-readable interface designed for agents.
2. Authority that can persist across multiple actions without exposing an owner key.
3. Limits enforced independently of the model's behavior.
4. Exact simulation and human-readable intent previews.
5. An audit trail from proposal through execution, sealing, and Zcash anchoring.
6. A fast way to stop an agent and recover its remaining budget.

## 2. Why this fits the existing architecture

The required primitives are already partly present:

- `zyn-vm` has expiring session-key delegation and separates operate, withdraw, and delegate capabilities.
- Signed intents bind the chain, VM, validity window, and typed intent fields.
- ZynZap has quotes, swaps, liquidity, public resting offers, receipts, and Zcash-anchored checkpoints.
- Nap keeps keys in its process and exposes one JSON API through both the local wallet and Tauri shell.
- The node already distinguishes public reads from signed writes.

Three gaps prevent the current session mechanism from being safe for an AI:

1. `CAP_OPERATE` reflects a warm-key threat model, not an agent threat model. Its grouping is deliberate: a warm key may waste value but cannot take it off Zyn. An external agent breaks that assumption because `Transfer` can send the account's entire balance to an attacker's Zyn account, which is still a total loss even though no asset left the chain.
2. Delegated authorization exists in the VM and verification libraries, but there is no delegated-submit operation in the public node RPC and no session lifecycle in Nap.
3. Authorization is currently checked before the application transition rather than carried as committed application input. A replica or future proof executor cannot independently reproduce a mandate decision unless the mandate and its credentials are present in the committed encoding and verified on the deterministic execution path.

These should be fixed before describing Nap as an autonomous agent wallet.

## 3. Design rule

AI output is never consensus input by itself.

The model may interpret a goal, inspect permitted state, compare options, and propose an action. Nap converts the proposal into a canonical intent. The existing deterministic VM computes the outcome and rejects anything outside the mandate. No model inference, remote response, wall clock, or floating-point result enters a state transition.

```text
 User goal
    |
    v
 AI agent -------- nap-agent CLI ---> Nap Agent Gateway
    |                                      |
    | action proposal                      | exact integer quote / simulation
    v                                      v
 Zyn Mandate policy <---------- Nap signer and policy boundary
                                           |
                                           | signed canonical intent
                                           v
                                     Zyn execution
                                           |
                                   receipt / state root
                                           |
                                           v
                                      Zcash anchor
```

## 4. Product modes

### 4.1 Advisor mode

The agent can read allowed wallet and chain information, request quotes, and draft actions. Every write requires a confirmation in Nap. This ships first and does not require autonomous authority.

Example:

> “Compare the ZEC.zy pools and prepare a swap of 0.25 ZEC.zy with no more than 0.5% slippage.”

Nap presents the exact route, amount in, expected amount out, minimum amount out, fees, expiry, and balance changes before signing.

### 4.2 Mandate mode

The user approves a bounded mandate once. Nap can then sign qualifying actions requested by the agent without another popup. Every action outside the mandate is rejected before signing and again by the accepting node.

Example mandate:

> For the next 50 epochs, use at most 5 ZEC.zy. Only submit exact-input swaps between ZEC.zy and USDC.zy through pools 1 and 4. Maximum 0.5% slippage. Never transfer, withdraw, change liquidity, create assets, bind destinations, or delegate.

### 4.3 Private advisor mode

Run the model locally or expose redacted data to a remote model. This is a privacy setting, not a separate protocol. Nap must clearly state what wallet data leaves the device.

## 5. Agent-facing interface: CLI first

Add a dedicated `nap-agent` CLI as a separate, allowlisted face over the Nap application core. Do not give an agent the existing generic `api(method, path, input)` function or unrestricted access to developer and operator commands.

CLI-first is the recommended delivery choice. Codex, Claude Code, local agents, scripts, and test harnesses can all call it without an integration SDK. Its inputs, outputs, exit codes, and audit behavior are straightforward to exercise end-to-end. MCP offers better tool discovery and richer host integration, but it should remain an adapter once the command and authorization contract is stable.

The CLI is a transport, not the security boundary. It should be a thin client to a Nap-owned policy and signing service over authenticated local IPC. It must not open wallet files or hold owner and session private keys itself. The AI process should be able to execute `nap-agent` without receiving filesystem access to Nap's application-data directory.

Use a Unix-domain socket on macOS/Linux and a named pipe on Windows, or authenticated loopback with a random per-install credential. The credential authorizes only the agent API; it must not unlock the generic wallet API.

All commands return a versioned JSON envelope on stdout. Human-readable diagnostics go to stderr, and process exit codes distinguish success, policy refusal, invalid input, unavailable service, and internal failure. Accept structured command arguments or JSON on stdin. Never interpolate model-generated text into a shell command.

Example envelope:

```json
{
  "schema": "nap.agent.v1",
  "ok": true,
  "request_id": "0199...",
  "state": "queued",
  "data": {}
}
```

### Read commands

| Command | Result |
| --- | --- |
| `nap-agent portfolio` | Permitted balances and pending activity |
| `nap-agent assets` | Asset ids, symbols, supply, and type |
| `nap-agent pools` | Reserves, fees, references, and freshness |
| `nap-agent quote swap …` | Exact integer quote and minimum-output options |
| `nap-agent offers` | Public resting offers and expiration epochs |
| `nap-agent anchors` | Epoch-to-Zcash anchor history |
| `nap-agent action <request-id>` | Current lifecycle state of one submitted action |

### Draft and simulation commands

| Command | Result |
| --- | --- |
| `nap-agent draft swap-exact-in …` | Canonical typed intent without a signature |
| `nap-agent draft offer …` | Canonical offer terms without a signature |
| `nap-agent simulate --intent-file -` | Acceptance or rejection, receipts, fees, and balance deltas against a named state root |
| `nap-agent check --mandate <id> --intent-file -` | Whether an intent is allowed, with a structured refusal reason |

### Write commands

| Command | Behavior |
| --- | --- |
| `nap-agent request-approval --intent-file -` | Opens an exact intent preview in Nap for the user |
| `nap-agent execute --mandate <id> --intent-file -` | Signs and submits only after policy checks pass |
| `nap-agent pause --mandate <id>` | Stops new signatures locally immediately |
| `nap-agent close --mandate <id>` | Revokes or closes the mandate and begins recovery of remaining funds |

CLI results must distinguish `proposed`, `awaiting_approval`, `approved`, `submitted`, `accepted`, `queued`, `sealed`, `anchored`, `settled`, `rejected`, and `expired`. A queued batch swap must never be reported as completed.

An MCP adapter may be added later by mapping tool schemas to these commands or, preferably, to the same internal agent service. It should add discovery and structured tool descriptions without introducing a second authorization path.

## 6. Zyn Mandate v1

### 6.1 Split capabilities before adding AI access

The current capability grouping was not an oversight. It draws a coherent line for a warm key on hardware the user controls: the key may trade badly, but it cannot withdraw or redirect an exit. That line is insufficient for an external agent. A transfer to an attacker-controlled Zyn account loses the full balance while technically remaining “on chain.” An agent permission therefore needs to distinguish economic transfer from reversible market activity.

Replace the broad operate bit with explicit capabilities:

```text
CAP_SWAP
CAP_OFFER
CAP_LIQUIDITY
CAP_TRANSFER
CAP_ITEM
CAP_COLLECTION
CAP_WITHDRAW
CAP_BIND
```

Delegated sessions must never receive `CAP_BIND` or the ability to create another delegation. The default agent mandate should contain `CAP_SWAP` only. `Transfer` must not count as trading.

### 6.2 Signed constraints

A mandate should bind at least:

```rust
struct Mandate {
    owner: AccountId,
    session_key: [u8; 32],
    permissions: u32,
    allowed_assets: Vec<AssetId>,
    allowed_pools: Vec<PoolId>,
    budgets: Vec<AssetBudget>,
    max_amount_per_action: Vec<AssetBudget>,
    max_slippage_bps: u16,
    max_actions: u32,
    valid_from_epoch: u64,
    valid_until_epoch: u64,
    salt: [u8; 32],
}
```

All values use integers or `Fixed`. Avoid a dollar-denominated limit in v1 because it makes authorization depend on reference-price freshness and cross-asset conversion. Define budgets per input asset instead.

The owner's signature covers every field, the chain id, and the VM id. Each agent signature covers the mandate id, the authorization envelope, and the complete intent.

The mandate, owner credential, session credential, and action must travel in one canonical committed form. One possible shape is:

```rust
struct CommittedMandatedIntent<I> {
    mandate: Mandate,
    owner_credential: Credential,
    session_signature: [u8; 64],
    action: I,
}
```

The exact generic boundary needs design work, but the requirement does not: mandate verification must execute wherever replicas and the future proof path re-execute transitions. Nap may preflight the same rule for fast feedback, but Nap cannot be the authority. RPC only transports the committed object; it does not define or weaken its meaning.

This preserves an existing safety property: replicas re-derive transition results instead of trusting the sequencer. That independent execution has already exposed a settlement discrepancy. Mandate validation belongs on the same path so an authorization discrepancy is equally reproducible and rejectable.

Adding this object changes the epoch intent commitment. It therefore requires coordinated wire, VM, journal, data-availability, replay, and state-version handling rather than an endpoint-only patch.

### 6.3 Budget enforcement

There are two viable designs.

**A. Dedicated agent account — recommended for the first autonomous release.** Nap creates a separate owner-controlled Zyn account, funds it with the maximum amount at risk, binds recovery to the user's wallet, and delegates only the allowed capabilities to the agent session. The account balance provides a simple hard loss ceiling. The session key never receives the account's owner key.

**B. Stateful mandate accounting.** Commit mandate definitions and per-asset usage in chain state. This supports limits over a user's existing account and immediate on-chain revocation, but changes application state, wire formats, roots, replay, and exit-related reasoning.

Start with A. Design B only after the agent interface has proven useful. Per-action limits, asset and pool allowlists, slippage, action count, and expiry can still be checked from the signed mandate. The dedicated account bounds cumulative loss without inventing cross-asset accounting.

### 6.4 Revocation

Local pause and chain revocation solve different failures:

- **Pause:** Nap immediately stops signing. This handles a misbehaving model or prompt injection.
- **Expiry:** every mandate ends at a fixed epoch even if Nap disappears.
- **Close:** the owner invalidates the mandate and recovers the dedicated account's remaining assets.

If v1 certificates remain stateless, close may require rotating the agent account or waiting for expiry. The UI must say so plainly. Do not label a local pause as an on-chain revocation.

## 7. Nap security boundary

Nap owns all private keys and exposes handles, never key bytes. The agent gateway must have its own allowlist rather than reusing the full wallet API surface.

The following operations are permanently unavailable to AI tools:

- Export or import spending keys.
- Read raw seeds or session private keys.
- Send native ZEC to an arbitrary address.
- Withdraw from Zyn.
- Bind or rebind a withdrawal destination.
- Change wallet networks, block servers, node endpoints, or security settings.
- Create broader mandates or delegate authority.

The agent gateway should use a Unix-domain socket on macOS/Linux and a named pipe on Windows, or an authenticated loopback channel with a random per-install token. Origin filtering alone is insufficient. Mobile can remain advisor-only initially.

Remote models create a separate privacy issue even when keys remain local. Nap should offer three disclosure policies:

1. Full: exact balances and history may be sent to the selected model provider.
2. Redacted: symbols, percentages, and bounded action context only.
3. Local: wallet information stays on the device.

Shielded memos, addresses, raw transaction data, and viewing keys should be excluded by default.

## 8. Deterministic simulation

Add a dry-run endpoint that applies a canonical intent to a cloned state and returns:

- State root used for the simulation.
- Accepted or rejected.
- Typed intent fields.
- Expected receipts.
- Per-asset balance deltas.
- Fees and minimum output.
- Whether settlement is immediate or queued until a seal.
- Mandate decision and the exact constraint used for any refusal.

The submitted intent must reference the state root or quote context that was simulated. Nap should re-simulate if the state has moved materially. For batch-cleared swaps, present the current quote as an estimate and `min_out` as the enforceable guarantee.

## 9. Action identity and audit trail

Every proposal gets a random client request id. Every canonical intent gets a deterministic digest. Retrying a tool call with the same request id must return the existing action instead of submitting another one.

Nap records:

```text
request id
model/provider identity, if disclosed
mandate id
natural-language goal or its local-only hash
typed intent
simulation root and result
approval event
intent digest and signature scheme
node acceptance or rejection
receipt
sealed epoch
Zcash anchor txid and height
```

The natural-language prompt should not be published to Zyn or Zcash. The canonical intent and mandate are the auditable agreement.

## 10. Threat model

| Threat | Required control |
| --- | --- |
| Prompt injection tells the model to drain funds | Transfers and withdrawals absent from the mandate; asset, pool, amount, and slippage limits |
| Agent process is compromised | Session secret stays in Nap; short expiry; dedicated funded account caps loss |
| Malicious local client calls hidden wallet methods | Separate allowlisted gateway and authenticated local channel |
| Model hallucinates an asset or pool | Structured ids resolved by Nap; simulation rejects unknown values |
| Quote becomes stale | State-root binding, expiry, re-simulation, and signed minimum output |
| Tool retry duplicates a trade | Request idempotency plus existing signature replay protection |
| Sequencer accepts an action outside its mandate | Replicas re-derive mandate validity from the committed credential and intent in the deterministic VM path |
| Mandate data is omitted or changed in transit | Include the mandate and both credentials in the canonical intent commitment; reject invalid or incomplete encodings |
| Sequencer omits an approved action | Existing forced-inclusion route can be considered later; keep it outside the first agent release |
| Remote model learns private activity | Disclosure controls, redaction, and local-model option |
| User thinks a queued swap has settled | Explicit lifecycle states and receipt/anchor tracking |

## 11. Delivery plan

### Milestone 0 — Correct the authorization vocabulary

- Split `CAP_OPERATE` into granular capabilities.
- Make transfer unavailable to the default session.
- Add tests proving a swap-only session cannot transfer, withdraw, bind, add liquidity, create assets, mint, burn, or delegate.
- Preserve and regression-test the existing fail-closed rule for unclassified intents: the default capability requirement remains all capabilities, which no delegated session can satisfy.
- Preserve chain and VM domain separation and expiry behavior.

**Exit criterion:** “swap-only” has a mechanically accurate meaning.

### Milestone 1 — Advisor mode

- Add the allowlisted agent gateway and versioned `nap-agent` CLI schemas.
- Implement portfolio, assets, pools, offers, quotes, anchors, draft, and simulation commands.
- Add an approval inbox to Nap.
- Require user confirmation for every write.
- Add disclosure controls and redact sensitive wallet data by default.

**Exit criterion:** an agent using only `nap-agent` can prepare a valid action but cannot sign, submit, export keys, send ZEC, or change wallet settings without Nap-mediated approval.

### Milestone 2 — Committed mandate format and activation

- Define one canonical committed envelope containing the mandate, owner credential, session credential, and action.
- Verify the complete envelope in the deterministic VM transition path used by the sequencer, replicas, replay, and the future proof path.
- Extend `swapvm::wire`, the epoch journal, data-availability encoding, replay, and proof inputs as one protocol change. Keep transport fields out of the committed format.
- Add the required `STATE_VERSION` step and any VM or domain-version step needed to prevent old and new encodings from being confused.
- Define an activation boundary and coordinate replica rollout so all replicas derive the same intent commitment and state root before and after activation.
- Add migration and compatibility tests for existing checkpoints and journals.

**Exit criterion:** the sequencer and an independent replica accept or reject the same mandated action using only committed data, and the format transition produces the expected roots at a defined activation point.

### Milestone 3 — Delegated RPC and Nap lifecycle

- Add a node RPC operation that transports the committed mandated-intent envelope without introducing a second authorization rule.
- Add session creation, storage, expiry, and signing handles to Nap.
- Add structured refusal codes for expired, disallowed, over-limit, wrong-asset, wrong-pool, and excessive-slippage actions.
- Persist request ids and action lifecycle states.

**Exit criterion:** one owner approval authorizes multiple permitted swaps; attempts to transfer or withdraw are refused during both sequencing and replica re-execution.

### Milestone 4 — Dedicated agent accounts

- Let Nap create and label a separate owner-controlled Zyn account.
- Fund it with an explicit maximum-at-risk amount.
- Bind recovery to the user's wallet before autonomous actions begin.
- Add pause, close, expiry countdown, and recovery flows.
- Display agent holdings separately from the primary account.

**Exit criterion:** compromising the agent path cannot affect assets outside the funded agent account or redirect recovery.

### Milestone 5 — Autonomous strategies

- Ship one narrow strategy: exact-input rebalancing or taking qualifying resting offers.
- Use deterministic trigger conditions and require a named mandate.
- Add notifications for execution, rejection, expiry, pause, and anchor confirmation.
- Keep model selection pluggable; support a local model where available.

**Exit criterion:** the demo can run through a complete goal → proposal → simulation → mandated execution → receipt → Zcash anchor lifecycle.

## 12. Verification

### Protocol and authorization tests

- Every intent maps to exactly one capability.
- Preserve the existing fail-closed default: every unclassified future intent requires all capabilities and remains unavailable to delegated sessions.
- Swap-only sessions cannot transfer even to another account owned by the same wallet.
- Every constraint changes the mandate signature payload.
- Mandates cannot cross chains, VMs, accounts, or expiry windows.
- A session signature cannot be replayed under another mandate.
- The sequencer, replicas, replay, and proof-input construction derive the same mandate decision from the same committed bytes.
- Removing or changing the mandate, owner credential, session credential, or action changes the intent commitment and invalidates authorization.
- Pre-activation journals and checkpoints replay under the old format; post-activation data requires the new state and wire version.
- Malformed policies and delegated frames never panic.

### Economic tests

- Exact-input amount cannot exceed its per-action limit.
- Only allowlisted pools and assets are accepted.
- `min_out` satisfies the mandate's slippage ceiling.
- The dedicated account cannot spend more than its funded balances.
- Cancelling an offer returns assets to the agent account and does not widen authority.

### Nap and CLI tests

- CLI commands expose no generic path, method, or raw-RPC escape hatch.
- JSON schemas and exit codes remain stable and versioned.
- Model-provided values are passed as structured arguments or stdin JSON, never shell-interpolated command text.
- Key material never appears in tool responses, logs, errors, or model context.
- Duplicate request ids do not create duplicate submissions.
- Approval text matches the canonical signed fields.
- A queued action cannot transition directly to settled without a receipt.
- Restart restores mandates, pending actions, and audit history.

### Adversarial scenarios

- Inject instructions through token symbols, item metadata, offer metadata, memos, and error strings.
- Ask the model to call undisclosed endpoints or place raw JSON in tool arguments.
- Race a quote against a state change.
- Repeatedly submit the maximum permitted action.
- Compromise the model process and attempt transfer, withdrawal, rebind, delegation, and key export.

## 13. Demo

The first public demonstration should avoid an open-ended “trade for me” prompt.

1. The user deposits 5 ZEC.zy into a dedicated agent account.
2. Nap shows a mandate allowing only exact-input swaps in one pair for 50 epochs, with a 0.5% slippage ceiling.
3. The user approves once.
4. An agent reads pools and requests a quote through `nap-agent`.
5. Nap simulates and executes an allowed swap without another approval.
6. A malicious or deliberately invalid instruction attempts a transfer and is rejected.
7. Nap follows the accepted action through seal and displays the Zcash anchor transaction.
8. The user pauses the agent and recovers the remaining assets.

The demonstration proves the product claim directly: the AI can act, but the mandate—not the model—controls the money.

## 14. Non-goals

- Running model inference inside the Zyn VM.
- Letting the model hold a Zcash spending key or Zyn owner key.
- Giving an agent unrestricted access to the existing wallet or operator CLIs.
- A generic CLI or MCP wrapper around every existing Nap API endpoint.
- Autonomous native-ZEC sends or withdrawals in the first release.
- Dollar-denominated budgets in v1.
- Advertising investment performance or an “AI trading edge.”
- Making one model provider part of the protocol.

## 15. Review questions

1. Is a dedicated agent account a sufficient cumulative loss bound for v1, or is stateful per-mandate usage accounting required immediately?
2. Should offer placement and cancellation ship with autonomous swaps, or remain advisor-only initially?
3. Should the committed authorization envelope be generic in `zyn-vm`, specific to SwapVM, or split into a generic signed wrapper plus VM-defined mandate constraints?
4. Which versions and domains must change at activation: `STATE_VERSION`, the VM version, wire tags, signature domains, or all four?
5. How should old journals, checkpoints, and replica catch-up cross the activation boundary without producing ambiguous roots?
6. How does an owner close a dedicated agent account without creating a second withdrawal path?
7. Which state change makes a quote stale enough to require re-approval?
8. Should remote-model access expose exact balances by default, or require an explicit per-provider permission?
9. Does `nap-agent` talk to a Nap sidecar, authenticated loopback service, Unix socket/named pipe, or a refactored persistent Nap Core process?

The first review should focus on questions 1, 3, 4, and 5. They determine the committed format, migration path, and cumulative loss bound; VM enforcement is a settled requirement.
