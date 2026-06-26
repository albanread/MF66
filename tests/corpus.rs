//! The differential corpus harness (design §7) — MF66's primary oracle.
//!
//! Runs the committed `tests/data/direct/*.t` corpus (imported from WF66, where
//! each expected value was validated against WF66's observable Forth state on
//! x86) through `Mf66Session`. A primitive whose asm symbol isn't in the kernel
//! yet is auto-classified **NYIMP** and listed — the NYIMP list is the porting
//! to-do list; porting a primitive flips it NYIMP → PASS, and PASS means "matches
//! WF66's observed behavior".
//!
//! The DSL (push/call/expect/reset + poke/expect_bytes/push_pad) asserts only
//! *observable state* (final stack, memory bytes) — never code bytes or dict
//! addresses, which legitimately differ across ISAs.
//!
//! Ported from WF66 `tests/harness.rs`. The `eval` (.in/.out) corpus comes online
//! once the interpreter boots (later in Phase 2).

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
fn data_driven_direct_tests() {
    let dir = data_dir().join("direct");
    let cases = collect_files(&dir, "t");
    if cases.is_empty() {
        eprintln!("note: no .t files under {} — nothing to run", dir.display());
        return;
    }
    let results: Vec<(PathBuf, Outcome)> =
        cases.iter().map(|p| (p.clone(), classify_direct(p))).collect();
    summarize_and_assert("direct", &results);
}

fn summarize_and_assert(kind: &str, results: &[(PathBuf, Outcome)]) {
    let (mut pass, mut fail, mut nyimp) = (0, 0, 0);
    let mut nyimp_list = Vec::new();
    let mut fail_list = Vec::new();
    for (path, outcome) in results {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match outcome {
            Outcome::Pass => pass += 1,
            Outcome::Nyimp(missing) => {
                nyimp += 1;
                nyimp_list.push(format!("{name} [missing: {}]", missing.join(" ")));
            }
            Outcome::Fail(msg) => {
                fail += 1;
                fail_list.push((name, msg.clone()));
            }
        }
    }
    eprintln!("── {kind} tests: {pass} PASS, {fail} FAIL, {nyimp} NYIMP ──");
    if !nyimp_list.is_empty() {
        eprintln!("  NYIMP:");
        for line in &nyimp_list {
            eprintln!("    {line}");
        }
    }
    if !fail_list.is_empty() {
        eprintln!("  FAIL:");
        for (name, msg) in &fail_list {
            eprintln!("    {name}: {msg}");
        }
        panic!("{fail} {kind} test(s) failed (see stderr for detail)");
    }
}

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data")
}

fn collect_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|r| r.ok().map(|e| e.path())).collect(),
        Err(_) => return Vec::new(),
    };
    v.retain(|p| p.extension() == Some(OsStr::new(ext)));
    v.sort();
    v
}

/// Direct-DSL commands: `#`/`;` comment, `push <int>`, `push_pad <off>`,
/// `poke <pad-off> <hex>`, `expect_bytes <pad-off> <hex>`, `call <sym>`,
/// `expect <int>…` (bottom-first), `reset`.
fn classify_direct(path: &Path) -> Outcome {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return Outcome::Fail(format!("read failed: {e}")),
    };

    let mut s = match Mf66Session::new() {
        Ok(s) => s,
        Err(e) => return Outcome::Fail(format!("session boot: {e}")),
    };

    // Pre-scan for missing asm symbols → NYIMP.
    let mut missing: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = strip_comment(line).trim();
        if let Some(rest) = trimmed.strip_prefix("call ") {
            let sym = rest.split_whitespace().next().unwrap_or("");
            if s.xt_of(sym).is_err() && !missing.contains(&sym.to_string()) {
                missing.push(sym.to_string());
            }
        }
    }
    if !missing.is_empty() {
        return Outcome::Nyimp(missing);
    }

    let pad_base = s.pad_base();
    for (i, line) in text.lines().enumerate() {
        let lineno = i + 1;
        let body = strip_comment(line).trim();
        if body.is_empty() {
            continue;
        }
        let mut parts = body.split_whitespace();
        let cmd = parts.next().unwrap();
        let res = (|| -> Result<(), String> {
            match cmd {
                "push" => {
                    let raw = parts.next().ok_or("push needs a value")?;
                    let v = parse_int(raw).ok_or_else(|| format!("bad int `{raw}`"))?;
                    s.push(v);
                }
                "push_pad" => {
                    let raw = parts.next().ok_or("push_pad needs an offset")?;
                    let off = parse_int(raw).ok_or_else(|| format!("bad int `{raw}`"))?;
                    s.push((pad_base as i64).wrapping_add(off));
                }
                "poke" => {
                    let off_raw = parts.next().ok_or("poke needs an offset")?;
                    let hex = parts.next().ok_or("poke needs hex bytes")?;
                    let off = parse_int(off_raw).ok_or_else(|| format!("bad offset `{off_raw}`"))?;
                    let bytes = parse_hex_bytes(hex).ok_or_else(|| format!("bad hex `{hex}`"))?;
                    let dst = (pad_base as i64).wrapping_add(off) as *mut u8;
                    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len()) };
                }
                "expect_bytes" => {
                    let off_raw = parts.next().ok_or("expect_bytes needs an offset")?;
                    let hex = parts.next().ok_or("expect_bytes needs hex bytes")?;
                    let off = parse_int(off_raw).ok_or_else(|| format!("bad offset `{off_raw}`"))?;
                    let want = parse_hex_bytes(hex).ok_or_else(|| format!("bad hex `{hex}`"))?;
                    let src = (pad_base as i64).wrapping_add(off) as *const u8;
                    let got: Vec<u8> = unsafe { std::slice::from_raw_parts(src, want.len()).to_vec() };
                    if got != want {
                        return Err(format!(
                            "bytes mismatch at PAD+{off:#x}\n      expected: {}\n      got     : {}",
                            hex_bytes(&want),
                            hex_bytes(&got)
                        ));
                    }
                }
                "call" => {
                    let sym = parts.next().ok_or("call needs a symbol")?;
                    s.call(sym).map_err(|e| format!("call {sym}: {e}"))?;
                }
                "expect" => {
                    let want_bot_first: Vec<i64> = parts
                        .map(|t| parse_int(t).ok_or_else(|| format!("bad int `{t}`")))
                        .collect::<Result<_, _>>()?;
                    let want: Vec<i64> = want_bot_first.iter().rev().copied().collect();
                    let got = s.stack();
                    if got != want {
                        return Err(format!(
                            "stack mismatch\n      expected (bottom→top): {want_bot_first:?}\n      got      (top→bottom): {got:?}"
                        ));
                    }
                }
                "reset" => s.reset(),
                other => return Err(format!("unknown command `{other}`")),
            }
            Ok(())
        })();
        if let Err(msg) = res {
            return Outcome::Fail(format!("line {lineno}: {msg}"));
        }
    }
    Outcome::Pass
}

// ── helpers (ported verbatim from WF66 tests/harness.rs) ─────────────────
fn strip_comment(line: &str) -> &str {
    let cut = line.find(|c| c == '#' || c == ';').unwrap_or(line.len());
    &line[..cut]
}

fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| *c != '_').collect();
    if cleaned.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    for pair in cleaned.as_bytes().chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
    }
    Some(out)
}

fn hex_bytes(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for byte in b {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

fn parse_int(s: &str) -> Option<i64> {
    let cleaned: String = s.chars().filter(|c| *c != '_').collect();
    let s: &str = &cleaned;
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok().map(|u| u as i64)
    } else if let Some(neg_hex) = s.strip_prefix("-0x").or_else(|| s.strip_prefix("-0X")) {
        u64::from_str_radix(neg_hex, 16).ok().map(|u| (u as i64).wrapping_neg())
    } else {
        s.parse().ok()
    }
}
