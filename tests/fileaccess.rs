#![cfg(target_os = "macos")]
use mf66::Mf66Session;
#[test] fn roundtrip() {
    let mut s=Mf66Session::new().unwrap();
    let path = "/tmp/mf66_filetest.txt";
    let _ = std::fs::remove_file(path);
    let prog = format!(r#"
create buf 256 allot
s" {path}" r/w create-file drop constant fid
s" Hello, World!" fid write-file drop
fid close-file drop
s" {path}" r/o open-file drop constant fid2
buf 256 fid2 read-file drop constant n
fid2 close-file drop
n . buf c@ . buf 7 + c@ .
"#);
    assert_eq!(s.eval_out(&prog).unwrap(), "13 72 87 ");
    let _ = std::fs::remove_file(path);
}
