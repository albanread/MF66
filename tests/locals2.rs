#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn locals_still_correct() {
    assert_eq!(out(": sum3 {: a b c :} a b + c + ; 1 2 3 sum3 ."), "6 ");
    assert_eq!(out(": avg {: x y z :} x y + z + 3 / ; 2 4 6 avg ."), "4 ");
    // store-then-read (cache): increment a local then read it
    assert_eq!(out(": bump {: n :} n 1 + to n n n + ; 5 bump ."), "12 ");  // (6)+(6)=12
    // recurse with locals still works (control flow → not inlined)
    assert_eq!(out(": fact {: n :} n 1 <= if 1 else n n 1 - recurse * then ; 5 fact ."), "120 ");
    // exit inside locals still tears down the frame (called 2000×, no LP drift)
    let mut s = Mf66Session::new().unwrap();
    s.eval(": le {: a b :} a 0= if 111 exit then a b + ;").unwrap();
    for _ in 0..2000 { assert_eq!(s.eval_out("0 5 le .").unwrap(), "111 "); s.reset_input(); }
    assert_eq!(s.eval_out("3 4 le .").unwrap(), "7 ");
}
#[test] fn locals_word_inlines() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": dist2 {: x y :} x x * y y * + ;").unwrap();    // straight-line locals leaf
    s.eval(": use 3 4 dist2 ;").unwrap();                     // should inline dist2 (nested frame)
    assert_eq!(s.eval_out("use .").unwrap(), "25 ");           // 9 + 16
    // nested inline: a locals word calling a locals word
    s.eval(": sq {: a :} a a * ;").unwrap();
    s.eval(": sumsq {: x y :} x sq y sq + ;").unwrap();        // inlines sq twice, nested frames
    assert_eq!(s.eval_out("3 4 sumsq .").unwrap(), "25 ");
}
