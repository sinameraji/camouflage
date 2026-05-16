//! Minimal inline-markdown tokenizer for TUI rendering.
//!
//! Handles the inline markup that assistant text actually contains in real
//! coding-agent transcripts:
//!
//! - `**bold**`
//! - `*italic*` and `_italic_`
//! - `` `code` ``
//! - backslash escapes (`\*`, `\` `, `\\`)
//!
//! Block-level markdown (headings, lists, fenced code) is *not* parsed here
//! — those are stylable per-line by the draw layer using leading-character
//! checks. The parser intentionally stays single-pass and allocation-light
//! so the TUI can re-parse visible rows every frame without a measurable
//! hit. CommonMark conformance is explicitly a non-goal.
//!
//! Browser/SDK consumers should ignore this module entirely and parse
//! markdown in their own language with the lib of their choice — the
//! `Snapshot` projection deliberately passes plain text through.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineStyle {
    Plain,
    Bold,
    Italic,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSpan {
    pub text: String,
    pub style: InlineStyle,
}

impl InlineSpan {
    pub fn plain(s: impl Into<String>) -> Self {
        Self { text: s.into(), style: InlineStyle::Plain }
    }
}

/// Tokenize `text` into styled spans. Unmatched / malformed markers degrade
/// to plain text so the user always sees their content (worst case: a `**`
/// shows literally instead of forming a bold run).
pub fn parse_inline(text: &str) -> Vec<InlineSpan> {
    let mut out: Vec<InlineSpan> = Vec::new();
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut plain_start = 0;

    let flush_plain =
        |out: &mut Vec<InlineSpan>, src: &str, from: usize, to: usize| {
            if to > from {
                out.push(InlineSpan::plain(&src[from..to]));
            }
        };

    while i < n {
        let c = bytes[i];

        // Backslash escape: emit the next character verbatim, swallow the backslash.
        if c == b'\\' && i + 1 < n {
            flush_plain(&mut out, text, plain_start, i);
            // emit the escaped char as plain
            let ch = bytes[i + 1];
            // Conservative: only escape markdown-significant chars; otherwise
            // keep the backslash.
            if matches!(ch, b'*' | b'_' | b'`' | b'\\') {
                out.push(InlineSpan::plain(
                    std::str::from_utf8(&bytes[i + 1..i + 2]).unwrap_or("?"),
                ));
                i += 2;
            } else {
                // Keep backslash + char as plain.
                out.push(InlineSpan::plain(
                    std::str::from_utf8(&bytes[i..i + 2]).unwrap_or("?"),
                ));
                i += 2;
            }
            plain_start = i;
            continue;
        }

        // Code span (highest precedence — its contents are literal).
        if c == b'`' {
            if let Some(end) = find_unescaped(bytes, i + 1, b'`') {
                flush_plain(&mut out, text, plain_start, i);
                let inner = &text[i + 1..end];
                out.push(InlineSpan { text: inner.to_string(), style: InlineStyle::Code });
                i = end + 1;
                plain_start = i;
                continue;
            }
        }

        // Bold: **...**
        if c == b'*' && i + 1 < n && bytes[i + 1] == b'*' {
            if let Some(end) = find_marker(bytes, i + 2, b"**") {
                flush_plain(&mut out, text, plain_start, i);
                let inner = &text[i + 2..end];
                // Recurse so nested italic/code inside bold still works.
                for mut sp in parse_inline(inner) {
                    if sp.style == InlineStyle::Plain {
                        sp.style = InlineStyle::Bold;
                    }
                    out.push(sp);
                }
                i = end + 2;
                plain_start = i;
                continue;
            }
        }

        // Italic: *...*  or  _..._
        if (c == b'*' || c == b'_') && i + 1 < n && bytes[i + 1] != c {
            let prev_is_word = i > 0 && is_word(bytes[i - 1]);
            // For underscore: the char *before* the opening `_` must be
            // non-word (else we're in the middle of an identifier like
            // snake_case). The char before content doesn't matter; what
            // matters is the *outer* boundary.
            let opening_ok = if c == b'_' { !prev_is_word } else { true };
            if opening_ok {
                if let Some(end) = find_byte(bytes, i + 1, c) {
                    // Bound: don't run away past a newline.
                    let line_break = bytes[i + 1..end].iter().position(|&b| b == b'\n');
                    let after = bytes.get(end + 1).copied();
                    // Closing `_`: the char *after* must also be non-word
                    // (or end-of-string). Asterisk is unconstrained on this
                    // side since `*` is rare inside identifiers.
                    let closing_ok = if c == b'_' {
                        after.map(|b| !is_word(b)).unwrap_or(true)
                    } else {
                        true
                    };
                    // Italic content must be non-empty and not start with a
                    // space (Markdown convention).
                    let content_ok = end > i + 1 && bytes[i + 1] != b' ';
                    if line_break.is_none() && closing_ok && content_ok {
                        flush_plain(&mut out, text, plain_start, i);
                        let inner = &text[i + 1..end];
                        for mut sp in parse_inline(inner) {
                            if sp.style == InlineStyle::Plain {
                                sp.style = InlineStyle::Italic;
                            }
                            out.push(sp);
                        }
                        i = end + 1;
                        plain_start = i;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }
    flush_plain(&mut out, text, plain_start, n);
    out
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn find_byte(bytes: &[u8], from: usize, target: u8) -> Option<usize> {
    bytes[from..].iter().position(|&b| b == target).map(|p| from + p)
}

fn find_unescaped(bytes: &[u8], from: usize, target: u8) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == target {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_marker(bytes: &[u8], from: usize, marker: &[u8]) -> Option<usize> {
    let m = marker.len();
    if from + m > bytes.len() {
        return None;
    }
    let mut i = from;
    while i + m <= bytes.len() {
        if &bytes[i..i + m] == marker {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(s: &str) -> Vec<(&str, InlineStyle)> {
        Box::leak(Box::new(parse_inline(s)))
            .iter()
            .map(|sp| (sp.text.as_str(), sp.style))
            .collect()
    }

    #[test]
    fn plain_text_round_trips() {
        assert_eq!(spans("hello world"), vec![("hello world", InlineStyle::Plain)]);
    }

    #[test]
    fn bold_basic() {
        assert_eq!(
            spans("a **b** c"),
            vec![
                ("a ", InlineStyle::Plain),
                ("b", InlineStyle::Bold),
                (" c", InlineStyle::Plain),
            ]
        );
    }

    #[test]
    fn italic_with_asterisk() {
        assert_eq!(
            spans("a *b* c"),
            vec![
                ("a ", InlineStyle::Plain),
                ("b", InlineStyle::Italic),
                (" c", InlineStyle::Plain),
            ]
        );
    }

    #[test]
    fn italic_with_underscore_avoids_snake_case() {
        // foo_bar must NOT be parsed as foo<italic>bar</italic>
        assert_eq!(spans("foo_bar_baz"), vec![("foo_bar_baz", InlineStyle::Plain)]);
    }

    #[test]
    fn italic_with_underscore_works_with_spaces() {
        assert_eq!(
            spans("an _emphatic_ word"),
            vec![
                ("an ", InlineStyle::Plain),
                ("emphatic", InlineStyle::Italic),
                (" word", InlineStyle::Plain),
            ]
        );
    }

    #[test]
    fn code_span() {
        assert_eq!(
            spans("run `npm test` now"),
            vec![
                ("run ", InlineStyle::Plain),
                ("npm test", InlineStyle::Code),
                (" now", InlineStyle::Plain),
            ]
        );
    }

    #[test]
    fn code_contents_are_literal() {
        // Markers inside backticks must NOT be parsed as bold/italic.
        assert_eq!(
            spans("`**not bold**`"),
            vec![("**not bold**", InlineStyle::Code)]
        );
    }

    #[test]
    fn unmatched_marker_falls_back_to_plain() {
        assert_eq!(spans("a **b c"), vec![("a **b c", InlineStyle::Plain)]);
    }

    #[test]
    fn backslash_escapes_asterisk() {
        assert_eq!(
            spans(r"a \*b\* c"),
            vec![
                ("a ", InlineStyle::Plain),
                ("*", InlineStyle::Plain),
                ("b", InlineStyle::Plain),
                ("*", InlineStyle::Plain),
                (" c", InlineStyle::Plain),
            ]
        );
    }

    #[test]
    fn nested_code_inside_bold() {
        let v = parse_inline("**use `cargo` daily**");
        // Expect: bold "use ", code "cargo" (also bold? no — code wins), bold " daily"
        // Current behaviour: code span keeps Code style; surrounding text is Bold.
        let kinds: Vec<_> = v.iter().map(|s| (s.text.as_str(), s.style)).collect();
        assert_eq!(
            kinds,
            vec![
                ("use ", InlineStyle::Bold),
                ("cargo", InlineStyle::Code),
                (" daily", InlineStyle::Bold),
            ]
        );
    }

    #[test]
    fn assistant_text_from_adapter_test() {
        // The actual problematic line from the real Cloudflare turn.
        let line = "Here are the files in `src/`, grouped by directory: **Root level** - `index.tsx`";
        let v = parse_inline(line);
        // Smoke: code spans + bold are recognised, no remaining literal markers.
        let joined: String = v.iter().map(|s| s.text.clone()).collect();
        assert!(!joined.contains("**"), "literal ** still in output: {joined}");
        assert!(!joined.contains('`'), "literal backticks still in output: {joined}");
        assert!(v.iter().any(|s| s.style == InlineStyle::Code));
        assert!(v.iter().any(|s| s.style == InlineStyle::Bold));
    }
}
