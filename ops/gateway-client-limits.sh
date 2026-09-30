#!/usr/bin/env bash
# Rate-limit the Contabo gateways per visitor, not per Cloudflare address.
#
#   ops/gateway-client-limits.sh
#
# Every request reaches nginx from Cloudflare — browsers through the proxied
# hostnames, Nap web through its Pages Functions — so `$binary_remote_addr` was
# a handful of Cloudflare addresses, and 8 r/s was one budget shared by every
# user. Now each limit (zynrpc, zynzec, zecrpc, and their connection limits)
# is keyed on `$zyn_client`:
#
#   1. the address Nap web's relay forwards (X-Zyn-Client), only when it comes
#      with the relay key (X-Zyn-Relay-Key == ZYN_RELAY_KEY);
#   2. else CF-Connecting-IP, only when the connection is from Cloudflare's
#      published ranges — 443 is open, and anyone can set that header;
#   3. else the connection's own address.
#
# The real-IP module is not used: access logs keep recording the connecting
# address, and a visitor's address exists only in the limit's memory.
#
# Steps: make the relay key (once, ~/.zyn/gateway-relay.key, 0600); install
# the nginx part on Contabo, test it, reload, or put everything back; set the
# key as a Pages secret on nap-web and redeploy it so the relay starts sending.
set -euo pipefail
cd "$(dirname "$0")/.."
host=root@95.111.240.24
key_file=$HOME/.zyn/gateway-relay.key

mkdir -p "$(dirname "$key_file")"
if [ ! -s "$key_file" ]; then
  (umask 077; openssl rand -hex 32 > "$key_file")
  echo "== new relay key in $key_file"
fi

echo "== nginx on Contabo"
# The key travels on stdin, never in an argument a process list would show.
{ cat "$key_file"; cat ops/gateway-client-limits.remote.sh; } | ssh -o BatchMode=yes "$host" 'read -r KEY; export KEY; bash -s -- install'

echo "== nap-web: relay key, then redeploy"
/opt/homebrew/bin/wrangler pages secret put ZYN_RELAY_KEY --project-name nap-web < "$key_file"
(cd apps/nap-web && npm run -s build && /opt/homebrew/bin/wrangler pages deploy dist --project-name nap-web --branch main --commit-dirty=true | tail -1)
echo "== done"
