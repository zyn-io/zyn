#!/usr/bin/env bash
# Epeaky's site → dist/: the static page; install.sh copied from the repo's
# installer (scripts/den/root/install.sh), so the one-liner on the page runs
# exactly what is published with Epeaky; and the agent's command line as an
# npm tarball (epeaky-agent.tgz), with its skill as skill.md.
#
#   ./build.sh && wrangler pages deploy dist --project-name epeaky-site --branch main
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
DEN="$HERE/../cave-den"
SRC="$HERE/../../scripts/den/root/install.sh"
[ -f "$SRC" ] || { echo "no installer at scripts/den/root/install.sh yet: not building" >&2; exit 1; }
# The agent goes out only as committed, like the rest of Epeaky.
if ! git -C "$DEN" diff --quiet HEAD -- agent web/den.js mls/src mls/Cargo.toml mls/Cargo.lock; then
  echo "the agent, den.js or mls/ has uncommitted changes: not building" >&2; exit 1
fi
rm -rf "$HERE/dist" && cp -R "$HERE/public" "$HERE/dist"
cp "$SRC" "$HERE/dist/install.sh"
(cd "$DEN/mls" && ./build.sh >/dev/null) && "$DEN/agent/build.sh" >/dev/null
TGZ=$(cd "$DEN/agent" && npm pack --silent --pack-destination "$HERE/dist")
mv "$HERE/dist/$TGZ" "$HERE/dist/epeaky-agent.tgz"
cp "$DEN/agent/skill/epeaky/SKILL.md" "$HERE/dist/skill.md"
cp "$DEN/agent/skill/epeaky-host/SKILL.md" "$HERE/dist/host-skill.md"
echo "site in $HERE/dist (install.sh $(wc -c < "$HERE/dist/install.sh" | tr -d ' ') bytes, epeaky-agent.tgz $(wc -c < "$HERE/dist/epeaky-agent.tgz" | tr -d ' ') bytes)"
