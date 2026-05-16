use anyhow::Result;
use camouflage_renderer::{RenderModel, RowKind, ViewportState};
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;

pub fn render<B: Backend>(
    terminal: &mut Terminal<B>,
    model: &RenderModel,
    viewport: &ViewportState,
    input_buf: &str,
    status: &str,
) -> Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(3),
            ])
            .split(area);

        // Header
        let header = Paragraph::new(Line::from(vec![
            Span::styled("Camouflage ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("session={}", short_uuid(&viewport.session_id.to_string())),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        f.render_widget(header, chunks[0]);

        // Transcript viewport — virtualized over history + live rows.
        let history = model.history_rows();
        let live = model.rows();
        let total = (history.len() + live.len()) as i64;
        let height = chunks[1].height as i64;
        let end = (total - viewport.scroll_offset).max(0);
        let start = (end - height).max(0);
        let combined = history.iter().chain(live.iter());
        let lines: Vec<Line> = combined
            .skip(start as usize)
            .take((end - start) as usize)
            .map(|r| row_to_line(r))
            .collect();
        let transcript = Paragraph::new(lines);
        f.render_widget(transcript, chunks[1]);

        // Status line — multi-segment, host-driven. Convention: known keys
        // (mode, phase, elapsed, tokens, cost, branch, warn) render in that
        // order with appropriate styling. Any other segments follow in
        // alphabetical order. `phase` falls back to the renderer-derived
        // `status` arg if the host hasn't set one yet.
        let follow = if viewport.auto_follow { "follow" } else { "scrolled" };
        let indicator = if !viewport.auto_follow {
            " | new output below ↓"
        } else {
            ""
        };
        let segs = model.status_segments();
        let mut spans: Vec<Span> = Vec::new();
        // mode badge
        if let Some(mode) = segs.get("mode") {
            let color = match mode.as_str() {
                "plan" => Color::Magenta,
                "auto" => Color::Green,
                _ => Color::Cyan,
            };
            spans.push(Span::styled(
                format!(" {} ", mode),
                Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
        }
        // phase — use host's if present, else fall back to the renderer-derived status
        let phase_str = segs
            .get("phase")
            .cloned()
            .unwrap_or_else(|| status.to_string());
        spans.push(Span::styled(
            phase_str,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
        // Conventional segments in order.
        for key in ["elapsed", "tokens", "cost", "branch"] {
            if let Some(v) = segs.get(key) {
                spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
                spans.push(Span::styled(v.clone(), Style::default().fg(Color::Gray)));
            }
        }
        // warn segment in yellow.
        if let Some(w) = segs.get("warn") {
            spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                w.clone(),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }
        // Unknown segments (anything not in the known set) — alphabetical.
        let known: &[&str] = &["mode", "phase", "elapsed", "tokens", "cost", "branch", "warn"];
        for (k, v) in segs.iter() {
            if !known.contains(&k.as_str()) {
                spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
                spans.push(Span::styled(
                    format!("{}={}", k, v),
                    Style::default().fg(Color::Gray),
                ));
            }
        }
        // Renderer-internal counts at the end (dimmed).
        spans.push(Span::styled(
            format!(
                "  [{}] live={} history={} total={}{}",
                follow,
                live.len(),
                history.len(),
                model.total_rows(),
                indicator,
            ),
            Style::default().fg(Color::DarkGray),
        ));
        let status_line = Paragraph::new(Line::from(spans));
        f.render_widget(status_line, chunks[2]);

        // Input
        let input = Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::default().fg(Color::Cyan)),
            Span::raw(input_buf),
        ]))
        .block(Block::default().borders(Borders::ALL).title("input"));
        f.render_widget(input, chunks[3]);
    })?;
    Ok(())
}

fn row_to_line<'a>(r: &'a camouflage_renderer::Row) -> Line<'a> {
    let (prefix, color) = match r.kind {
        RowKind::System => ("·", Color::DarkGray),
        RowKind::User => ("›", Color::Cyan),
        RowKind::Assistant => (" ", Color::White),
        RowKind::Tool => ("⚙", Color::Magenta),
        RowKind::Error => ("✗", Color::Red),
        RowKind::Marker => ("¶", Color::Blue),
    };
    Line::from(vec![
        Span::styled(format!("{} ", prefix), Style::default().fg(color)),
        Span::raw(r.text.as_str()),
    ])
}

fn short_uuid(s: &str) -> String {
    s.chars().take(8).collect()
}
