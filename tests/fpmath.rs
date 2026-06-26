#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn compares() {
    assert_eq!(out("1e 2e f< ."), "-1 ");      // 1<2
    assert_eq!(out("2e 1e f< ."), "0 ");
    assert_eq!(out("0e f0= ."), "-1 ");
    assert_eq!(out("1e f0= ."), "0 ");
    assert_eq!(out("-3e f0< ."), "-1 ");
    assert_eq!(out("3e f0< ."), "0 ");
}
#[test] fn conversions() {
    // d is a double-cell ( lo hi ); print the low cell after the round-trip.
    assert_eq!(out("5 0 d>f f>d drop ."), "5 ");
    assert_eq!(out("42 0 d>f 1e f+ f>d drop ."), "43 "); // 42.0 + 1.0 = 43
}
#[test] fn hardware_math() {
    assert_eq!(out("4e fsqrt f."), "2 ");
    assert_eq!(out("-3e fabs f."), "3 ");
    assert_eq!(out("3.7 floor f."), "3 ");
    assert_eq!(out("3.2 fround f."), "3 ");
    assert_eq!(out("3.9 ftrunc f."), "3 ");
}
#[test] fn transcendental() {
    assert_eq!(out("0e fsin f."), "0 ");
    assert_eq!(out("0e fcos f."), "1 ");
    assert_eq!(out("1e fexp fln f."), "1 ");        // ln(exp(1)) = 1
    assert_eq!(out("100e flog f."), "2 ");           // log10(100) = 2
    assert_eq!(out("2e 10e f** f."), "1024 ");       // 2^10
}
#[test] fn address_arith() {
    assert_eq!(out("8 floats ."), "64 ");
    assert_eq!(out("100 float+ ."), "108 ");
    assert_eq!(out("13 faligned ."), "16 ");
}
