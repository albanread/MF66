#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn new_helpers() {
    assert_eq!(out(": u1 0 unless 1 else 2 then ; u1 ."), "1 ");
    assert_eq!(out(": u3 5 unless 1 else 2 then ; u3 ."), "2 ");
    assert_eq!(out("10 20 30 third . . . ."), "10 30 20 10 ");
    assert_eq!(out("1e 2e f<> ."), "-1 ");
    assert_eq!(out("2e 2e f<> ."), "0 ");
    assert_eq!(out("1e f0<> ."), "-1 ");
    assert_eq!(out("0e f0<> ."), "0 ");
    assert_eq!(out("5 0 m+ . ."), "5 0 ");                 // d n -- d
    assert_eq!(out("1 0 2 0 du<= ."), "-1 ");
    assert_eq!(out(r#"s" hello" s" lo" ends-with? ."#), "-1 ");
    assert_eq!(out(r#"s" hello" s" he" ends-with? ."#), "0 ");
    assert_eq!(out("[undefined] nope_xyz ."), "-1 ");
    assert_eq!(out(": d1 ; [undefined] d1 ."), "0 ");
    assert_eq!(out(r#"s" x" environment? ."#), "0 ");
}
