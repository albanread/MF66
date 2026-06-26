#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn prims() {
    assert_eq!(out("1 2 3 rot . . ."), "1 3 2 ");      // ( 1 2 3 -- 2 3 1 ) print top-first: 1 3 2
    assert_eq!(out("1 2 3 -rot . . ."), "2 1 3 ");      // ( 1 2 3 -- 3 1 2 )
    assert_eq!(out("5 ?dup . ."), "5 5 ");
    assert_eq!(out("0 ?dup ."), "0 ");
    assert_eq!(out("1 2 3 depth ."), "3 ");
    assert_eq!(out("10 20 30 2 pick . . . ."), "10 30 20 10 ");
    assert_eq!(out("1 2 3 2 roll . . ."), "1 3 2 ");    // ( 1 2 3 -- 2 3 1 )
}

#[test] fn optimized_in_colon() {
    let mut s = Mf66Session::new().unwrap();
    // rot/-rot/tuck compiled (modeled as register motion, not calls)
    s.eval(": r3 rot ;").unwrap();
    assert_eq!(s.eval_out("1 2 3 r3 . . .").unwrap(), "1 3 2 ");
    s.eval(": mr -rot ;").unwrap();
    assert_eq!(s.eval_out("1 2 3 mr . . .").unwrap(), "2 1 3 ");
    s.eval(": tk tuck ;").unwrap();
    assert_eq!(s.eval_out("7 8 tk . . .").unwrap(), "8 7 8 ");
    // cancellation: swap swap is a no-op; rot -rot is a no-op
    s.eval(": ss swap swap ;").unwrap();
    assert_eq!(s.eval_out("5 9 ss . .").unwrap(), "9 5 ");
    s.eval(": rr rot -rot ;").unwrap();
    assert_eq!(s.eval_out("1 2 3 rr . . .").unwrap(), "3 2 1 ");
    // rot combined with arithmetic (motion + windowing)
    s.eval(": sum3 rot + + ;").unwrap();
    assert_eq!(s.eval_out("10 20 30 sum3 .").unwrap(), "60 ");
}
