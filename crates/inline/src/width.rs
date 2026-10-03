//! Display width and wrapping, measured in terminal cells per grapheme.

use crate::style::{Line, Span, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Cells a grapheme occupies. Emoji presentation sequences count as two.
pub fn grapheme_width(g: &str) -> usize {
    if g == "\t" {
        return 4;
    }
    let w = UnicodeWidthStr::width(g);
    if w == 1 && g.contains('\u{fe0f}') {
        2
    } else {
        w
    }
}

pub fn str_width(s: &str) -> usize {
    // Printable ASCII is one cell per byte. Most rows are plain ASCII, and
    // this runs for every row of every frame, so skip grapheme splitting.
    if s.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return s.len();
    }
    s.graphemes(true).map(grapheme_width).sum()
}

/// Truncate to at most `max` cells, adding `…` when anything was cut.
pub fn truncate(s: &str, max: usize) -> String {
    if str_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for g in s.graphemes(true) {
        let gw = grapheme_width(g);
        if w + gw + 1 > max {
            break;
        }
        out.push_str(g);
        w += gw;
    }
    out.push('…');
    out
}

/// Shorten to at most `max` cells by cutting the middle, so both the start
/// and the end (often a file name) stay readable: `npx tsx --te…store.ts`.
pub fn ellipsize_middle(s: &str, max: usize) -> String {
    if str_width(s) <= max {
        return s.to_string();
    }
    if max < 8 {
        return truncate(s, max);
    }
    let graphemes: Vec<&str> = s.graphemes(true).collect();
    let keep = max - 1;
    let tail_w = keep * 2 / 5;
    let head_w = keep - tail_w;
    let mut head = String::new();
    let mut w = 0;
    for g in &graphemes {
        let gw = grapheme_width(g);
        if w + gw > head_w {
            break;
        }
        head.push_str(g);
        w += gw;
    }
    let mut tail: Vec<&str> = Vec::new();
    let mut w = 0;
    for g in graphemes.iter().rev() {
        let gw = grapheme_width(g);
        if w + gw > tail_w {
            break;
        }
        tail.push(g);
        w += gw;
    }
    tail.reverse();
    format!("{head}…{}", tail.concat())
}

/// Word-wrap `line` to `width` cells. Continuation rows start with `indent`
/// (for hanging indents under a glyph column). Words longer than a row are
/// split by grapheme. Spaces at a wrap point are dropped.
pub fn wrap(line: &Line, width: usize, indent: &Line) -> Vec<Line> {
    let width = width.max(1);
    let indent_w = indent.width().min(width.saturating_sub(1));

    // Flatten into cells, each tagged with its style.
    let mut cells: Vec<(&str, Style, usize)> = Vec::new();
    for span in &line.spans {
        for g in span.text.graphemes(true) {
            if g == "\n" || g == "\r\n" {
                continue;
            }
            cells.push((g, span.style, grapheme_width(g)));
        }
    }

    let mut rows: Vec<Line> = Vec::new();
    let mut row = Line { spans: Vec::new(), fill: line.fill };
    let mut row_w = 0usize;
    let mut first = true;

    let push_cell = |row: &mut Line, g: &str, style: Style| {
        match row.spans.last_mut() {
            Some(last) if last.style == style => last.text.push_str(g),
            _ => row.spans.push(Span::styled(g, style)),
        }
    };

    let mut i = 0;
    while i < cells.len() {
        let limit = if first { width } else { width - indent_w };
        // Measure the next word (a run of non-space cells) or a space run.
        let is_space = cells[i].0.trim().is_empty();
        let mut j = i;
        let mut word_w = 0;
        while j < cells.len() && (cells[j].0.trim().is_empty()) == is_space {
            word_w += cells[j].2;
            j += 1;
        }

        if is_space {
            if row_w + word_w <= limit {
                for c in &cells[i..j] {
                    push_cell(&mut row, c.0, c.1);
                }
                row_w += word_w;
            } else {
                // Break here and swallow the spaces.
                rows.push(std::mem::replace(&mut row, Line { spans: Vec::new(), fill: line.fill }));
                row_w = 0;
                first = false;
            }
            i = j;
            continue;
        }

        if row_w + word_w <= limit {
            for c in &cells[i..j] {
                push_cell(&mut row, c.0, c.1);
            }
            row_w += word_w;
            i = j;
            continue;
        }

        if row_w > 0 && word_w <= width - indent_w {
            // The word fits on a fresh row.
            rows.push(std::mem::replace(&mut row, Line { spans: Vec::new(), fill: line.fill }));
            row_w = 0;
            first = false;
            continue;
        }

        // Split an overlong word across rows.
        for c in &cells[i..j] {
            let limit = if first { width } else { width - indent_w };
            if row_w + c.2 > limit && row_w > 0 {
                rows.push(std::mem::replace(&mut row, Line { spans: Vec::new(), fill: line.fill }));
                row_w = 0;
                first = false;
            }
            push_cell(&mut row, c.0, c.1);
            row_w += c.2;
        }
        i = j;
    }
    rows.push(row);

    // Trim trailing spaces and add the hanging indent to continuation rows.
    for (k, r) in rows.iter_mut().enumerate() {
        while let Some(last) = r.spans.last_mut() {
            let trimmed = last.text.trim_end().len();
            if trimmed == last.text.len() {
                break;
            }
            last.text.truncate(trimmed);
            if last.text.is_empty() {
                r.spans.pop();
            } else {
                break;
            }
        }
        if k > 0 && indent_w > 0 {
            let mut spans = indent.spans.clone();
            spans.append(&mut r.spans);
            r.spans = spans;
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.plain_text()).collect()
    }

    #[test]
    fn widths_count_cells() {
        assert_eq!(str_width("abc"), 3);
        assert_eq!(str_width("日本"), 4);
        assert_eq!(str_width("é"), 1);
        assert_eq!(str_width("e\u{301}"), 1);
    }

    #[test]
    fn wraps_on_word_boundaries_with_hanging_indent() {
        let l = Line::raw("the quick brown fox jumps over the lazy dog");
        let out = wrap(&l, 16, &Line::raw("  "));
        assert_eq!(texts(&out), vec!["the quick brown", "  fox jumps over", "  the lazy dog"]);
        assert!(out.iter().all(|r| r.width() <= 16));
    }

    #[test]
    fn splits_overlong_words() {
        let l = Line::raw("aaaaaaaaaaaa bb");
        let out = wrap(&l, 5, &Line::new());
        assert_eq!(texts(&out), vec!["aaaaa", "aaaaa", "aa bb"]);
    }

    #[test]
    fn keeps_wide_characters_whole() {
        let l = Line::raw("日本語のテキスト");
        let out = wrap(&l, 5, &Line::new());
        assert!(out.iter().all(|r| r.width() <= 5));
        assert_eq!(texts(&out).concat(), "日本語のテキスト");
    }

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("hi", 6), "hi");
    }

    #[test]
    fn ascii_fast_path_matches_grapheme_widths() {
        for s in ["", "plain text", "a\tb", "esc\x1b[0m", "café", "日本", "👍🏽", "~ !\x7f"] {
            let slow: usize = s.graphemes(true).map(grapheme_width).sum();
            assert_eq!(str_width(s), slow, "{s:?}");
        }
    }
}
