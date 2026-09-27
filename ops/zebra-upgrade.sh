#!/usr/bin/env bash
# Move one host's Zebra to a pinned zfnd/zebra release, keeping its state,
# config, ports, network and environment exactly as they were.
#
#   ops/zebra-upgrade.sh contabo 6.4.2     # mainnet Zebra (zebrad-mainnet)
#   ops/zebra-upgrade.sh hetzner 6.4.2     # testnet Zebra (zebrad)
#
# The new container is built from the old one's own settings (docker inspect),
# not retyped: the image's traps — CONFIG_FILE_PATH, ZEBRA_STATE__CACHE_DIR,
# ZCASHD_COMPAT (RUNBOOK §5f) — are carried over rather than remembered.
#
# The old container is stopped and kept as <name>-<old version>, restart off,
# so going back is three commands (printed at the end). Nothing is deleted.
# What reads Zebra (object node, settlers, lightd, zynscan) sees it gone for
# the minute it takes the new one to open its state, and retries.
set -euo pipefail

host=${1:?which host: contabo or hetzner}
tag=${2:?which Zebra release, e.g. 6.4.2}
case $host in
  contabo) to=root@95.111.240.24; name=zebrad-mainnet; rpc=8232;  chain=main ;;
  hetzner) to=root@168.119.53.39; name=zebrad;         rpc=18232; chain=test ;;
  *) echo "unknown host $host (contabo or hetzner)" >&2; exit 2 ;;
esac

ssh -o BatchMode=yes "$to" bash -s -- "$name" "$tag" "$rpc" "$chain" <<'REMOTE'
set -euo pipefail
name=$1 tag=$2 rpc=$3 chain=$4
img=zfnd/zebra:$tag

info() {
  curl -s -m 5 -X POST "http://127.0.0.1:$rpc" -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getblockchaininfo","params":[]}' 2>/dev/null |
    python3 -c 'import json,sys; r=json.load(sys.stdin)["result"]; print(r["chain"], r["blocks"], r["consensus"]["nextblock"])' 2>/dev/null || true
}

old_version=$(docker exec "$name" zebrad --version 2>/dev/null | awk '{print $2}')
echo "== $name: zebrad $old_version -> $tag"
[ "$old_version" = "$tag" ] && { echo "already on $tag"; exit 0; }
before=$(info); echo "   before: $before"
[ -n "$before" ] || { echo "the running Zebra does not answer on $rpc; not touching it" >&2; exit 1; }

docker pull -q "$img"

# The old container's settings, as docker run arguments. Environment the image
# sets itself (PATH, UID, ...) is left to the new image.
args=$(docker inspect "$name" "$(docker inspect -f '{{.Image}}' "$name")" | python3 -c '
import json, shlex, sys
c, image = json.load(sys.stdin)
own = set(image["Config"].get("Env") or [])
h = c["HostConfig"]
a = ["--restart", h["RestartPolicy"]["Name"] or "no"]
if h.get("NetworkMode") not in (None, "", "default", "bridge"):
    a += ["--network", h["NetworkMode"]]
for e in c["Config"].get("Env") or []:
    if e not in own:
        a += ["-e", e]
for b in h.get("Binds") or []:
    a += ["-v", b]
for port, binds in (h.get("PortBindings") or {}).items():
    for b in binds or []:
        a += ["-p", (b["HostIp"] + ":" if b["HostIp"] else "") + b["HostPort"] + ":" + port.split("/")[0]]
print(" ".join(shlex.quote(x) for x in a))
')
echo "   settings: $args"

# A clean shutdown: Zebra flushes its state on SIGTERM, and 10 s is not
# always enough for 300 GB of mainnet.
docker stop -t 180 "$name" >/dev/null
docker rename "$name" "$name-$old_version"
docker update --restart=no "$name-$old_version" >/dev/null
eval docker run -d --name "$name" $args "$img" zebrad >/dev/null

rollback="docker rm -f $name && docker rename $name-$old_version $name && docker update --restart=unless-stopped $name && docker start $name"
for i in $(seq 1 120); do
  now=$(info)
  if [ -n "$now" ]; then
    echo "   after:  $now   (zebrad $(docker exec "$name" zebrad --version | awk '{print $2}'))"
    set -- $now
    if [ "$1" != "$chain" ]; then
      echo "!! the new Zebra says chain '$1', not '$chain' — it did not read its config. Roll back:" >&2
      echo "   $rollback" >&2; exit 1
    fi
    echo "== up. The old container is kept, stopped, as $name-$old_version."
    echo "   Remove it once this has run a day:  docker rm $name-$old_version"
    echo "   Or go back:  $rollback"
    exit 0
  fi
  sleep 5
done
echo "!! no answer on $rpc after 10 minutes. docker logs --tail 50 $name; to go back:" >&2
echo "   $rollback" >&2
exit 1
REMOTE
