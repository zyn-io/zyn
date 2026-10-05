#!/usr/bin/env bash
# Publishes Nap to github.com/zyn-io/nap-wallet: the wallet, its builds and the
# libraries its WebAssembly is made from. Taken from this repository's HEAD
# only, so nothing uncommitted (another session's work in progress) is ever
# published. Shows what would change; pushes only with --push.
#
#   scripts/nap-wallet/sync.sh           build the public tree, test it, show the diff
#   scripts/nap-wallet/sync.sh --push    …and commit and push it
set -euo pipefail
ZYN=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
HERE="$ZYN/scripts/nap-wallet"
OUT=${OUT:-$(mktemp -d)/nap-wallet}
REV=$(git -C "$ZYN" rev-parse --short HEAD)

# What Nap is. Anything else in the workspace stays private.
PATHS=(
  apps/nap-web apps/extension apps/nap-mobile
  apps/nap-connect/nap-connect.js apps/nap-connect/README.md
  zyn-vm zyn-bridge zyn-custody zyn-pool
  # NU7 on testnet (§165): the patched zcash_protocol and the cfg that turns it on.
  vendor/zcash_protocol .cargo/config.toml
)
# Local tooling that works on the operator's own test wallets.
DROP=(apps/nap-web/testnet-wallet.local.mjs apps/nap-web/rehearse.local.mjs)

git clone -q https://github.com/zyn-io/nap-wallet.git "$OUT"
cd "$OUT"
# Start from nothing but the history and the license.
find . -mindepth 1 -maxdepth 1 ! -name .git ! -name LICENSE -exec rm -rf {} +
git -C "$ZYN" archive HEAD "${PATHS[@]}" | tar -x -C "$OUT"
for f in "${DROP[@]}"; do rm -f "$OUT/$f"; done
cp -R "$HERE/root/." "$OUT/"
# The workspace's lock, pruned to these four crates without upgrading any.
cp "$ZYN/Cargo.lock" "$OUT/Cargo.lock"
cargo metadata --format-version 1 --offline >/dev/null

# Nothing secret goes out: keys, phrases, tokens, private endpoints.
if grep -rIlE --exclude-dir=.git --exclude-dir=node_modules -e '-----BEGIN [A-Z ]*PRIVATE KEY-----|ZYN_SUBMIT_TOKEN=[^$ ]|"secretKey" *: *\[|xprv[0-9A-Za-z]{20,}|AKIA[0-9A-Z]{16}|ghp_[0-9A-Za-z]{30,}|sk-[0-9A-Za-z]{30,}' .; then
  echo "refusing: the files above look like they hold a secret" >&2
  exit 1
fi
# Nor where the servers are, or whose machine built it.
if grep -rIlE '(95\.111\.240|168\.119\.53|192\.168)\.[0-9]+|/Users/[a-z]+/' . --exclude-dir=.git --exclude-dir=node_modules --exclude-dir=target; then
  echo "refusing: the files above name a server address or a local path" >&2
  exit 1
fi
big=$(find . -path ./.git -prune -o -type f -size +50M -print)
[ -z "$big" ] || { echo "refusing: files over 50 MB: $big" >&2; exit 1; }

# It must build and pass on its own, without the rest of the workspace.
(cd apps/nap-web && npm ci --silent && npm test 2>&1 | grep -E '^ℹ (pass|fail|skipped)')
cargo test --workspace --quiet 2>&1 | grep -E '^test result' | awk '{p+=$4; f+=$6} END {print "cargo: " p " passed, " f " failed"; exit f > 0}'

git add -A
git status --short | sed -n '1,40p'
echo "$(git status --short | wc -l | tr -d ' ') paths change; tree at $OUT"
if [ "${1:-}" = --push ]; then
  git -c user.name="$(git -C "$ZYN" config user.name)" -c user.email="$(git -C "$ZYN" config user.email)" \
    commit -q -m "Nap as published from Zyn $REV" -m "The wallet (web, extension, mobile), Nap Connect's SDK, and the libraries its WebAssembly builds from."
  git push -q origin HEAD:main
  echo "pushed $(git rev-parse --short HEAD)"
fi
