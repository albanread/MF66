# MF66 — Apple Silicon (macOS arm64) Port Design

Status: design / planning. The authoritative plan for re-implementing **WF66**
(Windows x86-64 token-IR optimizing STC Forth) on Apple Silicon as **MF66**.

Synthesized from a full subsystem review + adversarial critique, verified against
the real substrate: `JASM/rust/src/a64/` (AArch64 encoder) and
`JASM/rust/src/native_macos.rs` (`MacJit`), `locus/vendor/newgc-core/src/`
(the GC), and `MacNCL/src/` (the REPL/IDE). Where the review and the critique
disagreed, **the critique's corrections are taken as authoritative** and called
out inline as ⚠.

---

## 1. Strategy — a retarget, not a rewrite

WF66 was built as a two-level engine: an **architecture-neutral Forth front-end
+ token-IR reducer** on top of an **x86-specific lowering + deferred-assembly
back-end**. The port cut falls exactly on that seam. The supporting
infrastructure already exists on Apple Silicon in sibling projects.

### Reused unchanged
- **WF66 front-end + token-IR reducer** (`src/wf66/mod.rs`): `Token` (`:475-567`),
  `IrBuilder` (`:574-695`), `reduce`/`reduce_tail` (`:1177-1259`), `const_fold`
  (`:731-750`), control-flow FSM (`CtlFrame` `:440`). Operates on tokens, never asm.
- **The deferred-assembly *algorithms*** — `coalesce_dsp` (`:1486-1528`),
  `window_fuse`/`fuse_window` (`:1557-1767`), `promote_hot_cells` (`:1780-1864`):
  permutation/liveness logic, register-name-agnostic once `Instr` carries
  abstract names. ⚠ but their *cost thresholds* are not arch-neutral — see §4.
- **JASM AArch64 backend** — `wfasm::a64::assemble` + `MacJit` (done; from the
  JASM Apple Silicon port). This is the LLVM-free backend WF66 already uses via
  `default-features=false`.
- **GC** — `newgc-core` from MacNCL (`PageHeap`, `collect_{minor,major,full}`).
  Already arm64-darwin clean. (WF66's `../NewGC` path dep is absent; source it
  from MacNCL.)
- **REPL/IDE** — MacNCL's: `Session::eval(&str)->Result<String,EvalError>`,
  `output::begin_capture/end_capture`, `Ide::handle_event`, Cocoa + Core
  Graphics/Text via a platform-neutral `SurfaceCmd` IR. WF66's Windows Direct2D
  `igui`/`newfactor` is **not ported**.

### What MF66 actually builds
(1) the AArch64 lowering+render leg of the back-end; (2) the kernel rewrite
(13.6k MASM lines → AArch64, the dominant effort); (3) runtime-substrate swaps
(loader, memory, crash handler, pin codegen); (4) GC + IDE glue.

---

## 2. The MF66 ABI — AArch64 register homes

AAPCS64 facts: callee-saved `x19–x28`, `v8–v15` (low 64 only); caller-saved
`x0–x17`, `v0–v7`/`v16–v31`; `x16`/`x17` = IP0/IP1 intra-procedure scratch;
**`x18` = platform register, reserved on Apple — never touch**; `x29`/`x30` =
FP/LR.

| Role | WF66 (x86) | MF66 (AArch64) | Class | Survives a call? |
|---|---|---|---|---|
| **TOS** | rax | **x0** | caller | no (it's arg0/ret0 — spilled across calls, as x86 did) |
| **DSP** | rbp | **x19** | callee | yes |
| **UP** | rbx | **x20** | callee | yes |
| **LP** (locals) | r15 (caller!) | **x21** | callee | yes (upgrade — removes the x86 r15-clobber hazard) |
| **FTOS** | xmm15 | **d8** | callee (low 64) | yes |
| **FSP** | `[rbx+FSP]` | **x22** (or memory) | callee | yes |
| **fusion scratch** | rsi/rdi/r8/r9/rcx/rdx | **x9–x15** (7) | caller | no (window-local) |
| **GP promotion** | r10/r11 (caller!) | **x23,x24** | callee | yes (upgrade — survives barriers unconditionally) |
| **FP promotion** ⚠ | — | **d9–d15** | callee (low 64) | yes (the critique's add — FP-heavy windows need it) |
| return stack | rsp | **sp** | — | STC native call/ret |
| FP scratch | xmm0/1 | **d0–d7** | caller | no |

**Reserved/forbidden (⚠ from the critique):**
- **`x16`/`x17` never appear in any allocatable pool.** `MacJit`'s far-call
  veneers emit `movz/movk x16; br x16` at relocation time
  (`native_macos.rs:60-67,200-213`), so x16 is clobbered by *any* `bl`/`blr`
  including loader-inserted veneers. "guaranteed free at call sites" is wrong;
  treat x16/x17 as **dead across every control transfer**. x16 is fine only as a
  throwaway immediately before `blr`.
- **`x18`** — Apple platform register. There is **no encoder/toolchain guard**
  (`a64/encode.rs` will happily emit it), so add a **CI grep gate** that fails if
  `x18`/`w18` (or 128-bit `q8`–`q15`, see below) appears in any kernel `.masm` or
  `render()` output, and assert x18 is absent from every `RegFile` pool.
- **`d8`–`d15`: only the low 64 bits are callee-saved.** No 128-bit `q8`–`q15`
  use anywhere (e.g. a NEON `cnt`/vector op allocating v8) or FTOS corrupts.
- **`x29` is unusable as a frame/unwind anchor in STC code** (no C frame per
  Forth word); the crash handler (§6) must walk the Forth return stack manually.

**Darwin call-out rule (⚠):** the Forth return stack on `sp` is frequently at
8-mod-16, but AAPCS64 requires `sp` 16-byte aligned at every `bl`/`blr`.

`sp` alignment is a **framing concern, optimized like `coalesce_dsp`** — track it
as data, guarantee it, emit fix-ups only where they are actually needed:

- **MVP / correctness first:** `aapcs_call` re-aligns defensively (save sp, `and
  sp,sp,#-16`, call, restore) so it is *always* correct regardless of caller
  parity. This is the Phase-1 implementation. Never call out with `sp` misaligned.
- **Optimized (Phase 5+):** the return stack only moves by *statically known*
  cell deltas (each `>r`/`r>`/nest/unnest is ±8, every word entry/exit is a fixed
  frame). So the compiler can carry an **`sp`-alignment parity** through the IR
  exactly as `coalesce_dsp` carries the running DSP delta, and **elide the
  realign at any call site where `sp` is provably 16-aligned** (e.g. an even
  number of return-stack pushes since the last known-aligned point, or a word
  that maintains a 16-aligned frame). The realign is emitted **only** at sites
  where parity is unknown or odd — minimal fix-ups, never blanket overhead, while
  the invariant (aligned at every `bl`) is preserved by construction.

This makes alignment a tracked property of the deferred-assembly buffer rather
than a per-call tax: the same "defer, track the delta, emit the minimal
adjustment at the window edge" discipline `coalesce_dsp`/`window_fuse` already
use. (Open: model alignment parity as a 1-bit lattice on the `Instr` stream, with
calls/labels/Raw spans as the points where unknown parity forces a realign.)

Why this mapping: everything that must survive a settle-barrier rt_* call (DSP,
UP, LP, FTOS, FSP, both promotion pools) is **callee-saved**, so the
`coalesce_dsp`/`promote_hot_cells` soundness conditions hold with no spill code;
TOS↔x0 unifies the Forth value reg with the C arg/ret reg.

---

## 3. Compiler back-end retarget (`src/wf66/mod.rs`)

**Decision:** `lower()` emits an arch-neutral `Instr` AST directly (today it
emits Intel text re-parsed by `parse_instrs`). `render()` becomes the only
arch-specific text generator; `parse_instrs` is kept only for the x86 round-trip
test and for delimiting opaque `Raw` spans.

- **`ArchReg` enum + per-arch `RegFile`** replace every baked register string
  (`mod.rs:22-24,29`; `Instr` enum `:1280-1302`; fusion pool `:1682`; promotion
  pool `:1780`).
- **`render()` per-`Instr`** (`:1411-1442`): `AdjustDsp`→`add/sub x19,#n`
  (decompose if `>4095`); `LoadCell`→`ldr [x19,#disp]` (decompose out-of-range);
  `CellAlu` (x86 memory-operand ALU, one insn) → **`ldr xT; <op>`** (two insns —
  the one real code-quality regression; no AArch64 memory-ALU).
- **emit retargets:** literals → `movz`+`movk` chain; multiply → `lsl`/`mul` and
  `add x0,x0,x0,lsl #k` for ×(2^k+1); **comparisons → `csetm`** (Forth −1/0 in
  one insn, *not* `cset`+`neg`); branches `jcc`→`b.<cc>` (drive condition codes
  off the encoder's `cond_code`, `a64/encode.rs:57`); FP `addsd…`→`fadd…`;
  indirect call → `movz/movk`→`blr` (direct `bl` when in ±128 MB).
- **⚠ adrp+add is *preferred*, not avoided.** The review said "default to
  movz/movk to preserve PIC"; that is backwards — `MacJit` resolves
  `@PAGE/@PAGEOFF` fixups post-placement (`native_macos.rs:224`), so both forms
  are loader-resolved absolutes with no PIC difference. `adrp+add` is 2 insns
  /±4 GB vs movz/movk's 4 insns; prefer it for in-range data, movz/movk only as
  the out-of-range fallback.
- **Assembler call:** `wfasm::rasm::assemble`→`wfasm::a64::assemble`; drop the
  `.intel_syntax` preamble.

---

## 4. ⚠ The deferred-assembly optimizer is under-scoped beyond register names

The critique's most important compiler correction: the passes are *not* free.

- **Fixed-width + no memory-ALU invalidates the promote/fuse *thresholds*, not
  just the register names.** A folded x86 `op reg,[rbp+disp]` is one instruction;
  on AArch64 it's `ldr`+`op` (two). The break-even for `promote_hot_cells` and
  `window_fuse` is therefore *different* — the passes can **pessimize** (emit
  more than the naïve path) while remaining "correct". Re-tune the thresholds as
  part of Phase 5, not "measure in Phase 9": a pessimizing optimizer in the
  baseline is worse than none.
- **NZCV flag liveness is not modeled in the `Instr` AST** (it never needed to be
  on x86 — flags were implicit and fused windows were arithmetic). On AArch64
  NZCV is set by `cmp`/`fcmp`/`adds` and consumed by `b.cc`/`csel`. **No pass may
  reorder a flag-setter past a flag-consumer.** Verify compare↔branch/select are
  never separated by `coalesce`/`fuse`/`promote` (the front-end compare→branch
  fusion helps, but the post-lowering AST passes are the risk).
- **`Raw` spans:** decide they're opaque blobs (barriers, no fusion across — fine)
  rather than needing an AArch64 `parse_instrs` recognizer.

---

## 5. ⚠ GC — precise moving collector vs untagged Forth stacks (the biggest mis-scope)

`newgc-core` is a **precise, moving/evacuating** collector that decides
pointer-ness by **tag bits in the word** (`RootScanner::visit(&mut Word)` →
`HeapLayout::classify(raw:u64)`, `scanner.rs:39`/`lisp_layout.rs:43`). This is
the Lisp model. **Forth data-stack cells are raw untagged 64-bit integers** — an
integer with a pointer-shaped bit pattern would be wrongly evacuated → heap
corruption; and a moving collector cannot safely update an ambiguous root.

**Therefore the GC manages the *tagged managed sub-heap only*** — the
`FloatVec`/`RefVec`/`String`/`Builder` objects reached via the `RefVec` graph and
the `HEAPPTR`/`LITERAL` regions (`runtime.rs:634-640`; the kernel's 3-bit tag
scheme: FloatVec=010, RefVec=011, String=100, Builder=101). The raw data/return
stacks are **not** precise roots. This forces a **kernel-wide invariant**: every
managed handle on a stack always carries its tag — *no raw managed pointer in a
cell ever, even transiently inside a primitive*. `classify` must return
`Immediate` for every untagged value.

This is **not 30–50 lines**; it is a correctness property spanning every kernel
primitive that touches a managed object, and must be audited kernel-wide.
WF66's existing `src/gc/layout.rs` (`Wf64Layout`) is a platform-neutral
`HeapLayout` and lifts directly; the *root precision* is the hard part, not the
layout. Roots = tagged regions + any tagged stack cells, with
`enter_native`/`leave_native` bracketing every Forth↔Rust boundary so a
collection sees a consistent snapshot.

---

## 6. Runtime substrate

| Concern | WF66 | MF66 |
|---|---|---|
| JIT loader | `NativeJit` | `MacJit` (`native_macos.rs`) |
| Regions | `VirtualAlloc2`+address-reqs | `mmap(MAP_ANON\|MAP_JIT)` for code; `mmap` RW for data; 16 KB align; no ±GB window |
| rt_* ABI | Win64 | AAPCS64 (Rust side unchanged; kernel call sites → `aapcs_call`) |
| Crash handler | VEH (`wfasm::seh`) | Mach exception server (`EXC_BAD_ACCESS`/`EXC_BAD_INSTRUCTION`) + `SIGBUS`/`SIGILL`; decode `__darwin_arm_thread_state64`; **walk the Forth return stack manually** (x29 unusable, §2); code-range registry kept |
| Register pinning | x86 byte codegen (`pin.rs:218-356`) | `emit_*_aarch64` (`ldr/str`); analysis `pin.rs:109-205` unchanged |
| ⚠ W^X | n/a | `pthread_jit_write_protect_np` is **per-thread**. Whole-runtime invariant: **MF66 is single-threaded for JIT-write.** `:`-compile and IDE `eval` must run on the *one* mutator thread; the IDE marshals eval to it (never calls `eval` on the AppKit thread). Agents (Phase 9) share that thread. |

`forth_main(u64×4)->u64` is unchanged (AAPCS64 passes args in x0–x3, ret x0).

---

## 7. Differential oracle (Mac-local, no Windows box on the critical path)

Criterion: identical **observable Forth state** (stacks, memory, output, THROW
codes) as WF66, not identical bytes.

1. **Committed corpus (primary, day-one).** Capture WF66's expected observable
   state once on x86 (ANS core suite + WF66's corpus) into a golden file; MF66
   asserts byte-identical observable state. The corpus *is* the oracle on arm64.
2. **Reducer cross-check (free).** The reducer runs natively on arm64; assert
   `reduce(tokens)` == the committed reduced stream — isolates front-end from
   lowering/kernel bugs. (Golden is frozen-on-x86 like level 1.)
3. ⚠ **Drop Wine from the critical path.** WF66 is a JIT with `VirtualAlloc2` +
   VEH; Wine emulates exactly that class unreliably. Expand the corpus from a
   *real* x86 Windows box when needed, not Wine.

⚠ **State canonicalization corrections:** compare (a) the canonical data-stack
image, (b) FP-stack image, (c) return stack, (d) **only the arch-neutral
dictionary header/data fields** (link/name/flags/data cells) — **not** the code
body or baked code addresses (the dict holds STC machine code + absolute
addresses, which differ by ISA and mmap address even after ASLR normalization),
(e) captured output, (f) THROW codes. The `opt-metrics` path (`iced-x86`,
x86-only) is gated off; metrics are diagnostic, never a gate.

---

## 8. Phased plan (smallest-first; each independently verifiable)

0. **Substrate smoke test** — hand-written AArch64 "return 42" via
   `a64::assemble`→`MacJit`→call; confirm W^X + extern. No WF66 code.
   **✅ DONE** (`tests/phase0.rs`, `src/{abi,jit}.rs`): leaf word, an AAPCS64 host
   callback through a far-call veneer, and a DSP-relative data-stack idiom all run;
   `src/abi.rs` encodes the register homes + pool invariants.
1. **Kernel macro library + ABI** (`macros.masm` K0) — register homes (§2),
   `aapcs_call` (**defensive** sp realign — always-correct; parity-elision is a
   Phase-5+ optimization, §2), `forth_main` prologue/epilogue (save x19–x28, x30,
   d8–d15). *Verify:* a 2-word kernel runs via `forth_main`.
   **✅ DONE** (`kernel/{macros,main,prims}.masm`, `src/{session,asm_macros,
   runtime}.rs`, `tests/phase1.rs`, `tests/kernel_lint.rs`): `forth_main`
   (callee-saved save/restore, sp↔return-stack switch, wire-format
   prologue/epilogue), proc/endp/next, an AArch64 `stk` Rust macro, and
   `aapcs_call`; 6 primitives (dup/drop/swap/+/1+ and a host-call word) run
   through `Mf66Session::{push,call,stack}`. The x18/q8–q15 grep gate is live.
   **Gotcha found:** `and sp, sp, #-16` is illegal (AND-immediate cannot target
   sp) — the realign math must go through a GPR (`and x17, x16, #-16; sub; mov
   sp, x17`); verified byte-identical to llvm-mc.
2. **Boot headless** — ⚠ port by **boot-criticality, not mechanical-vs-hard**:
   macros + stack + rstack + memory + the *subset* of arith (`+ - */mod`) and
   compare (`= < 0= 0<`) that `number`/`find` need + dict-find + number + parse +
   interpreter. (`number.masm` straddles the K1/K2 cut — it needs `udiv`; `find`
   needs compare and the 32-bit hash-compare upper-half audit.) *Verify:* REPL
   does integer arithmetic, stack ops, variables, `:`…`;`; corpus level 1.
   **IN PROGRESS:** the differential corpus harness is live (`tests/corpus.rs` +
   the 150-file `tests/data/direct/` corpus imported from WF66 — the day-one
   oracle); `docs/porting-guide.md` + `src/bin/asmcheck.rs` support a
   workflow-driven per-primitive port (translate → adversarial-verify → corpus
   gate). **Batch 1 done: 49 boot-critical register-only primitives** (arith 11,
   compare 20, logic 7, stack 11) → **corpus 50/150 PASS, 0 FAIL**. Remaining:
   memory + rstack primitives, the hard arith (double-cell/division, batch 2),
   then dict/number/parse/interpreter + the eval corpus.
3. **Hard primitives** — arith (128-bit `mul`+`umulh`/`smulh`, `sdiv`/`udiv`),
   compare (flag idioms → `csetm`/`csel`), strings (`bsr/bsf/popcnt`→`clz`/`rbit`/
   `cnt`), execute. *Verify:* full integer ANS core suite vs corpus.
4. **FP + math** — `float`/`fmath`, libm via `aapcs_call`, FTOS=d8, FSP=x22.
   *Verify:* FP suite matches within IEEE-754 identity.
5. **Optimizer back-end** — §3 + ⚠§4: `ArchReg`/`RegFile`, `lower()`→`Instr`,
   AArch64 `render()`, re-tuned promote/fuse thresholds, NZCV-liveness guard, and
   **sp-alignment-parity tracking** (§2 — elide redundant `aapcs_call` realigns
   where `sp` is provably 16-aligned, like `coalesce_dsp` for DSP). Kernel
   peephole (`compile.masm` K4) stays **disabled** (its backward scan assumes
   fixed x86 sizes; accept ~5–10% leaf loss; the Rust reducer has most of the
   win). *Verify:* round-trip tests; reducer cross-check; optimized == unoptimized
   == corpus observable state; **no `bl`/`blr` ever reached with misaligned sp**
   (alignment-elision soundness check).
6. **GC** — §5: Forth `HeapLayout`, tagged-root scanner, the kernel-wide tag
   invariant + `enter_native`/`leave_native`. *Verify:* allocation churn survives
   forced minor/major/full with no corruption; no live root dropped.
7. **Crash handler** — Mach server + signal fallback; manual Forth-return-stack
   backtrace. *Verify:* a fault in JIT'd Forth yields a symbolic dump.
8. **IDE** — `Mf66Session::eval` matching MacNCL's `Session::eval`; capture-buffer
   routing; stack-view publish; reuse REPL/transcript/Cocoa. ⚠ eval marshals to
   the mutator thread (W^X). *Verify:* interactive REPL in the MacNCL window.
9. **(opt)** kernel peephole re-enable (explicit watermarks — likely *not* worth
   it), iGui graphics shims, agents (Win32 fibers → ucontext/libdispatch; save
   x19–x28/x29/x30/sp/d8–d15; single mutator thread for W^X).

**Ship baseline = Phases 0–6** (headless, optimizing, GC'd). 7–9 are additive.

---

## 9. Risks & open questions

- **GC root precision (§5)** — the dominant correctness risk; kernel-wide tag
  invariant, not a small adapter.
- **Verify-cost dominates translate-cost** — for 13.6k hand-written asm lines the
  schedule lives in per-primitive differential verification, serialized behind
  the corpus. Label effort as (translate, verify) pairs.
- **`>r`/`r>`/`r@`/`2>r`/`rdrop` need from-scratch AArch64 semantics**, not
  translation: x86 pushes the return address to `rsp` automatically; AArch64 `bl`
  writes x30 (LR), spilled to `sp` only when the word nests. Define precisely what
  "top of the return stack" is (x30 vs `[sp]`).
- **MASM-idiom rewrites:** `rep movs`→ldr/str post-index loop (one shared macro,
  grep every `rep`); flag idioms→`csetm`/`csel`; 128-bit `rdx:rax`→
  `mul`+`umulh`/`smulh`; msvcrt→libm (IEEE-754 match).
- **Displacement/immediate ranges** — `render()` must range-check and decompose
  (12-bit ALU imm, scaled load/store offsets) or the JIT crashes cryptically.
- **Open questions:** FSP in x22 vs memory; pin pool callee-saved vs caller-saved;
  whether the x86 oracle is ever live (recommend: corpus frozen after Phase 5);
  macOS-only (assumed — pure manual catch/throw, no SEH/unwind tables).

---

### Appendix — verified substrate entry points
- `wfasm::a64::assemble` — `JASM/rust/src/a64/mod.rs:90`; encoder `a64/encode.rs`
  (cond codes `:57`, `@PAGE/@PAGEOFF`).
- `wfasm::native_macos::MacJit` — `native_macos.rs:77` (veneers clobber x16
  `:60-67,200-213`; adrp fixup `:224`).
- `newgc_core` collect/scanner — `locus/vendor/newgc-core/src/page_heap/cycle.rs`,
  `scanner.rs:39`, `lisp_layout.rs:43` (classify-by-tag).
- MacNCL `Session::eval` `ncl-compiler/src/lib.rs:215`; `output` capture
  `ncl-runtime/src/output.rs:20,25`; `IdeAction::Eval` `ncl-driver/src/main.rs:400`.
- WF66 reuse anchors — `src/wf66/mod.rs`: Token `:475`, reduce `:1177`, Instr
  `:1280`, coalesce_dsp `:1486`, window_fuse `:1557`, promote `:1780`, render
  `:1411`, assemble `:2131`.
