//! catch / throw — execute under a handler, unwind on a non-zero throw.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

fn out(src: &str) -> String {
    let mut s = Mf66Session::new().unwrap();
    s.eval_out(src).unwrap()
}

#[test]
fn catch_no_throw_returns_zero() {
    // ' word catch leaves the word's results then 0
    assert_eq!(out(": good 42 ; ' good catch . ."), "0 42 ");
}

#[test]
fn catch_catches_a_throw() {
    assert_eq!(out(": bad 7 throw ; ' bad catch ."), "7 ");
}

#[test]
fn throw_zero_is_a_noop() {
    assert_eq!(out(": ok0 0 throw 99 ; ' ok0 catch . ."), "0 99 ");
}

#[test]
fn throw_unwinds_nested_calls_and_restores_depth() {
    // deep throw skips intervening frames; the stack is restored to pre-call
    let mut s = Mf66Session::new().unwrap();
    s.eval(": inner 5 throw ;").unwrap();
    s.eval(": outer 1 2 inner 3 ;").unwrap(); // 1 2 pushed, then inner throws
    // before catch we put a sentinel 99 on the stack; catch restores DSP to here
    assert_eq!(s.eval_out("99 ' outer catch . .").unwrap(), "5 99 ");
}

#[test]
fn throw_inside_a_conditional() {
    assert_eq!(
        out(": maybe dup 0< if -9 throw then ; : t ' maybe catch ; -3 t . ."),
        "-9 -3 "
    );
}
