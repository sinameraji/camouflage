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
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};
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
    // Only the scrolled-indicator now contributes to the right side; the
    // debug counts (live/history/total) moved to the metrics overlay.
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
    viewport: &mut ViewportState,
    input_buf: &str,
    input_cursor: usize,
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
    slash_sub_state: Option<&(String, Vec<String>, bool, usize)>,
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
            if pp.help_open {
                // Help overlay: 1 title + 6 rows + 1 footer = 8 + borders
                10u16
            } else {
                // title + (diff preview if present) + 3 option rows + feedback + hints
                let base: u16 = 1 + 3 + 1 + 1; // title + options + feedback + hints
                let diff_lines: u16 = pp.diff.as_ref()
                    .map(|d| {
                        let preview = unified_diff_lines(&d.before, &d.after, 12).len() as u16;
                        // +1 for the "── path ──" header
                        preview + 1
                    })
                    .unwrap_or(0);
                (base + diff_lines).min(20) + 2
            }
        } else {
            let input_w = UnicodeWidthStr::width(input_buf).max(1);
            let content_lines = ((input_w + inner_w - 1) / inner_w).max(1).min(3) as u16;
            content_lines + 2 // top + bottom border
        };
        // Layout regions in order: header / transcript / spacer /
        // task ribbon (optional) / status / input. The 1-row spacer
        // between the transcript and the status row gives the last
        // line of streamed content visual breathing room from the
        // status bar — without it, a fully-filled viewport puts text
        // flush against the next chunk and reads as "cut off".
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1), // bottom spacer
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
        // When the viewport is "frozen" at a scroll position, anchor the
        // window to the snapshot taken at scroll-time so new streaming
        // rows don't push the user's view.
        // Per-row visual line count (post-wrap). Used both for visual
        // extent computation and the windowed render below.
        let visual_count = |r: &camouflage_renderer::Row| -> usize {
            if r.kind == RowKind::Assistant && r.text.contains('\n') {
                r.text
                    .split('\n')
                    .map(|seg| {
                        let w = UnicodeWidthStr::width(seg).max(1);
                        (w + avail - 1) / avail
                    })
                    .sum()
            } else {
                let w = UnicodeWidthStr::width(r.text.as_str()).max(1);
                (w + avail - 1) / avail
            }
        };
        let combined_all: Vec<&camouflage_renderer::Row> = history
            .iter()
            .chain(live.iter())
            .filter(|r| row_filter.map_or(true, |f| f.matches(r)))
            .collect();
        // Total visual lines for the *whole* (filtered) transcript. The
        // previous implementation truncated to `anchor` rows here, which
        // is what made the user's scroll "stuck": once frozen at e.g.
        // row 3 the viewport could never see anything past row 3 even
        // as 25 more streamed in below.
        let total_visual_now: i64 = combined_all.iter().map(|r| visual_count(r) as i64).sum();
        // Push the latest total_visual back to the viewport so the
        // input loop's `scroll_up` knows where to freeze on the *next*
        // press, and so input-history fetches can detect "near top".
        viewport.last_total_visual = total_visual_now;
        // Effective bottom-of-viewport visual line. With auto-follow,
        // it's the live tail. While frozen, it's the snapshot minus
        // however far the user has scrolled up. scroll_offset can go
        // negative — that's the user scrolling *down past* the frozen
        // bottom into content that streamed in during the freeze.
        let effective_bottom: i64 = match viewport.frozen_visual_bottom {
            Some(f) => (f - viewport.scroll_offset)
                .max(viewport_h as i64)
                .min(total_visual_now),
            None => total_visual_now,
        };
        // Unfreeze + resume auto-follow once the view actually catches
        // up to the live tail (the user has scrolled all the way down
        // through content that arrived during the freeze). Doing it
        // here, not in scroll_down, avoids the "page jump" the user
        // saw: the gap between the frozen bottom and the live tail is
        // traversed line-by-line, not in a single snap.
        if let Some(f) = viewport.frozen_visual_bottom {
            if (f - viewport.scroll_offset) >= total_visual_now {
                viewport.frozen_visual_bottom = None;
                viewport.scroll_offset = 0;
                viewport.auto_follow = true;
            }
            // Re-clamp the offset against the actual top of the
            // transcript so PageUp past the start doesn't pile up
            // phantom offset (which would force equal-and-opposite
            // PageDowns to recover).
            let max_off = (f - viewport_h as i64).max(0);
            if viewport.scroll_offset > max_off {
                viewport.scroll_offset = max_off;
            }
        }
        // Walk backward through the row list accumulating visual lines
        // until we have rows that cover the visible window. We need to
        // track *where* the bottommost pushed row ends within the
        // transcript so the Paragraph scroll math doesn't drift.
        let mut rows_taken_rev: Vec<&camouflage_renderer::Row> = Vec::new();
        let below_visible = (total_visual_now - effective_bottom).max(0);
        let needed_above_bottom = viewport_h as i64 + 4;
        let mut cum_from_end: i64 = 0;
        // Transcript line index of the bottom of the slice (1-indexed
        // visual line of the bottommost cell of the bottommost row we
        // included). Computed when we push the first row.
        let mut slice_bottom_line: i64 = effective_bottom;
        for r in combined_all.iter().rev() {
            let prev_cum = cum_from_end;
            cum_from_end += visual_count(r) as i64;
            if cum_from_end > below_visible {
                if rows_taken_rev.is_empty() {
                    // Row's bottom transcript line = total_visual - prev_cum.
                    slice_bottom_line = total_visual_now - prev_cum;
                }
                rows_taken_rev.push(*r);
            }
            if cum_from_end >= below_visible + needed_above_bottom {
                break;
            }
        }
        let mut rows_taken = rows_taken_rev;
        rows_taken.reverse();
        let active_tools = model.tools();
        let lines: Vec<Line> = rows_taken
            .iter()
            .flat_map(|r| row_to_lines(r, frame, active_tools, theme, model.has_active_stream()))
            .collect();
        // Use ratatui's own wrap engine to count visible lines — our
        // own visual_count is an approximation (it doesn't perfectly
        // model prefix glyphs, list markers, code-fence styling, etc.).
        // The under-count meant `scroll_top` was several lines too low,
        // so the bottom of the transcript (the most recent assistant
        // response and the user's "hi") got clipped off the viewport.
        // line_count(chunks[1].width) returns the exact post-wrap line
        // total that the Paragraph will produce, so subtracting
        // viewport_h gives a scroll_top that aligns the bottom of the
        // content with the bottom of the viewport.
        let probe = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
        let actual_visual = probe.line_count(chunks[1].width) as i64;
        let lines_below_visible_in_slice = (slice_bottom_line - effective_bottom).max(0);
        let scroll_top =
            (actual_visual - viewport_h as i64 - lines_below_visible_in_slice).max(0) as u16;
        let slice_visual: i64 = rows_taken.iter().map(|r| visual_count(r) as i64).sum();
        if crate::scrolllog::enabled() {
            crate::scrolllog::log_json(serde_json::json!({
                "ev": "render",
                "frame": frame,
                "scroll_offset": viewport.scroll_offset,
                "frozen_visual_bottom": viewport.frozen_visual_bottom,
                "auto_follow": viewport.auto_follow,
                "viewport_h": viewport_h,
                "total_rows": total_rows,
                "total_visual_now": total_visual_now,
                "effective_bottom": effective_bottom,
                "below_visible": below_visible,
                "rows_taken": rows_taken.len(),
                "slice_visual": slice_visual,
                "actual_visual": actual_visual,
                "slice_bottom_line": slice_bottom_line,
                "lines_below_visible_in_slice": lines_below_visible_in_slice,
                "scroll_top": scroll_top,
                "transcript_width": transcript_width,
                "avail": avail,
                "first_row_text_prefix": rows_taken.first().map(|r| r.text.chars().take(40).collect::<String>()),
                "last_row_text_prefix": rows_taken.last().map(|r| r.text.chars().take(40).collect::<String>()),
            }));
        }
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

        // Vertical scrollbar pinned to the right edge of the transcript
        // chunk. The position is in visual lines: top = 0, bottom =
        // `total_visual_now - viewport_h`. While auto-following the
        // live tail the thumb sits at the bottom. We skip rendering it
        // entirely if the transcript fits without scrolling (no point
        // showing a full-length thumb).
        if total_visual_now > viewport_h as i64 && inspector.is_none() {
            let scrollable_extent = (total_visual_now - viewport_h as i64).max(1) as usize;
            let position = (total_visual_now - effective_bottom).max(0) as usize;
            // Invert: position should be 0 at the top (oldest content
            // visible) and at max when bottom (latest visible).
            let bar_pos = scrollable_extent.saturating_sub(position);
            let mut state = ScrollbarState::new(scrollable_extent)
                .position(bar_pos)
                .viewport_content_length(viewport_h);
            let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .style(Style::default().fg(Color::DarkGray))
                .thumb_style(Style::default().fg(Color::Cyan));
            f.render_stateful_widget(bar, chunks[1], &mut state);
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
        // Subtle "scrolled" indicator only — the live=/history=/total=
        // counters were debug noise; they're now only visible via the
        // metrics overlay (M). Auto-follow is the default and silent.
        let _ = (follow, &live, &history); // suppress unused warnings
        if !indicator.is_empty() {
            spans.push(Span::styled(
                indicator.to_string(),
                Style::default().fg(Color::Yellow),
            ));
        }
        // Task ribbon (only if there are any active tasks).
        if has_tasks {
            use camouflage_renderer::model::BackgroundTaskState;
            let mut ribbon_spans: Vec<Span> = Vec::new();
            for (i, task) in model.background_tasks().iter().enumerate() {
                if i > 0 { ribbon_spans.push(Span::raw("  ")); }
                let (glyph, glyph_style, label_style) = match task.state {
                    BackgroundTaskState::Running => (
                        spinner_glyph(spinner_frame_now()).to_string(),
                        Style::default().fg(Color::Cyan),
                        Style::default().fg(Color::Gray),
                    ),
                    BackgroundTaskState::Done => (
                        "✓".to_string(),
                        Style::default().fg(rgb_to_color(theme.diff_add)),
                        Style::default().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT),
                    ),
                    BackgroundTaskState::Error => (
                        "✗".to_string(),
                        Style::default().fg(rgb_to_color(theme.error)),
                        Style::default().fg(Color::DarkGray),
                    ),
                };
                ribbon_spans.push(Span::styled(glyph, glyph_style));
                ribbon_spans.push(Span::raw(" "));
                let mut label = task.label.clone();
                if let Some(p) = task.progress {
                    let pct = (p.clamp(0.0, 1.0) * 100.0) as u32;
                    label.push_str(&format!(" {}%", pct));
                }
                ribbon_spans.push(Span::styled(label, label_style));
            }
            let ribbon = Paragraph::new(Line::from(ribbon_spans));
            f.render_widget(ribbon, chunks[3]);
        }

        let status_line = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false });
        f.render_widget(status_line, chunks[4]);

        // Bottom box: search prompt > permission widget > input prompt.
        let input_chunk = chunks[5];
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
            let yellow = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
            let dim = Style::default().fg(Color::DarkGray);
            let sel = Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD);
            let mut lines: Vec<Line> = Vec::new();
            if pp.help_open {
                // Help overlay (ports Ink's PermissionModal "?" panel).
                lines.push(Line::from(Span::styled(" Permission modal — keyboard shortcuts", yellow)));
                for (k, v) in [
                    ("↑ / ↓ or j / k", "navigate options"),
                    ("1 / 2 / 3", "pick option directly"),
                    ("Enter", "confirm highlighted"),
                    ("Esc", "deny and close"),
                    ("?", "toggle this help"),
                    ("(in feedback)", "Enter submit + deny; Esc deny without"),
                ] {
                    lines.push(Line::from(vec![
                        Span::styled(format!("   {:<18}", k), Style::default().fg(Color::Cyan)),
                        Span::raw(v),
                    ]));
                }
                lines.push(Line::from(Span::styled(" press any key to close", dim)));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(" ⚠ permission needed: ", yellow),
                    Span::styled(pp.action.clone(), Style::default().fg(Color::White)),
                    Span::styled(format!("  ({})", pp.tool), dim),
                ]));
                // Inline diff (ports Ink DiffView). Trimmed to keep the
                // bottom widget compact; the host can also surface the
                // full diff via PatchProposed for the in-transcript view.
                if let Some(d) = pp.diff.as_ref() {
                    lines.push(Line::from(Span::styled(format!("   ── {} ──", d.path), dim)));
                    for line in unified_diff_lines(&d.before, &d.after, 12) {
                        let style = if line.starts_with('+') {
                            Style::default().fg(Color::Green)
                        } else if line.starts_with('-') {
                            Style::default().fg(Color::Red)
                        } else if line.starts_with('@') {
                            Style::default().fg(Color::Magenta)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        lines.push(Line::from(Span::styled(format!("   {}", line), style)));
                    }
                }
                // Three options, vertically — Ink uses arrow + label per row.
                let labels = ["Allow once", "Allow for this session", "Something else"];
                let keys = [1, 2, 3];
                for (i, (label, key)) in labels.iter().zip(keys.iter()).enumerate() {
                    let style = if i == pp.selected { sel } else { Style::default().fg(Color::White) };
                    let arrow = if i == pp.selected { "▸ " } else { "  " };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {}", arrow), style),
                        Span::styled(format!("[{}] {}", key, label), style),
                    ]));
                }
                lines.push(Line::from(vec![
                    Span::styled("   feedback › ", dim),
                    Span::styled(
                        if permission_feedback.is_empty() {
                            "(optional — type a reason, sent with your choice)".to_string()
                        } else { permission_feedback.to_string() },
                        if permission_feedback.is_empty() {
                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)
                        } else { Style::default().fg(Color::White) },
                    ),
                ]));
                lines.push(Line::from(Span::styled(
                    "   ↑↓ navigate · Enter confirm · 1/2/3 pick · ? help · Esc deny",
                    dim,
                )));
            }
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
            // Cursor + scroll math. The input box has a fixed visible
            // height (3 content lines max — see bottom_height above), so
            // long voice-dictated prompts wrap past the bottom of the box.
            // We compute the total wrapped line count and, when it exceeds
            // what fits, scroll the Paragraph so the cursor's line is the
            // bottom visible line — and show a "+N lines · M chars" badge
            // in the title so the user knows nothing was truncated.
            let prompt_w: u16 = 2; // "› "
            let typed_before_cursor: String = input_buf.chars().take(input_cursor).collect();
            let cursor_col_advance = UnicodeWidthStr::width(typed_before_cursor.as_str()) as u16;
            let total_w = UnicodeWidthStr::width(input_buf) as u16 + prompt_w;
            let inner_w = input_chunk.width.saturating_sub(2).max(1);
            let total_x = 1u16 + prompt_w + cursor_col_advance; // 1 = left border
            let cursor_line = (total_x.saturating_sub(1)) / inner_w; // 0-indexed visual line
            let total_lines = if total_w == 0 { 1 } else { (total_w + inner_w - 1) / inner_w }.max(1);
            let visible_lines = input_chunk.height.saturating_sub(2).max(1);
            // Scroll so the cursor stays in view; clamp so we never scroll
            // past the end of the buffer.
            let scroll_off: u16 = cursor_line
                .saturating_sub(visible_lines.saturating_sub(1))
                .min(total_lines.saturating_sub(visible_lines));
            let title: String = if total_lines > visible_lines {
                let chars = input_buf.chars().count();
                format!(
                    "input · {} chars · {} lines (full text captured)",
                    chars, total_lines,
                )
            } else {
                "input".to_string()
            };
            let title_style = if total_lines > visible_lines {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            let input = Paragraph::new(Line::from(vec![
                Span::styled("› ", Style::default().fg(Color::Cyan)),
                Span::raw(input_buf),
            ]))
            .wrap(Wrap { trim: false })
            .scroll((scroll_off, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled(title, title_style)),
            );
            f.render_widget(input, input_chunk);
            let cursor_x = input_chunk.x + (total_x % inner_w);
            let cursor_y = input_chunk.y + 1 + cursor_line.saturating_sub(scroll_off);
            f.set_cursor(cursor_x, cursor_y);
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
        if let Some(sub) = slash_sub_state {
            if model.pending_permission().is_none() && search.is_none() {
                draw_slash_sub_picker_overlay(f, area, sub);
            }
        } else if input_buf.starts_with('/') && !model.slash_commands().is_empty()
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
        // CC-7 — KeyValueView modal. Same display-only treatment as Table.
        if let Some(k) = model.active_kv() {
            draw_kv_overlay(f, area, k, theme);
        }
        // CC-5 — Form modal. Painted last so it stacks on top of any
        // display-only modal (Form is interactive).
        if let Some(form) = model.active_form() {
            draw_form_overlay(f, area, form, theme);
        }
        // Optional: dump literal cell contents to disk so we can review
        // what actually hit the screen at this frame. Gated by env var
        // since the output is large.
        if crate::scrolllog::screen_enabled() {
            let buf = f.buffer_mut();
            crate::scrolllog::log_screen(area, buf, "full", frame);
            crate::scrolllog::log_screen(chunks[1], buf, "transcript", frame);
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
    // Two-tier ordering: recent files first (the ones the user has
    // actually touched), then everything else. Mirrors Ink's FilePicker
    // "Recent" / "All files" sections. Sort is stable, so within each
    // tier the original registration order is preserved.
    let mut matches: Vec<&camouflage_renderer::model::MentionEntry> = model
        .mention_candidates()
        .iter()
        .filter(|m| m.token.contains(partial))
        .collect();
    matches.sort_by_key(|m| !m.recent);
    if matches.is_empty() {
        return;
    }
    let visible = matches.len().min(12);
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
    let scroll_off = sel_idx.saturating_sub(visible - 1);
    let end = (scroll_off + visible).min(matches.len());
    // Build lines with section headers inserted at the recent→all boundary
    // when the window straddles it.
    let header = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let recent_in_window = matches[scroll_off..end].iter().any(|m| m.recent);
    let other_in_window = matches[scroll_off..end].iter().any(|m| !m.recent);
    let mut lines: Vec<Line> = Vec::new();
    if recent_in_window {
        lines.push(Line::from(Span::styled(" Recent", header)));
    }
    let mut saw_other_header = !other_in_window || !recent_in_window;
    for (i, m) in matches[scroll_off..end].iter().enumerate() {
        let absolute_i = scroll_off + i;
        if !saw_other_header && !m.recent {
            lines.push(Line::from(Span::styled(" All files", header)));
            saw_other_header = true;
        }
        let style = if absolute_i == sel_idx { sel } else { plain };
        let kind = m.kind.as_deref().unwrap_or("");
        let label = m.label.as_deref().unwrap_or("");
        let prefix = if m.recent { " ↻ @" } else { "   @" };
        lines.push(Line::from(vec![
            Span::styled(format!("{}{}", prefix, m.token), style),
            Span::styled(
                if kind.is_empty() { "  ".to_string() } else { format!("  [{}]  ", kind) },
                dim,
            ),
            Span::styled(label.to_string(), dim),
        ]));
    }
    if matches.len() > visible {
        lines.push(Line::from(vec![Span::styled(
            format!(" {}/{} ", sel_idx + 1, matches.len()),
            dim,
        )]));
    }

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" mentions  ↑/↓ wrap  Enter insert ")
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
    // Windowed view: keep the selected item visible by scrolling the
    // slice up when sel_idx exceeds the bottom of the visible window.
    let scroll_off = sel_idx.saturating_sub(visible - 1);
    let end = (scroll_off + visible).min(matches.len());
    // Source-badge palette mirrors Ink: project commands get accent
    // (they're the user's own), global is cyan (cross-repo), builtin is
    // dim grey (lowest information). The badge is right-aligned via
    // padding so columns line up.
    let badge_for = |src: Option<&str>| -> Option<(String, Style)> {
        match src {
            Some("project") => Some(("  [project]".into(), Style::default().fg(Color::Yellow))),
            Some("global")  => Some(("  [global] ".into(), Style::default().fg(Color::Cyan))),
            Some("builtin") => Some(("  [builtin]".into(), Style::default().fg(Color::DarkGray))),
            Some(other) if !other.is_empty() => Some((format!("  [{}]", other), Style::default().fg(Color::DarkGray))),
            _ => None,
        }
    };
    let mut lines: Vec<Line> = matches[scroll_off..end]
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let absolute_i = scroll_off + i;
            let style = if absolute_i == sel_idx { sel } else { plain };
            let hint = c.args_hint.as_deref().unwrap_or("");
            let mut spans = vec![
                Span::styled(format!(" /{}", c.name), style),
                Span::styled(if hint.is_empty() { "  ".to_string() } else { format!(" {}  ", hint) }, dim),
                Span::styled(c.description.clone(), dim),
            ];
            if let Some((txt, s)) = badge_for(c.source.as_deref()) {
                spans.push(Span::styled(txt, s));
            }
            Line::from(spans)
        })
        .collect();
    // Show a "X/Y" footer when the list is longer than the window.
    if matches.len() > visible {
        lines.push(Line::from(vec![Span::styled(
            format!(" {}/{} ", sel_idx + 1, matches.len()),
            dim,
        )]));
    }

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" slash commands  ↑/↓ wrap  Enter insert ")
                .border_style(Style::default().fg(Color::Cyan)),
        );
    f.render_widget(widget, overlay);
}

fn draw_slash_sub_picker_overlay(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    state: &(String, Vec<String>, bool, usize),
) {
    let (parent, options, optional, idx) = state;
    // Include a "(no arg)" trailing entry when the argument is optional.
    let mut entries: Vec<String> = options.clone();
    if *optional {
        entries.push("(no argument)".to_string());
    }
    if entries.is_empty() {
        return;
    }
    let visible = entries.len().min(8);
    let w: u16 = 50;
    let h: u16 = (visible as u16) + 2;
    if area.width < w + 2 || area.height < h + 6 {
        return;
    }
    let bottom_height: u16 = 5;
    let x = area.x + 2;
    let y = area
        .y
        .saturating_add(area.height)
        .saturating_sub(bottom_height)
        .saturating_sub(h);
    let overlay = ratatui::layout::Rect { x, y, width: w, height: h };
    f.render_widget(Clear, overlay);

    let dim = Style::default().fg(Color::DarkGray);
    let sel = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let plain = Style::default().fg(Color::White);

    let sel_idx = (*idx).min(entries.len() - 1);
    let scroll_off = sel_idx.saturating_sub(visible - 1);
    let end = (scroll_off + visible).min(entries.len());
    let lines: Vec<Line> = entries[scroll_off..end]
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let absolute_i = scroll_off + i;
            let style = if absolute_i == sel_idx { sel } else { plain };
            let preview = if absolute_i + 1 > options.len() {
                // sentinel for the optional "no argument" entry
                format!("   /{}", parent)
            } else {
                format!("   /{} {}", parent, label)
            };
            Line::from(vec![
                Span::styled(format!(" {}", label), style),
                Span::styled(preview, dim),
            ])
        })
        .collect();

    let title = format!(" /{} — pick · ↑/↓ · Enter submit · ←/Esc back ", parent);
    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
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

fn draw_form_overlay(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    form: &camouflage_renderer::model::FormState,
    theme: &camouflage_renderer::theme::Theme,
) {
    use camouflage_renderer::model::FormFieldKind;
    let label_w = form
        .fields
        .iter()
        .map(|f| UnicodeWidthStr::width(f.label.as_str()))
        .max()
        .unwrap_or(8);
    let value_w_target: usize = 32;
    let w = ((label_w + value_w_target + 6) as u16).max(40).min(area.width.saturating_sub(2));
    let h: u16 = (form.fields.len() as u16) + 4 + if form.title.is_some() { 1 } else { 0 };
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
    let label_style = Style::default().fg(rgb_to_color(theme.system));
    let value_style = Style::default().fg(rgb_to_color(theme.assistant));
    let focused_marker = Style::default()
        .fg(rgb_to_color(theme.accent))
        .add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = Vec::new();
    if let Some(t) = form.title.as_deref() {
        lines.push(Line::from(vec![Span::styled(format!(" {}", t), head)]));
    }
    for (i, field) in form.fields.iter().enumerate() {
        let is_focused = i == form.focused;
        let marker = if is_focused { "▸" } else { " " };
        let pad = label_w.saturating_sub(UnicodeWidthStr::width(field.label.as_str()));
        let display_value = match field.kind {
            FormFieldKind::Password => "•".repeat(field.value.chars().count()),
            FormFieldKind::Text => field.value.clone(),
        };
        let shown = if display_value.is_empty() {
            field
                .placeholder
                .as_deref()
                .map(|p| p.to_string())
                .unwrap_or_else(String::new)
        } else {
            display_value.clone()
        };
        let style_for_value = if display_value.is_empty() && field.placeholder.is_some() {
            dim
        } else if is_focused {
            value_style.add_modifier(Modifier::UNDERLINED)
        } else {
            value_style
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", marker), focused_marker),
            Span::styled(field.label.clone(), label_style),
            Span::raw(" ".repeat(pad)),
            Span::styled("   ", dim),
            Span::styled(shown, style_for_value),
            if field.required {
                Span::styled(" *", Style::default().fg(rgb_to_color(theme.error)))
            } else {
                Span::raw("")
            },
        ]));
    }
    let hint = if form.allow_cancel {
        "↑/↓ navigate · Enter next/submit · Esc cancel"
    } else {
        "↑/↓ navigate · Enter next/submit"
    };
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(hint, dim)]));

    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" form ")
            .border_style(Style::default().fg(rgb_to_color(theme.overlay_border))),
    );
    f.render_widget(widget, overlay);
}

fn draw_kv_overlay(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    k: &camouflage_renderer::model::KeyValueViewState,
    theme: &camouflage_renderer::theme::Theme,
) {
    let label_w = k
        .items
        .iter()
        .map(|i| UnicodeWidthStr::width(i.label.as_str()))
        .max()
        .unwrap_or(8);
    let value_w = k
        .items
        .iter()
        .map(|i| UnicodeWidthStr::width(i.value.as_str()))
        .max()
        .unwrap_or(16)
        .min(60);
    let w = ((label_w + value_w + 6) as u16).max(36).min(area.width.saturating_sub(2));
    let visible = k.items.len().min(16);
    let h: u16 = (visible as u16) + 3 + if k.title.is_some() { 1 } else { 0 };
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
    let label_style = Style::default().fg(rgb_to_color(theme.system));
    let value_style = Style::default().fg(rgb_to_color(theme.assistant));

    let mut lines: Vec<Line> = Vec::new();
    if let Some(title) = k.title.as_deref() {
        lines.push(Line::from(vec![Span::styled(format!(" {}", title), head)]));
    }
    for item in k.items.iter().take(visible) {
        let pad = label_w.saturating_sub(UnicodeWidthStr::width(item.label.as_str()));
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(item.label.clone(), label_style),
            Span::raw(" ".repeat(pad)),
            Span::styled("   ", dim),
            Span::styled(item.value.clone(), value_style),
        ]));
    }
    if k.items.len() > visible {
        lines.push(Line::from(vec![Span::styled(
            format!(" … {} more (truncated)", k.items.len() - visible),
            dim,
        )]));
    }
    lines.push(Line::from(vec![Span::styled("Esc to close", dim)]));

    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" details ")
            .border_style(Style::default().fg(rgb_to_color(theme.overlay_border))),
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
    // Windowed view keyed on the position of `sl.selected` within the
    // filtered `visible` indices. Scrolls when the selection moves past
    // the bottom; wraps cleanly when the host's input handler wraps the
    // index.
    let sel_pos_in_visible = visible
        .iter()
        .position(|&i| i == sl.selected)
        .unwrap_or(0);
    let scroll_off = sel_pos_in_visible.saturating_sub(entry_count - 1);
    let visible_window = &visible[scroll_off..(scroll_off + entry_count).min(visible.len())];
    for &i in visible_window {
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
    if visible.len() > entry_count {
        lines.push(Line::from(vec![Span::styled(
            format!(" {}/{} ", sel_pos_in_visible + 1, visible.len()),
            dim,
        )]));
    }
    let hint = if sl.allow_cancel {
        "↑/↓ wrap · Enter select · Esc cancel"
    } else {
        "↑/↓ wrap · Enter select"
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
            let st = r.tool_id.as_ref().and_then(|tid| active_tools.get(tid));
            if let Some(state) = st {
                if !state.finished {
                    (spinner_glyph(spinner_frame_now()).to_string(), spinner_color)
                } else {
                    // Status-aware glyphs match Ink's ToolView [ok]/[err]/[x]/[!].
                    use camouflage_renderer::model::ToolStatus;
                    let (g, c) = match state.status {
                        Some(ToolStatus::Error)     => ("✗", rgb_to_color(theme.error)),
                        Some(ToolStatus::Cancelled) => ("■", system_color),
                        Some(ToolStatus::Rejected)  => ("!", rgb_to_color(theme.error)),
                        _ => ("✓", rgb_to_color(theme.diff_add)),
                    };
                    (g.to_string(), c)
                }
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
            if state.repeated {
                spans.push(Span::styled(
                    " [warn] repeated".to_string(),
                    Style::default().fg(rgb_to_color(theme.error)),
                ));
            }
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

/// Expand a row into one-or-more Lines, splitting on embedded `\n` so
/// markdown paragraph breaks render correctly, and adding lightweight
/// block-level styling for code fences (```), bullet lists, and
/// numbered lists. Only Assistant rows are split; other row kinds
/// remain single-line.
///
/// Code fences toggle a flag; while in a fenced block the whole line is
/// painted with the code_fg/code_bg pair and inline parsing is skipped
/// (so `*literal asterisks*` stay literal in code).
fn row_to_lines<'a>(
    r: &'a camouflage_renderer::Row,
    frame: u64,
    active_tools: &std::collections::HashMap<String, camouflage_renderer::ToolState>,
    theme: &camouflage_renderer::theme::Theme,
    has_active_stream: bool,
) -> Vec<Line<'a>> {
    if r.kind != RowKind::Assistant || !r.text.contains('\n') {
        return vec![row_to_line(r, frame, active_tools, theme, has_active_stream)];
    }
    let assistant_color = rgb_to_color(theme.assistant);
    let code_fg = rgb_to_color(theme.code_fg);
    let code_bg = rgb_to_color(theme.code_bg);
    let accent = rgb_to_color(theme.accent);
    let mut out: Vec<Line> = Vec::new();
    let segments: Vec<&str> = r.text.split('\n').collect();
    let mut in_code = false;
    for (i, seg) in segments.iter().enumerate() {
        let mut spans: Vec<Span> = Vec::new();
        if i == 0 {
            spans.push(Span::styled("  ", Style::default().fg(assistant_color)));
        } else {
            spans.push(Span::raw("  "));
        }
        let trimmed = seg.trim_start();
        // ── code fence handling ──────────────────────────────────────
        if trimmed.starts_with("```") {
            // Toggle and render the fence itself dimly so the user can
            // see the boundary without it dominating the row.
            in_code = !in_code;
            spans.push(Span::styled(
                seg.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
            out.push(Line::from(spans));
            continue;
        }
        if in_code {
            // Whole-line code styling. Background bg covers visible
            // chars; trailing pad on short lines is left intentionally
            // unpainted to avoid loud half-row stripes.
            spans.push(Span::styled(
                seg.to_string(),
                Style::default().fg(code_fg).bg(code_bg),
            ));
            out.push(Line::from(spans));
            continue;
        }
        // ── bullets and numbered lists ───────────────────────────────
        // Match common markdown markers (-, *, +, 1.) and recolor the
        // glyph in accent so lists scan at a glance. The remaining text
        // still flows through parse_inline.
        let (list_glyph, body): (Option<&str>, &str) =
            if let Some(rest) = trimmed.strip_prefix("- ") { (Some("• "), rest) }
            else if let Some(rest) = trimmed.strip_prefix("* ") { (Some("• "), rest) }
            else if let Some(rest) = trimmed.strip_prefix("+ ") { (Some("• "), rest) }
            else {
                // numbered: "1. text", "23. text"
                let mut idx = 0usize;
                let bytes = trimmed.as_bytes();
                while idx < bytes.len() && bytes[idx].is_ascii_digit() { idx += 1; }
                if idx > 0 && trimmed.get(idx..idx + 2) == Some(". ") {
                    let prefix = &trimmed[..idx + 2];
                    let rest = &trimmed[idx + 2..];
                    spans.push(Span::styled(prefix.to_string(), Style::default().fg(accent)));
                    (None, rest)
                } else { (None, seg.as_ref()) }
            };
        if let Some(glyph) = list_glyph {
            spans.push(Span::styled(glyph.to_string(), Style::default().fg(accent)));
        }
        // Inline pass for the body.
        for sp in parse_inline(body) {
            let style = match sp.style {
                InlineStyle::Plain => Style::default().fg(assistant_color),
                InlineStyle::Bold => Style::default().fg(assistant_color).add_modifier(Modifier::BOLD),
                InlineStyle::Italic => Style::default().fg(assistant_color).add_modifier(Modifier::ITALIC),
                InlineStyle::Code => Style::default().fg(code_fg).bg(code_bg),
            };
            spans.push(Span::styled(sp.text, style));
        }
        out.push(Line::from(spans));
    }
    out
}

fn rgb_to_color(rgb: camouflage_renderer::theme::Rgb) -> Color {
    Color::Rgb(rgb.0, rgb.1, rgb.2)
}

fn short_uuid(s: &str) -> String {
    s.chars().take(8).collect()
}

/// Compact unified-diff renderer for the PermissionModal preview.
/// Hand-rolled instead of pulling in a full `similar` dep: we just need
/// "+ / - / space" prefixes per changed line, with up to `max` lines of
/// output. Identical content returns empty. Truncation is signalled by
/// a trailing "… (N more lines)" entry.
///
/// Cheap heuristic — counts adds/removes via a line-by-line LCS with a
/// small window. For deeply rewritten files the result is still useful
/// (shows the first N changes); for surgical edits it's near-optimal.
pub(crate) fn unified_diff_lines(before: &str, after: &str, max: usize) -> Vec<String> {
    if before == after { return vec![]; }
    let a: Vec<&str> = before.split('\n').collect();
    let b: Vec<&str> = after.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while (i < a.len() || j < b.len()) && out.len() < max {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            // context — keep light, only when adjacent to a change
            i += 1; j += 1;
            continue;
        }
        if i < a.len() && (j >= b.len() || !b[j..].iter().take(8).any(|x| *x == a[i])) {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else if j < b.len() {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    let remaining = (a.len().saturating_sub(i)) + (b.len().saturating_sub(j));
    if remaining > 0 && out.len() == max {
        out.push(format!("… ({} more lines)", remaining));
    }
    out
}
