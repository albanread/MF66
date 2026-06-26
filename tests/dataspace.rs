//! Data-space words: here / allot / , / c, on the VAR_HERE bump pointer.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn here_allot_advances() {
    let mut s = Mf66Session::new().unwrap();
    // here 16 allot here swap - .  => 16
    assert_eq!(s.eval_out("here 16 allot here swap - .").unwrap(), "16 ");
}

#[test]
fn comma_appends_cells() {
    let mut s = Mf66Session::new().unwrap();
    // remember start, append two cells, read them back
    assert_eq!(
        s.eval_out("here 111 , 222 , drop here 16 - dup @ swap 8 + @ . .")
            .unwrap(),
        "222 111 "
    );
}

#[test]
fn c_comma_appends_bytes() {
    let mut s = Mf66Session::new().unwrap();
    // here 65 c, 66 c, then read the two bytes
    assert_eq!(
        s.eval_out("here 65 c, 66 c, drop here 2 - dup c@ swap 1 + c@ . .")
            .unwrap(),
        "66 65 "
    );
}
