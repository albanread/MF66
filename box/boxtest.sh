#!/bin/sh
# boxtest.sh -- run one RISC OS command in a headless Arm box under QEMU,
# on a box slot of our own (A7232ToolChain's tools/boxslot.sh), and print
# its console.
#
#   ROSGD=<checkout>/rosgd sh boxtest.sh SHARE COMMAND [TIMEOUT]
#
# COMMAND is one word (the kernel splits its command line at spaces): an
# Obey file on the share, as HostFS::Host.$.go.  The box powers off when
# it returns.  ROSGD_INITRD may name another ROM.
set -eu
share=$1 cmd=$2 limit=${3:-120}
ROSGD=${ROSGD:-/Volumes/xc/BOX/src/A7232ToolChain-forth/rosgd}
eval "$("$ROSGD/tools/boxslot.sh" take --wait --note "MF66 boxtest")"
trap '"$ROSGD/tools/boxslot.sh" release "$BOX_SLOT" >/dev/null' EXIT
log=$(mktemp -t mf66boxtest)
ROSGD_DISPLAY=none ROSGD_SOUND=none ROSGD_SHARE=$share \
    ROSGD_APPEND="rosgd.run=$cmd panic=-1" \
    sh "$ROSGD/run/run-qemu.sh" </dev/null >"$log" 2>&1 &
pid=$!
t=0
while kill -0 $pid 2>/dev/null && [ $t -lt "$limit" ]; do
    sleep 1; t=$((t + 1))
    grep -q "rosgd: power off" "$log" 2>/dev/null && break
done
sleep 1
if kill -0 $pid 2>/dev/null; then
    pkill -TERM -P $pid 2>/dev/null || true
    kill $pid 2>/dev/null || true
    [ $t -ge "$limit" ] && echo "boxtest: timed out after $limit s"
fi
wait $pid 2>/dev/null || true
cat "$log"
rm -f "$log"
