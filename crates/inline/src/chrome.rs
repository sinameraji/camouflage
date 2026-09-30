//! The live region's fixed parts: spinner line, input box, pickers, prompts,
//! and footer. Only these (plus unfinished blocks) are ever redrawn.

use crate::blocks::{format_elapsed, render_diff, DiffLine, SPINNER};
use crate::editor::Editor;
use crate::style::{Color, Line, Span, Style};
use crate::theme::Theme;
use crate::width::{grapheme_width, str_width, truncate, wrap};
use unicode_segmentation::UnicodeSegmentation;

/// `⠹ Editing session.ts… (12s · ↓ 1.2k tokens · esc to interrupt)`.
/// A brighter band sweeps across the verb so the line reads as alive even
/// when the numbers don't change.
pub fn spinner_line(theme: &Theme, frame: usize, verb: &str, elapsed_ms: u64, tokens: Option<u64>, width: usize) -> Line {
    let mut line = Line::new();
    line.push(SPINNER[frame % SPINNER.len()], theme.accent());
    line.push(" ", Style::new());
    let label = format!("{verb}…");
    let gs: Vec<&str> = label.graphemes(true).collect();
    let span = gs.len() + 6;
    let head = frame % span;
    for (i, g) in gs.iter().enumerate() {
        let lit = i + 2 >= head && i <= head;
        line.push(*g, if lit { theme.accent().bold() } else { theme.accent() });
    }
    let mut meta = format!(" ({}", format_elapsed(elapsed_ms));
    if let Some(t) = tokens {
        meta.push_str(&format!(" · ↓ {} tokens", format_count(t)));
    }
    meta.push_str(" · esc to interrupt)");
    if line.width() + str_width(&meta) <= width {
        line.push(meta, theme.dim());
    }
    line
}

pub fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Default,
    AcceptEdits,
    Plan,
}

/// The rounded input box. Returns its rows and the cursor cell within them.
/// The border color shows the mode: dim, yellow for accept-edits, cyan for
/// plan.
pub fn input_box(
    theme: &Theme,
    editor: &Editor,
    mode: InputMode,
    placeholder: &str,
    width: usize,
    max_rows: usize,
) -> (Vec<Line>, (usize, usize)) {
    let border = match mode {
        InputMode::Default => theme.dim(),
        InputMode::AcceptEdits => Style::new().fg(Color::Yellow),
        InputMode::Plan => Style::new().fg(Color::Cyan),
    };
    let inner = width.saturating_sub(4).max(4);
    let text_w = inner.saturating_sub(2).max(2);
    let mut rows_out: Vec<Line> = Vec::new();
    let cursor = if editor.is_empty() {
        let mut l = Line::new();
        l.push("› ", theme.accent());
        l.push(truncate(placeholder, text_w), theme.dim());
        rows_out.push(l);
        (0, 2)
    } else {
        let text = editor.text();
        let rows = editor.rows(text_w);
        let (cr, cc) = editor.cursor_pos(text_w);
        // Keep the cursor row visible when the input is taller than allowed.
        let visible = max_rows.max(1);
        let first = if cr >= visible { cr + 1 - visible } else { 0 }.min(rows.len().saturating_sub(visible));
        for (i, r) in rows.iter().enumerate().skip(first).take(visible) {
            let mut l = Line::new();
            l.push(if i == 0 { "› " } else { "  " }, theme.accent());
            l.spans.extend(input_spans(&text[r.start..r.end], theme));
            rows_out.push(l);
        }
        (cr - first, 2 + cc)
    };

    let mut out = Vec::with_capacity(rows_out.len() + 2);
    out.push(Line::styled(format!("╭{}╮", "─".repeat(width.saturating_sub(2))), border));
    for r in rows_out {
        let pad = inner.saturating_sub(r.width());
        let mut l = Line::styled("│ ", border);
        l.spans.extend(r.spans);
        l.push(" ".repeat(pad), Style::new());
        l.push(" │", border);
        out.push(l);
    }
    out.push(Line::styled(format!("╰{}╯", "─".repeat(width.saturating_sub(2))), border));
    (out, (cursor.0 + 1, cursor.1 + 2))
}

/// Color `@mentions`, a leading `/command`, and paste chips.
fn input_spans(text: &str, theme: &Theme) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut push = |t: &str, s: Style| match spans.last_mut() {
        Some(last) if last.style == s => last.text.push_str(t),
        _ => spans.push(Span::styled(t, s)),
    };
    let mut rest = text;
    if rest.starts_with('/') {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        push(&rest[..end], theme.accent());
        rest = &rest[end..];
    }
    while !rest.is_empty() {
        if let Some(chip_start) = rest.find("[Pasted text #") {
            if let Some(len) = rest[chip_start..].find(" lines]") {
                let (before, chip_and_after) = rest.split_at(chip_start);
                push_words(before, theme, &mut push);
                let chip_len = len + " lines]".len();
                push(&chip_and_after[..chip_len], theme.accent().reverse());
                rest = &chip_and_after[chip_len..];
                continue;
            }
        }
        push_words(rest, theme, &mut push);
        break;
    }
    spans
}

fn push_words(text: &str, theme: &Theme, push: &mut impl FnMut(&str, Style)) {
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            push(" ", Style::new());
        }
        if word.len() > 1 && word.starts_with('@') {
            push(word, theme.accent());
        } else {
            push(word, Style::new());
        }
    }
}

/// A picker item: `label` plus an optional dimmed hint. `hits` are byte
/// offsets in `label` to bold (fuzzy-match highlights).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickItem {
    pub label: String,
    pub hint: Option<String>,
    pub hits: Vec<usize>,
}

/// A list under the input box (`/` commands, `@` files).
pub fn picker(theme: &Theme, items: &[PickItem], selected: usize, width: usize, max_rows: usize) -> Vec<Line> {
    if items.is_empty() {
        return Vec::new();
    }
    let rows = max_rows.max(1).min(items.len());
    let first = if selected >= rows { selected + 1 - rows } else { 0 };
    let label_w = items.iter().map(|i| str_width(&i.label)).max().unwrap_or(0).min(width / 2).max(8);
    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate().skip(first).take(rows) {
        let sel = i == selected;
        let mut l = Line::raw("  ");
        let base = if sel { theme.accent() } else { Style::new() };
        let mut used = 0;
        for (bi, g) in it.label.grapheme_indices(true) {
            let gw = grapheme_width(g);
            if used + gw > label_w {
                break;
            }
            let style = if it.hits.contains(&bi) || sel { base.bold() } else { base };
            l.push(g, style);
            used += gw;
        }
        if let Some(h) = &it.hint {
            l.push(" ".repeat(label_w - used + 2), Style::new());
            let room = width.saturating_sub(2 + label_w + 2);
            if room > 3 {
                l.push(truncate(h, room), if sel { Style::new() } else { theme.dim() });
            }
        }
        out.push(l);
    }
    if items.len() > rows {
        out.push(Line::styled(format!("  {}/{}", selected + 1, items.len()), theme.dim()));
    }
    out
}

/// What a prompt box asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptView {
    pub title: String,
    /// A second line under the title (a path, or an explanation).
    pub subtitle: Option<String>,
    /// Optional diff preview, already rendered (see `diff_preview`), so a
    /// redraw doesn't re-highlight it.
    pub diff_lines: Option<Vec<Line>>,
    pub question: Option<String>,
    pub options: Vec<String>,
    pub hints: Vec<Option<String>>,
    /// Index of the currently active option, marked with ✓.
    pub current: Option<usize>,
    pub selected: usize,
    /// Show `esc` next to the last option (it is what Esc does).
    pub esc_is_last: bool,
    pub footer: String,
}

/// A permission or selection prompt: a rounded box with an accent border
/// that replaces the input box while it is open.
pub fn prompt_box(theme: &Theme, p: &PromptView, width: usize, max_rows: usize) -> Vec<Line> {
    let inner = width.saturating_sub(4).max(8);
    let mut content: Vec<Line> = vec![Line::styled(truncate(&p.title, inner), theme.strong())];
    if let Some(s) = &p.subtitle {
        content.extend(wrap(&Line::styled(s.clone(), theme.dim()), inner, &Line::new()));
    }
    if let Some(diff_lines) = &p.diff_lines {
        content.push(Line::new());
        // Leave room for everything else; show the start of a long diff.
        let budget = max_rows.saturating_sub(8 + p.options.len()).max(3);
        let mut lines = diff_lines.clone();
        if lines.len() > budget {
            let more = lines.len() - budget;
            lines.truncate(budget);
            lines.push(Line::styled(format!("    … +{more} more lines"), theme.dim()));
        }
        content.extend(lines.into_iter().map(|mut l| {
            l.fill = None;
            clip(l, inner)
        }));
    }
    content.push(Line::new());
    if let Some(q) = &p.question {
        content.extend(wrap(&Line::raw(q.clone()), inner, &Line::new()));
    }
    for (i, o) in p.options.iter().enumerate() {
        let sel = i == p.selected;
        let mut l = Line::new();
        l.push(if sel { "› " } else { "  " }, theme.accent());
        l.push(format!("{}. ", i + 1), if sel { theme.accent() } else { theme.dim() });
        l.push(o.clone(), if sel { theme.accent().bold() } else { Style::new() });
        if p.current == Some(i) {
            l.push(" ✓", theme.ok());
        }
        if p.esc_is_last && i + 1 == p.options.len() {
            l.push("  esc", theme.dim());
        }
        if let Some(Some(h)) = p.hints.get(i) {
            let used = l.width();
            let col = used.max(40.min(inner / 2)) + 2;
            if col + 4 < inner {
                l.push(" ".repeat(col - used), Style::new());
                l.push(truncate(h, inner - col), theme.dim());
            }
        }
        content.push(clip(l, inner));
    }

    let border = theme.accent();
    let mut out = vec![Line::styled(format!("╭{}╮", "─".repeat(width.saturating_sub(2))), border)];
    for c in content {
        let pad = inner.saturating_sub(c.width());
        let mut l = Line::styled("│ ", border);
        l.spans.extend(c.spans);
        l.push(" ".repeat(pad), Style::new());
        l.push(" │", border);
        out.push(l);
    }
    out.push(Line::styled(format!("╰{}╯", "─".repeat(width.saturating_sub(2))), border));
    out.push(Line::styled(format!("  {}", truncate(&p.footer, width.saturating_sub(2))), theme.dim()));
    out
}

/// Render a diff for a prompt box of this total width. Do it once per
/// prompt (and per resize), not per frame.
pub fn diff_preview(theme: &Theme, path: &str, diff: &[DiffLine], width: usize) -> Vec<Line> {
    render_diff(path, diff, width.saturating_sub(4).max(8), theme)
}

fn clip(line: Line, width: usize) -> Line {
    if line.width() <= width {
        return line;
    }
    let mut out = Line { spans: Vec::new(), fill: line.fill };
    let mut used = 0;
    for s in line.spans {
        let w = s.width();
        if used + w < width {
            used += w;
            out.spans.push(s);
        } else {
            out.spans.push(Span::styled(truncate(&s.text, width - used), s.style));
            break;
        }
    }
    out
}

/// The dim line under the input: status or mode on the left, model and
/// usage on the right, dropped first when space runs out.
pub fn footer(theme: &Theme, left: Vec<Span>, right: &str, width: usize) -> Line {
    let avail = width.saturating_sub(4);
    let mut l = Line::raw("  ");
    let left_line = clip(Line::from_spans(left), avail);
    let lw = left_line.width();
    l.spans.extend(left_line.spans);
    let rw = str_width(right);
    if lw + 2 + rw <= avail {
        l.push(" ".repeat(avail - lw - rw), Style::new());
        l.push(right, theme.dim());
    }
    l
}

/// Footer text for the current mode.
pub fn mode_hint(theme: &Theme, mode: InputMode) -> Vec<Span> {
    match mode {
        InputMode::Default => vec![Span::styled("? for shortcuts", theme.dim())],
        InputMode::AcceptEdits => vec![
            Span::styled("⏵⏵ accept edits on", Style::new().fg(Color::Yellow)),
            Span::styled(" · shift+tab to switch", theme.dim()),
        ],
        InputMode::Plan => vec![
            Span::styled("⏸ plan mode on", Style::new().fg(Color::Cyan)),
            Span::styled(" · shift+tab to switch", theme.dim()),
        ],
    }
}

/// The shortcut list shown when the input holds just `?`.
pub fn shortcuts(theme: &Theme, width: usize) -> Vec<Line> {
    let cols = [
        ["/ for commands", "@ to mention files", "↑ for history"],
        ["shift+tab to switch modes", "\\ + enter for a newline", "ctrl+c twice to exit"],
        ["esc to interrupt", "ctrl+o to expand output", "ctrl+l to redraw"],
    ];
    let col_w = (width.saturating_sub(2)) / 3;
    (0..3)
        .map(|r| {
            let mut l = Line::raw("  ");
            for c in &cols {
                let cell = truncate(c[r], col_w.saturating_sub(1));
                let pad = col_w.saturating_sub(str_width(&cell));
                l.push(cell, theme.dim());
                l.push(" ".repeat(pad), Style::new());
            }
            l
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::EditKey;

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.plain_text()).collect()
    }

    #[test]
    fn input_box_shows_placeholder_and_parks_the_cursor() {
        let (lines, cursor) = input_box(&Theme::default(), &Editor::new(), InputMode::Default, "Ask anything", 20, 8);
        assert_eq!(text(&lines), vec!["╭──────────────────╮", "│ › Ask anything   │", "╰──────────────────╯"]);
        assert_eq!(cursor, (1, 4));
    }

    #[test]
    fn input_box_wraps_and_follows_the_cursor() {
        let mut e = Editor::new();
        e.handle(EditKey::Text("abcdefghijklmnopqrstuvwxyz".into()), 14);
        let (lines, cursor) = input_box(&Theme::default(), &e, InputMode::Plan, "", 20, 2);
        assert_eq!(lines.len(), 4, "{:?}", text(&lines));
        assert!(lines.iter().all(|l| l.width() == 20));
        assert_eq!(cursor.0, 2);
        assert_eq!(lines[0].spans[0].style.fg, Color::Cyan);
    }

    #[test]
    fn picker_scrolls_to_keep_the_selection_visible() {
        let items: Vec<PickItem> = (0..10).map(|i| PickItem { label: format!("/cmd{i}"), hint: Some("does a thing".into()), hits: vec![] }).collect();
        let lines = picker(&Theme::default(), &items, 7, 40, 4);
        let t = text(&lines);
        assert!(t[3].contains("/cmd7"));
        assert_eq!(t.last().unwrap().trim(), "8/10");
    }

    #[test]
    fn prompt_box_numbers_options_and_marks_the_selection() {
        let p = PromptView {
            title: "Edit file".into(),
            subtitle: Some("src/auth/session.ts".into()),
            diff_lines: None,
            question: Some("Make this edit?".into()),
            options: vec!["Yes".into(), "Yes, for this session".into(), "No".into()],
            hints: vec![],
            current: None,
            selected: 1,
            esc_is_last: true,
            footer: "↑↓ to choose · enter to confirm".into(),
        };
        let lines = prompt_box(&Theme::default(), &p, 40, 30);
        let t = text(&lines);
        assert!(t.iter().any(|l| l.contains("› 2. Yes, for this session")));
        assert!(t.iter().any(|l| l.contains("3. No  esc")));
        assert!(lines[..lines.len() - 1].iter().all(|l| l.width() == 40), "{t:?}");
    }

    #[test]
    fn footer_drops_the_right_side_when_narrow() {
        let t = Theme::default();
        let wide = footer(&t, mode_hint(&t, InputMode::Default), "kimi-k2.6 · 18.4k tokens · $0.04", 80);
        assert!(wide.plain_text().ends_with("$0.04"));
        let narrow = footer(&t, mode_hint(&t, InputMode::Default), "kimi-k2.6 · 18.4k tokens · $0.04", 30);
        assert_eq!(narrow.plain_text().trim(), "? for shortcuts");
    }

    #[test]
    fn spinner_line_fits_or_drops_its_meta() {
        let t = Theme::default();
        let l = spinner_line(&t, 3, "Reading", 12_000, Some(1234), 80);
        assert_eq!(l.plain_text(), "⠸ Reading… (12s · ↓ 1.2k tokens · esc to interrupt)");
        let narrow = spinner_line(&t, 3, "Reading", 12_000, Some(1234), 20);
        assert_eq!(narrow.plain_text(), "⠸ Reading…");
    }
}
