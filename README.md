# MF66 — Apple Silicon token-IR optimizing Forth

MF66 is an **Apple Silicon (macOS arm64)** re-implementation of
[WF66](https://github.com/albanread/WF66), a token-IR optimizing
subroutine-threaded Forth. It is JIT-compiled through the **LLVM-free JASM
AArch64 backend** (`wfasm::a64` + `wfasm::native_macos::MacJit`) — no LLVM at
build or run time.

## Strategy — a retarget, not a rewrite

WF66 splits cleanly into an architecture-neutral front-end + token-IR reducer on
top of an x86-specific lowering/back-end. MF66 reuses the neutral half and
rebuilds the AArch64 half, standing on substrate that already exists on Apple
Silicon:

| Reused | From | Status |
|---|---|---|
| AArch64 encoder + `MAP_JIT` loader | JASM (`a64` + `native_macos`) | ✅ done |
| GC (`newgc-core`) | MacNCL | reuse |
| REPL / IDE | MacNCL | reuse |
| Forth front-end + token-IR reducer | WF66 (`src/wf66`) | reuse |

New work: the AArch64 lowering/`render()` leg of the back-end, the STC kernel
(MASM → AArch64), the runtime-substrate swaps, and the GC/IDE glue.

The full plan, the verified ABI, and the phased roadmap live in
**[docs/design/mf66-apple-silicon.md](docs/design/mf66-apple-silicon.md)**. The
raw subsystem review + adversarial critique that produced it are under
[docs/review/](docs/review/).

## ABI (decided)

`TOS=x0`, `DSP=x19`, `UP=x20`, `LP=x21`, `FTOS=d8`, `FSP=x22`; everything that
must survive a settle-barrier `rt_*` call is callee-saved. `x16`/`x17`/`x18` are
forbidden in any pool (`MacJit` veneers own x16; x18 is Darwin-reserved). Source
of truth: [`src/abi.rs`](src/abi.rs).

## Status — Phase 2 (boot headless) — in progress

```
$ cargo test          # abi / kernel_lint / phase0 / phase1 / corpus
```

The **differential corpus** (`tests/data/direct/`, 150 `.t` files imported from
WF66 — the day-one oracle) drives a workflow-based per-primitive port: translate
the WF66 x86 proc → AArch64, adversarially verify it, then the word's `.t` flips
NYIMP → PASS (PASS = matches WF66's observed behavior). See
[docs/porting-guide.md](docs/porting-guide.md). **Batch 1: 49 boot-critical
register-only primitives** (arith/compare/logic/stack) → **corpus 50/150 PASS, 0
FAIL**. Next: memory + rstack, hard arith (double-cell/division), then
dict/number/parse/interpreter.

- **Phase 0** — a hand-written AArch64 word assembles through JASM, loads into
  `MAP_JIT` memory, flips W^X, and executes, incl. an AAPCS64 host callback via a
  far-call veneer and a DSP-relative data-stack idiom.
- **Phase 1** — the kernel macro library (`kernel/macros.masm`: register homes,
  `proc`/`endp`/`next`, the AArch64 `stk` macro, `aapcs_call`) and `forth_main`
  (callee-saved save/restore, `sp`↔return-stack switch, the wire-format
  prologue/epilogue) drive real `proc(…)…endp()` primitives — `dup drop swap + 1+`
  and a host-call word — through `Mf66Session::{push,call,stack}`. A grep gate
  enforces the x18 / q8–q15 ban.

Next: Phase 2 — boot the kernel headless (the boot-critical primitive subset +
dictionary + interpreter). See the design doc §8.
