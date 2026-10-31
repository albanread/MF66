#!/bin/sh
# deskbox.sh -- an Arm box in the desktop under QEMU (the toolchain's own),
# headless, on a box slot of our own, for A7232ToolChain's
# tests/lib/deskdrive.py to drive (its attach mode: DIR/pid names QEMU's
# pid and QMP socket).
#
#   ROSGD=<checkout>/rosgd sh deskbox.sh start DIR SHARE
#   python3 $ROSGD/tests/lib/deskdrive.py DIR shot a click 100 200 ...
#   sh deskbox.sh stop DIR
#
# run/run-qemu.sh boots into the desktop only with a window; a wrapper
# runs QEMU with -display none in place of the Metal window, so nothing
# opens on the Mac's screen, and adds the QMP socket.  The screen is read
# by deskdrive from the box's memory.
set -eu
ROSGD=${ROSGD:-/Volumes/xc/BOX/src/A7232ToolChain-forth/rosgd}
QEMU_BIN=${QEMU_BIN:-/Volumes/xc/BOX/src/A7232ToolChain/qemu/build-rosgd/qemu-system-aarch64}
cmd=$1 dir=$2
case $cmd in
start)
    share=$3
    mkdir -p "$dir/shots"
    dir=$(cd "$dir" && pwd)
    eval "$("$ROSGD/tools/boxslot.sh" take --wait --note "MF66 deskbox")"
    echo "$BOX_SLOT" > "$dir/slot"
    cat > "$dir/qemu-wrapper.sh" <<WRAP
#!/bin/sh
# run-qemu.sh's QEMU, headless, with a QMP socket
n=\$#; i=0; prev=
for a in "\$@"; do
    i=\$((i + 1))
    if [ "\$prev" = -display ]; then a=none; fi
    prev=\$a
    set -- "\$@" "\$a"
done
shift \$n
exec '$QEMU_BIN' "\$@" -qmp unix:qmp.sock,server=on,wait=off
WRAP
    chmod +x "$dir/qemu-wrapper.sh"
    rm -f "$dir/qmp.sock"
    ( cd "$dir" && ROSGD_QEMU="$dir/qemu-wrapper.sh" ROSGD_SHARE="$share" ROSGD_SOUND=none \
        ROSGD_SSH_PORT=$ROSGD_SSH_PORT ROSGD_SMB_PORT=$ROSGD_SMB_PORT \
        exec sh "$ROSGD/run/run-qemu.sh" </dev/null >"$dir/serial.log" 2>&1 ) &
    pid=$!
    echo "$pid qmp.sock" > "$dir/pid"
    for i in $(seq 60); do [ -S "$dir/qmp.sock" ] && break; sleep 1; done
    echo "deskbox: slot $BOX_SLOT, pid $pid, $dir"
    ;;
stop)
    dir=$(cd "$dir" && pwd)
    python3 "$ROSGD/tests/lib/deskdrive.py" stop "$dir" 2>/dev/null || true
    pid=$(cut -d' ' -f1 "$dir/pid" 2>/dev/null || true)
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
    [ -f "$dir/slot" ] && "$ROSGD/tools/boxslot.sh" release "$(cat "$dir/slot")" && rm -f "$dir/slot"
    ;;
esac
