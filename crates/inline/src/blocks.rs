//! Transcript blocks and how each one looks. This is the visual spec from
//! `docs/inline-redesign.md` §2 in code:
//!
//! - user prompts: a tinted band with a dim `›`, no speaker label
//! - assistant text: markdown indented two columns, no label
//! - tools: status glyph in column 0, **name** and args, then `└ summary`,
//!   inline diffs with line numbers, collapsible output
//! - one blank row between blocks; consecutive tool rows sit together

use crate::highlight::highlight;
use crate::markdown::render_markdown;
use crate::style::{Line, Span, Style};
use crate::theme::Theme;
use crate::width::{str_width, wrap};

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
    Tool(ToolBlock),
    Plan { items: Vec<PlanItem>, live: bool },
    Notice { kind: NoticeKind, text: String },
    /// Pre-styled lines from the host (the generic escape hatch).
    Custom { lines: Vec<Line> },
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
    let tight = |b: &Block| matches!(b, Block::Tool(_) | Block::Notice { .. });
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
        Block::Tool(t) => render_tool(t, width, theme, ctx),
        Block::Plan { items, live } => render_plan(items, *live, width, theme),
        Block::Notice { kind, text } => render_notice(*kind, text, width, theme),
        Block::Custom { lines } => lines.clone(),
    }
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

fn render_tool(t: &ToolBlock, width: usize, theme: &Theme, ctx: RenderCtx) -> Vec<Line> {
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

fn render_notice(kind: NoticeKind, text: &str, width: usize, theme: &Theme) -> Vec<Line> {
    let (glyph, gstyle, tstyle) = match kind {
        NoticeKind::Info => ("· ", theme.dim(), Style::new()),
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
    fn finished_tool_shows_glyph_name_args_and_summary() {
        let mut t = ToolBlock::new("Read", "src/auth/login.ts");
        t.status = ToolStatus::Ok;
        t.summary = Some("142 lines".into());
        let lines = render_block(&Block::Tool(t), 60, &Theme::default(), ctx());
        assert_eq!(text(&lines), vec!["✓ Read src/auth/login.ts", "  └ 142 lines"]);
    }

    #[test]
    fn running_tool_tails_its_output() {
        let mut t = ToolBlock::new("Bash", "npm test");
        t.output = (1..=6).map(|i| format!("line {i}")).collect();
        let lines = render_block(&Block::Tool(t), 60, &Theme::default(), ctx());
        assert_eq!(text(&lines), vec!["⠋ Bash npm test", "    line 3", "    line 4", "    line 5", "    line 6"]);
    }

    #[test]
    fn collapsed_output_offers_expansion() {
        let mut t = ToolBlock::new("Search", "\"issueSession\"");
        t.status = ToolStatus::Ok;
        t.summary = Some("4 matches".into());
        t.output = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let collapsed = text(&render_block(&Block::Tool(t.clone()), 60, &Theme::default(), ctx()));
        assert_eq!(collapsed.last().unwrap(), "    … 4 lines (ctrl+o to expand)");
        let expanded = text(&render_block(&Block::Tool(t), 60, &Theme::default(), RenderCtx { expanded: true, ..ctx() }));
        assert_eq!(expanded.len(), 2 + 4);
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
