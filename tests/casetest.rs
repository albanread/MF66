#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn run(def:&str, call:&str)->String{
    let mut s=Mf66Session::new().unwrap(); s.eval(def).unwrap(); s.eval_out(call).unwrap()
}
#[test] fn case_of_standard() {
    // standard CASE: endcase drops the selector; `dup` default returns it.
    let d = ": col case 1 of 11 endof 2 of 22 endof dup endcase ;";
    assert_eq!(run(d, "1 col ."), "11 ");
    assert_eq!(run(d, "2 col ."), "22 ");
    assert_eq!(run(d, "7 col ."), "7 ");        // unmatched → dup default returns selector
    // matched of consumes the selector, body result remains
    assert_eq!(run(": c2 case 65 of 1 endof 66 of 2 endof 0 endcase ;", "65 c2 ."), "1 ");
    // escape-decoder shape (like WF66's s-q-escape)
    let esc = ": esc case [char] n of 10 endof [char] t of 9 endof dup endcase ;";
    assert_eq!(run(esc, "char n esc ."), "10 ");
    assert_eq!(run(esc, "char t esc ."), "9 ");
    assert_eq!(run(esc, "char z esc ."), "122 ");   // unknown → the char itself
}
