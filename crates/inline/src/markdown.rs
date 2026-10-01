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
    let src = ansi_to_tags(src);
    let src: &str = &src;
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
    /// Open inline HTML tags, so `</span>` pops the style its `<span>` pushed.
    html: Vec<String>,
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
            html: Vec::new(),
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
            Event::Html(t) | Event::InlineHtml(t) => self.html(&t),
            Event::FootnoteReference(t) => self.text(&format!("[{t}]")),
            _ => {}
        }
    }

    /// Inline HTML from model text. Formatting tags become terminal styles
    /// (`<span style="color: red">`, `<font color>`, `<b>`, `<i>`, `<u>`,
    /// `<s>`, `<mark>`, `<br>`); other tags are dropped so raw markup never
    /// shows; text between tags (in HTML blocks) is kept.
    fn html(&mut self, raw: &str) {
        let mut rest = raw;
        while !rest.is_empty() {
            let Some(lt) = rest.find('<') else {
                self.text(rest.trim_end_matches('\n'));
                break;
            };
            if lt > 0 {
                self.text(&rest[..lt]);
            }
            let Some(gt) = rest[lt..].find('>') else {
                self.text(&rest[lt..]);
                break;
            };
            self.html_tag(&rest[lt + 1..lt + gt]);
            rest = &rest[lt + gt + 1..];
        }
    }

    fn html_tag(&mut self, inner: &str) {
        let inner = inner.trim();
        // ANSI color codes, rewritten as <ansi-…> tags before parsing.
        if let Some(params) = inner.strip_prefix("ansi-") {
            let params = params.replace('-', ";");
            if params.is_empty() || params == "0" {
                while let Some(pos) = self.html.iter().rposition(|t| t == "ansi") {
                    for _ in pos..self.html.len() {
                        self.pop_style();
                    }
                    self.html.truncate(pos);
                }
            } else {
                let st = crate::ansi::apply_sgr(Style::new(), &params);
                self.push_style(st);
                self.html.push("ansi".into());
            }
            return;
        }
        if let Some(name) = inner.strip_prefix('/') {
            let name = name.trim().to_ascii_lowercase();
            if let Some(pos) = self.html.iter().rposition(|t| *t == name) {
                for _ in pos..self.html.len() {
                    self.pop_style();
                }
                self.html.truncate(pos);
            }
            return;
        }
        let name: String = inner.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        let attrs = inner[name.len()..].to_ascii_lowercase();
        if name == "br" {
            self.flush_inline();
            return;
        }
        let style = match name.as_str() {
            "b" | "strong" => Some(Style::new().bold()),
            "i" | "em" => Some(Style::new().italic()),
            "u" | "ins" => Some(Style::new().underline()),
            "s" | "del" | "strike" => Some(Style::new().strike()),
            "mark" => Some(Style::new().bg(crate::style::Color::Yellow).fg(crate::style::Color::Black)),
            "code" | "kbd" => Some(self.theme.inline_code()),
            "span" | "font" | "div" | "p" => {
                let mut st = Style::new();
                if let Some(c) = css_value(&attrs, "color").or_else(|| attr_value(&attrs, "color")).and_then(|v| html_color(&v)) {
                    st = st.fg(c);
                }
                if let Some(c) = css_value(&attrs, "background-color").or_else(|| css_value(&attrs, "background")).and_then(|v| html_color(&v)) {
                    st = st.bg(c);
                }
                if css_value(&attrs, "font-weight").map(|w| w == "bold" || w.parse::<u32>().map(|n| n >= 600).unwrap_or(false)).unwrap_or(false) {
                    st = st.bold();
                }
                if css_value(&attrs, "font-style").as_deref() == Some("italic") {
                    st = st.italic();
                }
                Some(st)
            }
            _ => None,
        };
        // Self-closing tags (`<br/>`, `<span/>`) open nothing.
        if let Some(st) = style {
            if !inner.ends_with('/') {
                self.push_style(st);
                self.html.push(name);
            }
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

/// Rewrite ANSI SGR codes as `<ansi-…>` tags (other escapes are dropped).
/// pulldown-cmark splits text at `[`, so escapes can't be handled per text
/// event; as tags they arrive whole and nest with the markdown.
fn ansi_to_tags(src: &str) -> std::borrow::Cow<'_, str> {
    if !src.contains('\x1b') {
        return std::borrow::Cow::Borrowed(src);
    }
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        if chars.peek() != Some(&'[') {
            chars.next();
            continue;
        }
        chars.next();
        let mut params = String::new();
        let mut fin = None;
        for p in chars.by_ref() {
            if ('\x40'..='\x7e').contains(&p) {
                fin = Some(p);
                break;
            }
            params.push(p);
        }
        if fin == Some('m') && params.chars().all(|c| c.is_ascii_digit() || c == ';' || c == ':') {
            out.push_str(&format!("<ansi-{}>", params.replace([';', ':'], "-")));
        }
    }
    std::borrow::Cow::Owned(out)
}

/// `color: red` out of a `style="…"` attribute.
fn css_value(attrs: &str, prop: &str) -> Option<String> {
    let style = attr_value(attrs, "style")?;
    style.split(';').find_map(|decl| {
        let (k, v) = decl.split_once(':')?;
        (k.trim() == prop).then(|| v.trim().trim_end_matches("!important").trim().to_string())
    })
}

/// The value of `name="…"`, `name='…'` or `name=bare`.
fn attr_value(attrs: &str, name: &str) -> Option<String> {
    let mut search = attrs;
    while let Some(i) = search.find(name) {
        let before_ok = i == 0 || !search.as_bytes()[i - 1].is_ascii_alphanumeric() && search.as_bytes()[i - 1] != b'-';
        let after = search[i + name.len()..].trim_start();
        if before_ok {
            if let Some(v) = after.strip_prefix('=') {
                let v = v.trim_start();
                return Some(match v.chars().next() {
                    Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or("").to_string(),
                    _ => v.split_whitespace().next().unwrap_or("").to_string(),
                });
            }
        }
        search = &search[i + name.len()..];
    }
    None
}

/// CSS color names and hex values, mapped onto the terminal palette (hex
/// stays truecolor).
fn html_color(v: &str) -> Option<crate::style::Color> {
    use crate::style::Color;
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let full = match hex.len() {
            3 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
            6 => hex.to_string(),
            _ => return None,
        };
        let n = u32::from_str_radix(&full, 16).ok()?;
        return Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8));
    }
    Some(match v {
        "red" | "crimson" | "darkred" => Color::Red,
        "green" | "lime" | "darkgreen" | "seagreen" => Color::Green,
        "yellow" | "gold" => Color::Yellow,
        "blue" | "navy" | "royalblue" | "dodgerblue" => Color::Blue,
        "magenta" | "purple" | "violet" | "fuchsia" | "orchid" => Color::Magenta,
        "cyan" | "teal" | "aqua" | "turquoise" => Color::Cyan,
        "orange" | "darkorange" => Color::Indexed(208),
        "pink" | "hotpink" => Color::Indexed(205),
        "gray" | "grey" | "silver" | "darkgray" | "darkgrey" => Color::BrightBlack,
        "white" => Color::White,
        "black" => Color::Black,
        _ => return None,
    })
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
    fn inline_html_colors_become_terminal_colors() {
        use crate::style::Color;
        let lines = render_markdown(r##"Here it is in <span style="color: red;">red</span>, <b>bold</b> and <font color="#00ff00">green</font>."##, 60, &Theme::default());
        assert_eq!(text(&lines), vec!["Here it is in red, bold and green."]);
        let find = |t: &str| lines[0].spans.iter().find(|s| s.text == t).unwrap().style;
        assert_eq!(find("red").fg, Color::Red);
        assert!(find("bold").bold);
        assert_eq!(find("green").fg, Color::Rgb(0, 255, 0));
        assert!(find(", ").is_plain(), "styles end at the closing tag");
    }

    #[test]
    fn unknown_tags_never_show_as_raw_markup() {
        assert_eq!(md("a <details>b</details> c<br>d", 40), vec!["a b c", "d"]);
    }

    #[test]
    fn ansi_color_in_text_is_kept() {
        let lines = render_markdown("status: \x1b[32mok\x1b[0m done", 40, &Theme::default());
        assert_eq!(text(&lines), vec!["status: ok done"]);
        assert_eq!(lines[0].spans.iter().find(|s| s.text == "ok").unwrap().style.fg, crate::style::Color::Green);
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
