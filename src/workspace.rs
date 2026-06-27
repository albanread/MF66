//! The MF66 workspace — a headless state machine for the native IDE.
//!
//! It owns the interactive state (the REPL input line, the scrolling output log,
//! a command-history ring, and the latest data-stack snapshot) and turns it into
//! a `locus_ide_protocol::DrawBatch` to render. It consumes `UiEvent`s and, on
//! Enter, hands back the line to evaluate. It deliberately knows nothing about
//! AppKit *or* the Forth session, so it is unit-tested without a display: feed
//! events, feed eval results, assert on the produced `DrawBatch`.
//!
//! The `mf66-ui` binary wires this to `macide`'s window (events from the mailbox,
//! `present_main` of each frame) and an `Mf66Session` (the evaluator).

use locus_ide_protocol::draw::{Color, DrawBatch, DrawCmd, Rect};
use locus_ide_protocol::event::{KeyState, UiEvent};

// Win32 virtual-key codes the shell reports (macide maps macOS keycodes to these).
const VK_BACK: u32 = 0x08;
const VK_RETURN: u32 = 0x0D;
const VK_UP: u32 = 0x26;
const VK_DOWN: u32 = 0x28;

const STACK_W: f32 = 240.0; // right-hand stack pane width
const TITLE_H: f32 = 34.0;  // top title strip
const INPUT_H: f32 = 30.0;  // bottom input line
const LINE_H: f32 = 18.0;   // monospace line height
const FONT: f32 = 13.0;
const PAD: f32 = 10.0;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
}

// Dark theme.
fn c_bg() -> Color { rgb(24, 26, 33) }
fn c_pane() -> Color { rgb(31, 34, 43) }
fn c_strip() -> Color { rgb(38, 42, 53) }
fn c_text() -> Color { rgb(214, 218, 226) }
fn c_dim() -> Color { rgb(120, 126, 142) }
fn c_input() -> Color { rgb(168, 218, 255) }
fn c_ok() -> Color { rgb(148, 210, 189) }
fn c_err() -> Color { rgb(231, 111, 81) }
fn c_stack() -> Color { rgb(233, 196, 106) }
fn c_accent() -> Color { rgb(120, 170, 255) }

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Input,
    Output,
    Ok,
    Err,
    Note,
}

struct Line {
    text: String,
    kind: Kind,
}

/// What the host should do after `on_event`.
#[derive(Clone, Debug, PartialEq)]
pub enum Reaction {
    None,
    /// Enter was pressed — evaluate this line, then call [`Workspace::record`].
    Submit(String),
    /// The window was closed.
    Close,
}

pub struct Workspace {
    input: String,
    log: Vec<Line>,
    cmds: Vec<String>, // command history (for Up/Down recall)
    cmd_pos: usize,
    stack: Vec<i64>, // latest snapshot, top-first
    compiling: bool,
    width: f32,
    height: f32,
    frame: u64,
}

impl Workspace {
    pub fn new(width: f32, height: f32) -> Self {
        let mut w = Workspace {
            input: String::new(),
            log: Vec::new(),
            cmds: Vec::new(),
            cmd_pos: 0,
            stack: Vec::new(),
            compiling: false,
            width,
            height,
            frame: 0,
        };
        w.note("MF66 Workspace — optimizing Forth for Apple Silicon.");
        w.note("Type Forth and press Enter.  `bye` closes the window.");
        w
    }

    fn note(&mut self, s: &str) {
        self.log.push(Line { text: s.into(), kind: Kind::Note });
    }

    /// The current REPL input line (for read-back / agent inspection).
    pub fn input_text(&self) -> String {
        self.input.clone()
    }

    /// A plain-text snapshot of the visible screen — the output log plus the
    /// input line. Lets an agent observe the workspace without a display.
    pub fn screen_text(&self) -> String {
        let mut out = String::new();
        for l in &self.log {
            out.push_str(&l.text);
            out.push('\n');
        }
        let prompt = if self.compiling { "  … " } else { "ok> " };
        out.push_str(prompt);
        out.push_str(&self.input);
        out
    }

    /// Handle one UI event. Returns [`Reaction::Submit`] on Enter (the host then
    /// evaluates the line and calls [`record`](Self::record)).
    pub fn on_event(&mut self, ev: &UiEvent) -> Reaction {
        match ev {
            UiEvent::Close => Reaction::Close,
            UiEvent::Resize { width, height, .. } => {
                self.width = *width as f32;
                self.height = *height as f32;
                Reaction::None
            }
            UiEvent::Char { codepoint, .. } => {
                // Printable text only; control keys arrive as `Key` and would
                // otherwise double-insert.
                if let Some(c) = char::from_u32(*codepoint) {
                    if !c.is_control() {
                        self.input.push(c);
                    }
                }
                Reaction::None
            }
            UiEvent::Key { state: KeyState::Down, virtual_key, .. } => match *virtual_key {
                VK_RETURN => {
                    let line = std::mem::take(&mut self.input);
                    if !line.trim().is_empty() {
                        self.cmds.push(line.clone());
                    }
                    self.cmd_pos = self.cmds.len();
                    Reaction::Submit(line)
                }
                VK_BACK => {
                    self.input.pop();
                    Reaction::None
                }
                VK_UP => {
                    self.recall(-1);
                    Reaction::None
                }
                VK_DOWN => {
                    self.recall(1);
                    Reaction::None
                }
                _ => Reaction::None,
            },
            _ => Reaction::None,
        }
    }

    fn recall(&mut self, dir: i32) {
        if self.cmds.is_empty() {
            return;
        }
        let pos = self.cmd_pos as i32 + dir;
        if pos < 0 {
            self.cmd_pos = 0;
        } else if pos as usize >= self.cmds.len() {
            self.cmd_pos = self.cmds.len();
            self.input.clear();
            return;
        } else {
            self.cmd_pos = pos as usize;
        }
        self.input = self.cmds[self.cmd_pos].clone();
    }

    /// Record an evaluation: the line that was run, its result (Ok(output) or
    /// Err(message)), the resulting data stack (top-first), and whether a
    /// definition is still open.
    pub fn record(&mut self, line: String, result: Result<String, String>, stack: Vec<i64>, compiling: bool) {
        let prompt = if self.compiling { "  … " } else { "ok> " };
        self.log.push(Line { text: format!("{prompt}{line}"), kind: Kind::Input });
        match result {
            Ok(out) => {
                for l in out.split('\n') {
                    if !l.is_empty() {
                        self.log.push(Line { text: l.to_string(), kind: Kind::Output });
                    }
                }
                if !compiling {
                    self.log.push(Line { text: "ok".into(), kind: Kind::Ok });
                }
            }
            Err(e) => self.log.push(Line { text: format!("✗ {e}"), kind: Kind::Err }),
        }
        self.stack = stack;
        self.compiling = compiling;
        // Keep the log bounded.
        if self.log.len() > 2000 {
            self.log.drain(0..self.log.len() - 2000);
        }
    }

    /// Build the frame to present.
    pub fn render(&mut self) -> DrawBatch {
        self.frame += 1;
        let (w, h) = (self.width, self.height);
        let main_w = w - STACK_W;
        let mut cmds: Vec<DrawCmd> = Vec::with_capacity(64);
        let text = |s: &str, x: f32, y: f32, c: Color| DrawCmd::DrawText {
            text: s.to_string(),
            x,
            y,
            size: FONT,
            color: c,
        };
        let fill = |l: f32, t: f32, r: f32, b: f32, c: Color| DrawCmd::FillRect {
            rect: Rect { left: l, top: t, right: r, bottom: b },
            color: c,
        };

        cmds.push(DrawCmd::Clear(c_bg()));

        // ── title strip ──
        cmds.push(fill(0.0, 0.0, w, TITLE_H, c_strip()));
        cmds.push(text("MF66 Workspace", PAD, 22.0, c_accent()));
        cmds.push(text(
            &format!("depth {}", self.stack.len()),
            main_w - 90.0,
            22.0,
            c_dim(),
        ));

        // ── stack pane (right) ──
        cmds.push(fill(main_w, 0.0, w, h, c_pane()));
        cmds.push(text("data stack", main_w + PAD, 22.0, c_dim()));
        // top of stack at the top of the list
        let mut sy = TITLE_H + LINE_H;
        for (i, v) in self.stack.iter().enumerate() {
            if sy > h - LINE_H {
                cmds.push(text("…", main_w + PAD, sy, c_dim()));
                break;
            }
            let tag = if i == 0 { " ← top" } else { "" };
            cmds.push(text(&format!("{v}{tag}"), main_w + PAD, sy, c_stack()));
            sy += LINE_H;
        }

        // ── output log (left), tail that fits above the input line ──
        let log_top = TITLE_H + 4.0;
        let log_bot = h - INPUT_H - 4.0;
        let rows = ((log_bot - log_top) / LINE_H).floor().max(0.0) as usize;
        let start = self.log.len().saturating_sub(rows);
        let mut ly = log_bot - LINE_H + FONT * 0.8;
        for line in self.log[start..].iter().rev() {
            let c = match line.kind {
                Kind::Input => c_input(),
                Kind::Output => c_text(),
                Kind::Ok => c_ok(),
                Kind::Err => c_err(),
                Kind::Note => c_dim(),
            };
            cmds.push(text(&line.text, PAD, ly, c));
            ly -= LINE_H;
        }

        // ── input line (bottom) ──
        cmds.push(fill(0.0, h - INPUT_H, main_w, h, c_strip()));
        let prompt = if self.compiling { "  … " } else { "ok> " };
        cmds.push(text(
            &format!("{prompt}{}_", self.input),
            PAD,
            h - INPUT_H + 20.0,
            c_input(),
        ));

        DrawBatch { frame_id: self.frame, commands: cmds }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(c: char) -> UiEvent {
        UiEvent::Char { codepoint: c as u32, modifiers: 0 }
    }
    fn key(vk: u32) -> UiEvent {
        UiEvent::Key { state: KeyState::Down, virtual_key: vk, modifiers: 0 }
    }
    fn texts(b: &DrawBatch) -> Vec<String> {
        b.commands
            .iter()
            .filter_map(|c| match c {
                DrawCmd::DrawText { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn typing_and_submit() {
        let mut w = Workspace::new(900.0, 600.0);
        for c in "2 3 +".chars() {
            assert_eq!(w.on_event(&ch(c)), Reaction::None);
        }
        // input line shows what was typed
        assert!(texts(&w.render()).iter().any(|t| t.contains("2 3 +")));
        // Enter submits the line
        assert_eq!(w.on_event(&key(VK_RETURN)), Reaction::Submit("2 3 +".into()));
        // input cleared after submit
        assert!(!texts(&w.render()).iter().any(|t| t == "ok> 2 3 +_"));
    }

    #[test]
    fn backspace_edits_input() {
        let mut w = Workspace::new(900.0, 600.0);
        for c in "dup".chars() {
            w.on_event(&ch(c));
        }
        w.on_event(&key(VK_BACK));
        assert_eq!(w.on_event(&key(VK_RETURN)), Reaction::Submit("du".into()));
    }

    #[test]
    fn record_renders_output_stack_and_ok() {
        let mut w = Workspace::new(900.0, 600.0);
        w.record("2 3 + .".into(), Ok("5 ".into()), vec![], false);
        let t = texts(&w.render());
        assert!(t.iter().any(|s| s.contains("2 3 + .")), "input echoed");
        assert!(t.iter().any(|s| s == "5 "), "output shown");
        assert!(t.iter().any(|s| s == "ok"), "ok shown");
        // a stack snapshot renders, top-first with a marker
        w.record("7 11".into(), Ok(String::new()), vec![11, 7], false);
        let t = texts(&w.render());
        assert!(t.iter().any(|s| s == "11 ← top"), "TOS marked");
        assert!(t.iter().any(|s| s == "7"), "deeper item shown");
    }

    #[test]
    fn error_is_shown_and_compiling_withholds_ok() {
        let mut w = Workspace::new(900.0, 600.0);
        w.record("nope".into(), Err("undefined word: nope".into()), vec![], false);
        assert!(texts(&w.render()).iter().any(|s| s.starts_with("✗")));
        // mid-definition: no `ok`
        w.record(": foo".into(), Ok(String::new()), vec![], true);
        assert!(!texts(&w.render()).iter().any(|s| s == "ok"));
    }

    #[test]
    fn history_recall() {
        let mut w = Workspace::new(900.0, 600.0);
        w.on_event(&ch('a'));
        w.on_event(&key(VK_RETURN));
        w.on_event(&ch('b'));
        w.on_event(&key(VK_RETURN));
        w.on_event(&key(VK_UP)); // → "b"
        assert!(texts(&w.render()).iter().any(|t| t == "ok> b_"));
        w.on_event(&key(VK_UP)); // → "a"
        assert!(texts(&w.render()).iter().any(|t| t == "ok> a_"));
    }
}
