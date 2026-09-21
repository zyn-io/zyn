# Nap Zyn key derivation v1

Status: normative for new mnemonic-backed Nap wallets.

## Contract

Inputs are the 64-byte BIP-39 seed (including the optional BIP-39 passphrase)
and the ZIP-32/BIP-44 account index as an unsigned 32-bit integer below `2^31`.
Derive the 32-byte Ed25519 seed with HKDF-SHA256:

```text
IKM  = BIP39-Seed(mnemonic, passphrase)       # exactly 64 bytes
salt = UTF8("nap.zyn.ed25519.v1")
info = UTF8("account") || U32_BE(account)
OKM  = HKDF-SHA256(IKM, salt, info, 32)
```

Use `OKM` as the RFC 8032 Ed25519 seed. The public key becomes the Zyn account
under the existing rule:

```text
account_id = SHA256(UTF8("zyn.account.v1") || 0x01 || U32_BE(32) || public_key)
```

The derivation is versioned by its salt and is deliberately **not** scoped by
Zcash network or Zyn chain. This matches Nap's historical single `zyn.key`
identity and lets one phrase restore one Zyn identity on browser and desktop.
It also correlates that public account across chains; users who need separation
must choose another account index. Signed reads/intents bind a chain id, so the
shared identity does not permit cross-chain replay.

Derive from BIP-39 seed bytes—not normalized display text, an Orchard child,
or a transparent child. Desktop source of truth is `Wallet::zyn_signing_key`;
browser source is `apps/nap-web/src/core.js`.

## Test vectors

Both vectors use this 24-word mnemonic:

```text
abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art
```

| Passphrase | Index | Ed25519 seed | Public key | Zyn account |
|---|---:|---|---|---|
| empty | 0 | `57d47cefdba062bb9669a7a64e9072e49d2b5bc66892952429240e4c91b16183` | `308ab8b209813f5912287682b50950d62782abc61507f0a80abafd0f7a33a7a6` | `b85db260ec3a7c0a22c19c1f3380bfc75599c0ea4eeeeda69177ab12f9da56ea` |
| `nap passphrase` | 7 | `f4e1b20f8a0cd2e19ae9d85ce3057cbb13863be3630e87972afce0b14c513c2e` | `276237e6804911ecd6d44c3d170ac67ff8dc8abf87cf423489a71dddf68857eb` | `4c976ef0d248340b910e246439c3139911f1752e6ba1d3c198f41071e4503604` |

The fixture suite additionally pins the complete epoch-42 signed-read payload,
ZynZap v2 VM id, transfer intent, Ed25519 signature, and submission frame in
both Rust and JavaScript.

## Receiving accounts (v1, added 17 September 2026)

Status: normative. A wallet that implements the identity above but not this
will restore the phrase and still not find items received into these accounts.

An item received at the holder's identity puts that identity in the journal:
`MintCollectionItem` names its recipient, and a reader who can decrypt the
journal has the owner map. Receiving accounts exist so what the journal names
is an account with no other history.

They are a **separate branch**, not another value of the account index. There
the index means *which wallet*, and `zyn_signing_key` refuses an index that
differs from the wallet's on purpose — two personas must not share an identity.
Receiving accounts are a different axis, many per wallet, so they take their
own salt. The two can never collide.

```text
salt = UTF8("nap.zyn.receive.v1")

per collection (preferred)
  info = UTF8("collection") || U32_BE(account) || collection_id[32]

per item (only where per-item unlinkability is worth a read per item)
  info = UTF8("receive")    || U32_BE(account) || U32_BE(index)

OKM = HKDF-SHA256(IKM = BIP39-Seed, salt, info, 32)
```

`OKM` is the RFC 8032 Ed25519 seed, and the account id follows the same
`zyn.account.v1` rule as the identity.

**Prefer the per-collection form.** It needs no counter, so there is nothing to
issue in order and nothing to orphan, and discovery is one derivation per
collection rather than a scan. The indexed form carries a **gap limit of 20**:
discovery stops after twenty consecutive empty indices, which makes "issued in
order, never skipped" a recoverability requirement. A wallet that jumps to
index 50 over an empty 0..29 has created an item its own restore will not find.
Raising the gap later is safe; lowering it can orphan an item a previous
version could still reach.

Only a BIP-39 wallet has these. A legacy independent key has no seed to branch
from, and inventing one would produce accounts no restore could find.

### Test vectors

Same 24-word mnemonic as above, empty passphrase. `CAVEZ` is collection
`024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e`.

| Branch | Account | Ed25519 seed | Zyn account |
|---|---|---|---|
| collection `CAVEZ` | 0 | `7cf4fe776a4a36615e5ed17585d5eba8f99b00c4e0ed40f304135b191eedb48a` | `ccb0a7aa79d032e65df27afc13fc1fd5f013e30aca2f700fe5abdfdb839ecf53` |
| collection `CAVEZ` | 7 | — | `86c85b008f08476cc9bd7a881e29c1d676b71ff5b7ef685851ae0a72c89e9db9` |
| index 0 | 0 | `a3fa8f79e857f5c8a2d13eefba5d3a88665066196cff3dfbb829442007fb40b8` | `b23d6f7e2d40a71108412c0eaf13727b8404e3453be8b8322220da9189832b6d` |
| index 1 | 0 | — | `277dc2783c01ca1b317d4aede043febf77e97965da11dac3e24c99de072c8eb7` |

None of these equals the identity account `b85db260…` from the same seed, which
is the property the separate salt exists for and which `zynzapd::receive` tests
directly.

## Backups and compatibility

Nap backup version 2 adds a `zyn` object:

- Derived: `{"source":"bip39-hkdf","version":1,"account":0,…}`. No
  duplicate Zyn secret is needed.
- Legacy: `{"source":"legacy-ed25519","seed":"<64 hex>"}`. The independent
  seed is recovery-critical and is deliberately included in the explicit
  secret backup.

Version-1 backups still parse. A v1 backup never contained the independently
generated Zyn seed, so it cannot promise restoration of that legacy account;
Nap reports whether an imported backup carried complete Zyn material.

New mnemonic wallets write a non-secret derivation descriptor at `zyn.key`.
Existing 32-byte `zyn.key` files are treated as legacy and are never silently
replaced. `/api/zyn-key` inventories movable holdings and non-transferable
creator authorities. Automatic migration is **disabled**: its first draft
switched identities without proving settlement and did not provide resumable
checkpoints. The endpoint fails before any cancellation, transfer or key write.
Issue #15 stays open until verified migration and end-to-end recovery drills
are implemented. A phrase-only import into an existing legacy installation
preserves that independent Zyn key; use a complete v2 backup to restore both
identities explicitly. Do not assume the new phrase recovers a retained legacy key.

Restore stages and validates the replacement before installation. A durable
`nap-restore-pending.json` journal coordinates wallet key, wallet state, Zyn
descriptor and account-local caches. Startup rolls back an interrupted install
before opening either identity. Original files remain in a mode-0700
`restore-backup-*` directory with mode-0600 recovery files and `index.json`.
These copies contain secrets; protect them like the portable backup.

`POST /api/export-file` with explicit confirmation writes the complete backup
directly to a new mode-0600 file in the Nap data directory and returns its path.
The UI never receives the independent Zyn seed for export. `/api/export` returns
no portable backup when that would expose a legacy Zyn seed; its recovery phrase
display is not a substitute for the complete file backup.
