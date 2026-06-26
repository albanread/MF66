#![cfg(target_os = "macos")]
use mf66::Mf66Session;

#[test]
fn counter() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("class counter cell ivar: n :m init 0 to n ;m :m bump n 1+ to n ;m :m val n ;m end-class").unwrap();
    s.eval("counter new c").unwrap();
    s.eval("c -> init").unwrap();
    s.eval("c -> bump").unwrap();
    assert_eq!(s.eval_out("c -> val .").unwrap(), "1 ");
}

#[test]
fn polymorphism() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("class shape cell ivar: s :m area 0 ;m :m sets to s ;m end-class").unwrap();
    s.eval("shape subclass sq :m area s s * ;m end-class").unwrap();
    s.eval("sq new q 5 q -> sets").unwrap();
    assert_eq!(s.eval_out("q -> area .").unwrap(), "25 ");
}

#[test]
fn dnu() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("class thing :m greet 42 ;m end-class").unwrap();
    s.eval("thing new x").unwrap();
    assert_eq!(s.eval_out("x -> greet .").unwrap(), "42 ");
    s.eval(": try-frob x -> frob ;").unwrap();
    assert_eq!(s.eval_out("' try-frob catch .").unwrap(), "-2058 ");
}

#[test]
fn super_test() {
    let mut s = Mf66Session::new().unwrap();
    s.eval("class animal cell ivar: legs :m legs! to legs ;m :m speak 7 ;m :m noise legs ;m :m describe self -> speak ;m end-class").unwrap();
    s.eval("animal subclass dog :m speak 9 ;m :m noise super -> noise 100 + ;m end-class").unwrap();
    s.eval("dog new rex 4 rex -> legs!").unwrap();
    assert_eq!(s.eval_out("rex -> speak .").unwrap(), "9 ");
    assert_eq!(s.eval_out("rex -> describe .").unwrap(), "9 ");
    assert_eq!(s.eval_out("rex -> noise .").unwrap(), "104 ");
}

fn animal_dog() -> Mf66Session {
    let mut s = Mf66Session::new().unwrap();
    s.eval("class animal cell ivar: legs :m legs! to legs ;m :m speak 7 ;m :m noise legs ;m :m describe self -> speak ;m end-class").unwrap();
    s.eval("animal subclass dog :m speak 9 ;m :m noise super -> noise 100 + ;m end-class").unwrap();
    s.eval("dog new rex 4 rex -> legs!").unwrap();
    s
}

#[test]
fn super_speak() {
    let mut s = animal_dog();
    assert_eq!(s.eval_out("rex -> speak .").unwrap(), "9 ");
}

#[test]
fn super_describe() {
    let mut s = animal_dog();
    assert_eq!(s.eval_out("rex -> describe .").unwrap(), "9 ");
}

#[test]
fn super_noise() {
    let mut s = animal_dog();
    assert_eq!(s.eval_out("rex -> noise .").unwrap(), "104 ");
}
