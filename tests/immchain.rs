#![cfg(target_os = "macos")]
use mf66::Mf66Session;
#[test] fn immediate_immediate_chaining() {
    let mut s=Mf66Session::new().unwrap();
    // runtime base, chained increments → must collapse to one add #4
    s.eval(": a 1+ 1+ 1+ 1+ ;").unwrap();
    let wa = s.last_body_words();
    assert_eq!(s.eval_out("3 a .").unwrap(), "7 ");
    s.eval(": b 7 + 3 + ;").unwrap();                 // x + 10
    let wb = s.last_body_words();
    assert_eq!(s.eval_out("5 b .").unwrap(), "15 ");
    s.eval(": c 10 - 4 - ;").unwrap();                // x - 14
    assert_eq!(s.eval_out("20 c .").unwrap(), "6 ");
    s.eval(": d 2 * 3 * ;").unwrap();                 // x * 6
    assert_eq!(s.eval_out("5 d .").unwrap(), "30 ");
    s.eval(": e 12 and 10 and ;").unwrap();           // x & 8
    assert_eq!(s.eval_out("15 e .").unwrap(), "8 ");
    s.eval(": f 1 or 2 or 4 or ;").unwrap();          // x | 7
    assert_eq!(s.eval_out("0 f .").unwrap(), "7 ");
    // mixed op must NOT chain (correctness): x + 5 - 2 = x + 3
    s.eval(": g 5 + 2 - ;").unwrap();
    assert_eq!(s.eval_out("10 g .").unwrap(), "13 ");
    // the chained four-increment body is smaller than the old 4-add body (7)
    assert!(wa <= 5, "chained 1+×4 body = {wa} words (want <=5)");
    assert!(wb <= 5, "chained + +  body = {wb} words");
    println!("a={wa} b={wb} words");
}
