//! Forth syntax classification for the editor. `highlight` splits a source line
//! into spans tagged by token class; the workspace maps each tag to a colour and
//! emits a `DrawText` run. Pure + testable — per-line (no cross-line comment
//! state), which covers ordinary Forth (`\ …`, `( … )`, `s" … "` on one line).

/// Token class for colouring.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tag {
    Normal,
    Comment, // \ … and ( … )
    Str,     // s" … "  ." … "  .( … )
    Def,     // : ; and the defined name
    Control, // if then begin … do loop …
    Number,
    Core, // common primitives
}

const CONTROL: &[&str] = &[
    "if", "else", "then", "begin", "until", "while", "repeat", "do", "loop", "+loop", "?do",
    "leave", "again", "unloop", "case", "of", "endof", "endcase", "recurse", "exit", "does>",
    "[if]", "[else]", "[then]", "unless",
];

const CORE: &[&str] = &[
    "dup", "drop", "swap", "over", "nip", "rot", "-rot", "tuck", "?dup", "2dup", "2drop", "2swap",
    "+", "-", "*", "/", "mod", "/mod", "1+", "1-", "2*", "2/", "negate", "abs", "min", "max",
    "and", "or", "xor", "invert", "lshift", "rshift", "=", "<>", "<", ">", "<=", ">=", "u<",
    "0=", "0<", "0>", "@", "!", "+!", "c@", "c!", ".", "cr", "emit", "type", "space", ".s",
    "f+", "f-", "f*", "f/", "f.", "f@", "f!", "fdup", "fdrop", "fswap", "fsqrt", "fabs", "f<",
    ">r", "r>", "r@", "i", "j", "pick", "roll", "execute", "here", "allot", ",",
];

fn is_number(w: &str) -> bool {
    let s = w.strip_prefix(['-', '+']).unwrap_or(w);
    if s.is_empty() {
        return false;
    }
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix('$')) {
        return !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit());
    }
    if let Some(b) = s.strip_prefix('%') {
        return !b.is_empty() && b.chars().all(|c| c == '0' || c == '1');
    }
    // decimal int, or a float (digits with . and/or e)
    if s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    let floaty = s.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '-' | '+'));
    floaty && s.chars().any(|c| c.is_ascii_digit()) && (s.contains('.') || s.contains(['e', 'E']))
}

fn classify(w: &str) -> Tag {
    if w == ":" || w == ";" || w == ":noname" {
        Tag::Def
    } else if CONTROL.contains(&w) {
        Tag::Control
    } else if is_number(w) {
        Tag::Number
    } else if CORE.contains(&w) {
        Tag::Core
    } else {
        Tag::Normal
    }
}

/// Split `line` into consecutive (text, Tag) spans covering every character
/// (whitespace included, as `Normal`).
pub fn highlight(line: &str) -> Vec<(String, Tag)> {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut spans: Vec<(String, Tag)> = Vec::new();
    let mut i = 0;
    let mut name_next = false; // the word after `:` is the definition name
    let push = |spans: &mut Vec<(String, Tag)>, s: String, t: Tag| {
        if !s.is_empty() {
            spans.push((s, t));
        }
    };
    while i < n {
        if chars[i].is_whitespace() {
            let start = i;
            while i < n && chars[i].is_whitespace() {
                i += 1;
            }
            push(&mut spans, chars[start..i].iter().collect(), Tag::Normal);
            continue;
        }
        // read a word (run of non-whitespace)
        let start = i;
        while i < n && !chars[i].is_whitespace() {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();

        if word == "\\" {
            // line comment to end of line
            push(&mut spans, chars[start..n].iter().collect(), Tag::Comment);
            break;
        }
        if word == "(" {
            // paren comment to matching ) (or end of line)
            let mut j = i;
            while j < n && chars[j] != ')' {
                j += 1;
            }
            if j < n {
                j += 1; // include ')'
            }
            push(&mut spans, chars[start..j].iter().collect(), Tag::Comment);
            i = j;
            continue;
        }
        if matches!(word.as_str(), "s\"" | ".\"" | "c\"" | "abort\"" | "s\\\"") {
            // string to the next "
            let mut j = i;
            while j < n && chars[j] != '"' {
                j += 1;
            }
            if j < n {
                j += 1;
            }
            push(&mut spans, chars[start..j].iter().collect(), Tag::Str);
            i = j;
            continue;
        }
        if word == ".(" {
            let mut j = i;
            while j < n && chars[j] != ')' {
                j += 1;
            }
            if j < n {
                j += 1;
            }
            push(&mut spans, chars[start..j].iter().collect(), Tag::Str);
            i = j;
            continue;
        }

        let tag = if name_next { Tag::Def } else { classify(&word) };
        name_next = word == ":";
        push(&mut spans, word, tag);
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(line: &str) -> Vec<(String, Tag)> {
        highlight(line).into_iter().filter(|(s, _)| !s.trim().is_empty()).collect()
    }

    #[test]
    fn colon_def_name_and_control() {
        let v = tags(": sq dup * ;");
        assert_eq!(v[0], (":".into(), Tag::Def));
        assert_eq!(v[1], ("sq".into(), Tag::Def)); // the name
        assert_eq!(v[2], ("dup".into(), Tag::Core));
        assert_eq!(v[3], ("*".into(), Tag::Core));
        assert_eq!(v[4], (";".into(), Tag::Def));
        let c = tags("0 if 1 else 2 then");
        assert!(c.iter().any(|(s, t)| s == "if" && *t == Tag::Control));
        assert!(c.iter().any(|(s, t)| s == "else" && *t == Tag::Control));
    }

    #[test]
    fn numbers_strings_comments() {
        assert!(tags("42 0xFF -7 3.14 1e3").iter().all(|(_, t)| *t == Tag::Number));
        assert_eq!(tags(r#"s" hi there""#)[0].1, Tag::Str);
        let c = highlight(r#"1 2 \ a comment"#);
        assert!(c.iter().any(|(s, t)| s.contains("comment") && *t == Tag::Comment));
        let p = highlight("( stack: a b -- c ) +");
        assert!(p.iter().any(|(s, t)| s.contains("stack") && *t == Tag::Comment));
        assert!(p.iter().any(|(s, t)| s == "+" && *t == Tag::Core));
    }
}
