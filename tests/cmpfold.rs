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

#[test] fn min_max_inline_and_fold() {
    let mut s=Mf66Session::new().unwrap();
    // runtime min/max via cmp+csel
    assert_eq!(s.eval_out("5 3 max . 5 3 min .").unwrap(), "5 3 ");
    assert_eq!(s.eval_out("3 5 max . 3 5 min .").unwrap(), "5 3 ");
    assert_eq!(s.eval_out("-7 2 max . -7 2 min .").unwrap(), "2 -7 ");
    // unsigned: -1 is the largest unsigned
    assert_eq!(s.eval_out("-1 1 umax . -1 1 umin .").unwrap(), "-1 1 ");
    // const-fold inside a colon def → tiny body
    s.eval(": clamp10 10 min 0 max ;").unwrap();
    assert_eq!(s.eval_out("15 clamp10 . -3 clamp10 . 7 clamp10 .").unwrap(), "10 0 7 ");
    s.eval(": k 5 3 max ;").unwrap();
    assert_eq!(s.eval_out("k .").unwrap(), "5 ");
    assert!(s.last_body_words() <= 10, "folded max body {} words", s.last_body_words());
}
