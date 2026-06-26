//! `asmcheck <proc-file.masm>` — validate that a kernel proc snippet assembles.
//!
//! Wraps the snippet with `.text` + the kernel macros, runs it through the
//! front-end (+ the AArch64 `stk` macro) and the native A64 encoder, and exits 0
//! if it assembles or 1 with the error. Lets the Phase 2 integration filter out
//! translations that don't even assemble before they reach the shared kernel.

#![cfg(target_os = "macos")]

use std::fs;
use std::path::Path;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: asmcheck <proc-file.masm>");
        exit(2);
    }
    let manifest = env!("CARGO_MANIFEST_DIR");
    let macros = match fs::read_to_string(Path::new(manifest).join("kernel/macros.masm")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("read macros.masm: {e}");
            exit(2);
        }
    };
    let proc = match fs::read_to_string(&args[1]) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("read {}: {e}", args[1]);
            exit(2);
        }
    };
    let src = format!(".text\n{macros}\n{proc}\n");

    let mut asm = wfasm::Assembler::new();
    asm.register_macro("stk", mf66::asm_macros::stk);
    let text = match asm.assemble("asmcheck", &src) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("EXPAND ERROR: {e}");
            exit(1);
        }
    };
    match wfasm::a64::assemble(&text) {
        Ok(_) => println!("OK"),
        Err(e) => {
            eprintln!("A64 ERROR: {e}");
            exit(1);
        }
    }
}
