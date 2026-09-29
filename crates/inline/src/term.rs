//! The inline terminal writer.
//!
//! The screen is split in two:
//!
//! - **History**: finished lines. Each is written exactly once, above the live
//!   region, and from then on belongs to the terminal's own scrollback. It is
//!   never redrawn, so its cost never grows with the session.
//! - **Live region**: a few lines at the bottom (streaming text, spinner,
//!   input box, footer). Redrawn in place, rewriting only rows that changed.
//!
//! Invariants that keep the cursor accounting exact:
//! - Every row we write is pre-wrapped to at most `width - 1` cells, so the
//!   terminal never auto-wraps (writing into the last column leaves some
//!   terminals in a pending-wrap state that breaks erase-in-line).
//! - The live region is clamped to `height - 1` rows (keeping its bottom), so
//!   it never scrolls its own top off-screen, which is what forces Ink to
//!   clear the whole terminal.
//! - We never clear the screen or scrollback.

use crate::style::Line;
use std::io::{self, Write};

/// A frame of the live region.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveFrame {
    pub lines: Vec<Line>,
    /// Where to park the hardware cursor (row, column) within `lines`, e.g. at
    /// the input caret so IMEs place their composition window correctly.
    /// `None` hides the cursor.
    pub cursor: Option<(usize, usize)>,
}

pub struct InlineTerminal<W: Write> {
    out: W,
    width: u16,
    height: u16,
    /// Encoded rows currently on screen in the live region, top to bottom.
    drawn: Vec<String>,
    /// Display width of each drawn row, for reflow estimates on resize.
    drawn_widths: Vec<usize>,
    /// Row of the live region the cursor is on right now.
    cursor_row: usize,
    cursor_visible: bool,
    /// Wrap output in synchronized-update markers (DEC 2026) so terminals
    /// that support it paint each frame atomically.
    pub synchronized: bool,
}

impl<W: Write> InlineTerminal<W> {
    pub fn new(out: W, width: u16, height: u16) -> Self {
        InlineTerminal {
            out,
            width: width.max(2),
            height: height.max(2),
            drawn: Vec::new(),
            drawn_widths: Vec::new(),
            cursor_row: 0,
            cursor_visible: true,
            synchronized: true,
        }
    }

    /// Width available to content: one less than the terminal, see module docs.
    pub fn content_width(&self) -> usize {
        self.width as usize - 1
    }

    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    pub fn max_live_rows(&self) -> usize {
        self.height as usize - 1
    }

    pub fn get_ref(&self) -> &W {
        &self.out
    }

    pub fn get_mut(&mut self) -> &mut W {
        &mut self.out
    }

    /// Print finished lines into history, then redraw `live` below them.
    /// Lines must already be wrapped to `content_width()`; longer ones are
    /// truncated rather than allowed to auto-wrap.
    pub fn print(&mut self, history: &[Line], live: &LiveFrame) -> io::Result<()> {
        let mut buf = String::new();
        self.begin(&mut buf);
        self.move_to_top(&mut buf);
        let width = self.content_width();
        for line in history {
            encode_row(line, width, &mut buf);
            buf.push_str("\x1b[K\r\n");
        }
        // Everything that was live is now overwritten or below the cursor.
        buf.push_str("\x1b[J");
        self.drawn.clear();
        self.drawn_widths.clear();
        self.cursor_row = 0;
        self.draw_rows(live, &mut buf);
        self.end(&mut buf);
        self.flush(buf)
    }

    /// Redraw the live region, rewriting only rows that changed.
    pub fn draw(&mut self, live: &LiveFrame) -> io::Result<()> {
        let mut buf = String::new();
        self.begin(&mut buf);
        self.move_to_top(&mut buf);
        self.draw_rows(live, &mut buf);
        self.end(&mut buf);
        self.flush(buf)
    }

    /// Handle a terminal resize. Terminals reflow our hard-broken rows
    /// independently, so the old live region now occupies
    /// `sum(ceil(row_width / new_width))` rows. Clear exactly that and redraw.
    pub fn resize(&mut self, width: u16, height: u16, live: &LiveFrame) -> io::Result<()> {
        let new_w = width.max(2) as usize;
        let reflowed = |w: usize| if w == 0 { 1 } else { w.div_ceil(new_w) };
        let rows_above_cursor: usize =
            self.drawn_widths.iter().take(self.cursor_row).map(|&w| reflowed(w)).sum();
        let mut buf = String::new();
        self.begin(&mut buf);
        buf.push('\r');
        if rows_above_cursor > 0 {
            buf.push_str(&format!("\x1b[{}A", rows_above_cursor));
        }
        buf.push_str("\x1b[J");
        self.width = width.max(2);
        self.height = height.max(2);
        self.drawn.clear();
        self.drawn_widths.clear();
        self.cursor_row = 0;
        self.draw_rows(live, &mut buf);
        self.end(&mut buf);
        self.flush(buf)
    }

    /// Leave the live region on screen as ordinary text and put the cursor on
    /// a fresh line below it, e.g. on exit.
    pub fn finish(&mut self) -> io::Result<()> {
        let mut buf = String::new();
        let last = self.drawn.len().saturating_sub(1);
        if self.cursor_row < last {
            buf.push_str(&format!("\x1b[{}B", last - self.cursor_row));
        }
        if !self.drawn.is_empty() {
            buf.push_str("\r\n");
        }
        buf.push_str("\x1b[0m\x1b[?25h");
        self.drawn.clear();
        self.drawn_widths.clear();
        self.cursor_row = 0;
        self.cursor_visible = true;
        self.flush(buf)
    }

    /// Erase the live region entirely (e.g. before handing the terminal to a
    /// full-screen view) and leave the cursor where it started.
    pub fn clear_live(&mut self) -> io::Result<()> {
        let mut buf = String::new();
        self.move_to_top(&mut buf);
        buf.push_str("\x1b[J");
        self.drawn.clear();
        self.drawn_widths.clear();
        self.cursor_row = 0;
        self.flush(buf)
    }

    fn begin(&self, buf: &mut String) {
        if self.synchronized {
            buf.push_str("\x1b[?2026h");
        }
        if self.cursor_visible {
            buf.push_str("\x1b[?25l");
        }
    }

    fn end(&mut self, buf: &mut String) {
        if self.synchronized {
            buf.push_str("\x1b[?2026l");
        }
    }

    fn move_to_top(&mut self, buf: &mut String) {
        buf.push('\r');
        if self.cursor_row > 0 {
            buf.push_str(&format!("\x1b[{}A", self.cursor_row));
        }
        self.cursor_row = 0;
    }

    /// Precondition: cursor at column 0 of the live region's first row.
    fn draw_rows(&mut self, live: &LiveFrame, buf: &mut String) {
        let width = self.content_width();
        let max = self.max_live_rows();
        let skip = live.lines.len().saturating_sub(max);
        let rows: Vec<(String, usize)> = live.lines[skip..]
            .iter()
            .map(|l| {
                let mut s = String::new();
                encode_row(l, width, &mut s);
                (s, l.width().min(width))
            })
            .collect();

        let old = self.drawn.len();
        for (i, (row, _)) in rows.iter().enumerate() {
            if i > 0 {
                // Move down; create the row with a newline if it doesn't exist
                // yet (this may scroll the screen, which is fine).
                if i < old {
                    buf.push_str("\x1b[B");
                } else {
                    buf.push_str("\r\n");
                }
            }
            if i >= old || self.drawn[i] != *row {
                buf.push('\r');
                buf.push_str(row);
                buf.push_str("\x1b[K");
            }
        }
        let last = rows.len().saturating_sub(1);
        if rows.len() < old {
            // Rows that no longer exist: clear everything below the new last row.
            buf.push_str("\r\n\x1b[J\x1b[A");
        }
        if rows.is_empty() {
            buf.push_str("\r\x1b[K");
        }
        self.drawn = rows.iter().map(|(r, _)| r.clone()).collect();
        self.drawn_widths = rows.iter().map(|(_, w)| *w).collect();
        self.cursor_row = last;

        // Park the cursor.
        match live.cursor {
            Some((r, c)) if r >= skip && r - skip < self.drawn.len() => {
                let r = r - skip;
                if r < last {
                    buf.push_str(&format!("\x1b[{}A", last - r));
                }
                buf.push('\r');
                if c > 0 {
                    buf.push_str(&format!("\x1b[{}C", c.min(width)));
                }
                self.cursor_row = r;
                buf.push_str("\x1b[?25h");
                self.cursor_visible = true;
            }
            _ => {
                buf.push('\r');
                self.cursor_visible = false;
            }
        }
    }

    fn flush(&mut self, buf: String) -> io::Result<()> {
        self.out.write_all(buf.as_bytes())?;
        self.out.flush()
    }
}

/// Encode one row, truncating anything wider than `width` so it can't wrap.
fn encode_row(line: &Line, width: usize, out: &mut String) {
    if line.width() <= width {
        line.encode(width, out);
        return;
    }
    let mut clipped = Line { spans: Vec::new(), fill: line.fill };
    let mut used = 0;
    for span in &line.spans {
        let w = span.width();
        if used + w <= width.saturating_sub(1) {
            clipped.spans.push(span.clone());
            used += w;
        } else {
            let t = crate::width::truncate(&span.text, width - used);
            clipped.spans.push(crate::style::Span::styled(t, span.style));
            break;
        }
    }
    clipped.encode(width, out);
}
