#![cfg(target_os = "macos")]
use mf66::{Mf66Session, crash};
#[test]
fn catches_bad_fetch() {
    let mut s = Mf66Session::new().unwrap();
    // dereference address 1 → SIGSEGV, caught as Err(CrashInfo)
    let r = crash::guard(|| { let _ = s.eval("1 @"); });
    match r {
        Ok(_) => panic!("expected a crash"),
        Err(info) => {
            eprintln!("captured: {info}");
            assert_eq!(info.fault_addr, 1, "fault addr should be 1");
            assert_eq!(info.signal, libc::SIGSEGV);
        }
    }
}
#[test]
fn normal_runs_ok() {
    let mut s = Mf66Session::new().unwrap();
    let r = crash::guard(|| s.eval_out("2 3 + .").unwrap());
    assert_eq!(r.unwrap(), "5 ");
}
