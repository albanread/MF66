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

#[test] fn const_index_pick_roll_compiled() {
    let mut s = Mf66Session::new().unwrap();
    // N pick with literal N → static motion (0=dup,1=over,2=copy NNOS)
    s.eval(": p0 0 pick ;").unwrap();
    assert_eq!(s.eval_out("9 p0 . .").unwrap(), "9 9 ");      // dup
    s.eval(": p2 2 pick ;").unwrap();
    assert_eq!(s.eval_out("10 20 30 p2 . . . .").unwrap(), "10 30 20 10 "); // copy depth-2 (10)
    // N roll with literal N → static permutation (2 roll = rot)
    s.eval(": r2 2 roll ;").unwrap();
    assert_eq!(s.eval_out("1 2 3 r2 . . .").unwrap(), "1 3 2 ");
    s.eval(": r1 1 roll ;").unwrap();
    assert_eq!(s.eval_out("5 9 r1 . .").unwrap(), "5 9 ");    // swap → 9 5 on stack, print 5 9
    s.eval(": r3 3 roll ;").unwrap();
    assert_eq!(s.eval_out("1 2 3 4 r3 . . . .").unwrap(), "1 4 3 2 "); // move depth-3 (1) to top
}
