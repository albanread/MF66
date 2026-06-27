//! The CODE escape hatch, measured: a hot integer kernel (xorshift64) where the
//! optimizer is hamstrung — `lshift`/`rshift` are settle-barrier Calls — written
//! three ways: a normal colon definition (optimizer), a hand-written CODE word
//! (assembly), and clang -O2 C, plus CPython. Shows the CODE word reaching C.
//!   cargo test --test codebench -- --ignored --nocapture
#![cfg(target_os = "macos")]
use mf66::Mf66Session;
use std::process::Command;
use std::time::Instant;

const N: u64 = 5_000_000;
const MASK: u64 = 0x0FFF_FFFF; // 28 bits → positive, identical across impls

// Forth colon version: lshift/rshift are Calls → 3 settles per iteration.
const XS_FS: &str = ": xs-fs {: | x :} 1 to x 5000000 0 do \
    x x 13 lshift xor to x x x 7 rshift xor to x x x 17 lshift xor to x \
    loop x 268435455 and ;";

// Hand-written: x in x9, three eor-with-shifted-register, tight counter loop.
const XS_ASM: &str = "CODE xs-asm\n\
    mov  x9, #1\n\
    mov  x10, TOS\n\
.lp:\n\
    eor  x9, x9, x9, lsl #13\n\
    eor  x9, x9, x9, lsr #7\n\
    eor  x9, x9, x9, lsl #17\n\
    subs x10, x10, #1\n\
    b.ne .lp\n\
    and  TOS, x9, #0x0FFFFFFF\n\
    next()\n\
END-CODE";

const C_SRC: &str = r#"
#include <stdio.h>
#include <time.h>
#include <stdint.h>
static int64_t ns(){ struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t); return (int64_t)t.tv_sec*1000000000LL+t.tv_nsec; }
volatile long g;
__attribute__((noinline)) static long xs(long n){ uint64_t x=1; for(long i=0;i<n;i++){ x^=x<<13; x^=x>>7; x^=x<<17; } g=(long)x; return (long)(x & 0x0FFFFFFFUL); }
int main(){ volatile long M=5000000; int64_t best=INT64_MAX; long r=0; for(int k=0;k<5;k++){ int64_t t=ns(); r=xs(M); int64_t d=ns()-t; if(d<best)best=d; } printf("c %lld %lld\n",(long long)best,(long long)r); return 0; }
"#;

const PY_SRC: &str = r#"
import time
def xs(n):
    x=1; M=0xFFFFFFFFFFFFFFFF
    for _ in range(n):
        x ^= (x<<13)&M; x ^= x>>7; x ^= (x<<17)&M
    return x & 0x0FFFFFFF
best=None; r=None
for _ in range(3):
    t=time.perf_counter_ns(); r=xs(5000000); d=time.perf_counter_ns()-t
    best=d if best is None else min(best,d)
print("py", best, r)
"#;

fn min_ns(mut f: impl FnMut(), runs: usize) -> u128 {
    let mut best = u128::MAX;
    for _ in 0..runs { let t = Instant::now(); f(); best = best.min(t.elapsed().as_nanos()); }
    best
}
fn ext(prog: &str, args: &[&str]) -> Option<(u128, i64)> {
    let o = Command::new(prog).args(args).output().ok()?;
    if !o.status.success() { eprintln!("[skip] {}", String::from_utf8_lossy(&o.stderr)); return None; }
    let s = String::from_utf8_lossy(&o.stdout);
    let mut it = s.split_whitespace();
    let _ = it.next()?; Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

#[test]
#[ignore]
fn code_word_vs_optimizer_vs_c() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(XS_FS).unwrap();
    s.eval(XS_ASM).unwrap();
    // correctness: the hand-written CODE word must equal the colon version
    s.eval("xs-fs").unwrap();
    let r_fs = s.stack()[0];
    s.eval("5000000 xs-asm").unwrap();
    let r_asm = s.stack()[0];
    assert_eq!(r_fs, r_asm, "xs-asm (CODE) disagrees with xs-fs (colon)");

    let t_fs = min_ns(|| { s.eval("xs-fs").unwrap(); }, 5) as f64 / N as f64;
    let t_asm = min_ns(|| { s.eval("5000000 xs-asm").unwrap(); }, 7) as f64 / N as f64;

    // C + Python
    let dir = std::env::temp_dir();
    let (cp, cb, pp) = (dir.join("xs.c"), dir.join("xs"), dir.join("xs.py"));
    std::fs::write(&cp, C_SRC).unwrap();
    std::fs::write(&pp, PY_SRC).unwrap();
    let c = if Command::new("clang").args(["-O2","-o",cb.to_str().unwrap(),cp.to_str().unwrap()]).status().map(|s|s.success()).unwrap_or(false) {
        ext(cb.to_str().unwrap(), &[])
    } else { None };
    let py = ext("python3", &[pp.to_str().unwrap()]);

    let cn = c.map(|(n,_)| n as f64 / N as f64);
    let pn = py.map(|(n,_)| n as f64 / N as f64);
    // cross-check result across everything that ran
    if let Some((_,cr)) = c { assert_eq!(r_asm, cr, "CODE vs C result"); }
    if let Some((_,pr)) = py { assert_eq!(r_asm, pr, "CODE vs Python result"); }

    println!("\n  xorshift64 — ns/iteration ({}M iters, lower is better)", N/1_000_000);
    println!("  {:<22} {:>10}", "implementation", "ns/iter");
    println!("  {}", "-".repeat(34));
    println!("  {:<22} {:>10.2}", "MF66 colon (optimizer)", t_fs);
    println!("  {:<22} {:>10.2}", "MF66 CODE (asm)", t_asm);
    if let Some(c) = cn { println!("  {:<22} {:>10.2}", "C (clang -O2)", c); }
    if let Some(p) = pn { println!("  {:<22} {:>10.2}", "CPython", p); }
    println!("  {}", "-".repeat(34));
    if let Some(c) = cn {
        println!("  CODE is {:.2}x the colon version, {:.2}x C  (mask {MASK:#x}, result {r_asm})",
            t_fs / t_asm, t_asm / c);
    }
    println!();
}
