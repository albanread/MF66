#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn defer_is() {
    assert_eq!(out(": hi 42 ; defer foo ' hi is foo foo ."), "42 ");
    // re-point
    assert_eq!(out(": a 1 ; : b 2 ; defer d ' a is d d . ' b is d d ."), "1 2 ");
    // is in compile mode
    assert_eq!(out(": x 7 ; defer dd : set ['] x is dd ; set dd ."), "7 ");
}
#[test] fn twovalue_2to() {
    assert_eq!(out("10 20 2value pt pt . ."), "20 10 ");      // pt → 10 20
    assert_eq!(out("10 20 2value pt 77 88 2to pt pt . ."), "88 77 ");
    assert_eq!(out("1 2 2value q : bump 5 6 2to q ; bump q . ."), "6 5 ");
}
#[test] fn plus_to_and_synonym() {
    assert_eq!(out("5 value v 3 +to v v ."), "8 ");
    assert_eq!(out("5 value v : addten 10 +to v ; addten v ."), "15 ");
    assert_eq!(out(": orig 99 ; synonym alias orig alias ."), "99 ");
}
