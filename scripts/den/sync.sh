#!/usr/bin/env bash
# Publishes Epeaky (the den) to github.com/zyn-io/epeaky: the MLS core (mls/), the server
# (server/) and the page the server serves (web/).
# Taken from this repository's HEAD only, so nothing uncommitted (another
# session's work in progress) is ever published, and never with this
# repository's history: the den's paths carry the Cave store and the §183
# prototype. Shows what would change; pushes only with --push.
#
#   scripts/den/sync.sh           build the public tree, check it, test it, show the diff
#   scripts/den/sync.sh --push    …and commit and push it (the repo must already exist)
set -euo pipefail
ZYN=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
HERE="$ZYN/scripts/den"
OUT=${OUT:-$(mktemp -d)/den}
REV=$(git -C "$ZYN" rev-parse --short HEAD)
REPO=zyn-io/epeaky
PUSH=${1:-}

# The public repo's history if it exists; otherwise a new one with none.
if git ls-remote -q "https://github.com/$REPO.git" >/dev/null 2>&1; then
  git clone -q "https://github.com/$REPO.git" "$OUT"
  find "$OUT" -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
else
  [ "$PUSH" = --push ] && { echo "refusing: $REPO does not exist yet; create it on GitHub first (empty, no README)" >&2; exit 1; }
  mkdir -p "$OUT" && git -C "$OUT" init -q -b main
fi
cd "$OUT"

# What the den is. The Cave's add-on (apps/cave-den/cave), README, art and
# keys stay private.
git -C "$ZYN" archive HEAD:apps/cave-den mls server web | tar -x -C "$OUT"
cp -R "$HERE/root/." "$OUT/"

blocked=()

# Nothing secret goes out. The one private key allowed is the test fixture,
# and it must not be one of the published day keys.
if grep -rIlE --exclude-dir=.git -e '-----BEGIN [A-Z ]*PRIVATE KEY-----|"secretKey" *: *\[|xprv[0-9A-Za-z]{20,}|AKIA[0-9A-Z]{16}|ghp_[0-9A-Za-z]{30,}|sk-[0-9A-Za-z]{30,}' . | grep -v '^\./server/test/fixtures/keys/'; then
  echo "refusing: the files above look like they hold a secret" >&2; exit 1
fi
for pem in server/test/fixtures/keys/*.pem; do
  pub=$(openssl pkey -in "$pem" -pubout -outform DER 2>/dev/null | base64 | tr -d '\n')
  if grep -qF "$pub" "$ZYN/apps/cave-collection-z/den-keys.json"; then
    echo "refusing: $pem is a published day key" >&2; exit 1
  fi
done
if git -C "$ZYN" grep -qE '^SECRET_KEY_BASE=|secret_key_base: "[^"]{64,}"' HEAD -- apps/cave-den/server/config/prod.exs apps/cave-den/server/config/runtime.exs; then
  echo "refusing: prod config holds a secret_key_base" >&2; exit 1
fi
# Nor where the servers are, or whose machine built it.
if grep -rIlE --exclude-dir=.git '(95\.111\.240|168\.119\.53|192\.168\.1)\.[0-9]+|\{ *(95, *111, *240|168, *119, *53|192, *168, *1),|/Users/[a-z]+/' .; then
  echo "refusing: the files above name a server address or a local path" >&2; exit 1
fi
big=$(find . -path ./.git -prune -o -type f -size +20M -print)
[ -z "$big" ] || { echo "refusing: files over 20 MB: $big" >&2; exit 1; }

# Not ready to push while any of these remain (they are fine to look at).
# (§1xx are this workspace's decisions; an RFC's §4.3 is fine. The page test
# names these words to forbid them.)
refs=$(grep -rIlE --exclude-dir=.git --exclude=den.test.js -e '§1[0-9]{2}|DECISIONS|Contabo|cavebro\.win|Collection Z|ZynScan|ZCave' mls server web || true)
[ -z "$refs" ] || blocked+=("internal references (§, DECISIONS, Contabo, Cave names) in: $(echo $refs)")
grep -qE 'collection: "[0-9a-f]{64}"' server/config/config.exs \
  && blocked+=("server/config/config.exs hardcodes the Cave's collection")
grep -rqE 'zyn-rpc\.cavebro\.win' server/config && blocked+=("dev config points at the Cave's own RPC")

# It must build and pass on its own.
(cd mls && ./build.sh >/dev/null 2>&1 && cargo test --quiet 2>&1 | grep -E '^test result' | awk '{p+=$4; f+=$6} END {print "mls: " p " passed, " f " failed"; exit f > 0}')
(cd server && mix deps.get >/dev/null && (cd ../web && ./build.sh dist >/dev/null && node --test tests/*.test.js 2>&1 | grep -E '^ℹ (pass|fail)' | sed 's/^/web: /') && MIX_ENV=test mix test 2>&1 | grep -E '[0-9]+ tests?, [0-9]+ failures?' | sed 's/^/server: /' | tee /dev/stderr | grep -q ' 0 failures')
rm -rf mls/target mls/pkg server/_build server/deps web/dist web/node_modules

git add -A
git status --short | sed -n '1,60p'
echo "$(git status --short | wc -l | tr -d ' ') paths change; tree at $OUT"

if [ ${#blocked[@]} -gt 0 ]; then
  echo "not ready to publish:"; printf '  - %s\n' "${blocked[@]}"
  [ "$PUSH" = --push ] && exit 1
  exit 0
fi
if [ "$PUSH" = --push ]; then
  git -c user.name="$(git -C "$ZYN" config user.name)" -c user.email="$(git -C "$ZYN" config user.email)" \
    commit -q -m "Epeaky as published from Zyn $REV" -m "The MLS core, the server and the page."
  git push -q origin HEAD:main
  echo "pushed $(git rev-parse --short HEAD)"
fi
