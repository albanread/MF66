#![cfg(target_os = "macos")]
use mf66::Mf66Session;
// Pure stack motion should compile to almost nothing (free reindexing) and
// dup-fuse should be a single op; verify via correctness + tiny body size.
#[test]
fn motion_is_nearly_free() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": churn swap rot -rot swap drop nip ;").unwrap();
    // 5 motion words + drop/nip; correctness:
    assert_eq!(s.eval_out("1 2 3 churn .").unwrap(), "2 ");
    // body should be tiny — motion emits no code, only the settle of the result
    assert!(s.last_body_words() <= 12, "motion body {} words", s.last_body_words());
}
#[test]
fn dup_fuse_and_constfold() {
    let mut s = Mf66Session::new().unwrap();
    s.eval(": sq dup * ;").unwrap();
    assert_eq!(s.eval_out("7 sq .").unwrap(), "49 ");
    s.eval(": k 2 3 + 4 * ;").unwrap();      // const-folds to 20
    assert_eq!(s.eval_out("k .").unwrap(), "20 ");
    assert!(s.last_body_words() <= 12, "constfold body {} words", s.last_body_words());
}
