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

# Every ssh and scp is bounded: a hung connection ends instead of waiting.
ssh_opts=(-o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=4)

# Rollback commands, added once a host's backup exists, printed on any exit
# that is not a clean finish, and again at the end. `install` unlinks the
# destination first, so it works on a running executable (cp would fail with
# "Text file busy").
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

# The install-and-restore section runs on the host, detached (setsid/nohup),
# so a dropped ssh or a Ctrl-C here cannot leave the unit stopped: it finishes
# on its own, restoring the old binary on any failure, and writes its exit
# code to <script>.rc. This side polls for that file.
install_script() {
  cat <<'REMOTE'
unit=$1 bin=$2 stamp=$3
bak=$bin.bak-$stamp
new=$bin.new
refuse() { echo "$*"; rm -f "$new"; exit 1; }

# The unit must run exactly this binary (fixed-string match on a here-string,
# so grep's early exit cannot trip pipefail).
es=$(systemctl show -p ExecStart --value "$unit") || refuse "cannot read $unit"
grep -qF "path=$bin " <<<"$es" || refuse "$unit does not run $bin; not touching it"

# No other unit on this host may run the same file.
while read -r other; do
  [ -n "$other" ] && [ "$other" != "$unit.service" ] && [ "$other" != "$unit" ] || continue
  oes=$(systemctl show -p ExecStart --value "$other" 2>/dev/null || true)
  if grep -qF "path=$bin " <<<"$oes"; then refuse "$other also runs $bin; not touching it"; fi
done < <(systemctl list-unit-files --type=service --no-legend | awk '{print $1}')

[ ! -e "$bak" ] || refuse "$bak already exists; not overwriting it"
[ -s "$new" ] || refuse "$new is missing or empty"

# Nothing has changed yet: a failed stop is just a refusal.
systemctl stop "$unit" || refuse "systemctl stop $unit failed; nothing was changed"

made_bak=0 ok=0
if cp -p "$bin" "$bak"; then
  made_bak=1
  echo "BACKUP $bak"
  if install -m755 "$new" "$bin" && systemctl start "$unit" && sleep 5 && systemctl is-active --quiet "$unit"; then
    ok=1
  fi
fi
rm -f "$new"
if [ $ok = 1 ]; then
  echo "$unit active; previous binary kept as $bak"
  exit 0
fi

echo "FAILED installing $bin or starting $unit; restoring the previous binary"
journalctl -u "$unit" -n 20 --no-pager || true
systemctl stop "$unit" || true      # the new process may still be "activating"
if [ $made_bak = 1 ]; then
  install -m755 "$bak" "$bin" || echo "RESTORE of $bin FROM $bak FAILED"
fi
systemctl start "$unit" || echo "systemctl start $unit FAILED"
sleep 3
if systemctl is-active --quiet "$unit"; then
  echo "$unit is running again"
else
  echo "$unit is NOT active even after restoring; look at it now: journalctl -u $unit"
fi
exit 1
REMOTE
}

# install_on <host> <unit> <installed path> <local binary>
install_on() {
  local host=$1 unit=$2 bin=$3 src=$4
  local script=/root/.zyn-pay-install-$unit-$stamp.sh log rc i
  log=$script.log
  echo
  echo "!! do not interrupt while $unit is being replaced on $host. If you do, the host finishes the install or restores the old binary by itself; check with: ssh $host systemctl is-active $unit"
  scp "${ssh_opts[@]}" "$src" "$host:$bin.new" || { echo "scp to $host failed; nothing was touched" >&2; return 1; }
  install_script | ssh "${ssh_opts[@]}" "$host" "cat > $script" || { echo "could not place the install script on $host" >&2; return 1; }
  ssh "${ssh_opts[@]}" "$host" "setsid nohup bash -c 'bash \"\$0\" \"\$@\"; echo \$? > \"\$0.rc\"' $script $unit $bin $stamp > $log 2>&1 < /dev/null &" \
    || { echo "could not start the install on $host" >&2; return 1; }
  rc=
  for i in $(seq 1 90); do   # up to 7.5 min at 5 s
    sleep 5
    rc=$(ssh "${ssh_opts[@]}" "$host" "cat $script.rc 2>/dev/null" || true)
    [ -z "$rc" ] || break
  done
  local out
  out=$(ssh "${ssh_opts[@]}" "$host" "cat $log" || true)
  printf '%s\n' "$out" | sed 's/^/   [remote] /'
  # The backup exists once the remote says so; only then is a rollback valid.
  if grep -q '^BACKUP ' <<<"$out"; then
    rollbacks+=("ssh $host 'install -m755 $bin.bak-$stamp $bin && systemctl restart $unit'")
  fi
  if [ -z "$rc" ]; then
    echo "no result from $host after 7.5 min; the install is still running or ssh is down. Check: ssh $host 'cat $log; systemctl is-active $unit'" >&2
    return 1
  fi
  ssh "${ssh_opts[@]}" "$host" "rm -f $script $script.rc $log" || true
  [ "$rc" = 0 ] || { echo "install on $host failed (exit $rc); see the remote lines above" >&2; return 1; }
  echo "   rollback: ssh $host 'install -m755 $bin.bak-$stamp $bin && systemctl restart $unit'"
}

# gate_scan: ZynScan's /api/summary must say "agrees": true (its replay
# reproduces the node's root); false or null is a finding. It replays before
# it listens, so refused connections are retried. Bounded by wall-clock:
# 1200 s.
gate_scan() {
  echo "== waiting for $scan_unit /api/summary agrees:true (up to 20 min)"
  ssh "${ssh_opts[@]}" "$hetzner" bash -s -- "$scan_unit" <<'REMOTE'
set -u
unit=$1
end=$((SECONDS + 1200))
while [ $SECONDS -lt $end ]; do
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

# anchored_target: the epoch the chain has anchored through, from ZynScan
# (which gate_scan has just shown to be listening). A null, missing or zero
# value is a failure, never a target of 0. Prints the number. Up to 120 s.
anchored_target() {
  ssh "${ssh_opts[@]}" "$hetzner" bash -s <<'REMOTE'
set -u
end=$((SECONDS + 120))
while [ $SECONDS -lt $end ]; do
  v=$(curl -s -m 10 127.0.0.1:8093/api/summary | python3 -c 'import json,sys; v=json.load(sys.stdin).get("anchored_through"); print(v if isinstance(v, int) and v > 0 else "")' 2>/dev/null || true)
  if [ -n "$v" ]; then echo "$v"; exit 0; fi
  sleep 5
done
echo "ZynScan reports no anchored_through (null, missing or 0): there is no anchor to verify through" >&2
exit 1
REMOTE
}

# gate_replica <target>: the replica replays from genesis; pass only when its
# log, for this run of the unit, says it verified through at least <target>
# (> 0), without DISAGREES, and the unit stays active. Bounded by wall-clock:
# 600 s.
# Wording from zynzapd/src/bin/zyn-object-replica.rs:
#   "zyn-object-replica: chain {chain} verified through epoch {through}"
#   "zyn-object-replica: DISAGREES at epoch {epoch}: ..."
gate_replica() {
  local target=$1
  [[ $target =~ ^[0-9]+$ ]] && [ "$target" -gt 0 ] || { echo "bad anchored epoch '$target'" >&2; return 1; }
  echo "== waiting for $replica_unit to verify through epoch $target (up to 10 min)"
  ssh "${ssh_opts[@]}" "$contabo" bash -s -- "$replica_unit" "$target" <<'REMOTE'
set -u
unit=$1 target=$2
id=$(systemctl show -p InvocationID --value "$unit")
[ -n "$id" ] || { echo "no invocation id for $unit" >&2; exit 1; }
end=$((SECONDS + 600))
while [ $SECONDS -lt $end ]; do
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

cd "$(dirname "$0")/.."

step "0/3 build zyn-object-node + zynscan (aarch64) and zyn-object-replica (x86_64) locally"
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p zynzapd --bin zyn-object-node --bin zynscan
cargo zigbuild --release --target x86_64-unknown-linux-gnu -p zynzapd --bin zyn-object-replica

step "1/3 verifiers: Contabo $replica_unit (x86_64), then Hetzner $scan_unit (aarch64); the sequencer waits for both gates"
install_on "$contabo" "$replica_unit" "$replica_bin" "$x86/zyn-object-replica"
install_on "$hetzner" "$scan_unit" "$scan_bin" "$arm/zynscan"
gate_scan || { echo "ZynScan gate failed; the sequencer is NOT touched" >&2; exit 1; }
target=$(anchored_target) || { echo "no anchor target; the sequencer is NOT touched" >&2; exit 1; }
gate_replica "$target" || { echo "replica gate failed; the sequencer is NOT touched" >&2; exit 1; }

step "2/3 signers updated (per RUNBOOK custody section, §4v): the custodians run zyn-custodian, which never runs the ledger, so pay ops do not reach them and nothing is installed here. Confirm none of them runs a zyn-object node or replica"

step "3/3 sequencer: $node_unit on Hetzner (starts the pay clock once the chain has pay state)"
install_on "$hetzner" "$node_unit" "$node_bin" "$arm/zyn-object-node"
ssh "${ssh_opts[@]}" "$hetzner" "journalctl -u $node_unit -n 15 --no-pager"

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
