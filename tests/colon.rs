//! The colon compiler: `:`/`;` define new words as STC bodies in the executable
//! code arena (nest, a call/literal per token, unnest+ret), findable + runnable
//! like any primitive. This is the compiler half — eager (non-optimized) bodies.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn colon_basic() {
    let mut s = Mf66Session::new().unwrap();
    assert_eq!(s.eval_out(": square dup * ; 5 square .").unwrap(), "25 ");
}

#[test]
fn colon_with_literals() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": inc3 1 + 1 + 1 + ;").unwrap();
    assert_eq!(s.eval_out("10 inc3 .").unwrap(), "13 ");
}

#[test]
fn colon_calls_colon() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": square dup * ;").unwrap();
    s.eval(": quad square square ;").unwrap(); // (x^2)^2 = x^4
    assert_eq!(s.eval_out("2 quad .").unwrap(), "16 ");
}

#[test]
fn colon_uses_return_stack() {
    // >r / r> inside a definition must compose with the colon nest/unnest on RP.
    let mut s = Mf66Session::new().unwrap();
    s.eval(": stash >r r> ;").unwrap(); // ( x -- x ) round-trip through R
    assert_eq!(s.eval_out("42 stash .").unwrap(), "42 ");
}

#[test]
fn colon_redefine_uses_newest() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": foo 1 ;").unwrap();
    assert_eq!(s.eval_out("foo .").unwrap(), "1 ");
    s.eval(": foo 2 ;").unwrap();
    assert_eq!(s.eval_out("foo .").unwrap(), "2 ");
}

#[test]
fn colon_composes_deeply() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": double dup + ;").unwrap();
    s.eval(": quadruple double double ;").unwrap();
    s.eval(": octuple quadruple double ;").unwrap();
    assert_eq!(s.eval_out("3 octuple .").unwrap(), "24 "); // 3*8
}
