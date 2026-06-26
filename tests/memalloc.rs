#![cfg(target_os = "macos")]
use mf66::Mf66Session;
#[test] fn alloc_free_resize() {
    let mut s=Mf66Session::new().unwrap();
    assert_eq!(s.eval_out("100 allocate swap drop .").unwrap(), "0 ");
    // ANS-style: track the address in a variable across resize, then free
    let out = s.eval_out(
        "variable p  64 allocate drop p !  42 p @ c!  p @ c@ . \
         p @ 128 resize swap p ! .  p @ c@ .  p @ free ."
    ).unwrap();
    assert_eq!(out, "42 0 42 0 ", "got {out:?}"); // byte 42, resize ior 0, byte survives, free ior 0
}
