#!/usr/bin/env bash
# Zyn Pay rollout to the object testnet (chain 212), verifier first
# (DECISIONS §51.8, §188.13; RUNBOOK "Zyn Pay").
#
#   ops/zyn-pay-rollout.sh testnet
#
# Order: 1. the replica (Contabo) and ZynScan (Hetzner), 2. the signers,
# 3. the sequencer (Hetzner), which is the one that starts emitting pay ops.
# Mainnet 26460 waits for the NU7 switch and is refused here.
#
# Every step asks first (y/N) and stops on anything else. Each installed
# binary is backed up beside itself as <name>.bak-<date>, the way RUNBOOK
# records its backups, so going back is one `cp` and a restart.
set -euo pipefail

net=${1:?testnet or mainnet}
[ "$net" = testnet ] || { echo "mainnet waits for the NU7 switch" >&2; exit 1; }

# --- hosts and units (RUNBOOK §5f-bis, §5f-quindecies, §5f-nonies) ---------
hetzner=root@168.119.53.39        # aarch64: sequencer, ZynScan, custodians
contabo=root@95.111.240.24        # x86_64: the testnet object replica
node_unit=zyn-object-node         # Hetzner, RPC 127.0.0.1:8499
scan_unit=zynscan                 # Hetzner, chain 212, listens 127.0.0.1:8093
replica_unit=zyn-object-replica   # Contabo, verifies chain 212
node_bin=/usr/local/bin/zyn-object-node
scan_bin=/usr/local/bin/zynscan
# MUST BE CHECKED: RUNBOOK names the replica unit and its backup
# (/root/zyn-object-replica.bak-20260923) but never its install path. The
# usual place is below; the script refuses to go on unless the unit's
# ExecStart really uses this path.
replica_bin=/usr/local/bin/zyn-object-replica
# -----------------------------------------------------------------------------

stamp=$(date +%Y%m%d)
arm=target/aarch64-unknown-linux-gnu/release
x86=target/x86_64-unknown-linux-gnu/release

step() {
  local a
  read -r -p "$1 -- go? [y/N] " a
  [ "$a" = y ] || { echo "stopped before: $1" >&2; exit 1; }
}

# install_on <host> <unit> <installed path> <local binary>
# Stops the unit, backs up and installs the binary, starts it, and refuses to
# continue if the unit is not active a few seconds later.
install_on() {
  local host=$1 unit=$2 bin=$3 src=$4
  scp "$src" "$host:$bin.new"
  ssh "$host" bash -s -- "$unit" "$bin" "$stamp" <<'REMOTE'
set -euo pipefail
unit=$1 bin=$2 stamp=$3
systemctl cat "$unit" | grep -q "^ExecStart=.*$bin" \
  || { echo "$unit does not run $bin; not touching it" >&2; rm -f "$bin.new"; exit 1; }
systemctl stop "$unit"
cp -p "$bin" "$bin.bak-$stamp"
install -m755 "$bin.new" "$bin"
rm -f "$bin.new"
systemctl start "$unit"
sleep 5
systemctl is-active --quiet "$unit" || { journalctl -u "$unit" -n 30 --no-pager >&2; echo "$unit is not active" >&2; exit 1; }
echo "$unit active; previous binary kept as $bin.bak-$stamp"
REMOTE
}

cd "$(dirname "$0")/.."

step "0/3 build zyn-object-node + zynscan (aarch64) and zyn-object-replica (x86_64) locally"
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p zynzapd --bin zyn-object-node --bin zynscan
cargo zigbuild --release --target x86_64-unknown-linux-gnu -p zynzapd --bin zyn-object-replica

step "1/3 verifiers: Contabo $replica_unit (x86_64), then Hetzner $scan_unit (aarch64)"
install_on "$contabo" "$replica_unit" "$replica_bin" "$x86/zyn-object-replica"
echo "== replica log (it replays from genesis; wait for 'chain 212 verified through epoch N')"
ssh "$contabo" "journalctl -u $replica_unit -n 15 --no-pager"
step "1b/3 the replica's log above shows no halt; continue with ZynScan"
install_on "$hetzner" "$scan_unit" "$scan_bin" "$arm/zynscan"
echo "== zynscan: /api/summary 'agrees' must be true (false is a finding, not a restart)"
sleep 5
ssh "$hetzner" "curl -s -m 10 127.0.0.1:8093/api/summary" || true; echo

step "2/3 signers: nothing to install. The custodians (zyn-custodian, FROST shares, RUNBOOK §4v) sign anchors and exits and never run the ledger, so pay ops do not reach them. Confirm no zyn-object replica or node runs on any other host you control"

step "3/3 sequencer: $node_unit on Hetzner (starts the pay clock once the chain has pay state)"
install_on "$hetzner" "$node_unit" "$node_bin" "$arm/zyn-object-node"
ssh "$hetzner" "journalctl -u $node_unit -n 15 --no-pager"

cat <<'LIVE'

Rolled out to testnet 212. Live check, from the test wallets:

  1. Alice subscribes SHOP for 1 BOLD.zy daily, 2 periods funded.
  2. The clock pays period 0 at first_due (the node posts the clock every
     ZYN_PAY_TICK_SECS, default 10; ZynScan shows a "Pay clock" row).
  3. On Hetzner, with the schedule id from step 1:

       RPC=127.0.0.1:8499
       curl -s -X POST $RPC -d '{"method":"zyn_paySchedules","ids":"<schedule id hex>"}'

     It must show index 1 and pot 1.
  4. Alice cancels the schedule; the refund returned is 1.

The replica must keep printing "chain 212 verified through epoch N", and
ZynScan /api/summary must keep "agrees": true.
LIVE
