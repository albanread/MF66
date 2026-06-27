#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn sess() -> Mf66Session {
    let mut s = Mf66Session::new().unwrap();
    // fvariable version (cx/cy preset via f!)
    s.eval("fvariable zx fvariable zy fvariable cx fvariable cy").unwrap();
    s.eval(": dotv 0e zx f! 0e zy f! 0 \
        begin zx f@ zx f@ f* zy f@ zy f@ f* f+ 4e f<= over 50 < and while \
        zx f@ zx f@ f* zy f@ zy f@ f* f- cx f@ f+ \
        zx f@ zy f@ f* 2e f* cy f@ f+ zy f! zx f! 1+ repeat ;").unwrap();
    // float-locals version: ( F: cx cy -- ; -- iters )
    s.eval(": dotl {: float cx float cy | float zx float zy :} \
        0e to zx 0e to zy 0 \
        begin zx zx f* zy zy f* f+ 4e f<= over 50 < and while \
        zx zx f* zy zy f* f- cx f+ \
        zx zy f* 2e f* cy f+ to zy to zx 1+ repeat ;").unwrap();
    s
}
#[test] fn same_results() {
    let mut s = sess();
    // fvariable: set cx/cy then call; float-locals: push cx cy on FP stack
    assert_eq!(s.eval_out("0e cx f! 0e cy f! dotv .").unwrap(), "50 "); s.reset_input();
    assert_eq!(s.eval_out("0e 0e dotl .").unwrap(), "50 "); s.reset_input();
    assert_eq!(s.eval_out("2e cx f! 0e cy f! dotv .").unwrap(), "2 "); s.reset_input();
    assert_eq!(s.eval_out("2e 0e dotl .").unwrap(), "2 "); s.reset_input();
    // an interior-ish point: -0.5, 0.5
    let a = s.eval_out("-0.5e cx f! 0.5e cy f! dotv .").unwrap(); s.reset_input();
    let b = s.eval_out("-0.5e 0.5e dotl .").unwrap(); s.reset_input();
    assert_eq!(a, b, "fvariable {a} vs float-locals {b}");
}
#[test] fn body_sizes() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("fvariable zx fvariable zy fvariable cx fvariable cy").unwrap();
    s.eval(": dotv 0e zx f! 0e zy f! 0 begin zx f@ zx f@ f* zy f@ zy f@ f* f+ 4e f<= over 50 < and while zx f@ zx f@ f* zy f@ zy f@ f* f- cx f@ f+ zx f@ zy f@ f* 2e f* cy f@ f+ zy f! zx f! 1+ repeat ;").unwrap();
    let v = s.last_body_words();
    s.eval(": dotl {: float cx float cy | float zx float zy :} 0e to zx 0e to zy 0 begin zx zx f* zy zy f* f+ 4e f<= over 50 < and while zx zx f* zy zy f* f- cx f+ zx zy f* 2e f* cy f+ to zy to zx 1+ repeat ;").unwrap();
    let l = s.last_body_words();
    eprintln!("\nMandelbrot body: fvariable {v} words  vs  float-locals {l} words");
    eprintln!("\n{}", s.optimizer_report());
}
