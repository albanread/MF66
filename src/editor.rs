//! The workspace editor — a thin editing surface over the vendored rope buffer
//! (`rope_buffer::RopeBuffer`, the same O(log n) AVL rope WF66/MacNCL/locus use).
//! Cursor is a code-point offset; line/column is derived on demand. Supports a
//! selection (anchor + caret), an in-process clipboard, and snapshot undo/redo.
//! Pure + headless-testable: file I/O, edit ops, and a Forth re-indent formatter.

use std::path::PathBuf;

use crate::rope_buffer::{codepoints_to_utf8, RopeBuffer};

#[derive(Clone, Copy, PartialEq)]
enum Last {
    None,
    Insert,
    Delete,
}

pub struct Editor {
    rope: RopeBuffer,
    caret: usize,            // code-point offset
    anchor: Option<usize>,   // selection's fixed end (caret is the moving end)
    goal_col: usize,         // column vertical motion tries to keep
    clip: String,            // in-process clipboard
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
    last: Last,
    pub top: usize, // first visible line (scroll)
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
        Editor {
            rope: RopeBuffer::new(),
            caret: 0,
            anchor: None,
            goal_col: 0,
            clip: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            last: Last::None,
            top: 0,
            path: None,
            dirty: false,
        }
    }

    // ── text ──
    pub fn text(&self) -> String {
        self.rope.to_utf8()
    }
    pub fn set_text(&mut self, s: &str) {
        self.rope = RopeBuffer::from_utf8(s.as_bytes());
        self.caret = self.caret.min(self.rope.len());
        self.anchor = None;
        self.undo.clear();
        self.redo.clear();
        self.last = Last::None;
    }
    pub fn line_count(&self) -> usize {
        self.rope.line_count()
    }
    pub fn line(&self, row: usize) -> String {
        codepoints_to_utf8(&self.rope.get_line(row))
    }
    pub fn cursor(&self) -> (usize, usize) {
        self.rope.offset_to_line_col(self.caret)
    }
    pub fn offset_rowcol(&self, off: usize) -> (usize, usize) {
        self.rope.offset_to_line_col(off)
    }

    // ── file ops ──
    pub fn new_file(&mut self) {
        self.set_text("");
        self.caret = 0;
        self.top = 0;
        self.path = None;
        self.dirty = false;
    }
    pub fn load(&mut self, path: impl Into<PathBuf>) -> std::io::Result<()> {
        let p = path.into();
        let bytes = std::fs::read(&p)?;
        self.set_text(&String::from_utf8_lossy(&bytes));
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

    // ── selection ──
    /// Selection as a `(start, end)` offset pair (start < end), or `None`.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.anchor.and_then(|a| {
            if a == self.caret {
                None
            } else {
                Some((a.min(self.caret), a.max(self.caret)))
            }
        })
    }
    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(s, e)| codepoints_to_utf8(&self.rope.slice(s, e)))
    }
    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.caret = self.rope.len();
    }
    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }
    fn delete_selection(&mut self) -> bool {
        if let Some((s, e)) = self.selection() {
            self.rope.delete(s, e - s);
            self.caret = s;
            self.anchor = None;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    // ── clipboard ──
    pub fn clipboard(&self) -> &str {
        &self.clip
    }
    pub fn copy(&mut self) {
        if let Some(t) = self.selected_text() {
            self.clip = t;
        }
    }
    pub fn cut(&mut self) {
        if self.selection().is_some() {
            self.copy();
            self.checkpoint(Last::None);
            self.delete_selection();
        }
    }
    pub fn paste(&mut self) {
        let clip = self.clip.clone();
        if clip.is_empty() {
            return;
        }
        self.checkpoint(Last::None);
        self.delete_selection();
        for c in clip.chars() {
            self.rope.insert_char(self.caret, c as u32);
            self.caret += 1;
        }
        self.dirty = true;
        self.sync_goal();
    }

    // ── undo / redo ──
    fn checkpoint(&mut self, kind: Last) {
        let coalesce = matches!(
            (self.last, kind),
            (Last::Insert, Last::Insert) | (Last::Delete, Last::Delete)
        );
        if !coalesce {
            self.undo.push((self.rope.to_utf8(), self.caret));
            self.redo.clear();
            if self.undo.len() > 500 {
                self.undo.remove(0);
            }
        }
        self.last = kind;
    }
    pub fn undo(&mut self) -> bool {
        if let Some((text, caret)) = self.undo.pop() {
            self.redo.push((self.rope.to_utf8(), self.caret));
            self.rope = RopeBuffer::from_utf8(text.as_bytes());
            self.caret = caret.min(self.rope.len());
            self.anchor = None;
            self.last = Last::None;
            self.dirty = true;
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self) -> bool {
        if let Some((text, caret)) = self.redo.pop() {
            self.undo.push((self.rope.to_utf8(), self.caret));
            self.rope = RopeBuffer::from_utf8(text.as_bytes());
            self.caret = caret.min(self.rope.len());
            self.anchor = None;
            self.last = Last::None;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    // ── editing ──
    pub fn insert_char(&mut self, c: char) {
        self.checkpoint(Last::Insert);
        self.delete_selection();
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
        self.checkpoint(Last::None);
        self.delete_selection();
        self.rope.insert_char(self.caret, '\n' as u32);
        self.caret += 1;
        self.sync_goal();
        self.dirty = true;
    }
    pub fn backspace(&mut self) {
        self.checkpoint(Last::Delete);
        if self.delete_selection() {
            return;
        }
        if self.caret > 0 {
            self.rope.delete(self.caret - 1, 1);
            self.caret -= 1;
            self.sync_goal();
            self.dirty = true;
        }
    }
    pub fn delete_forward(&mut self) {
        self.checkpoint(Last::Delete);
        if self.delete_selection() {
            return;
        }
        if self.caret < self.rope.len() {
            self.rope.delete(self.caret, 1);
            self.dirty = true;
        }
    }

    // ── motion (extend = hold Shift to grow the selection) ──
    fn pre_move(&mut self, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.caret);
            }
        } else {
            self.anchor = None;
        }
        self.last = Last::None;
    }
    pub fn move_left(&mut self, extend: bool) {
        self.pre_move(extend);
        self.caret = self.caret.saturating_sub(1);
        self.sync_goal();
    }
    pub fn move_right(&mut self, extend: bool) {
        self.pre_move(extend);
        if self.caret < self.rope.len() {
            self.caret += 1;
        }
        self.sync_goal();
    }
    pub fn move_up(&mut self, extend: bool) {
        self.pre_move(extend);
        let (row, _) = self.cursor();
        if row > 0 {
            self.caret = self.rope.line_col_to_offset(row - 1, self.goal_col);
        }
    }
    pub fn move_down(&mut self, extend: bool) {
        self.pre_move(extend);
        let (row, _) = self.cursor();
        if row + 1 < self.rope.line_count() {
            self.caret = self.rope.line_col_to_offset(row + 1, self.goal_col);
        }
    }
    pub fn home(&mut self, extend: bool) {
        self.pre_move(extend);
        let (row, _) = self.cursor();
        if let Some((start, _)) = self.rope.line_range(row) {
            self.caret = start;
        }
        self.sync_goal();
    }
    pub fn end(&mut self, extend: bool) {
        self.pre_move(extend);
        let (row, _) = self.cursor();
        if let Some((_, end)) = self.rope.line_range(row) {
            let nl = self.rope.char_at(end.saturating_sub(1)) == Some('\n' as u32);
            self.caret = if nl && end > 0 { end - 1 } else { end };
        }
        self.sync_goal();
    }

    /// Place the caret at `(row, col)` (clamped), clearing the selection. Used by
    /// a mouse click and the `caret` verb.
    pub fn set_caret_rowcol(&mut self, row: usize, col: usize) {
        let row = row.min(self.rope.line_count().saturating_sub(1));
        self.caret = self.rope.line_col_to_offset(row, col);
        self.anchor = None;
        self.last = Last::None;
        self.sync_goal();
    }
    /// Extend the selection to `(row, col)` — the moving end of a drag.
    pub fn extend_to_rowcol(&mut self, row: usize, col: usize) {
        if self.anchor.is_none() {
            self.anchor = Some(self.caret);
        }
        let row = row.min(self.rope.line_count().saturating_sub(1));
        self.caret = self.rope.line_col_to_offset(row, col);
        self.last = Last::None;
        self.sync_goal();
    }
    /// Scroll the visible window by `delta` lines (mouse wheel).
    pub fn scroll(&mut self, delta: i32) {
        let max = self.rope.line_count().saturating_sub(1) as i32;
        self.top = (self.top as i32 + delta).clamp(0, max) as usize;
    }

    fn sync_goal(&mut self) {
        let (_, col) = self.cursor();
        self.goal_col = col;
    }

    pub fn ensure_visible(&mut self, rows: usize) {
        let (row, _) = self.cursor();
        if row < self.top {
            self.top = row;
        } else if rows > 0 && row >= self.top + rows {
            self.top = row + 1 - rows;
        }
    }

    /// Reindent + normalise spacing of the whole buffer.
    pub fn format(&mut self) {
        self.checkpoint(Last::None);
        let formatted = format_forth(&self.text());
        let caret = self.caret;
        self.rope = RopeBuffer::from_utf8(formatted.as_bytes());
        self.caret = caret.min(self.rope.len());
        self.anchor = None;
        self.dirty = true;
    }
}

fn opens(w: &str) -> bool {
    matches!(w, ":" | "if" | "begin" | "do" | "?do" | "case" | "of" | "[if]")
}
fn closes(w: &str) -> bool {
    matches!(
        w,
        ";" | "then" | "loop" | "+loop" | "repeat" | "until" | "again" | "endcase" | "endof" | "[then]"
    )
}
fn dedent_line(w: &str) -> bool {
    matches!(w, "else" | "while" | "of" | "endof" | "[else]")
}

/// Basic Forth source formatter: re-indent each line by structural depth and
/// collapse runs of whitespace between words to a single space.
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
        let this_depth = if closes(first) || dedent_line(first) {
            (depth - 1).max(0)
        } else {
            depth
        };
        for _ in 0..this_depth {
            out.push_str("  ");
        }
        out.push_str(&words.join(" "));
        out.push('\n');
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
                "\\" => break,
                _ if opens(w) => depth += 1,
                _ if closes(w) => depth -= 1,
                _ => {}
            }
        }
        depth = depth.max(0);
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_motion_undo() {
        let mut e = Editor::new();
        e.insert_str("2 3 +");
        assert_eq!(e.text(), "2 3 +");
        e.newline();
        e.insert_str("dup");
        assert_eq!(e.line(1), "dup");
        // undo the "dup" typing, then the newline
        assert!(e.undo());
        assert_eq!(e.text(), "2 3 +\n");
        assert!(e.undo());
        assert_eq!(e.text(), "2 3 +");
        // redo
        assert!(e.redo());
        assert_eq!(e.text(), "2 3 +\n");
    }

    #[test]
    fn selection_copy_cut_paste() {
        let mut e = Editor::new();
        e.insert_str("hello world");
        e.home(false); // col 0
        for _ in 0..5 {
            e.move_right(true); // select "hello"
        }
        assert_eq!(e.selected_text().as_deref(), Some("hello"));
        e.copy();
        assert_eq!(e.clipboard(), "hello");
        e.end(false);
        e.insert_char(' ');
        e.paste(); // → "hello world hello"
        assert_eq!(e.text(), "hello world hello");
        // cut a selection
        e.home(false);
        for _ in 0..5 {
            e.move_right(true);
        }
        e.cut();
        assert_eq!(e.text(), " world hello");
        assert_eq!(e.clipboard(), "hello");
    }

    #[test]
    fn select_all_then_replace() {
        let mut e = Editor::new();
        e.insert_str("old text");
        e.select_all();
        e.insert_char('X'); // replaces the whole selection
        assert_eq!(e.text(), "X");
    }

    #[test]
    fn format_and_file_roundtrip() {
        let mut e = Editor::new();
        e.set_text(": sq dup * ;\n: f 0 if 1 else 2 then ;");
        e.format();
        assert!(e.undo()); // format is undoable
        assert_eq!(e.text(), ": sq dup * ;\n: f 0 if 1 else 2 then ;");
        let p = std::env::temp_dir().join("mf66_editor_sel_test.f");
        e.save_as(&p).unwrap();
        let mut e2 = Editor::new();
        e2.load(&p).unwrap();
        assert!(e2.text().starts_with(": sq"));
        std::fs::remove_file(&p).ok();
    }
}
