//! Compile-time control flow: if/else/then, begin/until, begin/while/repeat —
//! immediate directives that emit + patch AArch64 branches in the colon body.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn if_else_then() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": sgn? 0 < if -1 else 1 then ;").unwrap();
    assert_eq!(s.eval_out("-5 sgn? .").unwrap(), "-1 ");
    s.reset();
    assert_eq!(s.eval_out("5 sgn? .").unwrap(), "1 ");
}

#[test]
fn if_then_no_else() {
    let mut s = Mf66Session::new().unwrap();
    // double the value only if it's negative
    s.eval(": absish dup 0 < if negate then ;").unwrap();
    assert_eq!(s.eval_out("-7 absish .").unwrap(), "7 ");
    s.reset();
    assert_eq!(s.eval_out("7 absish .").unwrap(), "7 ");
}

#[test]
fn begin_until() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": cd begin 1- dup 0= until ;").unwrap(); // count n down to 0
    s.eval("5 cd").unwrap();
    assert_eq!(s.stack(), vec![0]);
}

#[test]
fn begin_while_repeat() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": cd begin dup 0> while 1- repeat ;").unwrap(); // n -> 0
    s.eval("7 cd").unwrap();
    assert_eq!(s.stack(), vec![0]);
}

#[test]
fn unbalanced_control_errors() {
    let mut s = Mf66Session::new().unwrap();
    assert!(s.eval(": bad if 1 ;").is_err()); // `if` with no `then`
}

#[test]
fn factorial_with_loop() {
    // : fact ( n -- n! )  1 swap begin dup 0> while dup >r ... ;  use rstack
    // simpler accumulator: 1 over begin ... — do iterative factorial via swap/rot
    let mut s = Mf66Session::new().unwrap();
    s.eval(": fact 1 swap begin dup 0> while dup >r * r> 1- repeat drop ;").unwrap();
    assert_eq!(s.eval_out("5 fact .").unwrap(), "120 ");
}
