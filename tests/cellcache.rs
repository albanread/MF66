#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn hot_const_cell_fetch_cache_is_store_safe() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("variable cv 5 cv !").unwrap();
    s.eval(": twice cv @ cv @ + ;").unwrap();
    assert_eq!(s.eval_out("twice .").unwrap(), "10 ");

    // The cache is conservative: a store between two const-address @ reads must
    // invalidate, or this would incorrectly return the old value twice.
    s.eval("5 cv ! : storegap cv @ 7 cv ! cv @ + ;").unwrap();
    assert_eq!(s.eval_out("storegap .").unwrap(), "12 ");
}
