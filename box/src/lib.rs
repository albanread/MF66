//! MF66 for the BOX: the C API the native module (A7232ToolChain
//! rosgd/modules/forth/forth.c) calls. See box/mf66_box.h and
//! design/BOX_PORT.md.
//!
//! One session at a time, owned here; the module runs one Forth at a time.
//! Every entry point catches a Rust panic and reports it as an error, so a
//! panic never unwinds into C (it would abort /init).

use std::panic::{catch_unwind, AssertUnwindSafe};

use mf66::host::{self, Hooks};
use mf66::Mf66Session;

/// The words the box adds in Forth: the register block and the SWI words.
pub const PRELUDE: &str = include_str!("prelude.fs");

static mut SESSION: Option<Mf66Session> = None;

#[allow(static_mut_refs)]
fn session() -> Option<&'static mut Mf66Session> {
    unsafe { SESSION.as_mut() }
}

fn say(s: &str) {
    mf66::runtime::out(s.as_bytes());
}

fn guarded(f: impl FnOnce() -> i32) -> i32 {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(e) => {
            let m = e
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown".into());
            say(&format!("\nF66: internal error: {m}\n"));
            -1
        }
    }
}

/// Install the host's hooks (a table that lives as long as the program).
#[no_mangle]
pub extern "C" fn mf66_box_set_hooks(h: *const Hooks) {
    host::set_hooks(h);
}

/// Boot a fresh session (ending any other): 0, or -1 with the reason printed.
#[no_mangle]
pub extern "C" fn mf66_box_boot() -> i32 {
    guarded(|| {
        unsafe { SESSION = None };
        let mut s = match Mf66Session::new() {
            Ok(s) => s,
            Err(e) => {
                say(&format!("F66: the Forth did not start: {e:#}\n"));
                return -1;
            }
        };
        for (i, line) in PRELUDE.lines().enumerate() {
            if let Err(e) = s.eval(line) {
                say(&format!("F66: prelude line {}: {e:#}\n", i + 1));
                return -1;
            }
        }
        defined().clear();
        note_definitions(PRELUDE);
        unsafe { SESSION = Some(s) };
        0
    })
}

/// End the session.
#[no_mangle]
pub extern "C" fn mf66_box_end() {
    guarded(|| {
        unsafe { SESSION = None };
        0
    });
}

/// Is there a session?
#[no_mangle]
pub extern "C" fn mf66_box_running() -> i32 {
    session().is_some() as i32
}

fn bytes<'a>(p: *const u8, n: usize) -> &'a str {
    if p.is_null() || n == 0 {
        return "";
    }
    let b = unsafe { std::slice::from_raw_parts(p, n) };
    std::str::from_utf8(b).unwrap_or_else(|e| {
        // Latin-1 text: keep what is valid; RISC OS files are mostly ASCII.
        std::str::from_utf8(&b[..e.valid_up_to()]).unwrap_or("")
    })
}

/// Interpret one line (the REPL): 0 done, 1 `bye`, -1 an error (printed, the
/// input state reset).
#[no_mangle]
pub extern "C" fn mf66_box_line(p: *const u8, n: usize) -> i32 {
    guarded(|| {
        let Some(s) = session() else { return -1 };
        let line = bytes(p, n);
        note_definitions(line);
        match s.eval(line) {
            Ok(()) => {
                if s.wants_bye() {
                    1
                } else {
                    0
                }
            }
            Err(e) => {
                say(&format!(" ? {e:#}\n"));
                s.reset_input();
                -1
            }
        }
    })
}

/// Interpret a whole source text, line by line, so an error names its line
/// (`name:line: error: ...`): 0, 1 at `bye`, -1 at the first error.
#[no_mangle]
pub extern "C" fn mf66_box_eval(p: *const u8, n: usize, name: *const u8, nlen: usize) -> i32 {
    guarded(|| {
        let Some(s) = session() else { return -1 };
        let name = bytes(name, nlen).to_string();
        // Copy: the text may be in a buffer the program itself reuses.
        let src = bytes(p, n).to_string();
        note_definitions(&src);
        for (i, line) in src.lines().enumerate() {
            if let Err(e) = s.eval(line) {
                say(&format!("\n{name}:{}: error: {e:#}\n", i + 1));
                s.reset_input();
                return -1;
            }
            if s.wants_bye() {
                return 1;
            }
        }
        0
    })
}

/// Is a definition open (the REPL prints ` compiled` rather than ` ok`)?
#[no_mangle]
pub extern "C" fn mf66_box_compiling() -> i32 {
    session().map(|s| s.compiling() as i32).unwrap_or(0)
}

/// After a fault escaped from Forth code: drop any half-made definition and
/// clear the stacks.
#[no_mangle]
pub extern "C" fn mf66_box_recover() {
    guarded(|| {
        if let Some(s) = session() {
            s.reset_input();
        }
        0
    });
}

// ── For a console (the module's Wimp REPL window) ─────────────────────────

/// The data stack's depth.
#[no_mangle]
pub extern "C" fn mf66_box_depth() -> i64 {
    session().map(|s| s.depth() as i64).unwrap_or(0)
}

/// Up to `max` stack cells into `out`, top first: how many were written.
#[no_mangle]
pub extern "C" fn mf66_box_stack(out: *mut i64, max: usize) -> i64 {
    let Some(s) = session() else { return 0 };
    if out.is_null() {
        return 0;
    }
    let v = s.stack();
    let n = v.len().min(max);
    for (i, x) in v.iter().take(n).enumerate() {
        unsafe { out.add(i).write(*x) };
    }
    n as i64
}

/// The number base (`base`).
#[no_mangle]
pub extern "C" fn mf66_box_base() -> i64 {
    session().map(|s| s.num_base() as i64).unwrap_or(10)
}

/// The words a session knows, newest first, separated by spaces, written
/// into buf (zero-terminated, cut at max): the length written. The words a
/// program defined come first, then the box's (the prelude), then the
/// kernel's in alphabetical order.
#[no_mangle]
pub extern "C" fn mf66_box_words(buf: *mut u8, max: usize) -> usize {
    if buf.is_null() || max == 0 {
        return 0;
    }
    let mut t = String::new();
    let mut seen = std::collections::HashSet::new();
    let defs = defined();
    for (name, _) in defs.iter().rev() {
        if seen.insert(name.clone()) {
            t.push_str(name);
            t.push(' ');
        }
    }
    let mut prims: Vec<&str> = mf66::primitives::PRIMITIVES
        .iter()
        .map(|p| p.0)
        .filter(|n| !n.starts_with('(') && !seen.contains(*n))
        .collect();
    for w in PARSED_WORDS {
        if !seen.contains(*w) {
            prims.push(w);
        }
    }
    prims.sort();
    prims.dedup();
    for p in prims {
        t.push_str(p);
        t.push(' ');
    }
    let b = t.trim_end().as_bytes();
    let n = b.len().min(max - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(b.as_ptr(), buf, n);
        buf.add(n).write(0);
    }
    n
}

/// A word's stack comment, as `( a b -- c )`, into buf (zero-terminated):
/// its length, 0 if none is known. A program's own words give the comment
/// written after their name; the kernel's come from a table here.
#[no_mangle]
pub extern "C" fn mf66_box_word_help(name: *const u8, n: usize, buf: *mut u8, max: usize) -> usize {
    if buf.is_null() || max == 0 {
        return 0;
    }
    let name = bytes(name, n).to_ascii_lowercase();
    let found = defined()
        .iter()
        .rev()
        .find(|(w, _)| w.eq_ignore_ascii_case(&name))
        .and_then(|(_, c)| c.clone())
        .or_else(|| {
            KERNEL_HELP
                .iter()
                .find(|(w, _)| *w == name)
                .map(|(_, c)| c.to_string())
        });
    let Some(c) = found else { return 0 };
    let b = c.as_bytes();
    let k = b.len().min(max - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(b.as_ptr(), buf, k);
        buf.add(k).write(0);
    }
    k
}

/// Words the outer interpreter parses itself (not in the kernel's table).
const PARSED_WORDS: &[&str] = &[
    ":", ";", "variable", "constant", "value", "to", "create", "does>", "include", "s\"", ".\"",
    "z\"", "swi\"", "char", "[char]", "'", "[']", "if", "else", "then", "begin", "until", "while",
    "repeat", "do", "?do", "loop", "+loop", "leave", "exit", "recurse", "immediate", "postpone",
    "(", "\\", "bye",
];

/// Stack comments for the kernel's common words.
const KERNEL_HELP: &[(&str, &str)] = &[
    ("dup", "( x -- x x )"), ("drop", "( x -- )"), ("swap", "( a b -- b a )"),
    ("over", "( a b -- a b a )"), ("rot", "( a b c -- b c a )"), ("-rot", "( a b c -- c a b )"),
    ("nip", "( a b -- b )"), ("tuck", "( a b -- b a b )"), ("pick", "( xu .. x0 u -- xu .. x0 xu )"),
    ("2dup", "( a b -- a b a b )"), ("2drop", "( a b -- )"), ("2swap", "( a b c d -- c d a b )"),
    ("2over", "( a b c d -- a b c d a b )"), ("?dup", "( x -- 0 | x x )"), ("depth", "( -- n )"),
    (">r", "( x -- ) ( R: -- x )"), ("r>", "( -- x ) ( R: x -- )"), ("r@", "( -- x ) ( R: x -- x )"),
    ("+", "( a b -- a+b )"), ("-", "( a b -- a-b )"), ("*", "( a b -- a*b )"), ("/", "( a b -- a/b )"),
    ("mod", "( a b -- a mod b )"), ("/mod", "( a b -- rem quot )"), ("*/", "( a b c -- a*b/c )"),
    ("negate", "( n -- -n )"), ("abs", "( n -- |n| )"), ("min", "( a b -- min )"), ("max", "( a b -- max )"),
    ("1+", "( n -- n+1 )"), ("1-", "( n -- n-1 )"), ("2*", "( n -- 2n )"), ("2/", "( n -- n/2 )"),
    ("and", "( a b -- a&b )"), ("or", "( a b -- a|b )"), ("xor", "( a b -- a^b )"), ("invert", "( x -- ~x )"),
    ("lshift", "( x u -- x<<u )"), ("rshift", "( x u -- x>>u )"),
    ("=", "( a b -- flag )"), ("<>", "( a b -- flag )"), ("<", "( a b -- flag )"), (">", "( a b -- flag )"),
    ("0=", "( x -- flag )"), ("0<", "( n -- flag )"), ("0>", "( n -- flag )"), ("u<", "( u1 u2 -- flag )"),
    ("within", "( n lo hi -- flag )"),
    ("@", "( addr -- x )"), ("!", "( x addr -- )"), ("c@", "( addr -- c )"), ("c!", "( c addr -- )"),
    ("+!", "( n addr -- )"), ("cells", "( n -- n*8 )"), ("cell+", "( addr -- addr+8 )"), ("chars", "( n -- n )"),
    ("here", "( -- addr )"), ("allot", "( n -- )"), (",", "( x -- )"), ("c,", "( c -- )"),
    ("fill", "( addr u c -- )"), ("move", "( from to u -- )"), ("erase", "( addr u -- )"),
    ("count", "( c-addr -- addr u )"), ("type", "( addr u -- )"), ("emit", "( c -- )"), ("cr", "( -- )"),
    ("space", "( -- )"), ("spaces", "( n -- )"), (".", "( n -- )"), ("u.", "( u -- )"), (".r", "( n w -- )"),
    (".s", "( -- )"), ("key", "( -- c )"), ("key?", "( -- flag )"), ("accept", "( addr u -- u' )"),
    ("ms", "( u -- )"), ("compare", "( a1 u1 a2 u2 -- n )"), ("execute", "( xt -- )"),
    ("throw", "( n -- )"), ("catch", "( xt -- n )"), ("i", "( -- n )"), ("j", "( -- n )"),
    ("allocate", "( u -- addr ior )"), ("free", "( addr -- ior )"), ("resize", "( addr u -- addr' ior )"),
    ("open-file", "( c-addr u fam -- fileid ior )"), ("create-file", "( c-addr u fam -- fileid ior )"),
    ("close-file", "( fileid -- ior )"), ("read-file", "( addr u fileid -- u' ior )"),
    ("write-file", "( addr u fileid -- ior )"), ("read-line", "( addr u fileid -- u' flag ior )"),
    ("write-line", "( addr u fileid -- ior )"), ("delete-file", "( c-addr u -- ior )"),
    ("decimal", "( -- )"), ("hex", "( -- )"), ("base", "( -- addr )"),
    (":", "( \"name\" -- ) begins a definition"), (";", "( -- ) ends a definition"),
    ("variable", "( \"name\" -- )  name: ( -- addr )"), ("constant", "( x \"name\" -- )  name: ( -- x )"),
    ("value", "( x \"name\" -- )  name: ( -- x )"), ("to", "( x \"name\" -- )"),
    ("create", "( \"name\" -- )  name: ( -- addr )"), ("does>", "( -- ) the run time of a created word"),
    ("include", "( \"file\" -- ) loads a Forth file"), ("z\"", "( \"text\" -- c-addr )"),
    ("s\"", "( \"text\" -- addr u )"), ("swi\"", "( \"name\" -- n )"),
    ("sys", "( x0 .. xk-1 k n -- ) calls SWI n"), ("xsys", "( x0 .. xk-1 k n -- err|0 )"),
];

/// The words programs have defined, in order, with the stack comment after
/// the name where there is one. Found by reading the source as it is
/// given (mf66_box_line and _eval), not from the dictionary.
static mut DEFINED: Vec<(String, Option<String>)> = Vec::new();

#[allow(static_mut_refs)]
fn defined() -> &'static mut Vec<(String, Option<String>)> {
    unsafe { &mut DEFINED }
}

fn note_definitions(text: &str) {
    const DEFINERS: &[&str] = &[
        ":", "variable", "constant", "value", "create", "2variable", "2constant", "fvariable",
        "fconstant", "defer", "buffer:",
    ];
    for line in text.lines() {
        let line = match line.find(" \\ ") {
            Some(i) => &line[..i],
            None => line,
        };
        let toks: Vec<&str> = line.split_whitespace().collect();
        let mut i = 0;
        while i < toks.len() {
            let t = toks[i].to_ascii_lowercase();
            if t == "\\" {
                break;
            }
            if t == "include" && i + 1 < toks.len() {
                // the file's words too (it is read again when it runs)
                if let Ok(text) = mf66::runtime::read_whole(toks[i + 1]) {
                    note_definitions(&text);
                }
                i += 2;
                continue;
            }
            if DEFINERS.contains(&t.as_str()) && i + 1 < toks.len() {
                let name = toks[i + 1].to_string();
                let mut comment = None;
                if t == ":" && toks.get(i + 2) == Some(&"(") {
                    if let Some(end) = toks[i + 2..].iter().position(|w| w.ends_with(')')) {
                        comment = Some(toks[i + 2..=i + 2 + end].join(" "));
                    }
                }
                defined().push((name, comment));
                i += 2;
                continue;
            }
            i += 1;
        }
    }
}
