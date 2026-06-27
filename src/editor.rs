//! The workspace editor — a thin editing surface over the vendored rope buffer
//! (`rope_buffer::RopeBuffer`, the same O(log n) AVL rope WF66/MacNCL/locus use).
//! Cursor is a code-point offset into the rope; line/column is derived on demand.
//! Pure + headless-testable: file I/O, edit ops, and a Forth re-indent formatter.

use std::path::PathBuf;

use crate::rope_buffer::{codepoints_to_utf8, RopeBuffer};

pub struct Editor {
    rope: RopeBuffer,
    caret: usize,      // code-point offset
    goal_col: usize,   // column vertical motion tries to keep
    pub top: usize,    // first visible line (scroll)
    pub path: Option<PathBuf>,
    pub dirty: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        Editor { rope: RopeBuffer::new(), caret: 0, goal_col: 0, top: 0, path: None, dirty: false }
    }

    // ── text ──
    pub fn text(&self) -> String {
        self.rope.to_utf8()
    }
    pub fn set_text(&mut self, s: &str) {
        self.rope = RopeBuffer::from_utf8(s.as_bytes());
        self.caret = self.caret.min(self.rope.len());
    }
    pub fn line_count(&self) -> usize {
        self.rope.line_count()
    }
    pub fn line(&self, row: usize) -> String {
        codepoints_to_utf8(&self.rope.get_line(row))
    }
    /// Cursor as (row, col), 0-based.
    pub fn cursor(&self) -> (usize, usize) {
        self.rope.offset_to_line_col(self.caret)
    }

    // ── file ops ──
    pub fn new_file(&mut self) {
        self.rope = RopeBuffer::new();
        self.caret = 0;
        self.top = 0;
        self.path = None;
        self.dirty = false;
    }
    pub fn load(&mut self, path: impl Into<PathBuf>) -> std::io::Result<()> {
        let p = path.into();
        let bytes = std::fs::read(&p)?;
        self.rope = RopeBuffer::from_utf8(&bytes);
        self.caret = 0;
        self.top = 0;
        self.path = Some(p);
        self.dirty = false;
        Ok(())
    }
    pub fn save(&mut self) -> std::io::Result<()> {
        let p = self
            .path
            .clone()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "no file (use save-as)"))?;
        std::fs::write(&p, self.text())?;
        self.dirty = false;
        Ok(())
    }
    pub fn save_as(&mut self, path: impl Into<PathBuf>) -> std::io::Result<()> {
        self.path = Some(path.into());
        self.save()
    }
    pub fn file_label(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        if self.dirty {
            format!("{name} *")
        } else {
            name
        }
    }

    // ── editing ──
    pub fn insert_char(&mut self, c: char) {
        self.rope.insert_char(self.caret, c as u32);
        self.caret += 1;
        self.sync_goal();
        self.dirty = true;
    }
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
    }
    pub fn newline(&mut self) {
        self.insert_char('\n');
    }
    pub fn backspace(&mut self) {
        if self.caret > 0 {
            self.rope.delete(self.caret - 1, 1);
            self.caret -= 1;
            self.sync_goal();
            self.dirty = true;
        }
    }
    pub fn delete_forward(&mut self) {
        if self.caret < self.rope.len() {
            self.rope.delete(self.caret, 1);
            self.dirty = true;
        }
    }

    // ── motion ──
    pub fn move_left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
        self.sync_goal();
    }
    pub fn move_right(&mut self) {
        if self.caret < self.rope.len() {
            self.caret += 1;
        }
        self.sync_goal();
    }
    pub fn move_up(&mut self) {
        let (row, _) = self.cursor();
        if row > 0 {
            self.caret = self.rope.line_col_to_offset(row - 1, self.goal_col);
        }
    }
    pub fn move_down(&mut self) {
        let (row, _) = self.cursor();
        if row + 1 < self.rope.line_count() {
            self.caret = self.rope.line_col_to_offset(row + 1, self.goal_col);
        }
    }
    pub fn home(&mut self) {
        let (row, _) = self.cursor();
        if let Some((start, _)) = self.rope.line_range(row) {
            self.caret = start;
        }
        self.sync_goal();
    }
    pub fn end(&mut self) {
        let (row, _) = self.cursor();
        if let Some((_, end)) = self.rope.line_range(row) {
            // line_range end includes the trailing '\n'; stop before it.
            let nl = self.rope.char_at(end.saturating_sub(1)) == Some('\n' as u32);
            self.caret = if nl && end > 0 { end - 1 } else { end };
        }
        self.sync_goal();
    }

    fn sync_goal(&mut self) {
        let (_, col) = self.cursor();
        self.goal_col = col;
    }

    /// Scroll so the caret row is within `[top, top+rows)`.
    pub fn ensure_visible(&mut self, rows: usize) {
        let (row, _) = self.cursor();
        if row < self.top {
            self.top = row;
        } else if rows > 0 && row >= self.top + rows {
            self.top = row + 1 - rows;
        }
    }

    /// Reindent + normalise spacing of the whole buffer (a basic Forth pretty-
    /// printer: 2 spaces per nesting level, single-spaced words). Keeps the
    /// caret near where it was (clamped).
    pub fn format(&mut self) {
        let formatted = format_forth(&self.text());
        self.set_text(&formatted);
        self.dirty = true;
    }
}

/// Words that open / close an indentation level.
fn opens(w: &str) -> bool {
    matches!(w, ":" | "if" | "begin" | "do" | "?do" | "case" | "of" | "[if]")
}
fn closes(w: &str) -> bool {
    matches!(
        w,
        ";" | "then" | "loop" | "+loop" | "repeat" | "until" | "again" | "endcase" | "endof" | "[then]"
    )
}
/// Words that dedent only their own line (the level continues after).
fn dedent_line(w: &str) -> bool {
    matches!(w, "else" | "while" | "of" | "endof" | "[else]")
}

/// Basic Forth source formatter: re-indent each line by structural depth and
/// collapse runs of whitespace between words to a single space. Blank lines and
/// the interior of `( … )`/`\ …` comments and `s" … "` strings are left intact.
pub fn format_forth(src: &str) -> String {
    let mut depth: i32 = 0;
    let mut out = String::new();
    for raw in src.split('\n') {
        let line = raw.trim();
        if line.is_empty() {
            out.push('\n');
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let first = words[0];
        // a line that begins by closing a block dedents itself
        let this_depth = if closes(first) || dedent_line(first) {
            (depth - 1).max(0)
        } else {
            depth
        };
        for _ in 0..this_depth {
            out.push_str("  ");
        }
        // normalise spacing (comments/strings keep their text but lose run-length
        // spacing — acceptable for a v1 formatter)
        out.push_str(&words.join(" "));
        out.push('\n');
        // update running depth across the whole line's words
        let mut in_comment = false;
        for w in &words {
            if in_comment {
                if w.ends_with(')') {
                    in_comment = false;
                }
                continue;
            }
            match *w {
                "(" => in_comment = true,
                "\\" => break, // line comment — ignore the rest
                _ if opens(w) => depth += 1,
                _ if closes(w) => depth -= 1,
                _ => {}
            }
        }
        depth = depth.max(0);
    }
    // drop one trailing newline that split/rejoin introduces
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_lines() {
        let mut e = Editor::new();
        e.insert_str("2 3 +");
        assert_eq!(e.text(), "2 3 +");
        assert_eq!(e.cursor(), (0, 5));
        e.newline();
        e.insert_str("dup");
        assert_eq!(e.line_count(), 2);
        assert_eq!(e.line(1), "dup");
        assert_eq!(e.cursor(), (1, 3));
    }

    #[test]
    fn backspace_and_motion() {
        let mut e = Editor::new();
        e.insert_str("abc\ndef");
        e.move_up(); // → row 0, col 3 (goal 3)
        assert_eq!(e.cursor(), (0, 3));
        e.home();
        assert_eq!(e.cursor(), (0, 0));
        e.end();
        assert_eq!(e.cursor(), (0, 3));
        e.backspace();
        assert_eq!(e.line(0), "ab");
    }

    #[test]
    fn format_reindents() {
        let src = ": sq dup * ;\n: f 0 if 1 else 2 then ;";
        let out = format_forth(src);
        // body of a definition / if-branch indents
        assert!(out.contains(": sq"), "{out}");
        assert!(out.lines().any(|l| l.starts_with("  ") && l.contains("dup")) || out.contains(": sq dup * ;"));
        // round-trips through the editor
        let mut e = Editor::new();
        e.set_text(src);
        e.format();
        assert_eq!(e.text(), out);
    }

    #[test]
    fn file_roundtrip() {
        let mut e = Editor::new();
        e.insert_str(": greet 42 ;");
        let p = std::env::temp_dir().join("mf66_editor_test.f");
        e.save_as(&p).unwrap();
        assert!(!e.dirty);
        let mut e2 = Editor::new();
        e2.load(&p).unwrap();
        assert_eq!(e2.text(), ": greet 42 ;");
        assert_eq!(e2.file_label(), "mf66_editor_test.f");
        std::fs::remove_file(&p).ok();
    }
}
