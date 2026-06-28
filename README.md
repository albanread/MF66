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

How the optimizing Forth compiler itself works — the token IR, the reduce pass,
the deferred virtual-stack lowerer, register pinning, the STC ABI, and the
`CODE … END-CODE` escape hatch — is documented in **[compiler.md](compiler.md)**.

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
[docs/porting-guide.md](docs/porting-guide.md).

**The full integer / memory / string / number primitive layer is ported and
differentially verified — corpus 147/150 PASS, 0 FAIL** (161 kernel primitives
across arith/compare/logic/stack/memory/strings/number/dict). The remaining 3
need subsystems not yet built: `float_subset`+`fractal_iter` (FP → Phase 4) and
`self` (oop). Next: the dict/find/number/parse/interpreter substrate that lights
up a live REPL + the eval corpus.

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

## Performance

MF66 holds up against optimized native code. On Apple Silicon, across a suite of
integer / memory workloads (recursive `fib`, Collatz step-sum, Sieve of Eratosthenes,
a 64-bit LCG), the JIT runs **~2.6× slower than `clang -O2`** and **~22× faster than
CPython 3.14** (geometric mean) — and on the tight 64-bit LCG inner loop it *matches*
C (1.02×).

![MF66 Forth vs C (−O2) vs CPython 3.14 — compute time per benchmark, log scale](bench/benchmarks.svg)

| benchmark | MF66 Forth | C `-O2` | CPython 3.14 | MF66 vs C | MF66 vs Python |
|---|--:|--:|--:|--:|--:|
| `fib(34)` recursive       | 19.5 ms | 8.6 ms  | 385 ms  | 2.25× slower | 19.8× faster |
| `collatz` Σ steps 1..10⁶  | 357 ms  | 93 ms   | 5366 ms | 3.83× slower | 15.0× faster |
| `sieve` < 10⁶ (π = 78498) | 4.5 ms  | 0.9 ms  | 58 ms   | 5.0× slower  | 12.8× faster |
| `lcg` ×10⁸ (64-bit)       | 100 ms  | 98 ms   | 6205 ms | **1.02× ≈ C** | 61.8× faster |

The terminal **tail-call** optimization runs a 10⁸-deep tail-recursive LCG (`tlcg`)
in O(1) return-stack space at 1.49× a `DO`/`LOOP` — bit-identical result, 41× faster
than Python's loop, where without it the return stack would overflow outright.

Methodology, the full write-up, and a one-command reproducer (`bench/run.sh`) are in
**[bench/BENCHMARKS.md](bench/BENCHMARKS.md)**: best-of-5, each language timed with its
own compute-only monotonic clock, identical results across all three, and the C/Python
ports passed an adversarial fairness audit (`cc -O2 -S` confirms the loops aren't folded).
