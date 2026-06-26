//! Minimal AArch64 instruction encoders for the colon compiler (and later the
//! optimizer's `render`). Each returns a 32-bit instruction word. Verified
//! byte-for-byte against `wfasm::a64::assemble` in the tests below.

/// `movz Xd, #imm16, LSL #shift`  (shift ∈ {0,16,32,48}).
pub fn movz(rd: u32, imm16: u16, shift: u32) -> u32 {
    0xD280_0000 | ((shift / 16) << 21) | ((imm16 as u32) << 5) | (rd & 0x1F)
}
/// `movk Xd, #imm16, LSL #shift`.
pub fn movk(rd: u32, imm16: u16, shift: u32) -> u32 {
    0xF280_0000 | ((shift / 16) << 21) | ((imm16 as u32) << 5) | (rd & 0x1F)
}
/// `blr Xn`.
pub fn blr(rn: u32) -> u32 {
    0xD63F_0000 | ((rn & 0x1F) << 5)
}
/// `ret` (returns to x30).
pub fn ret() -> u32 {
    0xD65F_03C0
}
/// `str Xt, [Xn, #imm9]!`  (pre-index, 64-bit).
pub fn str_pre(rt: u32, rn: u32, imm9: i32) -> u32 {
    0xF800_0C00 | (((imm9 as u32) & 0x1FF) << 12) | ((rn & 0x1F) << 5) | (rt & 0x1F)
}
/// `ldr Xt, [Xn], #imm9`  (post-index, 64-bit).
pub fn ldr_post(rt: u32, rn: u32, imm9: i32) -> u32 {
    0xF840_0400 | (((imm9 as u32) & 0x1FF) << 12) | ((rn & 0x1F) << 5) | (rt & 0x1F)
}

/// Materialize a 64-bit `val` into `rd` via movz + up to 3 movk (always 4 words
/// here for a fixed-size, patchable sequence).
pub fn load_imm64(rd: u32, val: u64, out: &mut Vec<u32>) {
    out.push(movz(rd, (val & 0xFFFF) as u16, 0));
    out.push(movk(rd, ((val >> 16) & 0xFFFF) as u16, 16));
    out.push(movk(rd, ((val >> 32) & 0xFFFF) as u16, 32));
    out.push(movk(rd, ((val >> 48) & 0xFFFF) as u16, 48));
}

// ── MF66 ABI register numbers ────────────────────────────────────────────
const TOS: u32 = 0; // x0
const DSP: u32 = 19; // x19
const RP: u32 = 28; // x28
const CALL_TMP: u32 = 16; // x16 (veneer scratch)
const LR: u32 = 30; // x30
const CELL: i32 = 8;

/// Colon-word prologue (nest): save the link register onto the return stack.
pub fn emit_nest(out: &mut Vec<u32>) {
    out.push(str_pre(LR, RP, -CELL)); // str x30, [x28, #-8]!
}
/// Colon-word epilogue (unnest + return).
pub fn emit_unnest_ret(out: &mut Vec<u32>) {
    out.push(ldr_post(LR, RP, CELL)); // ldr x30, [x28], #8
    out.push(ret());
}
/// Compile a call to `xt` (veneer: works regardless of distance — `bl` range is
/// only ±128 MB). `movz/movk x16, xt; blr x16`.
pub fn emit_call(xt: u64, out: &mut Vec<u32>) {
    load_imm64(CALL_TMP, xt, out);
    out.push(blr(CALL_TMP));
}
/// Compile a literal push: spill the cached TOS, load `n` as the new TOS.
pub fn emit_lit(n: i64, out: &mut Vec<u32>) {
    out.push(str_pre(TOS, DSP, -CELL)); // str x0, [x19, #-8]!  (push old TOS)
    load_imm64(TOS, n as u64, out); // movz/movk x0, n
}

// ── Control-flow encoders (branch offsets are in WORDS, relative to self) ──

/// `mov Xd, Xm`  (= orr Xd, XZR, Xm).
pub fn mov_reg(rd: u32, rm: u32) -> u32 {
    0xAA00_03E0 | ((rm & 0x1F) << 16) | (rd & 0x1F)
}
/// `ldr Xt, [Xn]`  (unsigned offset 0).
pub fn ldr0(rt: u32, rn: u32) -> u32 {
    0xF940_0000 | ((rn & 0x1F) << 5) | (rt & 0x1F)
}
/// `add Xd, Xn, #imm12`.
pub fn add_imm(rd: u32, rn: u32, imm12: u32) -> u32 {
    0x9100_0000 | ((imm12 & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}
/// `cbz Xt, off`  — branch if Xt == 0; `off` is a signed instruction count.
pub fn cbz(rt: u32, off: i32) -> u32 {
    0xB400_0000 | (((off as u32) & 0x7FFFF) << 5) | (rt & 0x1F)
}
/// `b off`  — unconditional branch; `off` is a signed instruction count.
pub fn b(off: i32) -> u32 {
    0x1400_0000 | ((off as u32) & 0x03FF_FFFF)
}
/// `b.<cond> off`  — conditional branch; `off` is a signed instruction count.
pub fn bcond(cond: u32, off: i32) -> u32 {
    0x5400_0000 | (((off as u32) & 0x7FFFF) << 5) | (cond & 0xF)
}
/// Patch a previously-emitted `b.<cond>` (keeping its condition) to `target`.
pub fn patch_bcond(out: &mut [u32], at: usize, target: usize) {
    let cond = out[at] & 0xF;
    out[at] = bcond(cond, target as i32 - at as i32);
}

/// `ldr Xt, [Xn, #imm]`  (unsigned scaled offset; imm is bytes, must be /8).
pub fn ldr_off(rt: u32, rn: u32, imm: u32) -> u32 {
    0xF940_0000 | (((imm / 8) & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rt & 0x1F)
}
/// `str Xt, [Xn, #imm]`  (unsigned scaled offset).
pub fn str_off(rt: u32, rn: u32, imm: u32) -> u32 {
    0xF900_0000 | (((imm / 8) & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rt & 0x1F)
}
/// `add Xd, Xn, Xm`.
pub fn add_reg(rd: u32, rn: u32, rm: u32) -> u32 {
    0x8B00_0000 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}
/// `sub Xd, Xn, Xm`.
pub fn sub_reg(rd: u32, rn: u32, rm: u32) -> u32 {
    0xCB00_0000 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}
/// `eor Xd, Xn, Xm`.
pub fn eor_reg(rd: u32, rn: u32, rm: u32) -> u32 {
    0xCA00_0000 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}
/// `tbz Xt, #bit, off`  — branch if bit `bit` of Xt is 0; `off` = instruction count.
pub fn tbz(rt: u32, bit: u32, off: i32) -> u32 {
    let b5 = (bit >> 5) & 1;
    let b40 = bit & 0x1F;
    (b5 << 31) | 0x3600_0000 | (b40 << 19) | (((off as u32) & 0x3FFF) << 5) | (rt & 0x1F)
}

// ── DO/LOOP (counted loops) — frame on RP is [index@RP, limit@RP+8] ─────────

/// `do` ( limit start -- ): push [index=start, limit] onto RP, drop both.
pub fn emit_do(out: &mut Vec<u32>) {
    out.push(ldr0(9, DSP)); // x9 = limit (NOS)
    out.push(str_pre(9, RP, -CELL)); // push limit (deeper)
    out.push(str_pre(TOS, RP, -CELL)); // push index = start (top)
    out.push(ldr_off(TOS, DSP, CELL as u32)); // new TOS = item below limit
    out.push(add_imm(DSP, DSP, CELL as u32 * 2)); // drop ( limit start )
}
/// `loop`: index++; loop back to `top` unless (index-limit) changed sign.
pub fn emit_loop(out: &mut Vec<u32>, top: usize) {
    out.push(ldr0(9, RP)); // index
    out.push(ldr_off(10, RP, CELL as u32)); // limit
    out.push(sub_reg(11, 9, 10)); // d_old = index - limit
    out.push(add_imm(9, 9, 1)); // index++
    out.push(str_off(9, RP, 0));
    out.push(sub_reg(12, 9, 10)); // d_new
    out.push(eor_reg(11, 11, 12)); // sign change?
    let at = out.len();
    out.push(tbz(11, 63, top as i32 - at as i32)); // no change → loop back
    out.push(add_imm(RP, RP, 16)); // exit: drop the loop frame
}
/// `+loop` ( n -- ): index += n; same signed-crossing termination test.
pub fn emit_plus_loop(out: &mut Vec<u32>, top: usize) {
    out.push(ldr0(9, RP));
    out.push(ldr_off(10, RP, CELL as u32));
    out.push(sub_reg(11, 9, 10)); // d_old
    out.push(add_reg(9, 9, TOS)); // index += n
    out.push(str_off(9, RP, 0));
    out.push(sub_reg(12, 9, 10)); // d_new
    out.push(eor_reg(11, 11, 12));
    out.push(ldr0(TOS, DSP)); // consume n: raise NOS
    out.push(add_imm(DSP, DSP, CELL as u32));
    let at = out.len();
    out.push(tbz(11, 63, top as i32 - at as i32));
    out.push(add_imm(RP, RP, 16));
}

// ── Optimizer lowering encoders ───────────────────────────────────────────
/// `mul Xd, Xn, Xm`.
pub fn mul(rd: u32, rn: u32, rm: u32) -> u32 {
    0x9B00_7C00 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}
/// `and Xd, Xn, Xm`.
pub fn and_reg(rd: u32, rn: u32, rm: u32) -> u32 {
    0x8A00_0000 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}
/// `orr Xd, Xn, Xm`.
pub fn orr_reg(rd: u32, rn: u32, rm: u32) -> u32 {
    0xAA00_0000 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}
/// `sub Xd, Xn, #imm12`.
pub fn sub_imm(rd: u32, rn: u32, imm12: u32) -> u32 {
    0xD100_0000 | ((imm12 & 0xFFF) << 10) | ((rn & 0x1F) << 5) | (rd & 0x1F)
}
/// `cmp Xn, Xm`  (= subs xzr, Xn, Xm).
pub fn cmp_reg(rn: u32, rm: u32) -> u32 {
    0xEB00_0000 | ((rm & 0x1F) << 16) | ((rn & 0x1F) << 5) | 31
}
/// `cmp Xn, #imm12`.
pub fn cmp_imm(rn: u32, imm12: u32) -> u32 {
    0xF100_0000 | ((imm12 & 0xFFF) << 10) | ((rn & 0x1F) << 5) | 31
}
/// `csetm Xd, <cond>`  (Forth flag: all-ones if cond else 0 = csinv Xd,xzr,xzr,!cond).
pub fn csetm(rd: u32, cond: u32) -> u32 {
    let inv = cond ^ 1; // invert low bit of the condition
    0xDA80_0000 | (31 << 16) | ((inv & 0xF) << 12) | (31 << 5) | (rd & 0x1F)
}
/// `ldrb Wt, [Xn]`.
pub fn ldrb0(rt: u32, rn: u32) -> u32 {
    0x3940_0000 | ((rn & 0x1F) << 5) | (rt & 0x1F)
}
/// `strb Wt, [Xn]`.
pub fn strb0(rt: u32, rn: u32) -> u32 {
    0x3900_0000 | ((rn & 0x1F) << 5) | (rt & 0x1F)
}
/// `neg Xd, Xm`  (= sub Xd, xzr, Xm).
pub fn neg(rd: u32, rm: u32) -> u32 {
    0xCB00_03E0 | ((rm & 0x1F) << 16) | (rd & 0x1F)
}
/// `mvn Xd, Xm`  (= orn Xd, xzr, Xm).
pub fn mvn(rd: u32, rm: u32) -> u32 {
    0xAA20_03E0 | ((rm & 0x1F) << 16) | (rd & 0x1F)
}

// AArch64 condition codes.
pub const EQ: u32 = 0;
pub const NE: u32 = 1;
pub const HS: u32 = 2; // unsigned >=
pub const LO: u32 = 3; // unsigned <
pub const HI: u32 = 8; // unsigned >
pub const LS: u32 = 9; // unsigned <=
pub const GE: u32 = 10;
pub const LT: u32 = 11;
pub const GT: u32 = 12;
pub const LE: u32 = 13;
pub const MI: u32 = 4; // negative

const SCRATCH: u32 = 9; // x9

/// Compile a Forth `IF`/`UNTIL`/`WHILE` flag test: consume TOS (raise NOS),
/// remembering the flag, then a `cbz` placeholder (offset 0) on the flag.
/// Returns the index (in `out`) of the `cbz` word, to be patched later.
pub fn emit_flag_test_cbz(out: &mut Vec<u32>) -> usize {
    out.push(mov_reg(SCRATCH, TOS)); // x9 = flag (TOS)
    out.push(ldr0(TOS, DSP)); // TOS = NOS
    out.push(add_imm(DSP, DSP, CELL as u32)); // drop the flag
    let idx = out.len();
    out.push(cbz(SCRATCH, 0)); // placeholder; patch target later
    idx
}
/// Patch a previously-emitted `cbz` (at `at`) to branch to `target` (word index).
pub fn patch_cbz(out: &mut [u32], at: usize, target: usize) {
    out[at] = cbz(SCRATCH, target as i32 - at as i32);
}
/// Patch a previously-emitted `b` (at `at`) to branch to `target` (word index).
pub fn patch_b(out: &mut [u32], at: usize, target: usize) {
    out[at] = b(target as i32 - at as i32);
}

#[cfg(test)]
mod cf_tests {
    use super::*;
    fn asm(text: &str) -> Vec<u32> {
        let m = wfasm::a64::assemble(text).unwrap_or_else(|e| panic!("assemble {text:?}: {e}"));
        m.code.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }
    #[test]
    fn cf_encoders_match_assembler() {
        assert_eq!(vec![mov_reg(9, 0)], asm("mov x9, x0"));
        assert_eq!(vec![ldr0(0, 19)], asm("ldr x0, [x19]"));
        assert_eq!(vec![add_imm(19, 19, 8)], asm("add x19, x19, #8"));
        // cbz/b with a +2-word forward offset (label two instructions ahead)
        assert_eq!(asm("cbz x9, .L\nnop\n.L:")[0], cbz(9, 2));
        assert_eq!(asm("b .L\nnop\n.L:")[0], b(2));
        // backward (negative) offset
        assert_eq!(asm(".L:\nnop\nb .L")[1], b(-1));
        assert_eq!(asm("b.ge .L\nnop\n.L:")[0], bcond(GE, 2));
        assert_eq!(asm("b.eq .L\nnop\n.L:")[0], bcond(EQ, 2));
    }

    #[test]
    fn doloop_encoders_match_assembler() {
        assert_eq!(vec![ldr_off(9, 28, 8)], asm("ldr x9, [x28, #8]"));
        assert_eq!(vec![str_off(9, 28, 0)], asm("str x9, [x28]"));
        assert_eq!(vec![add_reg(9, 9, 0)], asm("add x9, x9, x0"));
        assert_eq!(vec![sub_reg(11, 9, 10)], asm("sub x11, x9, x10"));
        assert_eq!(vec![eor_reg(11, 11, 12)], asm("eor x11, x11, x12"));
        // a64 can't assemble tbz-to-label, so assert against llvm-mc-verified
        // constants (fixed bits match `llvm-mc -triple=aarch64-apple-darwin`).
        assert_eq!(tbz(11, 63, 2), 0xB6F8_004B);
        assert_eq!(tbz(9, 5, 2), 0x3628_0049);
    }

    #[test]
    fn opt_encoders_match_assembler() {
        assert_eq!(vec![mul(0, 0, 9)], asm("mul x0, x0, x9"));
        assert_eq!(vec![and_reg(0, 0, 9)], asm("and x0, x0, x9"));
        assert_eq!(vec![orr_reg(0, 0, 9)], asm("orr x0, x0, x9"));
        assert_eq!(vec![sub_imm(0, 0, 5)], asm("sub x0, x0, #5"));
        assert_eq!(vec![cmp_reg(9, 0)], asm("cmp x9, x0"));
        assert_eq!(vec![cmp_imm(0, 0)], asm("cmp x0, #0"));
        assert_eq!(vec![csetm(0, EQ)], asm("csetm x0, eq"));
        assert_eq!(vec![csetm(0, LT)], asm("csetm x0, lt"));
        assert_eq!(vec![ldrb0(0, 0)], asm("ldrb w0, [x0]"));
        assert_eq!(vec![strb0(9, 0)], asm("strb w9, [x0]"));
        assert_eq!(vec![neg(0, 0)], asm("neg x0, x0"));
        assert_eq!(vec![mvn(0, 0)], asm("mvn x0, x0"));
        assert_eq!(vec![ldr_post(9, 19, 8)], asm("ldr x9, [x19], #8"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asm(text: &str) -> Vec<u32> {
        let m = wfasm::a64::assemble(text).unwrap_or_else(|e| panic!("assemble {text:?}: {e}"));
        m.code.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    #[test]
    fn encoders_match_assembler() {
        assert_eq!(vec![movz(0, 0x1234, 0)], asm("movz x0, #0x1234"));
        assert_eq!(vec![movz(16, 0xABCD, 16)], asm("movz x16, #0xABCD, lsl #16"));
        assert_eq!(vec![movk(0, 0xFFFF, 32)], asm("movk x0, #0xFFFF, lsl #32"));
        assert_eq!(vec![movk(16, 0x1, 48)], asm("movk x16, #0x1, lsl #48"));
        assert_eq!(vec![blr(16)], asm("blr x16"));
        assert_eq!(vec![ret()], asm("ret"));
        assert_eq!(vec![str_pre(30, 28, -8)], asm("str x30, [x28, #-8]!"));
        assert_eq!(vec![ldr_post(30, 28, 8)], asm("ldr x30, [x28], #8"));
        assert_eq!(vec![str_pre(0, 19, -8)], asm("str x0, [x19, #-8]!"));
    }

    #[test]
    fn load_imm64_matches_assembler() {
        let mut out = Vec::new();
        load_imm64(0, 0x1234_5678_9ABC_DEF0, &mut out);
        assert_eq!(
            out,
            asm("movz x0, #0xDEF0\nmovk x0, #0x9ABC, lsl #16\nmovk x0, #0x5678, lsl #32\nmovk x0, #0x1234, lsl #48")
        );
    }
}
