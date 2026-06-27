#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn char_and_bracket_char() {
    assert_eq!(out("char A ."), "65 ");
    assert_eq!(out("char abc ."), "97 ");
    assert_eq!(out(": c [char] Z ; c ."), "90 ");
}
#[test] fn locals_introspection() {
    assert_eq!(out("lp@ lp0@ = ."), "-1 ");
    assert_eq!(out("lp0@ lp-limit - ."), "524288 ");
    assert_eq!(out("lp-smoke ."), "42 ");
    assert_eq!(out("lp-smoke lp-smoke + ."), "84 ");
    assert_eq!(out("lp-smoke lp@ lp0@ = . ."), "-1 42 ");   // 42 left, then -1 on top
    assert_eq!(out("0 ms lp@ lp0@ = ."), "-1 ");
}
