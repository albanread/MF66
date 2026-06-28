//! Division guards — ÷0 and signed INT_MIN/-1 raise a *recoverable* THROW
//! (-10 / -11), instead of AArch64 sdiv's silent 0 / wrap. Matches the intent of
//! WF66's x86 `idiv` #DE trap, but reports + recovers rather than crashing.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

fn err(src: &str) -> String {
    let mut s = Mf66Session::new().unwrap();
    format!("{:#}", s.eval_out(src).unwrap_err())
}

fn out(src: &str) -> String {
    Mf66Session::new().unwrap().eval_out(src).unwrap()
}

#[test]
fn divide_by_zero_throws_on_every_division_word() {
    for src in ["5 0 /", "5 0 mod", "5 0 /mod", "5 0 um/mod", "10 1 0 */", "10 1 0 */mod"] {
        let m = err(src);
        assert!(m.contains("division by zero"), "`{src}` -> {m}");
    }
}

#[test]
fn int_min_over_minus_one_is_out_of_range() {
    // 1<<63 = INT_MIN; INT_MIN / -1 overflows a signed cell.
    assert!(err("1 63 lshift -1 /").contains("out of range"));
    assert!(err("1 63 lshift -1 mod").contains("out of range"));
}

#[test]
fn normal_division_is_unaffected() {
    let mut s = Mf66Session::new().unwrap();
    assert_eq!(s.eval_out("7 2 / .").unwrap(), "3 ");
    assert_eq!(s.eval_out("-7 2 / .").unwrap(), "-3 "); // symmetric (toward zero)
    assert_eq!(s.eval_out("17 5 mod .").unwrap(), "2 ");
    assert_eq!(s.eval_out("100 7 /mod . .").unwrap(), "14 2 "); // quot rem
}

#[test]
fn the_throw_recovers_even_from_a_compiled_word() {
    // a ÷0 deep inside a colon word unwinds to the root handler; the session lives.
    let mut s = Mf66Session::new().unwrap();
    s.eval(": boom 5 0 / ;").unwrap();
    assert!(s.eval_out("boom").is_err());
    s.reset_input();
    assert_eq!(s.eval_out("6 3 / .").unwrap(), "2 "); // still alive
}

#[test]
fn catch_absorbs_a_division_throw_with_code_minus_10() {
    assert_eq!(out(": bad 5 0 / ; ' bad catch ."), "-10 ");
}
