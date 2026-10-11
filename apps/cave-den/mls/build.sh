#!/usr/bin/env bash
# The den's MLS and blind-token library, for the page: wasm and its bindings in pkg/.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir pkg target/wasm32-unknown-unknown/release/den_mls.wasm
ls -la pkg/den_mls_bg.wasm
