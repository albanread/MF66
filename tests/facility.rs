#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn facility_and_float() {
    assert_eq!(out("utime drop 0 >= ."), "-1 ");
    assert_eq!(out("utime nip 0 >= ."), "-1 ");
    assert_eq!(out("time&date swap drop swap drop swap drop swap drop swap drop 2020 > ."), "-1 ");
    assert_eq!(out("time&date drop drop drop drop drop 60 < ."), "-1 ");
    assert_eq!(out(r#"s" 3.14" >float ."#), "-1 ");
    assert_eq!(out(r#"s" 3.14" >float drop f0< 0= ."#), "-1 ");
    assert_eq!(out(r#"s" not-a-num" >float ."#), "0 ");
    assert_eq!(out(r#"s" 2.5" >float drop 2e f> ."#), "-1 ");   // 2.5 > 2.0
    assert_eq!(out("unused 0 > ."), "-1 ");
    assert_eq!(out("unused 32 allot unused - ."), "32 ");
}
