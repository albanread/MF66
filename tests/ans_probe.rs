#![cfg(target_os = "macos")]
use mf66::Mf66Session;
use std::fs;

#[test]
fn probe_tester() {
    let mut s = Mf66Session::new().unwrap();
    let tester = fs::read_to_string("/Users/oberon/claudeprojects/WF66/lib/tester.fs").unwrap();
    for (i, line) in tester.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('\\') { continue; }
        if let Err(e) = s.eval(line) {
            println!("LINE {}: {l:?}\n   ERR: {e}", i + 1);
            return;
        }
    }
    println!("tester.fs loaded OK");
}
