//! Scalar floating point: FP stack (FTOS=d8, FSP=x22), arithmetic, f., literals.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

fn out(src: &str) -> String {
    let mut s = Mf66Session::new().unwrap();
    s.eval_out(src).unwrap()
}

#[test]
fn fp_arithmetic_interpreted() {
    assert_eq!(out("1.5e0 2.5e0 f+ f."), "4 ");
    assert_eq!(out("3.0e0 2.0e0 f- f."), "1 ");
    assert_eq!(out("1.5e0 2.0e0 f* f."), "3 ");
    assert_eq!(out("10.0e0 4.0e0 f/ f."), "2.5 ");
    assert_eq!(out("5.0e0 fnegate f."), "-5 ");
}

#[test]
fn fp_literals_without_exponent() {
    assert_eq!(out("1.5 2.25 f+ f."), "3.75 ");
}

#[test]
fn fp_stack_ops() {
    assert_eq!(out("1.5e0 fdup f+ f."), "3 "); // dup then add
    assert_eq!(out("2.0e0 7.0e0 fswap f. f."), "2 7 "); // 7 then 2
    assert_eq!(out("3.0e0 9.0e0 fover f. f. f."), "3 9 3 ");
    assert_eq!(out("1.0e0 2.0e0 3.0e0 fdepth . fdrop fdrop fdrop"), "3 ");
}

#[test]
fn fp_in_a_colon_definition() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": third 3.0e0 f/ ;").unwrap();
    assert_eq!(s.eval_out("9.0e0 third f.").unwrap(), "3 ");
}

#[test]
fn fp_store_and_fetch() {
    // f! / f@ round-trip through a data-space address (pad scratch)
    let mut s = Mf66Session::new().unwrap();
    let addr = s.pad_base() + 0x80;
    s.push(addr as i64);
    s.eval("2.5e0").unwrap(); // ( F: 2.5 )  data: addr
    s.call("f_store").unwrap(); // [addr] = 2.5
    s.push(addr as i64);
    s.call("f_fetch").unwrap(); // ( F: 2.5 )
    assert_eq!(s.eval_out("f.").unwrap(), "2.5 ");
}
