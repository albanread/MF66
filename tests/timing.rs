//! FP performance harness: time MF66's register-resident numeric kernels against
//! the same algorithms in clang -O2 C and in CPython. Reports ns/iteration and
//! ratios. Ignored by default (it shells out to clang/python3 and runs for a few
//! seconds); run with:
//!   cargo test --test timing -- --ignored --nocapture
//!
//! The three implementations are kept in lockstep (same constants, same N) so the
//! stable kernels (rot2, fsin) cross-check to the same scaled-int result; logistic
//! is chaotic, so its result diverges across implementations (expected) and is not
//! checked — only timed.
#![cfg(target_os = "macos")]
use mf66::Mf66Session;
use std::process::Command;
use std::time::Instant;

const LOGISTIC_N: u64 = 5_000_000; // x = 3.9·x·(1-x)         (chaotic, pure FP)
const ROT2_N: u64 = 5_000_000; //     unit rotation by (.8,.6) (stable, 2 pins)
const FSIN_N: u64 = 1_000_000; //     s = sin(s)+0.5          (libm call in loop)

const C_SRC: &str = r#"
#include <stdio.h>
#include <math.h>
#include <time.h>
#include <stdint.h>
static int64_t ns(){ struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t); return (int64_t)t.tv_sec*1000000000LL+t.tv_nsec; }
/* noinline + a volatile-sink write give each kernel an observable side effect, so
   clang cannot hoist the (otherwise pure) call out of the timed region. */
volatile double g_sink;
__attribute__((noinline)) static double logistic(double x,long n){ for(long i=0;i<n;i++) x=3.9*x*(1.0-x); g_sink=x; return x; }
__attribute__((noinline)) static double rot2(double x,double y,long n){ for(long i=0;i<n;i++){ double a=0.8*x-0.6*y,b=0.6*x+0.8*y; x=a; y=b; } g_sink=x; return x; }
__attribute__((noinline)) static double fsink(double s,long n){ for(long i=0;i<n;i++) s=sin(s)+0.5; g_sink=s; return s; }
int main(){
  volatile long N1=5000000,N2=5000000,N3=1000000;     /* volatile: no cross-rep CSE */
  volatile double a=0.5,bx=1.0,by=0.0,c=0.0;
  { int64_t best=INT64_MAX; double r=0; for(int k=0;k<5;k++){ int64_t t=ns(); r=logistic(a,N1); int64_t d=ns()-t; if(d<best)best=d; } printf("logistic %lld %lld\n",(long long)best,(long long)(r*1e6)); }
  { int64_t best=INT64_MAX; double r=0; for(int k=0;k<5;k++){ int64_t t=ns(); r=rot2(bx,by,N2); int64_t d=ns()-t; if(d<best)best=d; } printf("rot2 %lld %lld\n",(long long)best,(long long)(r*1e6)); }
  { int64_t best=INT64_MAX; double r=0; for(int k=0;k<5;k++){ int64_t t=ns(); r=fsink(c,N3); int64_t d=ns()-t; if(d<best)best=d; } printf("fsin %lld %lld\n",(long long)best,(long long)(r*1e6)); }
  return 0;
}
"#;

const PY_SRC: &str = r#"
import math, time
def logistic(x, n):
    for _ in range(n): x = 3.9*x*(1.0-x)
    return x
def rot2(x, y, n):
    for _ in range(n):
        a = 0.8*x - 0.6*y; b = 0.6*x + 0.8*y; x = a; y = b
    return x
def fsink(s, n):
    sin = math.sin
    for _ in range(n): s = sin(s) + 0.5
    return s
def bench(name, f, args, n, reps=3):
    best = None
    for _ in range(reps):
        t = time.perf_counter_ns(); r = f(*args, n); d = time.perf_counter_ns() - t
        best = d if best is None else min(best, d)
    print(name, best, int(r*1e6))
bench("logistic", logistic, (0.5,), 5000000)
bench("rot2", rot2, (1.0, 0.0), 5000000)
bench("fsin", fsink, (0.0,), 1000000)
"#;

/// Minimum wall-clock (ns) of `runs` invocations of `f`.
fn min_ns(mut f: impl FnMut(), runs: usize) -> u128 {
    let mut best = u128::MAX;
    for _ in 0..runs {
        let t = Instant::now();
        f();
        best = best.min(t.elapsed().as_nanos());
    }
    best
}

/// Run an external bench program; parse "name total_ns result" lines.
fn run_external(label: &str, prog: &str, args: &[&str]) -> Option<Vec<(String, u128, i64)>> {
    let out = Command::new(prog).args(args).output().ok()?;
    if !out.status.success() {
        eprintln!("[skip {label}] {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(
        text.lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                let name = it.next()?.to_string();
                let ns: u128 = it.next()?.parse().ok()?;
                let res: i64 = it.next()?.parse().ok()?;
                Some((name, ns, res))
            })
            .collect(),
    )
}

#[test]
#[ignore]
fn fp_timing_vs_c_and_python() {
    let dir = std::env::temp_dir();
    let cpath = dir.join("mf66_bench.c");
    let cbin = dir.join("mf66_bench");
    let pypath = dir.join("mf66_bench.py");
    std::fs::write(&cpath, C_SRC).unwrap();
    std::fs::write(&pypath, PY_SRC).unwrap();

    // ── MF66: define the kernels, time each (min of 7), read the result int ──
    let mut s = Mf66Session::new().unwrap();
    let defs = [
        ("logistic", LOGISTIC_N, format!(
            ": logistic {{: | float x :}} 0.5e to x {} 0 do x 1e x f- f* 3.9e f* to x loop x 1000000e f* f>d drop ;",
            LOGISTIC_N)),
        ("rot2", ROT2_N, format!(
            ": rot2 {{: | float x float y :}} 1e to x 0e to y {} 0 do x 0.8e f* y 0.6e f* f- x 0.6e f* y 0.8e f* f+ to y to x loop x 1000000e f* f>d drop ;",
            ROT2_N)),
        ("fsin", FSIN_N, format!(
            ": fsin-k {{: | float s :}} 0e to s {} 0 do s fsin 0.5e f+ to s loop s 1000000e f* f>d drop ;",
            FSIN_N)),
    ];
    let calls = ["logistic", "rot2", "fsin-k"];
    let mut mf66: Vec<(String, u128, i64)> = Vec::new();
    for ((name, n, def), call) in defs.iter().zip(calls.iter()) {
        s.eval(def).unwrap();
        s.eval(call).unwrap(); // warm up + leave a result
        let res = s.stack()[0];
        let ns = min_ns(|| { s.eval(call).unwrap(); }, 7);
        mf66.push((name.to_string(), ns, res));
        let _ = n;
    }

    // ── C (clang -O2) and Python ──
    let cc = Command::new("clang")
        .args(["-O2", "-o", cbin.to_str().unwrap(), cpath.to_str().unwrap(), "-lm"])
        .status();
    let c = match cc {
        Ok(st) if st.success() => run_external("C", cbin.to_str().unwrap(), &[]),
        _ => { eprintln!("[skip C] clang unavailable"); None }
    };
    let py = run_external("Python", "python3", &[pypath.to_str().unwrap()]);

    // ── report ──
    let n_of = |name: &str| match name {
        "logistic" => LOGISTIC_N,
        "rot2" => ROT2_N,
        _ => FSIN_N,
    };
    let find = |v: &Option<Vec<(String, u128, i64)>>, name: &str| -> Option<(u128, i64)> {
        v.as_ref()?.iter().find(|(n, _, _)| n == name).map(|(_, ns, r)| (*ns, *r))
    };

    println!("\n  FP kernels — ns/iteration (lower is better), min of repeated runs");
    println!("  {:<10} {:>5} {:>12} {:>12} {:>12}   {:>8} {:>10}",
        "kernel", "N(M)", "MF66 ns/it", "C ns/it", "Python ns/it", "MF66/C", "Py/MF66");
    println!("  {}", "-".repeat(78));
    for (name, mns, mres) in &mf66 {
        let n = n_of(name) as f64;
        let mf = *mns as f64 / n;
        let (cns, cres) = find(&c, name).map(|(ns, r)| (ns as f64 / n, Some(r))).unwrap_or((f64::NAN, None));
        let (pns, pres) = find(&py, name).map(|(ns, r)| (ns as f64 / n, Some(r))).unwrap_or((f64::NAN, None));
        println!("  {:<10} {:>5.0} {:>12.2} {:>12.2} {:>12.2}   {:>7.2}x {:>9.1}x",
            name, n / 1e6, mf, cns, pns, mf / cns, pns / mf);
        // cross-check the stable kernels compute the same thing everywhere
        if name == "rot2" || name == "fsin" {
            if let Some(cr) = cres { assert!((mres - cr).abs() <= 2, "{name}: MF66 {mres} vs C {cr}"); }
            if let Some(pr) = pres { assert!((mres - pr).abs() <= 2, "{name}: MF66 {mres} vs Py {pr}"); }
        }
        let _ = mres;
    }
    println!("  {}", "-".repeat(78));
    println!("  (logistic is chaotic — results diverge across impls by design; rot2/fsin results cross-checked)\n");
}
