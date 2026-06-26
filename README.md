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

## Status — Phase 0 (substrate smoke test) ✅

```
$ cargo test          # 4 tests: ABI pool invariants + 3 JIT'd-AArch64 executions
```

Phase 0 proves, from the MF66 crate, that a hand-written AArch64 word assembles
through JASM, loads into `MAP_JIT` memory, flips W^X, and executes — including an
AAPCS64 host callback routed through a far-call veneer, and a DSP-relative
data-stack idiom. No WF66 code involved yet.

Next: Phase 1 (kernel macro library + `forth_main` ABI) → Phase 2 (boot the
kernel headless). See the design doc §8.
