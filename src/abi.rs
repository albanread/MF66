//! The MF66 AArch64 ABI — the single source of truth for register homes.
//!
//! Decided in `docs/design/mf66-apple-silicon.md` §2, justified against AAPCS64.
//! Everything that must survive a settle-barrier `rt_*` call is **callee-saved**
//! (`x19–x28`, low-64 of `v8–v15`), so the optimizer's coalesce/promote passes
//! are sound with no spill code at call sites.
//!
//! The kernel macro library and the back-end `RegFile` both reference these
//! names so the JIT'd code and the kernel share one register convention.

/// Top of the data stack. Caller-saved (`x0` = AAPCS64 arg0/ret0), so it is the
/// natural value flowing through `rt_*` calls; spilled to a cell across any call
/// that clobbers it, exactly as WF66 spills `rax`.
pub const TOS: &str = "x0";
/// Data-stack pointer — points at NOS, grows down by 8-byte cells. Callee-saved
/// so it is stable across barriers (precondition for `coalesce_dsp` soundness).
pub const DSP: &str = "x19";
/// User-area base pointer.
pub const UP: &str = "x20";
/// Locals-frame pointer (callee-saved — upgraded from WF66's caller-saved r15).
pub const LP: &str = "x21";
/// Float top-of-stack. Callee-saved (only the low 64 bits of `d8–d15` are
/// preserved — never use the full 128-bit `q8`). `d31` would be WRONG: it is
/// caller-saved and clobbered by every libm call.
pub const FTOS: &str = "d8";
/// Float-stack pointer (callee-saved; may fall back to `[UP + user_FSP]`).
pub const FSP: &str = "x22";
/// Return-stack pointer — a plain 8-byte full descending stack, callee-saved and
/// decoupled from `sp` (AArch64 `bl` puts the return address in x30, not on a
/// stack), so `sp` stays permanently 16-aligned for `bl`/`blr`.
pub const RP: &str = "x28";

/// Caller-saved scratch / parallel-move temporaries — window-local only (a fused
/// window contains no calls), so volatility is fine and the wider pool (7 vs
/// WF66's 6) gives `window_fuse` more freedom.
pub const FUSION_POOL: &[&str] = &["x9", "x10", "x11", "x12", "x13", "x14", "x15"];
/// Hot-cell promotion pool — callee-saved so promoted values survive barriers
/// unconditionally (WF66 used caller-saved r10/r11 and had to barrier-check).
pub const PROMO_POOL: &[&str] = &["x23", "x24"];
/// Float promotion pool — callee-saved low-64; for FP values that must survive a
/// libm call inside a window.
pub const FP_PROMO_POOL: &[&str] = &["d9", "d10", "d11", "d12", "d13", "d14", "d15"];

/// Indirect-call target scratch. AAPCS64 IP0 — clobbered by any `bl`/`blr`
/// (including `MacJit`'s far-call veneers), so only ever used immediately before
/// the call. NEVER place x16/x17 in any allocatable pool.
pub const CALL_TMP: &str = "x16";

/// Registers that MUST NOT appear in any pool or kernel `.masm` file:
/// - `x16`/`x17`: AAPCS64 IP0/IP1 — `MacJit` veneers (`movz/movk x16; br x16`)
///   clobber x16 at relocation time, so treat both as dead across every call.
/// - `x18`: the Darwin platform register — there is no encoder guard, so a CI
///   grep gate must enforce this across the hand-written kernel.
pub const FORBIDDEN: &[&str] = &["x16", "x17", "x18", "w16", "w17", "w18"];

#[cfg(test)]
mod tests {
    use super::*;

    /// No pool may contain a forbidden register, and the pools must be disjoint.
    #[test]
    fn pools_are_disjoint_and_avoid_forbidden() {
        let mut seen = std::collections::HashSet::new();
        for &r in FUSION_POOL.iter().chain(PROMO_POOL).chain([&DSP, &UP, &LP, &FSP, &RP]) {
            assert!(!FORBIDDEN.contains(&r), "{r} is forbidden but in a pool/home");
            assert!(seen.insert(r), "{r} appears in two pools/homes");
        }
        // TOS=x0 and CALL_TMP=x16 are intentionally caller-saved and not pooled.
        assert!(FORBIDDEN.contains(&CALL_TMP), "x16 must be in the forbidden-for-pools set");
    }
}
