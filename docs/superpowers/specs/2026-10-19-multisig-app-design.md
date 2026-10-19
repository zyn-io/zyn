# The multisig app: one signer for Solana, Ethereum, Zcash and Zyn

Status: design, approved section by section in chat on 19 Oct 2026. Not built.
Name: open ("the multisig app" below).

## 1. What it is, and for whom

Teams keep their existing multisigs and assets where they are: Squads v4 on
Solana, Safe on Ethereum. Each member signs from one identity, their Nap
recovery phrase, instead of separate Phantom and MetaMask wallets. The same
app also runs Zcash shielded multisigs (FROST) and Zyn native shared
accounts. Nothing moves to Zyn unless a team chooses to.

- First team: ourselves. Our Squads (`A334BUsB…`, bridge vault 0), our Safe
  (`0xEE741530…`) and Treasury 26460 (`b3e4c493…`).
- Then: DAOs and crypto-native teams already on Squads and Safe. Not
  institutions (they buy MPC: Fordefi, Utila, Fireblocks).
- Success: a team with a Squads and a Safe switches its members to Nap
  signers in minutes, then proposes and approves on both from one queue
  without anything pending ever being posted publicly.

### Decisions (the user, 19 Oct 2026)

| Decision | Choice |
|---|---|
| Where it lives | Its own site (like Jade and Lend); Nap is the signer through Nap Connect |
| Chains | Solana mainnet (Squads v4), Ethereum **layer 1 only** (Safe), Zcash (FROST, shielded), Zyn (native) |
| Multisig contracts | Squads' and Safe's own on-chain programs; never our own |
| Private queue | A relay we run (Worker + database) storing only end-to-end-encrypted data |
| Network fees | The multisig pays its own: the relayer submits and is refunded inside the approved transaction; members' signers never hold funds |
| Price | Free for now; the refund path is where a fee could go later |
| Zcash co-signers | Not limited to Nap: Zcash Foundation FROST + frostd + PCZT |
| Signer keys | From Nap's own recovery phrase, never from a wallet signature (unlike Zyn Lend's keyring) |

## 2. Why it is worth building (competitor check, 19 Oct 2026)

No verified product lets a team propose and sign on both its existing Squads
and its existing Safe from one app. Squads shows Safes view-only; Range only
monitors Safe/Squads/Realms; Den, Ledger Multisig and Safe are EVM-only;
MPC platforms hide signers but are vendor-run. Open space: one signer that is
a Squads member and a Safe owner; a private pending queue on Squads; owner
lists not linked to members' personal wallets; Zcash and Zyn in the same app.

Risks: Safe + Bermuda (private queue on Safe, testnet beta since 2 Oct 2026)
may take the Ethereum half of the privacy pitch, so the pitch leads with the
cross-chain signer and the Solana/Zcash/Zyn parts. Squads or Ledger may add
the other chain. Safe wrapper businesses have struggled (Coinshift shut down,
Multis acquired, Cobo retired Safe). Notes: scratchpad
`multisig-competitors-2026-10-19.md`.

## 3. Architecture

```
 Multisig site (own domain)          Nap (web / extension / mobile)
 ─────────────────────────           ──────────────────────────────
 create / import, queue,      ⇄      the signer: one recovery phrase →
 balances, history             Nap    Solana key, Ethereum key, Zyn account,
 builds every transaction    Connect  Zcash FROST shares
                                      decodes what it signs, then signs
        │
        ▼
 Private queue relay (Worker + D1)            Chains
 encrypted proposals and signatures;      →   Squads v4 (Solana mainnet)
 a frostd-compatible endpoint for Zcash;      Safe (Ethereum mainnet)
 the relayer submits fully signed             Zyn native shared accounts
 transactions and is refunded                 Zcash FROST (shielded)
```

### Units

1. **Signer core (Nap).** Keys, decoding, signing. Depends on nothing below.
2. **Queue relay.** Encrypted storage per multisig, submission as fee payer,
   frostd endpoint. Knows no keys and reads no content.
3. **Chain adapters (site).** One per chain: read state, build proposals,
   build final transactions. Squads and Safe are ports of our Rust code
   (`zynzapd/src/squads.rs`, `eth_bridge.rs` / `zyn-eth-bridge.rs`).
4. **Site UI.** Create or import, one queue across chains, balances,
   history, members.

### Signer keys (Nap)

From the existing 24-word BIP-39 phrase, on standard paths so a member can
recover a signer in another wallet if the app disappears:

- Solana: Phantom's path, `m/44'/501'/i'/0'` (Ed25519, SLIP-10).
- Ethereum: MetaMask's path, `m/44'/60'/0'/0/i` (secp256k1, BIP-32).
- Zyn: Nap's existing account derivation.
- Zcash: FROST key packages from distributed key generation, sealed in Nap's
  encrypted backup (`exportKeys`/`sealWallet`), not derivable from the phrase.

`i` is one index per multisig membership, so a person's signers in two
multisigs cannot be linked to each other. Nap records which index belongs to
which multisig in its sealed state; a restore rediscovers them by scanning
indexes against the multisigs the site lists.

New in Nap: deriving these keys; signing Solana transactions and EIP-712
SafeTx hashes with them; FROST rounds; new Nap Connect methods and approval
cards (section 6). Today Nap signs neither Solana nor EVM with its own keys
and has no secp256k1 account key.

## 4. Flows

### Import an existing Squads or Safe

1. Connect Phantom or MetaMask (read-only use). The site finds multisigs the
   wallet belongs to: Squads by member, Safe by owner. It reads members,
   threshold, balances.
2. Each member opens an invite in Nap, which derives their fresh signer for
   this multisig.
3. One owner-change proposal swaps each old wallet for its new signer,
   approved by the current owners the usual way (Squads app, Safe app, or
   here).
4. From then on members sign with Nap. Their old wallets are no longer owners.

### Create

- Solana: a Squads v4 multisig created directly through the program
  (on-chain creation fee read 19 Oct 2026: 0 lamports; the 0.1 SOL is the
  Squads app's own fee). Cost ≈ rent, ~0.003 SOL. Members are signers' keys.
- Ethereum: a Safe through Safe's official proxy factory, owners = signers'
  addresses; deployment gas repaid by the Safe on its first transaction.
- Zyn: `CreateShared` (section 5).
- Zcash: distributed key generation through frostd; the result is a shielded
  address held by the group.

### Every payment

1. A member proposes ("send 2 SOL to …"). It enters the encrypted queue.
2. Members approve in Nap (or another signer, section 7). Each signer shows
   what moves.
3. At the threshold the relay submits:
   - **Safe**: `execTransaction` with all signatures. The SafeTx's refund
     fields (`gasPrice`, `gasToken`, `refundReceiver` = relayer) repay gas
     from the Safe.
   - **Squads**: `vault_transaction_create` + `proposal_create` +
     `proposal_approve` per signer + `vault_transaction_execute`, signed
     offline by each member against durable nonces prepared when the
     proposal was made, submitted together. If the instructions do not fit
     one transaction, each transaction in the bundle gets its own nonce
     account (a nonce can be advanced only once per pre-signed
     transaction); the relayer is the nonce authority and creates and
     closes them. The vault transaction includes a small transfer repaying
     the relayer's fees and nonce rent.
   - **Zyn**: each approval is a ledger op; the one reaching the threshold
     executes.
   - **Zcash**: the proposer builds a PCZT; FROST round 1 happens on
     approval, round 2 completes in each member's Nap the next time it is
     open. The app shows "waiting for X to come online".
4. History shows who approved what, to members only.

Rejection: members reject, or a proposal expires. On Squads, advancing the
nonce for anything else cancels a pending bundle.

## 5. Zyn native shared accounts (new ledger ops)

- `CreateShared {members, threshold, nonce}`: account id derived from its
  creation; holds any Zyn asset; registers a viewing key so its Zcash deposit
  address credits directly (as Treasury 26460 now does).
- `Propose {shared, action, nonce}`. Version 1 actions: pay (any asset, to a
  Zyn account, or out through a bridge), Jade / Lend / swap actions, add or
  remove a member, change the threshold.
- `Approve {proposal}`: signed by a member's own Zyn account. The approval
  reaching the threshold applies the action. `Reject` closes a proposal once
  the threshold can no longer be reached.
- Visibility: proposals and approvals are not public; the operator sees them
  (the standard Zyn sentence, said in the same breath).
- Treasury 26460 can later move to a shared account with the same three
  members, ending hand-run threshold ceremonies for it.

These ops go through the normal objectvm path: tests in `objectvm/tests`,
replicas and ZynScan upgraded before the node (the usual order).

## 6. Nap Connect additions

| Method | Card shows | Signs |
|---|---|---|
| `msig_signer` | which multisig, chain, the derived signer address | nothing; returns the public key/address |
| `msig_solanaApprove` | decoded vault transaction: each transfer, recipient, the fee repayment, the nonce | the offline Squads bundle message |
| `msig_safeSign` | decoded SafeTx: to, value, data decoded where known, refund fields | the EIP-712 SafeTx hash |
| `msig_zynApprove` | the decoded action | the Approve op |
| `msig_frost` | DKG round, or the PCZT's outputs and amounts | a FROST round message |

Rules: Nap decodes and never trusts the site's description; owner changes
show who is added and removed; anything it cannot decode is shown as such and
needs a separate confirmation. Same consent model as today's cards.

## 7. Signers other than Nap

- Squads: the Squads app, Phantom, a Ledger. If a member approves in the
  Squads app, that proposal becomes public on-chain; the site says so.
- Safe: the Safe app, MetaMask, a Ledger. An "also post to Safe" switch per
  Safe (off by default) publishes to Safe's Transaction Service for members
  who live in Safe{Wallet}; it needs Safe's API key and makes pending items
  public.
- Zcash: Zcash Foundation FROST (`frost-core`, RedPallas ciphersuite), the
  frostd protocol (end-to-end encrypted, untrusted server), PCZT for
  transactions. Any frostd-speaking signer can take part: Nap, ZF's audited
  `frost-client`, wallets integrating frostd. Zkool: its docs do not say how
  members coordinate; check whether it speaks frostd, else ask its developer.
- Zyn: approvals are ordinary signed ops; any Zyn wallet can approve.

## 8. The relay

- Stores, per multisig: encrypted proposals, approvals and collected
  signatures; FROST session messages (frostd semantics). A group key per
  multisig, wrapped to each member's X25519 key (from their Nap phrase),
  rotated when members change. The relay never holds it.
- Submits fully signed transactions as fee payer (Solana) or `execTransaction`
  sender (Ethereum), and checks the repayment is in the transaction before
  paying for it.
- Holds no authority: every signature is checked on-chain or by members' Nap.
  If the relay disappears, the team still has standard Squads, Safe and FROST
  multisigs and can continue elsewhere.
- Rate limits per multisig and per IP; no accounts, no email.

## 9. What we may say (docs/MESSAGING.md rules)

- "Your signers aren't linked to your personal wallets": true unless a member
  funds their signer; members never need to, and the site warns if one is
  funded.
- "Pending proposals aren't public": Safe while "also post to Safe" is off;
  Squads while members sign here; Zyn with "the operator sees it today";
  Zcash fully shielded.
- Never claimed: private balances or transfers on Solana or Ethereum.
  Executed transactions there are public; said inline before create/import.
- Zyn ledger items keep the standard trust sentence.

## 10. Testing

- Squads on devnet, Safe on Sepolia, Zyn on testnet 212, Zcash on testnet.
- End-to-end, three browsers, 2-of-3: import, owner swap, propose, approve,
  execute, reject, expiry; one member signing in the Squads or Safe app; one
  Zcash member in `frost-client`; a used nonce failing safely.
- Nap unit tests: derivation vectors against Phantom and MetaMask (same phrase
  → same addresses); decoders refuse what they cannot parse.
- objectvm tests for the shared-account ops; replica and ZynScan agreement.
- Then mainnet with our own Squads, Safe and treasury.

## 11. Build order (separate plan per part)

1. Signer core in Nap (keys, decoding, signing, Connect methods).
2. Queue relay.
3. Squads + Safe adapters and site (import, create, queue). First usable
   version; our own Squads and Safe move here.
4. Zyn shared accounts (ledger ops + adapter).
5. Zcash FROST (frostd endpoint, Nap rounds, PCZT).

Rough size: 6–8 weeks for all; parts 1–3 about 3–4 weeks.

## 12. Open items

- Product name and domain.
- Zkool interop (frostd or not).
- Ironwood (NU7) support in the FROST and PCZT libraries; Orchard first if
  Ironwood is not ready.
- Squads' program authority can raise the creation fee at any time; read it
  at create time and show it.
- Squads bundle size: how many approvals fit one transaction; a sequence of
  nonce-ordered transactions otherwise.
- Our Squads has a bridge member (`8HVm…`) that proposes payouts: keep it a
  member when the owners switch to Nap signers.
