//! Drives InlineTerminal and feeds its bytes to a real VT100 emulator, then
//! checks the visible screen and the scrollback.

use camouflage_inline::{Color, InlineTerminal, Line, LiveFrame, Style};

const W: u16 = 40;
const H: u16 = 10;

struct Harness {
    term: InlineTerminal<Vec<u8>>,
    vt: vt100::Parser,
    fed: usize,
}

impl Harness {
    fn new() -> Self {
        Self::sized(W, H)
    }
    fn sized(w: u16, h: u16) -> Self {
        Harness { term: InlineTerminal::new(Vec::new(), w, h), vt: vt100::Parser::new(h, w, 1000), fed: 0 }
    }
    /// Feed any new output to the emulator; returns how many bytes were new.
    fn sync(&mut self) -> usize {
        let out = self.term.get_ref();
        let new = &out[self.fed..];
        self.vt.process(new);
        let n = new.len();
        self.fed = out.len();
        n
    }
    fn screen(&self) -> Vec<String> {
        self.vt.screen().rows(0, self.vt.screen().size().1).map(|r| r.trim_end().to_string()).collect()
    }
    /// Every line ever shown: scrollback followed by the screen.
    fn all_lines(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        let mut back = 0;
        loop {
            self.vt.screen_mut().set_scrollback(back + 1);
            if self.vt.screen().scrollback() == back {
                break;
            }
            back += 1;
        }
        self.vt.screen_mut().set_scrollback(back);
        let cols = self.vt.screen().size().1;
        // Walk the scrollback from the oldest row down.
        for offset in (1..=back).rev() {
            self.vt.screen_mut().set_scrollback(offset);
            let top = self.vt.screen().rows(0, cols).next().unwrap_or_default();
            lines.push(top.trim_end().to_string());
        }
        self.vt.screen_mut().set_scrollback(0);
        lines.extend(self.screen());
        lines
    }
}

fn live(lines: &[&str]) -> LiveFrame {
    LiveFrame { lines: lines.iter().map(|s| Line::raw(*s)).collect(), cursor: None }
}

fn nonempty(v: Vec<String>) -> Vec<String> {
    v.into_iter().filter(|s| !s.is_empty()).collect()
}

#[test]
fn prints_history_above_the_live_region() {
    let mut h = Harness::new();
    h.term.print(&[Line::raw("$ autopilot"), Line::raw("hello")], &live(&["> input", "footer"])).unwrap();
    h.sync();
    assert_eq!(nonempty(h.screen()), vec!["$ autopilot", "hello", "> input", "footer"]);
}

#[test]
fn redraw_replaces_the_live_region_in_place() {
    let mut h = Harness::new();
    h.term.print(&[Line::raw("history")], &live(&["⠋ Thinking", "> ", "footer"])).unwrap();
    h.term.draw(&live(&["⠙ Thinking", "> hi", "footer"])).unwrap();
    h.sync();
    assert_eq!(nonempty(h.screen()), vec!["history", "⠙ Thinking", "> hi", "footer"]);
}

#[test]
fn shrinking_the_live_region_leaves_no_leftovers() {
    let mut h = Harness::new();
    h.term.print(&[], &live(&["a", "b", "c", "d"])).unwrap();
    h.term.draw(&live(&["a", "b"])).unwrap();
    h.sync();
    assert_eq!(nonempty(h.screen()), vec!["a", "b"]);
    h.term.draw(&live(&[])).unwrap();
    h.sync();
    assert!(nonempty(h.screen()).is_empty());
}

#[test]
fn history_scrolls_into_scrollback_and_is_never_rewritten() {
    let mut h = Harness::new();
    for i in 0..30 {
        h.term.print(&[Line::raw(format!("line {i}"))], &live(&["> input", "footer"])).unwrap();
    }
    h.sync();
    let all = nonempty(h.all_lines());
    let expected: Vec<String> = (0..30).map(|i| format!("line {i}")).chain(["> input".into(), "footer".into()]).collect();
    assert_eq!(all, expected);
}

#[test]
fn unchanged_frames_write_almost_nothing() {
    let mut h = Harness::new();
    let history: Vec<Line> = (0..200).map(|i| Line::raw(format!("a fairly long history line number {i}"))).collect();
    h.term.print(&history, &live(&["⠋ Working", "> ", "footer"])).unwrap();
    h.sync();
    h.term.draw(&live(&["⠋ Working", "> ", "footer"])).unwrap();
    let same = h.sync();
    h.term.draw(&live(&["⠙ Working", "> ", "footer"])).unwrap();
    let one_row = h.sync();
    assert!(same < 40, "an identical frame wrote {same} bytes");
    assert!(one_row < 80, "a one-row change wrote {one_row} bytes");
}

#[test]
fn live_region_taller_than_the_screen_keeps_its_bottom() {
    let mut h = Harness::new();
    let rows: Vec<String> = (0..25).map(|i| format!("row {i}")).collect();
    let refs: Vec<&str> = rows.iter().map(|s| s.as_str()).collect();
    h.term.print(&[Line::raw("history")], &live(&refs)).unwrap();
    h.term.draw(&live(&refs)).unwrap();
    h.sync();
    let screen = nonempty(h.screen());
    assert_eq!(screen.last().unwrap(), "row 24");
    assert!(screen.len() <= H as usize);
    // Redrawing again must not push extra copies into scrollback.
    let before = nonempty(h.all_lines()).len();
    h.term.draw(&live(&refs)).unwrap();
    h.sync();
    assert_eq!(nonempty(h.all_lines()).len(), before);
}

#[test]
fn full_width_rows_never_autowrap() {
    let mut h = Harness::new();
    let exact = "x".repeat((W - 1) as usize);
    let too_long = "y".repeat(W as usize + 10);
    h.term.print(&[Line::raw(&exact), Line::raw(&too_long)], &live(&["> input"])).unwrap();
    h.sync();
    let screen = nonempty(h.screen());
    assert_eq!(screen.len(), 3, "{screen:?}");
    assert_eq!(screen[0], exact);
    assert!(screen[1].ends_with('…'));
}

#[test]
fn background_fill_spans_the_row() {
    let mut h = Harness::new();
    let band = Line::styled("› prompt", Style::new()).fill(Color::BrightBlack);
    h.term.print(&[band], &live(&[])).unwrap();
    h.sync();
    let cell = h.vt.screen().cell(0, W - 2).unwrap();
    assert_eq!(cell.bgcolor(), vt100::Color::Idx(8));
}

#[test]
fn cursor_is_parked_at_the_requested_cell() {
    let mut h = Harness::new();
    let frame = LiveFrame { lines: vec![Line::raw("╭──╮"), Line::raw("│ › hi"), Line::raw("╰──╯"), Line::raw("footer")], cursor: Some((1, 6)) };
    h.term.print(&[Line::raw("history")], &frame).unwrap();
    h.sync();
    let (row, col) = h.vt.screen().cursor_position();
    assert_eq!((row, col), (2, 6));
    assert!(!h.vt.screen().hide_cursor());
    // A redraw from a parked cursor still lands in the right place.
    h.term.draw(&LiveFrame { lines: vec![Line::raw("╭──╮"), Line::raw("│ › hey"), Line::raw("╰──╯"), Line::raw("footer")], cursor: Some((1, 7)) }).unwrap();
    h.sync();
    assert_eq!(nonempty(h.screen()), vec!["history", "╭──╮", "│ › hey", "╰──╯", "footer"]);
    assert_eq!(h.vt.screen().cursor_position(), (2, 7));
}

#[test]
fn resize_redraws_without_duplicating_the_live_region() {
    let mut h = Harness::sized(40, 10);
    h.term.print(&[Line::raw("history")], &live(&["a live row that is thirty cells", "footer"])).unwrap();
    h.sync();
    // Emulate the terminal reflowing to 20 columns, then tell the writer.
    h.vt.screen_mut().set_size(10, 20);
    h.term.resize(20, 10, &live(&["narrow", "footer"])).unwrap();
    h.sync();
    let screen = nonempty(h.screen());
    assert_eq!(screen.iter().filter(|l| l.as_str() == "footer").count(), 1, "{screen:?}");
    assert_eq!(screen.last().unwrap(), "footer");
}

#[test]
fn finish_leaves_the_live_region_as_plain_text() {
    let mut h = Harness::new();
    h.term.print(&[Line::raw("history")], &live(&["Resume with: autopilot --resume 7f3a"])).unwrap();
    h.term.finish().unwrap();
    h.sync();
    assert_eq!(nonempty(h.screen()), vec!["history", "Resume with: autopilot --resume 7f3a"]);
    assert_eq!(h.vt.screen().cursor_position().0, 2);
}
