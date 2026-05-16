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

        // Transcript viewport — virtualized: pick a window of rows from the
        // bounded model based on scroll offset.
        let rows = model.rows();
        let total = rows.len() as i64;
        let height = chunks[1].height as i64;
        let end = (total - viewport.scroll_offset).max(0);
        let start = (end - height).max(0);
        let lines: Vec<Line> = rows
            .iter()
            .skip(start as usize)
            .take((end - start) as usize)
            .map(|r| row_to_line(r))
            .collect();
        let transcript = Paragraph::new(lines);
        f.render_widget(transcript, chunks[1]);

        // Status line
        let follow = if viewport.auto_follow { "follow" } else { "scrolled" };
        let indicator = if !viewport.auto_follow {
            " | new output below ↓"
        } else {
            ""
        };
        let status_line = Paragraph::new(Line::from(vec![
            Span::styled(format!(" {} ", status), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("[{}] rows={} total={}{}",
                    follow,
                    rows.len(),
                    model.total_rows(),
                    indicator,
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
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
