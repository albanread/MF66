#![cfg(target_os = "macos")]
use mf66::Mf66Session;

fn session() -> Mf66Session {
    let mut s = Mf66Session::new().unwrap();
    // ── integer, stack-based ────────────────────────────────────────────────
    s.eval(": fact 1 swap 1+ 2 ?do i * loop ;").unwrap();           // n!
    s.eval(": fact-r dup 2 < if drop 1 else dup 1- recurse * then ;").unwrap();
    s.eval(": fib 0 1 rot 0 ?do over + swap loop drop ;").unwrap(); // fib(n)
    s.eval(": fib-r dup 2 < if else dup 1- recurse swap 2 - recurse + then ;").unwrap();
    s.eval(": gcd begin ?dup while tuck mod repeat ;").unwrap();
    s.eval(": tri 0 swap 1+ 0 ?do i + loop ;").unwrap();            // triangular sum
    s.eval(": pow 1 swap 0 ?do over * loop nip ;").unwrap();        // base^exp (exp on top)
    // ── locals (hot reads) ──────────────────────────────────────────────────
    s.eval(": poly {: x :} x x * x * 3 x * + 1 + ;").unwrap();      // x read 4× → hot
    s.eval(": dist2 {: x y :} x x * y y * + ;").unwrap();           // x,y read 2× → hot
    s.eval(": prime? {: n :} n 2 < if 0 exit then n 2 = if -1 exit then \
        n 1 and 0= if 0 exit then 3 begin dup dup * n <= while \
        n over mod 0= if drop 0 exit then 2 + repeat drop -1 ;").unwrap();
    s.eval(": count-primes 0 swap 2 ?do i prime? if 1+ then loop ;").unwrap();
    // ── floating point: Mandelbrot escape count (heavy fvariable re-reads) ────
    s.eval("fvariable zx fvariable zy fvariable cx fvariable cy").unwrap();
    s.eval(": dot 0e zx f! 0e zy f! 0 \
        begin zx f@ zx f@ f* zy f@ zy f@ f* f+ 4e f<= over 50 < and while \
        zx f@ zx f@ f* zy f@ zy f@ f* f- cx f@ f+ \
        zx f@ zy f@ f* 2e f* cy f@ f+ \
        zy f! zx f! 1+ repeat ;").unwrap();
    s
}

#[test] fn correctness() {
    let mut s = session();
    let chk = |s: &mut Mf66Session, prog: &str, want: &str| {
        assert_eq!(s.eval_out(prog).unwrap(), want, "prog: {prog}"); s.reset_input();
    };
    chk(&mut s, "5 fact .", "120 ");
    chk(&mut s, "10 fact .", "3628800 ");
    chk(&mut s, "6 fact-r .", "720 ");
    chk(&mut s, "10 fib .", "55 ");
    chk(&mut s, "10 fib-r .", "55 ");
    chk(&mut s, "48 36 gcd .", "12 ");
    chk(&mut s, "100 tri .", "5050 ");
    chk(&mut s, "2 10 pow .", "1024 ");
    chk(&mut s, "5 poly .", "141 "); // 125+15+1
    chk(&mut s, "3 4 dist2 .", "25 ");
    chk(&mut s, "7 prime? .", "-1 ");
    chk(&mut s, "9 prime? .", "0 ");
    chk(&mut s, "100 count-primes .", "25 ");  // 25 primes below 100
    chk(&mut s, "2e cx f! 0e cy f! dot .", "2 ");
    chk(&mut s, "0e cx f! 0e cy f! dot .", "50 ");     // (0,0) stays → hits cap
}

#[test] fn metrics() {
    let s = session();
    eprintln!("\n{}", s.optimizer_report());
}
