//! The TCL agentic control layer for the MF66 workspace.
//!
//! Embeds the `rust-tcl` interpreter and registers **verbs** that drive the IDE
//! headlessly — the same pattern the MRASM studio and Locus IDEs use, so a script
//! (or an agent emitting TCL) can manipulate and observe the workspace without a
//! desktop. Input verbs (`eval`, `type`, `key`), read-back verbs (`stack`,
//! `depth`, `input`, `compiling`, `output`, `screen`), `assert`/`assert-eq` (so a
//! script doubles as a UI test), and `screenshot` (rasterise the current frame to
//! a PNG via macide's headless `CgCanvas`). The driver state lives in a
//! thread-local, reached by the verb closures via [`with_driver`] — exactly how
//! studio threads its app state.

use std::cell::RefCell;

use locus_ide_protocol::event::{KeyState, UiEvent};
use macide::render::CgCanvas;
use rust_tcl::error::Error as TclError;
use rust_tcl::{Arity, Registry, Value};

use crate::workspace::{Focus, Reaction, Workspace};
use crate::Mf66Session;

const W: usize = 1000;
const H: usize = 680;

struct Driver {
    session: Mf66Session,
    ws: Workspace,
    last: String, // last eval's output (the `output` verb)
}

thread_local! {
    static DRIVER: RefCell<Option<Driver>> = const { RefCell::new(None) };
}

fn with_driver<R>(f: impl FnOnce(&mut Driver) -> R) -> R {
    DRIVER.with(|d| f(d.borrow_mut().as_mut().expect("mf66 tcl driver not initialised")))
}

/// Evaluate one Forth line through the session, mirror it into the workspace log,
/// and remember the output. Returns the captured output.
fn do_eval(d: &mut Driver, line: &str) -> String {
    let result = d.session.eval_out(line).map_err(|e| e.to_string());
    let out = match &result {
        Ok(o) => o.clone(),
        Err(e) => format!("\u{2717} {e}"),
    };
    d.ws
        .record(line.to_string(), result, d.session.stack(), d.session.compiling());
    d.last = out.clone();
    out
}

/// Apply a workspace reaction (the host loop's equivalent): evaluate a submitted
/// REPL line or editor buffer, or save the editor.
fn react(d: &mut Driver, r: Reaction) {
    match r {
        Reaction::Submit(line) | Reaction::EvalBuffer(line) => {
            do_eval(d, &line);
        }
        Reaction::Save => {
            let _ = d.ws.editor.save();
        }
        Reaction::None | Reaction::Close => {}
    }
}

/// The data stack as a space-separated string, bottom → top (natural reading
/// order: `2 3` after `2 3`, `5` after `2 3 +`).
fn stack_str(d: &Driver) -> String {
    let mut s = d.session.stack(); // top-first
    s.reverse();
    s.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ")
}

fn char_ev(c: char) -> UiEvent {
    UiEvent::Char { codepoint: c as u32, modifiers: 0 }
}
fn key_ev(vk: u32, mods: u32) -> UiEvent {
    UiEvent::Key { state: KeyState::Down, virtual_key: vk, modifiers: mods }
}

/// Map a key name (studio convention, with `Shift+`/`Ctrl+`/`Alt+`/`Cmd+`
/// prefixes) to a `(virtual_key, modifiers)` pair.
fn map_key(name: &str) -> Option<(u32, u32)> {
    let mut mods = 0u32;
    let mut s = name;
    loop {
        if let Some(r) = s.strip_prefix("Shift+") {
            mods |= 1;
            s = r;
        } else if let Some(r) = s.strip_prefix("Ctrl+").or_else(|| s.strip_prefix("Control+")) {
            mods |= 2;
            s = r;
        } else if let Some(r) = s.strip_prefix("Alt+") {
            mods |= 4;
            s = r;
        } else if let Some(r) = s.strip_prefix("Cmd+").or_else(|| s.strip_prefix("Command+")) {
            mods |= 8;
            s = r;
        } else {
            break;
        }
    }
    let vk = match s {
        "Enter" | "Return" => 0x0D,
        "Backspace" => 0x08,
        "Tab" => 0x09,
        "Escape" => 0x1B,
        "Left" => 0x25,
        "Up" => 0x26,
        "Right" => 0x27,
        "Down" => 0x28,
        "Home" => 0x24,
        "End" => 0x23,
        "Delete" => 0x2E,
        s if s.chars().count() == 1 => s.chars().next().unwrap().to_ascii_uppercase() as u32,
        _ => return None,
    };
    Some((vk, mods))
}

/// Render the current workspace frame to a PNG (headless — Core Graphics, no
/// AppKit), so an agent can *see* the IDE state.
fn screenshot(d: &mut Driver, path: &str) -> Result<(), String> {
    let batch = d.ws.render();
    let mut canvas = CgCanvas::new(W, H);
    canvas.execute(&batch.commands);
    // to_ppm() is P6 (ASCII header + RGB bytes); strip the header, re-encode RGB.
    let ppm = canvas.to_ppm();
    let mut i = 0;
    let mut newlines = 0;
    while i < ppm.len() && newlines < 3 {
        if ppm[i] == b'\n' {
            newlines += 1;
        }
        i += 1;
    }
    let rgb = &ppm[i..];
    let (pw, ph) = (canvas.width() as u32, canvas.height() as u32);
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), pw, ph);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(rgb))
        .map_err(|e| e.to_string())
}

fn registry() -> Registry {
    let mut r = Registry::with_core();

    // ── input ──
    r.register("eval", Arity::exact(1), |_, a| {
        let line = a[0].as_str().to_string();
        Ok(Value::new(with_driver(|d| do_eval(d, &line))))
    });
    r.register("type", Arity::exact(1), |_, a| {
        let text = a[0].as_str().to_string();
        with_driver(|d| {
            for c in text.chars() {
                d.ws.on_event(&char_ev(c));
            }
        });
        Ok(Value::new(""))
    });
    r.register("key", Arity::exact(1), |_, a| {
        let (vk, mods) = map_key(a[0].as_str())
            .ok_or_else(|| TclError::runtime(format!("unknown key: {}", a[0].as_str())))?;
        with_driver(|d| {
            let reaction = d.ws.on_event(&key_ev(vk, mods));
            react(d, reaction);
        });
        Ok(Value::new(""))
    });

    // ── editor / files ──
    r.register("focus", Arity::exact(1), |_, a| {
        let f = match a[0].as_str() {
            "editor" => Focus::Editor,
            "repl" => Focus::Repl,
            o => return Err(TclError::runtime(format!("focus: editor|repl, not {o}"))),
        };
        with_driver(|d| d.ws.focus = f);
        Ok(Value::new(""))
    });
    r.register("new", Arity::exact(0), |_, _| {
        with_driver(|d| d.ws.editor.new_file());
        Ok(Value::new(""))
    });
    r.register("open", Arity::exact(1), |_, a| {
        let p = a[0].as_str().to_string();
        with_driver(|d| d.ws.editor.load(&p)).map_err(|e| TclError::runtime(format!("open: {e}")))?;
        Ok(Value::new(p))
    });
    r.register("save", Arity::exact(0), |_, _| {
        with_driver(|d| d.ws.editor.save()).map_err(|e| TclError::runtime(format!("save: {e}")))?;
        Ok(Value::new(""))
    });
    r.register("save-as", Arity::exact(1), |_, a| {
        let p = a[0].as_str().to_string();
        with_driver(|d| d.ws.editor.save_as(&p))
            .map_err(|e| TclError::runtime(format!("save-as: {e}")))?;
        Ok(Value::new(p))
    });
    r.register("format", Arity::exact(0), |_, _| {
        with_driver(|d| d.ws.editor.format());
        Ok(Value::new(""))
    });
    r.register("editor-set", Arity::exact(1), |_, a| {
        let t = a[0].as_str().to_string();
        with_driver(|d| d.ws.editor.set_text(&t));
        Ok(Value::new(""))
    });
    r.register("editor-text", Arity::exact(0), |_, _| {
        Ok(Value::new(with_driver(|d| d.ws.editor.text())))
    });
    r.register("eval-buffer", Arity::exact(0), |_, _| {
        let buf = with_driver(|d| d.ws.editor.text());
        Ok(Value::new(with_driver(|d| do_eval(d, &buf))))
    });
    // selection / clipboard / undo
    r.register("editor-select-all", Arity::exact(0), |_, _| {
        with_driver(|d| d.ws.editor.select_all());
        Ok(Value::new(""))
    });
    r.register("editor-selection", Arity::exact(0), |_, _| {
        Ok(Value::new(with_driver(|d| d.ws.editor.selected_text().unwrap_or_default())))
    });
    r.register("editor-copy", Arity::exact(0), |_, _| {
        with_driver(|d| d.ws.editor.copy());
        Ok(Value::new(""))
    });
    r.register("editor-cut", Arity::exact(0), |_, _| {
        with_driver(|d| d.ws.editor.cut());
        Ok(Value::new(""))
    });
    r.register("editor-paste", Arity::exact(0), |_, _| {
        with_driver(|d| d.ws.editor.paste());
        Ok(Value::new(""))
    });
    r.register("editor-clipboard", Arity::exact(0), |_, _| {
        Ok(Value::new(with_driver(|d| d.ws.editor.clipboard().to_string())))
    });
    r.register("editor-undo", Arity::exact(0), |_, _| {
        Ok(Value::new(with_driver(|d| if d.ws.editor.undo() { "1" } else { "0" }.to_string())))
    });
    r.register("editor-redo", Arity::exact(0), |_, _| {
        Ok(Value::new(with_driver(|d| if d.ws.editor.redo() { "1" } else { "0" }.to_string())))
    });

    // ── read-back (for assertions / agent observation) ──
    r.register("stack", Arity::exact(0), |_, _| {
        Ok(Value::new(with_driver(|d| stack_str(d))))
    });
    r.register("depth", Arity::exact(0), |_, _| {
        Ok(Value::new(with_driver(|d| d.session.depth().to_string())))
    });
    r.register("output", Arity::exact(0), |_, _| Ok(Value::new(with_driver(|d| d.last.clone()))));
    r.register("input", Arity::exact(0), |_, _| {
        Ok(Value::new(with_driver(|d| d.ws.input_text())))
    });
    r.register("compiling", Arity::exact(0), |_, _| {
        Ok(Value::new(with_driver(|d| if d.session.compiling() { "1" } else { "0" }.to_string())))
    });
    r.register("screen", Arity::exact(0), |_, _| {
        Ok(Value::new(with_driver(|d| d.ws.screen_text())))
    });

    // ── snapshot ──
    r.register("screenshot", Arity::exact(1), |_, a| {
        let path = a[0].as_str().to_string();
        with_driver(|d| screenshot(d, &path))
            .map_err(|e| TclError::runtime(format!("screenshot: {e}")))?;
        Ok(Value::new(path))
    });

    // ── assertions (a script doubles as a UI test) ──
    r.register("assert", Arity::range(1, 2), |_, a| {
        let c = a[0].as_str();
        let truthy = !(c.is_empty() || c == "0" || c == "false");
        if !truthy {
            let msg = a.get(1).map(|m| m.as_str()).unwrap_or("");
            return Err(TclError::runtime(format!("assert failed: {msg}")));
        }
        Ok(Value::new(""))
    });
    r.register("assert-eq", Arity::range(2, 3), |_, a| {
        if a[0].as_str() != a[1].as_str() {
            let msg = a.get(2).map(|m| m.as_str()).unwrap_or("");
            return Err(TclError::runtime(format!(
                "assert-eq failed: got `{}` want `{}` {msg}",
                a[0].as_str(),
                a[1].as_str()
            )));
        }
        Ok(Value::new(""))
    });

    r
}

/// Run a TCL script against a fresh workspace + session (headless). The script's
/// verbs drive and observe the IDE; any `assert*` failure aborts with an error.
pub fn run_script(tcl: &str) -> anyhow::Result<()> {
    let session = Mf66Session::new()?;
    let ws = Workspace::new(W as f32, H as f32);
    DRIVER.with(|d| *d.borrow_mut() = Some(Driver { session, ws, last: String::new() }));
    let reg = registry();
    let result = rust_tcl::eval(tcl, &reg).map_err(|e| anyhow::anyhow!("tcl: {e}"));
    DRIVER.with(|d| *d.borrow_mut() = None);
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::run_script;

    #[test]
    fn agent_drives_and_asserts() {
        // Drive the IDE entirely through TCL verbs, with assertions.
        run_script(
            r#"
            eval "2 3 +"
            assert-eq [stack] "5" "2 3 + leaves 5"
            eval "dup *"
            assert-eq [stack] "25" "square"
            eval ": sq dup * ;"
            assert-eq [compiling] "0" "definition closed"
            eval "6 sq"
            assert-eq [stack] "25 36" "sq(6)=36 on top"
            type "9 9 +"
            key Enter
            assert-eq [stack] "25 36 18" "typed line evaluated on Enter"
        "#,
        )
        .expect("tcl script with assertions should pass");
    }

    #[test]
    fn failing_assert_is_an_error() {
        assert!(run_script("eval \"2 2 +\"\nassert-eq [stack] \"5\"").is_err());
    }
}
