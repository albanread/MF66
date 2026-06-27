#![cfg(target_os = "macos")]
use mf66::Mf66Session;
#[test] fn marker_rollback() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("marker rollback").unwrap();
    s.eval(": trial-word 12345 ;").unwrap();
    assert_eq!(s.eval_out("trial-word .").unwrap(), "12345 ");
    s.eval("rollback").unwrap();                      // execute the marker
    // trial-word and rollback are gone
    assert_eq!(s.eval_out("[defined] trial-word .").unwrap(), "0 ");
    assert_eq!(s.eval_out("[defined] rollback .").unwrap(), "0 ");
}
#[test] fn marker_reclaims_and_redefines() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("variable keep 99 keep !").unwrap();
    s.eval("marker m").unwrap();
    s.eval(": a 1 ; : b 2 ; variable v 7 v !").unwrap();
    assert_eq!(s.eval_out("a b + v @ + .").unwrap(), "10 ");
    s.eval("m").unwrap();
    // a/b/v forgotten; keep survives
    assert_eq!(s.eval_out("[defined] a [defined] v + .").unwrap(), "0 ");
    assert_eq!(s.eval_out("keep @ .").unwrap(), "99 ");
    // can redefine the forgotten names cleanly
    s.eval(": a 111 ;").unwrap();
    assert_eq!(s.eval_out("a .").unwrap(), "111 ");
}
#[test] fn nested_markers() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("marker outer : x 1 ; marker inner : y 2 ;").unwrap();
    assert_eq!(s.eval_out("x y + .").unwrap(), "3 ");
    s.eval("outer").unwrap();                          // rolls back x, y, AND inner
    assert_eq!(s.eval_out("[defined] x [defined] y [defined] inner + + .").unwrap(), "0 ");
}
