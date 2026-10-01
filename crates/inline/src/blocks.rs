//! Transcript blocks and how each one looks. This is the visual spec from
//! `docs/inline-redesign.md` §2 in code:
//!
//! - user prompts: a tinted band with a dim `›`, no speaker label
//! - assistant text: markdown indented two columns, no label
//! - tools: one quiet row each (glyph, name, args cut in the middle, then
//!   `+a −r` / failure / elapsed); a running tool adds one faint line of its
//!   latest output; failures show the end of theirs; runs of reads and
//!   searches fold into `Explored 5 files · 3 searches`. Ctrl+O shows the
//!   full layout (summary, diff, output).
//! - one blank row between blocks; consecutive tool rows sit together

use crate::highlight::highlight;
use crate::markdown::render_markdown;
use crate::style::{Line, Span, Style};
use crate::theme::Theme;
use crate::width::{ellipsize_middle, str_width, truncate, wrap};

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Ok,
    Error,
    /// The user declined it (permission denied).
    Declined,
    /// Interrupted before it finished.
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Context,
    Add,
    Del,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub line_no: Option<u32>,
    pub kind: DiffKind,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolBlock {
    pub name: String,
    pub args: String,
    pub status: ToolStatus,
    pub summary: Option<String>,
    /// Captured output, one entry per line.
    pub output: Vec<String>,
    /// Highlight output as this language (e.g. a written file); plain if None.
    pub output_lang: Option<String>,
    /// Output lines shown when collapsed. Errors show more by default.
    pub preview: usize,
    pub diff: Option<(String, Vec<DiffLine>)>,
    pub started_at_ms: Option<i64>,
    pub elapsed_ms: Option<u64>,
}

impl ToolBlock {
    pub fn new(name: impl Into<String>, args: impl Into<String>) -> Self {
        ToolBlock {
            name: name.into(),
            args: args.into(),
            status: ToolStatus::Running,
            summary: None,
            output: Vec::new(),
            output_lang: None,
            preview: 0,
            diff: None,
            started_at_ms: None,
            elapsed_ms: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanItem {
    pub title: String,
    pub status: PlanStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Success,
    Warn,
    Error,
    /// A turn was interrupted; printed under the last row.
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Welcome { title: String, detail: Vec<String> },
    User { text: String },
    Assistant { markdown: String },
    /// The model's reasoning before a reply. Drawn only when the user has
    /// turned reasoning on (Ctrl+R); the session skips it otherwise.
    Reasoning { text: String },
    Tool(ToolBlock),
    Plan { items: Vec<PlanItem>, live: bool },
    Notice { kind: NoticeKind, text: String },
    /// Pre-styled lines from the host (the generic escape hatch).
    Custom { lines: Vec<Line> },
    /// A run of read-only steps (reads, searches, listings) folded into one
    /// row: `✓ Explored 5 files · 3 searches`.
    Group(GroupBlock),
}

/// Consecutive exploring tools. Open while more may join; done once closed
/// and every tool has finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupBlock {
    pub tools: Vec<ToolBlock>,
    pub closed: bool,
}

/// What a read-only tool counts as in a group summary, or None when the
/// tool changes something (and so gets its own row).
pub fn explore_kind(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    Some(match n.as_str() {
        "read" | "view" | "cat" | "open" | "read_file" => "file",
        "grep" | "glob" | "find" | "search" | "rg" | "codesearch" | "code_search" | "web_search" | "websearch" => "search",
        "ls" | "list" | "list_dir" | "tree" => "folder",
        "web_fetch" | "webfetch" | "fetch" => "page",
        _ if n.starts_with("lsp_") => "lookup",
        _ => return None,
    })
}

fn plural(n: usize, noun: &str) -> String {
    match (n, noun) {
        (1, _) => format!("1 {noun}"),
        (_, "search") => format!("{n} searches"),
        _ => format!("{n} {noun}s"),
    }
}

/// Per-frame context: which spinner frame to show, the current time for
/// elapsed counters, and whether collapsed output is expanded.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderCtx {
    pub frame: usize,
    pub now_ms: i64,
    pub expanded: bool,
}

/// Whether a blank row separates `prev` and `next`.
pub fn gap_between(prev: &Block, next: &Block) -> bool {
    let tight = |b: &Block| matches!(b, Block::Tool(_) | Block::Group(_) | Block::Notice { .. });
    !(tight(prev) && tight(next))
}

pub fn render_block(block: &Block, width: usize, theme: &Theme, ctx: RenderCtx) -> Vec<Line> {
    match block {
        Block::Welcome { title, detail } => {
            let mut out = vec![Line::new().with(title.clone(), theme.accent().bold())];
            for d in detail {
                out.extend(wrap(&Line::styled(d.clone(), theme.dim()), width, &Line::new()));
            }
            out
        }
        Block::User { text } => render_user(text, width, theme),
        Block::Assistant { markdown } => indent_lines(render_markdown(markdown, width.saturating_sub(2), theme), 2),
        Block::Reasoning { text } => render_reasoning(text, width, theme),
        Block::Tool(t) => render_tool(t, width, theme, ctx),
        Block::Plan { items, live } => render_plan(items, *live, width, theme),
        Block::Notice { kind, text } => render_notice(*kind, text, width, theme),
        Block::Custom { lines } => lines.clone(),
        Block::Group(g) => render_group(g, width, theme, ctx),
    }
}

fn render_group(g: &GroupBlock, width: usize, theme: &Theme, ctx: RenderCtx) -> Vec<Line> {
    if ctx.expanded || g.tools.len() == 1 {
        return g.tools.iter().flat_map(|t| render_tool(t, width, theme, ctx)).collect();
    }
    let dim = theme.dim();
    let live = !g.closed || g.tools.iter().any(|t| t.status == ToolStatus::Running);
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for t in &g.tools {
        let kind = explore_kind(&t.name).unwrap_or("step");
        match counts.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((kind, 1)),
        }
    }
    let failed = g.tools.iter().filter(|t| t.status == ToolStatus::Error).count();
    let glyph = if live {
        Span::styled(SPINNER[ctx.frame % SPINNER.len()], theme.accent())
    } else if failed == g.tools.len() {
        Span::styled("✗", theme.err())
    } else {
        Span::styled("✓", theme.ok())
    };
    let mut head = Line::from_spans(vec![glyph, Span::raw(" ")]);
    head.push(if live { "Exploring" } else { "Explored" }, Style::new());
    let summary = counts.iter().map(|(k, n)| plural(*n, k)).collect::<Vec<_>>().join(" · ");
    head.push(format!(" {summary}"), dim);
    if failed > 0 {
        head.push(format!(" · {failed} failed"), theme.warn());
    }
    if !live {
        let start = g.tools.iter().filter_map(|t| t.started_at_ms).min();
        let end = g.tools.iter().filter_map(|t| Some(t.started_at_ms? + t.elapsed_ms? as i64)).max();
        if let (Some(s), Some(e)) = (start, end) {
            if e - s >= 1000 {
                head.push(format!(" · {}", format_elapsed((e - s) as u64)), dim);
            }
        }
    }
    let mut out = vec![one_line(head, width)];
    if live {
        if let Some(t) = g.tools.iter().rev().find(|t| t.status == ToolStatus::Running).or(g.tools.last()) {
            let text = format!("{} {}", t.name, flat(&t.args));
            out.push(Line::styled("  └ ", dim).with(ellipsize_middle(&text, width.saturating_sub(4)), dim));
        }
    }
    out
}

/// Args on one row: newlines shown as ⏎.
fn flat(s: &str) -> String {
    s.trim().replace('\n', " ⏎ ")
}

/// Cut a styled line to one row.
fn one_line(line: Line, width: usize) -> Line {
    let mut out = Line::new();
    let mut left = width;
    for sp in line.spans {
        if left == 0 {
            break;
        }
        let w = str_width(&sp.text);
        if w <= left {
            left -= w;
            out.spans.push(sp);
        } else {
            out.push(truncate(&sp.text, left), sp.style);
            left = 0;
        }
    }
    out
}

fn indent_lines(lines: Vec<Line>, n: usize) -> Vec<Line> {
    lines
        .into_iter()
        .map(|mut l| {
            if !l.spans.is_empty() {
                l.spans.insert(0, Span::raw(" ".repeat(n)));
            }
            l
        })
        .collect()
}

/// Split user text so `@mentions` show in the accent color.
fn mention_spans(text: &str, theme: &Theme) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            plain.push(' ');
        }
        if word.len() > 1 && word.starts_with('@') {
            if !plain.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut plain)));
            }
            spans.push(Span::styled(word, theme.accent()));
        } else {
            plain.push_str(word);
        }
    }
    if !plain.is_empty() {
        spans.push(Span::raw(plain));
    }
    spans
}

fn render_user(text: &str, width: usize, theme: &Theme) -> Vec<Line> {
    let mut out = Vec::new();
    for (i, para) in text.split('\n').enumerate() {
        let mut line = Line::new();
        line.fill = theme.user_band;
        line.push(if i == 0 { "› " } else { "  " }, theme.dim());
        line.spans.extend(mention_spans(para, theme));
        let mut indent = Line::raw("  ");
        indent.fill = theme.user_band;
        out.extend(wrap(&line, width, &indent));
    }
    out
}

fn status_glyph(t: &ToolBlock, theme: &Theme, ctx: RenderCtx) -> Span {
    match t.status {
        ToolStatus::Running => Span::styled(SPINNER[ctx.frame % SPINNER.len()], theme.accent()),
        ToolStatus::Ok => Span::styled("✓", theme.ok()),
        ToolStatus::Error => Span::styled("✗", theme.err()),
        ToolStatus::Declined => Span::styled("⊘", theme.warn()),
        ToolStatus::Stopped => Span::styled("■", theme.dim()),
    }
}

pub fn format_elapsed(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

fn lang_for_path(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or("")
}

/// A tool as one quiet row: `✓ Read src/a.ts · 2s`. Running tools add one
/// faint line of their latest output; failures show the end of theirs.
/// Ctrl+O (expanded) shows the full layout instead.
fn render_tool(t: &ToolBlock, width: usize, theme: &Theme, ctx: RenderCtx) -> Vec<Line> {
    if ctx.expanded {
        return render_tool_full(t, width, theme, ctx);
    }
    let dim = theme.dim();
    let elapsed = match t.status {
        ToolStatus::Running => t.started_at_ms.map(|s| (ctx.now_ms - s).max(0) as u64),
        _ => t.elapsed_ms,
    }
    .filter(|ms| *ms >= 1000);

    let mut suffix: Vec<Span> = Vec::new();
    let mut add = |text: String, style: Style| {
        suffix.push(Span::styled(" · ", dim));
        suffix.push(Span::styled(text, style));
    };
    match t.status {
        ToolStatus::Ok | ToolStatus::Running => {
            let counts = t.diff.as_ref().map(|(_, d)| crate::diff::count_changes(d)).or_else(|| {
                let s = t.summary.as_deref()?;
                let tail = s.rsplit(" · ").next()?;
                let (a, r) = tail.strip_prefix('+')?.split_once(" −")?;
                Some((a.parse().ok()?, r.parse().ok()?))
            });
            if let Some((a, r)) = counts {
                suffix.push(Span::styled(" · ", dim));
                suffix.push(Span::styled(format!("+{a}"), theme.ok()));
                suffix.push(Span::styled(" ", dim));
                suffix.push(Span::styled(format!("−{r}"), theme.err()));
            }
        }
        ToolStatus::Error => add(t.summary.clone().unwrap_or_else(|| "failed".into()), theme.err()),
        ToolStatus::Declined => add(t.summary.clone().unwrap_or_else(|| "declined".into()), theme.warn()),
        ToolStatus::Stopped => add("stopped".into(), dim),
    }
    if let Some(ms) = elapsed {
        suffix.push(Span::styled(format!(" · {}", format_elapsed(ms)), dim));
    }

    let name_w = str_width(&t.name);
    let suffix_w: usize = suffix.iter().map(|s| str_width(&s.text)).sum();
    let args_room = width.saturating_sub(2 + name_w + 1 + suffix_w);
    let mut head = Line::from_spans(vec![status_glyph(t, theme, ctx), Span::raw(" ")]);
    head.push(t.name.clone(), Style::new());
    let args = flat(&t.args);
    if !args.is_empty() && args_room > 3 {
        head.push(" ", Style::new()).push(ellipsize_middle(&args, args_room - 1), dim);
    }
    head.spans.extend(suffix);
    let mut out = vec![one_line(head, width)];

    let output = trim_trailing_empty(&t.output);
    let room = width.saturating_sub(4);
    match t.status {
        ToolStatus::Running => {
            if let Some(last) = output.iter().rev().find(|l| !l.trim().is_empty()) {
                out.push(Line::styled("  └ ", dim).with(truncate(last.trim(), room), dim));
            }
        }
        ToolStatus::Error => {
            const TAIL: usize = 6;
            let shown = &output[output.len().saturating_sub(TAIL)..];
            if output.len() > shown.len() {
                out.push(Line::styled(format!("    … {} earlier lines", output.len() - shown.len()), dim));
            }
            for l in shown {
                out.push(Line::raw("    ").with(truncate(l.trim_end(), room), theme.err()));
            }
        }
        _ => {}
    }
    out
}

fn render_tool_full(t: &ToolBlock, width: usize, theme: &Theme, ctx: RenderCtx) -> Vec<Line> {
    let dim = theme.dim();
    let mut out = Vec::new();

    let mut head = Line::from_spans(vec![status_glyph(t, theme, ctx), Span::raw(" ")]);
    head.push(t.name.clone(), theme.strong());
    if !t.args.is_empty() {
        head.push(" ", Style::new()).push(t.args.clone(), Style::new());
    }
    out.extend(wrap(&head, width, &Line::raw("  ")));

    let sub = |text: Line| -> Vec<Line> {
        let mut l = Line::styled("  └ ", dim);
        l.spans.extend(text.spans);
        wrap(&l, width, &Line::raw("    "))
    };
    let out_indent = "    ";

    if t.status == ToolStatus::Running {
        let output = trim_trailing_empty(&t.output);
        if output.is_empty() {
            if let Some(start) = t.started_at_ms {
                let ms = (ctx.now_ms - start).max(0) as u64;
                out.extend(sub(Line::styled(format_elapsed(ms), dim)));
            }
        } else {
            for l in output.iter().rev().take(4).rev() {
                out.extend(wrap(&Line::styled(format!("{out_indent}{l}"), dim), width, &Line::raw(out_indent)));
            }
        }
        return out;
    }

    if let Some(summary) = &t.summary {
        let style = match t.status {
            ToolStatus::Error => theme.err(),
            ToolStatus::Declined => theme.warn(),
            _ => dim,
        };
        let mut l = Line::styled(summary.clone(), style);
        if let Some(ms) = t.elapsed_ms.filter(|ms| *ms >= 1000) {
            l.push(format!(" · {}", format_elapsed(ms)), dim);
        }
        out.extend(sub(l));
    }

    if let Some((path, diff)) = &t.diff {
        out.extend(render_diff(path, diff, width, theme));
    }

    let output = trim_trailing_empty(&t.output);
    if !output.is_empty() {
        let show = if ctx.expanded { output.len() } else { t.preview.min(output.len()) };
        let highlighted = t.output_lang.as_deref().map(|lang| highlight(&output[..show].join("\n"), lang));
        for (i, l) in output[..show].iter().enumerate() {
            let mut line = Line::raw(out_indent);
            match &highlighted {
                Some(h) => line.spans.extend(h.get(i).cloned().unwrap_or_default()),
                None if t.status == ToolStatus::Error => {
                    line.push(l.clone(), theme.err());
                }
                None => {
                    line.push(l.clone(), dim);
                }
            }
            out.extend(wrap(&line, width, &Line::raw(out_indent)));
        }
        let rest = output.len() - show;
        if rest > 0 {
            let noun = if rest == 1 { "line" } else { "lines" };
            let what = if show > 0 { format!("… +{rest} more {noun}") } else { format!("… {rest} {noun}") };
            out.push(Line::styled(format!("{out_indent}{what} (ctrl+o to expand)"), dim));
        }
    }
    out
}

fn trim_trailing_empty(lines: &[String]) -> &[String] {
    let mut end = lines.len();
    while end > 0 && lines[end - 1].is_empty() {
        end -= 1;
    }
    &lines[..end]
}

pub fn render_diff(path: &str, diff: &[DiffLine], width: usize, theme: &Theme) -> Vec<Line> {
    let lang = lang_for_path(path);
    let code = diff.iter().map(|d| d.text.as_str()).collect::<Vec<_>>().join("\n");
    let hl = highlight(&code, lang);
    let num_w = diff.iter().filter_map(|d| d.line_no).max().map(|n| n.to_string().len()).unwrap_or(1).max(3);
    let mut out = Vec::new();
    for (i, d) in diff.iter().enumerate() {
        let (sign, sign_style, fill) = match d.kind {
            DiffKind::Add => ("+", theme.ok(), theme.diff_add_bg),
            DiffKind::Del => ("-", theme.err(), theme.diff_del_bg),
            DiffKind::Context => (" ", theme.dim(), None),
        };
        let num = d.line_no.map(|n| n.to_string()).unwrap_or_default();
        let mut line = Line::raw("    ");
        line.fill = fill;
        line.push(format!("{num:>num_w$} "), theme.dim());
        line.push(format!("{sign} "), sign_style);
        line.spans.extend(hl.get(i).cloned().unwrap_or_else(|| vec![Span::raw(d.text.clone())]));
        let mut indent = Line::raw(" ".repeat(4 + num_w + 3));
        indent.fill = fill;
        out.extend(wrap(&line, width, &indent));
    }
    out
}

fn render_plan(items: &[PlanItem], live: bool, width: usize, theme: &Theme) -> Vec<Line> {
    let done = items.iter().filter(|i| i.status == PlanStatus::Done).count();
    let mut head = Line::new();
    head.push(if live { "◆ " } else { "◇ " }, if live { theme.accent() } else { theme.dim() });
    head.push("Plan", theme.strong());
    head.push(format!(" · {done} of {} done{}", items.len(), if live { "" } else { " · stopped here" }), theme.dim());
    let mut out = vec![head];
    for it in items {
        let (glyph, gstyle, tstyle) = match it.status {
            PlanStatus::Done => ("✓ ", theme.ok(), theme.dim().strike()),
            PlanStatus::InProgress => ("› ", theme.accent(), theme.strong()),
            PlanStatus::Pending => ("○ ", theme.dim(), Style::new()),
        };
        let line = Line::raw("  ").with(glyph, gstyle).with(it.title.clone(), tstyle);
        out.extend(wrap(&line, width, &Line::raw("    ")));
    }
    out
}

/// Longest reasoning shown, like the Ink UI.
const REASONING_MAX: usize = 400;

fn render_reasoning(text: &str, width: usize, theme: &Theme) -> Vec<Line> {
    let text = text.trim();
    let shown: String = if text.chars().count() > REASONING_MAX {
        text.chars().take(REASONING_MAX).collect::<String>() + "…"
    } else {
        text.to_string()
    };
    let style = theme.dim().italic();
    let mut out = Vec::new();
    for (i, line) in shown.split('\n').enumerate() {
        let head = if i == 0 { Line::raw("  ").with("thinking… ", theme.dim()) } else { Line::raw("  ") };
        out.extend(wrap(&head.with(line, style), width, &Line::raw("  ")));
    }
    out
}

fn render_notice(kind: NoticeKind, text: &str, width: usize, theme: &Theme) -> Vec<Line> {
    let (glyph, gstyle, tstyle) = match kind {
        // Harness chatter (policy notes, "saved", status) is a hint, not
        // part of the reply: dim, so it never reads like the model talking.
        NoticeKind::Info => ("· ", theme.dim(), theme.dim()),
        NoticeKind::Success => ("✓ ", theme.ok(), Style::new()),
        NoticeKind::Warn => ("! ", theme.warn(), theme.warn()),
        NoticeKind::Error => ("✗ ", theme.err(), theme.err()),
        NoticeKind::Interrupted => ("  └ ", theme.err(), theme.err()),
    };
    let indent = " ".repeat(str_width(glyph));
    // Keep the host's line breaks (reports, lists); wrap each line under
    // the glyph.
    let mut out = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let head = if i == 0 { Line::styled(glyph, gstyle) } else { Line::raw(indent.clone()) };
        out.extend(wrap(&head.with(line, tstyle), width, &Line::raw(indent.clone())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.plain_text()).collect()
    }

    fn ctx() -> RenderCtx {
        RenderCtx { frame: 0, now_ms: 10_000, expanded: false }
    }

    #[test]
    fn user_prompt_is_a_band_with_a_dim_marker() {
        let t = Theme::default();
        let lines = render_block(&Block::User { text: "fix @src/a.ts please".into() }, 40, &t, ctx());
        assert_eq!(text(&lines), vec!["› fix @src/a.ts please"]);
        assert_eq!(lines[0].fill, t.user_band);
        assert!(lines[0].spans.iter().any(|s| s.text == "@src/a.ts" && s.style.fg == t.accent));
    }

    #[test]
    fn assistant_text_is_indented_without_a_label() {
        let lines = render_block(&Block::Assistant { markdown: "Found it.".into() }, 40, &Theme::default(), ctx());
        assert_eq!(text(&lines), vec!["  Found it."]);
    }

    #[test]
    fn finished_tool_is_one_quiet_row() {
        let mut t = ToolBlock::new("Read", "src/auth/login.ts");
        t.status = ToolStatus::Ok;
        t.summary = Some("142 lines".into());
        t.started_at_ms = Some(0);
        t.elapsed_ms = Some(2400);
        let lines = render_block(&Block::Tool(t.clone()), 60, &Theme::default(), ctx());
        assert_eq!(text(&lines), vec!["✓ Read src/auth/login.ts · 2s"]);
        let full = render_block(&Block::Tool(t), 60, &Theme::default(), RenderCtx { expanded: true, ..ctx() });
        assert_eq!(text(&full), vec!["✓ Read src/auth/login.ts", "  └ 142 lines · 2s"]);
    }

    #[test]
    fn long_commands_are_cut_in_the_middle() {
        let mut t = ToolBlock::new("Bash", "npx tsx --test src/server/aster-api.test.ts src/server/aster-store.test.ts src/server/aster-tools.test.ts");
        t.status = ToolStatus::Ok;
        let row = text(&render_block(&Block::Tool(t), 60, &Theme::default(), ctx()));
        assert_eq!(row.len(), 1);
        assert!(row[0].starts_with("✓ Bash npx tsx --test"), "{row:?}");
        assert!(row[0].ends_with("aster-tools.test.ts"), "{row:?}");
        assert!(str_width(&row[0]) <= 60);
    }

    #[test]
    fn failed_tool_shows_the_end_of_its_output() {
        let mut t = ToolBlock::new("Bash", "npm test");
        t.status = ToolStatus::Error;
        t.summary = Some("Exit code 1".into());
        t.output = (1..=9).map(|i| format!("line {i}")).collect();
        let lines = text(&render_block(&Block::Tool(t), 60, &Theme::default(), ctx()));
        assert_eq!(lines[0], "✗ Bash npm test · Exit code 1");
        assert_eq!(lines[1], "    … 3 earlier lines");
        assert_eq!(lines.last().unwrap(), "    line 9");
    }

    #[test]
    fn explore_groups_fold_into_one_row() {
        let tool = |name: &str, args: &str| {
            let mut t = ToolBlock::new(name, args);
            t.status = ToolStatus::Ok;
            t.started_at_ms = Some(0);
            t.elapsed_ms = Some(1500);
            t
        };
        let g = GroupBlock { tools: vec![tool("Read", "a.ts"), tool("Read", "b.ts"), tool("Grep", "foo"), tool("Glob", "*.rs")], closed: true };
        assert_eq!(text(&render_block(&Block::Group(g.clone()), 60, &Theme::default(), ctx())), vec!["✓ Explored 2 files · 2 searches · 1s"]);
        let mut live = g;
        live.closed = false;
        live.tools[3].status = ToolStatus::Running;
        assert_eq!(text(&render_block(&Block::Group(live), 60, &Theme::default(), ctx())), vec!["⠋ Exploring 2 files · 2 searches", "  └ Glob *.rs"]);
    }

    #[test]
    fn running_tool_tails_its_output() {
        let mut t = ToolBlock::new("Bash", "npm test");
        t.output = (1..=6).map(|i| format!("line {i}")).collect();
        let lines = render_block(&Block::Tool(t), 60, &Theme::default(), ctx());
        assert_eq!(text(&lines), vec!["⠋ Bash npm test", "  └ line 6"]);
    }

    #[test]
    fn collapsed_output_offers_expansion() {
        let mut t = ToolBlock::new("Search", "\"issueSession\"");
        t.status = ToolStatus::Ok;
        t.summary = Some("4 matches".into());
        t.output = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let collapsed = text(&render_block(&Block::Tool(t.clone()), 60, &Theme::default(), ctx()));
        assert_eq!(collapsed, vec!["✓ Search \"issueSession\""]);
        let expanded = text(&render_block(&Block::Tool(t), 60, &Theme::default(), RenderCtx { expanded: true, ..ctx() }));
        assert_eq!(expanded.len(), 2 + 4);
    }

    #[test]
    fn info_notices_are_dim_hints() {
        let lines = render_block(&Block::Notice { kind: NoticeKind::Info, text: "Subagent policy: auto".into() }, 60, &Theme::default(), ctx());
        assert!(lines[0].spans.iter().all(|s| s.text.trim().is_empty() || s.style == Theme::default().dim()), "{:?}", lines[0]);
    }

    #[test]
    fn diffs_show_line_numbers_signs_and_tints() {
        let t = Theme::default();
        let diff = vec![
            DiffLine { line_no: Some(44), kind: DiffKind::Del, text: "// TODO".into() },
            DiffLine { line_no: Some(44), kind: DiffKind::Add, text: "rotate()".into() },
        ];
        let lines = render_diff("src/a.ts", &diff, 60, &t);
        assert_eq!(text(&lines), vec!["     44 - // TODO", "     44 + rotate()"]);
        assert_eq!(lines[0].fill, t.diff_del_bg);
        assert_eq!(lines[1].fill, t.diff_add_bg);
    }

    #[test]
    fn plan_marks_progress() {
        let items = vec![
            PlanItem { title: "Trace".into(), status: PlanStatus::Done },
            PlanItem { title: "Rotate".into(), status: PlanStatus::InProgress },
            PlanItem { title: "Test".into(), status: PlanStatus::Pending },
        ];
        let lines = render_block(&Block::Plan { items, live: true }, 60, &Theme::default(), ctx());
        assert_eq!(text(&lines), vec!["◆ Plan · 1 of 3 done", "  ✓ Trace", "  › Rotate", "  ○ Test"]);
    }

    #[test]
    fn notices_keep_line_breaks() {
        let lines = render_block(&Block::Notice { kind: NoticeKind::Info, text: "Session  $0.01\nToday    $0.48".into() }, 60, &Theme::default(), ctx());
        assert_eq!(text(&lines), vec!["· Session  $0.01", "  Today    $0.48"]);
    }

    #[test]
    fn tool_rows_sit_together_but_prose_gets_a_gap() {
        let tool = Block::Tool(ToolBlock::new("Read", "a"));
        let prose = Block::Assistant { markdown: "x".into() };
        assert!(!gap_between(&tool, &tool));
        assert!(gap_between(&prose, &tool));
    }
}
