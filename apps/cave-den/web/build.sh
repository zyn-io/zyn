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
cp "$WEB"/{index.html,base.css,den.css,app.js,gate.js,den.js,den-ui.js,den-admin.js,config.json} "$OUT/"
cp "$WEB/voice-worker.js" "$OUT/den/voice-worker.js"
cp "$WEB/voice-io.js" "$OUT/den/voice-io.js"
for f in den_mls.js den_mls_bg.wasm; do
  [ -f "$WEB/../mls/pkg/$f" ] || { echo "mls/pkg/$f is missing: run mls/build.sh" >&2; exit 1; }
  cp "$WEB/../mls/pkg/$f" "$OUT/den/"
done
PHX="$WEB/../server/deps/phoenix/priv/static/phoenix.mjs"
[ -f "$PHX" ] || { echo "the server's deps are missing: run mix deps.get in server/" >&2; exit 1; }
cp "$PHX" "$OUT/den/phoenix.mjs"
if [ -n "$ADDON" ]; then
  "$ADDON/build.sh" "$OUT"
  # An add-on's own page must keep every element the scripts look up that
  # the core page has, or that part of the page silently goes missing.
  used=$(grep -ohE "el\('[a-z0-9-]+'\)|getElementById\('[a-z0-9-]+'\)" "$WEB"/app.js "$WEB"/den-ui.js | grep -oE "'[a-z0-9-]+'" | tr -d "'" | sort -u)
  for id in $used; do
    if grep -q "id=\"$id\"" "$WEB/index.html" && ! grep -q "id=\"$id\"" "$OUT/index.html"; then
      echo "warning: $ADDON/index.html has no #$id, which the core page has and the scripts use" >&2
    fi
  done
fi
echo "page in $OUT"
