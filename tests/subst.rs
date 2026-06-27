#![cfg(target_os = "macos")]
use mf66::Mf66Session;
fn out(s:&str)->String{ Mf66Session::new().unwrap().eval_out(s).unwrap() }
#[test] fn substitute_facility() {
    // boot must succeed (substitute defined)
    assert_eq!(out("subst-init s\" hi\" pad 64 substitute >r drop r> ."), "0 ");  // count 0
    // bind and expand
    let prog = r#"subst-init
        s" Alice" s" who" replaces
        s" Hello, %who%!" pad 64 substitute
        >r 2drop r> ."#;
    assert_eq!(out(prog), "1 ");   // 1 substitution
    // verify produced text length: "Hello, Alice!" = 13
    assert_eq!(out(r#"subst-init s" Alice" s" who" replaces s" Hello, %who%!" pad 64 substitute drop nip ."#), "13 ");
    // %% → literal %
    assert_eq!(out(r#"subst-init s" 50%% off" pad 64 substitute drop nip ."#), "7 ");  // "50% off"
}
