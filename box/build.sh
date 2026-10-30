#!/bin/sh
# build.sh -- MF66 for the BOX: libmf66-box.a, which rosgd's Makefile links
# into /init (A7232ToolChain branch forth, modules/forth).
#
#   sh box/build.sh
#
# cargo builds the box crate (the Forth, JASM's front end and AArch64 encoder,
# Rust's std) for aarch64-unknown-linux-musl as a static library. Its objects
# are then linked into one (ld.lld -r) and every symbol but the C API's
# (mf66_box_*) made local, so Rust's runtime and unwinder never meet another
# library's in /init. Debug information is dropped.
#
# Output: BOX/caches/mf66/out-aarch64/libmf66-box.a and mf66_box.h.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)              # BOX
out=${MF66_BOX_OUT:-$root/caches/mf66/out-aarch64}
target=aarch64-unknown-linux-musl
OBJCOPY=${OBJCOPY:-$(ls /opt/homebrew/opt/llvm@22/bin/llvm-objcopy 2>/dev/null || command -v llvm-objcopy)}
LLD=${LLD:-$(command -v ld.lld)}
AR=${AR:-$(dirname "$OBJCOPY")/llvm-ar}
[ -x "$AR" ] || AR=ar

cd "$here"
cargo build --release --target $target
lib="$here/target/$target/release/libmf66_box.a"

mkdir -p "$out/obj"
rm -f "$out/obj/"*.o
echo "link: libmf66-box.a"
# Rust's own libunwind goes in too (made local with the rest): /init has none.
unwind="$(rustc --print sysroot)/lib/rustlib/$target/lib/self-contained/libunwind.a"
"$LLD" -r -o "$out/obj/mf66_all.o" --whole-archive "$lib" --no-whole-archive "$unwind"
printf '%s\n' mf66_box_set_hooks mf66_box_boot mf66_box_end mf66_box_running \
    mf66_box_line mf66_box_eval mf66_box_compiling mf66_box_recover >"$out/obj/exports.txt"
"$OBJCOPY" --strip-debug --keep-global-symbols="$out/obj/exports.txt" \
    "$out/obj/mf66_all.o" "$out/obj/mf66_box.o"
rm -f "$out/libmf66-box.a" "$out/obj/mf66_all.o"
"$AR" rcs "$out/libmf66-box.a" "$out/obj/mf66_box.o"
cp "$here/mf66_box.h" "$out/mf66_box.h"
ls -l "$out/libmf66-box.a"
