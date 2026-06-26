#![cfg(target_os = "macos")]
use mf66::Mf66Session;
#[test] fn structures() {
    let mut s=Mf66Session::new().unwrap();
    s.eval("begin-structure point field: .x field: .y end-structure").unwrap();
    assert_eq!(s.eval_out("point .").unwrap(), "16 ");      // size
    assert_eq!(s.eval_out("0 .x .").unwrap(), "0 ");        // .x offset
    assert_eq!(s.eval_out("0 .y .").unwrap(), "8 ");        // .y offset
    s.eval("create pt point allot").unwrap();
    assert_eq!(s.eval_out("7 pt .x ! pt .x @ .").unwrap(), "7 ");
    assert_eq!(s.eval_out("11 pt .y ! pt .y @ .").unwrap(), "11 ");
    assert_eq!(s.eval_out("pt .x @ .").unwrap(), "7 ");     // .y store didn't clobber .x
}
#[test] fn constant_variable_via_createdoes() {
    let mut s=Mf66Session::new().unwrap();
    s.eval("42 constant answer").unwrap();
    assert_eq!(s.eval_out("answer .").unwrap(), "42 ");
    s.eval("variable v 99 v ! ").unwrap();
    assert_eq!(s.eval_out("v @ .").unwrap(), "99 ");
}
