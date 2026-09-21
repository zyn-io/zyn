# Signer federation (rung 2) — Plan

> Inline execution; not a git repo. **Spec:** docs/superpowers/specs/2026-09-08-signer-federation-design.md

1. **Node endorsement API** (`zyn/src/node.rs`): `add_endorsement(&mut self, anchor_id, signer, sig) -> bool` (needs a signer set; verifies via a temp `Certificate` + `Certificate::endorses`-equivalent; folds into `pending_certificates`, creating the cert if absent; returns whether counted). `certificate_clears(&self, anchor_id) -> bool`. `certificate_for(&self, anchor_id) -> Option<&Certificate>`. Tests: below/at threshold; outsider and bad sig ignored.
2. **`OP_ENDORSE`** (`zynzapd/src/rpc.rs`, op 18): `anchor_id[32] signer[32] sig[64]` → `add_endorsement`; ok/`ignored`. Refused on a replica. No authority. Test: valid folds, invalid dropped.
3. **Settler gating** (`zynzapd/src/anchor.rs`): `AnchorSettler` gains `signing: bool`. `follow_up` at depth: if `signing`, confirm only when `node.certificate_clears(id)`, passing `node.certificate_for(id)`; else `None`. Alert `AWAITING_SIGNATURES` past a bound. Fake-node test.
4. **Config + wiring** (`config.rs`, `main.rs`): `ZYN_SIGNER_SET` (hex pubkeys, comma), `ZYN_SIGNER_THRESHOLD`; `node.with_signers(SignerSet::new(...))`; `OP_ENDORSE` wired; settler `signing = set.is_some()`.
5. **Signer-replica** (`zyn-replica.rs`, `replica.rs`): `ZYN_SIGNER_KEY` (ed25519 seed) + `ZYN_ENDORSE_TO`; after each verified anchor, sign its id and POST `OP_ENDORSE`. `ZYN_SIGNER_SET` on the replica → `Replica` checks `certificate.verify(set)` during `verify_one`, else `Halt::Unendorsed`. Test in `tests/replica.rs`.
6. **Docs + live**: RUNBOOK §4u, DECISIONS §34. Three signer keys, threshold 2 on testnet; deposit stays pending until 2 endorse, then releases; stop one, chain continues; outsider ignored.
