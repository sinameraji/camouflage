//! CommonMark to wrapped terminal lines.
//!
//! Headings are bold, lists hang-indent under their markers, quotes and code
//! get a dim `│` gutter, code is syntax highlighted, and tables are drawn with
//! box-drawing separators (falling back to `a · b` rows when too wide). No
//! row is full-width, so printed output survives terminal resizes.

use crate::highlight::highlight;
use crate::style::{Line, Span, Style};
use crate::theme::Theme;
use crate::width::{str_width, wrap};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

pub fn render_markdown(src: &str, width: usize, theme: &Theme) -> Vec<Line> {
    let mut r = Renderer::new(width.max(10), theme);
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(src, opts) {
        r.event(ev);
    }
    r.flush_inline();
    r.out
}

struct TableState {
    rows: Vec<Vec<Vec<Span>>>,
    cell: Vec<Span>,
}

struct Renderer<'t> {
    width: usize,
    theme: &'t Theme,
    out: Vec<Line>,
    inline: Vec<Span>,
    styles: Vec<Style>,
    /// One entry per open list: the next ordinal, or None for bullets.
    lists: Vec<Option<u64>>,
    /// Width of each open list item's marker, for hanging indents.
    item_indents: Vec<usize>,
    item_marker: Option<String>,
    quote: usize,
    code: Option<(String, String)>,
    table: Option<TableState>,
    link: Option<String>,
}

impl<'t> Renderer<'t> {
    fn new(width: usize, theme: &'t Theme) -> Self {
        Renderer {
            width,
            theme,
            out: Vec::new(),
            inline: Vec::new(),
            styles: vec![Style::new()],
            lists: Vec::new(),
            item_indents: Vec::new(),
            item_marker: None,
            quote: 0,
            code: None,
            table: None,
            link: None,
        }
    }

    fn style(&self) -> Style {
        *self.styles.last().unwrap_or(&Style::new())
    }

    fn push_style(&mut self, s: Style) {
        let next = self.style().patch(s);
        self.styles.push(next);
    }

    fn pop_style(&mut self) {
        if self.styles.len() > 1 {
            self.styles.pop();
        }
    }

    fn text(&mut self, t: &str) {
        if let Some(table) = self.table.as_mut() {
            let style = *self.styles.last().unwrap();
            table.cell.push(Span::styled(t, style));
            return;
        }
        let style = self.style();
        match self.inline.last_mut() {
            Some(last) if last.style == style => last.text.push_str(t),
            _ => self.inline.push(Span::styled(t, style)),
        }
    }

    /// Blank line between blocks, except at the very top and inside lists.
    fn gap(&mut self) {
        if !self.out.is_empty() && self.lists.is_empty() && self.out.last().map(|l| !l.spans.is_empty()).unwrap_or(false) {
            self.out.push(Line::new());
        }
    }

    fn prefix(&self, gutter: Option<&str>) -> (Line, Line) {
        let dim = self.theme.dim();
        let mut first = Line::new();
        let mut cont = Line::new();
        for _ in 0..self.quote {
            first.push("│ ", dim);
            cont.push("│ ", dim);
        }
        let outer: usize = self.item_indents.iter().take(self.item_indents.len().saturating_sub(1)).sum();
        let own = self.item_indents.last().copied().unwrap_or(0);
        if outer + own > 0 {
            match &self.item_marker {
                Some(marker) => {
                    first.push(" ".repeat(outer), Style::new());
                    first.push(marker.clone(), dim);
                }
                None => {
                    first.push(" ".repeat(outer + own), Style::new());
                }
            }
            cont.push(" ".repeat(outer + own), Style::new());
        }
        if let Some(g) = gutter {
            first.push(g, dim);
            cont.push(g, dim);
        }
        (first, cont)
    }

    fn flush_inline(&mut self) {
        if self.inline.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.inline);
        let (mut first, cont) = self.prefix(None);
        self.item_marker = None;
        first.spans.extend(spans);
        self.out.extend(wrap(&first, self.width, &cont));
    }

    fn event(&mut self, ev: Event) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if let Some((_, code)) = self.code.as_mut() {
                    code.push_str(&t);
                } else {
                    self.text(&t);
                }
            }
            Event::Code(t) => {
                let style = self.style().patch(self.theme.inline_code());
                if let Some(table) = self.table.as_mut() {
                    table.cell.push(Span::styled(t.to_string(), style));
                } else {
                    self.inline.push(Span::styled(t.to_string(), style));
                }
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.flush_inline(),
            Event::Rule => {
                self.flush_inline();
                self.gap();
                let n = self.width.min(24);
                self.out.push(Line::styled("─".repeat(n), self.theme.dim()));
            }
            Event::TaskListMarker(done) => self.text(if done { "[x] " } else { "[ ] " }),
            Event::Html(t) | Event::InlineHtml(t) => {
                let dim = self.theme.dim();
                self.push_style(dim);
                self.text(t.trim_end_matches('\n'));
                self.pop_style();
            }
            Event::FootnoteReference(t) => self.text(&format!("[{t}]")),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                if self.item_marker.is_none() {
                    self.gap();
                }
            }
            Tag::Heading { .. } => {
                self.flush_inline();
                self.gap();
                self.push_style(Style::new().bold());
            }
            Tag::BlockQuote(_) => {
                self.flush_inline();
                self.gap();
                self.quote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush_inline();
                if self.item_marker.is_none() {
                    self.gap();
                }
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush_inline();
                if self.lists.is_empty() {
                    self.gap();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush_inline();
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                self.item_indents.push(str_width(&marker));
                self.item_marker = Some(marker);
            }
            Tag::Emphasis => self.push_style(Style::new().italic()),
            Tag::Strong => self.push_style(Style::new().bold()),
            Tag::Strikethrough => self.push_style(Style::new().strike()),
            Tag::Link { dest_url, .. } => {
                self.push_style(Style::new().underline());
                self.link = Some(dest_url.to_string());
            }
            Tag::Image { .. } => self.push_style(self.theme.dim()),
            Tag::Table(_) => {
                self.flush_inline();
                self.gap();
                self.table = Some(TableState { rows: Vec::new(), cell: Vec::new() });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.rows.push(Vec::new());
                }
            }
            Tag::TableCell => {
                if let Some(t) = self.table.as_mut() {
                    t.cell.clear();
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush_inline(),
            TagEnd::Heading(_) => {
                self.flush_inline();
                self.pop_style();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_inline();
                self.quote = self.quote.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                if let Some((lang, code)) = self.code.take() {
                    let (first, cont) = self.prefix(Some("│ "));
                    self.item_marker = None;
                    for (i, spans) in highlight(&code, &lang).into_iter().enumerate() {
                        let mut line = if i == 0 { first.clone() } else { cont.clone() };
                        line.spans.extend(spans);
                        self.out.extend(wrap(&line, self.width, &cont));
                    }
                }
            }
            TagEnd::List(_) => {
                self.flush_inline();
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush_inline();
                self.item_indents.pop();
                self.item_marker = None;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Image => self.pop_style(),
            TagEnd::Link => {
                self.pop_style();
                if let Some(url) = self.link.take() {
                    let shown: String = self.inline.iter().map(|s| s.text.as_str()).collect();
                    if url.starts_with("http") && !shown.ends_with(&url) {
                        let dim = self.theme.dim();
                        self.push_style(dim);
                        self.text(&format!(" ({url})"));
                        self.pop_style();
                    }
                }
            }
            TagEnd::TableCell => {
                if let Some(t) = self.table.as_mut() {
                    let cell = std::mem::take(&mut t.cell);
                    if let Some(row) = t.rows.last_mut() {
                        row.push(cell);
                    }
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.render_table(t.rows);
                }
            }
            _ => {}
        }
    }

    fn render_table(&mut self, rows: Vec<Vec<Vec<Span>>>) {
        let dim = self.theme.dim();
        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let cell_w = |c: &Vec<Span>| c.iter().map(|s| s.width()).sum::<usize>();
        let widths: Vec<usize> = (0..cols)
            .map(|i| rows.iter().map(|r| r.get(i).map(cell_w).unwrap_or(0)).max().unwrap_or(0))
            .collect();
        let total = widths.iter().sum::<usize>() + 3 * (cols - 1);
        let (_, indent) = self.prefix(None);

        if total + indent.width() > self.width {
            // Too wide for a grid: one wrapped `a · b · c` line per data row.
            for row in rows.iter().skip(1) {
                let mut line = indent.clone();
                for (i, cell) in row.iter().enumerate() {
                    if i > 0 {
                        line.push(" · ", dim);
                    }
                    line.spans.extend(cell.iter().cloned());
                }
                self.out.extend(wrap(&line, self.width, &indent));
            }
            return;
        }

        for (ri, row) in rows.iter().enumerate() {
            let mut line = indent.clone();
            for (i, col_w) in widths.iter().enumerate() {
                if i > 0 {
                    line.push(" │ ", dim);
                }
                let empty = Vec::new();
                let cell = row.get(i).unwrap_or(&empty);
                for s in cell {
                    let style = if ri == 0 { s.style.bold() } else { s.style };
                    line.spans.push(Span::styled(s.text.clone(), style));
                }
                let pad = col_w - cell_w(cell);
                if pad > 0 && i + 1 < cols {
                    line.push(" ".repeat(pad), Style::new());
                }
            }
            self.out.push(line);
            if ri == 0 {
                let mut sep = indent.clone();
                let bar: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
                sep.push(bar.join("─┼─"), dim);
                self.out.push(sep);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.plain_text()).collect()
    }

    fn md(src: &str, width: usize) -> Vec<String> {
        text(&render_markdown(src, width, &Theme::default()))
    }

    #[test]
    fn paragraphs_are_separated_by_one_blank_line() {
        assert_eq!(md("one\n\ntwo", 40), vec!["one", "", "two"]);
    }

    #[test]
    fn lists_hang_indent_under_their_markers() {
        let out = md("- alpha beta gamma delta\n- two\n\n1. first\n2. second", 14);
        assert_eq!(out, vec!["• alpha beta", "  gamma delta", "• two", "", "1. first", "2. second"]);
    }

    #[test]
    fn nested_lists_indent_further() {
        let out = md("- outer\n  - inner", 40);
        assert_eq!(out, vec!["• outer", "  • inner"]);
    }

    #[test]
    fn code_blocks_get_a_gutter_and_highlighting() {
        let lines = render_markdown("```ts\nconst a = 1\n```", 40, &Theme::default());
        assert_eq!(text(&lines), vec!["│ const a = 1"]);
        let kw = lines[0].spans.iter().find(|s| s.text.contains("const")).unwrap();
        assert_eq!(kw.style.fg, crate::style::Color::Magenta);
    }

    #[test]
    fn tables_draw_a_grid_with_a_bold_header() {
        let lines = render_markdown("| File | Change |\n|---|---|\n| `a.ts` | added |", 40, &Theme::default());
        assert_eq!(text(&lines), vec!["File │ Change", "─────┼───────", "a.ts │ added"]);
        assert!(lines[0].spans[0].style.bold);
    }

    #[test]
    fn tables_too_wide_fall_back_to_rows() {
        let out = md("| a | b |\n|---|---|\n| a long cell value | another long value |", 20);
        assert_eq!(out, vec!["a long cell value ·", "another long value"]);
    }

    #[test]
    fn quotes_get_a_gutter() {
        assert_eq!(md("> quoted text", 40), vec!["│ quoted text"]);
    }

    #[test]
    fn inline_styles_become_span_styles() {
        let lines = render_markdown("**bold** and `code` and *it*", 40, &Theme::default());
        let spans = &lines[0].spans;
        assert!(spans.iter().any(|s| s.text == "bold" && s.style.bold));
        assert!(spans.iter().any(|s| s.text == "code" && s.style.fg == crate::style::Color::Blue));
        assert!(spans.iter().any(|s| s.text == "it" && s.style.italic));
    }

    #[test]
    fn no_row_exceeds_the_width() {
        let src = "# A heading that is fairly long\n\nSome paragraph text that will need wrapping across rows.\n\n- item with enough words to wrap\n\n```rust\nfn main() { println!(\"a very long line of code that wraps\"); }\n```";
        for w in [12, 20, 33] {
            for l in render_markdown(src, w, &Theme::default()) {
                assert!(l.width() <= w, "{:?} is wider than {w}", l.plain_text());
            }
        }
    }
}
