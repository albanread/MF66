#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn float_locals() {
    assert_eq!(out(": t1 {: | float x :} 3e to x x x f* f. ; t1"), "9 ");        // 3²
    assert_eq!(out(": sq {: float x :} x x f* ; 4e sq f."), "16 ");              // float input
    assert_eq!(out(": hyp {: float x float y :} x x f* y y f* f+ fsqrt ; 3e 4e hyp f."), "5 ");
    assert_eq!(out(": scale {: n float x :} x n s>d d>f f* ; 4e 3 scale f."), "12 "); // mixed
}
#[test] fn float_locals_inline() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": fsq {: float x :} x x f* ;").unwrap();
    s.eval(": sumsq {: float a float b :} a fsq b fsq f+ ;").unwrap();           // inlines fsq twice
    assert_eq!(s.eval_out("3e 4e sumsq f.").unwrap(), "25 ");
}
