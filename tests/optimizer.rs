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
fn dup_fuse_and_strength_reduce_correct() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": sq dup * ;").unwrap(); // dup * → mul x0,x0,x0
    assert_eq!(s.eval_out("7 sq .").unwrap(), "49 ");
    s.eval(": neg2 negate ;").unwrap(); // negate → -1 * (xor/mul fold)
    assert_eq!(s.eval_out("9 neg2 .").unwrap(), "-9 ");
    s.eval(": cells8 cells ;").unwrap(); // cells → *8
    assert_eq!(s.eval_out("3 cells8 .").unwrap(), "24 ");
}
