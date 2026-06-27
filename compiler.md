# The MF66 Compiler

MF66 is an **optimizing Forth compiler written in Rust**, targeting Apple Silicon
(AArch64). It compiles each colon definition to native machine code at definition
time and runs it directly — there is no bytecode and no interpreter loop in the
hot path. This document explains how the compiler is built and why.

---

## 1. The central design decision: one compiler, not two

The ancestor system, **WF66** (Windows x86-64), is structured as *two* pieces:

- a **Forth compiler** — the `:` colon compiler that turns Forth source into a
  token IR, and
- a separate **Rust optimizer** that consumes that IR and produces machine code.

Two stages, two implementation languages, a serialized IR handed across the
boundary between them.

**MF66 collapses this into a single optimizing Forth compiler, entirely in
Rust.** There is no Forth-side compilation stage and no separate "optimizer" bolted
on afterwards. The same Rust code that *recognizes* a Forth word emits the token
IR, and the reduce + lower passes that *optimize* it are integral parts of the
same pipeline — not a downstream consumer. Recognition produces an IR that is
already shaped for optimization; optimization is not an afterthought pass over a
foreign representation.

The practical consequences:

- **No IR serialization boundary.** Tokens are Rust `enum` values
  (`opt::Tok`), produced and consumed in the same process, never marshalled.
- **The front-end knows the back-end's vocabulary.** When the recognizer sees
  `+`, it emits `Tok::Bin(Add)` — an *optimizable* token — instead of a call to a
  `+` primitive. The decision of "what can be optimized" lives in one place
  (`build_vocab`), shared by recognition and lowering.
- **One place to reason about correctness.** The settle-barrier model (below) is
  enforced by the lowerer and respected by the recognizer; there is no second
  implementation to keep in sync.

What MF66 keeps from WF66: the **token IR concept**, the **subroutine-threaded
(STC) execution model**, the **deferred virtual-stack lowering** idea, and the
**register/ABI conventions**. What it drops is the two-language split.

---

## 2. The pipeline

```
Forth source
   │
   │  eval()                       outer interpret/compile loop (session.rs)
   ▼
[recognition]  compile_token()     word → Tok IR  (resolve local/var/ivar/
   │                               vocab-inline/leaf-splice/Call)
   ▼
 Tok IR  (def.toks)                a straight-line "run" of tokens
   │
   │  flush_toks()                 fires at every control-flow boundary
   ▼
[reduce]  opt::reduce()            token-level peephole rewrites
   │
   ▼
[lower]   opt::lower()             deferred virtual-stack → AArch64 words
   │
   ▼
 def.body : Vec<u32>               machine code, accumulated per definition
   │
   │  commit_body()                epilogue + patch exits
   ▼
 CodeArena.commit()                copy into MAP_JIT (W^X) memory → xt
   │
   ▼
 dict entry  (name → xt)           callable, composable like any primitive
```

Control flow (`if`/`begin`/`do`/…) is handled by `compile_control`, which emits
branches straight into `def.body` and *flushes* the pending token run first — so
the lowerer only ever sees **straight-line code** (a "run"). This is the key
simplification that makes the deferred lowerer tractable.

---

## 3. The token IR (`opt::Tok`)

The IR is a flat `enum`. It is deliberately small and close to the machine, but
abstract enough to rewrite. The main families:

| Family | Tokens | Meaning |
|---|---|---|
| Literals | `Lit(i64)`, `FLit(bits)` | push an integer / float constant |
| Integer ALU | `Bin(op)`, `ImmBin(op,k)`, `DupBin(op)` | `+ - * and or xor`; folded forms |
| Compare | `Cmp(kind)` | `= < > <= 0= 0< …` → flag |
| Select | `Sel(kind)` | `min max umin umax` (branchless cmp+csel) |
| Stack | `Stk(op)` | `dup drop swap over rot …` |
| Memory | `Mem(kind)` | `@ ! c@ c!` |
| Locals | `LocalFetch/Store`, `LocalFFetch/FStore`, `OpenLocals`, `CloseLocals` | LP-relative locals frame |
| OOP | `IvarFetch/Store`, `SelfPush` | instance variables |
| Float | `FBin`, `FUn`, `FStk`, `FFetch`, `FStore`, `FCmp` | scalar double FP on the FP stack |
| Loop | `LoopIdx(off)` | `i`/`j` — load the do-loop index from `[RP+off]` |
| Dynamic stack | `PickN`, `RollN` | `pick`/`roll` |
| Barrier | `Call(xt)` | call another word (settles both stacks) |

The crucial distinction is **`Call` vs everything else**. A `Call` is an opaque
subroutine invocation — it forces a *settle barrier* (§7). Every other token is
something the lowerer can keep in registers and reorder around. So the entire
optimization strategy reduces to: **express as much as possible as non-`Call`
tokens, and emit as few settle barriers as possible.**

`build_vocab` (in `session.rs`) is the table that maps a primitive's assembly
symbol to its inline token(s): `f_plus → [FBin(Add)]`, `less → [Cmp(Lt)]`,
`i_word → [LoopIdx(0)]`, and so on. A primitive in this table is *inlined* as IR;
one that is not becomes a `Call`.

---

## 4. Reduce — token-level peephole (`opt::reduce`)

`reduce` runs first, as a single forward pass over a run, maintaining an output
vector it can look back into. Each rewrite is a local pattern:

- **const-fold** — `Lit Lit Bin` → `Lit` (and constant `Cmp`/`Sel`).
- **immediate-fold** — `Lit Bin` → `ImmBin(op,k)` (fold a constant operand into the op).
- **immediate-chain** — `ImmBin ImmBin` → one `ImmBin` (e.g. `1+ 1+` → `+2`).
- **dup-fuse** — `Dup Bin` → `DupBin(op)`.
- **dead-code** — `Lit Drop` / `Dup Drop` annihilate.
- **stack-cancel** — `swap swap`, `rot -rot` annihilate.
- **compare-negate** — `<cmp> 0=` → the inverse compare (integer *and* FP:
  `< 0=` → `>=`, `f< 0=` → the condition-inverted `FCmp`).

Reduce is pure IR→IR. It shrinks the token count and exposes opportunities for the
lowerer, but it does not allocate registers or emit code. The per-word metrics
(`Metrics::const_folds`, `imm_folds`, …) are tallied here.

---

## 5. Lower — the deferred virtual-stack model (`opt::Low`)

This is the heart of the compiler, and the reason MF66 is not "every word a
push/pop through memory."

A naive STC Forth keeps the data stack in memory: each operation loads its
operands, computes, stores the result. MF66 instead keeps a **window of the stack
in registers** and only touches memory at boundaries.

### The virtual stacks

`Low` holds two virtual stacks:

```rust
vs:  Vec<Loc>     // data stack window;  Loc = Const(i64) | Reg(u32)
fvs: Vec<u32>     // FP stack window;    d-registers
```

A `Loc` is either a **constant not yet materialized** or a **register holding a
live value**. The window models the *top* of the Forth stack; everything below it
lives in memory at `[DSP]` downward.

### Deferral

Operations manipulate the virtual stacks and emit code *lazily*:

- **Constants are deferred.** `Lit(5)` just pushes `Loc::Const(5)`. The `5` is
  materialized into a register only when something actually consumes it as a
  register operand — and often it never is (it folds into an immediate, or into
  another constant).
- **Stack motion is free.** `dup`, `swap`, `over`, `rot` are *re-indexing of `vs`*,
  not instructions. `swap` exchanges two `Vec` entries; no code is emitted.
- **ALU ops reuse registers.** `Bin(Add)` pops two `Loc`s; if one is a small
  constant it becomes an `add Xd,Xn,#imm`; otherwise it allocates from the pool
  and emits `add`. The result stays in a register on `vs`.

### Register pools

```
GP pool:  x9 … x15        (POOL)        — data-stack window + scratch
FP pool:  d9 … d15        (FPOOL)       — FP-stack window + scratch
```

`alloc`/`falloc` hand out a free pool register; `reserve(n)` guarantees `n` free
(spilling the window to memory if the pool is exhausted — counted as a `spill`).
The ABI's fixed registers are off-limits: TOS=`x0`, DSP=`x19`, UP=`x20`,
LP=`x21`, FSP=`x22`, RP=`x28`, FTOS=`d8`; `x16`/`x17` are veneer scratch, `x18`
is forbidden.

### Settle

At a **settle**, the virtual stacks are written back to the canonical Forth
layout: the data top in `x0`, the rest stored to `[DSP]` downward (`DSP`
adjusted by one `add`/`sub`), `vs` reset to `[Reg(0)]`; the FP top in `d8`, the
rest spilled to the FP memory stack. A settle happens:

- **before every `Call`** — the callee reads/writes the in-memory Forth stacks,
  so the window must be made real first, and
- **at the end of every run** — the next run starts from a clean canonical state.

`settle()` = `settle_data()` + `fsettle()`. This is where deferred constants get
materialized, deferred stores get flushed, and the `add DSP, …` bookkeeping is
emitted. The whole optimization payoff is **doing real work between settles and
emitting as few of them as possible.**

### The "run"

`lower()` is called on one **run** at a time — the straight-line span between two
control-flow boundaries. A fresh `Low` is created per run, so the virtual stacks
and caches are per-run; control-flow boundaries (`flush_toks`) settle and reset.
This is what keeps the lowerer simple: it never has to reason about branches, only
about a linear token sequence with a canonical state at each end.

---

## 6. Register residency: caching and pinning

The deferred window keeps values in registers *within* a run. Two further
mechanisms extend that residency.

### Within-run caches

Locals and `fvariable`s read repeatedly in a run are kept resident:

- `lcache` / `hot_locals` — integer locals (a stored or ≥2×-read local stays in a
  pool register; a cold single-read loads on demand, avoiding pessimization).
- `fvcache` / `hot_fvars` — `fvariable`s (a folded absolute address read ≥2×).
- `flcache` / `hot_flocals` — float locals.

A pre-scan in `lower()` marks the "hot" set; reads of a hot value become a `mov`
from its cache register instead of a frame `ldr`. Caches spill at settle barriers
(their pool registers don't survive a call anyway).

### Cross-iteration pinning

Caches are *per-run*, so they reset at every loop back-edge — a loop-carried local
(Mandelbrot's `zx`/`zy`) would reload each iteration. **Pinning** fixes this: in a
**call-free** loop, the float locals are pinned to fixed `d`-registers
(`fpins → d9…`) and integer locals to high GP registers (`ipins → x15, x14`,
chosen to dodge the do-loop counter scratch `x9–x12`) for the loop's whole
duration:

- a one-time **load preamble** before the loop,
- reserved registers carried across every run of the loop body (re-reserved after
  each settle),
- reads/writes become `fmov`/`mov` instead of frame traffic,
- a one-time **spill epilogue** where the loop exits.

The gate (`pin_safety`) classifies each body word as `Full` (no call — both float
and int pins survive), `FpOnly` (an FP-preserving libm call such as `fsin`, whose
wrapper preserves the callee-saved `d8–d15` but clobbers GP scratch — float pins
survive, int pins do not), or `Barrier` (anything else — no pinning). Pinning is
only sound where the pinned registers genuinely survive every operation in the
loop.

---

## 7. The STC ABI and why `Call` is the cost

MF66 is **subroutine-threaded**: every word is entered with `bl`/`blr` and ends
with `ret`. The Forth machine state is a fixed register convention (defined in
`kernel/macros.masm`):

```
x0  TOS    top of data stack (always in a register)
x19 DSP    data stack pointer (points at NOS, grows down by cell=8)
x20 UP     user-area base
x21 LP     locals-frame pointer
x22 FSP    float stack pointer
x28 RP     return / loop stack (NOT the C sp — bl puts the return addr in x30)
d8  FTOS   float top of stack (callee-saved low 64)
```

`DSP/UP/LP/FSP/RP/FTOS` are all AAPCS64 **callee-saved**, so they survive calls
into Rust runtime functions without spilling. The pool registers (`x9–x15`,
`d9–d15`) are **caller-saved scratch** — which is exactly why a `Call` forces a
settle: the window can't survive the call, so it must be made canonical (in memory
+ TOS/FTOS) before, and rebuilt after.

So the gap between MF66 and a heavyweight optimizer is *structural*, not a missing
pass: it is the per-call settle, the `fmov`/`mov` copies when reading pinned
locals, and the do-loop counter overhead — the price of the STC model and the
two-stack discipline. The optimizer's job is to minimize how often that price is
paid; it cannot make a `Call` free.

---

## 8. Inlining

A colon word whose body is a **straight-line leaf** (no control flow, under a size
cap) is captured as its reduced token IR in `inline_words`. When such a word is
later used, the recognizer *splices* its tokens into the caller's run instead of
emitting a `Call`. This both removes the call/settle and lets the spliced tokens
participate in the caller's reduce + lower (further folding, residency, pinning).

Locals frames inline too: `OpenLocals`/`CloseLocals` are balanced IR tokens, so a
leaf word that uses locals can be spliced as a nested sub-frame. This is how the
FP compares (`f<=` = `fswap f< 0=`) become pure vstack leaves and disappear into
their callers with no call overhead.

---

## 9. The backend: encoders and the code arena

- **`aenc.rs`** — hand-written AArch64 instruction encoders (one Rust `fn` per
  instruction form: `add_reg`, `ldr_off`, `fmul`, `fcmp`, `csetm`, …). Every
  encoder is **oracle-verified** against `llvm-mc` in unit tests, so the bytes are
  known-correct.
- **`codearena.rs`** — a `MAP_JIT` (W^X) executable region. `commit(&[u32])`
  flips the thread to writable, copies the words in, flips back to executable, and
  invalidates the icache — one W^X cycle per definition. Colon-word bodies and
  `CODE`-word bodies both live here, and `forth_main(xt, …)` enters either by
  address.

---

## 10. The escape hatch: `CODE … END-CODE`

Because MF66 is, underneath, an assembler (JASM/`wfasm`) with a Forth on top, it
exposes the classic Forth escape hatch for the hot 3% the optimizer can't reach:

```forth
CODE xs-asm
    mov  x10, TOS
    mov  x9, #1
.lp:
    eor  x9, x9, x9, lsl #13
    eor  x9, x9, x9, lsr #7
    eor  x9, x9, x9, lsl #17
    subs x10, x10, #1
    b.ne .lp
    and  TOS, x9, #0x0FFFFFFF
    next()
END-CODE
```

`define_code` assembles the body in the kernel's macro/register convention
(`kernel/macros.masm` is prepended), encodes it with `wfasm::a64::assemble`, and
commits the **self-contained** machine code into the `CodeArena`, bound with a
normal dictionary entry. Only leaf code is accepted: if the encoder reports any
relocations or externs (a `bl` to a runtime function or another word), it is
rejected — keeping the committed bytes position-independent. A `CODE` word is a
`Call` from the optimizer's view, so it composes exactly like any primitive.

---

## 11. Performance characteristics

Measured on Apple Silicon (`tests/timing.rs`, `tests/codebench.rs`), ns per
work-unit, cross-checked bit-for-bit against the same algorithm in clang and
CPython:

| kernel | MF66 | clang -O2 | CPython | notes |
|---|---|---|---|---|
| logistic (pinned FP) | 3.0 | 1.8 | 23 | latency-bound recurrence |
| rot2 (2 pinned FP locals) | 2.5 | 1.5 | 47 | latency-bound |
| Mandelbrot grid | 38 | 11 | 990 | per-pixel call + int↔float conv |
| prime sieve | 35 | 15 | 540 | `mod` is a Call; divider-bound |
| factorial (XOR) | 10 | 3 | 290 | per-call STC overhead |
| xorshift64 (`CODE`) | **1.35** | 1.6 | 135 | hand-written assembly |

The shape of the result: **pinned, call-free FP is within ~1.5–1.7× of clang
-O2; call-heavy integer code is 2.4–3.8×; everything is 8–25× faster than
CPython** (except libm-bound kernels, where all three converge on the same `sin`).
The remaining gap to clang is the structural STC cost of §7, not a peephole — and
where it matters, a `CODE` word reaches (or beats) clang directly.

A note from the experiments behind these numbers: the "obvious" levers are not
always wins. Inlining `i`/`j` (removing a settle barrier) helped; inlining `mod`
(`sdiv`) *hurt* the divider-bound prime sieve because removing the call exposed
the non-pipelined divider's latency on the loop's branch-critical path; FMA fusion
was neutral-to-negative because the hot FP loops are latency-bound dependency
chains, not throughput-bound. The compiler optimizes by **removing settle
barriers**, which is where the leverage actually is.

---

## 12. A worked example

`: sq dup * ;`

```
recognize :  Tok::Stk(Dup), Tok::Bin(Mul)
reduce    :  Dup Bin → DupBin(Mul)              ; dup-fuse
lower     :  vs starts [Reg(0)]  (TOS = x0, the input)
             DupBin(Mul): square the top in place → mul x0, x0, x0
             (no settle inside — straight-line leaf)
commit    :  epilogue = ret  ;  body = [ mul x0,x0,x0 ; ret ]
```

`sq` is a one-instruction leaf (`mul x0,x0,x0; ret`), and being a straight-line
leaf it is also captured in `inline_words` — so `: quad sq sq ;` splices to
`mul x0,x0,x0; mul x0,x0,x0` with no calls at all.

---

## 13. File map

| File | Role |
|---|---|
| `src/session.rs` | `eval` (interpret/compile loop), recognition (`compile_token`), control flow (`compile_control`), `flush_toks`, `commit_body`, `build_vocab`, `define_code`, the dictionary/ABI plumbing |
| `src/opt.rs` | the optimizer: `Tok` IR, `reduce`, the `Low` deferred lowerer (`lower`), caches, pinning, `Metrics` |
| `src/aenc.rs` | AArch64 instruction encoders (oracle-verified vs `llvm-mc`) |
| `src/codearena.rs` | `MAP_JIT` W^X executable arena (`commit`) |
| `src/jit.rs` | the boot loader (`MacJit`) that places the assembled kernel |
| `kernel/*.masm` | the primitive dictionary (each word a `proc … endp` JASM block) and `macros.masm` (the register/macro convention) |
| `tests/timing.rs`, `tests/codebench.rs` | the performance harnesses |
