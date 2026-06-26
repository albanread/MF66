//! Phase 0 — substrate smoke test (docs/design/mf66-apple-silicon.md §8).
//!
//! Proves the JASM AArch64 backend works end-to-end from the MF66 crate, with no
//! WF66 code involved: hand-written AArch64 (using the MF66 ABI, TOS=x0) →
//! `wfasm::a64::assemble` → `MacJit` (mmap MAP_JIT + W^X toggle + icache) →
//! call, including an AAPCS64 host callback routed through a far-call veneer.
//! This de-risks the loader/W^X/veneer path the whole port stands on.

#![cfg(target_os = "macos")]

use mf66::Mf66Jit;

/// A leaf word: TOS = 42, then double it → 84. The MF66 ABI keeps TOS in x0.
#[test]
fn phase0_leaf_word_runs() {
    let mut jit = Mf66Jit::new();
    jit.add_asm(
        "\
.globl mf66_double
mf66_double:
mov x0, #42
add x0, x0, x0
ret
",
    )
    .unwrap();
    let f: extern "C" fn() -> u64 = unsafe { jit.lookup_fn("mf66_double").unwrap() };
    assert_eq!(f(), 84, "(42)*2");
}

extern "C" fn rt_double(n: u64) -> u64 {
    n * 2
}

/// A word that calls back into a Rust host function (AAPCS64: arg/ret in x0).
/// `rt_double` lives in the test binary, typically >128 MB from the JIT region,
/// so the `bl` is resolved through `MacJit`'s absolute veneer — exercising the
/// exact far-call path MF66's settle-barrier `rt_*` calls will use.
#[test]
fn phase0_host_callback_via_veneer() {
    let mut jit = Mf66Jit::new();
    jit.define_extern("rt_double", rt_double as *const ()).unwrap();
    jit.add_asm(
        "\
.globl mf66_via_host
mf66_via_host:
stp x29, x30, [sp, #-16]!
mov x0, #21
bl rt_double
ldp x29, x30, [sp], #16
ret
",
    )
    .unwrap();
    let f: extern "C" fn() -> u64 = unsafe { jit.lookup_fn("mf66_via_host").unwrap() };
    assert_eq!(f(), 42, "rt_double(21)");
}

/// A tiny data-stack idiom using the MF66 ABI register homes (TOS=x0, DSP=x19):
/// caller passes a stack buffer pointer in x0; the word pushes 7 then 11 and
/// adds them (`11 7 + = 18`), returning the result in x0. Proves DSP-relative
/// `str`/`ldr` cell addressing — the shape every kernel primitive uses.
#[test]
fn phase0_data_stack_idiom() {
    let mut jit = Mf66Jit::new();
    // fn(dsp_top: *mut u64) -> u64
    //   x19 = DSP. push 7 (store to [x19], dsp-=8), TOS=7; push 11, TOS=11;
    //   '+' : add TOS, NOS  (TOS=11, NOS=[x19+8] after the second push... )
    // Keep it simple & self-contained: emulate `7 11 +` with explicit cells.
    jit.add_asm(
        "\
.globl mf66_add_demo
mf66_add_demo:
mov x19, x0          // DSP = caller buffer top
mov x0, #7           // TOS = 7
str x0, [x19, #-8]!  // push: store TOS to NOS slot, DSP -= 8
mov x0, #11          // TOS = 11
ldr x1, [x19]        // x1 = NOS (=7)
add x0, x0, x1       // TOS = 11 + 7 = 18
add x19, x19, #8     // drop NOS (stack balance)
ret
",
    )
    .unwrap();
    let f: extern "C" fn(*mut u64) -> u64 = unsafe { jit.lookup_fn("mf66_add_demo").unwrap() };
    let mut buf = [0u64; 16];
    let top = unsafe { buf.as_mut_ptr().add(8) }; // mid-buffer so it can grow down
    assert_eq!(f(top), 18, "11 7 +");
}
