
#[test]
fn probe_subst_cases() {
    let mut s = sess();
    s.eval("here 256 allot constant dst2").unwrap();
    // %% -> %
    assert_eq!(s.eval_out("s\" 100%%\" dst2 256 substitute . type").unwrap(), "0 100%");
    // unknown name -> kept verbatim with surrounding %
    assert_eq!(s.eval_out("s\" a%no%b\" dst2 256 substitute . type").unwrap(), "0 a%no%b");
    // plain text no %
    assert_eq!(s.eval_out("s\" plain\" dst2 256 substitute . type").unwrap(), "0 plain");
}
