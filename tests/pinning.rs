#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn pinned_fp_accumulator() {
    // call-free FP loop: s += 1.0, 100×  →  s = 100.0  (pinned across iterations)
    assert_eq!(out(": acc {: | float s :} 0e to s 100 0 do s 1e f+ to s loop s f. ; acc"), "100 ");
    // begin..until call-free FP loop
    assert_eq!(out(": acc2 {: | float s :} 0e to s 0 begin s 1e f+ to s 1+ dup 5 = until drop s f. ; acc2"), "5 ");
    // two pinned float locals
    assert_eq!(out(": two {: | float a float b :} 0e to a 0e to b 10 0 do a 1e f+ to a b 2e f+ to b loop a b f+ f. ; two"), "30 ");
    // a loop with a CALL (fsqrt is a call? no — vstack) — use f< (a call) → NOT pinned but still correct
    assert_eq!(out(": esc {: | float z :} 0e to z 0 begin z 9e f< while z 1e f+ to z 1+ repeat drop z f. ; esc"), "9 ");
}
