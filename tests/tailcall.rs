//! Terminal tail-call: a colon word's final call becomes a jump (the callee's
//! ret returns to *our* caller). Elides a ret, and turns self-recursion (via
//! RECURSE) into an O(1)-return-stack loop. Primitives are NOT tail-jumped — they
//! may touch RP, where the tail unnest would consume the wrong cell.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn straight_line_tail_call_is_correct() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": twice 2 * ;").unwrap();
    s.eval(": quad twice twice ;").unwrap(); // tail call to `twice`
    assert_eq!(s.eval_out("5 quad .").unwrap(), "20 ");
    // `quad` (straight-line, inlinable) must still inline correctly elsewhere:
    // the tail call is re-appended to its inline IR so the spliced form is whole.
    s.eval(": useq quad 1+ ;").unwrap();
    assert_eq!(s.eval_out("5 useq .").unwrap(), "21 ");
}

#[test]
fn self_recursion_is_an_o1_loop() {
    // 100_000 deep — far past the 64K-cell return stack. Without the tail-call
    // this overflows RP (SIGBUS / garbage); with it, RP stays constant.
    let mut s = Mf66Session::new().unwrap();
    s.eval(": cd dup 0= if drop exit then 1- recurse ;").unwrap();
    assert_eq!(s.eval_out("100000 cd depth .").unwrap(), "0 ");
}

#[test]
fn recursion_with_output_is_ordered() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": cd dup 0= if drop exit then dup . 1- recurse ;").unwrap();
    assert_eq!(s.eval_out("4 cd").unwrap(), "4 3 2 1 ");
}

#[test]
fn a_primitive_tail_is_not_jumped() {
    // `r>` is the last word but pops RP; tail-jumping it would unnest the wrong
    // cell (regression: SIGBUS). It must stay a plain call.
    let mut s = Mf66Session::new().unwrap();
    s.eval(": stash >r r> ;").unwrap(); // ( x -- x )
    assert_eq!(s.eval_out("42 stash .").unwrap(), "42 ");
    s.eval(": rot3 >r swap r> ;").unwrap(); // ( a b c -- b a c )
    assert_eq!(s.eval_out("1 2 3 rot3 . . .").unwrap(), "3 1 2 ");
}
