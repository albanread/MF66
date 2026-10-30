# MF66 on the BOX

This is the plan for porting MF66, the token-IR optimizing Forth, to the
BOX, our 64-bit RISC OS translated to C over Linux (A7232ToolChain/rosgd).
The result is `!F66`, a RISC OS application whose Forth runs source files
and an interactive REPL, calls RISC OS SWIs, and can open Declaritive
windows. The Cocoa IDE (mf66ide), macide, rust-tcl, the GUI binaries and
everything Windows are left out.

Written 30 Oct 2026 from the code of MF66 (cli/forth-box e839dcb), JASM
(main 0434afb) and A7232ToolChain (fasterbasic 552b32594d), following the
FasterBASIC port of the same day (FasterBASIC's design/BOX_PORT.md).

## 1. Where the Forth runs

The box gives native code three places to live (FasterBASIC's
BOX_PORT.md, section 1): inside `/init` (64-bit, `ros_swi` directly), in
the arena (32-bit pointers, A64X32, the SWI gate), or in a separate Linux
process (no SWIs).

MF66 is AArch64 code with 64-bit cells: TOS in x0, DSP x19, UP x20, every
address a full 64-bit pointer, the kernel and the optimizer's lowering
built on that. Moving it to the arena would mean an ILP32 Forth, a
different kernel; a separate engine could not make SWIs. So **MF66 goes
inside `/init` as a native module**, as Python3 and FasterBASIC do:

- the Forth (the Rust crate minus its IDE parts) and JASM's front end and
  AArch64 encoder are cross-built by cargo for `aarch64-unknown-linux-musl`
  as one static library, `libmf66-box.a`, and linked into `/init`;
- `rosgd/modules/forth/forth.c`, on the A7232ToolChain branch `forth`, is
  the seam: the commands, the VDU, the SWI hook, the fault escape;
- a fault in Forth code (a bad `@`, a runaway `execute`) is caught by the
  thread's `ros_fault_escape` (fault.h) around every run and becomes a
  RISC OS error; the box goes on.

### One difference from FasterBASIC: the data region is in the arena

MF66 keeps all its data in one 8 MiB region: the data, return, locals and
float stacks, the user area, the dictionary's headers and data space
(docs/design/mf66-memory-map.md). Its JIT code is elsewhere, in two
executable mappings. In the box the module gives the Forth that region
from a **dynamic area** (OS_DynamicArea 0, "F66 Forth"), so it lies in the
low 4 GB. Then every address a Forth program sees for its data, `here`,
`pad`, a `create`d buffer, an `s"` string, is a RISC OS address and can be
passed to a SWI as it is. FasterBASIC needed `BLOCK` and string copies
into the RMA because its data was in `/init`'s heap above 4 GB; MF66 does
not. `allocate` goes to the RMA for the same reason. Only the code
mappings are above 4 GB, and no SWI is given code.

The session's region comes from a host hook; on the Mac it is the
`alloc_zeroed` block it always was.

## 2. What changes

### JASM (branch `box-linux-jit`, worktree `src/JASM-box-linux`)

`native_macos::MacJit` is the AArch64 loader: it maps one region RWX with
`MAP_JIT`, flips W^X per thread with `pthread_jit_write_protect_np`, and
cleans the caches with `sys_icache_invalidate`. Its relocation and veneer
code is not macOS-specific. The platform calls move into a small module,
`jitmem`, with two implementations:

- macOS: as now;
- Linux AArch64: `mmap(PROT_READ|PROT_WRITE|PROT_EXEC, MAP_PRIVATE|
  MAP_ANONYMOUS)`, no W^X toggle (Linux has none to make), and the cache
  maintenance done inline (`dc cvau` / `dsb ish` / `ic ivau` / `dsb ish` /
  `isb` over the range, line sizes from CTR_EL0), which is what
  `__clear_cache` does.

`native_macos` is compiled on both and also exported as `native_a64`.
MF66's own `CodeArena` (the second mapping, for colon bodies) uses
`jitmem` too. MF66's `Cargo.toml` points its path dependency at the
worktree while the branch is open.

### MF66 (this branch)

1. **`lib.rs`** is no longer macOS-only: `cfg(target_arch = "aarch64")`.
   `crash.rs` (a signal handler) is left out of the box build; `/init`
   owns the signals.
2. **The kernel source is built in.** `kernel_path()` reads
   `kernel/*.masm` from the checkout at run time. The box has no checkout,
   so the files are `include_str!`'d and given to the assembler with
   `add_virtual_file`; `@include` finds them there. (The Mac build can do
   the same; it costs nothing.)
3. **Output streams.** `rt_emit`/`rt_type`/`rt_print_*` append to a
   capture buffer that `eval_out` returns after the line. A long program
   must print as it goes, so when a host writer is set (the box) they
   call it instead; the module's writer is OS_WriteC (a new line by
   OS_NewLine), which reaches the console, a TaskWindow or a redirection.
4. **Input.** The kernel has no `key`/`accept`; the REPL is in the host.
   New host-call primitives `key` (OS_ReadC), `key?` (OS_Byte 152 on the
   keyboard buffer), `accept` (OS_ReadLine32) go through hooks.
5. **Files and memory.** The file words (`open-file` ... in runtime.rs)
   use Rust's std, which would reach `/init`'s Linux files, not RISC OS's.
   In the box they go through hooks to OS_Find / OS_GBPB / OS_Args, so
   `s" $.Work.Data" r/o open-file` works on RISC OS names. `allocate`,
   `free`, `resize` go to the RMA (section 1).
6. **SWIs.** A host call `rt_swi(regs, number) -> err` and the words of
   section 3, defined in a kernel file `kernel/riscos.masm` and a short
   Forth prelude, both only in the box build.
7. **The box crate** `box/` (`mf66-box`, staticlib) depends on the `mf66`
   library and exports a C API (`box/mf66_box.h`): set the hooks, boot a
   session, evaluate a line or a whole file (with `name:line:` errors),
   ask whether a definition is open, recover after a fault, end the
   session. Every entry point is wrapped in `catch_unwind`, so a Rust
   panic becomes an error and not an abort of `/init`.
8. **Build.** `box/build.sh`: `cargo build --release --target
   aarch64-unknown-linux-musl -p mf66-box`, then a partial link of the
   archive with every symbol but `mf66_box_*` made local (Rust's
   runtime symbols must not meet another Rust or C++ library in `/init`),
   into `BOX/caches/mf66/out-aarch64/libmf66-box.a` with the header.

Nothing in the optimizer, the lowering or the kernel's words changes.

## 3. SWIs from Forth

RISC OS Forths (WimpForth among them) keep a register block. MF66 does
the same:

```forth
regs                ( -- addr )      \ R0..R9, ten cells
s" OS_Write0" swi#  ( c-addr u -- n ) \ OS_SWINumberFromString
xswi                ( n -- err|0 )   \ X form: R0..R9 in and out; the V flag
                                     \ is the result: 0, or the error block
swi                 ( n -- )         \ the same, but an error is THROWn
                                     \ (-1000 - its number is not used:
                                     \ the error's text is printed, code -2)
swi-flags           ( -- addr )      \ N Z C V of the last call as bits 3..0
sys                 ( x0..xk-1 k n -- ) \ R0..Rk-1 from the stack, then swi
r0 r1 ... r7        ( -- x )         \ a register after the call
>r0 ... (as `regs !`)
z"                  ( "text" -- c-addr ) \ a NUL-terminated string
swi" OS_Write0"     ( -- n )         \ the number, looked up when compiled
```

For example:

```forth
: hello  z" Hello from a SWI" 1 swi" OS_Write0" sys ;
: ticks  0 swi" OS_ReadMonotonicTime" sys r0 ;
\ X form and the V flag:
: try   0 swi" XOS_ReadVarVal" ...  xswi ?dup if count type then ;
```

Registers go in as their low 32 bits and come out zero-extended (RISC OS
registers are 32 bits; addresses below 4 GB are then right). `s>32` and
`32>s` convert a signed value. Strings and buffers are Forth data, so
already in the arena. Escape ends a program as it prints or calls a SWI,
as FasterBASIC's do, and the module yields to a TaskWindow every 10 cs.

**Declaritive** (JSON Wimp windows, SWI chunk &C0380) needs nothing
more: a Forth program calls `Wimp_Initialise`, then `Declaritive_Create`
with its JSON as a zero-terminated string, then polls. A small library
`Library/Declaritive` in Forth wraps it, and `Examples/Greeter` opens a
window with a text field and a button.

## 4. The commands and the application

- `*F66 <file> [<args>]` loads and runs a Forth source file in the
  calling task (command line, TaskWindow, Obey); errors as
  `file:line: error: ...`; the session ends with the file unless the file
  says `quit` (then the REPL follows).
- `*F66` with no file starts the REPL: OS_ReadLine32 for each line,
  ` ok` after it, ` compiled` while a definition is open, `bye` or Escape
  on an empty line to leave. It works at the command line and in a
  TaskWindow (`!F66` opens one).
- `*F66 -e "<forth>"` evaluates one line.
- Filetype **&A77, "Forth"**, run action `*F66 %*0` (set by the module's
  init and by `!Boot`), so double-clicking a Forth file runs it in a
  TaskWindow.
- `!F66` (not `!WimpForth`, which exists): `!Boot` (F66$Dir, the
  filetype, the sprites), `!Run` (a TaskWindow running `*F66`, or the
  file given), `!Sprites` (the application icon and the &A77 file icon),
  `!Help`, `Examples` (Hello, SWIs, Sieve, Greeter), `Library`.
- One run at a time (the session state is global), as FasterBASIC's.

## 5. Steps

1. **Design** (this file). Commit.
2. **JASM Linux loader** on `box-linux-jit`; MF66 builds and passes its
   tests on the Mac with it (`cargo test`), and cross-builds for
   aarch64-unknown-linux-musl.
3. **The box crate and the module**: `libmf66-box.a`, `modules/forth`,
   the Makefile and `boot/rom_contents.c`; `*F66` runs a file that prints
   and makes SWI calls (OS_Write0, OS_ReadMonotonicTime, an X SWI with an
   error) in the Arm box under QEMU on a slot of our own.
4. **REPL, input, files, allocate**; self-test checks
   (`boot/selftest_forth.c`); `tools/build-arm-mac.sh --check` shows only
   its 4 known failures.
5. **Declaritive** library and example; **`!F66`** on the user share;
   driven in the desktop with deskdrive: the icon, double-clicking a
   Forth file, the REPL in a TaskWindow, Greeter's window.
6. **Later**: the Intel box (section 6).

## 6. The Intel box

WF66, MF66's parent, is x86-64 on JASM's x86-64 encoder (`RasmEncoder`)
and its Windows loader (`native.rs`, SEH, Win64 calling convention). For
the Intel box the Forth would be WF66, not MF66: the front end and reducer
are shared, the x86-64 kernel and lowering are WF66's. It needs:

- a Linux x86-64 loader in JASM (`jitmem` with `mmap`; no SEH; the
  `native.rs` relocation code reused);
- WF66's host calls moved from the Win64 convention (rcx, rdx, r8, r9,
  32 bytes of shadow space) to System V (rdi, rsi, rdx, rcx, r8, r9), in
  its kernel macros (its `aapcs_call` equivalent) and lowering;
- WF66's Win32 parts (console, files, `VirtualAlloc`) replaced by the same
  hooks as MF66's, and the same box crate shape built for
  x86_64-unknown-linux-musl;
- `ros_fault_escape` exists on x86-64 already.

Until then the Intel ROM has the module without the library and `*F66`
says it needs the Arm box. No Intel emulation is run on this Mac, so
that work is proved on the Intel box's own build host.

## 7. As built (30 Oct 2026)

Branches: this repository's `cli/forth-box`; JASM's `box-linux-jit`
(worktree `src/JASM-box-linux`, on top of JASM's `macos-arm64`, which has
the lexer fix MF66's kernel needs, f3efa55: origin/main does not);
A7232ToolChain's `forth` (worktree `src/A7232ToolChain-forth`, on top of
`fasterbasic`).

**Build.** `sh box/build.sh` (seconds) makes
`BOX/caches/mf66/out-aarch64/libmf66-box.a` (16 MB, unstripped of code,
debug information dropped) and `mf66_box.h`. The Makefile finds it there
(`MF66_GUEST`), links it into `/init` and builds `modules/forth` with
`ROSGD_HAVE_MF66`; `make usershare` copies `box/app/!F66` from the MF66
checkout beside the toolchain. Rust's own libunwind goes into the partial
link, since `/init` has none; everything but `mf66_box_*` is local.

**What changed from the plan, and what was found.**
- The kernel has no `immediate`/`postpone`/`parse`, its outer interpreter
  being Rust: `z"`, `swi"` and `include` are parsing words there
  (src/session.rs), the rest of the SWI words are a Forth prelude
  (box/src/prelude.fs) on two kernel primitives, `(swi)` and `(swi#)`.
- The register block is eleven cells: R0..R9 and the flags.
- `*F66 -r <file>` replays a file as REPL input (each line echoed, ` ok`
  or ` compiled`, going on after errors and faults): the REPL's own test,
  as `*Exec` cannot be used in a headless box that powers off when its
  command returns.
- Escape is seen as a program prints, calls a SWI, waits in `ms` or reads
  a key; it ends the run by the same jump as a fault (sigsetjmp in the
  module), and the session is reset as after a fault.
- The data region's dynamic area ("F66 Forth", 8 MB) is made at the first
  run and kept; each boot zeroes it.
- `!Run` starts the REPL with TaskWindow's `-quit`, so `bye` ends the task
  and the window closes without asking; a file run keeps its window.
- Escape at the REPL's prompt (OS_ReadLine32 returns with C set) leaves
  the REPL, as `bye` does; Escape while Forth code runs stops it and the
  REPL goes on.
- A Declaritive program must be its own Wimp task, not inside a
  TaskWindow: `*WimpTask F66 <F66$Dir>.Examples.Greeter`.
- The Makefile's `MF66` is the first checkout beside the toolchain with a
  `box/app` (the main checkout, `src/MF66`, has none until this is merged).
- MF66's Forth has no `again`; `begin ... 0 until` is the endless loop.

**Proved in the Arm box under QEMU** (slot A, headless):
- `*F66` on files that print, define and run words (fib 25), and call
  SWIs by name and number, with a buffer in Forth data space, the X form
  with V set, and the flags; files written and read back through
  open-file and the rest; `allocate` below 4 GB; an error with its line; a
  fault (`0 @`) ending the run with "signal 11" and the next run normal.
- `tools/build-arm-mac.sh --check`: 4 of 2022 checks failed, the known four
  (three share checks, one MimeMap); the fourteen F66 checks
  (`boot/selftest_forth.c`) pass.
- In the desktop (deskdrive on `box/deskbox.sh`, the built user share):
  `!F66` and the &A77 file with their icons; double-clicking `!F66` gives
  the REPL in a TaskWindow (a definition, a SWI, an error, a fault, then
  `include <F66$Dir>.Examples.Sieve`: 78498 primes in 1 cs); `bye`;
  double-clicking a Forth file runs it in a TaskWindow; Greeter's
  Declaritive window, its live input and tick, and Quit; Escape stopping a
  loop with the REPL going on.

**Left.**
- The Intel box (section 6): WF66 on a Linux x86-64 loader with System V
  calls. Its ROM has the module as a stub.
- A Forth editor in the desktop (FasterBASIC has one); for now `!F66` is a
  TaskWindow REPL and files are edited in StrongED or Edit.
- `key`/`key?` are not used by any example yet and are proved only by
  building; `ms` likewise.
- MF66's own test suite on the Mac: all pass but `ans_core` (it reads
  WF66's tester from a path not on this Mac) and `phase0_data_stack_idiom`
  (it clobbers x19 without saving it; it failed before this port too).
