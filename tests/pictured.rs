#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn double_cell_pictured() {
    // $FEED as a double (lo=0xFEED, hi=0): 4 hex digits "FEED"
    assert_eq!(out("$FEED hex 0 <# # # # # #> nip decimal ."), "4 ");
    // decimal of a single via s>d-style (push 0 high)
    assert_eq!(out("12345 0 <# #s #> type"), "12345");
    assert_eq!(out("0 0 <# #s #> type"), "0");
    // sign + a negative magnitude
    assert_eq!(out("-7 dup abs 0 <# #s rot sign #> type"), "-7");
}
