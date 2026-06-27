//! `mf66-tcl` — drive the MF66 workspace headlessly with a TCL script.
//!
//! The agentic control surface: a TCL script (file or inline) issues verbs that
//! manipulate and observe the IDE — `eval`/`type`/`key`, the read-back verbs
//! (`stack`/`depth`/`screen`/…), `assert`/`assert-eq`, and `screenshot PATH.png`
//! — all without a desktop. An `assert*` failure exits non-zero, so a script
//! doubles as a UI test or an agent's verification step.
//!
//!   cargo run --bin mf66-tcl --features ui -- script.tcl
//!   cargo run --bin mf66-tcl --features ui -- -e 'eval "2 3 +" ; puts [stack]'

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let src = match args.split_first() {
        Some((flag, rest)) if flag == "-e" || flag == "--exec" => rest.join(" "),
        Some((path, _)) => match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("mf66-tcl: cannot read {path}: {e}");
                std::process::exit(2);
            }
        },
        None => {
            eprintln!("usage: mf66-tcl <script.tcl> | -e \"<tcl>\"");
            std::process::exit(2);
        }
    };
    if let Err(e) = mf66::wsdriver::run_script(&src) {
        eprintln!("mf66-tcl: {e}");
        std::process::exit(1);
    }
}
