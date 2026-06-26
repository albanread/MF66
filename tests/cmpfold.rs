#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn const_fold_comparisons() {
    assert_eq!(out("2 3 < ."), "-1 ");
    assert_eq!(out("5 3 < ."), "0 ");
    assert_eq!(out("4 4 = ."), "-1 ");
    assert_eq!(out("4 4 <> ."), "0 ");
    assert_eq!(out("5 0= ."), "0 ");
    assert_eq!(out("0 0= ."), "-1 ");
    assert_eq!(out("-3 0< ."), "-1 ");
    assert_eq!(out("7 0> ."), "-1 ");
}
#[test] fn const_fold_logical() {
    assert_eq!(out("6 3 and ."), "2 ");
    assert_eq!(out("4 1 or ."), "5 ");
    assert_eq!(out("5 invert ."), "-6 ");
    assert_eq!(out("5 negate ."), "-5 ");
}
#[test] fn immediate_and_folded_in_colon() {
    let mut s=Mf66Session::new().unwrap();
    s.eval(": small? 10 < ;").unwrap();         // x 10 < → cmp x,#10
    assert_eq!(s.eval_out("5 small? .").unwrap(), "-1 ");
    assert_eq!(s.eval_out("15 small? .").unwrap(), "0 ");
    s.eval(": k 2 3 < ;").unwrap();              // fully const → folds
    assert_eq!(s.eval_out("k .").unwrap(), "-1 ");
    assert!(s.last_body_words() <= 10, "folded cmp body {} words", s.last_body_words());
    s.eval(": pos? 0 > ;").unwrap();             // x 0 > → cmp x,#0
    assert_eq!(s.eval_out("3 pos? . -2 pos? .").unwrap(), "-1 0 ");
}

#[test] fn negate_comparison() {
    let mut s=Mf66Session::new().unwrap();
    s.eval(": ge < 0= ;").unwrap();          // a b < 0=  ==  a b >=
    assert_eq!(s.eval_out("5 3 ge .").unwrap(), "-1 ");   // 5>=3
    assert_eq!(s.eval_out("3 5 ge .").unwrap(), "0 ");
    assert_eq!(s.eval_out("4 4 ge .").unwrap(), "-1 ");
    s.eval(": ne = 0= ;").unwrap();           // = 0= == <>
    assert_eq!(s.eval_out("4 4 ne .").unwrap(), "0 ");
    assert_eq!(s.eval_out("4 5 ne .").unwrap(), "-1 ");
}
