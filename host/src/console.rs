//! For a console (rosgd/modules/forth/forthcon.c): the words a session
//! knows and their stack comments, shared by MF66 and WF66. The words
//! programs define are found by reading their source as it is given.

/// A word's stack comment: a program's own (written after its name), or one
/// from [`KERNEL_HELP`].
pub fn word_help(name: &str) -> Option<String> {
    let name = name.to_ascii_lowercase();
    defined()
        .iter()
        .rev()
        .find(|(w, _)| w.eq_ignore_ascii_case(&name))
        .and_then(|(_, c)| c.clone())
        .or_else(|| KERNEL_HELP.iter().find(|(w, _)| *w == name).map(|(_, c)| c.to_string()))
}

/// The words known, newest first, separated by spaces: the programs' own,
/// then `builtin` (the kernel's and the prelude's) with [`PARSED_WORDS`] in
/// alphabetical order.
pub fn words<'a>(builtin: impl Iterator<Item = &'a str>) -> String {
    let mut t = String::new();
    let mut seen = std::collections::HashSet::new();
    for (name, _) in defined().iter().rev() {
        if seen.insert(name.clone()) {
            t.push_str(name);
            t.push(' ');
        }
    }
    let mut prims: Vec<&str> = builtin
        .filter(|n| !n.starts_with('('))
        .chain(PARSED_WORDS.iter().copied())
        .filter(|n| !seen.contains(*n))
        .collect();
    prims.sort();
    prims.dedup();
    for p in prims {
        t.push_str(p);
        t.push(' ');
    }
    t.trim_end().to_string()
}

/// Copy `s` into (buf, max) zero-terminated, cut to fit: the length written.
pub fn put_cstr(s: &str, buf: *mut u8, max: usize) -> usize {
    if buf.is_null() || max == 0 {
        return 0;
    }
    let b = s.as_bytes();
    let n = b.len().min(max - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(b.as_ptr(), buf, n);
        buf.add(n).write(0);
    }
    n
}

/// Words the outer interpreter parses itself (not in the kernel's table).
pub const PARSED_WORDS: &[&str] = &[
    ":", ";", "variable", "constant", "value", "to", "create", "does>", "include", "s\"", ".\"",
    "z\"", "swi\"", "char", "[char]", "'", "[']", "if", "else", "then", "begin", "until", "while",
    "repeat", "do", "?do", "loop", "+loop", "leave", "exit", "recurse", "immediate", "postpone",
    "(", "\\", "bye",
];

/// Stack comments for the kernel's common words.
pub const KERNEL_HELP: &[(&str, &str)] = &[
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
pub fn defined() -> &'static mut Vec<(String, Option<String>)> {
    unsafe { &mut DEFINED }
}

/// Note the definitions in `text`; `read` gives an included file's text.
pub fn note_definitions(text: &str, read: &dyn Fn(&str) -> Option<String>) {
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
                if let Some(text) = read(toks[i + 1]) {
                    note_definitions(&text, read);
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
