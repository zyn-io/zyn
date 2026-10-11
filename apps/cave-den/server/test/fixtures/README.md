Test fixtures, made once with the den's wasm library (`apps/cave-den/mls`) and Nap's code:

- `keys/2026-10-12.pem` — a day key for tests only. It signs nothing real.
- `blinded.hex`, `token.json` — a token for 2026-10-12, device `ab…ab`, blinded by the wasm library, signed blind by
  `Den.Keys.blind_sign/2` with that key, and unblinded by the wasm library.
- `join.bin` (an external join, epoch 0), `text.bin` (words, epoch 1), `remove.bin` (a member's commit, epoch 2) — MLS
  messages in room `general` from the Rust library.
- `pass.json` — Nap's `daoPass` of a code of 32 × 0x11.
