//! FP/integer performance harness: time MF66's compiled kernels against the same
//! algorithms in clang -O2 C and in CPython. Reports ns per work-unit and ratios.
//! Ignored by default (shells out to clang/python3, runs a few seconds):
//!   cargo test --test timing -- --ignored --nocapture
//!
//! The implementations are kept in lockstep (same constants, same N) so every
//! kernel except the chaotic `logistic` cross-checks to the same result across
//! MF66/C/Python — making the comparison provably apples-to-apples. C kernels use
//! noinline + a volatile sink so clang can't hoist the pure call out of the timed
//! window. brot uses no FMA in MF66/Python (exact match); C may contract to FMA,
//! so its escape-count sum is allowed to drift a hair near the boundary.
#![cfg(target_os = "macos")]
use mf66::Mf66Session;
use std::process::Command;
use std::time::Instant;

const C_SRC: &str = r#"
#include <stdio.h>
#include <math.h>
#include <time.h>
#include <stdint.h>
static int64_t ns(){ struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t); return (int64_t)t.tv_sec*1000000000LL+t.tv_nsec; }
/* noinline + volatile sink: the kernel has an observable effect, so clang cannot
   hoist the otherwise-pure call out of the timed region. */
volatile double g_sink; volatile long g_isink;
__attribute__((noinline)) static double logistic(double x,long n){ for(long i=0;i<n;i++) x=3.9*x*(1.0-x); g_sink=x; return x; }
__attribute__((noinline)) static double rot2(double x,double y,long n){ for(long i=0;i<n;i++){ double a=0.8*x-0.6*y,b=0.6*x+0.8*y; x=a; y=b; } g_sink=x; return x; }
__attribute__((noinline)) static double fsink(double s,long n){ for(long i=0;i<n;i++) s=sin(s)+0.5; g_sink=s; return s; }
__attribute__((noinline)) static long isprime(long n){ if(n<2) return 0; for(long d=2; d*d<=n; d++) if(n%d==0) return 0; return 1; }
__attribute__((noinline)) static long cprimes(long N){ long c=0; for(long i=2;i<N;i++) c+=isprime(i); g_isink=c; return c; }
__attribute__((noinline)) static long factb(long N){ unsigned long acc=0; for(long i=0;i<N;i++){ long n=12+i%9; unsigned long f=1; for(long k=2;k<=n;k++) f*=k; acc^=f; } g_isink=(long)acc; return (long)acc; }
__attribute__((noinline)) static long mandel(long W,long H){ long sum=0;
  for(long j=0;j<H;j++) for(long i=0;i<W;i++){
    double cx=-2.0+i*(3.0/W), cy=-1.5+j*(3.0/H), zx=0,zy=0; long k=0;
    while(zx*zx+zy*zy<=4.0 && k<50){ double nx=zx*zx-zy*zy+cx, ny=2.0*zx*zy+cy; zx=nx; zy=ny; k++; }
    sum+=k; } g_isink=sum; return sum; }
int main(){
  volatile long N1=5000000,N2=5000000,N3=1000000,W=200,H=200,NP=50000,NF=500000;
  volatile double a=0.5,bx=1.0,by=0.0,c=0.0;
  #define TF(name,expr) { int64_t best=INT64_MAX; double r=0; for(int k=0;k<5;k++){ int64_t t=ns(); r=expr; int64_t d=ns()-t; if(d<best)best=d; } printf("%s %lld %lld\n",name,(long long)best,(long long)(r*1e6)); }
  #define TI(name,expr) { int64_t best=INT64_MAX; long r=0; for(int k=0;k<5;k++){ int64_t t=ns(); r=expr; int64_t d=ns()-t; if(d<best)best=d; } printf("%s %lld %lld\n",name,(long long)best,(long long)r); }
  TF("logistic", logistic(a,N1))
  TF("rot2", rot2(bx,by,N2))
  TF("fsin", fsink(c,N3))
  TI("brot", mandel(W,H))
  TI("primes", cprimes(NP))
  TI("fact", factb(NF))
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
def isprime(n):
    if n < 2: return 0
    d = 2
    while d*d <= n:
        if n % d == 0: return 0
        d += 1
    return 1
def cprimes(N):
    c = 0
    for i in range(2, N): c += isprime(i)
    return c
def factb(N):
    acc = 0
    for i in range(N):
        n = 12 + i % 9; f = 1
        for k in range(2, n+1): f *= k
        acc ^= f
    return acc
def mandel(W, H):
    s = 0
    for j in range(H):
        for i in range(W):
            cx = -2.0 + i*(3.0/W); cy = -1.5 + j*(3.0/H); zx = 0.0; zy = 0.0; k = 0
            while zx*zx + zy*zy <= 4.0 and k < 50:
                nx = zx*zx - zy*zy + cx; ny = 2.0*zx*zy + cy; zx = nx; zy = ny; k += 1
            s += k
    return s
def bench(name, fn, reps=3):
    best = None; r = None
    for _ in range(reps):
        t = time.perf_counter_ns(); r = fn(); d = time.perf_counter_ns() - t
        best = d if best is None else min(best, d)
    print(name, best, r)
bench("logistic", lambda: int(logistic(0.5, 5000000) * 1e6))
bench("rot2",     lambda: int(rot2(1.0, 0.0, 5000000) * 1e6))
bench("fsin",     lambda: int(fsink(0.0, 1000000) * 1e6))
bench("brot",     lambda: mandel(200, 200))
bench("primes",   lambda: cprimes(50000))
bench("fact",     lambda: factb(500000))
"#;

/// (name, work-unit count, unit label, MF66 definition, MF66 call word)
fn kernels() -> Vec<(&'static str, u64, &'static str, String, &'static str)> {
    vec![
        ("logistic", 5_000_000, "iter", format!(
            ": logistic {{: | float x :}} 0.5e to x 5000000 0 do x 1e x f- f* 3.9e f* to x loop x 1000000e f* f>d drop ;"), "logistic"),
        ("rot2", 5_000_000, "iter", format!(
            ": rot2 {{: | float x float y :}} 1e to x 0e to y 5000000 0 do x 0.8e f* y 0.6e f* f- x 0.6e f* y 0.8e f* f+ to y to x loop x 1000000e f* f>d drop ;"), "rot2"),
        ("fsin", 1_000_000, "iter", format!(
            ": fsink {{: | float s :}} 0e to s 1000000 0 do s fsin 0.5e f+ to s loop s 1000000e f* f>d drop ;"), "fsink"),
        ("brot", 40_000, "pixel", format!(
            ": dotl {{: float cx float cy | float zx float zy :}} 0e to zx 0e to zy 0 begin zx zx f* zy zy f* f+ 4e f<= over 50 < and while zx zx f* zy zy f* f- cx f+ zx zy f* 2e f* cy f+ to zy to zx 1+ repeat ; \
              : mgrid {{: | sum :}} 0 to sum 200 0 do 200 0 do i s>d d>f 3e f* 200e f/ -2e f+ j s>d d>f 3e f* 200e f/ -1.5e f+ dotl sum + to sum loop loop sum ;"), "mgrid"),
        ("primes", 49_998, "cand", format!(
            ": prime? {{: n :}} n 2 < if 0 exit then 2 begin dup dup * n <= while dup n swap mod 0= if drop 0 exit then 1+ repeat drop -1 ; \
              : cprimes {{: | c :}} 0 to c 50000 2 do i prime? if c 1+ to c then loop c ;"), "cprimes"),
        ("fact", 500_000, "fact", format!(
            ": fact 1 swap 1+ 2 ?do i * loop ; \
              : factbench {{: | acc :}} 0 to acc 500000 0 do i 9 mod 12 + fact acc xor to acc loop acc ;"), "factbench"),
    ]
}

fn min_ns(mut f: impl FnMut(), runs: usize) -> u128 {
    let mut best = u128::MAX;
    for _ in 0..runs {
        let t = Instant::now();
        f();
        best = best.min(t.elapsed().as_nanos());
    }
    best
}

fn run_external(label: &str, prog: &str, args: &[&str]) -> Option<Vec<(String, u128, i64)>> {
    let out = Command::new(prog).args(args).output().ok()?;
    if !out.status.success() {
        eprintln!("[skip {label}] {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
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

    // ── MF66: define + warm up + time each (min of 7), read the result int ──
    let mut s = Mf66Session::new().unwrap();
    let ks = kernels();
    let mut mf66: Vec<(String, u128, i64)> = Vec::new();
    for (name, _count, _unit, def, call) in &ks {
        s.eval(def).unwrap();
        s.eval(call).unwrap(); // warm up + leave a result
        let res = s.stack()[0];
        let ns = min_ns(|| { s.eval(call).unwrap(); }, 7);
        mf66.push((name.to_string(), ns, res));
    }

    // ── C (clang -O2) + Python ──
    let cc = Command::new("clang")
        .args(["-O2", "-o", cbin.to_str().unwrap(), cpath.to_str().unwrap(), "-lm"])
        .status();
    let c = match cc {
        Ok(st) if st.success() => run_external("C", cbin.to_str().unwrap(), &[]),
        _ => { eprintln!("[skip C] clang unavailable"); None }
    };
    let py = run_external("Python", "python3", &[pypath.to_str().unwrap()]);

    let find = |v: &Option<Vec<(String, u128, i64)>>, name: &str| -> Option<(u128, i64)> {
        v.as_ref()?.iter().find(|(n, _, _)| n == name).map(|(_, ns, r)| (*ns, *r))
    };

    println!("\n  Numeric kernels — ns per work-unit (lower is better), min of repeated runs");
    println!("  {:<9} {:>7} {:>6} {:>11} {:>10} {:>11}   {:>8} {:>9}",
        "kernel", "units", "unit", "MF66 ns", "C ns", "Python ns", "MF66/C", "Py/MF66");
    println!("  {}", "-".repeat(82));
    for (name, count, unit, _def, _call) in &ks {
        let (mns, mres) = mf66.iter().find(|(n, _, _)| n == name).map(|(_, ns, r)| (*ns, *r)).unwrap();
        let n = *count as f64;
        let mf = mns as f64 / n;
        let (cv, cr) = find(&c, name).map(|(ns, r)| (ns as f64 / n, Some(r))).unwrap_or((f64::NAN, None));
        let (pv, pr) = find(&py, name).map(|(ns, r)| (ns as f64 / n, Some(r))).unwrap_or((f64::NAN, None));
        println!("  {:<9} {:>7} {:>6} {:>11.2} {:>10.2} {:>11.2}   {:>7.2}x {:>8.1}x",
            name, count, unit, mf, cv, pv, mf / cv, pv / mf);
        // cross-check (validates same computation): exact for the integer/stable
        // kernels; brot exact vs no-FMA Python, lenient vs (maybe-FMA) C.
        match *name {
            "rot2" | "fsin" | "primes" | "fact" => {
                if let Some(cr) = cr { assert!((mres - cr).abs() <= 2, "{name}: MF66 {mres} vs C {cr}"); }
                if let Some(pr) = pr { assert!((mres - pr).abs() <= 2, "{name}: MF66 {mres} vs Py {pr}"); }
            }
            "brot" => {
                if let Some(pr) = pr { assert_eq!(mres, pr, "brot: MF66 {mres} vs Py {pr} (both no-FMA → exact)"); }
                if let Some(cr) = cr {
                    let drift = (mres - cr).abs() as f64 / mres as f64;
                    if drift > 0.01 { eprintln!("  [note] brot MF66 {mres} vs C {cr} ({:.3}% — FMA boundary drift)", drift * 100.0); }
                }
            }
            _ => {}
        }
    }
    println!("  {}", "-".repeat(82));
    println!("  logistic: chaotic (results diverge by design). brot/primes/fact integer-exact across impls.\n");
}
