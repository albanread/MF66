//! `mf66` — the interactive Forth interpreter (classic REPL).
//!
//! Boots an `Mf66Session`, optionally loads source files named on the command
//! line, then reads lines from stdin: each line is interpreted/compiled, its
//! output printed, and ` ok` echoed in the classic Forth style. A `:` definition
//! spanning several lines is held open (continuation prompt) until `;`. Errors
//! print and reset the input state without killing the session. `bye` (or EOF /
//! Ctrl-D) exits.
//!
//!   cargo run --bin mf66                 # interactive
//!   cargo run --bin mf66 -- lib/core.f   # load files, then interactive
//!   echo '2 3 + .' | cargo run --bin mf66

use std::io::{self, BufRead, Write};

use mf66::Mf66Session;

fn main() {
    let mut s = match Mf66Session::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mf66: boot failed: {e}");
            std::process::exit(1);
        }
    };

    // Load any files given on the command line (whole-file, so multi-line
    // definitions work), before dropping into the REPL.
    let mut exit_after = false;
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-e" | "--eval" => {
                let src = args.next().unwrap_or_default();
                feed(&mut s, &src, "-e");
                exit_after = true;
            }
            "-h" | "--help" => {
                println!("usage: mf66 [-e SRC] [FILE ...]   (no args → interactive REPL)");
                return;
            }
            path => match std::fs::read_to_string(path) {
                Ok(src) => feed(&mut s, &src, path),
                Err(e) => eprintln!("mf66: cannot read {path}: {e}"),
            },
        }
        if s.wants_bye() {
            return;
        }
    }
    if exit_after {
        return;
    }

    // Interactive loop.
    let stdin = io::stdin();
    let interactive = is_tty();
    if interactive {
        println!("MF66 — optimizing Forth for Apple Silicon.  Type `bye` to exit.");
    }
    let mut line = String::new();
    loop {
        io::stdout().flush().ok();
        line.clear();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,            // EOF / Ctrl-D
            Ok(_) => {}
            Err(e) => {
                eprintln!("mf66: read error: {e}");
                break;
            }
        }
        match s.eval_out(&line) {
            Ok(out) => {
                print!("{out}");
                // Classic Forth: ` ok` after a completed interpret-state line.
                // While a `:` definition is open, withhold it (continuation).
                if interactive && !s.compiling() {
                    println!(" ok");
                }
            }
            Err(e) => {
                println!(" ✗ {e}");
                s.reset_input(); // recover: drop any half-built definition
            }
        }
        if s.wants_bye() {
            break;
        }
    }
    if interactive {
        println!();
    }
}

/// Feed a whole source string (a loaded file or `-e` argument) to the session,
/// reporting any error without aborting.
fn feed(s: &mut Mf66Session, src: &str, what: &str) {
    match s.eval_out(src) {
        Ok(out) => print!("{out}"),
        Err(e) => {
            eprintln!("mf66: {what}: {e}");
            s.reset_input();
        }
    }
}

fn is_tty() -> bool {
    // 0 == STDIN_FILENO; isatty is in libc on macOS.
    extern "C" {
        fn isatty(fd: i32) -> i32;
    }
    unsafe { isatty(0) == 1 }
}
