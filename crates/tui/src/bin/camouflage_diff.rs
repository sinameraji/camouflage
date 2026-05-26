//! `camouflage-diff` — replay two stored sessions side-by-side.
//!
//! Loads two sessions from a SQLite store and renders them in a split
//! pane, walking forward in lockstep. The first event seq where the
//! two streams diverge (by event_type or payload) is highlighted so
//! the user can see at a glance where two runs of the same prompt
//! started behaving differently. Pure replay viewer — no input,
//! no persistence, no host emission. The TUI's normal --replay mode
//! handles single sessions; this binary handles pairs.
//!
//! Usage:
//!     camouflage-diff --store run.db --a <uuid-a> --b <uuid-b>

use anyhow::{Context, Result};
use camouflage_protocol::Event;
use camouflage_renderer::{RenderModel, Row, RowKind};
use camouflage_store::{EventStore, SqliteStore};
use clap::Parser;
use crossterm::{
    event::{self, Event as CtEvent, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "camouflage-diff", about = "Replay two sessions side-by-side.")]
struct Args {
    #[arg(long)]
    store: PathBuf,
    #[arg(long, value_name = "SESSION_A")]
    a: Uuid,
    #[arg(long, value_name = "SESSION_B")]
    b: Uuid,
    /// Events per second at 1.0× speed.
    #[arg(long, default_value_t = 40.0)]
    eps: f32,
}

struct Pane {
    label: String,
    events: Vec<Event>,
    model: RenderModel,
    position: usize,
}

impl Pane {
    fn step_forward(&mut self, n: usize) {
        for _ in 0..n {
            if self.position >= self.events.len() {
                break;
            }
            self.model.apply(&self.events[self.position]);
            self.position += 1;
        }
    }
    fn rebuild_to(&mut self, target: usize) {
        let target = target.min(self.events.len());
        self.model = RenderModel::new();
        for ev in &self.events[..target] {
            self.model.apply(ev);
        }
        self.position = target;
    }
}

/// First seq at which the two sessions diverge by event_type or
/// payload. Compared up to the shorter of the two lengths; if all
/// shared-prefix events match, the divergence sits at min(len_a,len_b).
fn first_divergence(a: &[Event], b: &[Event]) -> Option<i64> {
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i].event_type != b[i].event_type || a[i].payload != b[i].payload {
            return Some(a[i].seq);
        }
    }
    if a.len() != b.len() {
        // One side ran longer — the divergence is the first extra event.
        a.get(n).or(b.get(n)).map(|e| e.seq)
    } else {
        None
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let store = SqliteStore::open(&args.store)
        .with_context(|| format!("opening store at {}", args.store.display()))?;
    let events_a = store.load_session(args.a).context("loading session A")?;
    let events_b = store.load_session(args.b).context("loading session B")?;
    if events_a.is_empty() && events_b.is_empty() {
        anyhow::bail!("both sessions are empty (or not found)");
    }
    let diverge_seq = first_divergence(&events_a, &events_b);

    let mut a = Pane {
        label: short_uuid(&args.a.to_string()),
        events: events_a,
        model: RenderModel::new(),
        position: 0,
    };
    let mut b = Pane {
        label: short_uuid(&args.b.to_string()),
        events: events_b,
        model: RenderModel::new(),
        position: 0,
    };

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut playing = true;
    let mut speed_mult: f32 = 1.0;
    let speeds = [0.25_f32, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 64.0];
    let mut last_tick = Instant::now();
    let mut accumulator: f32 = 0.0;

    let res: Result<()> = loop {
        // Advance both panes in lockstep when playing.
        let dt = last_tick.elapsed().as_secs_f32();
        last_tick = Instant::now();
        if playing {
            accumulator += dt * args.eps * speed_mult;
            let to_step = accumulator as usize;
            if to_step > 0 {
                accumulator -= to_step as f32;
                a.step_forward(to_step);
                b.step_forward(to_step);
            }
            if a.position >= a.events.len() && b.position >= b.events.len() {
                playing = false;
            }
        }

        terminal.draw(|f| {
            let area = f.area();
            let outer = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // header
                    Constraint::Min(1),    // split transcripts
                    Constraint::Length(1), // timeline
                    Constraint::Length(1), // hints
                ])
                .split(area);

            let header = Paragraph::new(Line::from(vec![
                Span::styled(
                    "Camouflage diff",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(format!("A={}", a.label), Style::default().fg(Color::LightGreen)),
                Span::raw("  "),
                Span::styled(format!("B={}", b.label), Style::default().fg(Color::LightMagenta)),
                Span::raw("  "),
                match diverge_seq {
                    Some(s) => Span::styled(
                        format!("Δ first divergence at seq {s}"),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ),
                    None => Span::styled(
                        "Δ no divergence",
                        Style::default().fg(Color::DarkGray),
                    ),
                },
            ]));
            f.render_widget(header, outer[0]);

            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(outer[1]);
            render_pane(f, split[0], &a, "A", diverge_seq, Color::LightGreen);
            render_pane(f, split[1], &b, "B", diverge_seq, Color::LightMagenta);

            let total = a.events.len().max(b.events.len()).max(1);
            let pos = a.position.max(b.position);
            let bar = build_timeline(outer[2].width as usize, pos, total, speed_mult, playing);
            f.render_widget(bar, outer[2]);

            let hints = Paragraph::new(Line::from(vec![
                Span::styled("q", Style::default().fg(Color::Cyan)),
                Span::styled(" quit  ", Style::default().fg(Color::DarkGray)),
                Span::styled("Space", Style::default().fg(Color::Cyan)),
                Span::styled(" play/pause  ", Style::default().fg(Color::DarkGray)),
                Span::styled("← →", Style::default().fg(Color::Cyan)),
                Span::styled(" step  ", Style::default().fg(Color::DarkGray)),
                Span::styled("+ -", Style::default().fg(Color::Cyan)),
                Span::styled(" speed  ", Style::default().fg(Color::DarkGray)),
                Span::styled("1-9", Style::default().fg(Color::Cyan)),
                Span::styled(" jump %  ", Style::default().fg(Color::DarkGray)),
                Span::styled("0", Style::default().fg(Color::Cyan)),
                Span::styled(" restart  ", Style::default().fg(Color::DarkGray)),
                Span::styled("G", Style::default().fg(Color::Cyan)),
                Span::styled(" end", Style::default().fg(Color::DarkGray)),
            ]));
            f.render_widget(hints, outer[3]);
        })?;

        // Poll input briefly so the render loop stays ~30fps during playback.
        if event::poll(Duration::from_millis(33))? {
            if let CtEvent::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                    KeyCode::Char(' ') => {
                        playing = !playing;
                        last_tick = Instant::now();
                        accumulator = 0.0;
                    }
                    KeyCode::Right => {
                        playing = false;
                        a.step_forward(1);
                        b.step_forward(1);
                    }
                    KeyCode::Left => {
                        playing = false;
                        let ta = a.position.saturating_sub(1);
                        let tb = b.position.saturating_sub(1);
                        a.rebuild_to(ta);
                        b.rebuild_to(tb);
                    }
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        speed_mult = next_speed(&speeds, speed_mult, true);
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        speed_mult = next_speed(&speeds, speed_mult, false);
                    }
                    KeyCode::Char('0') => {
                        playing = false;
                        a.rebuild_to(0);
                        b.rebuild_to(0);
                    }
                    KeyCode::Char('G') => {
                        playing = false;
                        let na = a.events.len();
                        let nb = b.events.len();
                        a.rebuild_to(na);
                        b.rebuild_to(nb);
                    }
                    KeyCode::Char(c @ '1'..='9') => {
                        playing = false;
                        let pct = (c as usize - b'0' as usize) * 10;
                        let ta = a.events.len() * pct / 100;
                        let tb = b.events.len() * pct / 100;
                        a.rebuild_to(ta);
                        b.rebuild_to(tb);
                    }
                    _ => {}
                }
            }
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

fn next_speed(levels: &[f32], current: f32, faster: bool) -> f32 {
    let idx = levels
        .iter()
        .position(|&v| (v - current).abs() < 0.01)
        .unwrap_or(2);
    let new = if faster {
        (idx + 1).min(levels.len() - 1)
    } else {
        idx.saturating_sub(1)
    };
    levels[new]
}

fn render_pane(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    pane: &Pane,
    title: &str,
    diverge_seq: Option<i64>,
    accent: Color,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {} · {} · {}/{} ", title, pane.label, pane.position, pane.events.len()),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Render the most-recent N rows that fit; older rows scroll off.
    let lines: Vec<Line> = pane
        .model
        .rows()
        .iter()
        .rev()
        .take(inner.height as usize)
        .map(|r| row_line(r, diverge_seq))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn row_line<'a>(r: &'a Row, diverge_seq: Option<i64>) -> Line<'a> {
    let diverged = diverge_seq.map_or(false, |s| r.seq >= s);
    let base = match r.kind {
        RowKind::User => Color::Cyan,
        RowKind::Assistant => Color::White,
        RowKind::Tool => Color::Blue,
        RowKind::Error => Color::Red,
        RowKind::System => Color::DarkGray,
        RowKind::Control => Color::Yellow,
        RowKind::Marker => Color::Magenta,
        RowKind::Diff => Color::Green,
    };
    let prefix = match r.kind {
        RowKind::User => "› ",
        RowKind::Assistant => "  ",
        RowKind::Tool => "⚙ ",
        RowKind::Error => "✗ ",
        RowKind::System => "· ",
        RowKind::Control => "• ",
        RowKind::Marker => "¶ ",
        RowKind::Diff => "± ",
    };
    let mut style = Style::default().fg(base);
    if diverged {
        // Tint diverged rows so the eye finds the cut point fast.
        style = style.bg(Color::Rgb(48, 30, 0));
    }
    Line::from(vec![
        Span::styled(prefix.to_string(), style),
        Span::styled(r.text.clone(), style),
    ])
}

fn build_timeline<'a>(
    width: usize,
    position: usize,
    total: usize,
    speed: f32,
    playing: bool,
) -> Paragraph<'a> {
    let suffix = format!(
        " {:>3}%  {}/{}  {:.2}×  {}",
        if total == 0 { 0 } else { (position * 100 / total).min(100) },
        position,
        total,
        speed,
        if position >= total { "■" } else if playing { "▶" } else { "‖" },
    );
    let suffix_w = suffix.chars().count();
    let track_w = width.saturating_sub(suffix_w).max(4);
    let halves = if total == 0 {
        0
    } else {
        ((position as u64 * (track_w as u64) * 2) / total as u64) as usize
    };
    let full = halves / 2;
    let half = halves % 2;
    let mut track = String::with_capacity(track_w * 3);
    for _ in 0..full {
        track.push('█');
    }
    if half == 1 {
        track.push('▌');
    }
    for _ in 0..(track_w.saturating_sub(full + half)) {
        track.push('·');
    }
    Paragraph::new(Line::from(vec![
        Span::styled(track, Style::default().fg(Color::Cyan)),
        Span::styled(suffix, Style::default().fg(Color::DarkGray)),
    ]))
}

fn short_uuid(s: &str) -> String {
    s.chars().take(8).collect()
}
