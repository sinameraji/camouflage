//! Ctrl+O: a full-screen, searchable transcript built from the session store.
//!
//! The inline renderer prints finished blocks once and drops them, so the
//! only complete copy of a session is SQLite. This view reads the events
//! back, renders them through the same block renderer with tool output
//! expanded, and pages them in the alternate screen. Closing it restores the
//! live area; nothing is written to scrollback.
//!
//! The app owns the event loop and feeds keys here, so the key reader never
//! races the view for the terminal.

use crate::replay::{transcript, ReplayConfig};
use anyhow::Result;
use camouflage_inline::{Line, Style};
use camouflage_protocol::Event;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::Write;

const ENTER: &[u8] = b"\x1b[?1049h\x1b[?25l";
const LEAVE: &[u8] = b"\x1b[?1049l\x1b[?25h";

pub struct Transcript {
    events: Vec<Event>,
    cfg: ReplayConfig,
    lines: Vec<Line>,
    /// Index of the first visible line.
    scroll: usize,
    width: usize,
    height: usize,
    search: Option<Search>,
}

struct Search {
    query: String,
    matches: Vec<usize>,
    current: usize,
    /// True while the user is still typing the query.
    editing: bool,
}

impl Transcript {
    pub fn new(events: Vec<Event>, cfg: ReplayConfig) -> Self {
        let width = cfg.width;
        let height = cfg.height;
        let lines = transcript(&events, &cfg);
        Self {
            events,
            cfg,
            lines,
            scroll: 0,
            width,
            height,
            search: None,
        }
    }

    /// Switch to the alternate screen and draw the first page.
    pub fn enter(&self, out: &mut impl Write) -> Result<()> {
        out.write_all(ENTER)?;
        out.flush()?;
        self.draw(out)
    }

    /// Restore the normal screen. The caller redraws its live area.
    pub fn leave(&self, out: &mut impl Write) -> Result<()> {
        out.write_all(LEAVE)?;
        out.flush()?;
        Ok(())
    }

    /// Returns true when the view should close.
    pub fn handle_key(&mut self, k: KeyEvent) -> bool {
        if let Some(search) = self.search.as_mut() {
            if search.editing {
                match k.code {
                    KeyCode::Esc => self.search = None,
                    KeyCode::Enter => {
                        search.editing = false;
                        search.matches = find_matches(&self.lines, &search.query);
                        search.current = 0;
                        self.jump_to_current_match();
                    }
                    KeyCode::Backspace => {
                        search.query.pop();
                    }
                    KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                        search.query.push(c);
                    }
                    _ => {}
                }
                return false;
            }
        }

        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(self.body_height()),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(self.body_height()),
            KeyCode::Home | KeyCode::Char('g') => self.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.scroll = self.max_scroll(),
            KeyCode::Char('/') => {
                self.search = Some(Search {
                    query: String::new(),
                    matches: Vec::new(),
                    current: 0,
                    editing: true,
                });
            }
            KeyCode::Char('n') => self.next_match(true),
            KeyCode::Char('N') => self.next_match(false),
            _ => {}
        }
        self.clamp_scroll();
        false
    }

    /// Rebuild at the new size. The blocks are wrapped to the session width,
    /// so a resize has to re-render them.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.cfg.width = width.saturating_sub(1);
        self.cfg.height = height;
        self.lines = transcript(&self.events, &self.cfg);
        self.clamp_scroll();
    }

    fn body_height(&self) -> usize {
        self.height.saturating_sub(2).max(1)
    }

    fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(self.body_height())
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn jump_to_current_match(&mut self) {
        let Some(search) = &self.search else { return };
        let Some(&line) = search.matches.get(search.current) else { return };
        // Put the match a third of the way down, like less.
        self.scroll = line.saturating_sub(self.body_height() / 3);
        self.clamp_scroll();
    }

    fn next_match(&mut self, forward: bool) {
        let Some(search) = self.search.as_mut() else { return };
        if search.matches.is_empty() {
            return;
        }
        let n = search.matches.len();
        search.current = if forward {
            (search.current + 1) % n
        } else {
            (search.current + n - 1) % n
        };
        self.jump_to_current_match();
    }

    pub fn draw(&self, out: &mut impl Write) -> Result<()> {
        let mut buf = String::new();
        buf.push_str("\x1b[H\x1b[2J");

        let theme = &self.cfg.theme;
        let header = Line::styled(
            format!(
                " Transcript · {} lines · q close · / search ",
                self.lines.len()
            ),
            Style::new().fg(theme.accent).bold(),
        );
        header.encode(self.width, &mut buf);
        buf.push_str("\x1b[0m\r\n");

        let body = self.body_height();
        for line in self.lines.iter().skip(self.scroll).take(body) {
            line.encode(self.width, &mut buf);
            buf.push_str("\x1b[0m\r\n");
        }
        // Clear any rows the transcript doesn't fill.
        for _ in self.lines.len().saturating_sub(self.scroll).min(body)..body {
            buf.push_str("\x1b[0m\r\n");
        }

        let footer = self.footer();
        footer.encode(self.width, &mut buf);
        buf.push_str("\x1b[0m");

        out.write_all(buf.as_bytes())?;
        out.flush()?;
        Ok(())
    }

    fn footer(&self) -> Line {
        let dim = Style::new().dim();
        if let Some(search) = &self.search {
            if search.editing {
                return Line::raw(format!(
                    " /{}  (enter to jump, esc to cancel)",
                    search.query
                ));
            }
            let count = if search.matches.is_empty() {
                "no matches".to_string()
            } else {
                format!("{}/{}", search.current + 1, search.matches.len())
            };
            return Line::raw(format!(" /{}  {count}  (n/N to move)", search.query));
        }
        let total = self.lines.len();
        let first = (self.scroll + 1).min(total);
        let last = (self.scroll + self.body_height()).min(total);
        let mut line = Line::raw(format!(" {first}-{last} of {total} "));
        line.push(" · ctrl+o or q to close", dim);
        line
    }
}

fn find_matches(lines: &[Line], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = query.to_lowercase();
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.plain_text().to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use camouflage_inline::Theme;
    use camouflage_protocol::{EventType, SCHEMA_VERSION};
    use serde_json::json;
    use uuid::Uuid;

    fn ev(seq: i64, event_type: EventType, payload: serde_json::Value) -> Event {
        Event {
            id: Uuid::nil(),
            session_id: Uuid::nil(),
            seq,
            timestamp_ms: seq * 10,
            schema_version: SCHEMA_VERSION,
            event_type,
            payload,
        }
    }

    fn transcript_with(lines: &[&str]) -> Transcript {
        let mut events = vec![ev(0, EventType::SessionStarted, json!({}))];
        for (i, text) in lines.iter().enumerate() {
            events.push(ev(
                i as i64 + 1,
                EventType::UserMessageCreated,
                json!({ "text": text }),
            ));
        }
        Transcript::new(
            events,
            ReplayConfig {
                app_title: "autopilot".into(),
                theme: Theme::dark(),
                width: 80,
                height: 10,
            },
        )
    }

    #[test]
    fn search_finds_lines_case_insensitively() {
        let t = transcript_with(&["Fix the parser", "add a test", "PARSER again"]);
        let matches = find_matches(&t.lines, "parser");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn scrolling_is_clamped_to_the_transcript() {
        let mut t = transcript_with(&["one", "two", "three"]);
        t.scroll = 999;
        t.clamp_scroll();
        assert_eq!(t.scroll, t.max_scroll());
        t.scroll = 0;
        t.clamp_scroll();
        assert_eq!(t.scroll, 0);
    }

    #[test]
    fn q_closes_and_arrows_scroll() {
        let lines: Vec<String> = (0..20).map(|i| format!("line {i}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut t = transcript_with(&refs);
        assert!(!t.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        assert_eq!(t.scroll, 1);
        assert!(t.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    }

    #[test]
    fn slash_opens_search_and_enter_jumps() {
        let mut t = transcript_with(&["alpha", "beta", "gamma"]);
        t.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for c in "gamma".chars() {
            t.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        t.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let search = t.search.as_ref().expect("search stays open");
        assert_eq!(search.matches.len(), 1);
        assert!(t.scroll <= search.matches[0]);
    }

    #[test]
    fn resize_rewraps_the_transcript() {
        let mut t = transcript_with(&["a long line that wraps at narrow widths"]);
        let before = t.lines.len();
        t.resize(30, 10);
        assert!(t.lines.len() >= before);
    }
}
