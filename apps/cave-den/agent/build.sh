#!/usr/bin/env bash
# The den's own client for epeaky-agent, into runtime/: den.js from ../web, the
# MLS wasm from ../mls/pkg (made by ../mls/build.sh) and the Phoenix socket
# client from the server's deps, as web/build.sh takes them.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$HERE/runtime
rm -rf "$OUT" && mkdir -p "$OUT/den"
cp "$HERE/../web/den.js" "$OUT/den.js"
for f in den_mls.js den_mls_bg.wasm; do
  [ -f "$HERE/../mls/pkg/$f" ] || { echo "mls/pkg/$f is missing: run mls/build.sh" >&2; exit 1; }
  cp "$HERE/../mls/pkg/$f" "$OUT/den/"
done
PHX="$HERE/../server/deps/phoenix/priv/static/phoenix.mjs"
[ -f "$PHX" ] || { echo "the server's deps are missing: run mix deps.get in server/" >&2; exit 1; }
cp "$PHX" "$OUT/den/phoenix.mjs"
echo "runtime in $OUT"
