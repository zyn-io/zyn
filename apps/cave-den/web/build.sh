#!/usr/bin/env bash
# Builds the den's page into OUT (by default server/priv/page, which the den
# serves at "/"): these files, the MLS wasm from ../mls/pkg (made by
# ../mls/build.sh), and the Phoenix socket client from the server's own deps,
# so the page and the den speak the same version. A host's add-on directory
# goes on top: its build.sh gets OUT and may replace any file (the page, its
# styles, config.json).
#
#   web/build.sh [OUT] [ADDON]
set -euo pipefail
WEB=$(cd "$(dirname "$0")" && pwd)
OUT=${1:-$WEB/../server/priv/page}
ADDON=${2:-}
rm -rf "$OUT" && mkdir -p "$OUT/den"
cp "$WEB"/{index.html,base.css,den.css,app.js,gate.js,den.js,den-ui.js,config.json} "$OUT/"
cp "$WEB/voice-worker.js" "$OUT/den/voice-worker.js"
for f in den_mls.js den_mls_bg.wasm; do
  [ -f "$WEB/../mls/pkg/$f" ] || { echo "mls/pkg/$f is missing: run mls/build.sh" >&2; exit 1; }
  cp "$WEB/../mls/pkg/$f" "$OUT/den/"
done
PHX="$WEB/../server/deps/phoenix/priv/static/phoenix.mjs"
[ -f "$PHX" ] || { echo "the server's deps are missing: run mix deps.get in server/" >&2; exit 1; }
cp "$PHX" "$OUT/den/phoenix.mjs"
if [ -n "$ADDON" ]; then "$ADDON/build.sh" "$OUT"; fi
echo "page in $OUT"
