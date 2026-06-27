//! The MF66 workspace — a headless state machine for the native IDE.
//!
//! Owns an editor pane (rope-backed, syntax-coloured), a REPL (input line +
//! scrolling output log), a live data-stack view, and the focus between editor
//! and REPL. It turns `UiEvent`s into a `locus_ide_protocol::DrawBatch` to render
//! and, on Enter / ⌘-Enter, hands back what to evaluate. It knows nothing about
//! AppKit or the Forth session, so it is unit-tested without a display.

use locus_ide_protocol::draw::{Color, DrawBatch, DrawCmd, Rect};
use locus_ide_protocol::event::{KeyState, UiEvent};

use crate::editor::Editor;
use crate::fsyntax::{highlight, Tag};

// Win32 virtual-key codes the shell reports.
const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_DELETE: u32 = 0x2E;
const MOD_SHIFT: u32 = 1;
const MOD_COMMAND: u32 = 8;

const STACK_W: f32 = 230.0;
const TITLE_H: f32 = 32.0;
const INPUT_H: f32 = 28.0;
const LINE_H: f32 = 18.0;
const FONT: f32 = 13.0;
const CHAR_W: f32 = 7.8; // Menlo 13pt monospace advance
const GUTTER: f32 = 46.0;
const PAD: f32 = 10.0;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
}

// theme
fn c_bg() -> Color { rgb(24, 26, 33) }
fn c_editor() -> Color { rgb(28, 31, 39) }
fn c_pane() -> Color { rgb(31, 34, 43) }
fn c_strip() -> Color { rgb(38, 42, 53) }
fn c_text() -> Color { rgb(212, 212, 212) }
fn c_dim() -> Color { rgb(110, 116, 132) }
fn c_gutter() -> Color { rgb(80, 86, 100) }
fn c_input() -> Color { rgb(168, 218, 255) }
fn c_ok() -> Color { rgb(148, 210, 189) }
fn c_err() -> Color { rgb(231, 111, 81) }
fn c_stack() -> Color { rgb(233, 196, 106) }
fn c_accent() -> Color { rgb(120, 170, 255) }
fn c_focus() -> Color { rgb(86, 156, 214) }
fn c_caret() -> Color { rgb(220, 230, 255) }
fn c_sel() -> Color { rgb(48, 66, 104) }

// syntax tag → colour
fn tag_color(t: Tag) -> Color {
    match t {
        Tag::Normal => c_text(),
        Tag::Comment => rgb(106, 153, 85),
        Tag::Str => rgb(206, 145, 120),
        Tag::Def => rgb(197, 134, 192),
        Tag::Control => rgb(86, 156, 214),
        Tag::Number => rgb(181, 206, 168),
        Tag::Core => rgb(156, 220, 254),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Editor,
    Repl,
}

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
    /// REPL Enter — evaluate this line, then call [`Workspace::record`].
    Submit(String),
    /// Editor ⌘-Enter — evaluate the whole editor buffer.
    EvalBuffer(String),
    /// ⌘-S — save the editor to its current path.
    Save,
    Close,
}

pub struct Workspace {
    pub editor: Editor,
    pub focus: Focus,
    input: String,
    log: Vec<Line>,
    cmds: Vec<String>,
    cmd_pos: usize,
    stack: Vec<i64>,
    compiling: bool,
    width: f32,
    height: f32,
    frame: u64,
}

impl Workspace {
    pub fn new(width: f32, height: f32) -> Self {
        let mut w = Workspace {
            editor: Editor::new(),
            focus: Focus::Repl,
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
        w.note("MF66 Workspace — Tab switches editor⇄REPL.  ⌘⏎ runs the buffer.");
        w.editor.set_text("\\ Edit Forth here, then ⌘⏎ to run.\n: sq dup * ;\n: quad sq sq ;\n");
        w
    }

    fn note(&mut self, s: &str) {
        self.log.push(Line { text: s.into(), kind: Kind::Note });
    }

    pub fn input_text(&self) -> String {
        self.input.clone()
    }

    /// A plain-text snapshot of the visible state — for agent observation.
    pub fn screen_text(&self) -> String {
        let mut out = String::new();
        out.push_str("── editor ──\n");
        out.push_str(&self.editor.text());
        out.push_str("\n── repl ──\n");
        for l in &self.log {
            out.push_str(&l.text);
            out.push('\n');
        }
        let prompt = if self.compiling { "  … " } else { "ok> " };
        out.push_str(prompt);
        out.push_str(&self.input);
        out
    }

    pub fn on_event(&mut self, ev: &UiEvent) -> Reaction {
        match ev {
            UiEvent::Close => return Reaction::Close,
            UiEvent::Resize { width, height, .. } => {
                self.width = *width as f32;
                self.height = *height as f32;
                return Reaction::None;
            }
            // Tab toggles focus (no modifier).
            UiEvent::Key { state: KeyState::Down, virtual_key: VK_TAB, modifiers: 0 } => {
                self.focus = match self.focus {
                    Focus::Editor => Focus::Repl,
                    Focus::Repl => Focus::Editor,
                };
                return Reaction::None;
            }
            _ => {}
        }
        match self.focus {
            Focus::Repl => self.repl_event(ev),
            Focus::Editor => self.editor_event(ev),
        }
    }

    fn repl_event(&mut self, ev: &UiEvent) -> Reaction {
        match ev {
            UiEvent::Char { codepoint, .. } => {
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

    fn editor_event(&mut self, ev: &UiEvent) -> Reaction {
        match ev {
            UiEvent::Char { codepoint, modifiers } => {
                if *modifiers & MOD_COMMAND == 0 {
                    if let Some(c) = char::from_u32(*codepoint) {
                        if !c.is_control() {
                            self.editor.insert_char(c);
                        }
                    }
                }
                Reaction::None
            }
            UiEvent::Key { state: KeyState::Down, virtual_key, modifiers } => {
                let cmd = *modifiers & MOD_COMMAND != 0;
                let shift = *modifiers & MOD_SHIFT != 0;
                if cmd {
                    return match *virtual_key {
                        VK_RETURN => Reaction::EvalBuffer(self.editor.text()),
                        0x53 => Reaction::Save,                    // ⌘S save
                        0x46 if shift => { self.editor.format(); Reaction::None } // ⌘⇧F format
                        0x41 => { self.editor.select_all(); Reaction::None }      // ⌘A
                        0x43 => { self.editor.copy(); Reaction::None }            // ⌘C
                        0x58 => { self.editor.cut(); Reaction::None }             // ⌘X
                        0x56 => { self.editor.paste(); Reaction::None }           // ⌘V
                        0x5A if shift => { self.editor.redo(); Reaction::None }   // ⌘⇧Z
                        0x5A => { self.editor.undo(); Reaction::None }            // ⌘Z
                        _ => Reaction::None,
                    };
                }
                match *virtual_key {
                    VK_RETURN => self.editor.newline(),
                    VK_BACK => self.editor.backspace(),
                    VK_DELETE => self.editor.delete_forward(),
                    VK_LEFT => self.editor.move_left(shift),
                    VK_RIGHT => self.editor.move_right(shift),
                    VK_UP => self.editor.move_up(shift),
                    VK_DOWN => self.editor.move_down(shift),
                    VK_HOME => self.editor.home(shift),
                    VK_END => self.editor.end(shift),
                    _ => {}
                }
                Reaction::None
            }
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

    /// Record an evaluation (REPL line or editor buffer) into the output log +
    /// stack view.
    pub fn record(&mut self, line: String, result: Result<String, String>, stack: Vec<i64>, compiling: bool) {
        let prompt = if self.compiling { "  … " } else { "ok> " };
        for (i, l) in line.split('\n').enumerate() {
            let p = if i == 0 { prompt } else { "  … " };
            self.log.push(Line { text: format!("{p}{l}"), kind: Kind::Input });
        }
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
            Err(e) => self.log.push(Line { text: format!("\u{2717} {e}"), kind: Kind::Err }),
        }
        self.stack = stack;
        self.compiling = compiling;
        if self.log.len() > 2000 {
            self.log.drain(0..self.log.len() - 2000);
        }
    }

    pub fn render(&mut self) -> DrawBatch {
        self.frame += 1;
        let (w, h) = (self.width, self.height);
        let main_w = w - STACK_W;
        let editor_bot = TITLE_H + (h - TITLE_H) * 0.56;
        let mut cmds: Vec<DrawCmd> = Vec::with_capacity(256);
        let text = |s: &str, x: f32, y: f32, c: Color| DrawCmd::DrawText { text: s.to_string(), x, y, size: FONT, color: c };
        let fill = |l: f32, t: f32, r: f32, b: f32, c: Color| DrawCmd::FillRect { rect: Rect { left: l, top: t, right: r, bottom: b }, color: c };

        cmds.push(DrawCmd::Clear(c_bg()));

        // ── title strip ──
        cmds.push(fill(0.0, 0.0, w, TITLE_H, c_strip()));
        cmds.push(text("MF66", PAD, 21.0, c_accent()));
        cmds.push(text(&self.editor.file_label(), PAD + 46.0, 21.0, c_dim()));
        cmds.push(text(&format!("depth {}", self.stack.len()), main_w - 86.0, 21.0, c_dim()));

        // ── editor pane (upper-left) ──
        let ed_focus = self.focus == Focus::Editor;
        cmds.push(fill(0.0, TITLE_H, main_w, editor_bot, c_editor()));
        let rows = (((editor_bot - TITLE_H) / LINE_H).floor() as usize).max(1);
        self.editor.ensure_visible(rows);
        let (crow, ccol) = self.editor.cursor();
        let top = self.editor.top;
        let sel = self
            .editor
            .selection()
            .map(|(s, e)| (self.editor.offset_rowcol(s), self.editor.offset_rowcol(e)));
        for vis in 0..rows {
            let row = top + vis;
            if row >= self.editor.line_count() {
                break;
            }
            let y = TITLE_H + vis as f32 * LINE_H + FONT;
            let row_top = TITLE_H + vis as f32 * LINE_H;
            // selection background for this row
            if let Some(((sr, sc), (er, ec))) = sel {
                if row >= sr && row <= er {
                    let from = if row == sr { sc } else { 0 };
                    let to = if row == er {
                        ec
                    } else {
                        self.editor.line(row).chars().count() + 1 // include the newline
                    };
                    let x0 = GUTTER + from as f32 * CHAR_W;
                    let x1 = (GUTTER + to as f32 * CHAR_W).min(main_w);
                    if x1 > x0 {
                        cmds.push(fill(x0, row_top, x1, row_top + LINE_H, c_sel()));
                    }
                }
            }
            cmds.push(text(&format!("{:>3}", row + 1), 6.0, y, c_gutter()));
            let mut x = GUTTER;
            for (frag, tag) in highlight(&self.editor.line(row)) {
                if !frag.is_empty() && x < main_w {
                    cmds.push(text(&frag, x, y, tag_color(tag)));
                }
                x += frag.chars().count() as f32 * CHAR_W;
            }
            if row == crow {
                let cx = GUTTER + ccol as f32 * CHAR_W;
                let caret = if ed_focus { c_caret() } else { c_dim() };
                cmds.push(fill(cx, TITLE_H + vis as f32 * LINE_H + 2.0, cx + 1.5, TITLE_H + vis as f32 * LINE_H + LINE_H, caret));
            }
        }

        // ── REPL pane (lower-left) ──
        cmds.push(fill(0.0, editor_bot, main_w, h, c_bg()));
        cmds.push(fill(0.0, editor_bot, main_w, editor_bot + 1.0, c_dim())); // divider
        let log_top = editor_bot + 4.0;
        let log_bot = h - INPUT_H - 2.0;
        let lrows = ((log_bot - log_top) / LINE_H).floor().max(0.0) as usize;
        let start = self.log.len().saturating_sub(lrows);
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
        // input line
        cmds.push(fill(0.0, h - INPUT_H, main_w, h, c_strip()));
        let prompt = if self.compiling { "  … " } else { "ok> " };
        let repl_focus = self.focus == Focus::Repl;
        let cur = if repl_focus { "_" } else { "" };
        cmds.push(text(&format!("{prompt}{}{cur}", self.input), PAD, h - INPUT_H + 19.0, c_input()));

        // ── stack pane (right) ──
        cmds.push(fill(main_w, TITLE_H, w, h, c_pane()));
        cmds.push(text("data stack", main_w + PAD, TITLE_H + 20.0, c_dim()));
        let mut sy = TITLE_H + 20.0 + LINE_H;
        for (i, v) in self.stack.iter().enumerate() {
            if sy > h - LINE_H {
                cmds.push(text("…", main_w + PAD, sy, c_dim()));
                break;
            }
            let tag = if i == 0 { " ← top" } else { "" };
            cmds.push(text(&format!("{v}{tag}"), main_w + PAD, sy, c_stack()));
            sy += LINE_H;
        }

        // ── focus outline ──
        let outline = if ed_focus { Rect { left: 0.0, top: TITLE_H, right: main_w, bottom: editor_bot } } else { Rect { left: 0.0, top: h - INPUT_H, right: main_w, bottom: h } };
        cmds.push(DrawCmd::StrokeRect { rect: outline, color: c_focus(), thickness: 1.5 });

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
    fn keymod(vk: u32, m: u32) -> UiEvent {
        UiEvent::Key { state: KeyState::Down, virtual_key: vk, modifiers: m }
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
    fn repl_typing_and_submit() {
        let mut w = Workspace::new(900.0, 600.0);
        for c in "2 3 +".chars() {
            assert_eq!(w.on_event(&ch(c)), Reaction::None);
        }
        assert!(texts(&w.render()).iter().any(|t| t.contains("2 3 +")));
        assert_eq!(w.on_event(&key(VK_RETURN)), Reaction::Submit("2 3 +".into()));
    }

    #[test]
    fn tab_switches_focus_and_editor_types() {
        let mut w = Workspace::new(900.0, 600.0);
        assert_eq!(w.focus, Focus::Repl);
        w.on_event(&key(VK_TAB));
        assert_eq!(w.focus, Focus::Editor);
        // typing now edits the editor buffer
        w.editor.new_file();
        for c in "9 dup *".chars() {
            w.on_event(&ch(c));
        }
        assert_eq!(w.editor.text(), "9 dup *");
        // ⌘⏎ evaluates the buffer
        assert_eq!(w.on_event(&keymod(VK_RETURN, MOD_COMMAND)), Reaction::EvalBuffer("9 dup *".into()));
    }

    #[test]
    fn record_renders_output_stack_and_ok() {
        let mut w = Workspace::new(900.0, 600.0);
        w.record("2 3 + .".into(), Ok("5 ".into()), vec![], false);
        let t = texts(&w.render());
        assert!(t.iter().any(|s| s.contains("2 3 + .")));
        assert!(t.iter().any(|s| s == "5 "));
        assert!(t.iter().any(|s| s == "ok"));
        w.record("7 11".into(), Ok(String::new()), vec![11, 7], false);
        let t = texts(&w.render());
        assert!(t.iter().any(|s| s == "11 ← top"));
    }

    #[test]
    fn editor_save_reaction() {
        let mut w = Workspace::new(900.0, 600.0);
        w.on_event(&key(VK_TAB)); // → editor
        assert_eq!(w.on_event(&keymod(0x53, MOD_COMMAND)), Reaction::Save);
    }
}
