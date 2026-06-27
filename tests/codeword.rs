#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn code_word_basic() {
    assert_eq!(out("CODE my+ \n ldr x9, [DSP] \n add TOS, TOS, x9 \n stk(2,1) \n next() \n END-CODE\n 3 4 my+ ."), "7 ");
}
#[test] fn code_word_loop_and_compose() {
    // hand-written loop (local labels → no reloc): sum 1..n
    let def = "CODE sumto \n mov x9, #0 \n mov x10, #1 \n .lp: \n cmp x10, TOS \n b.gt .dn \n add x9, x9, x10 \n add x10, x10, #1 \n b .lp \n .dn: \n mov TOS, x9 \n stk(1,1) \n next() \n END-CODE\n";
    assert_eq!(out(&format!("{def} 5 sumto .")), "15 ");
    // a CODE word used inside a normal colon definition
    assert_eq!(out(&format!("{def} : t 10 sumto ; t .")), "55 ");
}
#[test] fn code_word_rejects_external_call() {
    let mut s = Mf66Session::new().unwrap();
    let r = s.eval("CODE bad \n bl rt_emit \n next() \n END-CODE");
    assert!(r.is_err(), "a CODE word calling an extern must be rejected");
}
