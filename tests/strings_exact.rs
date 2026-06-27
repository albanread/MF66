#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn exact_spacing() {
    assert_eq!(out(r#"s" hello   " -trailing nip ."#), "5 ");   // 3 trailing stripped
    assert_eq!(out(r#"s"     " -trailing nip ."#), "0 ");
    assert_eq!(out(r#"s"   a  b " -leading nip ."#), "5 ");      // "a  b " after strip
    assert_eq!(out(r#"s" exact   spaces" nip ."#), "14 ");       // interior preserved
    assert_eq!(out(r#"s" hi" type"#), "hi");
    assert_eq!(out(r#".( printed now)"#), "printed now");
}
#[test] fn escaped_strings() {
    assert_eq!(out(r#"s\" hello" nip ."#), "5 ");
    assert_eq!(out(r#"s\" a\nb" nip ."#), "3 ");                 // \n is one byte
    assert_eq!(out(r#"s\" a\nb" drop 1 + c@ ."#), "10 ");        // LF
    assert_eq!(out(r#"s\" a\tb" drop 1 + c@ ."#), "9 ");         // TAB
    assert_eq!(out(r#"s\" \\" drop c@ ."#), "92 ");              // backslash
    assert_eq!(out(r#"s\" \"" drop c@ ."#), "34 ");              // quote
    assert_eq!(out(r#"s\" \x41\x42" drop dup c@ swap 1+ c@ . ."#), "66 65 ");
    assert_eq!(out(r#"s\" \0" drop c@ ."#), "0 ");               // NUL
}
#[test] fn counted_string() {
    assert_eq!(out(r#"c" abc" c@ ."#), "3 ");                    // length byte
    assert_eq!(out(r#"c" abc" 1 + c@ ."#), "97 ");               // 'a'
}
