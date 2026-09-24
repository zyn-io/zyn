# Running the ZEC side

## Nap recovery/browser pre-release — 12 September 2026

See `docs/NAP-RELEASE.md` for the #15/#16 test and recovery runbook. Preserve
`nap-restore-pending.json` and its protected `restore-backup-*` directory after
an interrupted restore; restart Nap to roll back before loading either key.
Never delete a journal to bypass a recovery failure. Browser fixture tests use
port 4174 and public test phrases only. Claims/migration are not event-ready.

What to install and in what order, to get from this repo to a chain it can
watch. Everything here is outside the VM by design — see `DECISIONS` §7b.

**Nothing in this document has been run against mainnet.** Version numbers and
disk figures move; check them against upstream rather than trusting this file.

---

## 0. What may be claimed, and what is gated

**Gated on privacy tier 4 (A108): Zyn as private money.** Nobody holds a
spendable balance here under a privacy claim until their amounts and ownership
are hidden from the operator as well as from the public (§130.8).

**Not gated on tier 4:** the Cave sale, which opens on tier 1 plus Gate 2's
security items — the tier §70.1 selected for that product, and honest for a
collectible whose allocation record is already blind (§130.8a). Nor ZynZap or
the launchpad, whose constraint is their copy rather than their tier (§130.8b).
Nor Nap, whose Zcash half is private by Zcash's own guarantee — though its Zyn
tab inherits Zyn's tier, and its `--private` tint currently spans both (§130.8d,
A109). Nor testnet, `zec-multisig`, the grant, or funding the vault.

**Gated on A90, and time-critical:** any further epoch published in plaintext.
Sealing cannot be applied backwards — what is published unsealed stays readable
forever (§70.8).

**What an operator may truthfully say today**, and it is less than the site
implies:

| Asked | The true answer, this week |
|---|---|
| Is my deposit private? | Yes — it is a shielded Zcash note, and Zcash protects it |
| Is what I do inside Zyn private? | **No.** Every `Transfer` names sender, recipient and asset in the journal, and today's DA mirror ships those in plaintext (§92.1, A90) |
| Who can see my holdings? | Today, anyone who can fetch a bundle. After A90 deploys, the operator and every verifier holding `ZYN_DA_KEY` |
| Is my withdrawal private? | Yes — shielded again on the way out |

Privacy is lost *inside*, not at the boundary. §92.3's discipline is to name the
tier you are actually on; do not let a UI, a post or a sale page claim the tier
above the one that is deployed.

---

## 1. The node: Zebra

Use **Zebra** (`zebrad`), the Zcash Foundation's Rust implementation.

**Do not start a new deployment on `zcashd`.** ECC has been winding it down in
favour of Zebra plus a separate wallet; building custody on a component being
retired is a migration you would rather not schedule. Confirm the current status
before committing either way.

Zebra also happens to match this stack — same language, same ecosystem as the
`zcash_*` crates the watcher will use.

```
git clone https://github.com/ZcashFoundation/zebra
cargo install --locked zebrad
zebrad start
```

**Requirements**, from [Zebra's own docs][zreq] — recheck, they move:

| | minimum | recommended |
|---|---|---|
| CPU | 2 cores | 4 cores |
| RAM | 4 GB | 16 GB |
| Disk | **~30 GB testnet** (measured), ~300 GB mainnet | SSD/NVMe, room to double |
| Network | | 100 Mbps, ~300 GB/month |
| Ports | 18233 testnet | 8233 mainnet |

**Zebra's docs say ~10 GB for testnet. Measured, it is 30 GB and still
growing** — the testnet chain passed 4.09 million blocks. A 40 GB server filled
completely and left `zebrad` in a restart loop, 248 times, syncing to within a
few thousand blocks of the tip and then unable to write.

Treat a documentation figure about a live chain as an estimate. Budget **60 GB
for testnet**, and prefer an attached volume to a bigger server: it is cheaper
per gigabyte and can be resized later without touching the machine.

Syncing from a state snapshot temporarily needs about twice the disk while the
archive extracts.

[zreq]: https://zebra.zyfnd.org/user/requirements.html

### Oracle Cloud free tier

**Yes for testnet, and better than the laptop** — with one caveat that is not
theoretical.

Always Free gives **2 OCPU of Ampere A1 (ARM) and 12 GB RAM**, plus **200 GB**
of block storage. Against Zebra's own numbers that is above the stated minimum
(2 cores / 4 GB), below the recommendation (4 cores / 16 GB), and twenty times
the disk testnet needs. Zebra reportedly runs on an Orange Pi Zero 2W, which is
far weaker. The Zyn node beside it is negligible — 64 bytes per account and a
27 ms seal at 50k accounts.

It also fixes the laptop's real problem: it does not sleep, move, or get closed.

**The caveat.** Those numbers were **4 OCPU / 24 GB until 15 June 2026**, when
Oracle halved them — with no blog post, no notification, and running instances
shut down. Users found out when their machines stopped. Treat the free tier as
a platform that can change under you without warning, because it demonstrably
does.

Two consequences:

1. **Provision early.** ARM A1 capacity is the scarce resource; "out of host
   capacity" is the normal experience. Getting the instance is harder than
   using it.
2. **Idle instances get reclaimed** on Always Free accounts. A quiet testnet
   node can look idle. Upgrading to Pay As You Go keeps the Always Free
   resources free while removing that risk — worth checking the current terms
   before relying on it.

Losing the box costs a 10 GB resync and a state restore, which
`zyn::store::FileStore` is built for: it refuses a blob that does not commit to
its recorded root, so a resumed node cannot quietly diverge.

**What must not go there.** Signing shares. §6d.1 puts custody outside
governance; the same reasoning puts it outside infrastructure nobody controls.
A threshold share on a machine that can be silently reclaimed is a share you do
not own, and 15 June is the proof. Signers belong on machines you control —
ideally on machines *different people* control.

### The Frankfurt box (existing Oracle instance)

**Zebra does not fit. The Zyn node does.** They are separate processes that talk
over RPC and were never required to share a machine, so this splits cleanly.

| | Frankfurt box | Zebra needs |
|---|---|---|
| Shape | E2.1.Micro, 2 vCPU | 2 cores (min) |
| RAM free | **~520 MB** | **4 GB (min)** |
| Swap | **none** | — |
| Disk free | 41 GB | 10 GB testnet |

Disk was never the constraint; memory is, by a factor of eight against Zebra's
own floor. And with no swap the failure is not slowness — it is the OOM killer,
which **does not necessarily choose the process that caused it**. Attempting it
risks taking Levu down rather than failing honestly. That box's own notes record
learning this expensively once already, where the symptom was wrong prices
rather than an out-of-memory error.

The ARM shape that would fit (`VM.Standard.A1.Flex`) reports
`OUT_OF_HOST_CAPACITY` in all three Frankfurt availability domains on repeated
checks, and the micro quota is spent. There is no free box to move to.

**So Zebra goes elsewhere**, in rough order of preference: a small paid instance;
the M1 Air during development, accepting that it sleeps; or a public testnet
lightwalletd endpoint early on — acceptable while nothing of value is custodied,
and never once something is.

**The Zyn node is a good fit for the Frankfurt box.** Measured, it is 64 bytes
per account, a 27 ms seal at 50k accounts, and 3.2 MB of account state at that
size — a few hundred MB of headroom is ample. Follow that box's four
conventions: a shell-less system user owning only its state directory,
root-owned binaries, bind loopback and let Caddy terminate TLS, and its own
`conf.d` fragment validated before reload.

`MemoryMax=` and `CPUQuota=` on the unit are not optional here. On a machine
with no swap, a service that stops cleanly is worth more than one the kernel
shoots at random — and the neighbour is someone else's exchange.

**One thing to plan for:** DA snapshots run about **0.4 GiB/day**, so 41 GB of
free disk is roughly **100 days**. That needs a retention policy or off-box
storage before it needs one urgently.

**And not signing shares.** §6d.1 keeps custody out of governance; the same
reasoning keeps it off a shared free-tier machine whose terms changed silently
in June and whose neighbour is another project.

### Is an M1 MacBook Air enough?

**For testnet, comfortably.** An M1 Air is 8 cores and 8–16 GB against a
recommendation of 4 cores and 16 GB, and testnet wants 10 GB of disk. It will
sync in hours, not days.

**For mainnet, no** — and not because of speed. Three reasons, in order of how
much they matter:

1. **It is a laptop.** It sleeps, it moves, it gets closed. A sequencer that
   stops sealing is a chain that stops settling.
2. **Disk.** 300 GB and growing, against a 256 GB base model that is also the
   machine's only volume.
3. **The signer set must not be one machine.** Threshold custody whose shares
   all live on the same laptop is a single key with extra arithmetic
   (`DECISIONS` §7d). This is the one that does not go away with a bigger Mac.

So: **testnet on the Air, then move.** The Air is genuinely the right machine
for phase one, and genuinely the wrong one for phase two.

---

## 2. Reading the vault's deposits

A shielded deposit is not public. Seeing one means holding the vault's **incoming
viewing key** and trial-decrypting note ciphertexts. Two ways in:

**Directly against Zebra.** Fetch blocks by RPC and decrypt with
`zcash_client_backend`'s scanning. Fewest moving parts; you do the block
handling yourself.

**Through `lightwalletd`.** Run your own instance in front of your own Zebra —
never someone else's, for a vault — and consume compact blocks over gRPC.
Compact blocks are built for exactly this: enough to trial-decrypt, far less to
transfer. More infrastructure, much less bandwidth.

Either way the crates are the same:

| Crate | For |
|---|---|
| `zcash_client_backend` | block scanning, note decryption |
| `zcash_protocol`, `zcash_primitives` | core types, consensus parameters |
| `orchard`, `sapling-crypto` | pool-specific note handling |

### Wiring it to this repo

Implement `zyn_custody::watcher::ChainView` — three methods:

```rust
fn tip(&self) -> u64;
fn deposits_at(&self, height: u64) -> Vec<ObservedDeposit>;
fn balance_at(&self, height: u64) -> Option<Fixed>;
```

`deposits_at` is the whole integration: fetch the block, trial-decrypt each
output with the viewing key, and for each success read the memo with
`zyn_custody::memo::decode` to learn which Zyn account it credits. A memo that
does not decode is **not a deposit** — it is a manual refund, and the watcher
never guesses.

Everything above that seam is already written and tested: confirmation depth,
reorg handling, deduplication, index assignment, and the observe-then-credit
order the VM requires.

---

## 2b. Where these processes go

The obvious move is one box with Zebra, the watcher and the Zyn RPC on it. That
is right for testnet and wrong for mainnet, for one reason: **the watcher holds
the vault's incoming viewing key.**

That key cannot move funds. What it can do is read every deposit into the
vault — amounts, memos, and therefore which Zyn account each belongs to. On a
chain whose entire premise is privacy, that is the most sensitive material in
the system after the spending shares. It does not belong on a machine serving
untrusted traffic from the internet.

So, two roles that want different things:

| | Zebra + watcher | Zyn sequencer + RPC |
|---|---|---|
| Holds | the **viewing key** | no Zcash key at all |
| Exposure | none — outbound peer-to-peer only | public |
| Bound by | disk and IO | latency, connection count |
| Scales by | not scaling; run two | adding replicas |

**Box A — private.** Zebra plus the watcher. No public ingress; it dials out to
Zcash peers and talks to the sequencer over a private network. Wants dedicated
**NVMe**, not network-attached storage: the initial sync is IO-bound, and the
difference is a day against a week. Several hundred GB with room to double,
comfortable RAM. This is the machine worth paying for.

**Box B — public.** Sequencer and RPC. The measurements say one core and a few
hundred MiB (`DECISIONS` §7c), so it is small; it is public-facing, so it should
hold nothing that matters. Read replicas go beside it as traffic arrives, each
rebuilding its `AccountIndex` once per epoch.

**The signers are neither**, and are not discussed here beyond: not on A, not on
B, not on each other (§7d).

One box for testnet is fine and much less work. Split before the vault holds
anything, not after — moving a viewing key between machines is a worse day than
provisioning a second one.

---

## 3. The vault key ceremony

`zyn_custody::ceremony` runs FROST distributed key generation over RedPallas —
the scheme Orchard spend authorization uses. In this repo the rounds run in one
process for testing; a real ceremony runs the same sequence with the maps as
messages between separate machines.

What the code cannot give you, and what actually matters:

- **Ten independent participants**, on different providers and jurisdictions
  (§7d). Ten shares on one provider is a 1-of-1 system wearing a costume.
- **Air-gapped generation**, shares never leaving the machine that made them.
- **Backups** that are themselves threshold-protected. A share backed up to one
  place is a share with a single point of failure.
- **A verified group key.** Every participant must confirm the same one; the
  ceremony refuses to finish otherwise, because a vault only part of the group
  can sign for is a loss discovered at the worst moment.
- **A rehearsal on testnet**, end to end, including a signer being unavailable.

Distribute the **incoming viewing key** to every signer at the same time. That
is what lets each verify a deposit before endorsing the epoch that releases it,
and it is what makes the 7-of-10 seven checks instead of seven signatures
(`DECISIONS` §10.13).

---

## 4. Order of operations

1. **Sync Zebra on testnet.** Long, and nothing proceeds without it.
2. Implement `ChainView` against it; test against testnet deposits with a
   throwaway key.
3. Run the ceremony on testnet. Rehearse a signer being offline.
4. Run a full cycle: deposit → credit → anchor → trade → exit → payout.
5. Rehearse the failures: a reorg, a missing signer, a stalled sequencer, an
   exit that times out.
6. Only then mainnet, with `Vault::cap` set low and raised slowly — dust first,
   ours only, as a drill.
7. **Privacy tier 1 (A90, A94, P2) before the sale opens** (§0). A90 first: it
   cannot be applied to epochs already published.
8. **Privacy tier 4 (A108) before Zyn is offered as private money** — a
   spendable balance held here under a privacy claim. Not required for steps
   1–7.

Steps 4 and 5 are where the remaining unknowns are. Everything before them is
either written or a package install. Step 8 is the long one, and it is the only
step that stands between a working chain and the claim the brief makes.

---

## 4a. The deployed testnet node

Provisioned 4 September 2026. **This bills by the hour — destroy it when idle.**

| | |
|---|---|
| Host | `zyn-zebra-testnet`, Hetzner `cax11`, `fsn1` |
| Storage | 40 GB boot + **60 GB volume** `zyn-zebra-state`, chain state on the volume |
| Address | `168.119.53.39` |
| Cost | **€0.0118/hour** (€7.37/mo if left running) |
| OS | Ubuntu 24.04, aarch64, 2 cores, 3.7 GB, 38 GB disk |
| Zebra | `zfnd/zebra:latest` (arm64) in Docker, `--restart unless-stopped` |
| Access | SSH key `tobiasd@guanyis-mbp.home`, already yours |

```
ssh root@168.119.53.39
zyn-status          # chain, height, cache size, memory
docker logs -f zebrad
```

**4 GB swap was added.** The box has 3.7 GB usable against Zebra's 4 GB minimum,
and the Frankfurt box's own notes are explicit about what a tight fit with no
swap produces: the OOM killer, choosing whichever process it likes. Swap turns
that into slowness.

### The firewall, and the port that is not in it

`zyn-zebra` allows **22** and **18233** inbound, and nothing else. **18232, the
RPC, is deliberately absent** — it binds `127.0.0.1` as well, so it is closed
twice over. Cookie auth is disabled, which is only acceptable *because* of
those two facts; anything that opens that port must re-enable it first.

A bridge running elsewhere therefore reaches the RPC over an SSH tunnel, or
`zynzapd` runs on this same box. Not by opening 18232.

### Verified against the real node, not assumed

Every method `zyn_custody::zebra` depends on, called against this instance:

| method | result |
|---|---|
| `getblockcount` | bare number in `result` |
| `getblockchaininfo` | `chain: "test"` — confirms testnet |
| `getaddresstxids` | `[]`, no error — supported |
| `getaddressbalance` | `{balance, received}` |
| `getrawtransaction` verbose | `height`, `vout[].valueZat`, `vout[].scriptPubKey.addresses[]` |

That last row was the real risk and it is worth recording exactly. A **P2PKH**
output carries `addresses`:

```json
{"valueZat":600050000,
 "scriptPubKey":{"type":"pubkeyhash","addresses":["tmGf1r6Bo9HFdssAcFpYPehzcn1QyfurEeH"]}}
```

A coinbase **`pubkey`** output does not carry `addresses` at all. Our parser
skips any output without it, which is correct — but had it required the field
unconditionally it would have failed on the first block, and had it fallen back
to parsing the script it would have been guessing. `valueZat` is present
throughout, so the refusal to read money from the sibling `value` float costs
nothing.

### Stopping it

```
hcloud server delete zyn-zebra-testnet
```

That releases the primary IP with it. Check with `hcloud primary-ip list`
afterwards — an orphaned IPv4 is the usual way a deleted server keeps billing.

To keep the synced chain without paying for the server, snapshot first:

```
hcloud server create-image --type snapshot --description zebra-testnet zyn-zebra-testnet
```

Snapshots bill per GB-month and a testnet chain is small, so this is the cheap
way to avoid re-syncing.

---

## 4b. Running the node

`zynzapd` is the node as a process. It sequences, executes, seals, persists and
serves — and it does **not** talk to Zcash. What it runs is a **devnet**: every
unit of `ZEC.zy` comes from an operator intent rather than an observed deposit
and is backed by nothing. Everything above custody is real. Do not put value in
it.

```
cargo build --release -p zynzapd
ZYN_CHAIN_ID=42 ZYN_LISTEN=127.0.0.1:8199 ZYN_DATA_DIR=/opt/zynzap/state \
  ./target/release/zynzapd
```

| variable | default | |
|---|---|---|
| `ZYN_CHAIN_ID` | `1` | |
| `ZYN_LISTEN` | `127.0.0.1:8099` | Loopback on purpose — put Caddy in front. |
| `ZYN_DATA_DIR` | `./zyn-data` | |
| `ZYN_SEAL_EVERY` | `256` | Intents per epoch. Bounds what a hard kill loses. |
| `ZYN_SEAL_AFTER_SECS` | `60` | The failsafe that makes an epoch expiry mean wall-clock time (**S12**). |
| `ZYN_ANCHOR_EVERY` | `30` | Epochs per anchor. |
| `ZYN_SAVE_EVERY_SECS` | `30` | On top of the save every seal performs. |
| `ZYN_PARAMS` | `testnet` | `testnet` or `v1`. Applies to a *new* chain only — parameters are committed state (**S5**). |
| `ZYN_VAULT_SEED` | *unset* | Hex Orchard spending key. Selects the **shielded** path — the design. |
| `ZYN_BRIDGE_ADDRESSES` | *unset* | Transparent scaffold. Mutually exclusive with the seed. Neither set = devnet. |
| `ZYN_BRIDGE_FROM_HEIGHT` | `0` | Never scan below here. Trial decryption from genesis does not finish. |
| `ZYN_BRIDGE_MAX_BLOCKS` | `200` | Blocks per pass. A pass cut short resumes; it is not lost. |
| `ZYN_ZEBRA_HOST` / `ZYN_ZEBRA_PORT` | `127.0.0.1` / `18232` | |
| `ZYN_ZEBRA_USER` / `ZYN_ZEBRA_PASS` | *unset* | Basic auth, if the node wants it. |
| `ZYN_CONFIRMATIONS` | `10` | Depth before a deposit is credited. |
| `ZYN_BRIDGE_POLL_SECS` | `30` | Between deposit scans. |

### What it does and does not expose

Reads are public — status, pools, a quote, a snapshot, an account proof. Writes
need a signature, and the account is **recovered from it** rather than supplied
(**S14**). The control-plane operations that assert operator authority —
`ApplyBatch`, `Checkpoint`, `Restore`, `State` — are **not reachable over the
socket at all**, and answer as much:

```
$ ... op 1 ...
operator operation, not available over the network
```

### Durability, verified rather than asserted

- Saves on every seal and on `ZYN_SAVE_EVERY_SECS`.
- `SIGINT`/`SIGTERM` seal the epoch in flight, save, and exit — so a restart
  resumes on an epoch boundary rather than mid-epoch.
- Resuming reproduces the same state root.
- A corrupted state file is **refused**, with a non-zero exit so systemd sees a
  failure rather than a silent restart into a state nobody can reproduce.

### Under systemd, on the Frankfurt box

Follow that machine's conventions (§1) and cap it, because there is no swap:

```
[Service]
User=zynzap
ExecStart=/opt/zynzap/bin/zynzapd
Environment=ZYN_LISTEN=127.0.0.1:8199
Environment=ZYN_DATA_DIR=/opt/zynzap/state
MemoryMax=400M
CPUQuota=25%
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/zynzap/state
Restart=on-failure
```

`KillSignal` defaults to `SIGTERM`, which is what the clean-shutdown path
listens for. Leave it alone.

---

## 4c. Testing with faucets

The right way to prove a bridge: real testnet funds, no value at risk.

### Zcash — TAZ

| | |
|---|---|
| [zcashfaucet.jinolabs.xyz](https://zcashfaucet.jinolabs.xyz/) | **0.1 TAZ**, shielded z2z, browser proof-of-work |
| [fauzec.com](https://fauzec.com/) | mines its own TAZ; sends to Unified and Sapling addresses |
| Zcash Discord | ask, per the official testnet guide |

**Two things to settle before requesting anything.**

**Which pool the funds arrive in.** Our custody is Orchard — `zyn-custody` is
FROST over RedPallas, which is Orchard's spend-authorisation scheme. A faucet
that sends *Sapling* lands funds our watcher will not see, and the symptom is
silence rather than an error. Give the faucet a **Unified Address containing an
Orchard receiver** and confirm on an explorer which pool actually received,
before concluding anything about the watcher.

**Our own floors.** `Params::v1` sets `min_pool_xzec` to one whole unit. At 0.1
TAZ a request that is ten proof-of-work grants before a single pool exists —
`v1` is not inconvenient on testnet, it is a wall. Use `ZYN_PARAMS=testnet`
(the daemon's default), which drops the floor to 0.001 and changes **nothing
else**: fees, hop limits, staleness and `exit_timeout_epochs` stay exactly as
production has them, because a testnet that behaves differently tests a
different program.

### Solana — devnet SOL

[faucet.solana.com](https://faucet.solana.com/) gives **5 SOL twice an hour**,
more with a GitHub sign-in, plus `solana airdrop 2 --url devnet` from the CLI.

That is roughly **a hundred times** more accessible than the Zcash faucet, and
it compounds with three other advantages: Solana deposits are **transparent**,
so there is no viewing key and no note decryption; finality is a commitment
level rather than a confirmation depth; and there is nothing to mine.

### What this implies for order of work

`ZEC.zy` still ships first — that is a product decision (`DECISIONS` §6b) and it
does not move. But the **generic** half of the bridge is shared: `Vault`, the
`Watcher` state machine, exits, refunds, dust minima and settlement grouping are
all chain-agnostic, and `Watcher` is generic over `ChainView`.

So the cheapest way to prove that shared half is against the chain where funds
are free and deposits are readable, leaving only the Zcash-specific work —
note decryption and Orchard spend assembly — to debug against the chain where
they are neither. That is a testing choice, not a reordering of the product, and
worth keeping labelled as such so it does not drift into one.

### The SOL.zy / ZEC.zy pool

Last, and it needs three things at once: both bridges live, both assets issued,
and a reference price. The third is already built — `Pool.reference` and
`amm::divergence_fee` — so the pool itself is not the work; having two working
bridges is.

### One habit not to form

Testnet coins have no value, so it is tempting to run the signer ceremony with
every share on one machine. Do it once to prove the ceremony, then stop.
`DECISIONS` §7d and §6d.1 both say custody must not live on one box, and a
rehearsal that skips the property being rehearsed is not a rehearsal.

---

## 4d. The EVM vault

> **Phase-1 deployment deferred (DECISIONS §69).** Keep this implementation and
> its test vectors healthy, but do not deploy or fund a production EVM vault as
> part of the Phase-1 launch. Ethereum and BOLD.zy return to the critical path
> before Phase 2; this section remains the rehearsal procedure for that work.

Built and tested, not yet deployed. Full procedure in `contracts/README.md`;
what matters operationally:

**One address on every EVM chain.** Deploy through the deterministic CREATE2
proxy at `0x4e59b44847b379578588920cA78FbF26c0B4956C` with a fixed salt and
identical bytecode. Identical bytecode means identical compiler settings, which
is why `contracts/foundry.toml` pins `solc` and the optimizer rather than
leaving them to whatever is installed.

**Order.** Deploy, `registerAsset` for each asset, then accept deposits. An
asset id cannot be corrected once registered — only a new id added alongside —
so check the token address twice before the first call.

**Two keys, and neither is the other's backup.** `signer` authorises
withdrawals and is immutable; `admin` registers assets and pauses deposits and
can do nothing else. Start with a Safe as `signer`: its owner set rotates
without changing its address, so operator turnover never touches the vault.

**Nobody can pause withdrawals**, by construction. If exits need to stop, that
is a decision made on Zyn by not producing settlements — visible in state, not a
switch on a contract.

**Testnets first**: Base Sepolia (`ORIGIN_BASE_SEPOLIA`, chain 84532) alongside
the Zcash testnet node in §4a. `Rpc::connect` refuses a mainnet chain id, and
checks the endpoint really is the chain configured — the vault has the same
address everywhere, so a wrong URL finds a *real* contract with real logs.

**Running the watcher.** The RPC URL is the switch; nothing starts because some
other variable happened to be set.

```sh
ZYN_EVM_RPC_URL=https://sepolia.base.org \
ZYN_EVM_CHAIN_ID=84532 \
ZYN_EVM_VAULT=0x...            # the deployed vault
ZYN_EVM_ASSET=1                # Zyn asset id these deposits credit
ZYN_EVM_TOKEN=0x...            # omit for the chain's native asset
ZYN_EVM_DECIMALS=6
ZYN_EVM_FROM_HEIGHT=...        # the vault's deployment block
```

This runs **alongside** the Zcash bridge, not instead of it — `zynzapd` now
drives a list of bridges, one per watched chain, each with its own asset,
ceiling and progress file. That is the second bridge §14b was waiting for.

**Confirmations are zero here, deliberately.** The watcher's tip is the
`finalized` block. Depth-counting would be worse than unnecessary: it accepts
blocks finality has not covered while claiming a safety it cannot give.

**If upgrading a node that already has a bridge**, its `bridge-<chain>.progress`
is adopted on first load and rewritten keyed by asset. Nothing to do by hand,
but do not delete it — an empty dedup set re-credits every deposit it forgot.

```sh
cd contracts && forge test          # 19 tests
cargo test -p zyn-bridge            # 30, including the 13 EVM ones
```

The Rust and Solidity digests are tied by a shared vector. If either encoding
changes, regenerate both — a mismatch surfaces only as every signature failing
on a chain that already holds funds.

## 4e. The Solana bridge

> **Production deployment deferred (DECISIONS §69).** Devnet remains useful for
> regression and custody testing, but SOL.zy is not a Phase-1 product dependency.
> Do not let maintaining this path displace Cave secondary trading, the Cave
> launchpad or ZynZap graduation work.

Deposits and exits. The first asset that can leave — see `DECISIONS` §15.

**No program to deploy.** The vault is a plain account and the recipient comes
from a Memo-program instruction, as on Zcash. There is no upgrade authority to
hold or burn because there is nothing to upgrade.

```sh
ZYN_SOLANA_RPC_URL=https://api.devnet.solana.com \
ZYN_SOLANA_CLUSTER=devnet \
ZYN_SOLANA_VAULT=<base58 account> \
ZYN_SOLANA_ASSET=2 \
ZYN_SOLANA_FROM_SLOT=<slot the vault was funded at>
```

`connect` refuses mainnet-beta and checks the endpoint's **genesis hash**
matches the cluster configured — pointing a devnet watcher at testnet would
otherwise credit deposits that never happened where the vault is.

**Depositors must attach a memo**, or the funds are unattributable and need a
manual refund:

```
ZYN1:<32-byte Zyn account, 64 lowercase hex>
```

A UI produces this; nobody should type it. `solana transfer` takes `--with-memo`.

**Tell depositors the memo is public.** On Zcash it is inside the encrypted
note; here anyone can read it, so it links their Solana address to a Zyn
account. The account named need not be one they use elsewhere.

**Confirmations are zero**, as for EVM: the tip is the finalized slot. Skipped
slots are normal and are not treated as failures.

### Exits

The vault is a FROST-ed25519 key. Solana verifies its signatures as ordinary
ed25519, so there is nothing on-chain but an account and a **durable nonce**.

**1. The ceremony.** Produces one share per participant and prints the vault:

```sh
zyn-ceremony solana 2 3 /var/zyn/solana-shares     # -> vault address
```

In-process, so every share lands on this machine. That is a **devnet key in
three pieces**, and `zynzapd` says so at startup. A real vault runs each
participant's rounds on their own machine; the file format is the same.

**2. Fund the vault, then create its nonce account** with the vault as
authority — the vault is its own nonce authority, so a settlement has exactly
one signer:

```sh
solana airdrop 2 <vault> -u devnet
solana-keygen new -o nonce.json
solana create-nonce-account nonce.json 0.01 --nonce-authority <vault> -u devnet
solana address -k nonce.json                        # -> ZYN_SOLANA_NONCE
```

**3. Reveals.** An exit pays only to the address its account committed to
(`BindWithdrawal` holds `keccak(address ‖ salt)`). The operator needs the
preimage, one per line, and this file says where money goes — handle it like
a key:

```
<account-hex64> <solana-address-base58> <salt-hex64>
```

**4. Run.**

```sh
ZYN_SOLANA_SHARES=/var/zyn/solana-shares \
ZYN_SOLANA_THRESHOLD=2 \
ZYN_SOLANA_NONCE=<nonce account> \
ZYN_SOLANA_REVEALS=/var/zyn/solana-reveals
```

Each pass: pending exits → canonical settlement (`settle::group`,
oldest-first) → resolved against bindings → one transaction-sized group signed
against the nonce → **written to `settle-<chain>-<asset>.ledger`** → broadcast.
When the chain reports it *finalized*, `ConfirmWithdrawal` burns the units.
One transaction in flight at a time.

**Why a double payment cannot happen**, whatever crashes: every transaction
names the nonce's current value and consumes it, so two built against the
same value cannot both land. The ledger exists to remember which exits a
transaction covered, not to prevent a repeat — the chain does that.

**If the log says `NONCE ... WAS SPENT BY A TRANSACTION NOT IN THE LEDGER`**,
stop. Something other than this process signed with the vault's authority.
That is a second operator or a stolen share set, and settling again before
knowing which is the wrong move.

```sh
cargo test -p zyn-custody solana::     # watcher + signing, 16 tests
cargo test -p zynzapd settle           # ledger, 2 tests
# end to end against a local validator, ~35 s each:
solana-test-validator --reset --quiet &
ZYN_SOLANA_LIVE=1 cargo test -p zyn-custody --test solana_live
ZYN_SOLANA_LIVE=1 cargo test -p zynzapd --test solana_settle_live
```

## 4f. Zcash exits

`ZEC.zy` leaves through a **transparent** output — under NU6.3 the Orchard
pool cannot pay another shielded address (`DECISIONS` §17). Tell users.

**1. The ceremony**, which also writes the vault's viewing key:

```sh
zyn-ceremony zcash 2 3 /var/zyn/zcash-shares     # -> deposit address (utest1…)
cat /var/zyn/zcash-shares/viewing.hex            # -> ZYN_VAULT_FVK
```

The viewing key **must** be the threshold key's (`ceremony::orchard_viewing_key`);
a vault scanned with a seed's key receives notes the signers cannot spend. The
settler refuses a mismatch at startup.

**2. Deposits** go to that address with the memo from `zyn_custody::memo::encode`
(§4c for faucets). `ZYN_BRIDGE_FROM_HEIGHT` must be at or below the first
deposit's height: the note tree is seeded from the node at `from_height - 1`
and every commitment after it is appended by the scan.

**3. Reveals**: `<account-hex> <transparent address> <salt-hex>` per line, the
preimages of `BindWithdrawal`'s `keccak(kind ‖ hash160 ‖ salt)`.

**4. Run** — alongside the deposit settings in §4b:

```sh
ZYN_VAULT_FVK=<viewing.hex>              # instead of ZYN_VAULT_SEED
ZYN_ZEBRA_SHARES=/var/zyn/zcash-shares
ZYN_ZEBRA_THRESHOLD=2
ZYN_ZEBRA_REVEALS=/var/zyn/zcash-reveals
```

At startup the daemon seeds the tree from `z_gettreestate`, checks its root
against the node's, and walks the blocks the deposit watcher is already past.
The first settlement builds the proving key (seconds, hundreds of MB — the
cax11 has 4 GB; watch it). Each pass then: pending exits → settlement →
build, prove, sign, seal → `settle-<chain>-<asset>.ledger` → broadcast →
`ZYN_CONFIRMATIONS` deep → `ConfirmWithdrawal`. One transaction in flight.

**Expiry is the nonce.** A transaction unseen past its expiry height (built
height + 40) is abandoned and its exits rebuilt; the two cannot both land.

```sh
cargo test -p zyn-custody --release payout::            # envelope, real proof
ZYN_ZEBRA_LIVE=1 cargo test -p zyn-custody --test zebra_live   # tree vs testnet, needs the tunnel
cargo test -p zynzapd --release --test zcash_settle     # the daemon's loop, fake node
```

## 4g. The testnet loop, by hand

Everything a tester needs is `zyn-cli` against the public RPC
(`168.119.53.39:8099`, chain **11**). No wallet integration is required for
the demo; the CLI signs with a local ed25519 key over the same payload a
wallet would.

```sh
zyn-cli keygen  ~/zyn.key                 # -> your account id
zyn-cli memo    ~/zyn.key                 # -> ZYN1:<hex>  (the deposit memo)
zyn-cli status  168.119.53.39:8099 11
```

**Deposit.** Send testnet TAZ from any wallet (Zingo, YWallet) to the vault's
`utest1…` address printed by the node at startup, **with the memo above** in
the memo box. Wallets send to the Ironwood pool now; the vault reads both
pools (§18). Credit lands after `ZYN_CONFIRMATIONS` blocks and is spendable
once the epoch is anchored.

**Without the memo** the funds are unattributable and the scan stops at that
block (`zynzapd: deposit scan failed: … carry no Zyn memo`). The operator
resolves it in writing, with the depositor's account id:

```sh
echo "<txid> <account-hex>" >> /etc/zyn/zcash-attributions && systemctl restart zynzapd
```

`ZYN_ZEBRA_ATTRIBUTIONS=/etc/zyn/zcash-attributions` in the env file. The file
is the audit trail; keep it.

**Exit** — to your own shielded address (`utest1…`, any wallet's unified
address with an Orchard receiver) or a transparent one (`tm…`). Shielded
exits are paid from the Ironwood pool; if the vault only holds Orchard notes
a shielded exit waits and a transparent one goes through (§17, §18).

```sh
SALT=$(openssl rand -hex 32)
zyn-cli bind     168.119.53.39:8099 11 ~/zyn.key utest1… $SALT
zyn-cli withdraw 168.119.53.39:8099 11 ~/zyn.key 0.05 utest1… $SALT
zyn-cli reveal   ~/zyn.key utest1… $SALT       # -> one line; give it to the operator
zyn-cli balance  168.119.53.39:8099 11 ~/zyn.key
```

The node holds only `keccak(kind ‖ address ‖ salt)`; the reveal line is how
the operator learns where to pay, and it is checked against that commitment
before anything is signed. On the box:

```sh
echo "<reveal line>" >> /etc/zyn/zcash-reveals && systemctl restart zynzapd
```

The settler builds, proves, signs and broadcasts on its next pass; the exit
confirms on Zyn once the payout is `ZYN_CONFIRMATIONS` deep on Zcash.

## 4h. What is deployed (5 Sep 2026) — RETIRED

> **Chain 11 was retired on 22 Sep 2026.** `zynzapd.service` is stopped and
> disabled, `:8099` has no listener, and the testnet is now chain 212 (§5f-bis).
> This section is kept as the record of how that chain was deployed — its
> paths, its vault and its scan height are what a reader needs to interpret
> the history and the mirror's `chain-11/` tree. Do not follow it to stand
> anything up.

On `zyn-zebra-testnet` (168.119.53.39, cax11, Zebra in Docker on the 60 GB volume):

| | |
|---|---|
| unit | `systemctl status zynzapd` — `Restart=always`, enabled at boot |
| config | `/etc/zyn/zynzapd.env` (mode 600) |
| data | `/mnt/HC_Volume_106795140/zyn-data` — `chain-11.state`, `bridge-11-1.progress`, ledgers |
| shares | `/var/zyn/zcash-shares` — 2-of-3, **all on this box** (devnet key), plus `viewing.hex` |
| reveals | `/etc/zyn/zcash-reveals` — append a `zyn-cli reveal` line, then `systemctl restart zynzapd` |
| chain | **11**, public RPC `168.119.53.39:8099` (Hetzner firewall rule added) |
| vault | `utest1h6sfz7alnztp0vst6u0s8sxj9zhzrv5s7qjy5897qcdse75epp43x4uus90naxaea973q22nshuggyywujdj5zulckj2vppz85x2t407` |
| scanning from | 4,326,863 — deposits before that height are invisible to this vault |

Binaries are cross-compiled from a Mac with `cargo zigbuild --release --target
aarch64-unknown-linux-gnu -p zynzapd --bins` (~40 s) and copied to
`/usr/local/bin`. The box has no Rust toolchain and does not need one.

To redeploy: build, **`systemctl stop zynzapd`**, `scp` the three binaries
(a running executable cannot be overwritten), `systemctl start zynzapd`. The
state file and progress survive; a new binary resumes at the last seal.

**Rescanning.** `ZYN_BRIDGE_RESCAN_FROM=<height>` re-reads from that height on
the next start. Safe by construction: the dedup set, not the height, is what
prevents a re-credit. Remove it afterwards or every restart rescans.

**Both pools.** The daemon seeds an Orchard and an Ironwood tree and checks
each root against the node at startup (`… root matches the node`). A mismatch
refuses to start rather than witness against a tree the chain does not have.

**Done here** (5 Sep 2026): deposit `372abd69…`, exit `c8bc6fb7…` — the full
round trip. The proving key builds in ~13 s on the cax11 and the process sits
at ~40 MB afterwards. The vault's change from a payout is recognised as change
(it spends the vault's own note), not a deposit.

## 4i. A wallet on this machine (`zyn-wallet`)

`zyn-wallet` is a Zcash wallet built from the vault's own code: it scans
compact blocks from a `zyn-lightd` (§4o), keeps its own note trees on disk,
and spends v6 transactions from Ironwood when the recipient is shielded. It
works on either network; the server tells it which.

```sh
export ZYN_LIGHTD=168.119.53.39:8098          # the default; the testnet box
zyn-wallet new     ~/zyn-testnet.wallet        # utest1…, birthday = the tip
zyn-wallet address ~/zyn-testnet.wallet
zyn-wallet sync    ~/zyn-testnet.wallet        # notes and balance, incremental
zyn-wallet send    ~/zyn-testnet.wallet <to> <taz> [memo]
zyn-wallet reset   ~/zyn-testnet.wallet [birthday]   # rescan from a height
zyn-wallet backup  ~/zyn-testnet.wallet > ~/nap-testnet-backup.json
chmod 600 ~/nap-testnet-backup.json
zyn-wallet import-backup ~/restored.wallet ~/nap-testnet-backup.json
# Or restore standard BIP-39 words without putting them in shell history:
zyn-wallet import-mnemonic ~/restored.wallet <phrase-file-or-> <birthday> [account]
# Legacy raw Orchard keys remain accepted:
zyn-wallet import  ~/other.wallet <key-hex> <birthday>
```

New `<wallet>` files are mode-0600, versioned key records holding the entropy
for a 24-word English BIP-39 phrase. Orchard account 0 is ZIP-32
`m/32'/coin_type'/0'`; its transparent receiver is BIP-44
`m/44'/coin_type'/0'/0/0` from the same BIP-39 seed. Old 32-byte key files open
unchanged. `<wallet>.state` holds the network, birthday, both pool trees and
the held notes. Losing the state costs a rescan from the birthday; losing the
recovery phrase or backup costs the money. A wallet that has synced testnet
refuses a mainnet server, and the other way round.

`backup` writes a `nap-wallet-backup` version-1 JSON document containing every
restore input: network, birthday, account, key source and recovery material.
It is plaintext and controls the funds. For a mnemonic wallet it contains the
words and optional passphrase; for an old wallet it contains the raw Orchard
key. `import-backup` checks its version and network. `import-mnemonic` reads
words from a file, or from stdin when the path is `-`; set an optional BIP-39
passphrase in `ZYN_BIP39_PASSPHRASE`. Use a birthday at or before the first
receipt. A lower height costs scan time; a higher one misses funds.
Mnemonic restore covers account 0's Orchard and Ironwood notes and its
transparent receiver. Nap does not scan Sapling, so a phrase from a wallet
with Sapling funds will show only the pools Nap supports.

Scanning is incremental — each run continues from where the trees stopped,
up to three blocks behind the tip — and fast: the ~960-block history of the
first testnet wallet synced in under two seconds. A block with one of our
outputs is fetched in full (for the note and its memo); a spend is noticed
by its nullifier. After a `send` the spent notes leave the store at once and
the change shows up after three confirmations, so the balance dips to zero
in between — that is the state of the chain, not a bug.

**Depositing with the memo**, so the vault credits you with no human in the
loop:

```sh
zyn-wallet send ~/zyn-testnet.wallet <vault utest1…> 0.02 "$(zyn-cli memo ~/zyn.key)"
```

Sends spend Ironwood notes first and pay a shielded recipient from Ironwood in
a v6 transaction; a transparent recipient can be paid from either pool.

Verified 6 Sep 2026 from this Mac, no tunnel: reset to birthday 4,326,863,
sync found the 0.05 TAZ exit, the two spends and the change, balance
0.0288 TAZ; a 0.001 TAZ self-payment with a memo went out through the server
as `8037fd69…d654`.

## 4j. Solana on devnet (SOL.zy)

Deployed alongside the Zcash bridge on the same node. The daemon creates the
asset itself on first start with the operator-only `CreateBridgedAsset`, using
the exact public seeds `origin_network = "solana"` and empty `origin_asset`.
The resulting id is the published
`0f61f4340cd6a910ec8d7e73ef239569b83f16099e620786e71e7e7395c9f6df`,
not a runtime number, so `ZYN_SOLANA_ASSET` need not be set.

| | |
|---|---|
| vault | `7KacSYXuVhKSf8dpXcLY3qFZQYiZg2m6vKh4m1HxKZmA` — 2-of-3 from `zyn-ceremony solana`, shares in `/var/zyn/solana-shares` |
| asset | `SOL.zy`, id `0f61f434…95c9f6df` |
| watcher | `ZYN_SOLANA_RPC_URL=https://api.devnet.solana.com`, `ZYN_SOLANA_CLUSTER=devnet`, `ZYN_SOLANA_FROM_SLOT=<slot at deploy>` |
| nonce account | `BrRELAaQBLe3Yx3vNad8jjxDdJZTZ5SmwLHRFTFyd61H` (authority: the vault); settler armed |
| first deposit | `5WASwsyN…` 0.5 SOL with memo, slot 493,753,468 — credited with no human in the loop |
| first exit | `5Rkuyp6n…` 0.2 SOL to the payer, signed against the nonce, confirmed on Zyn 30 s later |

**Devnet SOL.** `solana airdrop` is rate-limited per IP and refused both this
machine and the server; <https://faucet.solana.com> (GitHub login) works. Fund
the payer `~/.zyn/devnet-payer.json` (`5KVxRb87TfDfpiEoQaPpGdcJDASa9MWxTjUYjQU5rZs6`),
then:

```sh
# the deposit, with the memo, straight from the CLI
solana transfer 7KacSYXuVhKSf8dpXcLY3qFZQYiZg2m6vKh4m1HxKZmA 0.5 -u devnet -k ~/.zyn/devnet-payer.json \
  --allow-unfunded-recipient --with-memo "ZYN1:<account-hex>"
# the nonce account, so exits can be signed
solana-keygen new -o ~/.zyn/devnet-nonce.json
solana create-nonce-account ~/.zyn/devnet-nonce.json 0.01 --nonce-authority 7KacSYXuVhKSf8dpXcLY3qFZQYiZg2m6vKh4m1HxKZmA -u devnet -k ~/.zyn/devnet-payer.json
```

Then on the box: `ZYN_SOLANA_SHARES=/var/zyn/solana-shares`, `ZYN_SOLANA_THRESHOLD=2`,
`ZYN_SOLANA_NONCE=<nonce address>`, `ZYN_SOLANA_REVEALS=/etc/zyn/solana-reveals`.

Exits from the user side: `zyn-cli bind-sol / withdraw-sol / reveal-sol`
(`zyn-cli withdraw-sol <rpc> 11 ~/zyn.key 2 0.1 <solana address> <salt>`).

**Redirect delay.** One binding per account (§20): binding a Solana address
after a Zcash one is a redirect and waits `exit_timeout_epochs`. The testnet
node runs `ZYN_EXIT_TIMEOUT_EPOCHS=10` (~10 min). Re-run `bind-sol` with the
same address after the delay to apply it, then `withdraw-sol`.

**Rate limits.** The public devnet RPC returns 429 under a per-slot scan;
the Solana view is address-indexed (§19) and retries 429s, and a pass in
which any call failed records no progress. A 429 at *startup* still exits
the process; systemd restarts it.

**Start heights.** Every bridge now honours its configured start
(`ZYN_SOLANA_FROM_SLOT`, `ZYN_EVM_FROM_HEIGHT`); Solana and most EVM
endpoints cannot serve old blocks, and a watcher that starts at zero fails
every pass.

## 4l. The first swap (step 2, done)

Pool 1 on the testnet node: `ZEC.zy : SOL.zy`, seeded 0.05 : 0.25 at 30 bps
by the operator's account. First swap: 0.01 `SOL.zy` → 0.00191 `ZEC.zy`.
`zyn-cli quote` reports the bracket a router needs — what solo execution
pays and what a perfectly netted batch would.

```sh
zyn-cli pools  168.119.53.39:8099 11
zyn-cli quote  168.119.53.39:8099 11 2 1 0.01           # asset-in, pool path, amount
zyn-cli swap   168.119.53.39:8099 11 ~/zyn.key 2 1 0.01 0.0017   # min-out from the quote
zyn-cli add-liquidity 168.119.53.39:8099 11 ~/zyn.key 1 0.01 0.05
```

So the loop the product is about has run on the real testnet: **SOL in,
swap, ZEC out shielded** — or the reverse — with nothing faked but the
key distribution.

## 4k. Items: mint and sell, privately

Ownership is hidden from everyone but the operator (§21): the published
snapshot is leaves only, and a record is served only to a request signed by
its own key. Two habits keep it that way — **shield** every account you use,
and fund item accounts from Zcash (or a fresh account), never straight from
a Solana deposit, whose memo is public.

```sh
zyn-cli shield    168.119.53.39:8099 11 ~/zyn.key            # once per account
zyn-cli mint-item 168.119.53.39:8099 11 ~/zyn.key ART 1 0.001 <sha256 of the content>
zyn-cli balance   168.119.53.39:8099 11 ~/zyn.key            # the new asset id appears
```

A sale is a co-signed trade; nothing rests on-chain and nothing is published:

```sh
# seller: item (asset 3), one unit, for 0.005 ZEC.zy, to a named buyer
zyn-cli offer  168.119.53.39:8099 11 ~/zyn.key 3 1 1 0.005 <buyer account hex>   # -> one line
# buyer: co-sign and submit
zyn-cli accept 168.119.53.39:8099 11 ~/buyer.key <that line>
```

The offer line is bound to the chain, the program, both parties and an
expiry; neither side can alter a field after signing. `zyn-cli transfer`
moves any asset between accounts (internal, unpublished).

**Re-seeding a pool at the market price.** A pool away from its market
charges the divergence fee, and that fee makes arbitraging it back a loss
(DECISIONS §5.7). To fix one: every LP removes their liquidity (the price
does not move), then a small swap corrects the price on what is left — the
locked minimum — then everyone adds liquidity back at the corrected ratio.
Account for the fee staying in the reserves when sizing that swap, or it
overshoots. `zyn-cli remove-liquidity <host:port> <chain> <key> <pool> <shares>`
was added for this.

**Bridged-asset markets.** Every bridged asset's own bridge fees build a
pot (`POT_ASSETS`, one balance per asset). When the pot is worth
`asset_threshold` ZEC.zy at a fresh reference and the protocol holds that
much ZEC.zy, a pool opens at the reference price, protocol-owned, and the
pot's contributors share `bootstrap_bps` of unminted ZYN, vesting
(DECISIONS §31). An asset that already has a market has its pot paired into
that market instead. The feed posts `UpdateAssetReference` for assets with
no pool; `OP_LAUNCH` (16) carries the list and the ZynZap Bridge tab shows
it. The two parameters can be changed after the ZYN launch graduated
(`SetLaunch` with everything else unchanged); nothing else can.

**Migrating a saved state.** A change to what the state root covers means
the saved file must be rewritten: stop the node, back up `chain-<id>.state`,
run `STATE_FILE=… cargo test -p swapvm --test decode_live -- --ignored`
(`migrate_a_saved_state` writes `<file>.migrated`), copy it into place, then
deploy. `restore_live` in `zynzapd/tests` runs exactly the check a starting
node does, which is the fastest way to tell a decode problem from a root
problem.

**Reference feeds.** `ZYN_FEEDS=1` runs the price feed (DECISIONS §29):
every `ZYN_FEED_INTERVAL` seconds (60) it prices both sides of every pool
it can, posts `UpdateReference` when the number moved 10 bps or is ten
minutes old, and reports as `price feeds` in `zyn-cli status`. Sources are
keyless: Coinbase, CoinGecko, Nasdaq, Yahoo, Frankfurter. `ZYN_FEED_MAP`
overrides symbols, e.g. `ZYN_FEED_MAP="TSLA=yahoo:TSLA,GOLD=coingecko:pax-gold|coinbase:PAXG"`.
`cargo test -p zynzapd --test feeds_live -- --ignored --nocapture` probes
every source once, including the on-chain Robinhood feeds and a price per
tokenised-stock issuer. Data files: `zynzapd/data/robinhood-stock-tokens.tsv`
(the registry at docs.robinhood.com/chain/contracts),
`robinhood-chainlink-feeds.tsv` (Chainlink's `feeds-robinhood-mainnet.json`),
`tokenized-stocks.tsv` (CoinGecko's coin list: xStocks, Ondo, Robinhood,
Dinari). Regenerate them from those sources when issuers add tokens. The testnet node runs with feeds on; pool 1
(ZEC.zy:SOL.zy) therefore charges the divergence fee whenever its spot
strays from the real SOL/ZEC price — which, on a testnet pool seeded at an
arbitrary ratio, is most of the time.

**Retired ZYN launch switches.** `ZYN_LAUNCH` and `ZYN_LAUNCH_THRESHOLD`
belong to §30's superseded bridge-fee/halving design. Current `zynzapd`
refuses to start when either is present, rather than silently activating the
old policy. Remove both variables. The old `OP_LAUNCH`/`OP_LAUNCH_ME`, codec
and state types remain temporarily for audit and replay of historical state;
the wallet overview and ZynZap Bridge tab no longer publish or render them.
Cave `LaunchCurve` is the only wallet-authorized fungible launch route.

**Batch clearing.** `ZYN_BATCH_CLEARING=1` in the env file queues single-hop swaps for the seal and clears each pool at one price (DECISIONS §28); the testnet node runs with it on. The auction window is the seal policy: `ZYN_SEAL_AFTER_SECS` (60) or `ZYN_SEAL_EVERY` intents, whichever first — the main loop seals by that policy, not on every tick.

## 4m. Operations (step 3)

**Alerts.** `zyn-cli status` lists every component with `ok` / `DOWN` and the
last error. Journal lines start `zynzapd: ALERT`. For a pager:

```
ZYN_ALERT_URL=https://hooks.slack.com/…      # or a Discord /slack webhook
ZYN_ALERT_AFTER_FAILURES=3                   # consecutive failed passes
ZYN_STALL_SECS=900                           # no good pass for this long
```

**Note trees** live in `notes-11-orchard.tree` / `notes-11-ironwood.tree`
next to the state, written at every save. A restart resumes from them after
checking the root against the node; delete them to force a re-seed (a
minute of catch-up). They are not secrets, but they are the vault's memory
of which notes it holds — back them up with the state.

**Reveals** arrive over the wire now:

```sh
zyn-cli reveal-submit 168.119.53.39:8099 11 ~/zyn.key <address> <salt-hex>
```

Accepted only if it matches the account's on-chain binding; the settler
picks it up on its next pass, and it is appended to the reveal file. The
files are still the audit trail and still work by hand.

**Fees.** An exit pays its own network fee: the recipient receives the exit
less ~0.0001 TAZ / 0.000005 SOL. Never let the vault pay — it then holds
less than it issued and the next attestation refuses. To top a vault up
without crediting anyone, deposit with the all-zero account as the memo:
`ZYN1:00…00` (64 zeros).

## 4n. Mirroring an NFT from Solana

Any SPL mint with zero decimals and a supply of one. Name it to the node:

```
ZYN_SOLANA_MINTS=<mint>:<symbol>[,<mint>:<symbol>…]      # e.g. xeDDUDGB…:PUNK.zy
```

On start the daemon creates the item on the chain if absent (`mirroring mint
… as PUNK.zy`) and watches the vault's token account for it (`watching token
account …`). The vault must hold some SOL surplus: it pays the rent of a
recipient's token account on exit.

**Deposit**, from any wallet, to the vault address with the memo — the wallet
derives the vault's token account itself:

```sh
spl-token transfer <mint> 1 7KacSYXuVhKSf8dpXcLY3qFZQYiZg2m6vKh4m1HxKZmA \
  --fund-recipient --allow-unfunded-recipient --with-memo "$(zyn-cli memo ~/zyn.key)" -u devnet
```

Credits as one whole unit of the item's asset id (`zyn-cli balance`). It is
then a Zyn item: shield the account, sell it with `offer`/`accept`, pool it.

**Exit** — bind a Solana address if not already bound (`bind-sol`), then:

```sh
zyn-cli withdraw-item 168.119.53.39:8099 11 ~/zyn.key <asset-id> <solana address> <salt>
zyn-cli reveal-submit 168.119.53.39:8099 11 ~/zyn.key <solana address> <salt>
```

The settler sends it to that address's token account (created if needed)
and burns it on Zyn once the payout is final. First mirrored item on devnet:
`xeDDUDGBJHG2Sx97uW3gE8UC8qyQ3XpwDsv9edSgjV9` as `PUNK.zy`, asset 5.

## 4o. The compact-block server (`zyn-lightd`)

One per network. Reads whole blocks from Zebra, keeps the compact form on
disk (`<data>/<height/1000>/<height>.cb`), serves ranges of it over a
length-prefixed TCP protocol (`zyn_custody::lightd`): INFO, BLOCKS,
TX, SEND, TREESTATE. Both shielded pools, v6 transactions. No keys, no
addresses — it learns which block ranges and which full transactions a
wallet asked for, nothing else (DECISIONS §24).

Deployed on `zyn-zebra-testnet`, port **8098** (firewall rule added):

| what | where |
|---|---|
| unit | `systemctl status zyn-lightd` — `Restart=always`, enabled at boot |
| env | `/etc/zyn/zyn-lightd.env` — `ZYN_LIGHTD_NETWORK=testnet`, `ZYN_LIGHTD_ZEBRA=127.0.0.1:18232`, `ZYN_LIGHTD_LISTEN=0.0.0.0:8098`, `ZYN_LIGHTD_DATA=/var/zyn/lightd` |
| cache | `/var/zyn/lightd` — safe to delete; it refills from Zebra |
| redeploy | build (`cargo zigbuild … --bin zyn-lightd`), `systemctl stop zyn-lightd`, scp, `systemctl start zyn-lightd` |

Check it from anywhere: `zyn-wallet sync <any wallet>` prints the network
and tip it saw.

**Mainnet** runs on the user's HP Pro Mini at home (6 Sep 2026), not on
Hetzner. It is a Windows 11 box; Docker Desktop there is broken (its WSL
distro will not start), so everything lives in a WSL2 Ubuntu distro named
`zyn` whose disk is on `D:\wsl\zyn` (C: is nearly full — never put state
there):

| what | where |
|---|---|
| reach it | `ssh tobia@192.168.1.21` (LAN; Mac key installed). Default shell is cmd; run `.ps1` files via `powershell -File`, or `wsl -d zyn -u root -- sh -c '…'` |
| Zebra | Docker Engine inside the distro: container `zebrad-mainnet` (zfnd/zebra, env as the testnet box, `Mainnet`), state `/var/lib/zyn/zebra` (ext4 on the D: VHDX), RPC 127.0.0.1:8232, P2P 8233 |
| block server | systemd unit `zyn-lightd` in the distro: mainnet, `ZYN_LIGHTD_LISTEN=0.0.0.0:8097`, cache `/var/lib/zyn/lightd`, binary `/usr/local/bin/zyn-lightd` (x86_64 zigbuild, copied via `D:\zyn\bin`) |
| networking | `.wslconfig` `networkingMode=mirrored`, so 8097 is the PC's own port; Windows firewall rule `zyn-lightd` allows it. LAN only; Tailscale for outside |
| **keepalive** | WSL stops a distro seconds after its last session ends, killing Zebra with it. Scheduled task `zyn-wsl-keepalive` (at logon, `wsl.exe -d zyn --exec /bin/sh -c "sleep infinity"`) holds it up — it must be started from the user's own interactive session (`Start-ScheduledTask zyn-wsl-keepalive`); remote `schtasks /run` does not launch it. After a reboot the user has to log in for it to fire |
| wallet | Nap Wallet → Settings → mainnet server `192.168.1.21:8097` |

Zebra's initial sync from genesis takes a day or more; INFO reports the
node's own height and an `estimated` chain height, and the wallet refuses
to create a new mainnet wallet until the two agree ("still syncing (h of
about H)"). A restore with a birthday the node has not reached is refused
the same way. Below a pool's activation height TREESTATE answers "no tree"
and the wallet starts that pool's tree empty.

## 4p. Nap Wallet (`nap-wallet`)

**Nap Wallet**: a shielded Zcash wallet on mainnet or testnet, with Zyn as
a connected feature — one local process, one page. Tabs: Wallet (holdings,
receive with QR, send with memo), Activity (history with memos, pending
until confirmed), Zyn (account, deposit, withdraw, private send on Zyn),
Portfolio (deployed value and pool positions), Items (private NFT custody,
receive, review, send and redeem), Agents (owner-created swap mandates), and
Settings (network switch, block servers, recovery phrase and portable backup,
rescan). Each network has its own wallet key (`wallet-<network>`) and
history (`history-<network>.json`) under the app dir; `settings.json`
remembers the network and servers. Zyn is available on testnet only for
now; on mainnet the wallet is a wallet.

```sh
nap-wallet                                   # keys in ~/.zyn/app, page on 127.0.0.1:8977
ZYN_APP_WALLET=~/zyn-testnet.wallet nap-wallet   # reuse an existing wallet key
```

| env | default | what |
|---|---|---|
| `ZYN_APP_DIR` | `~/.zyn/app` | wallet key + state, `zyn.key`, `binding` |
| `ZYN_APP_WALLET` | `$ZYN_APP_DIR/wallet` | the Zcash key record (or legacy 32-byte key; `.state` beside it) |
| `ZYN_APP_KEY` | `$ZYN_APP_DIR/zyn.key` | the Zyn account key (ed25519 seed) |
| `ZYN_LIGHTD` | `168.119.53.39:8098` | testnet block server default (§4o); both servers are then kept in `settings.json` |
| `ZYN_NODE` / `ZYN_CHAIN` | `168.119.53.39:8099` / `11` | the Zyn node |
| `ZYN_VAULT` | the testnet vault `utest1h6sf…x2t407` | where deposits go |
| `ZYN_APP_LISTEN` | `127.0.0.1:8977` | wallet UI listener; keep it on loopback |
| `ZYN_AGENT_LISTEN` | `127.0.0.1:8978` | separate, allowlisted agent listener |
| `ZYN_AGENT_TOKEN` | none | required 32+ character credential for the agent listener and `nap-agent` |

The page calls `/api/*` on the same origin; POSTs need an `X-Zyn` header a
cross-site form cannot set. Keys never leave the process. Slow things —
a sync, a payment being proved — run as a **job** on the app's own thread:
`POST /api/sync|send|deposit` return at once with `{started:true}`, the
overview carries `job` (kind, scan progress) while it runs, and the page
polls; while the wallet is busy the overview serves the last wallet view
rather than blocking. Anything that moves value gets a review dialog first
(to, amount, fee estimate, memo). `POST /api/export` returns the complete
versioned backup and, for new wallets, its BIP-39 words. `POST /api/import`
accepts `{backup}`, `{mnemonic, passphrase, account, birthday}`, or the legacy
`{key, birthday}`. Restore is allowed only while the current wallet is empty;
the replacement is fully parsed, derived and seeded before the old key is
removed. Binding: the app
binds the account to its **own** wallet address with a fresh salt (kept in
`$ZYN_APP_DIR/binding`) and reveals it to the operator over `OP_REVEAL`, so
a withdrawal is one amount field. The API also carries `/api/quote` and
`/api/swap` (used by the extension provider and by ZynZap tooling); the
wallet page does not show them — the AMM is ZynZap's product, the wallet is
the wallet.

### NFT receive, review and send

Use **Items**, not the generic Zyn asset form, for ordinary NFT custody:

1. **Receive:** open Items and share the 64-character Zyn account id, either
   with **Copy** or its QR. This is a receiving identifier, not a recovery key.
   The sender selects the NFT asset and sends one whole unit to that account.
2. **Review:** select the item. Nap shows its deterministic content identicon,
   full content commitment, asset id, collection phase and supply, pool backing,
   and redeem value. The identicon is intentional: no remote artwork is loaded
   until bytes can be fetched privately and verified against the commitment.
3. **Send:** choose **Send this NFT**, enter the recipient's 64-hex Zyn account
   id, then **Review transfer**. The confirmation names the item, asset id, full
   destination, amount (one whole NFT), settlement path, and the fact that the
   item leaves this wallet. There is no editable NFT amount field.
4. **Recovery:** leave the Zcash recovery checkbox off normally. Turn it on only
   if the Zyn node is censoring the transfer; it uses the existing forced-intent
   path and its Zcash fee/delay instead of creating a second NFT mechanism.
5. **Redeem is not send:** Redeem destroys a live collection item and pays its
   share of backing ZEC. Nap asks twice and names the destruction. Do not use it
   to transfer custody.

The receive QR is produced by `zynzapd::app::overview`; the action posts
`/api/transfer {to, asset, amount:"1", force}`. The VM still decides whether the
asset admits that amount and rejects fractions.

After changing the shared page, rebuild and verify every packaged copy:

```sh
sed -n '/<script>/,/<\/script>/p' zynzapd/ui/index.html | sed '1d;$d' | node --check -
cd apps/nap && npm run ui && cd ../..
cmp -s zynzapd/ui/index.html apps/nap/dist/index.html
cargo test -p zynzapd --bin nap-wallet
cargo test -p swapvm --test chain an_unwrapped_asset_cannot_be_fractionalised -- --exact
cargo test -p swapvm --test chain an_item_cannot_be_ended_by_a_partial_holder -- --exact
```

The **Agents** tab creates owner-signed, swap-only mandates with asset, pool,
per-action amount, slippage, and epoch limits. Nap persists the session keys
and action idempotency records in mode-0600 `agent.json`; neither appears in
the agent API. The separate listener serves only `/api/agent/*`, rejects
wallet methods and mandate creation even with a valid token, and the VM checks
the signed policy again during sequencing, replica replay, and proof execution.

```sh
export ZYN_AGENT_TOKEN="$(openssl rand -hex 32)"
nap-wallet

# In the agent process, with the same token but no access to ~/.zyn/app:
nap-agent pools
nap-agent quote 0 1 1000000000000000000
nap-agent check-swap MANDATE_ID 0 1 1000000000000000000 990000000000000000
nap-agent execute-swap request-001 MANDATE_ID 0 1 1000000000000000000 990000000000000000
nap-agent action request-001
nap-agent pause MANDATE_ID
```

`pause` and `close` stop new signatures locally. Delegations are stateless in
this version, so their on-chain authority ends at the mandate's expiry epoch;
the UI and API report that limit explicitly.

Operational failures are deliberately visible:

- `nap-wallet` refuses to start when `ZYN_AGENT_TOKEN` is absent or shorter
  than 32 characters. A `403` from `nap-agent` means its token differs from
  Nap's; a `404` means the requested route is outside the agent allowlist.
- Keep both listeners on loopback. If a proxy or tunnel exposes port 8978,
  the token becomes a remote signing credential for every active mandate.
- `$ZYN_APP_DIR/agent.json` contains session seeds and action-idempotency
  records. Keep it mode 0600 and back it up with the rest of the app directory.
  Invalid records are ignored on load; if `nap-agent mandates` loses an
  expected mandate, stop Nap and inspect or restore this file before issuing
  another action.
- If the token is exposed, stop Nap and rotate the token before restarting.
  If a session seed or complete delegated credential is exposed, rotating the
  token and pressing `pause` protect Nap only: a copied credential remains
  valid on chain until its expiry. Move the controlled balance with the owner
  key or wait for expiry; do not describe local pause as revocation.

The node gained `OP_ASSETS` (14) for this — id, symbol, supply, LP-of — so
the page can name balances. `zynzapd::client` is the node client as a
library; `zynzapd::wallet` the wallet. `zyn-cli` and `zyn-wallet` remain
the by-hand tools.

## 4q. Desktop, mobile, extension

Three faces of the wallet, one core (`zynzapd::app`, DECISIONS §26). The page is the same
file in all of them (`zynzapd/ui/index.html`); its `api()` picks the
transport: in-process `invoke` under Tauri, loopback HTTP otherwise.

**Desktop and mobile — `apps/nap`** (Tauri 2, one project for macOS,
Windows, Linux, iOS, Android). Keys live in the platform app-data dir
(`~/Library/Application Support/io.infinityteam.napwallet` on macOS); the same
`ZYN_*` env overrides apply when launched from a shell.

```sh
cd apps/nap
tauri dev                      # desktop, live
tauri build                    # .app / .dmg in src-tauri/target/release/bundle
# mobile — deferred (6 Sep 2026: "mobile can wait"). When picked up:
export ANDROID_HOME=~/Library/Android/sdk NDK_HOME=$ANDROID_HOME/ndk/27.1.12297006 \
       JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
tauri android init && tauri android build --debug   # apk under src-tauri/gen/android/app/build/outputs
tauri ios init && tauri ios build                   # needs a signing team in src-tauri/gen/apple
```

Both `init`s were run once (Xcode and Android Studio projects generated
fine; the Android Rust build got through the halo2 crates before it was
stopped in favour of desktop). The `gen/` directories were removed with the
rename to Nap Wallet; they regenerate in seconds.

The Rust core compiles for `aarch64-apple-ios`, `aarch64-apple-ios-sim`,
`aarch64-linux-android` (targets installed here). Proving an exit on a
phone takes about a minute and a few hundred MB; it runs off the UI thread.

**Browser extension — `apps/extension`** (Manifest V3). It does not hold
keys. The popup is the same page pointed at the local app on
`127.0.0.1:8977`, and `window.zyn` is a provider for sites:

```js
const { account, address, network } = await window.zyn.request({ method: 'zyn_account' });
await window.zyn.request({ method: 'zyn_swap', params: { asset_in: 1, asset_out: 2, amount: '0.01' } });
```

Reads (`zyn_account`, `zyn_balances`, `zyn_assets`, `zyn_pools`, `zyn_quote`,
`zyn_curves`, `zyn_curve`, `zyn_curveQuote`) answer at once. Writes
(`zyn_transfer`, `zyn_swap`, `zyn_deposit`, `zyn_withdraw`, `zyn_launchCurve`,
`zyn_buyCurve`, `zyn_sellCurve`) open an approval window naming the site and
the action; nothing is submitted until it is approved. Launch approval names
the immutable fee, 0.01 ZEC.zy creation cost and optional dev-buy ceiling;
curve trades name the exact token amount and maximum/minimum ZEC.zy. The local app answers
CORS only for `chrome-extension://`, `moz-extension://` and
`safari-web-extension://` origins, and still requires the `X-Zyn` header,
so a web page cannot reach it directly.

```sh
cd apps/extension && node build.mjs     # dist/ — load unpacked at chrome://extensions
```

To install: `chrome://extensions` → Developer mode → Load unpacked →
`apps/extension/dist`. Start `nap-wallet` (or the desktop app) first; the
popup says plainly when it is not running. Firefox needs the same files
with a `browser_specific_settings` id; Safari needs `xcrun
safari-web-extension-converter` — both later.

The extension needs `nap-wallet` (or the desktop app, once it also listens —
see §5) running on the machine.

## 4r. ZynZap, the front (`apps/zynzap`)

The AMM's own page: **Swap**, **Earn**, **Bridge**. One static source file,
`apps/zynzap/index.html`, with no framework or bundler; the deployment build
only copies it and the local font assets. It never holds a key; it asks Nap
Wallet to sign, two ways:

- hosted anywhere: through the Nap Wallet extension's `window.zyn` provider
  (every write opens the extension's approval window);
- served by the wallet itself at `http://127.0.0.1:8977/zynzap`: it calls
  the wallet's API on its own origin — how it is developed and tested.

**Hosted 12 Sep 2026:** `https://zynzap.pages.dev`. The hosted first run is
Bridge-first and accepts only a Nap provider reporting Zcash `mainnet` and Zyn
chain `26460`; a testnet or wrong-chain wallet leaves actions disabled and says
why. A visitor without the extension gets the actual bridge proposition plus a
complete, navigable read-only preview of Bridge, Swap and Earn. Wallet connection
reveals private balances and live quotes and enables actions; it does not unlock
the product explanation. The deployed CSP has `connect-src 'none'`: the page exchanges messages
with the extension and cannot quietly acquire a direct RPC dependency. Its
`script-src` admits the browser-extension schemes because Nap's content script
injects the provider into the page's JavaScript world; it does not admit any
web origin.

Build and deploy from the app directory:

```sh
cd apps/zynzap
npm test && npm run build
wrangler pages deploy dist --project-name zynzap --branch main
curl -fsS https://zynzap.pages.dev/ | grep 'MAINNET_CHAIN = 26460'
```

The build copies the already-audited Nap fonts from `zynzapd/ui/fonts`; do not
restore the old Google Fonts stylesheet. Cloudflare Pages applies `_headers`
from the build, including no-referrer, frame denial, and the no-network CSP.
The same source remains embedded in `nap-wallet`; its `/fonts/*` routes cover
those font URLs when the page is opened at `/zynzap`.

| tab | does |
|---|---|
| Swap | pay/receive with balances, live quote (rate, minimum received at 1% below, best case when the batch nets, pool fee), confirm, executed amount from the receipt |
| Earn | pools with reserves, fee and your share; add liquidity (the other side auto-fills at the pool ratio); positions with underlying amounts; remove shares |
| Bridge | Zcash: deposit from Nap Wallet with the memo, bind, withdraw. Solana: vault address + memo to copy for a deposit from any Solana wallet, bind a Solana address, withdraw SOL.zy or a mirrored asset. In-flight credits and exits. EVM marked as not deployed |

API added for it (all in `zynzapd::app`): `POST /api/liquidity/add
{pool, amount0, amount1}`, `POST /api/liquidity/remove {pool, shares}`,
`POST /api/bind-sol {address}`, `POST /api/withdraw-sol {asset, amount}`;
the overview's `zyn` section gained `positions`, `pools[].lp_asset /
lp_supply`, `bound_sol`, `solana_vault`, `binding.kind`. Submissions now
decode their first receipt, so a swap answers with `amount_out` and
liquidity calls with the amounts actually taken or returned. The
extension provider gained the matching `zyn_*` methods. One binding per
account: Zcash or Solana; the page says which is active.

Verified 6 Sep 2026 on testnet: add 0.001 ZEC.zy + 0.0046 SOL.zy → 0.95%
of pool 1; remove half (fees included); swap 0.002 SOL.zy → 0.000429 ZEC.zy.

## 4s. Verifying the chain: anchors on Zcash, mirrors, `zyn-replica` (7 Sep 2026)

Until 7 Sep no anchor had ever reached Zcash: `anchor_now` accepted the anchor
into an in-memory ledger, the ledger was forgotten at every restart, no intent
was kept after it was applied, and `OP_ACCOUNT_PROOF` proved against live
state. The exit hatch was an argument with no evidence. This section is what
replaced it.

### What the sequencer now does

- **Journals every intent before applying it** — `journal/chain-<id>/epoch-<N>.intents`,
  the VM's own bytes, so a replica reproduces every checkpoint. An intent the
  journal refuses is *not applied* (`Step::unrecorded`).
- **Keeps its lineage** — `chain-<id>.anchors` beside the state; a restart
  continues from the root Zcash last saw. Refuses a state older than its
  ledger, and a ledger with no state.
- **Anchors are self-sends with the root in the memo** (tag `ZYA`), built and
  FROST-signed exactly like an exit, 10,000 zat each. Proposed at the policy
  boundary, **accepted only when Zebra reports `ZYN_CONFIRMATIONS` depth** —
  which is when the epoch's deposits are released. Sealed epochs waiting on a
  trip and the proposal in flight survive a restart on disk
  (`pending-<id>-<epoch>.sealed`, `proposal-<id>.bin`). One anchor in flight
  at a time; unseen past expiry it is abandoned and the same anchor rebuilt.
- **Publishes a bundle per accepted anchor** to `ZYN_DA_DIR` (default
  `<data>/da`) and pushes it to `ZYN_DA_MIRRORS=url[|token],…`:
  `anchor.bin`, `certificate.bin`, `published.bin` (leaves, never records),
  `intents/epoch-<M>.intents` for every covered epoch, and an `index` line
  `epoch root anchor_id txid height`. A mirror outage never holds an anchor
  back; the publisher retries and the health line `da mirrors` says so.
- Without `ZYN_ZEBRA_SHARES` the node logs `anchors IN MEMORY ONLY` and runs
  the old V0 path. That is a devnet posture.

### The mirror

```
ZYN_MIRROR_DIR=/var/lib/zyn/mirror ZYN_MIRROR_LISTEN=0.0.0.0:8181 ZYN_MIRROR_TOKEN=<secret> zyn-mirror
```

`GET` any file; `PUT` with `Authorization: Bearer <token>`. Any static object
store works too — the files are checked, not trusted. **Every replica also
re-serves what it verified** (`ZYN_DA_LISTEN`), so the mirror set grows with
the verifier set.

### The replica

```
ZYN_CHAIN_ID=<chain> ZYN_DATA_DIR=/var/lib/zyn/replica ZYN_LISTEN=0.0.0.0:8100 \
ZYN_DA_MIRRORS=http://<mirror>:8181 ZYN_VAULT_FVK=<hex> \
ZYN_ZEBRA_HOST=<zebra> ZYN_ZEBRA_PORT=18232 ZYN_BRIDGE_FROM_HEIGHT=<vault birth> \
ZYN_SEQUENCER=<host:port> ZYN_DA_LISTEN=0.0.0.0:8182 zyn-replica
```

`zyn-replica` verifies an **AMM** chain — mainnet 26460, or chain 11 before it
was retired. The object ledger (chain 212, the testnet) has no replica yet:
`zyn_anchors` and the DA bundles are what a third party checks it with today.

Holds the vault's **viewing** key only. Each pass: scan Zcash for `ZYA`
self-sends at depth, fetch the bundle, check `anchor.bin` hashes to the memo's
id, check lineage, **replay the intents from its last verified state** and
require the anchored checkpoint to fall out, check the published leaves are
the tree that state produces, persist, re-serve. Serves the read RPC
(`OP_STATUS` reports role 1 and the verified epoch/height; `OP_SNAPSHOT` is
paged; `OP_ACCOUNT_PROOF` proves against the anchored root) and refuses
writes with the sequencer's address.

It **stops** — keeps serving, stops advancing — on: `FORK` (two ids for one
epoch), `LINEAGE` (does not continue), `BAD BUNDLE` (a file is not what the
anchor says), `ROOT UNREPRODUCIBLE` (the intents do not produce the root),
`MIRRORS DOWN`. Each is an alert; the first four mean a human must look
before anything else happens.

### Chain-format rollout gate

Any change to `STATE_VERSION`, VM version, journal format, or committed
authorization bytes is a coordinated verifier migration. An older replica
does the correct thing when it sees a newer format: it halts. That turns a
sequencer-first deploy into a quorum failure rather than a compatibility
warning.

For the v8 / `ZYNAUTH1` rollout on chain 11, signer2 is already stale and the
threshold is 2 of 3. Signer1 and signer3 are therefore both required. At the
start of the rollout the `ZYNAUTH1` marker count is 0 in the sequencer,
signer1 and signer3 binaries.

Deploy in this order:

1. Replace **signer1** (`~/.zyn/bin/zyn-replica` on this Mac), restart
   `dev.zyn.signer1`, and wait until its verified epoch equals the current
   anchored epoch.
2. Replace **signer3** on Windows WSL, restart only its signer service, and
   wait for the same catch-up check. Use
   `systemctl show -p ExecStart --value <signer3-service>` inside WSL to identify
   the binary actually run by the service rather than assuming a path.
3. Only after both checks pass, replace and restart the sequencer. Do not
   submit a delegated action before both verifiers are current.
4. Let one ordinary epoch seal and receive two endorsements, then submit the
   first delegated action and confirm both signers advance across it.

Check each active binary, using its path from launchd or systemd:

```sh
strings /path/to/active-binary | awk '$0 ~ /ZYNAUTH1/ {n++} END {print n+0}'
```

It must print at least 1 before that process is counted as upgraded. A copied
file is not evidence; check the `ExecStart` path and the process after restart.

Before the sequencer emits v8, a verifier can be rolled back to the old binary
because the chain is still v7. After any v8 state or `ZYNAUTH1` record has been
committed, **do not downgrade or re-base a verifier**. Stop new submissions,
keep the v8 state and journal intact, and fix the verifier forward. Re-basing
would discard its vote on earlier pending anchors (§50.15), the failure this
ordering exists to avoid.

### Leaving with the sequencer gone

Nap Wallet keeps `exit-<network>.json` — the record and its path to the
anchored root — refreshed at every anchor (the Zyn tab says which epoch).
By hand:

```
zyn-cli record <replica-or-sequencer> <chain> <key>        # keep the proof
zyn-cli exit-proof <key>.exit.json                          # print it, verified
zyn-cli verify-exit <key>.exit.json                         # anyone can check it
```

A proof names the root; the mirror's `index` names the anchor id and Zcash
txid for that epoch, and the anchor's memo on Zcash carries that id. Three
things anyone can compare, none of which needs the operator.

### What the first live run taught (7 Sep 2026, testnet chain 11)

Base root at the moment journaling began (epoch 1097, seq 3408):
`7e3e94b4f41be1f8628777e7dd5fb72364ef4864b42aeb7f32f4af9426f37446` — every
replica of chain 11 starts from it. First anchor ever on Zcash: epoch 1097,
root `28f8da6e…d876`, tx `357459ef…54e6`, height 4,331,685. Five anchors
confirmed and mirrored in the first half hour (1097, 1103, 1107, 1114, 1120);
a replica on a laptop holding only the viewing key reproduced all five from
Zcash and the mirror, and a holder's exit proof taken from that replica —
with the sequencer stopped — verified against root `2a4c9e39…d636`, which
the mirror's index ties to tx `8fcbe5d0…` at 4,331,717.

Then the sixth anchor was refused: *unknown Ironwood anchor*. The note-tree
feeder appended blocks straight at the chain tip with no reorg handling, so
a testnet reorg left orphaned commitments in both trees and every later
witness named a root Zebra never had. Three fixes, all live since: the trees
are fed only to `tip − ZYN_CONFIRMATIONS` (`Scanner::with_tree_lag`); the
anchor builder checks the tree's root against `z_gettreestate` before
witnessing and refuses in one line instead of in a doomed transaction; a
consensus refusal abandons the transaction at once and rebuilds the same
anchor rather than rebroadcasting it until expiry. A restart re-seeds a
diverged tree from the node's frontier (it did, in 20 s) and restores pending
epochs and the proposal in flight (42 of them, on that restart).

A second, self-inflicted divergence: the boot code resumed each pool's tree
from disk *independently* and re-seeded only the one that mismatched — but
`feed_block` appends every block to **both** trees, so re-seeding one pool
from the vault's birth walked five thousand blocks into the other pool a
second time. Each boot then found the *other* pool wrong. Both trees are now
resumed together or re-seeded together, and the anchor builder heals a
diverged pair in place (re-seed both, re-walk, log the commitment count it
had versus the node's) rather than asking for a restart.

And a third, which the lag itself introduced: the deposit scan fed the tree
with whatever block range *it* was reading, so once the tree trailed the tip
the next pass appended blocks above a gap. The tree is now fed contiguously
from where it stopped to the ceiling the lag allows, independently of the
deposit range. Anchors are also Ironwood-only now — an Orchard note cannot
pay a shielded output under NU6.3 — and with a single note in the vault they
serialise on the previous change confirming, which matches the policy's
cadence; the builder says "waiting for a spendable Ironwood note" rather than
failing.

The replica also halted on a `404` for a bundle the publisher had not pushed
yet — anchor at depth 6 versus a push on the next 30 s tick. A fetch failure
is now a retry, not a halt; only contradictions halt.

### Tests that pin this

`zyn`: journal round-trip/replay/tamper/gap; ledger and pending persistence;
propose/confirm/restore. `zynzapd`: `anchor` (send, confirm at depth,
abandon and rebuild, restart), `publish` (bundle, mirror PUT/GET, traversal
refused), `replica` (reproduces every root from sightings and files alone;
a flipped byte in *any* artefact halts it; fork halts it; exit proof
verifies; resumes from disk), `rpc` (paging, proof against the anchored
root, replica refuses writes), `resume`, `memo_tags`.

## 4t. Forced inclusion: the door the sequencer cannot shut (7 Sep 2026)

A signed intent the node ignores can be carried to the vault in a Zcash memo
(tag `ZYF`) on a 0.0001 note. The note's value is credited to the signer; the
frame is byte-for-byte what `OP_SUBMIT` carries, applied through the same
authorisation path — the user's signature, never operator authority. Accepted
or rejected by the VM, it is journaled: both are inclusion.

- **Nap Wallet:** tick "Force via Zcash" on a Zyn send. **CLI:** `zyn-cli
  force-memo <node> <chain> <key> transfer <to> <asset> <amount>` prints the
  memo for any wallet.
- **The rule:** applied by the anchor whose Zcash height is ≥ sighting + 20
  blocks. `zyn-replica` scans `ZYF` memos with the viewing key, verifies each
  signature, and reports **`CENSORSHIP`** (alert + status counter, persisted
  in `forced-<chain>.txt`) when one is overdue. Never a halt — the roots are
  still right; the operator is refusing service.
- **Replay index** (`replay-<chain>.idx`): a signature is applied once,
  whichever route it takes. Closes a gap that predates this: inside its epoch
  window a captured signed intent could be applied again.
- **Demonstrating it:** `ZYN_IGNORE_FORCED=1` on the sequencer (testnet only)
  makes it log and skip forced intents; force one from the wallet, wait 20
  blocks past confirmation, and the replica's status shows `censored 1`. Unset
  the flag and restart: the sighting is not marked seen, so it is applied on
  the next pass.

## 4u. Signer federation: a root deposits release on is one k verifiers reproduced (8 Sep 2026)

The anchor certificate, a stub until now, gates deposit release. Configure the
sequencer with `ZYN_SIGNER_SET` (comma-separated ed25519 pubkeys, hex) and
`ZYN_SIGNER_THRESHOLD` (default majority). An anchor still goes on Zcash on
its own; but its epoch's deposits stay pending until k signers endorse the
anchor id — and a signer endorses only a root it reproduced.

- **A signer is a replica with a key.** `zyn-replica` with `ZYN_SIGNER_KEY`
  (ed25519 seed) and `ZYN_ENDORSE_TO` (the sequencer): after it verifies an
  anchor, it signs the anchor id and POSTs `OP_ENDORSE`. It signs only what it
  replayed.
- **The sequencer** folds valid endorsements (`Node::add_endorsement` — in the
  set, signature opens the id) into the pending certificate, and the
  `AnchorSettler` releases the epoch only once the certificate clears,
  carrying the real certificate in `certificate.bin`. `AWAITING_SIGNATURES` in
  the log while it waits.
- **A verifying replica** with `ZYN_SIGNER_SET` refuses to advance past an
  anchor whose certificate is short: `Halt::Unendorsed`.
- **Safety:** a forged root goes on Zcash, no signer reproduces it, no
  certificate forms, the deposits behind it never release, and every signer
  reports `ROOT UNREPRODUCIBLE`. The sequencer can stall the chain; it cannot
  make it act on a state nobody else reproduced.
- **Honest limit:** the mechanism is k-of-n independent keys and replay. The
  testnet demo runs the signer processes on two machines sharing one Zebra —
  key and replay independence are real, block-source and machine independence
  partial. Production needs n operators on n infrastructures.

## 4v. Distributed custody signing: no box holds a quorum (8 Sep 2026)

The vault FROST key is split across `zyn-custodian` daemons, one share each,
on separate machines. To sign an anchor (or, once wired the same way, an exit)
a threshold must each answer two rounds; the sequencer coordinates but holds
no share.

- **Custodian:** `ZYN_CUSTODY_SHARE=<dir with one share-*.bin + public.bin>`
  and/or `ZYN_CUSTODY_SOLANA_SHARE=<same, Solana>`, `ZYN_CUSTODY_LISTEN=host:port`.
  Refuses to start with more than one share of either vault, and refuses ops
  for a vault whose share it does not hold. One daemon can serve both.
- **Sequencer:** `ZYN_CUSTODIANS=host:port,...` on the Zcash settle config.
  When set it loads only `public.bin` (no secret share) and signs anchors
  through the custodians; absent, it signs in-process as before.
- **Discipline:** a nonce is committed once, signed once, and forgotten on a
  custodian restart — fail closed. Everything on the wire is public; the share
  never leaves its machine. A custodian down between rounds fails the attempt;
  the sequencer retries and tolerates one down out of n>threshold.
- **Distributed key generation (`zyn-dkg`):** the shares no longer have to be
  born together. One relay (`zyn-dkg relay <n>`, forwards public broadcasts and
  sealed round-two ciphertext it cannot open) and N participants
  (`zyn-dkg join <relay> <index> <threshold> <n> <out-dir>`, each runs the
  FROST rounds locally and writes one share). No dir holds more than one share;
  the relay holds none. Round-two packages are sealed to their recipient
  (x25519 + ChaCha20-Poly1305). Cutting the live vault to a fresh distributed
  key is a deliberate migration (new vault address, drain + rescan), not done
  automatically.
- **Solana (8 Sep 2026):** the same, on the same daemons and the same socket
  (ops 3/4). Simpler than Zcash — plain FROST ed25519, one message, one
  signature, no per-action re-randomization — so a custodian commits one nonce
  per request. `ZYN_SOLANA_CUSTODIANS=host:port,...` on the Solana settle
  config; when set the sequencer loads only `public.bin` and holds no share.
  Proved end to end against `solana-test-validator`: the same exit, paid twice
  — once with the shares in-process, once by three daemons holding one each.
- **A custodian signs only what it committed to.** Round two must carry the
  same message round one committed the nonce to, on both vaults; a coordinator
  that swaps the message under a live nonce is refused (`WrongMessage`).

## 4w. Keeping the signer set up (8 Sep 2026)

The 8 Sep stall: the Mac's SSH tunnel to Hetzner died with a network change,
both Mac signers lost their Zcash view, and with only 1 of 3 endorsing, **no
deposit became spendable for about three hours**. Every settle pass reported
success the whole time.

- **Mac agents (`~/Library/LaunchAgents/dev.zyn.*.plist`)**, all `KeepAlive`,
  so a network change or a reboot reconnects instead of ending quietly:
  `dev.zyn.tunnel` (one ssh carrying `-L 18232`, `-L 8181`, `-R 8111`, with
  `ServerAliveInterval=15` and `ExitOnForwardFailure=yes`), `dev.zyn.signer1`
  (:8100), `dev.zyn.signer2` (:8101), `dev.zyn.custodian` (:8111),
  `dev.zyn.zebrad`. Binaries are **release** builds in `~/.zyn/bin`; logs in
  `~/.zyn/logs`. Verified by killing the tunnel: launchd restarts it and the
  remote reverse forward comes back.
- **The alert that was missing:** `AnchorSettler::endorsement_health(limit)`
  returns `Err` once an anchor has been confirmed on Zcash but unendorsed for
  `ZYN_ENDORSE_STALL_PASSES` passes (default 20 ≈ 10 min). Folded into the
  alerter's `"anchors"` health, so the state where nothing releases is a fault
  rather than a log line every tenth pass.
- **Independent Zcash view (in progress):** every signer read Zcash through a
  tunnel to *the sequencer's own Zebra*, so verification was circular and one
  dead tunnel took out two signers at once. The Mac now runs its own testnet
  `zebrad` (`~/.zyn/zebra`, RPC `127.0.0.1:18242`, `enable_cookie_auth = false`,
  state ~14 GB when synced). Signer1 stays on the tunnel (18232) until it
  catches up, then moves to 18242. The Windows signer needs the same and is
  waiting on disk.
- **Done 8 Sep (evening):** signer2 moved to the Windows box, so the Mac keeps
  one key and the always-on desktop holds two. With only two machines one of
  them must hold two; the choice is which, and a stationary box beats a
  travelling laptop on both liveness and exposure. `zyn-admin-tunnel` on
  Windows exposes its SSH on the sequencer, so it is administered with
  `ssh -J root@168.119.53.39 -p 2222 tobia@127.0.0.1` rather than only from the
  home LAN.
- **Still owed:** the signer set is 2-of-3 with **two keys on one box**, so that
  one laptop is both sufficient to certify and necessary for a quorum. Moving
  signer2 to a third host is the fix; it needs a host that is neither the
  laptop nor the sequencer, with ~40 GB for its own Zebra. With only two
  machines, no assignment avoids one of them holding two keys — safety (nobody
  certifies alone) and liveness (survive one box down) cannot both hold.
- **Oracle free tier, attempted 8 Sep:** the account (home region
  `eu-frankfurt-1`, one existing E2.1.Micro, 153 GB of the 200 GB block-storage
  allowance free) has the Ampere quota unused, but **every AD returns "Out of
  host capacity"** at 2 OCPU/12 GB and at 1 OCPU/6 GB. Always-Free resources
  exist only in the home region, so another region is not a way round it.
  `dev.zyn.oci-retry` (LaunchAgent, every 5 min, `~/.zyn/oci/try-launch.sh`)
  keeps asking and stops for good once `~/.zyn/oci/launched.json` exists, so it
  cannot claim two. Log: `~/.zyn/logs/oci-retry.log`.

## 4x. Deposit validation: endorsement that means the money is there (8 Sep 2026)

Replaying the published intents proves the sequencer computed honestly over
what it published. It does **not** prove the inputs were true: a fabricated
`CreditDeposit`, paired with an `AttestVaultBalance` raised to clear the
vault's `AboveObserved` check (`zyn-bridge/src/vault.rs:172`), replays
identically on every signer and produces a matching root. Every signer would
have endorsed it.

- **The hook:** every credit carries the Zcash txid in `external_ref`
  (`zyn-custody/src/watcher.rs:267`).
- **`zynzapd::backing`** — `SeenCredit`, the `Backing` trait, and
  `ZcashBacking`, which asks *its own* node: the transaction exists, is at
  `ZYN_CONFIRMATIONS` depth, and pays the vault at least what was credited
  (anchors excluded — a self-send backs nothing). Two credits against one txid
  are summed, so a replay cannot be split across indices.
- **`Halt::Unbacked`** joins `Fork`/`Diverged`/`Unendorsed`: the replica stops,
  keeps serving the last verified root, and a signer has nothing to endorse.
- **`ZYN_VERIFY_DEPOSITS=1`** turns it on, and it is **off by default** on
  purpose: a replica pointed at the sequencer's Zebra would be asking the party
  it is checking. Turn it on only once the signer has its own node — on the Mac
  that means moving `ZYN_ZEBRA_PORT` to 18242 first (§4w).
- **What it does not settle:** a memo-less deposit's owner. The chain shows the
  money arriving and nothing about whose it is; the sequencer says so out of
  `ZYN_ZEBRA_ATTRIBUTIONS`, a file no verifier can see (3 entries today). Such
  a credit passes on the strength of the money being real. Where a memo *does*
  name an account, the sequencer cannot disagree with it — that is checked.
## 4y. Per-account deposit addresses (8 Sep 2026)

One shielded address per account, so a depositor needs no memo and no wallet
feature — **the address is the attribution** — and a verifier can rederive
whose the money is instead of taking the operator's word.

- `deposit_index(account)` = SHA-256 of `"zyn.deposit.index.v1" ‖ account`,
  first 11 bytes, never zero. **Index 0 is the vault's own address** (change,
  anchors, top-ups), which is what separates a stranger's deposit from the
  vault paying itself with no note store.
- `VaultKeys::deposit_address(account)` issues it; `index_of(note)` recovers
  the index a note arrived at; `classify_addressed` attributes by address
  first, then memo, then change, then the operator's file — so deposits made
  the old way keep working.
- **`zynzapd::deposits::Book`** (`deposit-addresses-<chain>.bin`) records which
  accounts have been issued one. The index is derived from the account but the
  account cannot be derived back, so the sequencer keeps the mapping. It is a
  convenience, **not a trust root**: a verifier never reads it, and losing it
  costs only re-issuing.
- **`OP_DEPOSIT_ADDRESS` (19)**, `Node::deposit_address(account)` on the
  client. Unauthenticated on purpose: the caller already named the account, and
  without the vault's viewing key the address discloses nothing.
- `ZcashBacking` checks a note at a non-zero index belongs to an account the
  credit pays. **`ZYN_VERIFY_DEPOSITS=strict`** additionally refuses a memo-less
  credit at index 0 — change dressed as a deposit. Correct only once every
  deposit arrives at its account's own address, so it is a separate opt-in
  from `=1`.
- **Nap Wallet** shows the account's own address (`GET /api/deposit-address`,
  and `deposit_address` on the overview), remembers it in `deposit-address` in
  the app dir, and `/api/deposit` pays it with **no memo**. A node too old to
  issue one falls back to the shared vault address plus the memo, and the UI
  says which of the two it is showing.
- **Paying from another wallet** is a ZIP-321 request, not a copy: the
  overview carries `deposit_uri` (`zcash:<addr>?message=Zyn%20deposit`, plus
  `&memo=<base64url>` only on the fallback path) and `deposit_qr_svg`. The
  deposit panel shows the QR under "Pay from another wallet" and the copy
  button appends `&amount=` from the amount box. Memos are base64url **without
  padding** — the one detail a hand-rolled encoder gets wrong, so it is tested
  against the RFC 4648 vectors.
- Verified live 8 Sep against chain 11: the wallet for account `81dfe623…`
  was issued `utest10l0kjjdxk2uc8…`, distinct from the vault's own
  `utest1h6sfz7aln…`, `"memo": false`; the register on the box grew to 104
  bytes (8 magic + 3 accounts) with issuing proven idempotent.

## 4z. Mainnet readiness: the network stops being a constant (9 Sep 2026)

The daemon was structurally testnet-only, and one line of it was a launch
blocker: `shielded.rs` encoded every unified address with
`NetworkType::Test`, **hardcoded**. A mainnet vault would have handed out
`utest1…` deposit addresses. Nobody would have lost funds — a mainnet wallet
rejects them on the HRP — but the mint could not have happened, and the first
buyer would have been the one to find out.

- **`ZYN_NETWORK=testnet|mainnet|regtest`**, defaulting to testnet, so mainnet
  is always a deliberate act. Everything network-shaped derives from it:
  address encoding, every `Zebra::connect`, reveals parsing.
- **`VaultKeys::address` / `deposit_address` take the network as a required
  argument.** No default: a forgotten argument would be invisible until a
  depositor's wallet refused the address, so the compiler asks instead.
- **`Zebra::verify_network()` and a boot-time refusal.** `getblockchaininfo`
  reports `chain`; if it disagrees with `ZYN_NETWORK` the daemon will not
  start. This is the check that matters most before real money: every other
  safeguard assumes the chain underneath is the configured one, and nothing
  downstream can detect that it is not, because everything downstream is
  consistent with itself. Live boot now logs
  `Zcash Testnet confirmed by the node at 127.0.0.1:18232`.
- **The mainnet refusal moved** off `Zebra::connect` (which now allows it —
  the shielded vault is what runs on mainnet) and onto `Zebra::observe`, the
  transparent watcher, which reads addresses in the clear and was never meant
  to hold real money.
- **`zyn-ceremony` honours `ZYN_NETWORK` too**, printing which chain the vault
  is for and what its address should start with. A vault born for mainnet that
  printed a testnet address would be funded on the wrong chain.

**Still needed for a mainnet mint**, none of it code: a Zcash mainnet node
(~500–600 GB from our own measurement — 245.8 GB at 55.4% — against a
sequencer with 43 GB free and 3 GB RAM, so the box needs replacing, not
growing); days of initial sync; and a vault key born from the distributed DKG,
because the current one's shares have been copied between machines throughout
development.

## 5a. Collections over the wire (9 Sep 2026)

The VM knew a collection's floor; nothing could read it. A storefront cannot
be built on a number only the sequencer can see.

- **`OP_COLLECTIONS` (20)** — per collection: creator, symbol, cap, minted,
  outstanding, pool, fee_bps, phase, and `redeem_price`. The floor is *sent*
  rather than left to the caller to divide: it is the number everything else
  is judged against, and two implementations of one division is one too many.
- **`OP_ASSETS` (14) now carries `content` and `collection`** per asset, so an
  artwork is distinguishable from a token. Note this **changes the record
  layout** — client and sequencer must be deployed together.
- **Client**: `collections()`, `collection(id)`, `Asset::is_item()`,
  `CollectionView::{phase_name, remaining}`; and write helpers
  `create_collection`, `mint_collection_item`, `fund_collection`,
  `redeem_collection_item`, `advance_collection`.
- **Co-signed offers**: `signed_credential`, `offer_authorization`,
  `frame_co_signed`, `submit_co_signed`. A maker signs a listing now and a
  taker accepts it later from another machine, neither holding the other's
  key — both must sign the *same* intent under the *same* authorization, so
  the expiry is made once and shared.
- Verified live on chain 11: collections answers, and 2 of 7 assets report
  content, parsing with no trailing bytes.

## 5b. Resting offers and the anchor index over the wire (10 Sep 2026)

The VM gained escrowed offers (DECISIONS §39); nothing outside it could place
or take one, so a storefront could not be built on them.

- **`OP_OFFERS` (21)** — every resting offer: id, maker, the asset and amount
  committed, what it wants, and the epoch it lapses. Public by the same
  argument as the floor: an offer nobody can see is one nobody can take.
- **`OP_ANCHORS` (22)** — the DA index as `epoch root anchor_id txid height`,
  optional `limit` for the newest N. This is the mapping from a Zyn epoch to
  the Zcash transaction carrying its root. **A replica serves it from its own
  DA copy**, which is the point: the claim can be checked against something
  that never sequenced anything. A node holding no bundles **refuses** rather
  than returning an empty list — silence and "nothing was ever anchored" must
  not look alike.
- **Client**: `offers()`, `offer(id)`, `anchors(limit)`, and write helpers
  `place_offer`, `take_offer`, `cancel_offer`.
- **HTTP** (`zynzapd::app`): `GET /api/offers`, `GET /api/anchors`,
  `POST /api/offer|take|cancel-offer`, plus the collection lifecycle
  `POST /api/collection|fund|mint|advance`. `/api/overview` gained `offers`
  and a `settlement` block so a status strip is one call.
- **A paid claim is two intents, not one privileged one**: the buyer calls
  `/api/fund` (their money, their signature), then the creator calls
  `/api/mint` with `serial` from the verified allocation receipt and `to` set
  to the receipt's bound buyer. `FundCollection` is authorised by
  whoever pays, so the creator never moves a buyer's money.

## 5c. Monitoring: alerts that arrive, and a bot to ask (10 Sep 2026)

On 9 Sep the chain sat halted for over a day while `alert.rs` announced
`verification DOWN` to journald, where nobody reads. Detection was never the
problem; delivery was.

- **`ZYN_ALERT_URL` on all four hosts**, pointed at Telegram:
  `https://api.telegram.org/bot<token>/sendMessage?chat_id=<id>`. **No code
  changed** — the webhook body already carried `text`, and Telegram takes
  `chat_id` from the query string. Set it in `/etc/zyn/zynzapd.env`,
  `/etc/zyn/zyn-signer.env`, and the `EnvironmentVariables` of the two macOS
  LaunchAgents.
- **`zyn-tgbot`** runs beside the sequencer (`systemd`,
  `/etc/zyn/zyn-tgbot.env` at mode 600, `ZYN_TG_TOKEN`, `ZYN_TG_CHATS`,
  `ZYN_NODE`, `ZYN_CHAIN_ID`; optional `ZYN_ZEBRA_TESTNET`,
  `ZYN_ZEBRA_MAINNET`). Buttons for chain, signers, anchors, offers and both
  Zcash nodes. It holds no key and calls only public reads, so a leaked token
  discloses the height, which the DA store publishes anyway. Lock it to your
  own chat id — a bot that answers anyone is a bot whose token is a disclosure.
- **Zcash nodes are judged against `estimatedheight`**, not their own height.
  A stalled node reports whatever height it stopped at and looks healthy; only
  the gap to the network's view exposes it.
- **Expect ~50 s of `409 Conflict` after restarting the bot.** Telegram holds
  a long-poll that long, and the old connection has to lapse. It is not
  another poller. Do not run a `curl` against `getUpdates` while the bot runs
  — that *is* another poller, and it will steal the updates.
- **Censorship alerts report current state, not the record.** A past incident
  stays in `Replica::censored()` and in the health source's `scanned_to`, but
  only events within `CENSORSHIP_CURRENT` page anyone. An alarm that can never
  clear teaches its reader to scroll past.

## 5d. The M1 Air: the third fault domain (9–10 Sep 2026)

`zhang@192.168.1.27`. Closes §36.6: three signer keys on three machines, none
of them sufficient alone.

- **`dev.zyn.signer2`** — LaunchAgent, `~/.zyn/bin/zyn-replica`, data
  `~/.zyn/signer2`, RPC `127.0.0.1:8101`, pubkey `27048d32…`. The Windows box's
  `zyn-signer2` is **stopped and disabled**; the key must not run twice.
- **`dev.zyn.tunnel`** — forwards `18232`, `8181` and `8120` to Hetzner with
  the restricted key `~/.ssh/hetzner`.
- **`dev.zyn.zebrad`** — its own **testnet** Zebra, config
  `~/.zyn/zebra/zebrad.toml`, state `~/.zyn/zebra/state`, RPC
  `127.0.0.1:18242`. Pointing signer2 at it is what closes §36.4/§36.5 and
  unblocks strict deposit verification. **Testnet only**: mainnet needs
  500–600 GB and this box has ~110 GB.
- Holds **one share of the mainnet vault** at `~zhang/.zyn/custody-main` (02),
  served by a custodian on `127.0.0.1:8210` under launchd, tunnelled to the
  mainnet sequencer as `127.0.0.1:8213`. The login is **`zhang`**, not `tobiasd`.

## 5e. The Contabo host: the mainnet sequencer's home (10 Sep 2026)

`root@95.111.240.24` — Storage VPS 30, Ubuntu 24.04, 6 vCPU, 17 GB RAM, 968 GB.
Rented so mainnet stops depending on a home machine that reboots for Windows
Update.

- **Hardened at provisioning**: key-only SSH (`PasswordAuthentication no`,
  `PermitRootLogin prohibit-password` in `/etc/ssh/sshd_config.d/10-zyn.conf`),
  `ufw` allowing only 22 and 8233, `fail2ban`, unattended security updates.
  Keep the root password anyway — Contabo's rescue console still wants it.
- **Mainnet Zebra** in Docker: config `/etc/zyn/zebrad.toml`, state
  `/var/lib/zyn/zebra`, RPC published to **loopback only**
  (`-p 127.0.0.1:8232:8232`), P2P 8233.
- Three things the `zfnd/zebra` image makes awkward, all of which cost an hour:
  it wraps `zebrad` in an `entrypoint.sh`, so config arrives via
  **`CONFIG_FILE_PATH`**, not arguments; it defaults `ZCASHD_COMPAT` to
  **true**; and the state directory must *also* be named in
  **`ZEBRA_STATE__CACHE_DIR`**, because the entrypoint only chowns what that
  variable points at — without it the container runs as uid 10001 against a
  root-owned directory and panics with `PermissionDenied`.
- Measured **6,261 blocks/min**, roughly nine hours to tip. Shared SATA was not
  the bottleneck the price suggested.

### The second Zebra: testnet, for the cavebro replica (10 Sep 2026)

`rpc.cavebro.win` is meant to answer here rather than at Hetzner or a home
machine, and the chain it serves (11) is **testnet**, so this host needs a
testnet Zebra beside the mainnet one. Testnet state is ~14 GB against mainnet's
300 GB, so it costs almost nothing.

```sh
# /etc/zyn/zebrad-testnet.toml — network = "Testnet", rpc 18232, p2p 18233
docker run -d --name zebrad-testnet --restart unless-stopped \
  -e CONFIG_FILE_PATH=/etc/zebrad/zebrad.toml \
  -e ZEBRA_STATE__CACHE_DIR=/var/cache/zebrad-cache \
  -e ZEBRA_ZCASHD_COMPAT__ENABLED=false \
  -p 127.0.0.1:18232:18232 -p 18233:18233 \
  -v /var/lib/zyn/zebra-testnet:/var/cache/zebrad-cache \
  -v /etc/zyn/zebrad-testnet.toml:/etc/zebrad/zebrad.toml \
  zfnd/zebra:latest zebrad
```

- The three env vars above are not optional and are the same trap as the
  mainnet container. **Omitting `CONFIG_FILE_PATH` does not fail** — the
  entrypoint writes its own default config and the container syncs *mainnet*
  while looking healthy. Check `docker logs` for `network=Testnet` and the RPC
  for `"chain":"test"`; a container that answers nothing on 18232 is one that
  never read the config.
- RPC is bound to **127.0.0.1** on both. Only P2P is public.
- Measured **16,800 blocks/min**, a few hours to tip. Both syncs together put
  load at ~7 on 6 cores, which is fine and drops when testnet finishes.

### The reverse tunnels, and why they stop recovering (10 Sep 2026)

The home box holds its forwards on the sequencer through `zyn-zebra-tunnel`
(8232), `zyn-custody-tunnel` (8112) and `zyn-admin-tunnel` (2222), each an
`ssh -R` restricted by `permitlisten` in Hetzner's `authorized_keys`.

**The failure that looks like a tunnel bug and is not.** Hetzner shipped with
`ClientAliveInterval 0`, so sshd never probed its clients. Every tunnel the home
link dropped left a session **alive on the server holding its port**, and the
next attempt died at once:

```
Remote: /root/.ssh/authorized_keys:2: key options: permitopen permitlisten
remote forward failure for: listen 127.0.0.1:8232
Error: remote port forwarding failed for listen port 8232
```

Read literally that says the key is not allowed to listen there. It is: the
entry carries `permitlisten="127.0.0.1:8232"`. The bind failed because a dead
session still owned the port, and nothing on the box would ever release it.
Four such zombies had accumulated over twelve hours; `kill`ing them made the
forward bind on the next try.

Fixed durably in `/etc/ssh/sshd_config.d/10-reap-dead-tunnels.conf`:
`ClientAliveInterval 30`, `ClientAliveCountMax 3`. Dead clients are now reaped
in 90 s instead of never.

**Mirrored to Contabo on 21 Sep**, which had never had it — its `sshd_config`
still carried the commented-out `#ClientAliveInterval 0`, so the *mainnet*
custody forward on 8211 had no reaping at all. One sequencer was cured in
September and the other was not, because the diagnosis was written up against
the box it happened on. When a fix is about sshd rather than about Zyn, apply
it to every host that terminates a tunnel.

**A restart loop can be its own cause.** Testnet's 8112 forward failed with
`remote port forwarding failed for listen port 8112` while *nothing held the
port* — `ss` showed it unbound. The unit was restarting every ~6 s and each
attempt raced its predecessor's half-dead session. Stopping the unit and running
the identical `ssh` by hand bound first try. Before hunting a permission or a
zombie, stop the unit and try once: `permitlisten` was correct all along, and so
was the key.

**To check, do not trust a substring.** `ss -lnt | grep 8232` matches the
testnet Zebra's `18232` and reads as success while 8232 is unbound. Anchor it:
`ss -lnt | grep "127.0.0.1:8232"`.

**The real cause: nobody was signed in to Windows.** WSL needs an interactive
user session to live in. With the console parked at the sign-in screen, WSL
existed only inside whichever `ssh` session invoked it, and everything it owned
— systemd units included — was reaped when that session closed. The tunnels
bound and died within 30 s forever, `zebrad` restarted before it could finish
opening its database, and every detached transfer vanished.

Two measurements pointed away from the network and should be repeated before
anyone blames it again: a plain outbound `ssh` from WSL held a full 180 s, and
the tunnel command run in the foreground held 180/180. Both survived *because a
session was holding WSL open the whole time*. WSL2's NAT is not the problem, and
`networkingMode=mirrored` is not the fix.

How it got that way (10 Sep): a restart was initiated at **07:08:55** (`1074`),
logging the user off two seconds later (`4647`), followed by three reboot cycles
to 07:14 — a Windows Update batch. Nobody signed back in. The only interactive
logons after that boot are `DWM-1`/`UMFD-0`/`UMFD-1`, which are the pseudo-
accounts Windows runs *at the sign-in screen*.

**The fix is to be signed in**, and to stay signed in. Verified: with the console
`Active`, all three forwards held for three minutes straight and Hetzner read the
home Zebra at tip through the tunnel.

- Disconnecting RDP or sleeping the client is **safe** — the session goes `Disc`
  and keeps running. Signing out is not. Neither `MaxDisconnectionTime` nor
  `MaxIdleTime` is set, so a disconnected session is never reaped.
- `ZynWslKeepAlive` (Task Scheduler, ONLOGON, `C:\zyn-keepalive.cmd`) holds a
  `sleep infinity` session open. The launcher must **not** exit — an earlier
  version used `start /b`, returned immediately, and Task Scheduler killed the
  tree with it. It loops instead.
- None of this survives a reboot. The next Windows Update leaves the box at the
  sign-in screen with signer 3, custodian 3 and mainnet lightd down until a human
  signs in. Auto sign-in is the only cure, against the cost that a powered-on
  machine holding a vault share is then an unlocked one.

### A replica halts with `StaleBase`: diagnosing a settlement hole

`HALTED: LINEAGE break at epoch N: StaleBase` means an anchor's `previous_root`
does not match the lineage head. **Do not reset the replica first.** Its state is
derived; if the chain really has a hole, a resync hits the identical wall and you
have destroyed the evidence. Establish where the fault is before touching it.

**0. First, check whether the replica had finished reading.** Before §62 this
was the most likely cause and it looks identical to a real hole. Compare the
replica's scan cursor with the chain tip:

```bash
grep -oE 'scanned to [0-9]+' ~/.zyn/logs/signerN.log | tail -1
curl -s -u x:x --data '{"jsonrpc":"2.0","id":1,"method":"getblockcount"}' \
  -H 'content-type:application/json' http://127.0.0.1:18232/
```

If the cursor is behind the tip, the parent may simply be in a block it has not
read yet — a **repaired** anchor carries its original epoch but lands at today's
height, potentially thousands of blocks ahead of everything built on it. A
replica on the §62 binary defers instead of halting and logs `N sighting(s) do
not continue the lineage yet; holding them while the scan reads to ...`. One on
an older binary latches, and the fix is to upgrade it, not to re-base it —
re-basing forfeits its vote on every earlier epoch (§A17) and is how a signer
set ends up permanently below threshold.

**1. Is the published chain intact?** Anchors are `zyn.anchor.v1`-framed; the
field order in `zyn/src/anchor.rs::encode` is magic(13) ‖ chain(4) ‖ epoch(8) ‖
state_root(32) ‖ intent_root(32) ‖ seq(8) ‖ … ‖ previous_root(32) ‖ epochs(8) ‖ —
**`previous_root` is at offset 153, not 25.** Corrected 12 Sep: reading it at 25
(as this file previously said) yields the *state root of the epoch before*, and
makes a healthy anchor look like it does not chain. Verify any offset by
searching the bundle for a root you already know from a log line before trusting
it — `state_root` is at 57 ‖
gross_volume ‖ **previous_root(32)** ‖ epochs(8) ‖ actions(8). Fetch
`chain-<id>/<epoch>/anchor.bin` from a mirror for consecutive epochs and check
each `previous_root` equals the prior `state_root`. If they chain, the DA copy is
fine and the fault is elsewhere.

Note `parent_root` links *epochs* and `previous_root` links *anchors* — comparing
the wrong one will tell you the chain is broken when it is not.

**2. What is the replica's actual head?** It logs it: `chain 11 — verified
through epoch 4321 at height 4337722`. If that is *earlier* than the epoch in the
halt message, the replica is missing an anchor in between, and the question is
why it never accepted it.

**3. Is the missing anchor on Zcash?** The replica only accepts anchors it has
seen on chain, which is the whole point of it. Take the txid from the DA index
and ask Zebra:

```sh
curl -s -u zebra:zebra --data '{"jsonrpc":"2.0","id":1,"method":"getrawtransaction","params":["<txid>",1]}' \
  -H 'content-type: application/json' http://127.0.0.1:18232
```

An error means it never confirmed. Check `getrawmempool` too: absent from both is
a transaction that expired and was dropped.

**A `height 0` index row is ambiguous** until §48.6 is deployed — it means either
"confirmed, never recorded" or "never landed". Resolve it against Zebra rather
than assuming, which is how C12 got recorded the wrong way round the first time.

**Healing.** A published anchor cannot be rewritten: the next one already commits
to its root, so the lineage cannot be routed around the gap. The missing anchors
have to reach Zcash. The `ZYA` memo identifies the *anchor*, not the transaction,
so a fresh vault self-send carrying the same memo settles it — a different txid is
fine, and the index row should be corrected to the new one. Moving
`ZYN_BASE_ROOT` past the gap is not a fix; it declares the epochs unverifiable
and calls the chain healthy (§50.6).

### Anchors confirmed but never endorsed: the set is below threshold

Symptom, from the sequencer's own alert:

```
anchor <id> is on Zcash and confirmed; awaiting signatures (N pass(es))
ALERT zyn chain 11 — anchors STILL DOWN: ... unendorsed for N passes
```

This is **not** a settlement hole. The anchor reached Zcash and confirmed. What
failed is the signer set: `ZYN_SIGNER_THRESHOLD` endorsements never arrived, so
the certificate never cleared and no deposit became spendable. Same user-visible
symptom as an attack; the cause is usually that a signer is simply not running.

**0. First, check whether the sequencer still HOLDS the endorsements.** This is
the cheapest check and it was the answer on 11-12 Sep (§63). Signatures for an
*in-flight* anchor live in memory; only a settled anchor's certificate is
persisted. If the sequencer restarted after a signer endorsed, that signature is
gone and — before A20 — was never re-sent, because a signer endorses each anchor
once. Compare the sequencer's uptime against the signer's last endorsement:

```bash
ssh root@<sequencer> 'systemctl show zynzapd -p ActiveEnterTimestamp --value'
ls -lT ~/.zyn/logs/signerN.log        # local time; the sequencer logs UTC
```

If the sequencer started *after* the signer's endorsement, that is your fault —
no code is broken. On the A20 binary a signer re-offers every pass and seeds its
head anchor at startup, so **restarting the signer is the fix**:

```bash
launchctl unload ~/Library/LaunchAgents/dev.zyn.signerN.plist
launchctl load   ~/Library/LaunchAgents/dev.zyn.signerN.plist
grep -E "endorsed anchor|re-offered" ~/.zyn/logs/signerN.log | tail -3
```

Look for `certificate now clears`. Plain `counted` means the signature was
accepted but the set is still short.

**1. Count who is actually endorsing.** On the sequencer:

```bash
grep -E "ZYN_SIGNER_SET|ZYN_SIGNER_THRESHOLD" /etc/zyn/zynzapd.env
```

Then, on each signer host, read the *live* log — not the one in the data
directory, which a LaunchAgent may not be writing to:

```bash
plutil -extract StandardOutPath raw ~/Library/LaunchAgents/dev.zyn.signerN.plist
```

A healthy signer's last lines alternate `verified ... through epoch E` and
`endorsed anchor <id>`. A signer parked on the *same* epoch as the stalled
anchor has endorsed and is waiting for its peers — it is not the problem.

**2. Map pubkeys to signers before changing anything.** The set is positional
hex; derive each signer's pubkey from its seed (ed25519, `SigningKey::from_bytes`
→ `verifying_key`) and find its slot. Never print a seed — derive and print only
the public half.

**3. A re-based signer has forfeited its vote on everything older (§A17).** A
signer whose base was moved forward to clear a halt reports `verified through
epoch none at height 0` and can never endorse an epoch below its new base. It
cannot help with the stalled anchor. Bringing up a *different* signer from a base
below the stall is the fix; re-basing it back is not, and destroys evidence.

**4. Reviving a retired signer.** Its plist may be parked as
`~/.zyn/retired-dev.zyn.signerN.plist.bak`. Copy it into `~/Library/LaunchAgents/`,
point it at the current binary, and load it:

```bash
cp ~/.zyn/retired-dev.zyn.signer2.plist.bak ~/Library/LaunchAgents/dev.zyn.signer2.plist
plutil -replace ProgramArguments -json '["'"$HOME"'/.zyn/bin/zyn-replica-a16"]' \
  ~/Library/LaunchAgents/dev.zyn.signer2.plist
launchctl load ~/Library/LaunchAgents/dev.zyn.signer2.plist
```

**Expect several silent minutes.** The first pass rescans Zcash from the
signer's last height; it logs nothing and burns almost no CPU while doing it.
Do not conclude it is hung — confirm its RPC port is bound (`lsof -nP
-iTCP:8101 -sTCP:LISTEN`) and wait for the first `verified N anchor(s)` line.

**5. Most index entries reading `height 0` is normal, not a fault.** 217 of 318
entries on chain 11 carry height 0 (C11: `write_local` never replaced the
height-0 line written at broadcast). Replicas scan Zcash for the `ZYA` memo
themselves and do not trust that column — so a wall of zeros is not evidence of
an unsettled chain. Only the *sequencer's* own confirmation matters.



### Withdrawals: what is slow, what is not, and the one race

Anchor cadence does not affect how fast ZEC reaches a user. A payout is its own
Zcash transaction; "final" in the exit path means *that* transaction confirming,
not an anchor covering it. Anchoring less often trades anchor cost against how
long a state root waits to be committed, and nothing else.

Exits need no manual batching. `settle.rs` groups pending exits canonically,
oldest-first, signs one transaction-sized group against the vault's durable
nonce, and burns the units only once it is final — roughly twenty exits per
finality window. The group is derived from state, so nobody chooses who is in
it; introducing a manual or scheduled run would replace that property with an
operator decision.

**The race to watch.** `Exit::may_cancel` checks elapsed epochs and nothing
else — it cannot see that a payout is already signed and broadcast. If
settlement stalls past `exit_timeout_epochs`, a user may cancel, be credited
back on Zyn, and *also* receive the ZEC when the in-flight transaction lands.
The only defence is margin:

```
exit_timeout_epochs  >  worst-case settlement + confirmations + longest
                        outage tolerated without intervention
```

So if settlement stops, that is not merely a liveness problem to fix at leisure.
Past the deadline it becomes a correctness problem, and the response is to
restore settlement or raise the timeout *before* the deadline passes — not
after.

### What `rpc.cavebro.win` will serve

Not a proxy: a **`zyn-replica`** (§40.2, §48.2), which verifies the chain from
Zcash and the DA mirror alone and sequenced nothing. "The cavebro team runs an
RPC" is then literally true rather than a re-label of our own sequencer. It
needs the testnet Zebra above, the vault **viewing** key, and Hetzner's mirror
at `:8181` — which already serves `chain-11/index` over plain HTTP.

The raw IP stays the code default until the name resolves *and* answers.
Flipping `DEFAULT_NODE` first would break every client for the gap between.

## 5g2. `zec-rpc.cavebro.win` — the public Zcash RPC (11 Sep 2026)

DECISIONS §53. nginx on Contabo terminates TLS; **Zebra stays on loopback** and
nothing reaches it except through the filter.

- Method allowlist in **njs**, `/etc/nginx/njs/zecrpc.js`. Not an nginx `map`:
  `$request_body` is empty during the rewrite phase, so a regex map denies
  *every* request while looking like working security. njs also checks a
  JSON-RPC **batch**, which a regex cannot do honestly — one disallowed method
  refuses the whole batch.
- Absent on purpose: `stop`, `addnode`/`setban`, `getblocktemplate`/
  `submitblock`, `getpeerinfo`. Present: reads plus `sendrawtransaction`.
- 8 r/s, burst 20, 8 connections per IP; 64 KB body cap; `POST`/`OPTIONS` only.
- Certificate by certbot webroot; `certbot.timer` enabled.

To check it end to end, in one line:

```sh
curl -s -X POST -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getblockchaininfo"}' \
  https://zec-rpc.cavebro.win/ | head -c 120
# and this must return 403:
curl -s -o /dev/null -w '%{http_code}\n' -X POST -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"stop"}' https://zec-rpc.cavebro.win/
```

**`zyn-rpc.cavebro.win` resolves and answers nothing, deliberately.** Port 8099
is closed at the firewall; the Zyn RPC stays private until ZynZap is mature,
which is after Cave's secondary trading opens.

## 5g2b. Running the Cave draw (18 Sep 2026)

DECISIONS §108, A98. Runs **once**, after the close is buried. It serves
nothing, mints nothing and signs nothing — it reads the chain and prints the
assignment.

The close and reveal are taken from the published announcement, so point it at
that file rather than typing heights:

```json
{
  "announced_at_zcash_height": 3490000,
  "announced_at_drand_round": 5000000,
  "deposit_close_zcash_height": 3492000,
  "publication_txid": "<64 hex: the ZYP memo transaction>",
  "publication_height": 3490000
}
```

`publication_height` must be the height the memo actually confirmed at — the
resolver refuses when it disagrees with the announcement, which is the
backdating check (§101, §108.1).

```sh
export ZYN_SALE_MANIFEST=examples/zyn-pfp-concepts/out/content.json
export ZYN_SALE_ANNOUNCEMENT=/etc/zyn/cave-close.json
export ZYN_SALE_SCAN_FROM=<first height a deposit could have landed>
export ZYN_VAULT_FVK=<96 hex, read-only>
export ZYN_ZEBRA_HOST=127.0.0.1 ZYN_ZEBRA_PORT=8232
zyn-sale draw
```

Before the reveal, prove the beacon is reachable — an unreachable one at reveal
time is a sale that cannot be drawn:

```sh
cargo test -p zynzapd --test drand_live -- --ignored
```

It refuses, in this order, on: a manifest that rebuilds a root other than
`02b01b16…`; an announcement that gives less notice than the policy promised or
whose publication landed at another height; a missing `ZYN_SALE_SCAN_FROM`; a
tip that has not buried the close by 6 confirmations; a reveal block that does
not exist yet. Each refusal names what to fix.

**Two operators should run it and compare.** The draw is deterministic
(§106.2), so the assignment root printed by both must match; that is what
`Run::agrees_with` checks and what makes the result verifiable without either
of them being trusted.

## 5g3. `cave-node-status.pages.dev` — the node's public face (18 Sep 2026)

DECISIONS §107. A public page showing the mainnet tip, refreshing every 15s.
The browser reaches `GET /api/status` on the page's own origin and nothing
else; the Pages Function calls Zebra server-side.

The upstream is a **project secret**, not repo content:

```sh
cd apps/cave-node-status
printf 'https://zec-rpc.cavebro.win/' | \
  npx wrangler pages secret put ZEC_RPC_URL --project-name cave-node-status
```

Redeploy:

```sh
cd apps/cave-node-status
npm test && npm run build
npx wrangler pages deploy --project-name cave-node-status --branch main --commit-dirty=true
```

Check it end to end — six fields, and no seventh door:

```sh
curl -s https://cave-node-status.pages.dev/api/status
# a JSON-RPC POST must return 405, not a tip:
curl -s -o /dev/null -w '%{http_code}\n' -X POST -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getblockchaininfo"}' \
  https://cave-node-status.pages.dev/api/status
```

If the page says *"The public signal cannot reach the private node"*, the
Function is answering and Zebra is not — check `zec-rpc.cavebro.win` per 5g2
before touching this project. A missing `ZEC_RPC_URL` gives 503, not 502.

## 5h. Zyn mainnet: chain 26460 (11 Sep 2026)

DECISIONS §52, §54, §59. Everything mainnet runs on **Contabo**
(`95.111.240.24`) and depends on no home machine for liveness — only for
custody signatures.

| | |
|---|---|
| chain id | **26460**, permanent: it is inside the signing domain and every anchor commitment |
| params | `ZYN_PARAMS=v1` (production floors) **and** `ZYN_NETWORK=mainnet` — two different questions, both required |
| config | `/etc/zyn/zynzapd-mainnet.env`, mode 600 |
| unit | `zynzapd-mainnet` |
| Zcash | that host's own mainnet Zebra, loopback `127.0.0.1:8232` |
| vault | **viewing key only**; `ZYN_ZEBRA_SHARES=/var/lib/zyn/vault-public` holds `public.bin` and no share |
| custody | `ZYN_CUSTODIANS=127.0.0.1:8212,8211,8213` → Hetzner 01, Windows 03, Air 02 |
| DA | local `/var/lib/zyn/mainnet-da` plus the Hetzner mirror over a tunnel |
| base root | `da91df7dea062d274e9f972ce776c9b680472a1f834262438d3310153250b11b` |

**Health in one line.** The startup log is the check; these three lines mean it
is a mainnet posture rather than a devnet one:

```
anchors signed by 3 custodian(s), threshold 2 — this box holds no share
anchors ON ZCASH — self-sends from the vault, accepted at 12 confirmation(s)
DA bundles written to /var/lib/zyn/mainnet-da and pushed to 1 mirror(s)
```

If it instead says `anchors IN MEMORY ONLY (no ZYN_ZEBRA_SHARES)`, nothing is
settling to Zcash however healthy the rest looks.

### The custody tunnels

Each share-holder opens a reverse tunnel **to** the sequencer, so no custodian
needs an inbound hole or a public address. Hetzner is the exception: Contabo
opens an outbound tunnel to it, because Hetzner's cloud firewall blocks
inbound 8210 and the OS firewall is not the one doing it.

| share | host | local | on Contabo | key restriction |
|---|---|---|---|---|
| 01 | Hetzner | 8210 | 8212 | Contabo's key, `permitopen` 8181 + 8210 |
| 02 | Air | 8210 | 8213 | `permitlisten="127.0.0.1:8213"` |
| 03 | Windows | 8210 | 8211 | `permitlisten="127.0.0.1:8211"` |

Each key forwards exactly its own port, so a stolen custody key is worth one
forward and nothing else.

### Gotchas that cost an hour each

- **`ZYN_ZEBRA_REVEALS` must exist even when empty.** A fresh chain has no
  account attributions and the node refuses to start on a missing file rather
  than inventing one: `touch` it.
- **`ZYN_NETWORK` is separate from `ZYN_PARAMS`.** Setting only the latter
  gives `this node is on the main chain, but we are configured for test —
  refusing to run against the wrong chain`, which is the guard working.
- **Upgrade replicas and signers before the sequencer, never after.** A
  decoder refuses any `STATE_VERSION` above its own, so a newer sequencer's
  base state reads as `base state does not match its root: Decode` on an older
  replica. This bit mainnet on its first day (§59.5).

### The mainnet verifier

`zyn-replica-main` on the Windows box, chain 26460, reading **that machine's
own** mainnet Zebra rather than the sequencer's — a replica pointed at the node
it is checking is asking the party under audit (§40.2). Installed as a second
binary so the testnet replica keeps the one it was running.

## 5f. The mainnet vault, generated distributed (10 Sep 2026)

DECISIONS §42. The key was never whole on any machine, at creation or since —
unlike the testnet vault, which was dealt and had its shares copied about.

- **Address** `u1kc38pww6c8ahz0z20g9kv9s7afrc2npmr8s6nngw00qufveq43q5m2vh8u7wxpaltl0vjs7etr7y7d0pcm5dtc24pqhu5qfe5cwtyc2u`
- **Shares, one each, 2-of-3** — *moved 11 Sep, see DECISIONS §55*: Hetzner
  `/var/zyn/custody-main` (01), Air `~zhang/.zyn/custody-main` (02), Windows WSL
  `/var/zyn/custody-main` (03). The Mac holds **none**: it travels and sleeps,
  and share 01 was moved off it for uptime.
- **The mainnet sequencer (Contabo) holds no share** — it has `public.bin` only,
  and compromising it yields the coordinator and zero shares. That is the claim
  §42 exists to make, and it survives the move. Hetzner is *not* the mainnet
  sequencer; it runs testnet and the DA mirror.
- **Paper backups**: one page per share, rendered on the machine that holds it,
  stored in three different places (§60). Any two reconstruct the vault.
- **To run a ceremony**: relay on Hetzner **loopback**
  (`systemd-run --unit=zyn-dkg-relay --setenv=ZYN_DKG_LISTEN=127.0.0.1:8120
  /usr/local/bin/zyn-dkg relay 3`), each participant reaching it through its
  existing managed tunnel — `permitopen` for 8120 is in Hetzner's
  `authorized_keys`, and 8120 is in `zyn-tunnel` on Windows and
  `dev.zyn.tunnel` on the Air. Then
  `zyn-dkg join 127.0.0.1:8120 <index> 2 3 <out-dir>` on each, simultaneously.
- **Ad-hoc `nohup` and `ssh -f` forwards do not survive the session that
  created them.** That cost three failed attempts. Use the managed units.
- **Verify three things** before trusting the result: every participant derived
  an identical `public.bin`, each directory holds **exactly one** `share-*.bin`,
  and `zyn-dkg address <dir> mainnet` agrees on all three. Check the address
  *before* funding — a vault born on the wrong network is discovered by
  whoever pays it.

## 5f-octies. The object testnet (22 Sep 2026)

Chain 212 runs on **Hetzner**, where the vault, Zebra and custodians already
are — a settler that had to reach them over a tunnel would add a failure mode
for nothing.

```
zyn-object-node.service     the chain, anchoring, DA bundles
zyn-object-tunnel.service   exposes its RPC on the public gateway (8500)
/etc/zyn/zyn-object.env     0600; carries ZYN_VAULT_FVK
```

`https://zyn-rpc.cavebro.win/object` is served through the tunnel from
Contabo's nginx. The old devnet node on Contabo is stopped and disabled.

**Anchoring is opt-in and all-or-nothing.** Without a Zebra, a vault viewing
key and custodians, the node says `DEVNET … anchoring OFF` and carries
nothing. With them it says `TESTNET … anchoring ON`, and the status JSON says
the same — it reported `devnet` while anchoring was live until 22 Sep, which
is the wrong lie in either direction.

**It anchors from the vault chain 11 used.** 234 Ironwood notes, ~0.0001 TAZ
burned per anchor, so no faucet request is needed for years. Anchors are paid
from **Ironwood**, never Orchard: post-NU6.3 the Orchard pool forbids
cross-address transfers by consensus, so a payout from it must go to a
transparent output and the exit becomes public.

**The note trees must reach the tip before the first anchor.** Seeding fixes
only where a tree starts; until it is walked forward the vault's notes are on
chain and invisible, and every anchor fails with *waiting for a spendable
Ironwood note* while the money sits there. Each batch is persisted, so a
restart resumes rather than starting again — catch-up from the vault's birth
is hours, not minutes. **Do not quote a completion time for it.**

## 5f-quaterdecies. The launchpad: curve terms, testnet, a local devnet (23 Sep 2026)

**Terms** (operator, `zyn_submit` with the node's token, never printed): intent
`25 ‖ program("configure-curves") ‖ fee_bps u32 ‖ bond i128` (DECISIONS §141.5).
Testnet 212 has 100 bps and no bond. Ceilings: 2%, 1 ZEC.zy. A token keeps the
fee in force when it was made.

**Replicas must understand ops 23–25 before any curve intent reaches their
chain**, or they stop verifying at the first one. Contabo's testnet replica was
updated 23 Sep (backup `/root/zyn-object-replica.bak-20260923`).

**A local devnet for the whole life** (graduation needs ~17 ZEC.zy, more than
testnet faucets give): run `zyn-object-node` with `ZYN_CHAIN_ID=212`, its own
`ZYN_DATA_DIR`, and a `ZYN_SUBMIT_TOKEN` of 32+ characters (shorter is refused);
no anchoring config means a devnet. Credit test ZEC.zy with an operator
`Deposit` (op 6) sealed to the account's viewing key — the same shape
`ingress::credit` builds. Point nap-web's preview at it with
`NAP_DEV_RPC=http://127.0.0.1:8399`.

**Mainnet 26460 (23 Sep):** node and Windows replica run the curve-aware
build (backups `/root/zyn-object-node.bak-20260923b` on Contabo,
`/usr/local/bin/zyn-object-replica.bak-20260923` in WSL `zyn`). Fees, the Cave Z
registration and curve terms were **set on 24 Sep 2026** (receipts `FeesSet
{bridge_bps: 50}`, `CollectionRegistered` for `024277b6…5f1e` — creator
Treasury `b3e4c493…fe99`, quote ZEC.zy, 100 bps, cap 4,444 — and
`CurveTermsSet {fee_bps: 100, bond: 0.05}`), through the same operator door as
testnet's — token read on the box, never printed. The Windows replica has
nothing to replay yet: no DA bundle exists until the first anchor, which waits
on a funded vault.
Without terms mainnet curves charge nothing and need no bond.

**Jade** builds with `npm run build:launchpad` (to `apps/cave-market/dist`,
which the store build also uses — build one, deploy it, then the other) and
deploys to the `cave-jade` Pages project. `?network=testnet` reads chain 212.

### Cave Jade: deploy, content store, history (23 Sep 2026)

- **Deploy** from `apps/cave-market/jade-stage` (its `wrangler.toml` binds KV `CAVE_JADE_CONTENT`, id `6fec82c33ade4ebba5836456d67cdb04`; `functions/` are the Pages Functions): `node scripts/build-launchpad.mjs && cd jade-stage && npx wrangler@4.40.0 pages deploy dist --project-name cave-jade --branch main`.
- **Content** lives at KV `c:<token>`. To hide a token's picture and description: `npx wrangler@4.40.0 kv key put --namespace-id 6fec82c33ade4ebba5836456d67cdb04 hide:<token> 1 --remote`; delete that key to show it again.
- **Local:** `npx wrangler@4.40.0 pages dev dist --port 8788 --kv CAVE_JADE_CONTENT` in `jade-stage`; a local node has no CORS, so put a proxy in front of it and open `/?network=testnet&rpc=http://127.0.0.1:<proxy>` (the `rpc`/`nap` overrides work on localhost only).
- **History** is `chain-<id>.history` in the node's data dir (JSON lines, append-only). Losing it loses only charts; it restarts from the next epoch.
- **Live e2e:** `JADE=… NAP=https://nap.cavebro.win TOKEN=<testnet curve token> PHRASE_FILE=<funded testnet phrase> node apps/nap-connect/tests/jade.e2e.mjs` — connect, buy with a floor, auto-collect, sell a lot, auto-collect.

## 5f-septendecies. SOL.zy and ETH.zy bridges, testnet (24 Sep 2026, DECISIONS §146)

**SOL (Solana devnet ↔ testnet 212)** — `zyn-sol-bridge.service` on Hetzner, env `/etc/zyn/zyn-sol-bridge.env` plus the node's own env for the token. Vault `7KacSYXuVhKSf8dpXcLY3qFZQYiZg2m6vKh4m1HxKZmA` (FROST 2-of-3; public package `/var/zyn/solana-shares/public.bin`; custodians `127.0.0.1:8110/8111/8112`), durable nonce `BrRELAaQBLe3Yx3vNad8jjxDdJZTZ5SmwLHRFTFyd61H`. State in `/var/lib/zyn/sol-bridge` (`deposit-cursor`, `inflight.json` — a damaged `inflight.json` stops the bridge on purpose: it is the record that prevents paying twice). `zyn-sol-bridge status` shows the vault balance against what Zyn has issued. Top up the vault's lamports for rent/fees with a plain transfer (no memo = credits nobody).

**ETH (Sepolia ↔ testnet 212)** — deployed 24 Sep 2026: `ZynSigners` `0x95cb60b0e6FE122ea0AA21725E83e3ef00AEfa41` (2 of owners `0x395b…fbb2`, `0x096E…F6e3`, `0x4966…381e`), `ZynVault` `0xe17bb5943b34133f6a8a5D221BE4eD9493B3ef27` (asset 1 = native ETH, admin and gas payer `0xfFb1…4E89`). `zyn-eth-bridge.service` on Hetzner, keys in `/etc/zyn/eth-keys/` (0600). How it was deployed:

```sh
# keys: scratchpad eth-sepolia/{owner1,owner2,owner3,payer}.key (0600)
RPC=https://ethereum-sepolia-rpc.publicnode.com
forge create src/ZynSigners.sol:ZynSigners --rpc-url $RPC --private-key $(cat payer.key) --broadcast \
  --constructor-args "[$(cat owner1.addr),$(cat owner2.addr),$(cat owner3.addr)]" 2
forge create src/ZynVault.sol:ZynVault --rpc-url $RPC --private-key $(cat payer.key) --broadcast \
  --constructor-args <signers> $(cat payer.addr)
cast send <vault> "registerAsset(uint32,address)" 1 0x0000000000000000000000000000000000000000 --rpc-url $RPC --private-key $(cat payer.key)
```

then `zyn-eth-bridge register` and a `zyn-eth-bridge run` unit (env `ZYN_ETH_RPC`, `ZYN_ETH_VAULT`, `ZYN_ETH_KEYS`, `ZYN_ETH_PAYER`, `ZYN_ETH_STATE=/var/lib/zyn/eth-bridge`), and the vault in `apps/nap-web/public/config.json` → testnet `bridges.eth` (`{"chainId":11155111,"vault":"0x…","assetId":1,"explorer":"https://sepolia.etherscan.io/tx/{tx}"}`).

Both bridges refuse mainnet by design (§146.10).

## 5f-sedecies. Running Collection Z's sale (24 Sep 2026, DECISIONS §144)

The operator's half is one binary, `zyn-sale-draw`, run on the box that holds the node — it reads the submit token from the node's env file and writes the draw beside the node's data so the node can serve it.

```sh
# on Contabo (mainnet); the token never appears on screen
set -a; . /etc/zyn/zyn-object-mainnet.env; set +a     # ZYN_SUBMIT_TOKEN, ZYN_ZEBRA_*
export ZYN_SALE_NETWORK=mainnet ZYN_SALE_NODE=http://127.0.0.1:8599 \
       ZYN_SALE_DATA=/var/lib/zyn/object-mainnet \
       ZYN_SALE_ANNOUNCEMENT=/root/cave-z-close.json \
       ZYN_SALE_MANIFEST=/root/content.json               # examples/zyn-pfp-concepts/out/content.json
zyn-sale-draw status
```

1. **Open** — `zyn-sale-draw open` posts `SetSale` at the terms' prices (0.01 / 0.005 / 0.0133, 10% treasury). CAVEZ must already be registered (it is on 26460).
2. **Announce the close** — publish `{announced_at_zcash_height, announced_at_drand_round, deposit_close_zcash_height}`. The close must be ≥ 1,152 blocks after the announcement, and **the drand round must be late enough that round + 2,400 falls after the close** (drand quicknet: round = (unix − 1692803367)/3 + 1). `close` refuses otherwise.
3. **Close** — `zyn-sale-draw close` once Zcash reaches the close height; it posts `CloseSale` with that block's hash.
4. **Draw** — `zyn-sale-draw draw` once the reveal block (close + 100) and the reveal drand round both exist. It checks the manifest rebuilds the committed root, runs the engine over every ticket in order, writes `sale-<collection>-draw.json` into the node's data dir (served by `zyn_saleDraw`) and posts the root. Publish the same file anywhere you like — its root is what counts.
5. **Open mint** — `zyn-sale-draw open-mint` after the claim window (16,128 blocks from the reveal block). It writes `sale-<collection>-open.json` and posts the order's root.

**Burn tickets (DECISIONS §145).** Once the token exists, add `apps/cave-collection-z/burn.json` (`{"cluster":"mainnet","symbol","mint","decimals","per_ticket","token_program","rpc"?}` — `rpc` optional, a paid endpoint that answers browsers; the build adds its origin to the policy), redeploy the store, and on Contabo:

```sh
export ZYN_BURN_MINT=<mint> ZYN_BURN_PER_TICKET=<base units per ticket> \
       ZYN_BURN_NODE=http://127.0.0.1:8599 ZYN_BURN_CURSOR=/var/lib/zyn/burn-cursor   # + ZYN_SUBMIT_TOKEN from the env file
zyn-burn-watch cap <n>      # after `open`: how many burn tickets the sale takes
zyn-burn-watch run          # as a service; `status` shows tickets left
```

Run `zyn-burn-watch once` immediately before `zyn-sale-draw close`, so burns that finalized in time are posted before the door shuts. A paid RPC (`ZYN_BURN_RPC` for the watcher, `rpc` in `burn.json` for the page) is worth it on launch day — the public one rate-limits.

Rehearsal: `ZYN_SALE_POLICY=rehearsal` (2-block notice, 2-block reveal, 60 drand rounds; `ZYN_SALE_CLAIM_WINDOW`, default 2). Its digest differs from the published policy, so it is for devnets and testnet only. The 24 Sep rehearsal ran against a local devnet (`scratchpad/devsale/sale.mjs` drove three wallets through Nap Connect's own methods) with testnet Zcash blocks from the Hetzner Zebra.

## 5f-quindecies. ZynScan (24 Sep 2026)

- **Mainnet** `zynscan-mainnet.service` on Contabo: `ZYN_CHAIN_ID=26460`, `ZYN_SCAN_DATA=/var/lib/zyn/object-mainnet`, node `127.0.0.1:8599`, listens `127.0.0.1:8093`.
- **Testnet** `zynscan.service` on Hetzner: chain 212, data `/var/lib/zyn-object`, node `127.0.0.1:8499`, listens `127.0.0.1:8093`; `zyn-object-tunnel` also forwards it as Contabo `127.0.0.1:8593` (unit backup `/root/zyn-object-tunnel.service.bak-20260924`).
- **Gateway** `zyn-rpc.cavebro.win/scan/` and `/scan-testnet/` (nginx `sites-enabled/zyn-rpc`, backup `/root/nginx-zyn-rpc.bak-20260924`): GET/HEAD/OPTIONS only; zynscan's own CORS headers are hidden so the server-wide ones are the only ones.
- **Site** `apps/zynscan` → Pages project `zynscan`: `node scripts/build.mjs && npx wrangler@4.40.0 pages deploy dist --project-name zynscan --branch main`.
- Both units run as root under `ProtectSystem=strict` (read-only filesystem; the mainnet data dir is root-only). zynscan replays from genesis on start and whenever the journal stops matching what it read; `/api/summary` `agrees` must be `true` — `false` means the journal no longer reproduces the node's root, which is a finding, not a restart.
- Deploy a new build like the node: zigbuild (`aarch64` Hetzner, `x86_64` Contabo), `install -m755` to `/usr/local/bin/zynscan`, restart the unit.

## 5f-terdecies. The block servers' HTTP door, for browser Nap (23 Sep 2026)

`zyn-lightd` answers HTTP as well as its TCP frames when `ZYN_LIGHTD_HTTP` is
set: POST one request frame, get the reply frame back (DECISIONS §139).

- **Hetzner (testnet):** `ZYN_LIGHTD_HTTP=127.0.0.1:8095` in
  `/etc/zyn/zyn-lightd.env`. TCP stays on `0.0.0.0:8098`.
- **Contabo (mainnet):** drop-in
  `/etc/systemd/system/zyn-lightd.service.d/http.conf` with the same port. TCP
  stays on `127.0.0.1:8097`.
- Check either: `printf '\001' | curl -s --data-binary @- http://127.0.0.1:8095/ | xxd | head -1`
  → `00 01…` (mainnet) or `00 02…` (testnet), then the tip.

**Routed 23 Sep:**

1. Hetzner's `zyn-object-tunnel` carries `-R 127.0.0.1:8595:127.0.0.1:8095`
   beside the RPC forward, so testnet's door is on Contabo at :8595 (the
   `hetzner-object-tunnel` key has no listen restriction). Backup of the old
   unit: `/root/zyn-object-tunnel.service.bak-20260923` on Hetzner.
2. `zyn-rpc.cavebro.win` on Contabo's nginx: `location = /zec` → :8095
   (mainnet), `location = /zec-testnet` → :8595, POST/OPTIONS only, 1 MB
   bodies, 60 s reads, own rate zone `zynzec` (20 r/s, burst 60;
   `/etc/nginx/conf.d/zyn-zec-limits.conf`), since all browser traffic
   comes from Cloudflare's few addresses. Backup:
   `/root/zyn-rpc.nginx.bak-20260923`.
3. nap-web deployed; `functions/zec/[[path]].js` maps `/zec/mainnet` →
   `$ZYN_RPC_URL/zec` and `/zec/testnet` → `/zec-testnet`.

Check from anywhere: `printf '\001' | curl -s --data-binary @- https://nap.cavebro.win/zec/mainnet | xxd | head -1`.

Local testing without any of that: `node relay.mjs 168.119.53.39 8098 18095`
(HTTP → testnet's public TCP port), then
`NAP_DEV_ZEC=http://127.0.0.1:18095 npx vite preview`.

**Rebuilding the core:** `apps/nap-web/zec-core/build.sh`. It needs the
wasm32 target, `wasm-bindgen-cli` 0.2.128 and Homebrew's llvm, and writes
`apps/nap-web/src/zec-pkg`, which is committed so a Pages build needs no Rust.

## 5f-duodecies. Fees into the ZYN pot (23 Sep 2026)

Operator intents, sent through `zyn_submit` with the node's `ZYN_SUBMIT_TOKEN`
(read on the box, never printed). The intent hex for each is in
`apps/nap-web/tests/object-vectors.json` → `market` for the fixture values;
build others with `zyn-object-vectors` or `ObjectVm::encode_intent`.

```sh
T=$(grep '^ZYN_SUBMIT_TOKEN=' /etc/zyn/zyn-object.env | cut -d= -f2-)
curl -s -X POST 127.0.0.1:8499 -d "{\"method\":\"zyn_submit\",\"token\":\"$T\",\"intent\":\"<hex>\"}"
```

- `SetFees {bridge_bps}` — 50 bps is §30's rate. **Testnet 212: set to 50 on
  23 Sep.** Mainnet: not set.
- `RegisterCollection {asset, creator, quote, fee_bps, cap}` — once per
  collection, immutable. Cave Collection Z: asset `024277b6…5f1e`, 100 bps a
  side, creator Treasury 26460, quote ZEC.zy, cap 4444. Register before the
  first mint.
- **Delivering an item** (the primary sale): `zyn_mintItem {token, collection,
  owner, serial}` — serial as a string. The node seals it to the buyer's
  registered viewing key; the buyer must have registered (Nap does on first
  open). Each serial mints once, never past the cap.

`zyn_market` shows `fees`, `zyn_pot` and `collections`; check the pot against
the fees after any credit, withdrawal or sale.

## 5f-undecies. The bridge from the wallet (23 Sep 2026)

**In:** Nap asks `zyn_depositAddress {account}`; if the answer says
`registered: false` it signs a `Register` first. The node appends every account
it gives an address to `<data>/chain-<id>.deposit-accounts` and recognises
those addresses from then on, registered or not — a payment before registering
waits as a debt (`chain-<id>.pending`) and is credited when the account
registers. It no longer falls through to the memo rules and stops the scan.

**Out:** Nap picks a random salt, asks `zyn_exitCommitment {address, salt}`,
signs `Withdraw` naming that commitment, then sends `zyn_exitReveal {address,
salt}`. The node accepts a reveal only when an unpaid exit already carries the
commitment, hands it to the exit settler and appends `<address> <salt>` to
`ZYN_EXIT_REVEALS`. The wallet keeps unrevealed exits in `localStorage` and
retries on every refresh. The Zcash fee comes out of the payout.

**Anchors never spend backing.** The anchor builder refuses unless the vault
holds issued ZEC.zy + unpaid exits + waiting deposits + 20,000 zat. The
difference is the **fee reserve**, topped up with memo `ZYN1:` + 64 zeros
(credited to nobody). Without one the node logs `anchoring paused: … top up the
fee reserve by at least N zat` — deposits and exits still work, roots just are
not carried to Zcash until it is funded.

## 5f-decies. The market (23 Sep 2026)

Pools, batch swaps and token launches live in the object VM (`objectvm::market`)
and are on every object node from this build on. Nothing turns them on: the
first `CreatePool` or `CreateLaunch` does, and until then the chain's roots and
saved state are byte-identical to before.

**What the public sees:** each pool's reserves, shares and fee, and each sale's
terms and total raised — `zyn_market`. **What it does not:** who traded,
provided liquidity or committed. Those records sit in the root only as leaves
blinded by the user's own nonce.

**Rhythm:** a swap is escrowed in the epoch it arrives in and clears at that
epoch's seal, every order in a pool at one FM-AMM price. Proceeds, refunds,
pool shares and sale allocations become **claims**; the owner collects them
with a signed `Claim`. `zyn_claims ids=<hex,…>` answers only for ids, which
derive from the owner's nonce — there is no "claims for this account".

**Sales close at an epoch, and epochs need to move.** While any sale is open
the node seals on the clock (`ZYN_EPOCH_SECS`) even when nobody trades, so
"open for 60 epochs" means at most 60 × that. With no sale open, empty
epochs are not sealed, as before.

**Front end:** `/market.html` in nap-web, the same origin as the wallet, so it
opens the kept wallet with the same password and reaches the chain through the
same `/rpc` route.

**Local devnet** for trying it end to end, without touching either chain:

```sh
ZYN_CHAIN_ID=26460 ZYN_LISTEN=127.0.0.1:8399 ZYN_DATA_DIR=/tmp/dev \
ZYN_EPOCH_SECS=15 ZYN_SUBMIT_TOKEN=$(openssl rand -hex 24) zyn-object-node
cd apps/nap-web && npm run build && NAP_DEV_RPC=http://127.0.0.1:8399 npx vite preview
```

Fund a devnet wallet with operator `Deposit`s through `zyn_submit` (the token
above); a `Mint` creates no ciphertext, so a wallet cannot see it. Use
`vite preview`, not `vite dev`: the dev server injects styles inline and the
shipped CSP blocks them.

## 5f-nonies. Sealed journals and the object replica (23 Sep 2026)

Both object ledgers seal their published journals. `ZYN_DA_KEY` (64 hex) is
in `/etc/zyn/zyn-object.env` on Hetzner and `/etc/zyn/zyn-object-mainnet.env`
on Contabo; each was generated on its server and has never left it except
by pipe. Compare keys by fingerprint, never by printing them:

```sh
grep '^ZYN_DA_KEY=' <file> | cut -d= -f2 | tr -d '\n' | sha256sum | cut -c1-12
```

(testnet `804d5ff38223`, mainnet `424708a31869`.) A key that is set but
malformed stops the node; an unset one starts it with a loud
`journals will be published UNSEALED`.

Contabo's `zyn-object-replica` holds the **testnet** key in
`/etc/zyn/zyn-object-replica.env` (a drop-in adds the EnvironmentFile). It
replays from genesis every pass and must print
`chain 212 verified through epoch N`.

**What an anchor commits:** the seal's *checkpoint* root — the state just
before the seal advances the epoch — not the state after it. And the I/O
limit is part of the root: the node's default is 16 (`ZYN_MAX_IO`), and a
verifier must run the same one or it disagrees with an honest chain.

**A bundle carries every epoch since the previous anchor**, read from the
ledger, not the anchor's own `epochs` count: a node restarted between a seal
and its anchor forgets the pending epoch and undercounts. Testnet's first
anchor (epoch 1) went out covering only epoch 1 that way, and unsealed. It
was repaired in place:

```sh
ZYN_DA_KEY=$(grep '^ZYN_DA_KEY=' /etc/zyn/zyn-object.env | cut -d= -f2) \
ZYN_CHAIN_ID=212 zyn-da-reseal 1 0 /var/lib/zyn-object/da /var/zyn/mirror
```

`zyn-da-reseal <anchor-epoch> <first-epoch> <dir>…` rewrites only journal
files; the anchor, snapshot and certificate are untouched, and the replica
still checks the journal produces them.

## 5f-septies. Attributions are keyed on the byte order you cannot see (22 Sep 2026)

A shielded deposit with no Zyn memo stops the note-tree walk dead —
`Unattributable` — and until it is attributed the vault's notes stay invisible
and every anchor fails for want of a note sitting a few blocks further on.

The trap: **Zcash txids are displayed reversed.** `getblock`, every explorer
and every log line you are likely to copy from show one byte order; the
scanner keys `ZYN_*_ATTRIBUTIONS` on the other. An entry in the wrong order
matches nothing and is indistinguishable from a missing one, so the symptom is
"I attributed it and nothing changed".

Getting from one to the other:

```sh
python3 -c "print(bytes.fromhex('<txid>')[::-1].hex())"
```

The error now names the transaction, in the order the file wants, so this
should not need diagnosing twice. It previously reported only a height and an
amount, which left an operator eliminating candidates by hand — that cost most
of an afternoon here.

**The all-zero account means nobody**: it raises the vault's balance and
credits no user, which is exactly what faucet funding is. Attribute a top-up
that way rather than inventing an owner for it.

## 5f-sexies. Checking the object ledger (22 Sep 2026)

```sh
ZYN_CHAIN_ID=212 ZYN_DATA_DIR=/var/lib/zyn-object \
ZYN_DA_DIR=/var/lib/zyn-object/da zyn-object-publish
```

Two answers. **Publishable** means every closed epoch ends with a seal — the
check a publisher runs before building a bundle, and the one a replica refuses
without. **Replayed** applies the journal from genesis and prints the root it
reaches; that root must equal the one `/object` serves.

```
chain 212: 2 epoch(s) on disk, 2 publishable, 0 refused
replayed 4 intent(s) across 2 epoch(s) -> root f3461acf…
```

Replay is the answer that matters. "Publishable" only says each epoch is
closed; replay says the intents actually produce the state being served. An
epoch whose records were filed under the wrong number passes the first and
fails the second — which is precisely how chain 11 ended up with an anchored
root its own intents could not reproduce.

**Three traps, all of which cost time here:**

- **Seals are records.** A v2 journal holds *authorized* intents, so a bare
  one decodes as nothing and the epoch reads as never closed. `Authorized::
  operator` is the honest envelope for something arriving on the operator's
  own RPC — it names who vouched rather than implying a signature nobody made.
- **Replay applies seals too.** A seal advances the epoch and is part of what
  the root commits to. Skipping them reaches a different root and reads as a
  corrupt journal when nothing is wrong.
- **State and journal must start together.** Deleting one and keeping the
  other leaves a chain that cannot be replayed from genesis, which is the
  single property it exists to have. If you wipe, wipe both.

The newest epoch is normally still open and therefore not publishable. That is
the system working, and the tool says "still open" rather than counting it as
a refusal.

## 5f-quinquies. Chain 11 is retired (22 Sep 2026)

Stopped and disabled on Hetzner. Nothing is deleted: 10,442 epoch files and
64 MB of state stay on the volume, because A126's sequence collision is the
only surviving evidence of a bug worth understanding.

**Why, rather than repairing it.** Its history is poisoned at epoch 9190 — one
sequence number carrying two different intents, so the anchored root
corresponds to no consistent intent set (A126). It held 0.1 **TAZ** from a
faucet and two accounts, both ours. It was sealing an epoch every five seconds
that could never anchor — 1,251 of them by the end — and its degraded signer
set generated alerts that masked real problems on mainnet. Keeping it running
was not neutral; it was noise plus a growing pile of unanchorable epochs.

It also proved nothing that mainnet does not: **26460 anchors to Zcash for
real.** Chain 11's remaining value is forensic.

`/testnet` now answers **410 Gone** with a JSON-RPC error naming the date and
pointing at `/object` and `/`. A 502 from a dead upstream reads as an outage,
and the difference matters to anyone deciding whether to wait for it.

**The signer and custodian hosts stay as they are.** They were rearranged to
one identity per machine earlier the same day and are the set the next testnet
should use; tearing them down to rebuild them would be work for its own sake.

Config backups: `/root/zyn-rpc.bak.retire-testnet` and `.bak.retire2`.

**Both chain-11 JSON-RPC gateways are stopped and disabled** (22 Sep) —
`zyn-rpc.service` on Hetzner and `zyn-rpc-testnet.service` on Contabo. There
were two, which is why the first sweep found only one: the search was for
services on the box that ran the chain, and the second gateway sat on the
*other* box, relaying across to `168.119.53.39:8099`. When retiring a chain,
look for its clients on every host, not on its own.

The Hetzner one listened on `127.0.0.1:8095` relaying to the sequencer's
`:8099`, and it outlived both: `:8099` had no listener, nothing in nginx on
either box routed to `:8095`, no other unit referenced it, and it held no
established connection. It stayed up for a week answering for a chain that was
gone — a gateway to nothing is not harmless, it is a thing that looks like
infrastructure. The object ledger needs no equivalent: it speaks JSON over
HTTP itself, which is what `zyn-rpc` existed to provide for a binary node.

## 5f-quinquies. Mainnet on the object ledger — chain 26460 (handed over 23 Sep 2026)

**Live.** `zyn-object-mainnet.service` on Contabo, `127.0.0.1:8599`, config
`/etc/zyn/zyn-object-mainnet.env` (0600). Public at the gateway's root,
`https://zyn-rpc.cavebro.win/`. The AMM chain (`zynzapd-mainnet`) and its
gateway (`zyn-rpc-mainnet`, `:8096`) are stopped and disabled; its state,
journal and trees stay in `/var/lib/zyn/mainnet-v12`, snapshotted to
`/root/amm-26460-final-20260922.tgz` (0600). Its published history is at
Hetzner's `/var/zyn/mirror/chain-26460-amm-retired-20260922`. Bridge floor
**3,492,731**. Started with trees at the tip (3,492,729), copied from the AMM
chain.

**It does not anchor yet.** The vault holds one Ironwood note worth less than
an anchor costs, and a top-up is not planned soon. Epochs seal and are served;
roots reach no chain until the vault is funded. The status says
`anchored_epoch: 0`, and the bot pages when the first one lands.

**What retired with the AMM chain: NAPMOLE, 333 items.** Collection
`d3090c31228b5614ddd13e7ff29bb251e211cfbfe1057a279be2a67460619261`, cap 333,
all 333 held by the creator `b3e4c493bb79349270260f7d2e2882b4f1a736348935493331e18c95098cfe99`,
minted in epochs 2–4 — anchored, since the last anchor was 2457. Ours, and to
be minted again on the object ledger. The exact content hashes and serials are
in the retired state; `zyn-inspect /var/lib/zyn/mainnet-v12 26460` lists them
and their holders without a key. A collection `CAVEZ` (cap 4444) existed with
nothing minted.

**The handover took two passes, and the script now catches both causes.** Its
safety check counted ZEC backing only, so it would have retired a chain
holding 333 items without a word; it now counts outstanding collection items
and needs `--retire-collections` to proceed. And the object-node binary on
Contabo predated the bridge floor, so it ignored `ZYN_BRIDGE_FROM_HEIGHT` and
started scanning from the vault's birth — stopped before it wrote a cursor or
recorded a debt, redeployed, restarted at 3,492,730. The script now refuses a
binary that does not know the floor, and stops the ledger if the floor line is
missing from its log after start.

### Where the mainnet vault's shares are

Three shares from the 10 Sep DKG; threshold two.

| slot on Contabo | share | host | how it reaches Contabo |
|---|---|---|---|
| `8212` | 01 | **Hetzner**, `zyn-custodian-main`, `127.0.0.1:8210`, `/var/zyn/custody-main` — moved from the Mac on 11 Sep | `zyn-mirror-tunnel` on Contabo, `-L 8212:127.0.0.1:8210` |
| `8213` | — | **MacBook Air** | reverse tunnel *into* Contabo, key `air-custody-tunnel`, open since 22 Sep 12:36 |
| `8211` | — | **Windows box** (HP Pro Mini, `192.168.1.21`), inside the WSL distro `zyn` | key `win-custody-tunnel` is authorized; up since 23 Sep, in the object ledger's list |

Two of two live shares at threshold two: one outage stops anchoring and exits.
The third's placement is undecided.

**Earlier text, kept for the reasoning:**

**Why 26460, the AMM chain's id.** Both ledgers custody the same vault from
the same height (same viewing-key fingerprint, both from 3,479,600). Side by
side under *any* ids they would both credit every deposit — two ledgers
issuing against one backing — and both would build anchors from the same
notes, so Zcash refuses one. The AMM chain therefore stops before the object
ledger starts, and that frees the id. It is the id the wallet config, the
extension, the docs and the gateway root already use.

**The handover is a script**, `/usr/local/sbin/zyn-handover-26460`
(source `ops/mainnet/`). With no argument it checks and prints; with
`--execute` it:

1. refuses unless the AMM chain holds nothing — backing `0.0`, pools `0`
   (as of 22 Sep: backing 0.0, pools 0, 2 accounts, both ours);
2. stops and disables `zynzapd-mainnet` — its state and journal stay on disk;
3. reads where its bridge stopped and writes that + 1 as
   `ZYN_BRIDGE_FROM_HEIGHT` (A139);
4. copies its `notes-26460-*.tree` — this vault's trees, at the tip — so the
   object ledger does not walk 13,000+ blocks;
5. moves Hetzner's `/var/zyn/mirror/chain-26460` to
   `chain-26460-amm-retired-<date>`. Publish paths are `chain-<id>/<epoch>`,
   so the new chain's epoch 0 and index would otherwise land on the old
   ones. **A verifier of the retired AMM chain reads the archived path from
   then on.** Its anchors stay on Zcash under chain id 26460 forever; the
   object ledger's settler tracks its own txids and never scans for
   sightings, and the object replica checks ±12 blocks around each of its
   own anchors, all above the handover — so neither is confused by them;
6. starts `zyn-object-mainnet` and checks it answers as 26460;
7. repoints nginx `location = /` from `:8096` to `:8599`, `nginx -t` or
   restore;
8. retires `zyn-rpc-mainnet`.

It prints the rollback. The dry run passes every precondition.

**What the handover does not fix: the vault is empty.** It holds one Ironwood
note worth less than an anchor costs (≥20,000 zat: 10,000 self-send +
10,000 ZIP-317 fee), which is why the AMM chain has not anchored since epoch
2457 at 21:52 UTC 21 Sep. Top it up from a **mainnet** Nap wallet with
`/api/topup` — it pays the shared vault address with the all-zero memo, which
every scanner reads as "credited to nobody". A plain send with no memo stops
the scan instead.

**Custody: all three shares, threshold two (since 23 Sep, 09:30).** `8212` on
Contabo is Hetzner's share 01 tunnelled in (`zyn-mirror-tunnel`,
`-L 8212:127.0.0.1:8210`); `8213` is the Air's share 02 (reverse tunnel);
`8211` is the Windows box's share 03 (`zyn-custody-tunnel-main` in WSL `zyn`,
`-R 127.0.0.1:8211:127.0.0.1:8210`, checked every two minutes by
`zyn-tunnel-watch@8211-main-…`). `ZYN_CUSTODIANS=127.0.0.1:8212,127.0.0.1:8211,127.0.0.1:8213`
in `/etc/zyn/zyn-object-mainnet.env`. One share can be down without stopping
anchoring or exits.

**The Windows box after its restart (23 Sep):** `zyn-signer` and `zyn-tunnel`
verified chain 11 and are **disabled** (chain 11 is retired); `zebrad-mainnet`
restarted on its own and filled D: before, so its restart policy is now `no`
and it is stopped; both custodians run the current build (the 8 Sep one is
kept as `zyn-custodian.sep08`).

## 5f-quater. One signer per machine (22 Sep 2026)

Chain 11's set is 2-of-3 (`ZYN_SIGNER_THRESHOLD=2`). Until today **two of the
three keys ran on one laptop**, so that laptop was simultaneously sufficient to
certify anything and necessary for anything to be certified. Both moved off.

| identity | host | init | port |
|---|---|---|---|
| `9ce67be6…` | Oracle `levu-paperd` 92.5.12.15 | systemd `zyn-signer1` | 8100 |
| `27048d32…` | Air 192.168.1.27 (user `zhang`) | launchd `dev.zyn.signer2` | 8101 |
| `cc158dc4…` | Windows box | systemd `zyn-signer` | — |

A signer endorses **anchors**; it holds no vault share and can move nothing.
Its endorsement is what releases deposits (`anchor.rs`: *"Release deposits only
after a signer set endorses each anchor"*), so a set below threshold stops
deposits becoming spendable — which is exactly what happened all of 21–22 Sep.

**A signer needs three things from elsewhere**: Hetzner's testnet Zebra
(18232), the DA mirror (8181), and the sequencer to endorse to (8099). The
first two come over an ssh tunnel whose key on Hetzner is restricted to
exactly that:

```
restrict,port-forwarding,permitopen="127.0.0.1:18232",permitopen="127.0.0.1:8181"
```

No shell, no agent, no pty. A signer reads a chain and endorses; a tunnel key
that could do more is a tunnel key that eventually will.

**`ZYN_DA_KEY` is not optional any more.** Bundles are sealed from epoch 9184,
so a verifier without the key cannot read one. signer2 had never been given it
— it was distributed to the publisher and to signer1 only — which is its own
reason that signer could not advance. Both new hosts have it; verified by
comparing digests rather than keys.

**Two traps, both hit today:**

- A manual test tunnel left running holds 18232/8181 and the service unit then
  cannot bind them. With `Requires=zyn-tunnel.service` the signer is stopped
  every time the tunnel fails, so it looks like a signer fault and is not.
  Kill stray `ssh -L` before starting the units.
- The Air was **already** running a copy of `27048d32`. Installing it a third
  time on Oracle would have put one signing key on three machines, which is
  worse hygiene than the problem being solved. One identity, one host.

**Still owed:** the retired plists sit in `~/.zyn/retired/` (0600) on the Mac
rather than being erased, so the move is reversible while the new hosts prove
themselves. The original condition — "erase them once chain 11 has anchored
from the new set" — can no longer be met: chain 11 was retired on 22 Sep and
will never anchor again. These signers endorsed an **AMM** chain, and the
testnet that replaced it (chain 212) settles through the FROST custodians
instead, so the signer set they belong to has no live chain at all. Erase them
when mainnet 26460 next anchors from this set, or when the keys are rotated —
not on a condition that cannot occur.

## 5f-ter. The tunnel watchdog (22 Sep 2026)

A reverse tunnel can be perfectly healthy and carry no forward. That is A110,
and it froze both chains again on 22 Sep.

`ExitOnForwardFailure=yes` — which these units already set — only covers
**establishment**. If the remote forward is torn down afterwards, because the
server reaped it or a half-dead predecessor still held the port, ssh keeps a
live session with nothing on it. `systemctl is-active` says `active`,
`ServerAliveInterval` sees a responsive peer, and the sequencer cannot reach
the custodian at all. Every local signal says fine.

So the check has to ask the far end, and the sequencer already answers: its
health output names each custodian by the port the forward should land on.

```sh
/usr/local/bin/zyn-tunnel-watch <port> <chain> <unit>
```

Installed on the Windows box as `zyn-tunnel-watch@<port>-<chain>-<unit>.timer`,
every two minutes:

```
zyn-tunnel-watch@8211-main-zyn-custody-tunnel-main.timer
zyn-tunnel-watch@8112-testnet-zyn-custody-tunnel.timer
```

Three things it deliberately does **not** do:

- **It does not act on one failed poll.** Two consecutive strikes are required.
  A single failure is as likely to be the path to the sequencer as the tunnel,
  and a restart on a blip is its own outage — a restart races the session it
  just killed for the port, which is precisely how this failure reproduces.
- **It does not count an unreachable sequencer as a strike.** That says nothing
  about the tunnel.
- **It does not guess the chain.** This host holds a share of both vaults and
  the two tunnels land on different sequencers, so the chain is part of the
  instance name. An unknown chain exits 64 rather than checking the wrong one.

Sources are kept in `ops/tunnel-watch/` so the box is reproducible.

**The Mac still needs an equivalent.** It runs signers 1 and 2 under launchd,
and both their tunnels were dead on 22 Sep with `status 255` while the signers
themselves ran. launchd has no timer units; a `StartInterval` agent doing the
same check is the shape.

## 5f-bis. The object-ledger testnet — chain 212 (21 Sep 2026, promoted 22 Sep)

DECISIONS §133. **Chain 212 is the testnet.** It replaced chain 11, which was
retired on 22 Sep 2026 (A126/A128). Anything still naming chain 11 as "the
testnet" is stale: that chain produces no epochs and its RPC has no listener.

It exists so Cave's mint, listings and claims can be exercised against
something real before they are exercised against money.

- **On Hetzner** (`168.119.53.39`), `zyn-object-node.service`,
  `127.0.0.1:8499`, config `/etc/zyn/zyn-object.env` (0600 — it carries
  `ZYN_VAULT_FVK`), state in `/var/lib/zyn-object`. It reaches the public
  gateway through `zyn-object-tunnel.service`, a reverse tunnel to Contabo's
  `127.0.0.1:8500`, which nginx serves as
  `https://zyn-rpc.cavebro.win/object`. It ran on Contabo as a devnet until
  22 Sep; the node moved to Hetzner because that is where the testnet Zebra,
  the vault and the custodians already are.
- **Anchoring is ON.** Roots are carried to Zcash testnet and settle there,
  and the status says so in words rather than leaving a reader to guess:
  `"anchoring": "on — roots are carried to Zcash and settle there"`. The
  devnet wording (`"none — devnet, roots reach no chain"`) is still what a
  node without a Zebra, a vault FVK and custodians reports, and it is never
  the better of the two by default.
- **The deposit bridge** (22 Sep, A133). Confirmed ZEC in the vault becomes an
  object its holder can find. Opt-in on `ZYN_BRIDGE_SEED`, the way anchoring is
  opt-in: without it the banner still says objects are unbacked, because they
  are. `ZYN_ZEC_ASSET` defaults to the same `XZEC` id the AMM chains use, so one
  wallet recognises bridged ZEC on either ledger.
  - **The seed is part of the chain's identity.** Deposit nonces are
    `HKDF(seed, txid)`, so a restarted bridge rebuilds the *identical* intent
    and being refused as a duplicate is proof the work was done. Change the
    seed and the same deposit would build a different object — one the
    holder's wallet never looks for. It lives in `/etc/zyn/zyn-object.env`
    (0600) beside the vault FVK. **Back it up.** It cannot spend anything, but
    anyone holding it can link every deposit commitment to its Zcash
    transaction.
  - **Every pass checks the backing first.** The watcher attests to the
    vault's balance before the credits it covers, and a pass whose attestation
    comes up short credits *nothing* — including deposits that would
    individually have been fine. The vault is not short by one deposit; it is
    short, and no intent repairs that. The cursor does not advance either, so
    the next pass looks at the same range rather than stepping over the
    evidence.
  - **It waits for the note trees.** Before they reach the tip the vault's own
    notes are invisible, so the balance the watcher would attest to is not the
    vault's balance — and attesting too low is exactly what the solvency gate
    treats as an emergency. It runs in the same thread as the note walk, which
    is also what stops two scanners feeding one tree twice.
  - **A deposit for an unregistered holder is owed, not dropped.** It goes in
    `chain-<id>.pending` (`ZYNPEND1`) and is credited the moment that account
    registers a viewing key. Nothing expires it. Debts are written before the
    cursor: if only one survives a crash it must be the list of what is owed,
    because a cursor that is behind only costs a rescan.
- **Exits** (22 Sep, A134). `ZYN_EXIT_REVEALS`, `<address> <salt-hex>` per
  line. An object exit names a **commitment**, `H(address ‖ salt)`, not an
  address — the chain never carries where anyone's money went — so nothing can
  be paid until the holder reveals the preimage out of band. A reveal is a
  proof: it is matched by recomputing the commitment, so one naming an address
  the exit never named is a stranger, not a redirection. A line that does not
  parse refuses the whole file: a reveal the operator believes they filed and
  that was quietly dropped is an exit that waits forever for a reason nobody
  can see. `zyn_status` reports `owed_zatoshi`, counting exits nobody has
  revealed — an unrevealed exit is still a debt.
- **A replica** (22 Sep, A135). `zyn-object-replica.service` on **Contabo**,
  verifying chain 212 from the mirror on a box that does not sequence it. It
  refetches the published intents, replays them through `ObjectVm` from
  genesis, and compares the root with the one the chain published; with
  `ZYN_VAULT_FVK` and a testnet Zebra it also checks Zcash carries the anchor.
  Contabo has no testnet Zebra, so today it is **replay-only and says so on
  every start**. Tunnelling to the sequencer's Zebra would not fix that — the
  box being verified would be supplying the evidence (§118.3).
- **The wallet reads and writes it** (22 Sep, A136). `apps/nap-web/src/object.js`.
  An object ledger publishes commitments and nullifiers and says nothing about
  who owns what, so a balance is **computed**, not asked for: open every
  published note, drop the ones whose nullifier has appeared, and what is left
  is the holding. The commitment is recomputed rather than taken from the feed
  — a mirror could pair a real ciphertext with someone else's commitment.
  Amounts are `BigInt`, because WAD units exceed what a double holds exactly.
  - **Both implementations are pinned to one fixture**,
    `apps/nap-web/tests/object-vectors.json`, printed by `zyn-object-vectors`.
    `object.test.mjs` holds the browser to it; `object_vectors_are_pinned.rs`
    holds Rust to it, including a frozen ciphertext and the encoded `Register`
    and `Spend` intents. Neither side can move alone — and it has to be a
    fixture rather than a review, because the symptom of drift is **silence**:
    a wallet looks at the right chain, opens nothing, reports zero, and that
    is indistinguishable from having been robbed.
  - **The object keys ride in the sealed blob.** They come from the BIP39 seed,
    which `deriveIdentity` zeroes, and `identityFromSecretKey` — the path a
    kept wallet takes — never has it. Derived lazily, a wallet would work from
    the phrase and report empty from a device. A blob sealed before this still
    opens and says so rather than showing a silent zero.
- **The bridge floor** (22 Sep, A139). `ZYN_BRIDGE_FROM_HEIGHT=4380198` on
  chain 212. A vault outlives the ledgers that custody it, and the bridge
  scanned from the vault's *birth* — so every deposit chain 11 had already
  credited would have been credited again on 212, with each ledger's
  solvency check passing on its own. The floor is where the previous chain's
  bridge stopped: chain 11's progress file
  (`/mnt/HC_Volume_106795140/zyn-data-v12/bridge-11-<XZEC>.progress`,
  `ZYNBRDG2`, `scanned_to` is the big-endian u64 after the magic) reads
  4,380,197, written when it stopped at 12:33 UTC having credited two
  deposits. **Every handover of a vault sets this.** A saved cursor below the
  floor is raised, not trusted.
- **Scan throughput is the operational limit.** A 200-block step took the
  better part of an hour on the cax11; `ZYN_SCAN_STEP=25` now lands one every
  ~2.5 minutes, so progress is durable and visible. It is the same work per
  block either way — roughly **600 blocks/hour**, which is ~16 hours for a
  10,000-block catch-up. Sizing a box for a testnet catch-up is one thing;
  mainnet's chain is longer.
- **Objects are unbacked until that bridge is turned on.** Anchoring a root and
  backing an object are different promises, and the banner says them
  separately.
- **The routes beside it**: `/` is mainnet 26460. `/testnet` is **410 Gone**
  since 22 Sep, returning a JSON-RPC error naming chain 11's retirement and
  pointing at `/object` — a retired chain answers, rather than timing out and
  looking like an outage.
- **Methods**: `zyn_status`, `zyn_submit` (a hex intent from
  `ObjectVm::encode_intent`), `zyn_published` (commitments, ephemeral keys
  and ciphertexts — everything a wallet needs to rebuild itself and nothing
  privileged), `zyn_anchors` (the epoch-to-Zcash index, read from the
  published index file so it cannot disagree with what a replica serves),
  `zyn_offers` (resting listings, sellers omitted), `zyn_submitSigned` (the
  holders' door: a signed frame, A145), `zyn_depositAddress`, and since 23
  Sep `zyn_market` (pools and sales, public) and `zyn_claims` (what is owed,
  by id only — §5f-decies). Also `zyn_resolveAddress` (the Zyn account behind a vault deposit address), `zyn_viewingKey`, `zyn_items`, `zyn_exitCommitment`, `zyn_exitReveal`, and on the operator's token `zyn_mintItem`. `zyn_submit` is the operator's and needs
  `ZYN_SUBMIT_TOKEN` (A144). Every reply carries the request's own id (A150).
- **What `zyn_status` reports beyond the counts** (22 Sep, A129):
  `anchored_epoch` / `anchored_height` / `anchor_txid` for the last root that
  landed, and `synced_to` / `sync_target` / `sync_error` for the vault's note
  trees. Before that both lived only in journald, so "is the chain still
  reaching Zcash" was a question you could only answer over ssh.
- **To reset it**: `systemctl stop zyn-object-node; rm -rf
  /var/lib/zyn-object/*; systemctl start zyn-object-node`. Wallets that had
  scanned it will report zero afterwards, correctly, because the chain they
  scanned no longer exists.
- **A browser GET on `/object` answers with the chain's status** (22 Sep). It
  used to return nginx's raw `405 Not Allowed` page, which is what anybody
  clicking a link in a post or a chat would see — a working endpoint that
  reads as broken. nginx now rewrites a GET to an `internal` location that
  performs the `zyn_status` call on the reader's behalf (`proxy_method POST`
  + `proxy_set_body`). `POST` is untouched, and `/object-status` returns 404
  if requested directly. Config backup at `/root/zyn-rpc.bak.getstatus`.
  `/` still 405s on GET, deliberately: the mainnet sequencer's surface is not
  widened to make a link prettier. `/testnet` now answers 410 on any method.
- **The node bounds what it will read** (22 Sep). A request over 256 KB, or a
  declared `Content-Length` over it, gets a 413; a connection that stalls is
  dropped after 15 s. Before that the read buffer grew without a ceiling, so a
  lying `Content-Length` or headers that never ended grew it until the box ran
  out of memory — and that box runs the mainnet sequencer. nginx caps the
  public route already; this is so the loopback listener does not depend on
  something in front of it.
- **Resetting the devnet is expected after a state-shape change.** The state
  root gained `SECTION_LIVE` on 22 Sep, so the persisted chain would have
  served roots the new binary disagrees with. `systemctl stop
  zyn-object-node; rm -rf /var/lib/zyn-object/*; systemctl start
  zyn-object-node`. Wallets that had scanned it report zero afterwards,
  correctly — the chain they scanned no longer exists.
- **Rate limiting is shared** with the other two routes (`limit_req zone=zynrpc
  burst=20`). Hammering `/object` in a loop returns nothing and looks like an
  outage; space the requests before concluding anything is broken.

## 5g. The doormat: transparent receive and shielding (10 Sep 2026)

DECISIONS §45. Every exchange pays out transparent, so a wallet that cannot
receive one cannot be funded without a second wallet in between.

- **`GET /api/transparent`** — address, total, fee, net, outputs.
  **`POST /api/shield`** — sweeps everything to the holder's own shielded
  address. No amount, no destination: there is deliberately no way to spend
  transparent funds anywhere else.
- Derivation is **BIP-32/44 at `m/44'/133'/account'/0/index`**, so the funds can
  always be swept with another wallet.
- **`UTXOS` (op 6)** is the light protocol's one exception to "nothing that
  takes an address" — a compact block carries only Orchard actions, so there is
  nothing client-side to scan for transparent funds. Requires Zebra's
  `getaddressutxos`, which it serves.
- Four things the chain enforces, each found by being refused:
  **shielding targets Ironwood** (from NU6.3 the Orchard value balance must be
  non-negative, so value cannot flow into that pool at all); the shielded side
  is a **change** output, not a payment; **ZIP-317 charges for the bundle's
  padded action count** (two), not its single output; and `finalize_io` already
  signs the dummy spends, so a sweep signing **zero** actions is correct.
- Proven end to end on testnet: 40,000 zat swept in `dd13839…f094ef`, leaving
  the transparent balance at zero.

## 5i. Cave creator launch frontend (12 Sep 2026)

The creator surface is part of the existing Cave Market app, not another site:

```sh
cd apps/cave-market
npm test
npm run build
python3 -m http.server 4178 --bind 127.0.0.1
# open http://127.0.0.1:4178/#launch
```

The static build writes `apps/cave-market/dist`. **Building does not deploy** the
Cloudflare Pages project or change a domain. Keep it that way until a deployment
is explicitly requested.

What can be exercised now:

1. Edit collection name, supply and entry price.
2. Switch receipt quantity among **Fixed**, **Weighted draw** and **Buyer
   chooses**. Weighted rows are editable and must total 100%; add/remove supports
   up to six outcomes.
3. Choose identity assignment independently: concealed public shuffle, instant
   public randomness, or sequential indices.
4. Read expected objects and capacity in the policy card. For buyer-selected
   quantity this is a range; for a draw it is an expectation, not a reservation
   guarantee.
5. Run the 10,000-entry browser preview. It uses `crypto.getRandomValues`, is
   labelled **not the draw**, and must never be copied into backend allocation.

What still remains impossible until the A2 external adapters wrap the now-built
engine and durable `zyn-io/cave:cave-sale` ledger:

- publishing a launch and returning independently checkable timestamp evidence;
- operating the continuous Zcash scanner and its restart/progress loop;
- fetching public entropy and finalizing without an operator supplying evidence;
- signing/submitting entry credit, floor funding and mint plans with Treasury
  26460 authority;
- wiring those operations into the storefront.

### Randomness engine and backend handoff

The pure Rust engine is `cave-randomness`. It implements canonical
`cave-allocation/v1` policy bytes and digest, committed deposit-close height,
manifest and ticket commitments,
fixed/weighted/buyer-chosen quantities, concealed/instant/sequential identity,
verified drand Quicknet evidence, unbiased sampling, allocation receipts and
full replay verification. Check it independently before service work:

```sh
cargo test --manifest-path /path/to/zyn-io/cave/cave-randomness/Cargo.toml
cargo test --manifest-path /path/to/zyn-io/cave/cave-sale/Cargo.toml
cargo test --manifest-path /path/to/zyn-io/cave/cave-zcash/Cargo.toml
```

The backend must consume that API through `zyn-io/cave:cave-sale`, not
reconstruct policy from the form's DOM.
Return the digest and public timestamp evidence to the UI only after the exact
canonical bytes and manifest root have been durably published.

The ledger reserves the fixed amount, selected buyer amount, or **maximum
possible output** for an unresolved weighted entry and is idempotent by exact
deposit outpoint and ticket id. UI percentages may total 100 for authoring
clarity, but engine weights are arbitrary positive relatives and must not be
rescaled after commitment. Never fall back to local RNG if a beacon or canonical
Zcash block is unavailable: pause allocation. Verify deposit confirmation/position,
prove each future beacon target was committed before it became knowable, and
prove the reveal block hash belongs to the committed height. Persist and publish
the append-only ledger and engine receipts. The publish button can be enabled
only after those service invariants have tests and the wallet approval names the
exact digest being locked.

### Cave entry payment and floor funding

Generate the shielded payment memo with `cave-zcash::entry_memo`. It is a strict
`ZYC/v1` instruction carrying the allocation-policy digest and Zyn recipient;
never use the ordinary `ZYN/v1` account-deposit memo for a Cave sale payment.
The shared vault scanner excludes `ZYC` before address attribution, so the same
ZEC cannot become both the buyer's ZEC.zy and sale proceeds. A malformed `ZYC`
stops classification for operator review rather than falling through.

After the configured Zcash depth, feed the canonical sighting and future drand
targets through `cave-zcash::accept_entry`. For each admitted ticket:

1. Submit the ledger's `BridgeCreditPlan` as an operator `CreditDeposit` to
   Treasury 26460 and record only a matching accepted VM receipt.
2. Wait until a confirmed anchor covers the credited epoch.
3. Submit the resulting `FloorFundingPlan` with Treasury authority: 90% goes to
   `FundCollection`; the remaining 10% stays in Treasury.
4. Record only the matching funding receipt. Mint plans for that ticket remain
   unavailable until this succeeds.

Both records are idempotent and are fully revalidated on ledger restart. The
continuous scanner loop, Treasury threshold signer and later claim-payment
settlement are still service work; do not label the standalone libraries as a
live sale daemon.

Before production, replace the entry memo's current bare allocation-policy
reference with the canonical `cave-sale-terms/v1` digest from DECISIONS §64.16.
Those bytes must bind the policy and manifest root, collection, real Treasury 26460
AccountId, Zcash network/asset, `0.01 / 0.005 / 0.0133 ZEC` entry/claim/open-mint
prices, minimum confirmation rule, explicit deposit and claim close heights,
and the 90/10 floor/Treasury split. Ticket ids must bind that digest. A claim
memo continues to reference its ticket id, which then transitively commits the
same terms and recipient. Refuse startup if configured values disagree with the
published bytes; never substitute a service default or a wall-clock timeout.

`cave-zcash::claim_memo` is a codec, not a completed claim rail. A complete claim
service must admit one sufficiently confirmed claim payment per eligible ticket,
reject wrong amount/purpose/reference/recipient and duplicate outpoints, credit
the exact ZEC.zy to Treasury once, wait for the credited epoch to be anchor-final,
fund 90% through `FundCollection`, retain 10% in Treasury, then submit the
receipt-bound serial mint. Persist and replay-check every observed VM receipt in
the same way as entry settlement.

## 5j. Cave fungible launchpad and atomic ZynZap graduation (implemented, staged; 14 Sep 2026)

The consensus core is executable: `LaunchCurve`, `BuyCurve` and `SellCurve`
live in `swapvm`; reaching 660M sold graduates atomically without a separate
advance transaction. `OP_CURVES`/`zyn_curves` expose committed launch state,
`OP_CURVE_QUOTE`/`zyn_curveQuote` return exact chain quotes, and the Rust client
submits all three signed intents. Cave is the product surface and `swapvm` is
the single conserved ledger throughout Curve and AMM.

This is **not permissionless-write-enabled yet**. The standalone Jade frontend,
wallet provider writes and replica explorer evidence are built, and the static
artifact is live at `https://cave-jade.pages.dev`. Receipt-derived OHLCV, the
state upgrade rehearsal and a funded two-wallet testnet run remain release
gates. `launch.cavebro.win` has been attached to the Pages project; the operator
owns its final DNS/hostname activation.

### Product boundary and checked-in surface

Build the fungible product separately from Cave's pixel-art NFT application:

```sh
cd apps/cave-market
npm test
npm run build:launchpad
# standalone output: apps/cave-market/dist
```

The production Pages surface is `https://cave-jade.pages.dev`; the target custom
host is `launch.cavebro.win`.
The root route is the Cave Jade coin gallery, `/launch` is the “Pick a Stone”
workbench, and `/coin/<address>` is the per-asset chart/trade page. There is no
marketing landing page. Dark jade with restrained metallic/mineral texture is
the working brand; do not inherit the NFT app's pixel theme. Rough/window/reveal
wordplay is provisional and may change without changing protocol state.

The checked-in frontend mirrors the fixed 1B/eight-decimal supply, exact Unicode
symbol, 0.5–3% fee and optional 5% dev buy. It reads committed launch rows and
exact quotes only through Nap Wallet's injected provider; launch, buy, sell and
graduated swaps open extension-owned approvals. The deterministic curve chart is
state, not price history: historical OHLCV stays absent until A35 indexes real
receipts. Treat a successful local build, a Pages deployment and custom-domain
activation as three separate checks; only the first two are complete here.

### Existing ZynZap baseline and reuse boundary

The repository already has constant-product pool math, canonical pair creation,
LP accounting, swap quotes, single-hop batch clearing, replica replay and public
pool reads. It also has an internal `CreateToken` intent that creates a fixed
supply token and ZEC.zy pool in one transition. That intent is **not** the Cave
launchpad: it skips the reversible curve, accepts creator-selected launch
inputs and returns LP shares that can be removed.

Reuse the lower-level pool construction and LP-share math only. Do not route a
Cave launch through `CreateToken`, expose it under a new HTTP name or let a
creator call it to bypass the protocol template. Current migration status:

1. **Done in code:** A25's 32-byte asset/pool/collection identifier migration uses the
   frozen `ADDRESS_SCOPE_V1 = b"zynzap\x00\x02"`, independent of the execution
   VM version.
2. **Done in current code:** the superseded ZYN emission, halving, burn and
   bridge-fee launch controls are disabled/removed from the current product
   path; historical state and wire reads remain only for audit and migration.
3. **Partially complete:** launch records, fee policy and derived vault addresses
   are committed. Move pool reserves from inline fields to `vault(pool)` balances
   and finish the reviewed `STATE_VERSION`/VM-version rollout. Upgrade replicas and signers before or alongside the
   sequencer under §59.5.

ZYN issuance is not a launchpad prerequisite. Cave can accrue into the keyless
pot, but cannot mint ZYN, promise claims against it or decide how it is eventually
matched. That later action belongs to the separate ZYN issuance design.

Cave NFT secondary trading now uses `50% future-ZYN-POL / 40% collection floor /
10% creator` for both negotiated and resting offers. The receipt names all three
destinations and atomic remainder goes to future POL. Nap Mole and Cave's 4,444
collection use Treasury 26460 as creator once its real threshold AccountId is
provisioned; the name is fixed, the 32-byte account is not yet pinned.

### State and accounting contract

For a launch with curve quantity `q`, the working v1 curve family is:

```text
marginal price:       p(q) = p0 + s*q
refundable reserve:  R(q) = p0*q + s*q^2/2
buy Δ:               R(q + Δ) - R(q), rounded ZEC.zy up
sell Δ:              R(q) - R(q - Δ), rounded ZEC.zy down
```

The v1 template fixes all economic curve values:

```text
fixed supply S:           1,000,000,000 tokens (8 decimals)
curve inventory Qg:      660,000,000 tokens
graduation inventory:    340,000,000 tokens
starting price p0:       0.009 / S = 0.000000000009 ZEC.zy/token
graduation price Pg:     100 / S = 0.0000001 ZEC.zy/token
slope s:                 (Pg - p0) / Qg
                         ≈ 1.5150151515151515e-16 ZEC.zy/token²
reserve at graduation:   R(Qg) = 33.00297 ZEC.zy before fees
```

Encode `p0` and `Pg` as exact fixed-point constants and evaluate the interval
integral in token atomic units with the division last; do not store the
unrepresentable fixed-point slope. Round buy principal up and sell principal
down at the ZEC.zy WAD boundary. The first atomic token therefore costs one raw
ZEC.zy unit of principal rather than rounding to a free buy. The displayed
decimals are explanatory. Graduation occurs at
`q = Qg`, when `p(q) * S = 100 ZEC.zy`. There is no age, expiry, wallet-count or
NFT gate. Any unsold graduation inventory left after exact pool seeding becomes
permanently unreachable; it is not a later allocation.

A buy credits tokens only after the full ZEC.zy principal and fee debit succeeds.
A sell returns tokens to the keyless curve inventory only in the same transition
that pays its ZEC.zy principal. Per-trade principal is the exact interval rounded
toward the reserve, so repeated trades may leave atomic rounding surplus; fee
pots are accounted separately and never count as graduation progress.

The creator chooses a whole-basis-point trading fee from `50–300 bps`; omitted
means `100 bps` (`1%`). Commit it at creation and carry it unchanged into the
graduated pool. It cannot be edited after a buyer has seen the launch.

For fungible curve fee `F`:

```text
50% F  → keyless future-ZYN-POL ZEC.zy pot
40% F  → this launch's keyless graduation-liquidity pot
10% F  → this launch's creator escrow, released only at graduation
```

The fee is always ZEC.zy: input on a buy, output on a sell. Cave NFT secondary
trading uses the same percentages, but its 40% goes to the collection redemption
floor and its 10% goes directly to the collection creator. Both the 333 Nap Mole
and 4,444 Cave collections use the protocol account named `Treasury 26460` as
creator and fee recipient. Pin its real threshold `AccountId` before deriving
or minting either collection.

For integer fee `F`, compute creator escrow as `floor(F*10/100)` and graduation
pot as `floor(F*40/100)`; future ZYN POL receives the remainder. This makes rounding
conservative and preserves every atomic ZEC.zy unit.

### Permissionless creation fee and dev buy

`LaunchCurve` is available to every valid wallet. NFT ownership is never
read as an authorization condition. Debit the non-refundable launch fee before
creating state:

```text
0.009 ZEC.zy  → launch's locked pair-seed balance
0.001 ZEC.zy  → protocol treasury
```

The current VM holds the seed in the launch's shared derived vault and commits
its purpose by rule. The separate `pair-seed` account is the A31 storage target.

The pair seed is not curve principal and cannot pay a pre-graduation sell. On
graduation it joins the pool's ZEC side; the opening token reserve still has to
satisfy `T = Z/P`. Call it an opening-liquidity seed in APIs and approval text,
not a floor guarantee. It remains committed to the launch for as long as the
curve remains below graduation.

The creation intent may include one exact `dev_buy_tokens` amount plus
`dev_max_zec`. Execute it atomically after the launch fee at the same initial
curve price, chosen fee and rounding as a public buy. Refuse a request above
50,000,000 tokens (5% of supply). The receipt names paid principal, fee and
tokens; no other creator allocation exists.

The launch record committed to state must contain at least:

- creator, native asset id/address, symbol, unit and content commitment;
- fixed one-billion supply and the curve/graduation inventory allocation;
- immutable trading fee in basis points and optional atomic dev-buy result;
- curve-template version, `q`, refundable reserve and all three fee pots;
- creation height/epoch and graduation price/quantity;
- launch-fee treasury payment and pair-seed balance;
- phase (`Curve`, `AMM`);
- canonical ZEC.zy pool id once graduated.

The native-token invariant must allow `genesis_pool = None` only when a matching
committed launch record is in `Curve`. Entering `AMM` and setting
the canonical pool id are one state transition; an unlaunched native token may
not use this exception.

### Implemented VM and API surface

Keep the write surface small and intent-shaped:

- `LaunchCurve`: commits identity, fixed allocations, template version,
  immutable `fee_bps`; debits the `0.01 ZEC.zy` launch fee, creates
  the token and current derived launch vault, optionally performs the capped dev buy,
  and enters `Curve`;
- `BuyCurve`: names exact token output and maximum ZEC.zy input;
- `SellCurve`: names exact token input and minimum ZEC.zy output;

There is no `AdvanceCurve`: the buy that reaches 660M performs graduation on
the same scratch state, preventing a qualified curve from being stranded or a
separate advancement call from racing the terminal buy.

Receipts must expose launch id, phase, `q`, principal, fee and its three
destinations, outputs and post-trade reserve. Graduation additionally exposes
the pool id, terminal price, both deposited reserves and locked LP amount.
Public reads list launches and return the same fields from replica state. Cave's
HTTP routes should only translate those typed reads and signed intents; they do
not maintain an authoritative launch database.

**Current receipt/read status (14 Sep):** `CurveCreated`, `CurveBought`,
`CurveSold` and `CurveGraduated` are typed, encoded and replayable. The public
curve list exposes identity, fee, sold quantity, creator/graduation accruals,
graduation reserves/overflow/locked LP, marginal price and pool status; the
quote RPC exposes principal, total fee, settlement, post-trade quantity/price
and whether the trade graduates. The buy/sell receipts do **not yet** name the
three fee destinations, and the shared launch-vault layout means the public read
cannot expose a separately held refundable-reserve balance. Add those fields
with A31/A34 rather than deriving them from illustrative frontend data.

Current reproducible evidence:

- `cargo test -p swapvm --test cave_launch` — 10/10 dedicated curve and
  graduated-pool fee tests pass.
- `cargo test -p zynzapd cave_gallery_and_quote_are_public_and_exact` — the
  public gallery/quote RPC test graduates a curve and verifies the sell quote's
  ZEC.zy fee asset plus total, pool, protocol, creator and POL shares.
- `cargo test -p swapvm collection_fee` and `cargo test -p zynzapd --test demo`
  — the shared NFT fee path and both socket-demo flows pass under the current
  50/40/10 floor math.
- `cd apps/cave-market && npm test && npm run build:launchpad` — 9/9 frontend
  tests pass and the standalone build emits only `jade.html`/CSS/JS equivalents
  as the SPA artifact.
- `cargo test --workspace` — the current v4 VM identity, receipt layout, swap/NFT
  accounting, daemon, browser fixture, custody and settlement suites pass
  together.

The provider and HTTP surface must also expose typed, explicitly oriented
`zec_per_token` quotes. Do not export an unnamed reserve ratio: canonical pool
ordering commonly makes the existing spot helper return token per ZEC, the
inverse of the curve and chart price.

### Graduated-pool compatibility contract

Add a committed per-pool fee policy rather than special-casing Cave in the web
server. Existing pools keep their input-asset fee rule. A graduated native pool
uses `ZecLegSplit`, including the token's immutable 10% creator share:

```text
ZEC.zy -> token:
  gross ZEC input X; total fee F
  invariant input = X - F
  pool ZEC increase = X - F + pool_share(F)
  future-ZYN-POL credit = pol_share(F)
  creator credit = creator_share(F)

token -> ZEC.zy:
  full token input prices gross ZEC output G
  user ZEC output = G - F
  pool retains pool_share(F)
  future-ZYN-POL credit = pol_share(F)
  creator credit = creator_share(F)
```

Use `creator_share = floor(10% F)`, `pool_share = floor(45% F)` and
`pol_share = F - creator_share - pool_share`; the odd atomic remainder therefore
goes to the future-ZYN-POL pot. Quotes, hop results and receipts must name `fee_asset = ZEC.zy`,
total fee and all three shares. Apply the same accounting inside the single-hop
batch-clearing path; an immediate quote and a sealed order may not implement
different fee policies.

**Compatibility status (14 Sep):** A45 is resolved. The graduated-pool marker,
two-way `45/45/10` routing and explicit ZEC.zy fee asset are implemented. A sell
prices the full token input to gross ZEC.zy, deducts its fee from that output and
pays the trader net ZEC.zy. Exact-in, exact-out and sealed-batch tests reconcile
the pool retention, creator credit and POL credit and prove no launched-token
fee reaches an external account. Hop receipts expose the fee asset, total and
all three shares. V3 introduced this consensus/receipt change; current v4 also
pins the derived future-ZYN-POL destination and capability-gates legacy
`CreateToken`. The separately frozen address scope keeps existing asset and
pool addresses stable.

The graduated pool has `reference = None`, uses the canonical token:ZEC.zy pair,
and sends every minted LP share to a derived keyless lock that cannot submit
`RemoveLiquidity`. Token-to-token routing may later use two canonical hops through
ZEC.zy, but launch v1 only requires its direct token:ZEC.zy market.

Pre-graduation curve trades use the integral, not the AMM. Execute exact-in,
minimum-output intents sequentially in committed intent order with no partial
fills; the containing epoch makes the receipt final. A displayed quote is
provisional until that execution. After graduation, use ZynZap's existing
swap/order and batch-clearing path. Approval text and the market page must name
this execution-model change.

### Graduation transaction

Graduation is one VM transition, in this order on scratch state:

1. Re-check phase `Curve`, `q = 660,000,000` and marginal price
   `Pg = 0.0000001 ZEC.zy`; do not inspect age, NFT ownership or holder count.
2. Prepare the canonical graduation entirely on scratch state; the qualifying
   buy still emits its terminal quote and graduation receipt, and the final
   phase marker is written only after pool construction succeeds.
3. Compute candidate `Zc` from refundable reserve, 40% graduation pot and the
   `0.009 ZEC.zy` pair seed.
4. Set `Zp = min(Zc, 34 ZEC.zy)`, compute `T = Zp/Pg` under the versioned atomic
   rounding rule, and route overflow `Zc - Zp` to future ZYN POL. Refuse if `T`
   exceeds the fixed 340,000,000-token graduation inventory.
5. Create the canonical token:ZEC.zy pool, move `Zp` and `T`, make any unused
   graduation tokens unreachable, and record every LP share in the pool's
   permanent consensus lock. A31 moves that lock into its derived account.
6. Release the escrowed 10% fee share to the creator, record the terminal curve
   values, and set phase to `AMM`.
7. Commit all of it or none of it. A failure leaves the curve at its previous
   state and releases no creator fee.

The AMM's opening spot must equal the terminal curve price within the exact
fixed-point rule. “Close enough” is a failed graduation because the difference
is an immediate arbitrage paid by launch participants.

The public price is always `ZEC.zy per token`. Reserve ordering is internal. A
graduation test must compute the terminal curve price and the opening pool price
through independent code paths, orient both the same way, and compare their exact
fixed-point values.

### Persistent curve

A curve that has not qualified for graduation remains in `Curve`. Buys and sells
continue under the same formula and fee policy without a deadline. An inactive
launch may leave the default gallery, but its asset page, quotes and transaction
path remain reachable by asset address. The creator and operator can never debit
the refundable reserve, pair seed, graduation pot or creator escrow directly.

### Verification gate before an alpha launch

Status as of 14 Sep 2026; `✅` means direct automated evidence exists, `🔨`
means the behavior exists with incomplete adversarial/matrix coverage, and `⏳`
means the release evidence is still absent.

- 🔨 Exact interval math, monotonic endpoints, buy-up/sell-down rounding and a
  1,000-step deterministic reversible sequence pass. Broader randomized/property
  fuzzing remains. Reconcile the ideal integral plus curve-favouring atomic
  rounding surplus; do not require literal `reserve == R(q)` after every trade.
- ✅ `p0 = 0.009/1B`, `Pg = 100/1B`, `q = 660M`, `R(q) = 33.00297 ZEC.zy`,
  340M graduation inventory and exact fixed-point boundary vectors are pinned.
- ✅ Native-token supply conservation, live-curve inventory, state codec
  roundtrip, canonical pool installation, exact opening price and all-LP locking
  pass the VM invariants and dedicated integration suite.
- ⏳ Force failure at every graduation debit and prove no partial pool, released
  creator fee or phase change survives; scratch-state application provides the
  atomic mechanism, but the failure matrix is not yet tested.
- ⏳ Add explicit negative-authority tests proving creator/operator accounts
  cannot debit refundable reserve, launch-retained balances or POL.
- ✅ Curve fee routing and NFT `50/40/10` are implemented. Graduated ZEC.zy →
  token and token → ZEC.zy both use the `45/45/10` ZEC-leg rule in immediate,
  exact-output and sealed-batch execution; hop receipts reconcile every share.
  Curve buy/sell receipts still omit the three individual fee destinations.
- ⏳ Exercise corrected `ZecLegSplit` in both directions through immediate and
  single-hop batch settlement; reconcile user output, pool reserve, creator and
  POL for every atomic unit.
- 🔨 Creation atomically routes `0.009/0.001 ZEC.zy` and executes a paid,
  maximum-5% dev buy. Add focused rejection tests for unfunded creation, 5%+1,
  and slippage even though the checks exist in code.
- 🔨 The immutable `50–300 bps` bounds and persistence are enforced. Add direct
  lower/upper/out-of-range and post-graduation tests.
- 🔨 Codec restart after 1,000 curve trades passes and no expiry code exists.
  Add a long epoch jump plus replica replay case to prove age is irrelevant.
- 🔨 Graduation caps ZEC at 34, preserves exact opening price and routes cap or
  token-rounding residual to POL. A normal valid v1 launch remains below 34 even
  at 300 bps, so exercise the defensive above-cap branch with a controlled
  synthetic state and assert the POL balance explicitly.
- 🔨 A graduated curve rejects further curve quotes and the state invariant
  requires a live curve for a pool-less native token. Add explicit pre-graduation
  AMM and post-graduation sell rejection tests.
- 🔨 No account receives graduation LP and `locked == lp_supply`; generic
  `RemoveLiquidity` cannot present those shares. Add a direct graduated-pool
  removal attempt and migrate the lock to its derived account under A31.
- ✅ Public curve rows and Cave/zynscan name `ZEC.zy per token`; before
  graduation this is the curve marginal price and afterward it is the live pool
  ratio oriented by asset identity, independent of canonical pair ordering.
- ✅ Cave and zynscan expose committed reserve, accumulated creator/graduation
  fees, overflow, immutable opening liquidity and locked LP; Cave has no
  illustrative fallback and all writes use extension-owned wallet approval.
- ⏳ A31 still migrates the presently committed accounting fields/keyless pots
  into separately derived physical accounts so an explorer can reconcile each
  address directly rather than only the state fields and root.

### Discovery and chart projection

Coin galleries and chart pages need a projection service, not additional
consensus balances. Index accepted curve receipts until graduation, then swap
receipts for the recorded canonical pool id, into OHLCV candles. Key every event
by launch id, asset id, phase and pool id so the boundary cannot double-count a
graduation epoch. Rebuild the index from published receipts/journals and compare
its last price with a replica quote during rehearsal. If the projection is down,
show state and disable the chart; never disable exits or settlement.

### Product rollout

1. Internal two-wallet rehearsal with tiny testnet value.
2. Public read-only Cave pages against rehearsed state, including independently
   checkable curve, fee and graduation fields.
3. Permissionless public alpha with at most two accepted launches per creator in
   any rolling 60 chain epochs; no global quota, active-curve cap, value cap,
   NFT gate or manual-review gate.
4. Raise infrastructure capacity only after one long-lived inactive curve and
   one atomic graduation have been exercised end to end, including restart and
   replica replay. Do not introduce protocol buy/value/active-curve caps as a
   rollout substitute.
5. Open broader traffic only after reconciliation remains clean under concurrent
   launches and both directions of graduated trading.

Nap Mole ownership is outside every launchpad gate and fee calculation. Its only
planned utility here is indirect: the separate credit system may multiply credits
for an owner. Test that launchpad receipts and state roots do not depend on that
multiplier.

The A25 identifier prerequisite is now in the core: asset, pool and collection
ids are derived 32-byte values under §66's frozen scope; state, wire, Merkle,
receipts, RPC, wallet/client and bridge paths use them, with pinned native,
bridged, pool, LP and vault vectors. A `LaunchCurve` intent names the owner/fee
recipient as creator and derives the asset from its validated, unmodified UTF-8
symbol. An existing derived asset is rejected rather than overwritten. Still
move pool reserve quantities through `vault(pool)` balances and roll the state
format verifier-first before deployment.

The frozen storage target derives separate `curve-inventory`,
`graduation-inventory`, `curve-reserve`, `pair-seed`, `graduation-fees` and
`creator-escrow` accounts per Cave asset, plus `lp-lock` from the graduated pool
and global `zyn-pol` from ZEC.zy. Current Cave, NFT and graduated-pool writes use
the pinned global address `f40435d1…7e61`; historical §30 compatibility code
still names legacy keyless `POT_POL = [0xF3;32]`. A31 must migrate any live
legacy balance and split the remaining launch-vault balances alongside the
pool-vault migration. Replicas and zynscan then reconcile each address against
the corresponding launch fields.

Public clients encode raw ids as §66 Bech32m addresses; lowercase hex is only a
developer fallback. Accept 1–32-byte valid UTF-8 symbols and 1–96-byte valid
UTF-8 display names under §66.11 and §69.29. Do not normalize them. Duplicate
and visually confusable symbols, including `ZYN`, are legal; show the short asset
address and grant the official badge only by exact membership in the pinned
official-address set. Reports may warn or hide a launch from default discovery,
but cannot freeze its curve, pool, sells or exits.

Enforce the launch rate from committed state: for a creator, reject a third
accepted `LaunchCurve` whose epoch lies within the previous 59 chain
epochs. Do not cap a single buy, aggregate curve reserve, number of live curves,
sells or exits. Property-test the boundary at epochs 59 and 60 and across restart
and replica replay.

Do not use token count as the rollout gate. Watch independently funded holders,
repeat traders, net ZEC.zy reserve, completed curve sells, graduated locked liquidity
and fee reconciliation. Those are the measurements that distinguish circulation
from generated noise.

## 5. Still missing

Ordered by what blocks what.

1. ~~**A `ChainView` against Zebra.**~~ Built — `zyn_custody::zebra`, speaking
   `getblockcount`, `getaddresstxids`, `getrawtransaction`. **Transparent
   only**, and it refuses mainnet at construction.
2. ~~**A daemon and an RPC.**~~ Built — `zynzapd`, §4b.
3. ~~**The watcher connected to the chain.**~~ Built — `zynzapd::bridge`, the
   only place an observation of another chain becomes a credit on this one.
   Progress is persisted *before* intents are submitted, so a crash loses a
   deposit rather than minting one.
4. ~~**Shielded deposits.**~~ Detection built — `zyn_custody::shielded`. Trial
   decryption with the vault's Orchard IVK, memo decoded, account recovered.
   Proven by an encrypt-then-decrypt round trip, and by a second vault's key
   failing to see the same note. `cargo run -p zyn-custody --example
   vault-address` prints the deposit address.
   **Wired into the daemon.** `ZYN_VAULT_SEED` selects the shielded path and
   the daemon prints the vault's deposit address at startup; setting both it
   and `ZYN_BRIDGE_ADDRESSES` is refused, because two scanners crediting one
   asset would double it.
   Verified against the live node: blocks 500,000–500,061 scanned, **26 of 26
   real transactions parsed, 0 undecodable**. `cargo run -p zyn-custody
   --example scan-report` reports that ratio, which is the only thing that
   tells a scanner finding nothing apart from a scanner reading nothing.
   **Not yet exercised against a real Orchard bundle** — testnet NU5 activates
   near height 1,842,000 and the node is still below it. The decryption path is
   proven synthetically; the first real shielded deposit will be its first real
   test.
5. **Orchard spend assembly.** Withdrawals, and now the largest gap. FROST
   solves *spend authorization*; a payout also needs note commitment tree
   witnesses, a proof, a bundle, and a binding signature derived from value
   commitments rather than threshold-signed.
6. **The viewing key the ceremony does not produce.** An Orchard full viewing
   key is `(ak, nk, rivk)` and the DKG produces only `ak`. `nk` and `rivk`
   grant viewing but never spending, so every signer and the watcher may hold
   them — but they must be generated once and distributed deliberately. A
   signer set that ran a DKG and stopped can authorise a spend and cannot find
   anything to spend.
7. **Solana.** One constant (`ORIGIN_SOLANA`). The `Watcher` is generic over
   `ChainView`, so its state machine transfers; custody does not — Solana is
   ed25519, a different FROST ciphersuite from RedPallas.
8. **Monitoring.** Vault balance against issued supply, sequencer liveness,
   signer availability, and the age of the oldest unsettled exit.

## Mainnet genesis — frozen at STATE_VERSION 12, 15 Sep 2026

Chain 26460 is re-run at v12 and **stopped**, service `disable`d.

| | |
|---|---|
| ZYN | `49eb9241e2a0b1163a1eed09bb91a575c93bbc6e76cd7afeb65263640fd169d5` — 21,000,000, **fixed**, no mint authority |
| ZYN reserve | `c13c1826…39b0` holds **all 21,000,000** |
| ZYN POL pot | `51aece7b…6ba6` holds 0 — it fills from ZEC.zy accrual at 0.0002 (§71.24) |
| Treasury | `b3e4c493bb79349270260f7d2e2882b4f1a736348935493331e18c95098cfe99` |
| NAPMOLE | `d3090c31…9261` — cap 333, 100 bps, **Closed**, 333/333 |
| CAVEZ | `024277b6…5f1e` — cap 4444, 100 bps, **Minting**, 0 |
| Base root | `29de5266179130872cfc14b4517f21b042acb7c6e850e423c813331a9b60f25d` |
| Market | not due — no opening date until 420 ZEC.zy has accrued |
| Zcash clock | last observed 3,484,298 before the stop; it resumes when the node does |
| Data | `/var/lib/zyn/mainnet-v12`, DA in `/var/lib/zyn/mainnet-v12-da` |
| Backup | `/var/backups/zyn/mainnet-v12-genesis-20260915-152842.tar.gz`, sha256 `ab2dd263…ad045`, copy on the Mac |

Every earlier state is **untouched**: `mainnet{,-da}` is v10, `mainnet-v11{,-da}`
is v11. Each reset repointed the env file rather than clearing directories, so
rolling back is a one-line edit plus the matching binary — `zynzapd.v10`,
`zynzapd.v11` — with `zynzapd-mainnet.env.v10` and `.v11` holding the old paths.

**The Zcash height clock must be running** for the market schedule to advance
(§71.30). The node feeds it once a minute from its own Zebra; if that connection
fails it says so at startup, and the grace period simply cannot begin. Silence
in the journal means it is working.

Still no Zcash anchor: the vault has never been paid, so the anchor pass fails
with "waiting for a spendable Ironwood note". Expected, not a fault.

### How this was re-run (the v10 → v11 reset)

### Resetting mainnet to v11

1. **Deploy.** Cross-compiled on the Mac — the box is x86_64 and has no Rust
   toolchain. Check `uname -m` on the target first (§75.5).

   ```sh
   cargo zigbuild --release --target x86_64-unknown-linux-gnu -p zynzapd --bin zynzapd
   scp target/x86_64-unknown-linux-gnu/release/zynzapd root@95.111.240.24:/usr/local/bin/zynzapd
   ```

   The service is already stopped; a running executable cannot be overwritten.

2. **Clear the v10 state.** Archive it first, then delete
   `/var/lib/zyn/mainnet` and `/var/lib/zyn/mainnet-da`. The node will not load
   a v10 file and will refuse to start against one.

3. **Start.** `systemctl enable --now zynzapd-mainnet`. The node issues ZYN at
   its own genesis before anything else touches state; confirm with
   `journalctl -u zynzapd-mainnet -n 20 | grep "ZYN issued"`.

4. **Run the 338 collection intents** from the Mac, through two custodians and
   the SSH tunnel — the procedure under *Re-running or extending genesis*
   below, unchanged.

5. **Verify both halves against the chain**, not against what was intended:

   ```sh
   cargo run --example verify_zyn       # 21M fixed, all of it in the two pots
   cargo run --example verify_genesis   # 333 items: address, binding, content
   ```

Then stop the node and back up, as before.

## Mainnet genesis — frozen 15 Sep 2026 (STATE_VERSION 10, superseded)

Chain 26460 holds its genesis objects and is **stopped**. The service is also
`disable`d, so a reboot does not restart it.

| | |
|---|---|
| Treasury | `b3e4c493bb79349270260f7d2e2882b4f1a736348935493331e18c95098cfe99` (FROST Ed25519, 2-of-3) |
| NAPMOLE | `d3090c31228b5614ddd13e7ff29bb251e211cfbfe1057a279be2a67460619261` — cap 333, fee 100 bps, phase **Closed**, 333/333 minted |
| CAVEZ | `024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e` — cap 4444, fee 100 bps, phase **Minting**, 0 minted |
| Sealed | epoch 0, DA bundle published for endorsement (5 files) |
| Backup | `/var/backups/zyn/mainnet-genesis-20260915-010516.tar.gz`, sha256 `98fc482b…8412`, copy on the Mac at `~/.zyn/backup/mainnet-genesis/` |

Nothing is anchored to Zcash yet: the vault holds no ZEC, so the anchor pass
fails with "waiting for a spendable Ironwood note". That is the expected state,
not a fault — anchoring begins once the vault is funded.

Treasury shares (one each, never two in one place):

| Holder | Path | Identifier |
|---|---|---|
| Hetzner (`root@168.119.53.39`) | `/var/zyn/treasury` | `01…` — moved from the Mac 18 Sep, §111 |
| Air (`zhang@192.168.1.27`) | `~/.zyn/treasury` | `02…` |
| Windows box | `/var/zyn/treasury` (WSL) | `03…` |

### Re-running or extending genesis

`zyn-genesis` resumes from what is already on chain, so a re-run is safe: it
skips every intent already applied and submits only the remainder. `plan`
contacts nothing and signs nothing.

```sh
# custodians first — two of the three, each on the box holding its share
ZYN_CUSTODY_TREASURY_SHARE=$HOME/.zyn/treasury ZYN_CUSTODY_LISTEN=127.0.0.1:8211 zyn-custodian &
ssh zhang@192.168.1.27 'ZYN_CUSTODY_TREASURY_SHARE=$HOME/.zyn/treasury ZYN_CUSTODY_LISTEN=0.0.0.0:8211 ~/zyn-custodian &'

ssh -f -N -L 18100:127.0.0.1:8099 root@95.111.240.24   # the node RPC is not public

export ZYN_GENESIS_CHAIN=26460 ZYN_GENESIS_NODE=127.0.0.1:18100        ZYN_GENESIS_CUSTODIANS=127.0.0.1:8211,192.168.1.27:8211 ZYN_GENESIS_THRESHOLD=2        ZYN_GENESIS_TREASURY=$HOME/.zyn/treasury        ZYN_GENESIS_MANIFEST=apps/nap/branding/nap-mole-collection-333/out/content.json
zyn-genesis plan && zyn-genesis run
cargo run --example verify_genesis    # address, binding and content, item by item
```

Stop the custodians afterwards. They hold shares and should not sit listening.

**ZYN is issued by the node, not by `zyn-genesis`.** It is the chain's native
asset, and native addresses are operator-only so no account can forge one
(§71.19). The node issues it once at startup if absent: 21,000,000, fixed
supply, no mint authority, split 2,100,000 to `ZYN_POL_ZYN` and 18,900,000 to
`ZYN_RESERVE`, both keyless. On chain 26460 it lands on the pinned
`ZYN_MAINNET` = `49eb9241…69d5`, which `verify_zyn` asserts.

**Serials are 0-based.** The VM requires `serial < cap`, so the 333 occupy
serials 0..=332 while collectors count from one: `MOLE001` is serial 0, and
`MOLE333` is serial 332. `zyn-genesis` binds the two in one place so the
off-by-one cannot spread.

## Cave whitelist, end to end (15 Sep 2026)

### Applying — what a visitor sees

`cavebro.win/whitelist`. Three steps (follow, repost and comment, paste the
link), then handle, link and an optional note. The link is required because it
is the only thing a reviewer can open and check.

It collects **nothing else** — no email, no wallet address, no Zcash address.
An invitation binds an account on first use (§74.13), so asking for one here
would gather exactly what that design avoids, before anyone has decided to buy.

### Spam, in layers

Aimed at volume rather than identity, because the last word is a person reading
the application. In rising order of what they cost an attacker:

| | |
|---|---|
| honeypot field | free; catches form-fillers that read the DOM |
| dwell check | a person takes seconds, a script takes none |
| rate limit | six per address per hour |
| handle dedupe | one handle applies once |
| post dedupe | one post backs one handle |
| proof of work | the only one whose cost scales with volume |

The proof is tied to the applicant's own handle and a ten-minute window, so it
needs no secret, no round trip and no third party: work done for one handle is
worth nothing for another, and work done an hour ago is worth nothing now. It
runs when the handle field loses focus, so it finishes while the rest of the
form is being filled. ~18 bits, roughly 900 ms on a desktop.

There is deliberately **no CAPTCHA**. If volume ever outgrows the above,
Cloudflare Turnstile is a dashboard key and a script tag, not a redesign.

### Reviewing

```sh
cd apps/cave-collection-z
npm run review     # print the queue: handle, date, the link to open, the note
npm run export     # timestamped JSON + a flat handle/link list into register/
```

Or `/list` in Telegram, which the bot answers from the same endpoint.

**Run `export` before and after a review session.** KV is durable, but it is one
company's copy of a list you are going to act on, and the same account serves
the site and the blobs. The Telegram messages are a second copy already,
arriving one at a time; the export is the one you can grep. `register/` is
gitignored — it holds other people's handles.

### Where it all lives

| | |
|---|---|
| page | `apps/cave-collection-z/whitelist.html`, served at `/whitelist` |
| intake | `functions/api/whitelist.js` |
| listing | `functions/api/whitelist/list.js`, bearer-token gated |
| store | KV namespace `CAVE_WHITELIST` = `8181407749be40ff89d7691132723ae7` |
| art | `/blobs/<hash>`, staged by `scripts/build.mjs`, immutable and CORS-open |

Secrets on the Pages project: `TG_TOKEN`, `TG_CHAT` (the same
`@Duchinski_bot` that monitors the chain) and `WHITELIST_ADMIN_TOKEN`. The bot
holds the listing token in `/etc/zyn/zyn-tgbot.env` as `ZYN_WHITELIST_TOKEN`.

**Pages secrets only bind on deploy.** Setting one and not redeploying leaves
the old version running without it, which reads as "not configured" rather than
as an error.

### Invitations

```sh
ZYN_WHITELIST=~/.zyn/cave-whitelist.tsv zyn-whitelist issue 50 2   # 50 codes, 2 mints each
ZYN_WHITELIST=… zyn-whitelist open|close|status
```

Codes print once and the file keeps only fingerprints. A registry is **closed
until opened**, so one that is loaded and never opened cannot sell anything by
accident, and closing survives a restart (§74.14).

### Claiming a Nap Mole

`zyn-claim` is the 333's gift path, not the 4444's sale. It needs two custodians
reachable while it serves, because a transfer is signed at claim time and the
signature commits to the recipient's account — which nobody knows until they
say so. Bring them up for the event and take them down after.

```sh
ZYN_CLAIM_BOOK=… zyn-claim issue 1 333     # codes, printed once
ZYN_CLAIM_BOOK=… ZYN_CLAIM_COLLECTION=d3090c31…9261 \
  ZYN_CLAIM_CUSTODIANS=127.0.0.1:8211,192.168.1.27:8211 \
  ZYN_CLAIM_TREASURY=~/.zyn/treasury zyn-claim serve
```

## Recovering from a Zcash testnet reorg (15 Sep 2026)

Testnet reorgs go far deeper than any confirmation count. On 15 Sep one reached
at least **1,240 blocks** and took three settled anchors — epochs 10477, 10487
and 10495 — off the chain. The sequencer noticed, refused to extend a lineage no
verifier could follow, and stopped. That is correct: a stalled chain is
recoverable, an unverifiable one needs archaeology.

**Diagnosing it.** The alert says an anchor "is no longer on Zcash", which is
also what an unreachable node looks like. Tell them apart before acting:

```sh
# Is the node reachable and at tip? (testnet Zebra listens on 18232, not 8232)
curl -s -X POST -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getblockchaininfo","params":[]}' \
  http://127.0.0.1:18232
# Can it find *any* historical transaction? If not, the index is the problem.
# If it can, and the anchor's block now holds only a coinbase, it is a real reorg.
```

Count **distinct** anchors, not log lines — the same few are re-logged every
pass, and 1,638 lines was three anchors over six hours.

**Repairing it.** Re-anchoring is manual by design: name the epochs and restart.

```sh
# /etc/zyn/zynzapd.env
ZYN_REANCHOR_EPOCHS=10477,10487,10495
```

They queue at startup and go out once the vault has a spendable note.

**Budget most of a day, and know the chain is down for it.** The reorg
invalidates the note trees, so the restart triggers a full resync — and that
resync runs in *startup*, before the main loop. Measured on 15 Sep: about 1,800
blocks an hour, roughly 18,000 blocks to cover, near seven hours end to end.

While it runs the node does not serve. The RPC socket is bound early, so `ss`
shows a listener and nothing looks wrong, but nothing is accepting: `Recv-Q`
climbs to the backlog size and clients time out. Signers log `cannot reach the
node … Operation timed out` and endorsements stop. Check `Recv-Q`, not the
presence of a listener.

So the trade a restart makes is worth stating before taking it: anchoring was
already stopped, but *everything else was working*. Restarting exchanges a
stalled anchor for a chain that answers nobody until the resync finishes. On
testnet that is acceptable. On mainnet, schedule it.

Remove the variable once the re-anchors confirm.

**A constraint worth knowing:** the §81 signer fix cannot reach testnet. It
lives in a tree at `STATE_VERSION` 12, and chain 11 runs v10, so a v12
`zyn-replica` rejects the state outright with "verified state does not match its
root: Decode" and the signers exit. The testnet signers stay on the pre-§81
binary until chain 11 is reset. Keep the previous binary beside any replacement.

## Ownership privacy: the three settings and the order they go in (17 Sep 2026)

Three switches, all off by default, all safe to deploy before they are turned
on. Full reasoning in DECISIONS §93, §95, §97; the public summary is
`docs/NFT-PUBLIC-PRIVACY.md`.

### 1. Seal the journal and base state — `ZYN_DA_KEY`

Those two files are the only ones in a bundle that carry ownership, and they
carry it because they are what a verifier replays. Sealed, a mirror keeps bytes
it cannot read.

```bash
openssl rand -hex 32      # the key. A MIRROR MUST NEVER BE GIVEN IT.
```

Set `ZYN_DA_KEY=<hex>` on **every verifier first**, and on the **sequencer
last**. A sequencer sealing before verifiers have the key halts them with
`… is sealed and this verifier has no ZYN_DA_KEY`, which is a clear message but
a stopped chain.

- verifiers: the Mac's two signer plists, the Air's plist, `/etc/zyn/zyn-signer.env`
- sequencer: `/etc/zyn/zynzapd.env`

Deploying the binaries with no key set changes nothing — the publisher emits
byte-for-byte what it always did, and a test asserts exactly that. So binaries
can go everywhere first, and the switch flipped afterwards.

Privacy begins at the cutoff and is never retroactive (§70.8): everything on the
mirrors today is plaintext and stays plaintext. Do not promise otherwise.

### 2. Narrow the vault viewing key — `ZYN_BACKING_SIGNERS`

Anyone holding the vault FVK can map a Zcash payment to a Zyn account (§97.1).
That derivability is deliberate — it is what makes a credit checkable instead of
the operator's word — so the move is to shrink who holds the key, not to change
what it derives.

**The order matters, and getting it wrong silently removes the guarantee.** A
replica without the FVK skips the backing check and endorses anyway, so if the
key is removed before the backing set is named, a threshold can be met entirely
by signers who cannot check a credit at all, and a fabricated deposit settles
unexamined.

```bash
# 1. FIRST: name who verifies backing, on the sequencer.
ZYN_BACKING_SIGNERS=<hex pubkey>,<hex pubkey>
ZYN_BACKING_THRESHOLD=2
systemctl restart zynzapd
# Look for: "zynzapd: BACKING — 2 of 2 endorsers must be verifiers …"

# 2. ONLY THEN: remove ZYN_VAULT_FVK from replicas that are not in that set.
```

A backing verifier logs `zyn-replica: BACKING VERIFIER — each credit is checked
against this node's own Zcash…` at startup. One that does not print that line is
not checking, whatever else it says.

### 3. Receive items into per-collection accounts

No setting: the wallet exposes `GET /api/receiving/collection?collection=<hex>`
and the mint takes that account as the recipient. `GET /api/receiving/next` and
`GET /api/receiving?index=N` are the indexed form, for the rare case that wants
per-item unlinkability and will pay a read per item for it.

Item operations then have to be signed by the account holding the item, so
`POST /api/offer`, `/api/take` and `/api/cancel-offer` take an optional
`collection`. Absent, they sign with the identity — which is correct for
anything minted before this.

### Checking it worked

```bash
# sealed at rest, and the journal is not readable
ssh root@168.119.53.39 "head -c 8 /var/zyn/mirror/chain-11/<epoch>/intents/epoch-<n>.intents"
# expect: ZYNSEAL<0x01>

# a verifier still verifies
journalctl -u zyn-signer -n 5    # "verified N anchor(s); through epoch …"
```

If a verifier halts on a sealed file, it has the wrong key or no key — those
report differently on purpose, because the fixes differ.
