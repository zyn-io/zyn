#!/usr/bin/env bash
# Start the Zakura bootstrap on Contabo (see ops/zakura-bootstrap.remote.sh).
#
#   ops/zakura-setup.sh            # LIMIT=15M by default; LIMIT=25M to go faster
#
# It runs there as the transient unit `zakura-bootstrap`, so it keeps going
# after this returns. Nothing reads from Zakura until we point something at it.
set -euo pipefail
cd "$(dirname "$0")/.."
host=root@95.111.240.24
scp -q ops/zakura-bootstrap.remote.sh "$host:/tmp/zakura-bootstrap"
ssh -o BatchMode=yes "$host" bash -s -- "${LIMIT:-15M}" <<'REMOTE'
set -euo pipefail
install -m755 /tmp/zakura-bootstrap /usr/local/sbin/zakura-bootstrap && rm /tmp/zakura-bootstrap
if systemctl is-active --quiet zakura-bootstrap; then
  echo "already running — journalctl -u zakura-bootstrap -f"; exit 0
fi
systemctl reset-failed zakura-bootstrap 2>/dev/null || true
# Idle disk priority and a low CPU share: Zebra and the gateways come first.
systemd-run --unit zakura-bootstrap --collect -p Nice=10 -p IOSchedulingClass=idle \
  -E LIMIT="$1" /usr/local/sbin/zakura-bootstrap >/dev/null
sleep 5
journalctl -u zakura-bootstrap --no-pager | tail -4
echo "started. Watch: ssh root@95.111.240.24 journalctl -u zakura-bootstrap -f"
REMOTE
