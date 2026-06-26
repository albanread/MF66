//! The interpreter core: `eval` runs Forth source by parse → find → execute,
//! else number → push. Results are left on the data stack (`stack()` is top-first).
//! Output (`.`/`type`) and the kernel-side parse/interpret loop come next.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn eval_arithmetic() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("2 3 +").unwrap();
    assert_eq!(s.stack(), vec![5]);
}

#[test]
fn eval_words_and_numbers_mixed() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("10 dup * 1+").unwrap(); // 10 10 * = 100, 1+ = 101
    assert_eq!(s.stack(), vec![101]);
}

#[test]
fn eval_stack_shuffles_and_compare() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("7 4 swap -").unwrap(); // 4 7 -  = -3
    assert_eq!(s.stack(), vec![-3]);
    s.reset();
    s.eval("5 5 =").unwrap(); // -1 (true)
    assert_eq!(s.stack(), vec![-1]);
    s.reset();
    s.eval("3 9 <").unwrap(); // -1 (3 < 9)
    assert_eq!(s.stack(), vec![-1]);
}

#[test]
fn eval_negative_literals() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("-5 negate").unwrap();
    assert_eq!(s.stack(), vec![5]);
}

#[test]
fn eval_undefined_word_errors() {
    let mut s = Mf66Session::new().unwrap();
    assert!(s.eval("2 frobnicate").is_err());
}

#[test]
fn eval_memory_via_pad() {
    // store/fetch through the dictionary words on a scratch address clear of PAD
    // (push_name writes each token into PAD, so the data address must not overlap)
    let mut s = Mf66Session::new().unwrap();
    let pad = s.pad_base() as i64 + 0x800;
    s.push(1234);
    s.push(pad);
    s.eval("!").unwrap(); // ( 1234 addr -- )  store
    assert_eq!(s.depth(), 0);
    s.push(pad);
    s.eval("@").unwrap(); // ( addr -- 1234 )
    assert_eq!(s.stack(), vec![1234]);
}
