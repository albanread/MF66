//! Regression: `sym@PAGE` / `sym@PAGEOFF` (the `adrp`+`add` symbol-address idiom)
//! survives the front-end and is resolved by `MacJit` to the real address.
//! This is what lets MF66 take a kernel symbol's address in assembly (JASM
//! front-end fix; the a64 encoder + loader already supported the relocs).

#![cfg(target_os = "macos")]

use mf66::Mf66Jit;

#[test]
fn adrp_add_resolves_symbol_address() {
    let mut jit = Mf66Jit::new();
    jit.add_asm(
        "\
.globl addr_of_target
addr_of_target:
adrp x0, target_fn@PAGE
add  x0, x0, target_fn@PAGEOFF
ret

.globl target_fn
target_fn:
mov x0, #99
ret
",
    )
    .unwrap();
    let get_addr: extern "C" fn() -> u64 = unsafe { jit.lookup_fn("addr_of_target").unwrap() };
    let target: u64 = jit.lookup_addr("target_fn").unwrap();
    assert_eq!(get_addr(), target, "adrp+add must yield target_fn's real address");
    // and the address is actually callable
    let f: extern "C" fn() -> u64 = unsafe { std::mem::transmute(get_addr()) };
    assert_eq!(f(), 99);
}
