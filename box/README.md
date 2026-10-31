# MF66 on the BOX

The Arm box's F66 module (A7232ToolChain rosgd/modules/forth) links this
crate's static library into `/init`. design/BOX_PORT.md says how it works;
WF66's box/README.md says how the Intel box does the same with WF66.

## Rebuilding the library

From the checkouts in BOX/src (MF66, with JASM beside it):

    cd /Volumes/xc/BOX/src/MF66
    git pull --ff-only origin main
    (cd ../JASM && git pull --ff-only origin main)
    sh box/build.sh

This writes `BOX/caches/mf66/out-aarch64/libmf66-box.a` and `mf66_box.h`,
which rosgd's Makefile links when it builds the Arm box
(`tools/build-arm-mac.sh`). `MF66_BOX_OUT=<dir>` writes elsewhere (to try a
build without touching the shared cache).

`host/` (the f66-host crate) is the hooks table and the console helpers,
shared with WF66; `mf66_box.h` is the C interface both libraries export.
