#!/usr/bin/env bash
# run_rung.sh <name> <ok-regex> <fail-regex> [net]
# Boots $S/img/<name>.esp.img under QEMU, releases the keypress gate via the monitor,
# waits for the token, writes $S/logs/<name>.serial.log (+ .host.log for net rungs).
# Exit 0 iff the OK token appeared. ACCEL=kvm|tcg (default tcg: what the baseline proved on).
set -u
S=$(cd "$(dirname "$0")" && pwd)
name=$1; ok_re=$2; fail_re=$3; net=${4:-}
ACCEL=${ACCEL:-tcg}
TIMEOUT=${TIMEOUT:-180}
HOSTPORT=${HOSTPORT:-45123}
mkdir -p "$S/logs" "$S/run"
SER=$S/logs/$name.serial.log
HLOG=$S/logs/$name.host.log
MON=/tmp/claude-1000/mx-$name-$$.sock
VARS=$S/run/$name.vars.fd
: > "$SER"; rm -f "$MON"
cp -f /usr/share/OVMF/OVMF_VARS_4M.fd "$VARS"
netargs=(-net none)
if [ -n "$net" ]; then
  netargs=(-netdev "user,id=n0,hostfwd=tcp:127.0.0.1:${HOSTPORT}-:7000" -device e1000,netdev=n0,mac=52:54:00:12:34:56)
fi
qemu-system-x86_64 \
  -accel "$ACCEL" -machine q35,i8042=on,usb=off -cpu max -smp 4 -m 2G \
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,unit=1,file="$VARS" \
  -drive file="$S/img/$name.esp.img",format=raw,if=none,id=esp \
  -device virtio-blk-pci,drive=esp,bus=pcie.0,addr=0x04,bootindex=1 \
  "${netargs[@]}" \
  -display none -no-reboot -no-shutdown \
  -serial "file:$SER" -monitor "unix:$MON,server,nowait" &
QPID=$!
cleanup() { kill $QPID 2>/dev/null; sleep 1; kill -9 $QPID 2>/dev/null; rm -f "$MON"; }
trap cleanup EXIT

gate=0
for i in $(seq 1 "$TIMEOUT"); do
  grep -qi 'press any key' "$SER" 2>/dev/null && { gate=1; break; }
  kill -0 $QPID 2>/dev/null || { echo "[$name] qemu exited before gate"; break; }
  sleep 1
done
echo "[$name] gate_seen=$gate after ${i}s"
[ "$gate" = 1 ] || { tail -n 30 "$SER"; exit 1; }
for k in 1 2 3; do printf 'sendkey ret\n' | nc -U -q1 "$MON" >/dev/null 2>&1; sleep 1; done

HPID=
if [ -n "$net" ]; then
  : > "$HLOG"
  lis=0
  for i in $(seq 1 "$TIMEOUT"); do
    grep -q 'MORPH_NET_LISTENING' "$SER" 2>/dev/null && { lis=1; break; }
    grep -Eq "$fail_re" "$SER" 2>/dev/null && break
    kill -0 $QPID 2>/dev/null || break
    sleep 1
  done
  echo "[$name] listening_seen=$lis after ${i}s"
  python3 "${HOSTCLIENT:-$S/ws/net/hostclient.py}" "$HOSTPORT" >"$HLOG" 2>&1 &
  HPID=$!
fi

ok=0
for i in $(seq 1 "$TIMEOUT"); do
  grep -Eq "$ok_re" "$SER" 2>/dev/null && { ok=1; break; }
  grep -Eq "$fail_re|exited code=" "$SER" 2>/dev/null && break
  kill -0 $QPID 2>/dev/null || { echo "[$name] qemu exited"; break; }
  sleep 1
done
[ -n "$HPID" ] && wait $HPID 2>/dev/null
sleep 2
echo "[$name] ok=$ok after ${i}s"
if [ "$ok" = 1 ]; then echo "[$name] PASS: $(grep -Eo "$ok_re" "$SER" | head -1)"; else echo "[$name] FAIL"; tail -n 40 "$SER"; fi
[ -n "$net" ] && { echo "[$name] host: $(cat "$HLOG")"; }
[ "$ok" = 1 ]
