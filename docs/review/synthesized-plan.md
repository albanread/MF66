All key seams confirmed against the real code. The JASM A64 backend takes assembler text (`a64::assemble`), `MacJit` provides the loader, newgc-core's `collect_minor/major/full` take a `visit_roots` closure over a `RootScanner`, and MacNCL exposes `Session::eval(&str) -> Result<String, EvalError>` plus `output::begin_capture/end_capture` and `Ide::handle_event -> IdeAction::Eval`. I have everything needed to write the design.

# MF66 — Apple Silicon (macOS arm64) Port Design

> Target file (future MF66 repo): `docs/design/mf66-apple-silicon.md`
> Status: design / planning. Lead architect document.
> Substrate verified against: `JASM/rust/src/a64/` (A64 encoder), `JASM/rust/src/native_macos.rs` (`MacJit`), `locus/vendor/newgc-core/src/page_heap/` (GC), `MacNCL/src/ncl-compiler/src/lib.rs` + `ncl-runtime/src/output.rs` + `ncl-driver/src/main.rs` (REPL/IDE).

---

## 1. Strategy & what is reused unchanged

MF66 is a **retarget, not a rewrite**. WF66 was deliberately built as a two-level engine — an architecture-neutral Forth front-end and token-IR reducer sitting on top of an x86-specific lowering + deferred-assembly back-end — and that seam is exactly where the port cut falls. The supporting infrastructure (assembler, JIT loader, GC, IDE) already exists on Apple Silicon in sibling projects. The job of MF66 is to (a) build the AArch64 lowering/render leg of the back-end, (b) rewrite the 13.6k-line MASM kernel in AArch64 syntax, (c) swap four runtime substrate calls, and (d) glue in two pre-existing macOS subsystems (GC, IDE).

### 1.1 Reused unchanged — the WF66 arch-neutral front-end + reducer

These ship from WF66 with zero edits. They operate on `Token` values, never on assembly:

- **Token IR** — `Token` enum (`src/wf66/mod.rs:475-567`): `Lit, Inline, Stack, Mem, Ctl, Cmp, CmpCtl, FpBin, FpStack, FpMem, Call, LocalFetch, LocalStore`, etc.
- **IR builder** — `IrBuilder` (`mod.rs:574-695`).
- **Reduction engine** — `reduce`, `reduce_tail`, `reduce_pair` (`mod.rs:1177-1259`); rules: const-fold, DCE, imm-fold, dup-fuse, compare→branch fusion.
- **Constant folding** — `const_fold` (`mod.rs:731-750`) and `Fop::eval` (`mod.rs:50-59`), wrapping i64 arithmetic.
- **Token-level FP rewrites** — `fold_fp_abs_mem` (`mod.rs:1903-1923`).
- **Deferrability predicate** — `is_deferrable` (`mod.rs:1871-1901`).
- **Control-flow FSM management** — `CtlFrame` (`mod.rs:440-443`); the label/control-stack logic in `emit_cmp_ctl`/`emit_ctl` (`mod.rs:1048-1161`) reuses (only the terminal branch mnemonic changes).
- **Compile orchestration entry** — `compile_definition` (`mod.rs:1262-1265`): `reduce → lower → assemble`.
- **The deferred-assembly *algorithm* layer** — `coalesce_dsp`, `window_fuse`/`fuse_window`, `promote_hot_cells`/`promote_run` (`mod.rs:1486-1864`). These operate on the `Instr` AST and are register-name-agnostic *once* `Instr` carries abstract register names (see §2.1). The *symbolic-simulation parallel-move synthesis* and *hot-cell promotion* are pure permutation/liveness algorithms — they need no retargeting beyond the pool list.

### 1.2 Reused from sibling projects — already on Apple Silicon

- **JASM AArch64 backend** — verified present: `a64::assemble(text: &str) -> Result<EncodedModule>` (`JASM/rust/src/a64/mod.rs:90`), a full encoder (`a64/encode.rs`, 2430 lines) covering mov/mvn/movz/movk/movn, integer ALU, loads/stores (pre/post-index, bitmask-immediate, `@PAGE/@PAGEOFF` fixups), branches (`b/bl/b.cond/cbz/cbnz`), condition-code mapping (`cond_code`, `encode.rs:57`). This is the LLVM-free encoder WF66 already depends on via `default-features=false`.
- **`MacJit` loader** — `JASM/rust/src/native_macos.rs:77`: `with_capacity` (`:95`, `mmap(MAP_PRIVATE|MAP_ANON|MAP_JIT)`), `load_module` (`:148`), `define_extern` (`:142`), `finalize` (`:174`, flips W^X via `pthread_jit_write_protect_np`), `lookup`/`has_symbol`. Implements `crate::backend::Loader` (`:304`). This is the drop-in replacement for `NativeJit`.
- **MacNCL GC (`newgc-core`)** — `locus/vendor/newgc-core/`: `PageHeap<L>`, generational collector with `collect_minor`/`collect_major`/`collect_full` taking a `visit_roots` closure over a `RootScanner<L>` (`page_heap/cycle.rs:127,234,353`; `coordinator_api.rs:184`). Already arm64-darwin clean (libc `mmap` path).
- **MacNCL REPL/IDE** — `MacNCL/src/`: `Session::eval(&str) -> Result<String, EvalError>` (`ncl-compiler/src/lib.rs:215`), `output::begin_capture`/`end_capture` (`ncl-runtime/src/output.rs:20,25`), `Ide::handle_event -> IdeAction::Eval(String)` (`ncl-driver/src/main.rs:400`), Cocoa window + Core Graphics/Core Text rendering (platform-neutral `SurfaceCmd` IR).

### 1.3 What this leaves to do

Roughly four buckets: (1) the AArch64 **lowering + render** leg of the WF66 back-end; (2) the **kernel rewrite** (MASM → AArch64, the largest single effort); (3) the **runtime substrate** swaps (loader, memory, crash handler, pin codegen); (4) **GC + IDE glue** (small — both subsystems already exist). Everything algebraic and structural is free.

---

## 2. Per-arch retarget surface

### 2.1 WF66 compiler back-end — lowering + deferred-assembly buffer

The single most important refactor decision: **`lower()` must emit an `Instr` AST directly, not Intel-syntax text that is re-parsed.** Today the pipeline is `lower() → text → parse_instrs() → [coalesce/fuse/promote] → render() → text → assemble()`. The text→AST round-trip (`parse_instrs`, `mod.rs:1340-1406`) is the most x86-coupled component (it pattern-matches `add rbp`, `mov [rbp+disp]`, Intel mnemonics). Keeping it forces two parsers (x86 + AArch64) with shared fuse/promote logic — a permanent maintenance tax (compiler-review risk item).

**Decision:** introduce an explicit arch-neutral `Instr` AST as the *output* of `lower()`. `parse_instrs` is retained only for the x86 round-trip test and for ingesting `Raw` spans; the optimizer passes consume the AST that `lower()` produced. `render()` becomes the *only* arch-specific text generator.

#### Abstract register model

Replace every hardcoded register string with an `ArchReg` enum resolved by a per-arch `RegFile`:

```
enum ArchReg { Tos, Dsp, Up, Lp, FpPtr, Ftos,
               Scratch(u8 /*0..N*/), Promo(u8 /*0..M*/), CallTmp }
```

- `mod.rs:22-24, 29` (TOS=rax/DSP=rbp/FSP=[rbx+0x1218]) → `RegFile` queries.
- `mod.rs:1280-1302` (`Instr` enum: `AdjustDsp, LoadCell, StoreCell, RegMove, CellAlu`) → fields become `ArchReg`/`String` abstract names, not `rbp`/`rax`. **Effort: large.** The `Instr` records themselves are already arch-neutral in *shape*; the win is removing baked register strings.
- `mod.rs:1682-1683` (fusion pool `rsi/rdi/r8/r9/rcx/rdx`) and `mod.rs:1780-1781` (promotion pool `r10/r11`) → supplied by `RegFile` per-arch. AArch64 supplies a wider caller-saved pool (see §2.2). **Effort: small** (the algorithms are agnostic; only the pool list and `reg_present` metrics scan at `mod.rs:2066-2069` change).

#### `render()` — the new AArch64 text generator (`mod.rs:1411-1442`)

This is where x86 memory-operand ALU has no 1:1 mapping. Per-`Instr` rendering:

| `Instr` | x86 render | AArch64 render |
|---|---|---|
| `AdjustDsp(n)` | `add/sub rbp,n` | `add/sub x<DSP>,x<DSP>,#n` (12-bit imm; decompose if `|n|>4095`) |
| `LoadCell(dst, disp)` | `mov dst,[rbp+disp]` | `ldr x<dst>,[x<DSP>,#disp]` (decompose if `disp` out of unsigned-scaled / `ldur` ±255 range) |
| `StoreCell(src, disp)` | `mov [rbp+disp],src` | `str x<src>,[x<DSP>,#disp]` |
| `RegMove(d,s)` | `mov d,s` | `mov x<d>,x<s>` |
| `CellAlu(op, reg, disp)` | `op reg,[rbp+disp]` (one insn) | **decompose**: `ldr xT,[x<DSP>,#disp]; <op> x<reg>,x<reg>,xT` |

`CellAlu` decomposition is the one place code quality regresses — x86 folds a load into the ALU op; AArch64 cannot. Mitigation: this only matters inside hot fused windows, and the extra scratch (`xT`) comes from the (larger) AArch64 fusion pool. **Effort for render(): huge** (it is the load-bearing new code), but it is *additive* — the x86 `render` stays behind a cfg/dispatch.

#### Per-`Instr`/emit retargets

- **Literal load** (`mod.rs:815-824`): x86 `xor/mov/movabs` → AArch64 `movz x0,#lo16` + up to three `movk x0,#chunk,lsl #N`. No single-insn imm64. Range-classify: 0 → `movz x0,#0`; fits 16 bits → one `movz`; else `movz`+`movk` chain. **Effort: medium.**
- **Multiply strength reduction** (`mod.rs:849-876`): `shl`→`lsl`; `imul`→`mul`; the `lea [rax+rax*k]` trick has no equivalent — use `add x0,x0,x0,lsl #k` for ×(2^k+1) (covers 3/5/9 in one insn), else `mov xT,#k; mul x0,x0,xT`. **Effort: medium.**
- **Comparisons / flags** (`mod.rs:376-405`, `1048-1087`): x86 `setcc→movzx→neg` (materialize Forth -1/0) → AArch64 `cset x0,<cc>` gives 0/1; Forth needs all-bits, so `csetm x0,<cc>` (set −1/0 in one insn) is the correct primitive, **not** `cset`+`neg`. Conditional branches `jcc`→`b.<cc>`. Audit `CmpOp::inv_jcc` against AArch64 signed (`lt/le/gt/ge`) vs unsigned (`lo/ls/hi/hs`) condition codes — the encoder's `cond_code` (`a64/encode.rs:57`) is the authority. **Effort: large.**
- **FP lowering** (`mod.rs:234-327, 951-959`): FTOS `xmm15`→`d31`; scratch `xmm0`→`d0`; `addsd/subsd/mulsd/divsd`→`fadd/fsub/fmul/fdiv` (3-operand `fadd d31,d31,dT`); `movsd`→`fmov`/`ldr d`/`str d`. No memory-operand FP — decompose like `CellAlu`. FSP-coalescing algorithm unchanged (pointer stays in `FpPtr` register across runs). **Effort: medium.**
- **Indirect call** (`mod.rs:1001-1003`): `movabs rcx,xt; call rcx` → `movz/movk` chain into `CallTmp` (x16, AAPCS64's IP0 intra-procedure scratch — guaranteed free at call sites) then `blr x16`. STC uses `bl rel26` (±128 MB) where the target is in range; the kernel/dict fit within 128 MB so direct `bl` is the common path, `movz/movk + blr` the far fallback. **Effort: small.**
- **DSP coalescing** (`mod.rs:1486-1528`, `mem_rbp`/`parse_rbp_mem` `mod.rs:1310-1334`): make DSP-register-agnostic; offset arithmetic identical. One new concern: AArch64 `ldr/str` scaled-offset range is narrower than x86 disp32 — `render` must spill out-of-range displacements (`add xT,x<DSP>,#hi; ldr x0,[xT,#lo]`). **Effort: medium.**
- **Assembler integration** (`mod.rs:2131`): `wfasm::rasm::assemble(&asm)` → `wfasm::a64::assemble(&asm)` (verified entry point). Drop `.intel_syntax noprefix` preamble (`mod.rs:886`); A64 assembler needs no syntax directive. **Effort: small** *because the AArch64 assembler already exists.*
- **Round-trip tests** (`mod.rs:2223-2248`): add AArch64 variants asserting `render(lower(tokens))` assembles and that displacement/imm ranges are respected. **Effort: small.**

### 2.2 The MF66 ABI — AArch64 register homes

Concrete mapping, justified against AAPCS64 (callee-saved x19–x28; caller-saved x0–x18; x16/x17 = IP0/IP1 intra-procedure scratch; x18 = **platform register, reserved on Apple platforms — never touch**; x29 = FP, x30 = LR; v8–v15 callee-saved low 64 bits, v0–v7 caller-saved):

| Role | x86-64 (WF66) | AArch64 (MF66) | AAPCS64 class | Survives settle-barrier call? | Rationale |
|---|---|---|---|---|---|
| **TOS** | RAX | **x0** | caller-saved | No — but it *is* arg0/ret0 | TOS is the value flowing through rt_* calls; mapping to x0 makes it the natural first arg and return, matching `forth_main(u64,u64,u64,u64)->u64` (`lib.rs:1326`). Spilled to a cell across any call that clobbers it, exactly as x86 does. |
| **DSP** | RBP | **x19** | callee-saved | **Yes** | Must survive rt_* calls so the data stack pointer is stable across barriers (precondition for `coalesce_dsp` soundness). Callee-saved guarantees this with no spill. |
| **UP** | RBX | **x20** | callee-saved | **Yes** | User-area base; read on every user-cell access; must persist across everything. |
| **LP** | R15 | **x21** | callee-saved | **Yes** | Locals pointer. WF66 had R15 as *caller-saved* and accepted the risk; on AArch64 we *upgrade* it to callee-saved x21, eliminating the "rt_* clobbers x15" hazard the runtime review flags. |
| **FTOS** | XMM15 | **d8** | callee-saved (low 64) | **Yes** | FP top-of-stack must survive libm calls just as xmm15 (Win64 nonvolatile) did. d8 is the lowest callee-saved V-reg; only the low 64 bits are preserved, which is all a `double` needs. (Review suggested d31 — but d31 is caller-saved on AAPCS64 and would be clobbered by libm; **d8 is the correct choice**.) |
| **FpPtr (FSP)** | mem `[rbx+0x1218]` | **x22** (or keep in mem) | callee-saved | Yes | Promoting FSP from memory into callee-saved x22 lets FSP-coalescing keep the pointer live across FP runs cheaply. Falls back to `[x20+user_FSP]` if register pressure demands. |
| **Fusion scratch pool** | rsi/rdi/r8/r9/rcx/rdx (6) | **x9,x10,x11,x12,x13,x14,x15 (7)** | caller-saved | No (window-local only) | Parallel-move temps live only inside Raw-free windows that contain no calls, so caller-saved is fine and a *wider* pool (7 vs 6) gives `window_fuse` more freedom. |
| **Promotion pool** | r10/r11 (2) | **x23,x24** | callee-saved | **Yes** | Hot-cell promotion holds a value across a run that may include barriers; on x86 r10/r11 were caller-saved and the pass had to sound-check barrier-crossing. On AArch64 we use *callee-saved* x23/x24 so promoted values survive calls unconditionally — strictly safer. |
| **Call scratch** | rcx | **x16** (IP0) | caller-saved, ABI-scratch | No | Indirect-call target register; AAPCS64 explicitly reserves x16/x17 for exactly this. |
| Return stack | RSP | **sp** | — | — | STC: native call/ret = Forth return stack, unchanged. |
| FP scratch | xmm0/xmm1 | **d0–d7** | caller-saved | No | libm arg/result + transient FP scratch. |

**Reserved/avoided:** x18 (Apple platform register), x17 (IP1, leave for linker veneers), x25–x28 (free callee-saved headroom for future pinning), x29/x30 (FP/LR — STC frame + return address).

Key ABI properties this mapping guarantees:
1. Everything that must survive a settle-barrier rt_* call (DSP, UP, LP, FTOS, FSP, both promotion regs) is **callee-saved** — no spill code at call sites, and the `coalesce_dsp`/`promote_hot_cells` soundness conditions hold by construction.
2. TOS↔x0 unifies the Forth value register with the C ABI arg/ret register, eliminating shuffles at the rt_* boundary.
3. The fusion pool is caller-saved (cheap, window-local); the promotion pool is callee-saved (survives barriers) — this is *cleaner* than x86, where promotion used caller-saved r10/r11 and needed an explicit barrier guard.

### 2.3 STC kernel — 13.6k-line MASM → AArch64 rewrite

This is the dominant effort. Strategy: **macro library first, then primitives file-by-file, mechanical files before idiom-heavy files.** The kernel target syntax is whatever `a64::assemble` accepts (the same encoder the back-end emits to), so the kernel and the JIT'd code share one assembler.

#### Phase order within the kernel

**Step K0 — macro library (`kernel/macros.masm:2-330`).** Define the AArch64 register homes (§2.2) and the call macro. The Win64 `win64_call` (`macros.masm:313-330`, 32-byte shadow space + r12 spill) becomes `aapcs_call`: save LR, ensure sp 16-aligned, marshal args x0–x7, `bl`/`blr`, restore. `brk` macro (`macros.masm:379`) → `brk #0`. This single macro file unblocks every other file. **Hard** (gates everything) but small in lines.

**Step K1 — mechanical primitives (translate-in-place):**
- `stack.masm`, `rstack.masm` — except `xchg rbp,rsp` 2swap trick (`stack.masm:174-186`) and `rep movs` n>r/nr> (`rstack.masm:181-219`, expand to ldr/str post-index loops).
- `logic.masm` — shifts drop the CL constraint (`shl rax,cl`→`lsl x0,x0,x1`); bit ops `and/or/xor`→`and/orr/eor`.
- `memory.masm` — `@`/`!` become `ldr`/`str`.
- `locals.masm` — `[r15+disp]`→`[x21,#disp]` (ldur ±255, else add+ldr); frame sub.
- `dict.masm` (975 lines), `oop.masm` (285) — pure loads/compares/hash; **mostly mechanical** (only `lea`→`add`). Audit 32-bit `cmp eax,imm32` hash compares (`dict.masm` dn_hash) for upper-half assumptions.
- `number.masm`, `parse.masm` — algorithms portable; `lodsb`/`rep movs`→ldr/str post-index loops.

**Step K2 — idiom-heavy primitives (per-instruction care):**
- `arith.masm:29-36,222-231,313-416` — `mul/imul rdx:rax`→`mul`+`umulh`/`smulh` for 128-bit (um*, m*, um/mod, sm/rem, */mod); `cqo+idiv`→`sdiv`; `div`→`udiv`. No implicit rdx:rax pairing — explicit two-register results.
- `compare.masm:24-177` — `sbb rax,rax` carry-smear and `setcc`→`csetm`/`cset` + `csel`/`csneg`. Every comparison word (0=, 0<, =, <, u<, min, max, within, d=, du<, …) needs per-insn condition mapping.
- `strings.masm:24-49` — `bsr/bsf/popcnt`→`clz`/`rbit+clz` for ctz / `cnt`; msbit = `63 - clz(x)`; zero-input saturation via `cmp`+`csel`.
- `execute.masm:31-65` — tail `jmp rcx`→`br x16`; catch/throw unwinding stays manual (Forth HANDLER frame), no SEH.

**Step K3 — FP + math (`float.masm:1-160`, `fmath.masm:1-78`):** No x87 (already XMM-only). `movsd`→`ldr/str d`; arg shuffles `movsd xmm0,xmm15`→`fmov d0,d8`. **msvcrt → libm/libSystem** (sqrt, sin, cos, …) via `aapcs_call` — link-level change, not a code rewrite. FTOS save/restore `[user_FTOS_SAVE]`→`str d8,[x20,#off]`.

**Step K4 — kernel-side peephole optimizer (`compile.masm`, ~3230 lines).** This is the hardest and is **deferred** (see §4 phasing). It back-scans `HERE` for prior emit patterns using fixed x86 instruction sizes (watermarks `user_LAST_LIT_END`, etc., `compile.masm:150-192`). AArch64's variable-length movz/movk literal sequences break the fixed-offset assumption. **MVP decision: disable kernel peephole fusion for MF66** (5–10% leaf-word perf loss, safe). The real optimizer lives in the Rust WF66 token-IR reducer anyway; the kernel peephole is a secondary win. Opt-phase: re-enable with explicit per-instruction watermarks rather than implicit byte sizes.

**Mechanical vs hard summary:**
- *Mechanical:* dict, oop, memory, locals, stack basics, logic — translate-in-place.
- *Hard:* arith (128-bit mul/div), compare (flag idioms), strings (bit-scan), the macro library (gates all), the kernel peephole (defer/disable).

#### Reusable-as-is in the kernel (data, not code)
Dictionary header layout, user-area offsets, STC threading model, catch/throw frame layout, OOP vtable structure, dict bucket chains, variable-stub *shape*, parse/number algorithms — all arch-neutral memory/control structures (only the embedded instructions change).

### 2.4 Runtime substrate

| Concern | WF66 (Windows/x86) | MF66 (macOS/arm64) | Effort |
|---|---|---|---|
| JIT loader | `wfasm::native::NativeJit` (`lib.rs:1402-1403`) | `wfasm::native_macos::MacJit` (`native_macos.rs:77`) — `with_capacity`/`load_module`/`define_extern`/`finalize` | medium |
| Forth region | `VirtualAlloc2`+`MEM_ADDRESS_REQUIREMENTS` (`lib.rs:1158-1192`) | `mmap(MAP_PRIVATE\|MAP_ANON)`, drop near-memory window, 0x4000 page align | medium |
| JIT arena | `lib.rs:1200-1231` | `mmap`, no ±128 MB window (bl reach + far veneers), 0x4000 align | small |
| Locals region | `VirtualAlloc` (`lib.rs:1249-1262`) | `mmap` RW, 0x4000 align; x21=LP init unchanged | small |
| Var region | `VirtualAlloc2` ±512 MB (`lib.rs:1276-1320`) | `mmap` RW; no near window (no RIP-relative; movz/movk or ADRP+ADD) | small |
| rt_* ABI | extern "C" = Win64 (`lib.rs:1463-1576`) | extern "C" = AAPCS64 — **Rust side needs no change**; kernel call sites change to `aapcs_call` | large (kernel) |
| `forth_main` | `extern "system" fn(u64×4)->u64` (`lib.rs:1326`) | unchanged — AAPCS64 puts args in x0–x3, ret x0 | small |
| Crash handler | VEH `wfasm::seh` (`lib.rs:1421-1422,1654-1681`) | **Mach exception server** (`task_set_exception_ports`, `EXC_BAD_ACCESS`/`EXC_BAD_INSTRUCTION`) + POSIX `SIGBUS`/`SIGILL` fallback; read AArch64 `__darwin_arm_thread_state64` frame; keep code-range + Forth-dump registration | large (~300 lines) |
| Register pinning codegen | x86 bytes `pin.rs:218-356` | `emit_*_aarch64`: `ldr/str x9,[xN,#disp]`; pin analysis `pin.rs:109-205` unchanged; gate x86 behind cfg | large |
| Pin pool | r9/r10/r11 caller-saved | callee-saved x25–x27 (survive barriers) or x9–x15 caller-saved if window-local | small |
| Keyboard I/O | `_kbhit`/`_getwch` (`runtime.rs:65-69`) | already-gated stdin BufRead path; later libedit raw mode | small |
| iGui Win32/GDI | `lib.rs:71-77`, `src/igui.rs` | **out of scope** — headless first; IDE from MacNCL (§2.6) | huge (deferred) |

Position-independence is satisfied: movz/movk chains produce absolute immediates; `render()` must **avoid** `adr/adrp`-to-label PC-relative forms (the `@PAGE/@PAGEOFF` fixups the A64 encoder supports) except where the loader resolves them, to preserve the same PIC discipline x86 had with movabs.

### 2.5 GC — wire newgc-core to the Forth kernel

The GC ships from MacNCL/locus essentially unchanged; MF66 supplies only the language adapter. Three integration points:

1. **`HeapLayout` impl for Forth** — mirror `lisp_layout.rs:36-121` (`classify(raw:u64) -> WordKind` at `:43`). Forth supplies its own tag scheme (the kernel review documents the 3-bit tag: FloatVec=010, RefVec=011, String=100, Builder=101; managed strings use a 3-bit tag + 61-bit pointer). `ObjectLayout` describes pointer-field ranges per type. WF66's existing `src/gc/layout.rs` (`Wf64Layout`, `mod.rs:44-229`) is already a `HeapLayout` impl and is **platform-neutral** — it can be lifted directly; this is mostly a "point it at newgc-core from MacNCL instead of the bundled copy." **Effort: small.**

2. **Root scanner adapter** — provide a `visit_roots` closure to `collect_minor`/`collect_major`/`collect_full` (`cycle.rs:127/234/353`) that enumerates Forth roots: **data stack** (TOS in x0 *plus* spilled cells from DSP top down to SP0), **return stack** (sp region), **FP stack** (FTOS in d8 + FSP region), and **dictionary / HEAPPTR+LITERAL regions** (`runtime.rs:634-640` already extracts `(base,next)` pairs from the user area — reuse verbatim). Each root visited via `RootScanner::visit(&mut Word)`. **Effort: small (~30–50 lines)**, but correctness-critical (see §5).

3. **Heap init** — `PageHeap::with_reservation` (`heap.rs:95`) on the MacNCL/libc `mmap` path; WF66's `src/gc/heap.rs` wrappers (`alloc_floatvec/alloc_refvec/alloc_string/alloc_builder/collect_*`) are platform-neutral and reused. rt_* GC entry points (`rt_gc_collect`, `rt_gc_auto_step`, `rt_vec_alloc_floats/refs`, `runtime.rs:582-628,1363-1444`) are arch-neutral Rust. **Effort: small** (defer to MacNCL's port).

The mutator discipline (`enter_native`/`leave_native` bracketing FFI and eval boundaries) must wrap every Forth↔Rust call boundary so a GC triggered inside an rt_* call sees a consistent root snapshot — this is the linchpin invariant.

### 2.6 IDE — plug MacNCL's REPL/IDE onto the MF66 core

MacNCL's IDE is eval-agnostic. The seam is `Session::eval(src:&str) -> Result<String, EvalError>`. MF66 provides a Forth `eval` matching that contract and reuses everything else:

- **Eval entry** — implement `Mf66Session::eval(&str)` = parse/find/compile XT → `enter_native` → call native code → `leave_native` → format return stack / catch THROW → `Result<String, EvalError>`. Wrap with `output::begin_capture()`/`end_capture()` (`output.rs:20,25`) so Forth `.`/`type`/`emit` (rt_emit/rt_type) route into the REPL transcript exactly as Lisp `format t` does. **Effort: medium.**
- **REPL driver** — `Ide::handle_event -> IdeAction::Eval(String)` (`ncl-driver/src/main.rs:400`) and the transcript/history/bracket-balance logic are eval-agnostic; reuse unchanged. Bracket-balance check adapts to Forth's `:`…`;` instead of parens.
- **Stack view** — MacNCL's `stack_view::publish` model takes a snapshot; feed it the Forth data stack each eval.
- **Rendering** — Cocoa window + Core Graphics/Core Text + `SurfaceCmd` IR all reused unchanged.
- **iGui graphics words** — optional Forth shims (mirror MacNCL `igui_mac/shims.rs` and `dispatch_gui_event`) routing `gpane-*`/`DrawLine`/`FillRect` to the same `SurfaceCmd` canvas. **Effort: large, deferred** (not on the baseline path).

The headless core runs first; the IDE is additive and never blocks the kernel/compiler bring-up.

---

## 3. The differential-oracle story

MF66's correctness criterion: **for any Forth program, MF66 produces the same observable Forth state (data stack, return stack, FP stack, memory, output text, thrown exceptions) as WF66.** WF66 is the semantic oracle.

Three layered oracles, in increasing fidelity:

1. **Committed corpus oracle (Mac-local, primary, day-one).** WF66 already validates the WF66 reducer differentially against the eager WF65 baseline. Capture WF66's expected outputs once on the x86 box — for the ANS core test suite plus WF66's own test corpus — into a committed golden file: `(source → final .s output, stack image, emitted text)`. MF66 runs the identical sources and asserts byte-identical observable state. No x86 box needed at MF66 build time; the corpus is the oracle. This is the workhorse for CI on Apple Silicon.

2. **Token-IR reducer cross-check (architecture-independent, free).** The WF66 reducer is arch-neutral and runs natively on arm64. For every definition, assert `reduce(tokens)` on MF66 == the committed reduced token stream from WF66. This isolates *front-end/reducer* regressions from *lowering/kernel* regressions — if the reduced IR matches but observable state diverges, the bug is in AArch64 lowering or the kernel, not the optimizer.

3. **Live x86 oracle via Wine/Windows box (secondary, for new programs).** For programs not in the committed corpus, run WF66 under Wine or on a Windows/x86 host and diff against MF66. Used to *expand* the corpus, not for routine CI.

**State-canonicalization for the diff:** the settle-everywhere ABI makes this clean — at every word boundary the entire Forth state except TOS is canonical in memory. So the oracle compares: (a) the canonical data-stack image (DSP↓ to SP0, with TOS folded in), (b) the FP-stack image (FSP region, FTOS folded), (c) the return stack, (d) the byte-for-byte dictionary/HERE region after `forget` normalization, (e) captured output text, (f) thrown THROW codes. ASLR/address normalization (the WF66 `golden.rs` technique) masks absolute addresses before comparison.

**What is *not* expected to match:** emitted machine bytes (different ISA) and instruction counts. The `opt-metrics`/`golden.rs` x86 path (uses `iced-x86`) is gated off on arm64; MF66 metrics either stub out or use a capstone-arm64 disassembler — but metrics are diagnostic, never a correctness gate.

---

## 4. Phased plan (smallest-first, each independently verifiable)

Dependency order is strict left-to-right within each phase; phases are cumulative.

**Phase 0 — Substrate smoke test.**
Wire `MacJit` into a throwaway harness; assemble a hand-written AArch64 "return 42" via `a64::assemble` → `MacJit::load_module` → `finalize` → call. Confirm W^X toggle and `define_extern`/`lookup`. *Verify:* function returns 42; an extern call returns. **No WF66 code involved.** Gates everything.

**Phase 1 — Kernel macro library + ABI.**
Implement §2.2 register homes and the `aapcs_call` macro (K0). Port `forth_main` prologue/epilogue (save x19–x28, x30, d8–d15). *Verify:* a trivial 2-word kernel (`lit`, `+`) assembles and runs a single colon-free invocation returning a known TOS via `forth_main`.

**Phase 2 — Boot the kernel headless (core primitives).**
Port K1 (mechanical: stack, rstack, logic, memory, locals, dict, number, parse) + the interpreter/QUIT loop. Bind rt_* externs (Rust side unchanged; kernel uses `aapcs_call`). Bring up `alloc_forth_region`/`alloc_jit_arena`/`alloc_locals_region`/`alloc_var_region` on `mmap`. *Verify:* dictionary bootstraps; REPL evaluates integer arithmetic, stack ops, variables, `:`…`;` definitions of integer words; `.s` prints. Differential oracle (corpus level 1) on the integer subset.

**Phase 3 — Hard primitives: arith / compare / strings / execute.**
Port K2. *Verify:* full integer ANS core test suite passes against the committed corpus oracle; 128-bit mul/div words, all comparison/flag words, bit-scan words match WF66 bit-for-bit in observable state.

**Phase 4 — FP + math.**
Port K3 (float/fmath, libm linkage, FTOS=d8, FSP=x22). *Verify:* FP test suite (Mandelbrot, FFT, transcendental) matches WF66 FP-stack images within IEEE-754 identity.

**Phase 5 — Enable the WF66 optimizer back-end (AArch64 lowering + render).**
Land §2.1: abstract `ArchReg`/`RegFile`, `lower()`→`Instr` AST, AArch64 `render()`, AArch64 emit_* (literals, multiply, compare/branch, FP, call). Run `coalesce_dsp`/`window_fuse`/`promote_hot_cells` unchanged over the AArch64 AST. *Verify:* (a) round-trip/assemble tests (`mod.rs:2223-2248` arm64 variants); (b) reducer cross-check (oracle level 2) — reduced IR matches WF66; (c) optimized definitions produce identical observable state as the Phase 2–4 unoptimized path *and* as the corpus oracle. Kernel peephole (K4) stays **disabled**.

**Phase 6 — Wire GC.**
Implement §2.5: Forth `HeapLayout`, root-scanner closure (data/return/FP stacks + dict/HEAPPTR/LITERAL), `enter_native`/`leave_native` bracketing. *Verify:* allocation-heavy test (vector/string churn) survives forced minor+major+full collections with no corruption; stack-stress test confirms no live root is dropped (use-after-free canary).

**Phase 7 — Crash handler.**
Mach exception server + SIGBUS/SIGILL fallback; AArch64 thread-state frame decode; Forth dump from JIT code ranges. *Verify:* a deliberate fault in JIT'd Forth produces a symbolic Forth dump, not a silent crash; the dumper itself is guarded (page-readable checks).

**Phase 8 — Wire IDE.**
Implement §2.6: `Mf66Session::eval` matching MacNCL's `Session::eval` contract; capture-buffer routing; stack-view publish; reuse REPL/transcript/Cocoa rendering. *Verify:* interactive REPL in the MacNCL window evaluates Forth, shows transcript output and live stack.

**Phase 9 (opt) — Kernel peephole re-enable + iGui graphics + agents.**
K4 with explicit watermarks; `gpane-*` SurfaceCmd shims; green-thread scheduler (replace Win32 fibers with pthread/ucontext or libdispatch context-switch, per the gc_aux review). *Verify:* perf parity within target; agent round-robin test; graphics demos render.

Baseline ship target = Phases 0–6 (headless, optimizing, GC'd). Phases 7–9 are quality/feature additions.

---

## 5. Risks & open questions

**Taint set re-validation.** The WF66 taint set (Opaque tokens → fall back to eager baseline for do/loop, I/O, return-stack ops, FP-comparisons) assumes Win64 conventions. On AAPCS64 some entries may relax or shift — e.g., return-stack interference differs because LR/x30 is special, and FP-comparison taint may change since AArch64 sets NZCV from `fcmp` differently. **Action:** re-audit each taint rule against AAPCS64 before enabling the optimizer (Phase 5); the design is built to make the taint set tunable.

**FP stack on AArch64 — the d8 vs d31 trap.** The reviews repeatedly suggest FTOS=d31. **d31 is caller-saved and would be clobbered by every libm call** — FTOS must be callee-saved to survive math callouts (the property xmm15 had under Win64). MF66 uses **d8** (lowest callee-saved). A single residual d31 reference in any emit/kernel path silently corrupts floats. **Action:** grep the entire FP path; the all-float test suite (Phase 4) is the backstop.

**Settle-barrier calls across AAPCS64.** Soundness of `coalesce_dsp` and `promote_hot_cells` depends on DSP/UP/promoted regs being stable across rt_* barriers. The §2.2 mapping guarantees this by putting all of them in callee-saved regs — but this is only sound if **no rt_* function violates AAPCS64** (Rust does, but hand-checked asm or toolchain quirks could). **Action:** audit every rt_* extern; the upgrade of LP and the promotion pool to callee-saved (vs WF66's caller-saved) removes the largest x86-era hazard.

**MASM-idiom rewrites with no 1:1 mapping:**
- *`rep movs` (rstack/parse/number/strings, ~200+ lines):* mechanical expansion to ldr/str post-index loops; risk is *missing one* → silent corruption or infinite loop. **Action:** a single shared copy-loop macro, applied uniformly; grep for every `rep`.
- *Flag idioms (`sbb rax,rax`, `setcc`):* the Forth -1/0 flag needs `csetm` (not `cset`+`neg`); cmov→csel/csneg. Per-instruction; easy to get the condition code inverted. **Action:** drive condition mapping off the encoder's `cond_code` table (`a64/encode.rs:57`), unit-test each comparison word.
- *128-bit mul/div (`rdx:rax`, `cqo+idiv`):* no implicit register pair; `mul`+`umulh`/`smulh`, `sdiv`/`udiv`. **Action:** the arith test suite must cover overflow/high-half cases explicitly.
- *No x87:* a non-issue (WF66 is already XMM-only) — but msvcrt→libm is a hard link-level dependency; transcendental results must match IEEE-754.

**`CellAlu` decomposition / code-quality regression.** AArch64 has no memory-operand ALU, so fused `op reg,[rbp+disp]` decomposes to load+alu, losing a micro-optimization inside hot windows. **Action:** accept for MVP; the wider 7-register fusion pool mitigates the extra scratch demand; measure in Phase 9.

**Displacement / immediate range limits.** x86 disp32 and imm32 freely encode where AArch64 has 12-bit shifted ALU immediates and narrower scaled load/store offsets. `render()` must detect out-of-range and decompose (or the JIT crashes cryptically). **Action:** range-checks in `render()` with explicit decomposition paths; round-trip tests must include boundary displacements.

**Kernel peephole optimizer (`compile.masm`, 3230 lines).** Its backward-scan assumes fixed x86 instruction sizes; AArch64 variable-length movz/movk literals break it. **Decision:** disabled for MVP (accept 5–10% leaf-word loss). **Open question:** is the explicit-watermark rewrite (Phase 9) worth it, given the Rust token-IR reducer already captures most of the win? Likely no — may stay permanently disabled.

**Position-independence vs `adrp`.** The A64 encoder supports `@PAGE/@PAGEOFF` PC-relative fixups, which are *shorter* than movz/movk chains but break the absolute-immediate PIC discipline WF66 relied on. **Action:** `render()` must default to movz/movk absolutes; only use ADRP+ADD where the loader is the one resolving the fixup. Decide explicitly per emit site.

**Green threads / agents.** Win32 fibers (`agents.rs:115-148`) have no macOS equivalent; the per-agent state swap (USER_HANDLER/USER_FSP/USER_SELF) plus native-stack save/restore must be hand-rolled on AArch64 (save x19–x28, x29, x30, sp, d8–d15). Subtle CFI/stack-corruption bugs. **Action:** Phase 9; round-robin test harness essential; single-threaded W^X (`pthread_jit_write_protect_np` is per-thread) means agents must share one mutator thread or each manage its own W^X toggle.

**Open questions for the team:**
1. FSP — keep in callee-saved x22, or leave in memory `[x20+user_FSP]`? (Register is faster for FP-heavy code; memory frees a register.)
2. Pin pool — callee-saved x25–x27 (survives barriers, fewer regs) vs caller-saved x9–x15 (more regs, window-local only)? Depends on whether pinning must span calls.
3. Is the live x86 oracle (Wine box) maintained long-term, or is the committed corpus frozen as the sole oracle after Phase 5?
4. Does MF66 ever need Windows-on-ARM64 (ARM64EC SEH/XDATA), or is it macOS-only? (Affects the crash-handler and unwind strategy — design assumes macOS-only, pure manual Forth catch/throw.)

---

### Appendix — verified substrate entry points

- `wfasm::a64::assemble(&str) -> Result<EncodedModule>` — `JASM/rust/src/a64/mod.rs:90` (encoder: `a64/encode.rs`, 2430 lines; cond codes `:57`; `@PAGE/@PAGEOFF` fixups).
- `wfasm::native_macos::MacJit` — `JASM/rust/src/native_macos.rs:77` (`with_capacity:95`, `load_module:148`, `define_extern:142`, `finalize:174`, `lookup:274`; implements `backend::Loader:304`).
- `newgc_core::PageHeap::{collect_minor,collect_major,collect_full}(visit_roots)` — `locus/vendor/newgc-core/src/page_heap/cycle.rs:127,234,353`; `RootScanner` — `page_heap/scanner.rs`; `HeapLayout::classify` reference impl `lisp_layout.rs:43`.
- `Session::eval(&str) -> Result<String, EvalError>` — `MacNCL/src/ncl-compiler/src/lib.rs:215`; `output::begin_capture/end_capture` — `ncl-runtime/src/output.rs:20,25`; `IdeAction::Eval` dispatch — `ncl-driver/src/main.rs:400`.
- WF66 reuse-as-is anchors — `src/wf66/mod.rs`: Token `:475-567`, IrBuilder `:574-695`, reduce `:1177-1259`, Instr enum `:1280-1302`, coalesce_dsp `:1486-1528`, window_fuse `:1557-1767`, promote_hot_cells `:1780-1864`, render `:1411-1442`, assemble call `:2131`.