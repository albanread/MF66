# MF66.app — the MF66 Forth IDE

A self-contained macOS app bundle: the native Cocoa **IDE** plus the **MF66 Forth
engine**. Identifies as "MF66" in the menu bar and dock.

```
MF66.app/Contents/
  MacOS/mf66ide        the IDE  (MacModula2 / Cocoa — NSSplitView, NSTextView, tabs)
  MacOS/mf66           the Forth interpreter/compiler (Rust, JIT) — used by Build & Run
  MacOS/mf66-tcl       the persistent-engine bridge — drives the CNSL console + STAT
  Resources/examples/  sample .f programs
```

**Launch:** `open MF66.app`, or drag it to `/Applications`. The IDE resolves the
engine executables relative to its own location, so the bundle is relocatable.

What it does: edit `.f` (Forth) and `.masm` (JASM) files with syntax highlighting,
**Build & Run** a file through `mf66` (errors redden + jump to the line), a live
**CNSL** terminal to a persistent Forth image (type at the prompt, multi-line `:`
definitions), and a **STAT** dashboard (data/return/float/local stacks + state).

## Rebuilding

The IDE source lives in the WindowsModula2 tree (built with `newm2-driver`); the
engine is this repo's `mf66` / `mf66-tcl`. One script rebuilds everything and
re-assembles this bundle:

```sh
/Users/oberon/claudeprojects/WindowsModula2/projects/mf66ide/make-bundle.sh
```

It builds `mf66ide`, runs `cargo build --release --features ui --bin mf66 --bin mf66-tcl`,
assembles `MF66.app` here, and ad-hoc signs each Mach-O + the bundle.
