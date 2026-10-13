#!/usr/bin/env bash
# The Cave's den page on top of the core one (web/build.sh OUT cave): its
# page, the store's styles, fonts and art, and its config — day keys from
# cavebro.win, entry through cavebro.win/pass. Not published with the den.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
STORE="$HERE/../../cave-collection-z"
OUT=$1
cp "$HERE"/{index.html,cave.css,config.json} "$OUT/"
cp "$STORE/styles.css" "$OUT/styles.css"
mkdir -p "$OUT/assets/fonts"
cp "$STORE/assets/cave-mark.svg" "$STORE/assets/cave-background.png" "$OUT/assets/"
cp "$STORE"/assets/fonts/*.ttf "$STORE"/assets/fonts/*.txt "$OUT/assets/fonts/"
