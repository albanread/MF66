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
