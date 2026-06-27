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

#[test] fn pinned_int_locals() {
    // call-free do-loop, int accumulator pinned (x15) across iterations
    assert_eq!(out(": s {: | acc :} 0 to acc 100 0 do acc 1+ to acc loop acc . ; s"), "100 ");
    // begin..until int local
    assert_eq!(out(": cd {: | n :} 0 to n begin n 1+ to n n 5 = until n . ; cd"), "5 ");
    // two int locals pinned (x15,x14)
    assert_eq!(out(": t2 {: | a b :} 0 to a 0 to b 10 0 do a 2 + to a b 3 + to b loop a b + . ; t2"), "50 ");
    // mixed: a float local (d9) AND an int local (x15) pinned in the same loop
    assert_eq!(out(": mx {: | float z n :} 0e to z 0 to n 4 0 do z 1e f+ to z n 1+ to n loop n . z f. ; mx"), "4 4 ");
    // do-loop using i (a Call) → must NOT pin, but still correct
    assert_eq!(out(": si {: | acc :} 0 to acc 5 0 do acc i + to acc loop acc . ; si"), "10 ");
}
