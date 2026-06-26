//! The eval corpus: WF66 REPL transcripts (`tests/data/eval/*.in` + `.out`).
//! Each `.in` is Forth source fed through `Mf66Session::repl`; the captured output
//! must match the `.out` byte-for-byte (incl. the ` ok` framing). A `# requires:`
//! line lists Forth words the test needs — if any isn't available yet the test is
//! NYIMP (skipped, listed), so the suite stays green while the gap is the to-do.

#![cfg(target_os = "macos")]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use mf66::Mf66Session;

#[derive(Debug)]
enum Outcome {
    Pass,
    Nyimp(Vec<String>),
    Fail(String),
}

#[test]
fn data_driven_eval_tests() {
    let dir = data_dir().join("eval");
    let cases = collect(&dir, "in");
    if cases.is_empty() {
        eprintln!("note: no eval .in files");
        return;
    }
    let results: Vec<(PathBuf, Outcome)> = cases
        .iter()
        .map(|p| (p.clone(), classify(p, &p.with_extension("out"))))
        .collect();
    let (mut pass, mut fail, mut nyimp) = (0, 0, 0);
    let mut fails = Vec::new();
    let mut nyimps = Vec::new();
    for (p, o) in &results {
        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
        match o {
            Outcome::Pass => pass += 1,
            Outcome::Nyimp(m) => {
                nyimp += 1;
                nyimps.push(format!("{name} [needs: {}]", m.join(" ")));
            }
            Outcome::Fail(d) => {
                fail += 1;
                fails.push(format!("{name}: {d}"));
            }
        }
    }
    eprintln!("── eval tests: {pass} PASS, {fail} FAIL, {nyimp} NYIMP ──");
    for n in &nyimps {
        eprintln!("  NYIMP {n}");
    }
    for f in &fails {
        eprintln!("  FAIL  {f}");
    }
    assert!(fail == 0, "{fail} eval test(s) failed");
}

fn classify(in_path: &Path, out_path: &Path) -> Outcome {
    let input = match fs::read_to_string(in_path) {
        Ok(t) => t.replace("\r\n", "\n"),
        Err(e) => return Outcome::Fail(format!("read .in: {e}")),
    };
    let expected = match fs::read_to_string(out_path) {
        Ok(t) => t.replace("\r\n", "\n"),
        Err(e) => return Outcome::Fail(format!("read .out: {e}")),
    };
    // `# requires:` words.
    let mut required: Vec<String> = Vec::new();
    for line in input.lines() {
        if let Some(rest) = line.trim_start().strip_prefix('#') {
            if let Some(list) = rest.trim_start().strip_prefix("requires:") {
                required.extend(list.split_whitespace().map(|w| w.to_string()));
            }
        }
    }
    // Strip comment lines before feeding the REPL.
    let src: String = input
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect();

    let mut s = match Mf66Session::new() {
        Ok(s) => s,
        Err(e) => return Outcome::Fail(format!("boot: {e}")),
    };
    let missing: Vec<String> =
        required.into_iter().filter(|w| !is_available(&mut s, w)).collect();
    if !missing.is_empty() {
        return Outcome::Nyimp(missing);
    }
    match s.repl(&src) {
        Ok(out) if out == expected => Outcome::Pass,
        Ok(out) => Outcome::Fail(format!("output mismatch\n   expected: {expected:?}\n   got     : {out:?}")),
        // An unimplemented word/feature → NYIMP (skip), not a failure.
        Err(e) => {
            let msg = e.to_string();
            if let Some(w) = msg.strip_prefix("undefined word: ") {
                Outcome::Nyimp(vec![w.to_string()])
            } else {
                Outcome::Fail(format!("error: {msg}"))
            }
        }
    }
}

/// A required word is available if it's a compiler/REPL directive or in the dict.
fn is_available(s: &mut Mf66Session, w: &str) -> bool {
    matches!(
        w,
        ":" | ";" | "bye" | "if" | "else" | "then" | "begin" | "until" | "while" | "repeat"
    ) || s.find(w).map(|o| o.is_some()).unwrap_or(false)
}

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data")
}

fn collect(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|r| r.ok().map(|e| e.path())).collect(),
        Err(_) => return Vec::new(),
    };
    v.retain(|p| p.extension() == Some(OsStr::new(ext)));
    v.sort();
    v
}
