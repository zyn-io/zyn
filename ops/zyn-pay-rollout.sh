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

stamp=$(date +%Y%m%d-%H%M%S)   # to the second: a re-run never reuses a backup name
arm=target/aarch64-unknown-linux-gnu/release
x86=target/x86_64-unknown-linux-gnu/release

# Rollback commands, collected as each install starts and printed on any exit
# that is not a clean finish, and again at the end.
rollbacks=()
print_rollbacks() {
  [ ${#rollbacks[@]} -gt 0 ] || return 0
  echo "== rollback (per host; each restores the binary kept by this run and restarts its unit):"
  printf '   %s\n' "${rollbacks[@]}"
}
trap 'rc=$?; if [ $rc -ne 0 ]; then echo "STOPPED (exit $rc)" >&2; print_rollbacks >&2; fi' EXIT

step() {
  local a
  read -r -p "$1 -- go? [y/N] " a
  [ "$a" = y ] || { echo "stopped before: $1" >&2; exit 1; }
}

# install_on <host> <unit> <installed path> <local binary>
# Stops the unit, backs up and installs the binary, starts it. If anything
# fails after the stop, the backup is put back and the old binary is started
# again on the remote side before this exits non-zero: the unit is never left
# stopped by this script.
install_on() {
  local host=$1 unit=$2 bin=$3 src=$4
  rollbacks+=("ssh $host 'cp -p $bin.bak-$stamp $bin && systemctl restart $unit'")
  scp "$src" "$host:$bin.new" || { echo "scp to $host failed; nothing was touched" >&2; return 1; }
  ssh "$host" bash -s -- "$unit" "$bin" "$stamp" <<'REMOTE' || return 1
set -euo pipefail
unit=$1 bin=$2 stamp=$3
bak=$bin.bak-$stamp
new=$bin.new
refuse() { echo "$*" >&2; rm -f "$new"; exit 1; }

# The unit must run exactly this binary (fixed-string match; a here-string
# keeps grep's early exit from tripping pipefail).
es=$(systemctl show -p ExecStart --value "$unit")
grep -qF "path=$bin " <<<"$es" || refuse "$unit does not run $bin; not touching it"

# No other unit on this host may run the same file.
while read -r other; do
  [ -n "$other" ] && [ "$other" != "$unit.service" ] && [ "$other" != "$unit" ] || continue
  oes=$(systemctl show -p ExecStart --value "$other" 2>/dev/null || true)
  if grep -qF "path=$bin " <<<"$oes"; then refuse "$other also runs $bin; not touching it"; fi
done < <(systemctl list-unit-files --type=service --no-legend | awk '{print $1}')

[ ! -e "$bak" ] || refuse "$bak already exists; not overwriting it"
[ -s "$new" ] || refuse "$new is missing or empty"

systemctl stop "$unit"
made_bak=0
if cp -p "$bin" "$bak" && made_bak=1 \
   && install -m755 "$new" "$bin" \
   && systemctl start "$unit" && sleep 5 && systemctl is-active --quiet "$unit"; then
  rm -f "$new"
  echo "$unit active; previous binary kept as $bak"
  exit 0
fi

echo "FAILED installing $bin or starting $unit; restoring the previous binary" >&2
journalctl -u "$unit" -n 20 --no-pager >&2 || true
[ "$made_bak" = 1 ] && cp -p "$bak" "$bin"
systemctl restart "$unit" || true
rm -f "$new"
if systemctl is-active --quiet "$unit"; then
  echo "$unit is running the previous binary again" >&2
else
  echo "$unit is NOT active even after restoring; look at it now: journalctl -u $unit" >&2
fi
exit 1
REMOTE
  echo "   rollback: ssh $host 'cp -p $bin.bak-$stamp $bin && systemctl restart $unit'"
}

# gate_replica: the replica replays from genesis; pass only when its log, for
# this run of the unit, says it verified through the epoch the chain has
# anchored (ZynScan's anchored_through), without DISAGREES, and the unit stays
# active. Up to 10 minutes.
# Wording from zynzapd/src/bin/zyn-object-replica.rs:
#   "zyn-object-replica: chain {chain} verified through epoch {through}"
#   "zyn-object-replica: DISAGREES at epoch {epoch}: ..."
gate_replica() {
  local target
  target=$(ssh "$hetzner" "curl -s -m 10 127.0.0.1:8093/api/summary | python3 -c 'import json,sys; v=json.load(sys.stdin).get(\"anchored_through\"); print(v if v is not None else 0)'") \
    || { echo "cannot read the chain's anchored epoch from ZynScan; not going on" >&2; return 1; }
  [[ $target =~ ^[0-9]+$ ]] || { echo "unexpected anchored_through '$target'" >&2; return 1; }
  echo "== waiting for $replica_unit to verify through epoch $target (up to 10 min)"
  ssh "$contabo" bash -s -- "$replica_unit" "$target" <<'REMOTE'
set -euo pipefail
unit=$1 target=$2
id=$(systemctl show -p InvocationID --value "$unit")
for _ in $(seq 1 60); do
  systemctl is-active --quiet "$unit" || { echo "$unit stopped being active" >&2; exit 1; }
  log=$(journalctl "_SYSTEMD_INVOCATION_ID=$id" --no-pager -o cat || true)
  if grep -qF 'DISAGREES' <<<"$log"; then echo "$unit reports DISAGREES" >&2; grep -F 'DISAGREES' <<<"$log" | tail -3 >&2; exit 1; fi
  n=$(grep -oE 'verified through epoch [0-9]+' <<<"$log" | tail -1 | grep -oE '[0-9]+$' || true)
  if [ -n "$n" ] && [ "$n" -ge "$target" ]; then echo "$unit verified through epoch $n (chain anchored through $target)"; exit 0; fi
  sleep 10
done
echo "timed out: $unit has not verified through epoch $target" >&2
exit 1
REMOTE
}

# gate_scan: ZynScan's /api/summary must say "agrees": true (its replay
# reproduces the node's root); false or null is a finding. Up to 10 minutes.
gate_scan() {
  echo "== waiting for $scan_unit /api/summary agrees:true (up to 10 min)"
  ssh "$hetzner" bash -s -- "$scan_unit" <<'REMOTE'
set -euo pipefail
unit=$1
for _ in $(seq 1 60); do
  systemctl is-active --quiet "$unit" || { echo "$unit stopped being active" >&2; exit 1; }
  if curl -s -m 10 127.0.0.1:8093/api/summary | python3 -c 'import json,sys; sys.exit(0 if json.load(sys.stdin).get("agrees") is True else 1)' 2>/dev/null; then
    echo "$unit agrees: true"; exit 0
  fi
  sleep 10
done
echo "timed out: $unit does not report agrees: true" >&2
exit 1
REMOTE
}

cd "$(dirname "$0")/.."

step "0/3 build zyn-object-node + zynscan (aarch64) and zyn-object-replica (x86_64) locally"
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p zynzapd --bin zyn-object-node --bin zynscan
cargo zigbuild --release --target x86_64-unknown-linux-gnu -p zynzapd --bin zyn-object-replica

step "1/3 verifiers: Contabo $replica_unit (x86_64), then Hetzner $scan_unit (aarch64); the sequencer waits for both to pass"
install_on "$contabo" "$replica_unit" "$replica_bin" "$x86/zyn-object-replica"
install_on "$hetzner" "$scan_unit" "$scan_bin" "$arm/zynscan"
gate_replica || { echo "replica gate failed; the sequencer is NOT touched" >&2; exit 1; }
gate_scan || { echo "ZynScan gate failed; the sequencer is NOT touched" >&2; exit 1; }

step "2/3 signers updated (per RUNBOOK custody section, §4v): the custodians run zyn-custodian, which never runs the ledger, so pay ops do not reach them and nothing is installed here. Confirm none of them runs a zyn-object node or replica"

step "3/3 sequencer: $node_unit on Hetzner (starts the pay clock once the chain has pay state)"
install_on "$hetzner" "$node_unit" "$node_bin" "$arm/zyn-object-node"
ssh "$hetzner" "journalctl -u $node_unit -n 15 --no-pager"

print_rollbacks

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
