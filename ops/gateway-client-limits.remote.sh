# Runs on Contabo, from ops/gateway-client-limits.sh (the key arrives in $KEY).
#
#   install          write the config into /etc/nginx, `nginx -t`, reload;
#                    on any failure put every file back as it was
#   render <dir>     write the same files under <dir> only (for testing)
set -euo pipefail
mode=${1:-install}
[ -n "${KEY:-}" ] || { echo "no relay key" >&2; exit 1; }
printf '%s' "$KEY" | grep -Eq '^[0-9a-f]{64}$' || { echo "the relay key is not 64 hex digits" >&2; exit 1; }

ZONES="conf.d/zyn-rpc-limits.conf conf.d/zec-rpc-map.conf conf.d/zyn-zec-limits.conf"

render() {  # $1 = an nginx root: /etc/nginx, or a copy of it
  local root=$1 v4 v6
  v4=$(curl -fsS -m 10 https://www.cloudflare.com/ips-v4)
  v6=$(curl -fsS -m 10 https://www.cloudflare.com/ips-v6)
  [ "$(printf '%s\n' "$v4" | grep -c /)" -ge 10 ] || { echo "Cloudflare's ranges did not load" >&2; exit 1; }

  # An anchored regex, not a plain key: a 64-digit key overflows nginx's
  # default map hash bucket, and regexes are not hashed.
  (umask 077; printf '"~^%s$" 1;\n' "$KEY" > "$root/zyn-relay-key.map")
  {
    echo "# Who a request is from, for the rate limits — ops/gateway-client-limits.sh."
    echo "# Loads first (00-) so the limit zones can name \$zyn_client."
    echo "geo \$zyn_from_cloudflare {"
    echo "    default 0;"
    printf '%s\n%s\n' "$v4" "$v6" | grep / | sed 's/^/    /; s/$/ 1;/'
    echo "}"
    echo "map \$http_x_zyn_relay_key \$zyn_relay_ok {"
    echo "    default 0;"
    echo "    include $root/zyn-relay-key.map;"
    echo "}"
    echo "map \$zyn_relay_ok \$zyn_relayed_client { 1 \$http_x_zyn_client; default \"\"; }"
    echo "map \$zyn_from_cloudflare \$zyn_edge_client { 1 \$http_cf_connecting_ip; default \"\"; }"
    echo "map \$zyn_relayed_client \$zyn_client_a { \"\" \$zyn_edge_client; default \$zyn_relayed_client; }"
    echo "map \$zyn_client_a \$zyn_client { \"\" \$remote_addr; default \$zyn_client_a; }"
  } > "$root/conf.d/00-zyn-client.conf"
  for z in $ZONES; do
    sed -i -E 's/^(limit_(req|conn)_zone) \$binary_remote_addr /\1 $zyn_client /' "$root/$z"
  done
}

case $mode in
  render)
    render "$2"
    ;;
  install)
    backup=/root/nginx-before-client-limits-$(date +%Y%m%d%H%M%S)
    mkdir -p "$backup/conf.d"
    for z in $ZONES; do cp -p "/etc/nginx/$z" "$backup/$z"; done
    restore() {
      for z in $ZONES; do cp -p "$backup/$z" "/etc/nginx/$z"; done
      rm -f /etc/nginx/conf.d/00-zyn-client.conf /etc/nginx/zyn-relay-key.map
    }
    render /etc/nginx
    if nginx -t 2>&1 | tail -2; then
      systemctl reload nginx
      grep -h "limit_req_zone\|limit_conn_zone" /etc/nginx/conf.d/*.conf | sed 's/^/   /'
      echo "   reloaded. The files as they were: $backup"
    else
      restore
      echo "!! nginx -t failed; everything is back as it was" >&2
      exit 1
    fi
    ;;
esac
