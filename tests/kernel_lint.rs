//! ABI register-discipline gate for the hand-written kernel (design §2).
//!
//! There is no encoder guard for these, so this grep gate is the enforcement:
//!   - `x18`/`w18` — the Darwin platform register, reserved; touching it is UB.
//!   - `q8`–`q15` — only the low 64 bits of v8–v15 are callee-saved; a 128-bit
//!     write to one corrupts a saved FP register (e.g. FTOS=d8).

#![cfg(target_os = "macos")]

use std::fs;
use std::path::PathBuf;

const FORBIDDEN: &[&str] = &[
    "x18", "w18", "q8", "q9", "q10", "q11", "q12", "q13", "q14", "q15",
];

#[test]
fn kernel_avoids_forbidden_registers() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("kernel");
    let mut violations = Vec::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("masm") {
            continue;
        }
        let src = fs::read_to_string(&path).unwrap();
        for (i, line) in src.lines().enumerate() {
            let code = line.split(';').next().unwrap_or("").to_lowercase(); // strip comments
            for &bad in FORBIDDEN {
                if contains_token(&code, bad) {
                    violations.push(format!(
                        "{}:{}: forbidden register `{bad}` in `{}`",
                        path.file_name().unwrap().to_string_lossy(),
                        i + 1,
                        code.trim()
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "MF66 ABI register-discipline violations (design §2):\n{}",
        violations.join("\n")
    );
}

/// `needle` appears in `hay` bounded by non-identifier chars (so `0x18` does not
/// match `x18`, and `q15` does not match `q150`).
fn contains_token(hay: &str, needle: &str) -> bool {
    let bytes = hay.as_bytes();
    let mut from = 0;
    while let Some(pos) = hay[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0 || !is_ident(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_ident(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
