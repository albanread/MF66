//! constant / variable — runtime-constant defining words (OOP foundation).

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

fn out(src: &str) -> String {
    let mut s = Mf66Session::new().unwrap();
    s.eval_out(src).unwrap()
}

#[test]
fn constant_pushes_value() {
    assert_eq!(out("42 constant answer answer ."), "42 ");
    assert_eq!(out("7 constant seven seven seven + ."), "14 ");
}

#[test]
fn variable_fetch_store() {
    assert_eq!(out("variable v 99 v ! v @ ."), "99 ");
    assert_eq!(out("variable v 5 v ! 10 v +! v @ ."), "15 ");
}

#[test]
fn constant_usable_in_colon() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("10 constant ten").unwrap();
    s.eval(": add-ten ten + ;").unwrap();
    assert_eq!(s.eval_out("5 add-ten .").unwrap(), "15 ");
}

#[test]
fn two_variables_distinct() {
    assert_eq!(out("variable a variable b 1 a ! 2 b ! a @ b @ + ."), "3 ");
}
