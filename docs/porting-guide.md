# MF66 kernel porting guide — WF66 x86-64 MASM → AArch64

How to translate a WF66 `proc(sym) … endp()` primitive to the MF66 AArch64
kernel. The reference for every translation agent. Goal: **identical observable
Forth behavior** (verified by the word's `tests/data/direct/<word>.t`), not a
literal instruction-for-instruction copy.

## Register map (the MF66 ABI — see `kernel/macros.masm`)

| Role | WF66 (x86) | MF66 (AArch64) | Notes |
|---|---|---|---|
| TOS | `rax` | `x0` (alias `TOS`) | cached top of data stack |
| NOS | `[rbp]` | `[x19]` (`[DSP]`) | second item, in memory |
| NNOS | `[rbp+8]` | `[x19, #cell]` | third item |
| DSP | `rbp` | `x19` (`DSP`) | points at NOS, grows **down**; push = `sub`, pop = `add` |
| UP | `rbx` | `x20` (`UP`) | user area base |
| LP | `r15` | `x21` (`LP`) | locals frame |
| FTOS | `xmm15` | `d8` (`FTOS`) | FP top of stack |
| scratch | `rcx rdx rsi rdi r8–r11` | **`x9`–`x15`** (caller-saved) | free inside a primitive |
| return stk | `rsp` | `sp` | STC; `next()` = `ret` |

**Forbidden:** never emit `x16`, `x17` (veneer/IP scratch — clobbered by any
`bl`), `x18` (Darwin-reserved), or 128-bit `q8`–`q15`. The `kernel_lint` test
fails the build if you do. Use `x9`–`x15` for scratch.

## Shape of a primitive

```
; word  ( stack-effect )   ; one-line description
proc(asm_sym)
    ; manipulate TOS in x0, NOS at [DSP], scratch in x9-x15
    stk(in, out)      ; emits the DSP adjust for the net stack effect
    next()            ; = ret
endp()
```

- `stk(in, out)` adjusts DSP for the declared effect: `stk(2,1)` pops one cell
  (`add x19,x19,#8`), `stk(0,1)` pushes (`sub x19,x19,#8`), `stk(1,1)` emits
  nothing. **Always declare `stk` with the same `(in,out)` WF66 used.**
- Comments are `;` (the front-end lexer rejects `//`).
- Immediates: `#cell` works (`cell`=8 is substituted); `add x0, x0, #1`.

## Idiom translation table

| x86 (WF66) | AArch64 (MF66) | Note |
|---|---|---|
| `neg rax` | `neg x0, x0` | |
| `not rax` | `mvn x0, x0` | invert |
| `add rax, [rbp]` | `ldr x9, [DSP]` ; `add x0, x0, x9` | **no memory-operand ALU** — load NOS to a scratch first |
| `sub rax, [rbp]` | `ldr x9, [DSP]` ; `sub x0, x0, x9` | computes TOS−NOS; for NOS−TOS use `sub x0, x9, x0` |
| `and/or/xor rax, [rbp]` | `ldr x9,[DSP]` ; `and/orr/eor x0,x0,x9` | |
| `add rax, imm` | `add x0, x0, #imm` | imm ≤ 4095 (else materialize in a scratch) |
| `imul rax, [rbp]` | `ldr x9,[DSP]` ; `mul x0, x0, x9` | |
| `lea rax,[rax+rax*N]` / `imul rax,K` | `add x0,x0,x0,lsl #k` / `lsl`+`add` | strength-reduced small-constant multiply |
| `shl/shr/sar rax, cl` | `lsl/lsr/asr x0, x0, x9` (count in `x9`) | variable shift; mask count if WF66 did |
| `shl/shr/sar rax, imm` | `lsl/lsr/asr x0, x0, #imm` | |
| `cqo` ; `idiv rbx` | `sdiv` + `msub` | quotient `sdiv xq,xn,xd`; remainder `msub xr,xq,xd,xn` |
| `mul`/`div` (unsigned) | `udiv` + `msub` ; `umulh` for high half | |
| `rdx:rax` 128-bit product | `mul` (low) + `smulh`/`umulh` (high) | double-cell arithmetic |
| `movzx eax, byte [..]` | `ldrb w0, [..]` | zero-extends; `ldrsb` to sign-extend |
| `mov byte [..], al` | `strb w0, [..]` | |
| `rep movsb` | post-indexed `ldrb`/`strb` loop | one shared idiom; see strings later |

### Flag / comparison idioms → `cmp` + `cset`/`csetm`  (critical)

WF66 produces Forth flags (`0` / `-1`) with `sub`/`sbb` or `setcc` tricks. On
AArch64 use `cmp` then a conditional-set. **Forth true is all-ones (`-1`), so use
`csetm` (sets `-1`/`0`), not `cset` (sets `1`/`0`).**

| WF66 pattern | meaning | AArch64 |
|---|---|---|
| `sub rax,[rbp]` ; `sub rax,1` ; `sbb rax,rax` | `=` → −1/0 | `ldr x9,[DSP]` ; `cmp x0,x9` ; `csetm x0, eq` |
| `<>` variant | `≠` | `cmp x0,x9` ; `csetm x0, ne` |
| signed `<` (`n1 n2 -- f`) | NOS < TOS | `ldr x9,[DSP]` ; `cmp x9, x0` ; `csetm x0, lt` |
| signed `>` | NOS > TOS | `cmp x9, x0` ; `csetm x0, gt` |
| unsigned `u<` / `u>` | | `cmp x9, x0` ; `csetm x0, lo` / `hi` |
| `0=` | `csetm x0, eq` after `cmp x0,#0` | or `cmp x0,#0` |
| `0<` | `csetm x0, mi` (`cmp x0,#0`) | sign |
| `min`/`max` | | `cmp`/`csel x0, x0, x9, lt|gt` |

Mind the operand order: WF66 keeps TOS in `rax` and NOS at `[rbp]`; check which
operand is which in the stack effect and set the condition accordingly. **When in
doubt, the `.t` file's `push a / push b / call word / expect f` cases pin the
exact semantics — make those pass.**

## Multi-cell stack ops (2dup, 2swap, …)

These are pure `mov` shuffles of TOS + memory cells. Translate `mov` between
`rax`/`[rbp+N]` to `ldr`/`str`/`mov` on `x0`/`[x19,#N]`, keeping TOS cached in
x0. Watch the `stk` net effect and that the cached TOS ends up correct.

## Output contract (for the workflow)

Return the complete `proc(sym) … endp()` block in MF66 AArch64 style, ready to
paste into the category kernel file, plus the `stk(in,out)` you used and any
nonobvious decision (which operand order, which condition code, why a scratch).
Do not include `@include` lines or surrounding file scaffolding.
