//! The optimizing compiler: token-IR reduce (const-fold / immediate-fold /
//! dup-fuse / DCE) + AArch64 lowering. Correctness is covered by the eval +
//! colon + control corpora (optimized output == WF66); these tests prove the
//! optimization actually happens (the compiled body shrinks / folds) and stays
//! correct.

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn const_fold_collapses_to_one_literal() {
    let mut s = Mf66Session::new().unwrap();
    // 2 3 + 4 * → 20, folded to a single literal load (nest + load + unnest/ret).
    s.eval(": k 2 3 + 4 * ;").unwrap();
    let folded = s.last_body_words();
    // a single literal: nest(1) + str+movz/movk(≤5) + unnest(1)+ret(1) ≤ 9
    assert!(folded <= 12, "const-folded body should be tiny (one literal), got {folded} words");
    assert_eq!(s.eval_out("k .").unwrap(), "20 ");
}

#[test]
fn inlining_beats_eager_calls() {
    // A body of inlinable ops must be far smaller than one veneer call (5 words)
    // per word would produce.
    let mut s = Mf66Session::new().unwrap();
    s.eval(": w over + swap - dup * ;").unwrap();
    let opt = s.last_body_words();
    // 5 ops eagerly = 5 veneer calls = 25 words + nest/unnest. Inlined is far less.
    assert!(opt < 20, "inlined body should beat eager calls, got {opt} words");
}

#[test]
fn optimized_arithmetic_is_correct() {
    let mut s = Mf66Session::new().unwrap();
    // dup-fuse (dup +), immediate-fold (1+, 2*), inline (-, *)
    s.eval(": f dup + 1+ 2 * ;").unwrap(); // (2a+1)*2 = 4a+2
    assert_eq!(s.eval_out("5 f .").unwrap(), "22 "); // (2*5+1)*2 = 22
}

#[test]
fn algebraic_identities_collapse() {
    let mut s = Mf66Session::new().unwrap();
    assert_eq!(s.eval_out(": a 0 + ; 5 a .").unwrap(), "5 "); // x + 0 = x
    assert_eq!(s.eval_out(": b 1 * ; 5 b .").unwrap(), "5 "); // x * 1 = x
    assert_eq!(s.eval_out(": c 0 * ; 9 c .").unwrap(), "0 "); // x * 0 = 0
    assert_eq!(s.eval_out(": d -1 and ; 7 d .").unwrap(), "7 "); // x and -1 = x
    assert_eq!(s.eval_out(": e dup xor ; 6 e .").unwrap(), "0 "); // x xor x = 0
    assert_eq!(s.eval_out(": g dup and ; 6 g .").unwrap(), "6 "); // x and x = x
    // `0 +` collapses to nothing — the body is empty (just the colon frame).
    s.eval(": id 0 + 0 + ;").unwrap();
    assert!(s.last_body_words() <= 4, "0 + 0 + should vanish");
}

#[test]
fn multiply_by_power_of_two_is_a_shift() {
    let mut s = Mf66Session::new().unwrap();
    assert_eq!(s.eval_out(": x8 8 * ; 5 x8 .").unwrap(), "40 ");
    assert_eq!(s.eval_out(": k 1024 * ; 3 k .").unwrap(), "3072 ");
    assert_eq!(s.eval_out("-5 4 * .").unwrap(), "-20 "); // signed shift
    // x * 16 should lower to a single lsl, not movz-16 + mul
    s.eval(": m 16 * ;").unwrap();
    assert!(s.last_body_words() <= 10, "x*16 should be one lsl, got {}", s.last_body_words());
}

#[test]
fn dup_fuse_and_strength_reduce_correct() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": sq dup * ;").unwrap(); // dup * → mul x0,x0,x0
    assert_eq!(s.eval_out("7 sq .").unwrap(), "49 ");
    s.eval(": neg2 negate ;").unwrap(); // negate → -1 * (xor/mul fold)
    assert_eq!(s.eval_out("9 neg2 .").unwrap(), "-9 ");
    s.eval(": cells8 cells ;").unwrap(); // cells → *8
    assert_eq!(s.eval_out("3 cells8 .").unwrap(), "24 ");
}

#[test]
fn shifts_inline_and_fuse() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": sh 13 lshift ;").unwrap();
    assert_eq!(s.eval_out("1 sh .").unwrap(), "8192 ");
    assert!(s.last_body_words() <= 8, "constant lshift should inline, got {}", s.last_body_words());

    s.eval(": ar 2/ ;").unwrap();
    assert_eq!(s.eval_out("-7 ar . 8 ar .").unwrap(), "-4 4 ");

    // xorshift idiom: no shift calls/settles, and `dup k shift xor` lowers to one shifted EOR.
    s.eval(": xs1 dup 13 lshift xor ;").unwrap();
    assert_eq!(s.eval_out("1 xs1 .").unwrap(), "8193 ");
    assert!(s.last_body_words() <= 5, "dup/shift/xor should fuse tightly, got {}", s.last_body_words());
}
