#!/usr/bin/env bash
# A second mainnet node on Contabo: Zakura beside Zebra (RUNBOOK "Zakura").
# Installed and started by ops/zakura-setup.sh as the transient unit
# `zakura-bootstrap`; watch it with `journalctl -u zakura-bootstrap -f`.
#
#   1. the latest archive snapshot, from Zakura's own latest.json: downloaded
#      with resume, capped (LIMIT) so Zebra and the gateways keep their link,
#      and checked against its SHA-256 before anything is unpacked;
#   2. unpacked into /var/lib/zyn/zakura, the tarball then deleted;
#   3. the container, pinned to the version that made the snapshot. RPC on
#      127.0.0.1:8242 only; P2P outbound only (no port published);
#   4. a cross-check: the same block hash and note-tree roots from both nodes.
#
# Safe to run again: each step is skipped when its result is already there.
set -euo pipefail
NAME=zakura-mainnet
STATE=/var/lib/zyn/zakura
DL=/var/lib/zyn/zakura-download
RPC=8242
LIMIT=${LIMIT:-15M}
say() { echo "zakura-bootstrap: $*"; }

if docker ps -a --format '{{.Names}}' | grep -qx "$NAME"; then
  say "the container $NAME already exists; going straight to the check"
else
  manifest=$(curl -fsS https://zakura.valargroup.dev/mainnet/latest.json)
  field() { printf '%s' "$manifest" | python3 -c "import json,sys; print(json.load(sys.stdin)['$1'])"; }
  url=$(field url); sha=$(field sha256); file=$(field filename)
  size=$(field size_bytes); version=$(field zakura_version); height=$(field height)
  [ "$(field snapshot_kind)" = archive ] || { say "latest.json is not an archive snapshot"; exit 1; }
  say "snapshot $file: height $height, $((size / 1000000000)) GB, made by Zakura $version"

  if [ ! -d "$STATE/state" ]; then
    have=0; [ -f "$DL/$file" ] && have=$(stat -c %s "$DL/$file")
    need=$(( (size - have) + size + 30000000000 ))   # the rest of the download, the unpacked state, 30 GB spare
    free=$(( $(df --output=avail -B1 /var/lib/zyn | tail -1) ))
    if [ "$free" -lt "$need" ]; then
      say "not enough disk: $((free / 1000000000)) GB free, $((need / 1000000000)) GB needed"; exit 1
    fi
    command -v aria2c >/dev/null || { say "installing aria2"; DEBIAN_FRONTEND=noninteractive apt-get install -y -q aria2 >/dev/null; }
    mkdir -p "$DL"
    # A partial download of an older snapshot is dead weight once latest.json
    # has moved on: 272 GB of it.
    for old in "$DL"/*.tar.zst; do
      [ -e "$old" ] && [ "$(basename "$old")" != "$file" ] && { say "removing an older partial download, $(basename "$old")"; rm -f "$old" "$old.aria2"; }
    done
    say "downloading at up to $LIMIT/s (resumes where it stopped)"
    # One dropped connection ended the first attempt (28 Sep, 10 GB in). Each
    # connection now retries for ever, and if aria2 gives up anyway the loop
    # resumes it from its control file.
    for attempt in $(seq 1 50); do
      if aria2c -c -x8 -s8 --max-overall-download-limit="$LIMIT" --summary-interval=600 \
        --console-log-level=warn --file-allocation=none \
        --max-tries=0 --retry-wait=20 --timeout=60 --connect-timeout=30 \
        --checksum=sha-256="$sha" -d "$DL" -o "$file" "$url"; then
        break
      fi
      [ "$attempt" -eq 50 ] && { say "the download failed 50 times; giving up"; exit 1; }
      say "the download stopped (attempt $attempt); resuming in a minute"
      sleep 60
    done
    say "downloaded and matches its SHA-256; unpacking"
    rm -rf "$STATE.partial"; mkdir -p "$STATE.partial"
    zstd -dc "$DL/$file" | tar -x -C "$STATE.partial"
    [ -d "$STATE.partial/state" ] || { say "the snapshot did not contain state/"; exit 1; }
    rm -rf "$STATE"; mv "$STATE.partial" "$STATE"
    rm -f "$DL/$file"; rmdir "$DL" 2>/dev/null || true
    say "unpacked: $(du -sh "$STATE" | cut -f1)"
  fi

  chown -R 10001:10001 "$STATE"
  docker pull -q "zakuracore/zakura:$version" >/dev/null
  docker run -d --name "$NAME" --restart unless-stopped \
    -e ZAKURA_NETWORK__NETWORK=Mainnet \
    -e ZAKURA_NETWORK__LISTEN_ADDR=0.0.0.0:8233 \
    -e ZAKURA_STATE__CACHE_DIR=/var/cache/zakura \
    -e ZAKURA_RPC__COOKIE_DIR=/var/cache/zakura \
    -e ZAKURA_RPC__LISTEN_ADDR=0.0.0.0:8232 \
    -e ZAKURA_RPC__ENABLE_COOKIE_AUTH=false \
    -p 127.0.0.1:$RPC:8232 \
    -v "$STATE":/var/cache/zakura \
    "zakuracore/zakura:$version" zakurad >/dev/null
  say "started $NAME (zakura $version), RPC 127.0.0.1:$RPC"
fi

rpc() {
  curl -s -m 10 -X POST "http://127.0.0.1:$1" -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":$3}"
}
res() { python3 -c "import json,sys; print(json.dumps(json.load(sys.stdin)['result']))" 2>/dev/null; }

say "waiting for it to answer and catch up with Zebra"
for i in $(seq 1 360); do
  z=$(rpc 8232 getblockcount '[]' | res || echo 0)
  k=$(rpc $RPC getblockcount '[]' | res || echo 0)
  if [ "${k:-0}" -gt 0 ] && [ "$k" -ge "$((z - 2))" ]; then break; fi
  [ $((i % 12)) -eq 0 ] && say "zakura at ${k:-no answer yet}, zebra at $z"
  sleep 10
done
chain=$(rpc $RPC getblockchaininfo '[]' | python3 -c "import json,sys; r=json.load(sys.stdin)['result']; print(r['chain'], r['consensus']['nextblock'])")
say "zakura: chain $chain, height $k (zebra $z)"

# The same answers from both, a few blocks back from the lower tip.
h=$(( (k < z ? k : z) - 10 ))
a=$(rpc 8232 getblockhash "[$h]" | res); b=$(rpc $RPC getblockhash "[$h]" | res)
ta=$(rpc 8232 z_gettreestate "[\"$h\"]" | python3 -c "import json,sys; r=json.load(sys.stdin)['result']; print(r['orchard']['commitments']['finalRoot'])")
tb=$(rpc $RPC z_gettreestate "[\"$h\"]" | python3 -c "import json,sys; r=json.load(sys.stdin)['result']; print(r['orchard']['commitments']['finalRoot'])")
if [ "$a" = "$b" ] && [ "$ta" = "$tb" ] && [ -n "$a" ]; then
  say "AGREES with Zebra at height $h: block $a, Orchard root $ta"
  # Zakura releases halt themselves a few weeks after they ship; say when.
  say "this release stops at: $(rpc $RPC getdeprecationinfo '[]' | res)"
else
  say "DISAGREES with Zebra at height $h: block $a / $b, Orchard root $ta / $tb"; exit 1
fi
