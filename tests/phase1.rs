//! Phase 1 — kernel macro library + headless boot path (design §8).
//!
//! Drives real `proc(...)…endp()` primitives, assembled from `kernel/*.masm`
//! through the front-end (+ the AArch64 `stk` macro) and `MacJit`, via the
//! AArch64 `forth_main` wire-format translator. Proves the macro library
//! (register homes, proc/endp/next, stk, aapcs_call) and the forth_main
//! prologue/epilogue (callee-saved save/restore, sp↔return-stack switch) work
//! end-to-end. `stack()` is top-first.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn phase1_plus() {
    let mut s = Mf66Session::new().unwrap();
    s.push(7);
    s.push(11);
    s.call("plus").unwrap();
    assert_eq!(s.stack(), vec![18]); // 11 7 +
}

#[test]
fn phase1_one_plus() {
    let mut s = Mf66Session::new().unwrap();
    s.push(41);
    s.call("one_plus").unwrap();
    assert_eq!(s.stack(), vec![42]);
}

#[test]
fn phase1_stack_ops() {
    let mut s = Mf66Session::new().unwrap();
    s.push(1);
    s.push(2);
    s.push(3);
    assert_eq!(s.stack(), vec![3, 2, 1]);
    s.call("dup_").unwrap(); // 1 2 3 3
    assert_eq!(s.stack(), vec![3, 3, 2, 1]);
    s.call("drop_").unwrap(); // 1 2 3
    assert_eq!(s.stack(), vec![3, 2, 1]);
    s.call("swap_").unwrap(); // 1 3 2
    assert_eq!(s.stack(), vec![2, 3, 1]);
}

/// Exercises `aapcs_call` (LR save + sp realign) + a bound host extern, all the
/// way through forth_main's sp↔return-stack switch.
#[test]
fn phase1_host_call_via_aapcs() {
    let mut s = Mf66Session::new().unwrap();
    s.push(21);
    s.call("rt_double_word").unwrap();
    assert_eq!(s.stack(), vec![42]);
}

/// State must persist across separate forth_main invocations (DSP saved via
/// user_DSP_SAVE; the data stack lives in the region, not in registers).
#[test]
fn phase1_state_persists_across_calls() {
    let mut s = Mf66Session::new().unwrap();
    s.push(40);
    s.call("one_plus").unwrap();
    s.call("one_plus").unwrap();
    assert_eq!(s.stack(), vec![42]);
    assert_eq!(s.depth(), 1);
}

#[test]
fn phase1_reset_clears_stack() {
    let mut s = Mf66Session::new().unwrap();
    s.push(1);
    s.push(2);
    s.call("plus").unwrap();
    assert_eq!(s.depth(), 1);
    s.reset();
    assert_eq!(s.depth(), 0);
    assert_eq!(s.stack(), Vec::<i64>::new());
}
