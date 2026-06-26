//! Dictionary stage 1–2: header layout + the 512-bucket FNV-1a hash overlay +
//! `find` (kernel/dict.masm). Exercises `(create)` → `find-name` round-trips
//! through the real hash/overlay/search-order path. `nt` is the name-token
//! address (→ counted-string length byte then chars).

#![cfg(target_os = "macos")]

use mf66::Mf66Session;

#[test]
fn create_then_find() {
    let mut s = Mf66Session::new().unwrap();
    s.create_word("hello").unwrap();
    assert!(s.find("hello").unwrap().is_some(), "created word should be found");
}

#[test]
fn unknown_word_not_found() {
    let mut s = Mf66Session::new().unwrap();
    s.create_word("foo").unwrap();
    assert_eq!(s.find("bar").unwrap(), None);
    // empty dictionary: nothing is found
    let mut s2 = Mf66Session::new().unwrap();
    assert_eq!(s2.find("anything").unwrap(), None);
}

#[test]
fn find_is_case_insensitive() {
    let mut s = Mf66Session::new().unwrap();
    s.create_word("Dup").unwrap();
    assert!(s.find("dup").unwrap().is_some());
    assert!(s.find("DUP").unwrap().is_some());
    assert!(s.find("dUp").unwrap().is_some());
}

#[test]
fn many_words_all_found_incl_collisions() {
    let mut s = Mf66Session::new().unwrap();
    // mix of names incl. symbols/digits so several land in the same bucket
    let words = [
        "alpha", "beta", "gamma", "swap", "over", "rot", "2dup", "+", "-", "*",
        "=", "<>", "cell+", "0<", "u<", "within", "negate", "lshift",
    ];
    for w in words {
        s.create_word(w).unwrap();
    }
    for w in words {
        assert!(s.find(w).unwrap().is_some(), "{w} should be found");
    }
    assert_eq!(s.find("missing").unwrap(), None);
}

#[test]
fn nt_points_at_counted_name() {
    let mut s = Mf66Session::new().unwrap();
    s.create_word("widget").unwrap();
    let nt = s.find("widget").unwrap().expect("found");
    let len = unsafe { (nt as *const u8).read() };
    assert_eq!(len, 6, "name length byte at nt");
    let name: Vec<u8> = (0..6).map(|i| unsafe { (nt as *const u8).add(1 + i).read() }).collect();
    assert_eq!(&name, b"widget", "name chars after the length byte");
}

#[test]
fn redefinition_finds_newest_first() {
    // Both headers exist; find walks most-recent-first within the bucket.
    let mut s = Mf66Session::new().unwrap();
    s.create_word("x").unwrap();
    let first = s.find("x").unwrap().unwrap();
    s.create_word("x").unwrap();
    let second = s.find("x").unwrap().unwrap();
    assert_ne!(first, second, "newest definition shadows the older one");
}

/// The interpreter's core path: publish primitives, then find → name>interpret →
/// execute them with data-stack args (no asm-symbol shortcut).
#[test]
fn publish_find_interpret_execute() {
    let mut s = Mf66Session::new().unwrap();
    s.publish("dup", "dup_", false).unwrap();
    s.publish("+", "plus", false).unwrap();
    s.publish("1+", "one_plus", false).unwrap();
    // nt -> xt resolves to the real primitive code address
    let nt = s.find("dup").unwrap().unwrap();
    assert!(nt != 0);
    // execute through the dictionary
    s.push(7);
    s.run_word("dup").unwrap(); // 7 -> 7 7
    assert_eq!(s.stack(), vec![7, 7]);
    s.run_word("+").unwrap(); // 7 7 -> 14
    assert_eq!(s.stack(), vec![14]);
    s.run_word("1+").unwrap(); // 14 -> 15
    assert_eq!(s.stack(), vec![15]);
}

/// After boot, every kernel primitive is findable + executable by its Forth name
/// (no manual publish, no asm-symbol shortcut) — the dictionary bootstrap works.
#[test]
fn bootstrap_makes_primitives_findable() {
    let mut s = Mf66Session::new().unwrap();
    for name in ["dup", "swap", "drop", "+", "-", "*", "=", "<", "negate", "2dup", "cell+", "and", "or", "@", "!"] {
        assert!(s.find(name).unwrap().is_some(), "{name} should be in the dictionary after boot");
    }
    s.push(6);
    s.push(7);
    s.run_word("*").unwrap(); // 6 7 * -> 42
    assert_eq!(s.stack(), vec![42]);
    s.run_word("negate").unwrap(); // -> -42
    assert_eq!(s.stack(), vec![-42]);
}
