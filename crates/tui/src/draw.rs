use anyhow::Result;
use camouflage_renderer::{RenderModel, RowKind, ViewportState};
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;

/// Braille-dots spinner frames. Borrowed from KimiFlare's `dots` style.
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn spinner_glyph(frame: u64) -> char {
    SPINNER[(frame as usize) % SPINNER.len()]
}

pub fn render<B: Backend>(
    terminal: &mut Terminal<B>,
    model: &RenderModel,
    viewport: &ViewportState,
    input_buf: &str,
    status: &str,
    frame: u64,
) -> Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        // Layout regions: header / transcript / task-ribbon (optional) /
        // status / input or permission. Permission widget needs 4 rows
        // (top border + title row + button row + bottom border).
        let has_tasks = !model.background_tasks().is_empty();
        let task_line: u16 = if has_tasks { 1 } else { 0 };
        let bottom_height: u16 = if model.pending_permission().is_some() { 4 } else { 3 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(task_line),
                Constraint::Length(1),
                Constraint::Length(bottom_height),
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
        let active_tools = model.tools();
        let lines: Vec<Line> = combined
            .skip(start as usize)
            .take((end - start) as usize)
            .map(|r| row_to_line(r, frame, active_tools))
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
        // phase — use host's if present, else fall back to the renderer-derived status.
        // Prepend a spinner glyph when the phase indicates work-in-flight.
        let phase_str = segs
            .get("phase")
            .cloned()
            .unwrap_or_else(|| status.to_string());
        let needs_spinner = matches!(
            phase_str.as_str(),
            "thinking" | "streaming" | "tool" | "running"
        );
        if needs_spinner {
            spans.push(Span::styled(
                format!("{} ", spinner_glyph(frame)),
                Style::default().fg(Color::Yellow),
            ));
        }
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
        // Task ribbon (only if there are any active tasks).
        if has_tasks {
            let mut ribbon_spans: Vec<Span> = Vec::new();
            for (i, task) in model.background_tasks().iter().enumerate() {
                if i > 0 {
                    ribbon_spans.push(Span::styled("  ", Style::default()));
                }
                ribbon_spans.push(Span::styled(
                    spinner_glyph(frame).to_string(),
                    Style::default().fg(Color::Cyan),
                ));
                ribbon_spans.push(Span::raw(" "));
                let mut label = task.label.clone();
                if let Some(p) = task.progress {
                    let pct = (p.clamp(0.0, 1.0) * 100.0) as u32;
                    label.push_str(&format!(" {}%", pct));
                }
                ribbon_spans.push(Span::styled(label, Style::default().fg(Color::Gray)));
            }
            let ribbon = Paragraph::new(Line::from(ribbon_spans));
            f.render_widget(ribbon, chunks[2]);
        }

        let status_line = Paragraph::new(Line::from(spans));
        f.render_widget(status_line, chunks[3]);

        // Bottom box: either the input prompt or the pending-permission widget.
        let input_chunk = chunks[4];
        if let Some(pp) = model.pending_permission() {
            let lines: Vec<Line> = vec![
                Line::from(vec![
                    Span::styled(
                        format!(" ⚠ permission needed: "),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(pp.action.clone(), Style::default().fg(Color::White)),
                    Span::styled(
                        format!("  ({})", pp.tool),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("   [1]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw(" allow once   "),
                    Span::styled("[2]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw(" allow for session   "),
                    Span::styled("[3]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::raw(" deny   "),
                    Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
                    Span::raw(" deny"),
                ]),
            ];
            let widget = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("permission")
                    .border_style(Style::default().fg(Color::Yellow)),
            );
            f.render_widget(widget, input_chunk);
        } else {
            let input = Paragraph::new(Line::from(vec![
                Span::styled("› ", Style::default().fg(Color::Cyan)),
                Span::raw(input_buf),
            ]))
            .block(Block::default().borders(Borders::ALL).title("input"));
            f.render_widget(input, input_chunk);
        }
    })?;
    Ok(())
}

fn row_to_line<'a>(
    r: &'a camouflage_renderer::Row,
    frame: u64,
    active_tools: &std::collections::HashMap<String, camouflage_renderer::ToolState>,
) -> Line<'a> {
    // Assistant row that's open (active stream) but has no text yet → spinner only.
    // Tool row whose ToolState is not yet finished → spinner instead of ✓.
    let (prefix, color) = match r.kind {
        RowKind::System => ("·".to_string(), Color::DarkGray),
        RowKind::User => ("›".to_string(), Color::Cyan),
        RowKind::Assistant => {
            if r.text.is_empty() {
                (spinner_glyph(frame).to_string(), Color::Yellow)
            } else {
                (" ".to_string(), Color::White)
            }
        }
        RowKind::Tool => {
            let unfinished = r
                .tool_id
                .as_ref()
                .and_then(|tid| active_tools.get(tid))
                .map(|st| !st.finished)
                .unwrap_or(false);
            if unfinished {
                (spinner_glyph(frame).to_string(), Color::Yellow)
            } else {
                ("⚙".to_string(), Color::Magenta)
            }
        }
        RowKind::Error => ("✗".to_string(), Color::Red),
        RowKind::Marker => ("¶".to_string(), Color::Blue),
    };
    Line::from(vec![
        Span::styled(format!("{} ", prefix), Style::default().fg(color)),
        Span::raw(r.text.as_str()),
    ])
}

fn short_uuid(s: &str) -> String {
    s.chars().take(8).collect()
}
