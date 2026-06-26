//! MF66 host runtime functions, bound into the kernel as AAPCS64 externs.
//!
//! Output goes to a thread-local capture buffer (the session runs single-threaded
//! for JIT, so one thread-local suffices); `Mf66Session::eval_out` clears it,
//! runs, and takes the result. This mirrors WF66's `rt_emit`/`rt_type`/
//! `rt_print_int` and their exact formatting so the eval corpus matches.

use std::cell::RefCell;

thread_local! {
    static CAPTURE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Clear the capture buffer (before an `eval`).
pub fn capture_clear() {
    CAPTURE.with(|b| b.borrow_mut().clear());
}

/// Append a host string to the capture buffer (e.g. the REPL's ` ok`).
pub fn capture_str(s: &str) {
    CAPTURE.with(|b| b.borrow_mut().extend_from_slice(s.as_bytes()));
}

/// Take the captured output as a string (after an `eval`).
pub fn capture_take() -> String {
    CAPTURE.with(|b| {
        let bytes = std::mem::take(&mut *b.borrow_mut());
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// `emit ( c -- )` host side: append one byte.
pub extern "C" fn rt_emit(ch: u64) -> u64 {
    CAPTURE.with(|b| b.borrow_mut().push(ch as u8));
    0
}

/// `type ( addr u -- )` host side: append `len` bytes from `addr`.
pub extern "C" fn rt_type(addr: u64, len: u64) -> u64 {
    if addr != 0 && len > 0 {
        let slice = unsafe { std::slice::from_raw_parts(addr as *const u8, len as usize) };
        CAPTURE.with(|b| b.borrow_mut().extend_from_slice(slice));
    }
    0
}

/// `. ( n -- )` host side: signed number in `base`, then ONE trailing space
/// (byte-identical to WF66's `rt_print_int`).
pub extern "C" fn rt_print_int(n: u64, base: u64) -> u64 {
    let s = n as i64;
    let b = if (2..=36).contains(&base) { base as u32 } else { 10 };
    let out = if b == 10 {
        format!("{s} ")
    } else {
        let (neg, mag) = if s < 0 {
            (true, (s as i128).unsigned_abs())
        } else {
            (false, s as u128)
        };
        let mut digits = Vec::with_capacity(24);
        let mut v = mag;
        if v == 0 {
            digits.push(b'0');
        }
        while v > 0 {
            let d = (v % b as u128) as u8;
            digits.push(if d < 10 { b'0' + d } else { b'A' + (d - 10) });
            v /= b as u128;
        }
        let mut out = String::new();
        if neg {
            out.push('-');
        }
        for &d in digits.iter().rev() {
            out.push(d as char);
        }
        out.push(' ');
        out
    };
    CAPTURE.with(|b| b.borrow_mut().extend_from_slice(out.as_bytes()));
    0
}

/// `rt_double(n) -> 2n` — a Phase-1 host-call smoke target for `aapcs_call`.
pub extern "C" fn rt_double(n: u64) -> u64 {
    n.wrapping_mul(2)
}

/// `f.` — print a float in Forth style (shortest round-trip, trailing space).
pub extern "C" fn rt_print_float(x: f64) {
    capture_str(&format!("{x} "));
}

// Transcendental math for the FP word set (libm via Rust's f64 methods, which
// lower to the platform math library). One-argument:
pub extern "C" fn rt_fsin(x: f64) -> f64 { x.sin() }
pub extern "C" fn rt_fcos(x: f64) -> f64 { x.cos() }
pub extern "C" fn rt_ftan(x: f64) -> f64 { x.tan() }
pub extern "C" fn rt_fexp(x: f64) -> f64 { x.exp() }
pub extern "C" fn rt_fln(x: f64) -> f64 { x.ln() }
pub extern "C" fn rt_flog(x: f64) -> f64 { x.log10() }
pub extern "C" fn rt_fatan(x: f64) -> f64 { x.atan() }
pub extern "C" fn rt_fasin(x: f64) -> f64 { x.asin() }
pub extern "C" fn rt_facos(x: f64) -> f64 { x.acos() }
// Two-argument (a in d0, b in d1):
pub extern "C" fn rt_fpow(a: f64, b: f64) -> f64 { a.powf(b) }
pub extern "C" fn rt_fatan2(a: f64, b: f64) -> f64 { a.atan2(b) }

/// The built-in runtime externs every session binds before assembling the
/// kernel. Names must match the `bl`/`aapcs_call` targets in the kernel.
pub fn externs() -> Vec<(&'static str, *const ())> {
    vec![
        ("rt_double", rt_double as *const ()),
        ("rt_emit", rt_emit as *const ()),
        ("rt_type", rt_type as *const ()),
        ("rt_print_int", rt_print_int as *const ()),
        ("rt_print_float", rt_print_float as *const ()),
        ("rt_fsin", rt_fsin as *const ()),
        ("rt_fcos", rt_fcos as *const ()),
        ("rt_ftan", rt_ftan as *const ()),
        ("rt_fexp", rt_fexp as *const ()),
        ("rt_fln", rt_fln as *const ()),
        ("rt_flog", rt_flog as *const ()),
        ("rt_fatan", rt_fatan as *const ()),
        ("rt_fasin", rt_fasin as *const ()),
        ("rt_facos", rt_facos as *const ()),
        ("rt_fpow", rt_fpow as *const ()),
        ("rt_fatan2", rt_fatan2 as *const ()),
    ]
}
