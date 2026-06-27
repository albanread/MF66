//! Method-dispatch performance. The same trivial method (bump a cell ivar) is
//! driven five ways: an early-bound send (static receiver class → method xt
//! resolved at compile time), a dynamic send (receiver via a variable → vtable
//! lookup), a plain colon call (baseline for "a call + settle"), a C manual-vtable
//! virtual call, and a CPython method call. Reports ns per dispatch.
//!   cargo test --test oopbench -- --ignored --nocapture
#![cfg(target_os = "macos")]
use mf66::Mf66Session;
use std::process::Command;
use std::time::Instant;

const N: u64 = 10_000_000;

const SETUP: &str = "\
class counter cell ivar: n :m bump n 1+ to n ;m :m val n ;m :m reset 0 to n ;m end-class \
counter new c \
variable holder  c holder ! \
0 value cnt \
: inc cnt 1+ to cnt ; \
: be 10000000 0 do c -> bump loop ; \
: bd 10000000 0 do holder @ -> bump loop ; \
: bc 10000000 0 do inc loop ;";

const C_SRC: &str = r#"
#include <stdio.h>
#include <time.h>
#include <stdint.h>
static int64_t ns(){ struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t); return (int64_t)t.tv_sec*1000000000LL+t.tv_nsec; }
struct Obj; typedef void(*M)(struct Obj*);
struct Obj { M* vt; long n; };
__attribute__((noinline)) static void bump(struct Obj* o){ o->n++; }
static M vtable[1] = { bump };
volatile long g;
__attribute__((noinline)) static long bench(long N){
    struct Obj o = { vtable, 0 }; struct Obj* p = &o;
    for(long i=0;i<N;i++) p->vt[0](p);     /* indirect dispatch: load vt, load vt[0], blr */
    g = o.n; return o.n;
}
int main(){ volatile long NN=10000000; int64_t best=INT64_MAX; long r=0;
    for(int k=0;k<5;k++){ int64_t t=ns(); r=bench(NN); int64_t d=ns()-t; if(d<best)best=d; }
    printf("c %lld %lld\n",(long long)best,(long long)r); return 0; }
"#;

const PY_SRC: &str = r#"
import time
class Counter:
    __slots__=('n',)
    def __init__(self): self.n=0
    def bump(self): self.n += 1
def bench(N):
    c=Counter()
    for _ in range(N): c.bump()
    return c.n
best=None; r=None
for _ in range(3):
    t=time.perf_counter_ns(); r=bench(10000000); d=time.perf_counter_ns()-t
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
fn method_dispatch() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(SETUP).unwrap();
    // correctness: one clean run of each bumps the counter to N
    s.eval("c -> reset be").unwrap();
    assert_eq!(s.eval_out("c -> val .").unwrap(), "10000000 ");
    s.eval("c -> reset bd").unwrap();
    assert_eq!(s.eval_out("c -> val .").unwrap(), "10000000 ");

    let warm = |s: &mut Mf66Session, w: &str| { s.eval(w).unwrap(); };
    warm(&mut s, "be"); let t_be = min_ns(|| { s.eval("be").unwrap(); }, 7) as f64 / N as f64;
    warm(&mut s, "bd"); let t_bd = min_ns(|| { s.eval("bd").unwrap(); }, 7) as f64 / N as f64;
    warm(&mut s, "bc"); let t_bc = min_ns(|| { s.eval("bc").unwrap(); }, 7) as f64 / N as f64;

    let dir = std::env::temp_dir();
    let (cp, cb, pp) = (dir.join("oo.c"), dir.join("oo"), dir.join("oo.py"));
    std::fs::write(&cp, C_SRC).unwrap();
    std::fs::write(&pp, PY_SRC).unwrap();
    let c = if Command::new("clang").args(["-O2","-o",cb.to_str().unwrap(),cp.to_str().unwrap()]).status().map(|x|x.success()).unwrap_or(false) {
        ext(cb.to_str().unwrap(), &[]).map(|(n,_)| n as f64 / N as f64)
    } else { None };
    let py = ext("python3", &[pp.to_str().unwrap()]).map(|(n,_)| n as f64 / N as f64);

    println!("\n  method dispatch — ns per call ({}M calls, trivial body: bump a cell ivar)", N/1_000_000);
    println!("  {:<26} {:>9}", "form", "ns/call");
    println!("  {}", "-".repeat(38));
    println!("  {:<26} {:>9.2}", "MF66 send (early-bound)", t_be);
    println!("  {:<26} {:>9.2}", "MF66 send (dynamic vtable)", t_bd);
    println!("  {:<26} {:>9.2}", "MF66 plain colon call", t_bc);
    if let Some(c) = c { println!("  {:<26} {:>9.2}", "C virtual (manual vtable)", c); }
    if let Some(p) = py { println!("  {:<26} {:>9.2}", "CPython method call", p); }
    println!("  {}", "-".repeat(38));
    println!("  (dynamic adds the [class][sel] lookup over early-bound; both wrap a settle + bl)\n");
}
