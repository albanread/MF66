#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn batch1() {
    assert_eq!(out("bl ."), "32 ");
    assert_eq!(out("3 spaces s\" x\" type"), "   x");
    assert_eq!(out("max-u 1+ ."), "0 ");
    assert_eq!(out("max-n 1+ min-n = ."), "-1 ");
    assert_eq!(out("max-char ."), "255 ");
    assert_eq!(out("decimal 255 hex. base @ ."), "FF 10 ");
    assert_eq!(out("decimal 8 oct. base @ ."), "10 10 ");
    assert_eq!(out("42 dec. base @ ."), "42 10 ");
    assert_eq!(out("pad 4 + char- pad - ."), "3 ");
    assert_eq!(out("-1 s\" nope\" assert 123 ."), "123 ");   // true → no abort
    assert_eq!(out("5 0 3 ud.r"), "  5");                     // right-justified width 3
    assert_eq!(out("noop 7 ."), "7 ");
}
