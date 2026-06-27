#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn run(defs:&[&str], call:&str)->String{
    let mut s=Mf66Session::new().unwrap();
    for d in defs { s.eval(d).unwrap(); }
    s.eval_out(call).unwrap()
}
#[test] fn qdo() {
    assert_eq!(run(&[": cf4 0 0 ?do 1 loop ;"], "0 cf4 ."), "0 ");        // skip
    assert_eq!(run(&[": cf4b 1 0 ?do 1 loop ;"], "0 cf4b . ."), "1 0 ");   // run once
    assert_eq!(run(&[": sum 0 swap 0 ?do i + loop ;"], "5 sum ."), "10 ");
}
#[test] fn leave_exits() {
    assert_eq!(run(&[": lv 10 0 do i dup 3 = if drop leave then loop ;"], "lv . . ."), "2 1 0 ");
    assert_eq!(run(&[": lv2 0 10 0 do 1+ dup 4 = if leave then loop ;"], "lv2 ."), "4 ");
}
#[test] fn plus_loop() {
    assert_eq!(run(&[": tens 0 100 0 do i + 10 +loop ;"], "tens ."), "450 ");
}

#[test] fn exit_and_unloop_exit() {
    // plain early exit
    assert_eq!(run(&[": e1 dup 0= if 99 exit then 7 ;"], "0 e1 ."), "99 ");
    assert_eq!(run(&[": e1 dup 0= if 99 exit then drop 7 ;"], "5 e1 ."), "7 ");
    // unloop + exit out of a do-loop
    assert_eq!(run(&[": ul 5 0 do i 2 = if unloop 99 exit then loop -1 ;"], "ul ."), "99 ");
}
