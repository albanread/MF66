#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ let mut s2=Mf66Session::new().unwrap(); s2.eval_out(s).unwrap() }
#[test] fn constant_via_create_does() {
    let mut s=Mf66Session::new().unwrap();
    s.eval(": myconst create , does> @ ;").unwrap();
    s.eval("42 myconst the-answer").unwrap();
    assert_eq!(s.eval_out("the-answer .").unwrap(), "42 ");
    s.eval("7 myconst seven").unwrap();
    assert_eq!(s.eval_out("seven seven + .").unwrap(), "14 ");
}
#[test] fn variable_via_create() {
    let mut s=Mf66Session::new().unwrap();
    s.eval(": myvar create 0 , ;").unwrap();   // no does> — pushes the addr
    s.eval("myvar counter").unwrap();
    assert_eq!(s.eval_out("99 counter ! counter @ .").unwrap(), "99 ");
}
#[test] fn array_via_create_does() {
    let mut s=Mf66Session::new().unwrap();
    s.eval(": array create cells allot does> swap cells + ;").unwrap();
    s.eval("5 array v").unwrap();
    s.eval("11 0 v ! 22 1 v ! 33 2 v !").unwrap();
    assert_eq!(s.eval_out("0 v @ 1 v @ 2 v @ . . .").unwrap(), "33 22 11 ");
}
#[test] fn const_used_in_colon() {
    let mut s=Mf66Session::new().unwrap();
    s.eval(": myconst create , does> @ ;").unwrap();
    s.eval("10 myconst ten").unwrap();
    s.eval(": add-ten ten + ;").unwrap();   // the created word used inside a colon
    assert_eq!(s.eval_out("5 add-ten .").unwrap(), "15 ");
}
