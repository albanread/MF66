#![cfg(target_os = "macos")]
use mf66::Mf66Session;
#[test] fn inline_leaf_words() {
    let mut s = Mf66Session::new().unwrap();
    // a small straight-line leaf
    s.eval(": double dup + ;").unwrap();
    s.eval(": quad double double ;").unwrap();      // should inline `double` twice
    assert_eq!(s.eval_out("5 quad .").unwrap(), "20 ");
    // transitive: inline word calling another inline word
    s.eval(": inc 1+ ;").unwrap();
    s.eval(": add3 inc inc inc ;").unwrap();         // inlines inc×3 → +3 (imm-chain!)
    assert_eq!(s.eval_out("10 add3 .").unwrap(), "13 ");
    // body of add3 should be tiny (inlined + imm-chained to one add #3)
    assert!(s.last_body_words() <= 6, "add3 body {} words (want tiny via inline+chain)", s.last_body_words());
    // control-flow words are NOT inlined (still correct)
    s.eval(": clamp dup 0 < if drop 0 then ;").unwrap();
    s.eval(": use-clamp clamp 1+ ;").unwrap();
    assert_eq!(s.eval_out("-5 use-clamp .").unwrap(), "1 ");
    assert_eq!(s.eval_out("7 use-clamp .").unwrap(), "8 ");
}
