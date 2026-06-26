//! MF66's AArch64 Rust-side assembler macros, registered with the front-end
//! `Assembler`. The built-in `wfasm::asm::macros::stk` emits x86 (`add/sub rbp`),
//! so MF66 supplies an AArch64 equivalent that adjusts DSP (x19).

use wfasm::asm::expand::RustMacroCtx;

/// `stk(in, out)` — emit the data-stack-pointer adjustment for a primitive with
/// signature `(in -> out)`. DSP=x19 grows downward, so a net push lowers it and
/// a net pop raises it. Cell size comes from `@assign cell = N` (default 8).
///
/// Register with `asm.register_macro("stk", mf66::asm_macros::stk)`.
///
/// ```text
/// stk(2, 1)  →  add x19, x19, #8     (net pop of one cell)
/// stk(0, 1)  →  sub x19, x19, #8     (net push of one cell)
/// stk(1, 1)  →  (nothing)
/// ```
pub fn stk(ctx: &mut RustMacroCtx<'_>) -> Result<(), String> {
    if ctx.count() != 2 {
        return Err(format!("stk: expected 2 args (in, out), got {}", ctx.count()));
    }
    let in_count = ctx.parse_int(0)?;
    let out_count = ctx.parse_int(1)?;
    if in_count < 0 || out_count < 0 {
        return Err(format!(
            "stk: counts must be non-negative (got in={in_count}, out={out_count})"
        ));
    }
    let cell = ctx.lookup_int("cell").unwrap_or(8);
    let delta = (out_count - in_count) * cell;
    use std::cmp::Ordering;
    match delta.cmp(&0) {
        // Net push: DSP grows down.
        Ordering::Greater => ctx.emit_line(&format!("sub x19, x19, #{delta}\n"))?,
        // Net pop: DSP rises.
        Ordering::Less => ctx.emit_line(&format!("add x19, x19, #{}\n", -delta))?,
        Ordering::Equal => {}
    }
    Ok(())
}
