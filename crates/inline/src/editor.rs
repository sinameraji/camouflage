//! The prompt editor: a multi-line text buffer with readline-style editing.
//!
//! Fixes for the v2.1 input problems (docs/inline-redesign.md §1.1):
//! - the cursor moves by grapheme, so accents, CJK and emoji edit correctly
//! - Up/Down move between visual rows first, then walk history, and the
//!   draft you were typing is restored when you come back down
//! - `\` + Enter, Shift+Enter and Alt+Enter insert a newline in any terminal
//! - large pastes collapse into a `[Pasted text #1 +42 lines]` chip that is
//!   expanded back to the real text on submit
//! - kill/yank (Ctrl+K/U/W/Y) and undo (Ctrl+Z / Ctrl+_)
//!
//! Keys arrive as [`EditKey`], independent of any terminal library.

use crate::width::grapheme_width;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditKey {
    /// Typed text (one key, or an IME commit).
    Text(String),
    /// A bracketed paste.
    Paste(String),
    Enter { newline: bool },
    Backspace { word: bool },
    Delete { word: bool },
    Left { word: bool },
    Right { word: bool },
    Up,
    Down,
    LineStart,
    LineEnd,
    KillToEnd,
    KillToStart,
    Yank,
    Undo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditOutcome {
    /// Nothing changed.
    None,
    /// The buffer or cursor changed; redraw.
    Changed,
    /// Submit. `text` has paste chips expanded; `display` is what was typed.
    Submit { text: String, display: String },
}

/// A visual row of the buffer after wrapping: byte range `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
}

const PASTE_LINES: usize = 3;
const PASTE_CHARS: usize = 800;
const MAX_HISTORY: usize = 500;
const MAX_UNDO: usize = 200;

#[derive(Clone, Debug, Default)]
pub struct Editor {
    buf: String,
    /// Byte offset of the cursor, always on a grapheme boundary.
    cur: usize,
    history: Vec<String>,
    hist_idx: Option<usize>,
    draft: Option<String>,
    pastes: Vec<String>,
    undo: Vec<(String, usize)>,
    kill: String,
    /// Whether the last change was plain typing (consecutive typing is one
    /// undo step).
    typing: bool,
}

impl Editor {
    pub fn new() -> Self {
        Editor::default()
    }

    pub fn text(&self) -> &str {
        &self.buf
    }

    pub fn cursor(&self) -> usize {
        self.cur
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn set_history(&mut self, history: Vec<String>) {
        self.history = history;
        self.hist_idx = None;
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Replace the buffer (e.g. completing a picker selection).
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.snapshot(false);
        self.buf = text.into();
        self.cur = self.buf.len();
    }

    /// Replace `[from, cursor)` with `text`, e.g. finishing an `@mention`.
    pub fn replace_before_cursor(&mut self, from: usize, text: &str) {
        self.snapshot(false);
        let from = from.min(self.cur);
        self.buf.replace_range(from..self.cur, text);
        self.cur = from + text.len();
    }

    pub fn clear(&mut self) {
        if !self.buf.is_empty() {
            self.snapshot(false);
        }
        self.buf.clear();
        self.cur = 0;
        self.hist_idx = None;
        self.draft = None;
    }

    pub fn handle(&mut self, key: EditKey, width: usize) -> EditOutcome {
        use EditKey::*;
        match key {
            Text(t) => {
                let t = t.replace("\r\n", "\n").replace('\r', "\n");
                if t.is_empty() {
                    return EditOutcome::None;
                }
                self.snapshot(true);
                self.insert(&t);
            }
            Paste(t) => {
                let t = t.replace("\r\n", "\n").replace('\r', "\n");
                self.snapshot(false);
                let lines = t.lines().count().max(1);
                if lines > PASTE_LINES || t.chars().count() > PASTE_CHARS {
                    self.pastes.push(t);
                    let chip = format!("[Pasted text #{} +{} lines]", self.pastes.len(), lines);
                    self.insert(&chip);
                } else {
                    self.insert(&t);
                }
            }
            Enter { newline } => {
                if newline {
                    self.snapshot(false);
                    self.insert("\n");
                } else if self.buf[..self.cur].ends_with('\\') {
                    self.snapshot(false);
                    self.buf.replace_range(self.cur - 1..self.cur, "\n");
                } else {
                    return self.submit();
                }
            }
            Backspace { word } => {
                if self.cur == 0 {
                    return EditOutcome::None;
                }
                self.snapshot(false);
                let from = if let Some(start) = self.chip_before_cursor() {
                    start
                } else if word {
                    word_left(&self.buf, self.cur)
                } else {
                    prev_grapheme(&self.buf, self.cur)
                };
                self.buf.replace_range(from..self.cur, "");
                self.cur = from;
            }
            Delete { word } => {
                if self.cur >= self.buf.len() {
                    return EditOutcome::None;
                }
                self.snapshot(false);
                let to = if word { word_right(&self.buf, self.cur) } else { next_grapheme(&self.buf, self.cur) };
                self.buf.replace_range(self.cur..to, "");
            }
            Left { word } => {
                self.cur = if word { word_left(&self.buf, self.cur) } else { prev_grapheme(&self.buf, self.cur) };
                self.typing = false;
            }
            Right { word } => {
                self.cur = if word { word_right(&self.buf, self.cur) } else { next_grapheme(&self.buf, self.cur) };
                self.typing = false;
            }
            Up => {
                if !self.move_vertical(-1, width) {
                    return self.history_prev();
                }
            }
            Down => {
                if !self.move_vertical(1, width) {
                    return self.history_next();
                }
            }
            LineStart => {
                self.cur = self.buf[..self.cur].rfind('\n').map(|i| i + 1).unwrap_or(0);
            }
            LineEnd => {
                self.cur = self.buf[self.cur..].find('\n').map(|i| self.cur + i).unwrap_or(self.buf.len());
            }
            KillToEnd => {
                let end = self.buf[self.cur..].find('\n').map(|i| self.cur + i).unwrap_or(self.buf.len());
                let end = if end == self.cur && end < self.buf.len() { end + 1 } else { end };
                if end == self.cur {
                    return EditOutcome::None;
                }
                self.snapshot(false);
                self.kill = self.buf[self.cur..end].to_string();
                self.buf.replace_range(self.cur..end, "");
            }
            KillToStart => {
                let start = self.buf[..self.cur].rfind('\n').map(|i| i + 1).unwrap_or(0);
                if start == self.cur {
                    return EditOutcome::None;
                }
                self.snapshot(false);
                self.kill = self.buf[start..self.cur].to_string();
                self.buf.replace_range(start..self.cur, "");
                self.cur = start;
            }
            Yank => {
                if self.kill.is_empty() {
                    return EditOutcome::None;
                }
                self.snapshot(false);
                let k = self.kill.clone();
                self.insert(&k);
            }
            Undo => {
                let Some((buf, cur)) = self.undo.pop() else {
                    return EditOutcome::None;
                };
                self.buf = buf;
                self.cur = cur;
                self.typing = false;
            }
        }
        EditOutcome::Changed
    }

    fn insert(&mut self, t: &str) {
        self.buf.insert_str(self.cur, t);
        self.cur += t.len();
    }

    fn snapshot(&mut self, typing: bool) {
        if typing && self.typing {
            return;
        }
        self.typing = typing;
        self.undo.push((self.buf.clone(), self.cur));
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
    }

    fn submit(&mut self) -> EditOutcome {
        if self.buf.trim().is_empty() {
            return EditOutcome::None;
        }
        let display = std::mem::take(&mut self.buf);
        let text = self.expand_chips(&display);
        if self.history.last() != Some(&display) {
            self.history.push(display.clone());
            if self.history.len() > MAX_HISTORY {
                self.history.remove(0);
            }
        }
        self.cur = 0;
        self.hist_idx = None;
        self.draft = None;
        self.undo.clear();
        self.typing = false;
        EditOutcome::Submit { text, display }
    }

    fn expand_chips(&self, s: &str) -> String {
        let mut out = s.to_string();
        for (i, paste) in self.pastes.iter().enumerate().rev() {
            let n = i + 1;
            let lines = paste.lines().count().max(1);
            out = out.replace(&format!("[Pasted text #{n} +{lines} lines]"), paste);
        }
        out
    }

    /// Start of a paste chip that ends exactly at the cursor, if any.
    fn chip_before_cursor(&self) -> Option<usize> {
        let before = &self.buf[..self.cur];
        if !before.ends_with(" lines]") {
            return None;
        }
        let start = before.rfind("[Pasted text #")?;
        let chip = &before[start..];
        let inner = chip.strip_prefix("[Pasted text #")?.strip_suffix(" lines]")?;
        let (n, count) = inner.split_once(" +")?;
        (n.parse::<usize>().is_ok() && count.parse::<usize>().is_ok()).then_some(start)
    }

    fn history_prev(&mut self) -> EditOutcome {
        if self.history.is_empty() {
            return EditOutcome::None;
        }
        let idx = match self.hist_idx {
            None => {
                self.draft = Some(self.buf.clone());
                self.history.len() - 1
            }
            Some(0) => return EditOutcome::None,
            Some(i) => i - 1,
        };
        self.hist_idx = Some(idx);
        self.buf = self.history[idx].clone();
        self.cur = self.buf.len();
        EditOutcome::Changed
    }

    fn history_next(&mut self) -> EditOutcome {
        let Some(i) = self.hist_idx else {
            return EditOutcome::None;
        };
        if i + 1 < self.history.len() {
            self.hist_idx = Some(i + 1);
            self.buf = self.history[i + 1].clone();
        } else {
            self.hist_idx = None;
            self.buf = self.draft.take().unwrap_or_default();
        }
        self.cur = self.buf.len();
        EditOutcome::Changed
    }

    /// Wrap the buffer into visual rows of at most `width` cells.
    pub fn rows(&self, width: usize) -> Vec<Row> {
        let width = width.max(1);
        let mut rows = Vec::new();
        let mut off = 0;
        for logical in self.buf.split('\n') {
            let mut start = off;
            let mut w = 0;
            let mut idx = off;
            if logical.is_empty() {
                rows.push(Row { start: off, end: off });
            }
            let mut pushed_any = false;
            for g in logical.graphemes(true) {
                let gw = grapheme_width(g);
                if w + gw > width && w > 0 {
                    rows.push(Row { start, end: idx });
                    start = idx;
                    w = 0;
                }
                w += gw;
                idx += g.len();
                pushed_any = true;
            }
            if pushed_any {
                rows.push(Row { start, end: idx });
            }
            off += logical.len() + 1;
        }
        rows
    }

    /// The visual (row, column) of the cursor at this width.
    pub fn cursor_pos(&self, width: usize) -> (usize, usize) {
        let rows = self.rows(width);
        let r = cursor_row(&rows, self.cur);
        let col = self.buf[rows[r].start..self.cur].graphemes(true).map(grapheme_width).sum();
        (r, col)
    }

    fn move_vertical(&mut self, dir: i32, width: usize) -> bool {
        let rows = self.rows(width);
        let r = cursor_row(&rows, self.cur);
        let target = r as i32 + dir;
        if target < 0 || target as usize >= rows.len() {
            return false;
        }
        let col: usize = self.buf[rows[r].start..self.cur].graphemes(true).map(grapheme_width).sum();
        let t = rows[target as usize];
        let mut i = t.start;
        let mut w = 0;
        for g in self.buf[t.start..t.end].graphemes(true) {
            if w + grapheme_width(g) > col {
                break;
            }
            w += grapheme_width(g);
            i += g.len();
        }
        self.cur = i;
        self.typing = false;
        true
    }
}

fn cursor_row(rows: &[Row], cur: usize) -> usize {
    for (k, r) in rows.iter().enumerate() {
        let is_last = k + 1 == rows.len();
        // A cursor exactly at a soft wrap belongs to the next row; at a hard
        // line end (or the end of the buffer) it stays on this one.
        if cur >= r.start && (cur < r.end || (cur == r.end && (is_last || rows[k + 1].start != cur))) {
            return k;
        }
    }
    rows.len().saturating_sub(1)
}

fn prev_grapheme(s: &str, i: usize) -> usize {
    s[..i].grapheme_indices(true).next_back().map(|(j, _)| j).unwrap_or(0)
}

fn next_grapheme(s: &str, i: usize) -> usize {
    s[i..].graphemes(true).next().map(|g| i + g.len()).unwrap_or(s.len())
}

fn word_left(s: &str, i: usize) -> usize {
    let gs: Vec<(usize, &str)> = s[..i].grapheme_indices(true).collect();
    let mut k = gs.len();
    while k > 0 && gs[k - 1].1.trim().is_empty() {
        k -= 1;
    }
    while k > 0 && !gs[k - 1].1.trim().is_empty() {
        k -= 1;
    }
    gs.get(k).map(|(j, _)| *j).unwrap_or(i)
}

fn word_right(s: &str, i: usize) -> usize {
    let mut j = i;
    let mut it = s[i..].graphemes(true).peekable();
    while let Some(g) = it.peek() {
        if !g.trim().is_empty() {
            break;
        }
        j += g.len();
        it.next();
    }
    for g in it {
        if g.trim().is_empty() {
            break;
        }
        j += g.len();
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;
    use EditKey::*;

    fn typed(e: &mut Editor, s: &str) {
        for ch in s.chars() {
            e.handle(Text(ch.to_string()), 80);
        }
    }

    #[test]
    fn edits_graphemes_not_bytes() {
        let mut e = Editor::new();
        typed(&mut e, "café 👍🏽 日本");
        e.handle(Backspace { word: false }, 80);
        assert_eq!(e.text(), "café 👍🏽 日");
        e.handle(Left { word: true }, 80);
        e.handle(Left { word: false }, 80);
        e.handle(Backspace { word: false }, 80);
        assert_eq!(e.text(), "café  日", "the emoji with its skin tone is one grapheme");
    }

    #[test]
    fn capital_letters_are_just_text() {
        let mut e = Editor::new();
        typed(&mut e, "The Mouse Should eXit? no");
        assert_eq!(e.text(), "The Mouse Should eXit? no");
    }

    #[test]
    fn backslash_enter_and_modified_enter_insert_newlines() {
        let mut e = Editor::new();
        typed(&mut e, "one\\");
        assert_eq!(e.handle(Enter { newline: false }, 80), EditOutcome::Changed);
        typed(&mut e, "two");
        e.handle(Enter { newline: true }, 80);
        typed(&mut e, "three");
        assert_eq!(e.text(), "one\ntwo\nthree");
        match e.handle(Enter { newline: false }, 80) {
            EditOutcome::Submit { text, .. } => assert_eq!(text, "one\ntwo\nthree"),
            other => panic!("{other:?}"),
        }
        assert!(e.is_empty());
    }

    #[test]
    fn up_moves_between_lines_before_touching_history() {
        let mut e = Editor::new();
        e.set_history(vec!["old prompt".into()]);
        typed(&mut e, "first");
        e.handle(Enter { newline: true }, 80);
        typed(&mut e, "second");
        e.handle(Up, 80);
        assert_eq!(e.text(), "first\nsecond");
        assert_eq!(e.cursor_pos(80), (0, 5));
        e.handle(Up, 80);
        assert_eq!(e.text(), "old prompt");
    }

    #[test]
    fn history_keeps_the_draft() {
        let mut e = Editor::new();
        e.set_history(vec!["a".into(), "b".into()]);
        typed(&mut e, "half-typed");
        e.handle(Up, 80);
        e.handle(Up, 80);
        assert_eq!(e.text(), "a");
        e.handle(Down, 80);
        e.handle(Down, 80);
        assert_eq!(e.text(), "half-typed");
    }

    #[test]
    fn big_pastes_become_a_chip_that_expands_on_submit() {
        let mut e = Editor::new();
        typed(&mut e, "look: ");
        let blob = (1..=10).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        e.handle(Paste(blob.clone()), 80);
        assert_eq!(e.text(), "look: [Pasted text #1 +10 lines]");
        match e.handle(Enter { newline: false }, 80) {
            EditOutcome::Submit { text, display } => {
                assert_eq!(text, format!("look: {blob}"));
                assert_eq!(display, "look: [Pasted text #1 +10 lines]");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn backspace_removes_a_whole_chip() {
        let mut e = Editor::new();
        e.handle(Paste("a\nb\nc\nd\ne".into()), 80);
        e.handle(Backspace { word: false }, 80);
        assert!(e.is_empty());
    }

    #[test]
    fn small_pastes_keep_their_newlines() {
        let mut e = Editor::new();
        e.handle(Paste("x\ny".into()), 80);
        assert_eq!(e.text(), "x\ny");
    }

    #[test]
    fn kill_yank_and_undo() {
        let mut e = Editor::new();
        typed(&mut e, "hello world");
        e.handle(Backspace { word: true }, 80);
        assert_eq!(e.text(), "hello ");
        e.handle(KillToStart, 80);
        assert_eq!(e.text(), "");
        e.handle(Yank, 80);
        assert_eq!(e.text(), "hello ");
        e.handle(Undo, 80);
        assert_eq!(e.text(), "");
        e.handle(Undo, 80);
        assert_eq!(e.text(), "hello ");
        e.handle(Undo, 80);
        assert_eq!(e.text(), "hello world");
    }

    #[test]
    fn rows_wrap_by_width_and_track_the_cursor() {
        let mut e = Editor::new();
        typed(&mut e, "abcdefghij");
        assert_eq!(e.rows(4).len(), 3);
        assert_eq!(e.cursor_pos(4), (2, 2));
        e.handle(Up, 4);
        assert_eq!(e.cursor_pos(4), (1, 2));
    }
}
