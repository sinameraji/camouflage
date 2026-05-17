use anyhow::Result;
use camouflage_renderer::{
    format_elapsed_ms,
    markdown::{parse_inline, InlineStyle},
    RenderModel, RowKind, ViewportState,
};
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Terminal;
use unicode_width::UnicodeWidthStr;

/// Compute the total display width the status line will need to draw all
/// segments — used to decide how many vertical rows to give the status
/// region in the layout. Mirrors the order in which the status row is
/// actually built below.
fn status_total_width(
    model: &RenderModel,
    viewport: &ViewportState,
    status: &str,
) -> usize {
    let segs = model.status_segments();
    let mut w: usize = 0;
    if let Some(mode) = segs.get("mode") {
        // Rendered as " {mode} " (3 + len) plus a trailing space → +1.
        w += 3 + UnicodeWidthStr::width(mode.as_str()) + 1;
    }
    let phase_str = segs
        .get("phase")
        .cloned()
        .unwrap_or_else(|| status.to_string());
    let needs_spinner = matches!(
        phase_str.as_str(),
        "thinking" | "streaming" | "tool" | "running"
    );
    if needs_spinner {
        w += 2; // spinner glyph + space
    }
    w += UnicodeWidthStr::width(phase_str.as_str());
    for key in ["elapsed", "tokens", "cost", "branch"] {
        if let Some(v) = segs.get(key) {
            w += 3; // " · "
            w += UnicodeWidthStr::width(v.as_str());
        }
    }
    if let Some(v) = segs.get("warn") {
        w += 3;
        w += UnicodeWidthStr::width(v.as_str());
    }
    let known: &[&str] = &[
        "mode", "phase", "elapsed", "tokens", "cost", "branch", "warn",
    ];
    for (k, v) in segs.iter() {
        if !known.contains(&k.as_str()) {
            w += 3;
            w += UnicodeWidthStr::width(k.as_str()) + 1 + UnicodeWidthStr::width(v.as_str());
        }
    }
    // Counts + optional indicator.
    let counts = format!(
        "  [{}] live={} history={} total={}",
        if viewport.auto_follow { "follow" } else { "scrolled" },
        model.rows().len(),
        model.history_rows().len(),
        model.total_rows(),
    );
    w += UnicodeWidthStr::width(counts.as_str());
    if !viewport.auto_follow {
        w += UnicodeWidthStr::width(" | new output below ↓");
    }
    w
}

/// Braille-dots spinner frames. Borrowed from KimiFlare's `dots` style.
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn spinner_glyph(frame: u64) -> char {
    SPINNER[(frame as usize) % SPINNER.len()]
}

/// TB3 fix: drive the spinner glyph from wall-clock time, not the
/// per-redraw `frame_counter`. Scrolling causes extra redraws but
/// shouldn't advance the spinner — that was the user-visible bug.
/// 80 ms per glyph ≈ 12.5 fps, comfortable for the eye.
fn spinner_frame_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    ms / 80
}

/// View-time projection of the event inspector. None = panel closed.
pub struct InspectorView<'a> {
    /// Rows-from-newest of the focused row (0 = bottom).
    pub cursor_offset: usize,
    /// Seq of the focused event, if resolved.
    pub focused_seq: Option<i64>,
    /// Pretty-printed JSON of the focused event.
    pub json: &'a str,
}

/// Inline search prompt projection. None = closed.
pub struct SearchView<'a> {
    pub query: &'a str,
}

/// v0.4: live runtime metrics shown in the toggleable overlay.
pub struct MetricsView {
    pub total_events: u64,
    pub events_per_sec: f64,
    pub frame_us: u128,
    pub session_secs: u64,
    pub rows_live: usize,
    pub row_cap: usize,
    pub history_rows: usize,
    pub background_tasks: usize,
}

/// Row-kind filter cycled with `f`. None = show everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowFilterKind {
    Errors,
    Tools,
    Patches,
    Permissions,
}

impl RowFilterKind {
    fn matches(self, r: &camouflage_renderer::Row) -> bool {
        match self {
            Self::Errors => r.kind == RowKind::Error,
            Self::Tools => r.kind == RowKind::Tool,
            // Patches/permissions live in System rows; match by row text
            // prefix produced by RenderModel::apply.
            Self::Patches => {
                r.kind == RowKind::Diff
                    || (r.kind == RowKind::System && r.text.starts_with("patch "))
            }
            Self::Permissions => {
                r.kind == RowKind::System && r.text.starts_with("permission ")
            }
        }
    }
}

pub fn render<B: Backend>(
    terminal: &mut Terminal<B>,
    model: &RenderModel,
    viewport: &ViewportState,
    input_buf: &str,
    status: &str,
    frame: u64,
    inspector: Option<InspectorView<'_>>,
    row_filter: Option<RowFilterKind>,
    search: Option<SearchView<'_>>,
    help_open: bool,
    metrics: Option<MetricsView>,
    theme: &camouflage_renderer::theme::Theme,
    tool_output_open: bool,
    permission_feedback: &str,
    slash_picker_index: usize,
    mention_picker_index: usize,
) -> Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        // Layout regions: header / transcript / task-ribbon (optional) /
        // status / input or permission. Permission widget needs 4 rows
        // (top border + title row + button row + bottom border).
        //
        // Status line grows to 2 or 3 visual lines when host segments don't
        // fit horizontally so the user can see them all. Capped at 3 to
        // protect the transcript from being squeezed on tiny terminals.
        let has_tasks = !model.background_tasks().is_empty();
        let task_line: u16 = if has_tasks { 1 } else { 0 };
        let status_text_width = status_total_width(model, viewport, status);
        let inner_w = area.width.saturating_sub(2).max(1) as usize; // box content width (subtract borders)
        let status_height: u16 = {
            let w = area.width.max(1) as usize;
            (((status_text_width + w - 1) / w).max(1).min(3)) as u16
        };
        // Bottom box height grows to fit wrapped content.
        let bottom_height: u16 = if let Some(pp) = model.pending_permission() {
            // Three logical content lines: title + buttons + feedback. Each
            // may wrap on a narrow terminal; total visual lines = sum.
            let title_w = UnicodeWidthStr::width(" ⚠ permission needed: ")
                + UnicodeWidthStr::width(pp.action.as_str())
                + 4 + UnicodeWidthStr::width(pp.tool.as_str()); // "  (tool)"
            let buttons_w =
                UnicodeWidthStr::width("   [1] allow once   [2] allow for session   [3] deny   [Esc] deny");
            let feedback_w = UnicodeWidthStr::width("   feedback › ")
                + UnicodeWidthStr::width(permission_feedback).max(
                    UnicodeWidthStr::width("(optional — type a reason, sent with your choice)"),
                );
            let title_lines = ((title_w + inner_w - 1) / inner_w).max(1);
            let button_lines = ((buttons_w + inner_w - 1) / inner_w).max(1);
            let feedback_lines = ((feedback_w + inner_w - 1) / inner_w).max(1);
            let content = (title_lines + button_lines + feedback_lines).max(3).min(8) as u16;
            content + 2 // top + bottom border
        } else {
            let input_w = UnicodeWidthStr::width(input_buf).max(1);
            let content_lines = ((input_w + inner_w - 1) / inner_w).max(1).min(3) as u16;
            content_lines + 2 // top + bottom border
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(task_line),
                Constraint::Length(status_height),
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

        // Transcript viewport. Long rows wrap onto multiple visual lines so
        // the full content is visible. To keep auto-follow aligned to the
        // bottom edge we:
        //   1) take a window of logical rows ending at `(total - scroll_offset)`,
        //      generously sized so all visual lines fit
        //   2) precompute the visual line count per row given the current width
        //   3) tell the Paragraph to scroll past the leading visual lines so
        //      the bottom-most row sits flush with the viewport bottom
        let history = model.history_rows();
        let live = model.rows();
        let total_rows = (history.len() + live.len()) as i64;
        let viewport_h = chunks[1].height as usize;
        let transcript_width = chunks[1].width as usize;
        let avail = transcript_width.saturating_sub(2).max(1); // prefix glyph + space
        let end = (total_rows - viewport.scroll_offset).max(0) as usize;
        // Walk backwards from `end` until we've accumulated enough visual
        // lines to fill the viewport (plus a small safety margin).
        let mut rows_taken: Vec<&camouflage_renderer::Row> = Vec::new();
        let mut visual_so_far: usize = 0;
        let combined: Vec<&camouflage_renderer::Row> = history
            .iter()
            .chain(live.iter())
            .take(end)
            .filter(|r| row_filter.map_or(true, |f| f.matches(r)))
            .collect();
        for r in combined.iter().rev() {
            rows_taken.push(*r);
            let text_w = UnicodeWidthStr::width(r.text.as_str()).max(1);
            visual_so_far += (text_w + avail - 1) / avail;
            if visual_so_far >= viewport_h + 4 {
                break;
            }
        }
        rows_taken.reverse();
        let active_tools = model.tools();
        let lines: Vec<Line> = rows_taken
            .iter()
            .map(|r| row_to_line(r, frame, active_tools, theme, model.has_active_stream()))
            .collect();
        // Scroll so the bottom of the wrapped content sits on the bottom of
        // the chunk. `visual_so_far` is the total visual lines of the slice;
        // when it exceeds the viewport, skip the leading ones.
        let scroll_top = visual_so_far.saturating_sub(viewport_h) as u16;
        let transcript = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll_top, 0));

        // When the inspector is open, split chunks[1] horizontally so the
        // transcript shares the row with a JSON detail panel on the right.
        if let Some(insp) = inspector.as_ref() {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(20), Constraint::Length((chunks[1].width / 2).min(60).max(30))])
                .split(chunks[1]);
            f.render_widget(transcript, split[0]);
            // Inspector pane.
            let header_text = match insp.focused_seq {
                Some(seq) => format!("event @ seq {} (offset {})", seq, insp.cursor_offset),
                None => "no event under cursor".to_string(),
            };
            let body = if insp.json.is_empty() {
                "(loading…)".to_string()
            } else {
                insp.json.to_string()
            };
            let panel = Paragraph::new(body)
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(header_text)
                        .border_style(Style::default().fg(Color::Cyan)),
                );
            f.render_widget(panel, split[1]);
        } else {
            f.render_widget(transcript, chunks[1]);
        }

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
                format!("{} ", spinner_glyph(spinner_frame_now())),
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
        // Renderer-internal counts at the end (dimmed). With wrap enabled
        // these naturally flow to the next visual line on narrow terminals
        // rather than being clipped.
        spans.push(Span::styled(
            format!(
                "  [{}] live={} history={} total={}",
                follow,
                live.len(),
                history.len(),
                model.total_rows(),
            ),
            Style::default().fg(Color::DarkGray),
        ));
        if !indicator.is_empty() {
            spans.push(Span::styled(
                indicator.to_string(),
                Style::default().fg(Color::Yellow),
            ));
        }
        // Task ribbon (only if there are any active tasks).
        if has_tasks {
            let mut ribbon_spans: Vec<Span> = Vec::new();
            for (i, task) in model.background_tasks().iter().enumerate() {
                if i > 0 {
                    ribbon_spans.push(Span::styled("  ", Style::default()));
                }
                ribbon_spans.push(Span::styled(
                    spinner_glyph(spinner_frame_now()).to_string(),
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

        let status_line = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false });
        f.render_widget(status_line, chunks[3]);

        // Bottom box: search prompt > permission widget > input prompt.
        let input_chunk = chunks[4];
        if let Some(sv) = search.as_ref() {
            let widget = Paragraph::new(Line::from(vec![
                Span::styled("/", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(sv.query),
                Span::styled(
                    "   (Enter to search · Esc cancel · n/N next/prev)",
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("search")
                    .border_style(Style::default().fg(Color::Cyan)),
            );
            f.render_widget(widget, input_chunk);
        } else if let Some(pp) = model.pending_permission() {
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
                Line::from(vec![
                    Span::styled(
                        "   feedback › ",
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        if permission_feedback.is_empty() {
                            "(optional — type a reason, sent with your choice)".to_string()
                        } else {
                            permission_feedback.to_string()
                        },
                        if permission_feedback.is_empty() {
                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)
                        } else {
                            Style::default().fg(Color::White)
                        },
                    ),
                ]),
            ];
            let widget = Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(
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
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title("input"));
            f.render_widget(input, input_chunk);
        }
        if help_open {
            draw_help_overlay(f, area);
        }
        if let Some(m) = metrics.as_ref() {
            draw_metrics_overlay(f, area, m);
        }
        if tool_output_open {
            draw_tool_output_overlay(f, area, model);
        }
        if input_buf.starts_with('/') && !model.slash_commands().is_empty()
            && model.pending_permission().is_none()
            && search.is_none()
        {
            draw_slash_picker_overlay(f, area, input_buf, model, slash_picker_index);
        }
        if !model.mention_candidates().is_empty()
            && model.pending_permission().is_none()
            && search.is_none()
        {
            if let Some(partial) = mention_picker_partial(input_buf) {
                draw_mention_picker_overlay(f, area, &partial, model, mention_picker_index);
            }
        }
        // CC-1 — SelectList modal renders on top of everything else
        // (above help, metrics, etc.) since it's a focused input modal.
        if let Some(sl) = model.active_select_list() {
            draw_select_list_overlay(f, area, sl, theme);
        }
        // CC-2 — Confirm modal, painted last so it stacks above SelectList
        // if both are somehow active (shouldn't happen in practice).
        if let Some(c) = model.active_confirm() {
            draw_confirm_overlay(f, area, c, theme);
        }
        // CC-3 — toasts in the top-right. Read-only peek; the app loop
        // calls prune_expired_toasts() each tick to drop stale entries.
        let toasts = model.peek_toasts();
        if !toasts.is_empty() {
            draw_toasts(f, area, toasts, theme);
        }
        // CC-6 — Table modal. Painted before SelectList/Confirm in z-order
        // since those are interactive and should sit on top.
        if let Some(t) = model.active_table() {
            draw_table_overlay(f, area, t, theme);
        }
    })?;
    Ok(())
}

fn draw_help_overlay(f: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
    // Centered overlay: 60% width, ~75% height, never smaller than 36×14.
    let w = area.width.saturating_mul(60) / 100;
    let w = w.max(36).min(area.width.saturating_sub(2));
    let h = area.height.saturating_mul(75) / 100;
    let h = h.max(14).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let overlay = ratatui::layout::Rect { x, y, width: w, height: h };

    f.render_widget(Clear, overlay);

    let dim = Style::default().fg(Color::DarkGray);
    let key = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let head = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let rows: Vec<Line> = vec![
        Line::from(vec![Span::styled("Navigation", head)]),
        kv("↑/↓ · PgUp/PgDn · Home/End", "scroll", key, dim),
        kv("Ctrl+E", "jump to latest", key, dim),
        Line::from(""),
        Line::from(vec![Span::styled("Inspection (v0.2)", head)]),
        kv("i", "toggle event-inspector panel", key, dim),
        kv("f", "cycle row-kind filter", key, dim),
        kv("Ctrl+F", "open inline search", key, dim),
        kv("n / N", "next / prev search match", key, dim),
        kv("m", "bookmark current focus", key, dim),
        kv("'", "cycle to next bookmark", key, dim),
        Line::from(""),
        Line::from(vec![Span::styled("Replay (--replay)", head)]),
        kv("Space", "toggle play / pause", key, dim),
        kv("→ / ←", "step forward / backward 1 event", key, dim),
        kv("+ / -", "adjust replay speed (0.25× – 64×)", key, dim),
        kv("0", "restart replay", key, dim),
        Line::from(""),
        Line::from(vec![Span::styled("Other", head)]),
        kv("Enter", "submit input", key, dim),
        kv("Esc", "cancel active stream", key, dim),
        kv("1 / 2 / 3", "permission: allow once / session / deny", key, dim),
        kv("r", "reload (in --replay mode: restart)", key, dim),
        kv("M", "toggle live-metrics overlay", key, dim),
        kv("T", "cycle theme", key, dim),
        kv("X", "tool-output overlay (most recent)", key, dim),
        kv("q · Ctrl+C", "quit", key, dim),
        Line::from(""),
        Line::from(vec![
            Span::styled("?", key),
            Span::styled(" again to close", dim),
        ]),
    ];

    let widget = Paragraph::new(rows)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" help ")
                .border_style(Style::default().fg(Color::Yellow)),
        );
    f.render_widget(widget, overlay);
}

/// Mirror of the helper in app.rs — kept private so draw doesn't depend
/// on app internals. Returns the partial text after the last `@` if the
/// rest of the input is whitespace-free.
fn mention_picker_partial(buf: &str) -> Option<String> {
    let at = buf.rfind('@')?;
    let tail = &buf[at + 1..];
    if tail.contains(char::is_whitespace) {
        return None;
    }
    Some(tail.to_string())
}

fn draw_mention_picker_overlay(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    partial: &str,
    model: &RenderModel,
    selected: usize,
) {
    let matches: Vec<&camouflage_renderer::model::MentionEntry> = model
        .mention_candidates()
        .iter()
        .filter(|m| m.token.contains(partial))
        .collect();
    if matches.is_empty() {
        return;
    }
    let visible = matches.len().min(8);
    let w: u16 = 60;
    let h: u16 = (visible as u16) + 2;
    if area.width < w + 2 || area.height < h + 6 {
        return;
    }
    let bottom_height: u16 = 5;
    let x = area.x + 2;
    let y = area.y.saturating_add(area.height).saturating_sub(bottom_height).saturating_sub(h);
    let overlay = ratatui::layout::Rect { x, y, width: w, height: h };
    f.render_widget(Clear, overlay);

    let dim = Style::default().fg(Color::DarkGray);
    let sel = Style::default().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD);
    let plain = Style::default().fg(Color::White);

    let sel_idx = selected.min(matches.len() - 1);
    let lines: Vec<Line> = matches
        .iter()
        .take(visible)
        .enumerate()
        .map(|(i, m)| {
            let style = if i == sel_idx { sel } else { plain };
            let kind = m.kind.as_deref().unwrap_or("");
            let label = m.label.as_deref().unwrap_or("");
            Line::from(vec![
                Span::styled(format!(" @{}", m.token), style),
                Span::styled(
                    if kind.is_empty() { "  ".to_string() } else { format!("  [{}]  ", kind) },
                    dim,
                ),
                Span::styled(label.to_string(), dim),
            ])
        })
        .collect();

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" mentions  ↑/↓ select  Enter insert ")
                .border_style(Style::default().fg(Color::Magenta)),
        );
    f.render_widget(widget, overlay);
}

fn draw_slash_picker_overlay(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    input_buf: &str,
    model: &RenderModel,
    selected: usize,
) {
    let needle: String = input_buf.chars().skip(1).collect();
    let matches: Vec<&camouflage_renderer::model::SlashCmdEntry> = model
        .slash_commands()
        .iter()
        .filter(|c| c.name.starts_with(&needle))
        .collect();
    if matches.is_empty() {
        return;
    }
    let visible = matches.len().min(8);
    let w: u16 = 50;
    let h: u16 = (visible as u16) + 2; // entries + border
    if area.width < w + 2 || area.height < h + 6 {
        return;
    }
    // Anchor above the input box (which is the bottom 3+ rows).
    let bottom_height: u16 = 5; // approximate; place above the input
    let x = area.x + 2;
    let y = area.y.saturating_add(area.height).saturating_sub(bottom_height).saturating_sub(h);
    let overlay = ratatui::layout::Rect { x, y, width: w, height: h };
    f.render_widget(Clear, overlay);

    let dim = Style::default().fg(Color::DarkGray);
    let sel = Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD);
    let plain = Style::default().fg(Color::White);

    let sel_idx = selected.min(matches.len() - 1);
    let lines: Vec<Line> = matches
        .iter()
        .take(visible)
        .enumerate()
        .map(|(i, c)| {
            let style = if i == sel_idx { sel } else { plain };
            let hint = c.args_hint.as_deref().unwrap_or("");
            Line::from(vec![
                Span::styled(format!(" /{}", c.name), style),
                Span::styled(if hint.is_empty() { "  ".to_string() } else { format!(" {}  ", hint) }, dim),
                Span::styled(c.description.clone(), dim),
            ])
        })
        .collect();

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" slash commands  ↑/↓ select  Enter insert ")
                .border_style(Style::default().fg(Color::Cyan)),
        );
    f.render_widget(widget, overlay);
}

fn draw_tool_output_overlay(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    model: &RenderModel,
) {
    // Find the most-recent tool by started_ms. If there are no tools yet,
    // show a placeholder.
    let recent = model
        .tools()
        .values()
        .max_by_key(|t| t.started_ms)
        .cloned();
    let w = area.width.saturating_mul(80) / 100;
    let w = w.max(40).min(area.width.saturating_sub(2));
    let h = area.height.saturating_mul(80) / 100;
    let h = h.max(10).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let overlay = ratatui::layout::Rect { x, y, width: w, height: h };
    f.render_widget(Clear, overlay);

    let dim = Style::default().fg(Color::DarkGray);
    let head = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let val = Style::default().fg(Color::White);
    let err = Style::default().fg(Color::Red);

    let mut lines: Vec<Line> = Vec::new();
    match recent {
        None => {
            lines.push(Line::from(vec![Span::styled(
                "no tool executions in this session yet",
                dim,
            )]));
        }
        Some(t) => {
            lines.push(Line::from(vec![
                Span::styled(format!("{}", t.tool), head),
                Span::raw("  "),
                Span::styled(format!("{}", t.command), dim),
            ]));
            let finished = if t.finished {
                format!("exit={}", t.exit_code.unwrap_or(0))
            } else {
                "(running)".to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(finished, dim),
                Span::raw("  "),
                Span::styled(format!("stdout={}B  stderr={}B", t.stdout_bytes, t.stderr_bytes), dim),
            ]));
            lines.push(Line::from(""));
            if !t.recent_stdout.is_empty() {
                lines.push(Line::from(vec![Span::styled("─ stdout ─", head)]));
                for raw_line in t.recent_stdout.lines() {
                    lines.push(Line::from(vec![Span::styled(raw_line.to_string(), val)]));
                }
            }
            if !t.recent_stderr.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled("─ stderr ─", head)]));
                for raw_line in t.recent_stderr.lines() {
                    lines.push(Line::from(vec![Span::styled(raw_line.to_string(), err)]));
                }
            }
            if t.recent_stdout.is_empty() && t.recent_stderr.is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    "(no captured output yet)",
                    dim,
                )]));
            }
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("X", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(" again to close", dim),
    ]));

    let widget = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" tool output (most recent) ")
                .border_style(Style::default().fg(Color::Yellow)),
        );
    f.render_widget(widget, overlay);
}

fn draw_table_overlay(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    t: &camouflage_renderer::model::TableState,
    theme: &camouflage_renderer::theme::Theme,
) {
    use camouflage_renderer::model::TableAlign;
    // Column widths: max(header, max cell) per column, capped to 30 chars.
    let widths: Vec<usize> = t
        .columns
        .iter()
        .enumerate()
        .map(|(ci, col)| {
            let header_w = UnicodeWidthStr::width(col.label.as_str());
            let cell_w = t
                .rows
                .iter()
                .map(|r| r.get(ci).map(|s| UnicodeWidthStr::width(s.as_str())).unwrap_or(0))
                .max()
                .unwrap_or(0);
            header_w.max(cell_w).min(30)
        })
        .collect();
    let total_w: usize = widths.iter().sum::<usize>() + (widths.len().saturating_sub(1)) * 3 + 4;
    let row_count = t.rows.len().min(20);
    let h: u16 = (row_count as u16) + 4 + if t.title.is_some() { 1 } else { 0 }; // border + header + sep + rows + hint
    let w = (total_w as u16).max(40).min(area.width.saturating_sub(2));
    if area.width < w + 2 || area.height < h + 2 {
        return;
    }
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let overlay = ratatui::layout::Rect { x, y, width: w, height: h };
    f.render_widget(Clear, overlay);

    let dim = Style::default().fg(Color::DarkGray);
    let head = Style::default()
        .fg(rgb_to_color(theme.accent))
        .add_modifier(Modifier::BOLD);
    let cell_style = Style::default().fg(rgb_to_color(theme.assistant));

    let mut lines: Vec<Line> = Vec::new();
    if let Some(title) = t.title.as_deref() {
        lines.push(Line::from(vec![Span::styled(format!(" {}", title), head)]));
    }
    // Header row
    let mut header_spans: Vec<Span> = vec![Span::raw(" ")];
    for (i, col) in t.columns.iter().enumerate() {
        let cell = pad_cell(&col.label, widths[i], col.align);
        header_spans.push(Span::styled(cell, head));
        if i + 1 < t.columns.len() {
            header_spans.push(Span::styled("  │  ", dim));
        }
    }
    lines.push(Line::from(header_spans));
    lines.push(Line::from(vec![Span::styled(
        "─".repeat((w.saturating_sub(2)) as usize),
        dim,
    )]));
    for row in t.rows.iter().take(row_count) {
        let mut spans: Vec<Span> = vec![Span::raw(" ")];
        for (i, col) in t.columns.iter().enumerate() {
            let raw = row.get(i).cloned().unwrap_or_default();
            let cell = pad_cell(&raw, widths[i], col.align);
            spans.push(Span::styled(cell, cell_style));
            if i + 1 < t.columns.len() {
                spans.push(Span::styled("  │  ", dim));
            }
        }
        lines.push(Line::from(spans));
    }
    if t.rows.len() > row_count {
        lines.push(Line::from(vec![Span::styled(
            format!(" … {} more rows (truncated)", t.rows.len() - row_count),
            dim,
        )]));
    }
    lines.push(Line::from(vec![Span::styled("Esc to close", dim)]));

    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" table ")
            .border_style(Style::default().fg(rgb_to_color(theme.overlay_border))),
    );
    f.render_widget(widget, overlay);
}

/// Truncate-or-pad `s` to exactly `width` visible columns according to alignment.
fn pad_cell(s: &str, width: usize, align: camouflage_renderer::model::TableAlign) -> String {
    use camouflage_renderer::model::TableAlign;
    let sw = UnicodeWidthStr::width(s);
    if sw >= width {
        // Truncate. Walk char boundaries.
        let mut out = String::with_capacity(width);
        let mut acc = 0;
        for ch in s.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if acc + cw > width { break; }
            out.push(ch);
            acc += cw;
        }
        return out;
    }
    let pad = width - sw;
    match align {
        TableAlign::Left => format!("{}{}", s, " ".repeat(pad)),
        TableAlign::Right => format!("{}{}", " ".repeat(pad), s),
        TableAlign::Center => {
            let l = pad / 2;
            let r = pad - l;
            format!("{}{}{}", " ".repeat(l), s, " ".repeat(r))
        }
    }
}

fn draw_toasts(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    toasts: &[camouflage_renderer::model::ToastState],
    theme: &camouflage_renderer::theme::Theme,
) {
    use camouflage_renderer::model::ToastKind;
    // Newest toasts at the top of the stack. Take up to 5 to avoid
    // dominating the screen.
    let visible: Vec<_> = toasts.iter().rev().take(5).collect();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut next_y = area.y + 1;
    for t in visible {
        if t.expires_ms <= now_ms { continue; }
        let max_w = area.width.saturating_sub(4).min(60).max(20);
        let text_w = UnicodeWidthStr::width(t.text.as_str()) as u16;
        let inner_w = text_w.min(max_w.saturating_sub(4));
        let w = inner_w + 4;
        if next_y + 3 > area.y + area.height { break; }
        if w > area.width { continue; }
        let x = area.x + area.width.saturating_sub(w + 1);
        let rect = ratatui::layout::Rect { x, y: next_y, width: w, height: 3 };
        f.render_widget(Clear, rect);
        let (label, color) = match t.kind {
            ToastKind::Info    => (" i ", rgb_to_color(theme.accent)),
            ToastKind::Success => (" ✓ ", rgb_to_color(theme.diff_add)),
            ToastKind::Warn    => (" ! ", rgb_to_color(theme.spinner)),
            ToastKind::Error   => (" ✗ ", rgb_to_color(theme.error)),
        };
        let widget = Paragraph::new(Line::from(vec![
            Span::styled(label, Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(t.text.as_str(), Style::default().fg(rgb_to_color(theme.assistant))),
        ]))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color)),
        );
        f.render_widget(widget, rect);
        next_y += 3;
    }
}

fn draw_confirm_overlay(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    c: &camouflage_renderer::model::ConfirmState,
    theme: &camouflage_renderer::theme::Theme,
) {
    let prompt_w = UnicodeWidthStr::width(c.prompt.as_str()) as u16;
    let yes_w = UnicodeWidthStr::width(c.yes_label.as_str()) as u16;
    let no_w = UnicodeWidthStr::width(c.no_label.as_str()) as u16;
    // " [Y] {yes} (y)   [N] {no} (n) " — approximate width
    let buttons_w = yes_w + no_w + 24;
    let w = prompt_w.max(buttons_w).saturating_add(4).max(36).min(area.width.saturating_sub(2));
    let h: u16 = 5; // border + prompt + spacer + buttons + hint
    if area.width < w + 2 || area.height < h + 2 {
        return;
    }
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let overlay = ratatui::layout::Rect { x, y, width: w, height: h };
    f.render_widget(Clear, overlay);

    let dim = Style::default().fg(Color::DarkGray);
    let head = Style::default()
        .fg(rgb_to_color(theme.accent))
        .add_modifier(Modifier::BOLD);
    let sel = Style::default()
        .fg(Color::Black)
        .bg(rgb_to_color(theme.accent))
        .add_modifier(Modifier::BOLD);
    let unsel = Style::default().fg(rgb_to_color(theme.assistant));

    let yes_style = if c.selected_yes { sel } else { unsel };
    let no_style = if c.selected_yes { unsel } else { sel };

    let hint = if c.allow_cancel {
        "y/n · ←/→ toggle · Enter confirm · Esc cancel"
    } else {
        "y/n · ←/→ toggle · Enter confirm"
    };

    let lines: Vec<Line> = vec![
        Line::from(vec![Span::styled(format!(" {}", c.prompt), head)]),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            Span::styled(format!(" {} ", c.yes_label), yes_style),
            Span::styled("  (y)   ", dim),
            Span::styled(format!(" {} ", c.no_label), no_style),
            Span::styled("  (n)", dim),
        ]),
        Line::from(vec![Span::styled(hint, dim)]),
    ];

    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" confirm ")
            .border_style(Style::default().fg(rgb_to_color(theme.overlay_border))),
    );
    f.render_widget(widget, overlay);
}

fn draw_select_list_overlay(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    sl: &camouflage_renderer::model::SelectListState,
    theme: &camouflage_renderer::theme::Theme,
) {
    let visible = sl.filtered_indices();
    let entry_count = visible.len().min(12); // cap visible entries; longer lists scroll
    // Compute width from the longest visible label so the modal doesn't
    // hug too narrowly.
    let max_label = visible
        .iter()
        .map(|&i| {
            let e = &sl.options[i];
            UnicodeWidthStr::width(e.label.as_str())
                + e.description
                    .as_deref()
                    .map(|d| UnicodeWidthStr::width(d) + 3)
                    .unwrap_or(0)
        })
        .max()
        .unwrap_or(20);
    let prompt_w = UnicodeWidthStr::width(sl.prompt.as_str());
    let mut w = (max_label + 6).max(prompt_w + 4).max(40) as u16;
    w = w.min(area.width.saturating_sub(2));
    let h: u16 = (entry_count as u16)
        + 2  // top/bottom border
        + 1  // prompt row
        + 1  // separator
        + if sl.allow_filter { 2 } else { 0 } // filter row + separator
        + 1; // hints row
    if area.width < w + 2 || area.height < h + 2 {
        return;
    }
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let overlay = ratatui::layout::Rect { x, y, width: w, height: h };
    f.render_widget(Clear, overlay);

    let dim = Style::default().fg(Color::DarkGray);
    let plain = Style::default().fg(rgb_to_color(theme.assistant));
    let head = Style::default()
        .fg(rgb_to_color(theme.accent))
        .add_modifier(Modifier::BOLD);
    let sel_bg = rgb_to_color(theme.accent);
    let sel_fg = Color::Black;
    let sel = Style::default().fg(sel_fg).bg(sel_bg).add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![Span::styled(format!(" {}", sl.prompt), head)]));
    if sl.allow_filter {
        let filter_hint = if sl.filter.is_empty() {
            Span::styled("(type to filter)", dim)
        } else {
            Span::styled(sl.filter.as_str(), plain)
        };
        lines.push(Line::from(vec![
            Span::styled(" filter › ", dim),
            filter_hint,
        ]));
    }
    lines.push(Line::from(vec![Span::styled(
        "─".repeat((w.saturating_sub(2)) as usize),
        dim,
    )]));
    for &i in visible.iter().take(entry_count) {
        let entry = &sl.options[i];
        let style = if i == sl.selected { sel } else { plain };
        let mut spans = vec![
            Span::styled(if i == sl.selected { " ▸ " } else { "   " }, style),
            Span::styled(entry.label.as_str(), style),
        ];
        if let Some(desc) = entry.description.as_deref() {
            spans.push(Span::styled(format!("  {}", desc), dim));
        }
        lines.push(Line::from(spans));
    }
    let hint = if sl.allow_cancel {
        "↑/↓ navigate · Enter select · Esc cancel"
    } else {
        "↑/↓ navigate · Enter select"
    };
    lines.push(Line::from(vec![Span::styled(hint, dim)]));

    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" select ")
            .border_style(Style::default().fg(rgb_to_color(theme.overlay_border))),
    );
    f.render_widget(widget, overlay);
}

fn draw_metrics_overlay(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    m: &MetricsView,
) {
    // Small top-right panel. Doesn't take focus; just informational.
    let w: u16 = 36;
    let h: u16 = 11;
    if area.width < w + 2 || area.height < h + 2 {
        return;
    }
    let overlay = ratatui::layout::Rect {
        x: area.x + area.width.saturating_sub(w + 1),
        y: area.y + 1,
        width: w,
        height: h,
    };
    f.render_widget(Clear, overlay);
    let dim = Style::default().fg(Color::DarkGray);
    let val = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
    let cap_pct = if m.row_cap == 0 { 0 } else { m.rows_live * 100 / m.row_cap };
    let frame_ms = m.frame_us as f64 / 1000.0;
    let rows = vec![
        Line::from(vec![Span::styled("session  ", dim), Span::styled(format!("{}s", m.session_secs), val)]),
        Line::from(vec![Span::styled("events   ", dim), Span::styled(format!("{}", m.total_events), val)]),
        Line::from(vec![Span::styled("rate     ", dim), Span::styled(format!("{:.1}/s", m.events_per_sec), val)]),
        Line::from(vec![Span::styled("frame    ", dim), Span::styled(format!("{:.2}ms", frame_ms), val)]),
        Line::from(vec![Span::styled("rows     ", dim), Span::styled(format!("{}/{} ({}%)", m.rows_live, m.row_cap, cap_pct), val)]),
        Line::from(vec![Span::styled("history  ", dim), Span::styled(format!("{}", m.history_rows), val)]),
        Line::from(vec![Span::styled("tasks    ", dim), Span::styled(format!("{}", m.background_tasks), val)]),
        Line::from(""),
        Line::from(vec![Span::styled("M", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), Span::styled(" again to close", dim)]),
    ];
    let widget = Paragraph::new(rows)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" metrics ")
                .border_style(Style::default().fg(Color::Cyan)),
        );
    f.render_widget(widget, overlay);
}

fn kv<'a>(k: &'a str, desc: &'a str, key_style: Style, dim: Style) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(k, key_style),
        Span::raw("  "),
        Span::styled(desc, dim),
    ])
}

fn row_to_line<'a>(
    r: &'a camouflage_renderer::Row,
    frame: u64,
    active_tools: &std::collections::HashMap<String, camouflage_renderer::ToolState>,
    theme: &camouflage_renderer::theme::Theme,
    has_active_stream: bool,
) -> Line<'a> {
    let spinner_color = rgb_to_color(theme.spinner);
    let user_color = rgb_to_color(theme.user);
    let assistant_color = rgb_to_color(theme.assistant);
    let tool_color = rgb_to_color(theme.tool);
    let error_color = rgb_to_color(theme.error);
    let system_color = rgb_to_color(theme.system);
    let marker_color = rgb_to_color(theme.marker);
    let diff_add = rgb_to_color(theme.diff_add);
    let diff_remove = rgb_to_color(theme.diff_remove);
    let diff_hunk = rgb_to_color(theme.diff_hunk);
    // Assistant row that's open (active stream) but has no text yet → spinner only.
    // Tool row whose ToolState is not yet finished → spinner instead of ✓.
    let (prefix, color) = match r.kind {
        RowKind::System => ("·".to_string(), system_color),
        RowKind::User => ("›".to_string(), user_color),
        RowKind::Assistant => {
            // Only show the spinner on an empty assistant row when a stream
            // is actually in flight. Without this check, completed-but-
            // empty rows kept spinning forever in idle sessions.
            if r.text.is_empty() && has_active_stream {
                (spinner_glyph(spinner_frame_now()).to_string(), spinner_color)
            } else {
                (" ".to_string(), assistant_color)
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
                (spinner_glyph(spinner_frame_now()).to_string(), spinner_color)
            } else {
                ("⚙".to_string(), tool_color)
            }
        }
        RowKind::Error => ("✗".to_string(), error_color),
        RowKind::Marker => ("¶".to_string(), marker_color),
        RowKind::Diff => {
            let marker = r.text.chars().next().unwrap_or(' ');
            let (glyph, c) = match marker {
                '+' => (" ", diff_add),
                '-' => (" ", diff_remove),
                '@' => (" ", diff_hunk),
                _ => (" ", system_color),
            };
            (glyph.to_string(), c)
        }
    };
    // For an in-flight tool row, append the running elapsed time so the
    // user can see how long the tool has been executing.
    let mut spans: Vec<Span> = vec![
        Span::styled(format!("{} ", prefix), Style::default().fg(color)),
    ];
    if r.kind == RowKind::Assistant && !r.text.is_empty() {
        let code_fg = rgb_to_color(theme.code_fg);
        let code_bg = rgb_to_color(theme.code_bg);
        for sp in parse_inline(&r.text) {
            let style = match sp.style {
                InlineStyle::Plain => Style::default().fg(assistant_color),
                InlineStyle::Bold => Style::default()
                    .fg(assistant_color)
                    .add_modifier(Modifier::BOLD),
                InlineStyle::Italic => Style::default()
                    .fg(assistant_color)
                    .add_modifier(Modifier::ITALIC),
                InlineStyle::Code => Style::default().fg(code_fg).bg(code_bg),
            };
            spans.push(Span::styled(sp.text, style));
        }
    } else if r.kind == RowKind::Diff {
        spans.push(Span::styled(r.text.as_str(), Style::default().fg(color)));
    } else {
        spans.push(Span::raw(r.text.as_str()));
    }
    if r.kind == RowKind::Tool {
        if let Some(state) = r.tool_id.as_ref().and_then(|tid| active_tools.get(tid)) {
            if !state.finished {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let elapsed = (now_ms - state.started_ms).max(0);
                spans.push(Span::styled(
                    format!(" ({})", format_elapsed_ms(elapsed)),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
    }
    Line::from(spans)
}

fn rgb_to_color(rgb: camouflage_renderer::theme::Rgb) -> Color {
    Color::Rgb(rgb.0, rgb.1, rgb.2)
}

fn short_uuid(s: &str) -> String {
    s.chars().take(8).collect()
}
