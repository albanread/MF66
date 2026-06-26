//! MF66 host runtime functions, bound into the kernel as AAPCS64 externs.
//!
//! Phase 1 carries a single smoke helper so `aapcs_call` has something to call.
//! The real runtime surface (`rt_emit`, `rt_type`, `rt_read_line`,
//! `rt_print_int`, …) arrives with the headless boot in Phase 2.

/// `rt_double(n) -> 2n` — a Phase-1 host-call smoke target for `aapcs_call`.
pub extern "C" fn rt_double(n: u64) -> u64 {
    n.wrapping_mul(2)
}

/// The built-in runtime externs every session binds before assembling the
/// kernel. Order is irrelevant; names must match the `bl`/`aapcs_call` targets.
pub fn externs() -> Vec<(&'static str, *const ())> {
    vec![("rt_double", rt_double as *const ())]
}
