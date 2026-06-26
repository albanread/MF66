#![cfg(target_os = "macos")]
use mf66::Mf66Session;
#[test] fn variable_folds_in_compiled_code() {
    let mut s=Mf66Session::new().unwrap();
    s.eval("variable v 5 v !").unwrap();
    // compiled access: v @ / v ! become ldr/str, no veneer call
    s.eval(": bump v @ 1+ v ! ;").unwrap();
    s.eval("bump bump bump").unwrap();
    assert_eq!(s.eval_out("v @ .").unwrap(), "8 ");
    // a word reading the variable twice
    s.eval(": dbl v @ v @ + ;").unwrap();
    assert_eq!(s.eval_out("dbl .").unwrap(), "16 ");
    // the compiled body should be tiny (no calls) — sanity on the win
    s.eval(": touch v @ drop ;").unwrap();
    assert!(s.last_body_words() <= 10, "var access body {} words", s.last_body_words());
}
