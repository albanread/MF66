//! Local variables: `{: in… | uninit… :}` declares an LP-relative frame; locals
//! are read by name and written with `to`; the frame is freed at `;`.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn locals_inputs_and_uninit() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": tp {: x y | t :} x y + to t t . ;").unwrap();
    assert_eq!(s.eval_out("7 8 tp").unwrap(), "15 ");
}

#[test]
fn locals_to_reassigns() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": tt {: a :} a . 99 to a a . ;").unwrap();
    assert_eq!(s.eval_out("42 tt").unwrap(), "42 99 ");
}

#[test]
fn locals_with_arithmetic_and_order() {
    let mut s = Mf66Session::new().unwrap();
    // inputs bind in declaration order: 10 20 avg → a=10 b=20 → (10+20)/2 = 15
    s.eval(": avg {: a b :} a b + 2 / ;").unwrap();
    assert_eq!(s.eval_out("10 20 avg .").unwrap(), "15 ");
}

#[test]
fn locals_in_a_loop() {
    let mut s = Mf66Session::new().unwrap();
    // sum 0..n-1 using a local accumulator
    s.eval(": sum {: n :} 0 n 0 do i + loop ;").unwrap();
    assert_eq!(s.eval_out("5 sum .").unwrap(), "10 "); // 0+1+2+3+4
}
