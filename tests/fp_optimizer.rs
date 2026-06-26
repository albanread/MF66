#![cfg(target_os = "macos")]
use mf66::Mf66Session;
#[test] fn compiled_fp() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": sq fdup f* ;").unwrap();
    assert_eq!(s.eval_out("3e sq f.").unwrap(), "9 ");
    s.eval(": avg f+ 2e f/ ;").unwrap();
    assert_eq!(s.eval_out("10e 20e avg f.").unwrap(), "15 ");
    s.eval(": poly fdup f* fdup f* ;").unwrap();   // x^4
    assert_eq!(s.eval_out("2e poly f.").unwrap(), "16 ");
    // FP motion + arith stays in d-registers; verify a chain is correct
    s.eval(": expr 1e f+ fdup f* fsqrt ;").unwrap();  // sqrt((x+1)^2) = |x+1|
    assert_eq!(s.eval_out("3e expr f.").unwrap(), "4 ");
    // mixed data + FP in one word
    s.eval(": mix dup . fdup f+ ;").unwrap();         // print int, double the float
    assert_eq!(s.eval_out("7 5e mix f.").unwrap(), "7 10 ");
}
