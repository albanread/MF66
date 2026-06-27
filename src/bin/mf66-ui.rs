//! `mf66-ui` — the native macOS Forth workspace.
//!
//! A window (the `macide` AppKit + Core-Text shell, the same iGui lineage NCL
//! uses) with a REPL/output log, a live data-stack view, and an input line —
//! driven by an `Mf66Session` on a worker thread. Mirrors how WF66's `wf64-ui`
//! drives the Windows iGui: the AppKit loop runs on the main thread, the worker
//! owns the session, drains input events from the mailbox, evaluates, and
//! presents a `DrawBatch` each tick.
//!
//!   cargo run --bin mf66-ui --features gui

use locus_ide_protocol::event::UiEvent;
use macide::{mailbox, window};
use mf66::workspace::{Reaction, Workspace};
use mf66::Mf66Session;

const W: f64 = 1000.0;
const H: f64 = 680.0;

fn main() -> Result<(), String> {
    window::run("MF66 Workspace", W, H, || {
        // The session (and its JIT + thread-local output capture) lives entirely
        // on this worker thread; it never crosses to the AppKit main thread.
        let mut session = match Mf66Session::new() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("mf66-ui: session boot failed: {e}");
                std::process::exit(1);
            }
        };
        let mut ws = Workspace::new(W as f32, H as f32);

        loop {
            for we in mailbox::drain() {
                if !matches!(we.event, UiEvent::Mouse(_)) {
                    match ws.on_event(&we.event) {
                        Reaction::Close => std::process::exit(0),
                        Reaction::Submit(line) => {
                            let result = session.eval_out(&line).map_err(|e| e.to_string());
                            ws.record(line, result, session.stack(), session.compiling());
                            if session.wants_bye() {
                                std::process::exit(0);
                            }
                        }
                        Reaction::None => {}
                    }
                }
            }
            window::present_main(ws.render());
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    })
}
