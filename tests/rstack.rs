//! Return-stack primitives (kernel/rstack.masm). The corpus has no direct `.t`
//! files for these (they're normally exercised via colon words / eval), so they
//! get explicit round-trip tests here. The return stack persists across
//! `forth_main` calls via `user_RSP_CURRENT`, so `>r` in one call and `r>` in the
//! next see the same stack. `stack()` is top-first.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn to_r_then_r_from_roundtrips() {
    let mut s = Mf66Session::new().unwrap();
    s.push(5);
    s.call("to_r").unwrap(); // ( 5 -- )  R: 5
    assert_eq!(s.depth(), 0, "data stack empty after >r");
    s.call("r_from").unwrap(); // ( -- 5 )
    assert_eq!(s.stack(), vec![5]);
}

#[test]
fn r_fetch_peeks_without_popping() {
    let mut s = Mf66Session::new().unwrap();
    s.push(42);
    s.call("to_r").unwrap();
    s.call("r_fetch").unwrap(); // ( -- 42 ) R: 42 (still there)
    assert_eq!(s.stack(), vec![42]);
    s.call("r_from").unwrap(); // ( -- 42 ) pops the remaining copy
    assert_eq!(s.stack(), vec![42, 42]);
}

#[test]
fn dup_to_r_keeps_data_copy() {
    let mut s = Mf66Session::new().unwrap();
    s.push(7);
    s.call("dup_to_r").unwrap(); // ( 7 -- 7 ) R: 7
    assert_eq!(s.stack(), vec![7]);
    s.call("r_from").unwrap();
    assert_eq!(s.stack(), vec![7, 7]);
}

#[test]
fn rdrop_discards_top_of_return_stack() {
    let mut s = Mf66Session::new().unwrap();
    s.push(1);
    s.push(2);
    s.call("to_r").unwrap(); // ( 1 2 -- 1 ) R: 2
    s.call("rdrop").unwrap(); // R: (empty)
    assert_eq!(s.stack(), vec![1], "2 went to R then dropped; 1 remains");
}

#[test]
fn two_to_r_then_two_r_from_preserves_order() {
    let mut s = Mf66Session::new().unwrap();
    s.push(1);
    s.push(2);
    s.call("two_to_r").unwrap(); // ( 1 2 -- ) R: 1 2
    assert_eq!(s.depth(), 0);
    s.call("two_r_from").unwrap(); // ( -- 1 2 )
    assert_eq!(s.stack(), vec![2, 1]); // top-first: 2 on top
}

#[test]
fn two_r_fetch_peeks_pair() {
    let mut s = Mf66Session::new().unwrap();
    s.push(10);
    s.push(20);
    s.call("two_to_r").unwrap();
    s.call("two_r_fetch").unwrap(); // ( -- 10 20 ) R unchanged
    assert_eq!(s.stack(), vec![20, 10]);
    s.call("two_rdrop").unwrap(); // R: (empty)
    assert_eq!(s.stack(), vec![20, 10], "2rdrop leaves data alone");
}

#[test]
fn rp_fetch_is_stable_and_store_roundtrips() {
    let mut s = Mf66Session::new().unwrap();
    // rp@ twice with no intervening R ops → same pointer.
    s.call("rp_fetch").unwrap();
    s.call("rp_fetch").unwrap();
    let v = s.stack();
    assert_eq!(v.len(), 2);
    assert_eq!(v[0], v[1], "rp@ stable between calls");
    // Save rp, stash a value on R, then restore rp → the value is discarded.
    let mut s = Mf66Session::new().unwrap();
    s.call("rp_fetch").unwrap(); // ( -- rp0 )
    s.push(99);
    s.call("to_r").unwrap(); // R: 99 ; data: rp0
    s.call("rp_store").unwrap(); // rp! rp0 → R unwound, data empty
    assert_eq!(s.depth(), 0);
}
