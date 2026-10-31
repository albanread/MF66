# MF66 on the BOX (Arm)

The BOX's F66 module (A7232ToolChain rosgd/modules/forth) links this
directory's library into the Arm `/init`. design/BOX_PORT.md describes the
port; on the Intel box the same module links WF66's library, built from
WF66's box/ with this directory's header, `mf66_box.h`.

## Rebuilding the library

From the checkouts in /Volumes/xc/BOX/src (the ordinary ones, not
worktrees), on the Studio:

    cd /Volumes/xc/BOX/src
    git -C JASM pull --ff-only origin main
    git -C MF66 pull --ff-only origin main
    cd MF66 && sh box/build.sh

This makes `BOX/caches/mf66/out-aarch64/libmf66-box.a` and copies
`mf66_box.h` beside it. It needs rustup's stable cargo with the
`aarch64-unknown-linux-musl` target, and LLVM's `ld.lld` and
`llvm-objcopy` (Homebrew's llvm@22). rosgd's Makefile (`MF66_GUEST`) finds
the library there; `tools/build-arm-mac.sh` then links it into the ROM, and
`make usershare` copies `box/app/!F66` from this checkout.

`host/` (the f66-host crate) is the hooks table and the console's helpers;
WF66 uses it too, as `../MF66/host`.
