#!/usr/bin/env bash
# Epeaky's site → dist/: the static page, and install.sh copied from the
# repo's installer (scripts/den/root/install.sh), so the one-liner on the page
# runs exactly what is published with Epeaky.
#
#   ./build.sh && wrangler pages deploy dist --project-name epeaky-site --branch main
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SRC="$HERE/../../scripts/den/root/install.sh"
[ -f "$SRC" ] || { echo "no installer at scripts/den/root/install.sh yet: not building" >&2; exit 1; }
rm -rf "$HERE/dist" && cp -R "$HERE/public" "$HERE/dist"
cp "$SRC" "$HERE/dist/install.sh"
echo "site in $HERE/dist (install.sh $(wc -c < "$HERE/dist/install.sh" | tr -d ' ') bytes)"
