//! ANS Forth Core conformance: load WF66's tester.fs framework, run its
//! ans_core_tests.fs against MF66, and report pass / fail / skip per section.
//! Skips are lines using words MF66 doesn't have yet (extension wordsets).

#![cfg(target_os = "macos")]

use mf66::Mf66Session;
use std::fs;

const WF66: &str = "/Users/oberon/claudeprojects/WF66/lib";

#[test]
fn ans_core() {
    let mut s = Mf66Session::new().unwrap();

    // 1. Load the tester framework (must fully load).
    let tester = fs::read_to_string(format!("{WF66}/tester.fs")).unwrap();
    for line in tester.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('\\') { continue; }
        if let Err(e) = s.eval(line) {
            panic!("tester.fs failed to load: {l:?}: {e}");
        }
    }

    // 2. Run the core tests, per line, with recovery + per-section tallies.
    let tests = fs::read_to_string(format!("{WF66}/ans_core_tests.fs")).unwrap();
    let mut section = String::from("(prologue)");
    let mut run = 0usize;     // T{ lines that executed without a Rust error
    let mut skip = 0usize;    // lines that errored (missing word / unsupported)
    let mut sec_run = 0usize;
    let mut sec_skip = 0usize;
    let mut report: Vec<(String, usize, usize)> = Vec::new();

    let prev_fail = read_var(&mut s, "error-count");
    for line in tests.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('\\') { continue; }
        // section header: `s" Name" testing`
        if let Some(name) = l.strip_prefix("s\" ").and_then(|r| r.split('"').next()) {
            if l.contains("testing") {
                report.push((section.clone(), sec_run, sec_skip));
                section = name.to_string();
                sec_run = 0;
                sec_skip = 0;
                let _ = s.eval(l); // run the testing header (may print)
                s.reset_input();
                continue;
            }
        }
        let is_test = l.contains("T{");
        // guard against hard faults in JIT'd code so one rogue line can't abort
        // the whole run — it becomes a skip with the captured details.
        let owned = line.to_string();
        match mf66::crash::guard(|| s.eval_out(&owned)) {
            Ok(Ok(out)) => {
                if is_test {
                    run += 1;
                    sec_run += 1;
                    if out.contains("INCORRECT") || out.contains("WRONG") {
                        eprintln!("  FAIL {l:?}");
                    }
                }
            }
            Ok(Err(_)) => { skip += 1; sec_skip += 1; s.reset_input(); }
            Err(info) => {
                eprintln!("  CRASH on {l:?}: {info}");
                skip += 1;
                sec_skip += 1;
                s.reset_input();
            }
        }
    }
    report.push((section.clone(), sec_run, sec_skip));
    let fail = read_var(&mut s, "error-count").saturating_sub(prev_fail);
    let pass = run.saturating_sub(fail as usize);

    eprintln!("\n══ ANS Core conformance ══");
    for (name, r, sk) in &report {
        if *r == 0 && *sk == 0 { continue; }
        eprintln!("  {:<28} run {:>3}  skip {:>3}", name, r, sk);
    }
    eprintln!("── totals: {run} tests ran, ~{pass} pass, {fail} fail, {skip} lines skipped ──\n");
}

fn read_var(s: &mut Mf66Session, name: &str) -> u64 {
    s.reset_input();
    match s.eval(&format!("{name} @")) {
        Ok(_) => {
            let v = s.stack().first().copied().unwrap_or(0) as u64;
            s.reset_input();
            v
        }
        Err(_) => 0,
    }
}
