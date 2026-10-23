//! Styled text: colors, attributes, spans and lines, plus SGR encoding.
//!
//! Colors default to the terminal's own 16-color palette so output matches
//! the user's theme. Truecolor is available for hosts that insist.

use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Color {
    /// The terminal's default foreground or background.
    #[default]
    Default,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    fn write_sgr(self, out: &mut String, background: bool) {
        let base = if background { 40 } else { 30 };
        let bright = if background { 100 } else { 90 };
        let code = |n: u8| n as u32;
        match self {
            Color::Default => {
                let _ = write!(out, ";{}", if background { 49 } else { 39 });
            }
            Color::Black => { let _ = write!(out, ";{}", base); }
            Color::Red => { let _ = write!(out, ";{}", base + 1); }
            Color::Green => { let _ = write!(out, ";{}", base + 2); }
            Color::Yellow => { let _ = write!(out, ";{}", base + 3); }
            Color::Blue => { let _ = write!(out, ";{}", base + 4); }
            Color::Magenta => { let _ = write!(out, ";{}", base + 5); }
            Color::Cyan => { let _ = write!(out, ";{}", base + 6); }
            Color::White => { let _ = write!(out, ";{}", base + 7); }
            Color::BrightBlack => { let _ = write!(out, ";{}", bright); }
            Color::BrightRed => { let _ = write!(out, ";{}", bright + 1); }
            Color::BrightGreen => { let _ = write!(out, ";{}", bright + 2); }
            Color::BrightYellow => { let _ = write!(out, ";{}", bright + 3); }
            Color::BrightBlue => { let _ = write!(out, ";{}", bright + 4); }
            Color::BrightMagenta => { let _ = write!(out, ";{}", bright + 5); }
            Color::BrightCyan => { let _ = write!(out, ";{}", bright + 6); }
            Color::BrightWhite => { let _ = write!(out, ";{}", bright + 7); }
            Color::Indexed(n) => {
                let _ = write!(out, ";{};5;{}", if background { 48 } else { 38 }, code(n));
            }
            Color::Rgb(r, g, b) => {
                let _ = write!(out, ";{};2;{};{};{}", if background { 48 } else { 38 }, r, g, b);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub reverse: bool,
}

impl Style {
    pub const fn new() -> Self {
        Style {
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            strike: false,
            reverse: false,
        }
    }
    pub const fn fg(mut self, c: Color) -> Self { self.fg = c; self }
    pub const fn bg(mut self, c: Color) -> Self { self.bg = c; self }
    pub const fn bold(mut self) -> Self { self.bold = true; self }
    pub const fn dim(mut self) -> Self { self.dim = true; self }
    pub const fn italic(mut self) -> Self { self.italic = true; self }
    pub const fn underline(mut self) -> Self { self.underline = true; self }
    pub const fn strike(mut self) -> Self { self.strike = true; self }
    pub const fn reverse(mut self) -> Self { self.reverse = true; self }

    /// Layer `other` on top: its non-default colors and set attributes win.
    pub fn patch(self, other: Style) -> Style {
        Style {
            fg: if other.fg == Color::Default { self.fg } else { other.fg },
            bg: if other.bg == Color::Default { self.bg } else { other.bg },
            bold: self.bold || other.bold,
            dim: self.dim || other.dim,
            italic: self.italic || other.italic,
            underline: self.underline || other.underline,
            strike: self.strike || other.strike,
            reverse: self.reverse || other.reverse,
        }
    }

    pub fn is_plain(&self) -> bool {
        *self == Style::new()
    }

    /// Append the SGR sequence that switches to this style from a reset state.
    pub fn write_sgr(&self, out: &mut String) {
        out.push_str("\x1b[0");
        if self.bold { out.push_str(";1"); }
        if self.dim { out.push_str(";2"); }
        if self.italic { out.push_str(";3"); }
        if self.underline { out.push_str(";4"); }
        if self.reverse { out.push_str(";7"); }
        if self.strike { out.push_str(";9"); }
        if self.fg != Color::Default { self.fg.write_sgr(out, false); }
        if self.bg != Color::Default { self.bg.write_sgr(out, true); }
        out.push('m');
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

impl Span {
    pub fn raw(text: impl Into<String>) -> Self {
        Span { text: text.into(), style: Style::new() }
    }
    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        Span { text: text.into(), style }
    }
    pub fn width(&self) -> usize {
        crate::width::str_width(&self.text)
    }
}

/// One terminal row of styled text. `fill` paints the rest of the row (up to
/// the render width) with a background, for bands like user prompts and diffs.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Line {
    pub spans: Vec<Span>,
    pub fill: Option<Color>,
}

impl Line {
    pub fn new() -> Self {
        Line::default()
    }
    pub fn from_spans(spans: Vec<Span>) -> Self {
        Line { spans, fill: None }
    }
    pub fn raw(text: impl Into<String>) -> Self {
        Line::from_spans(vec![Span::raw(text)])
    }
    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        Line::from_spans(vec![Span::styled(text, style)])
    }
    pub fn push(&mut self, text: impl Into<String>, style: Style) -> &mut Self {
        self.spans.push(Span::styled(text, style));
        self
    }
    pub fn with(mut self, text: impl Into<String>, style: Style) -> Self {
        self.push(text, style);
        self
    }
    pub fn fill(mut self, bg: Color) -> Self {
        self.fill = Some(bg);
        self
    }
    pub fn width(&self) -> usize {
        self.spans.iter().map(Span::width).sum()
    }
    pub fn plain_text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    /// Encode as terminal bytes: styled spans, then the background fill up to
    /// `width`, always ending in a reset.
    pub fn encode(&self, width: usize, out: &mut String) {
        let mut current = Style::new();
        for span in &self.spans {
            let style = match self.fill {
                Some(bg) if span.style.bg == Color::Default => span.style.bg(bg),
                _ => span.style,
            };
            if style != current {
                style.write_sgr(out);
                current = style;
            }
            for ch in span.text.chars() {
                // Never emit raw control characters: they would move the
                // cursor and break line accounting. Bidi format controls
                // are dropped too, so untrusted text (e.g. a permission
                // prompt's action) can't be shown in reordered visual order.
                if ch == '\t' {
                    out.push_str("    ");
                } else if !ch.is_control() && !is_bidi_control(ch) {
                    out.push(ch);
                }
            }
        }
        if let Some(bg) = self.fill {
            let used = self.width();
            if used < width {
                let fill = Style::new().bg(bg);
                if fill != current {
                    fill.write_sgr(out);
                    current = fill;
                }
                for _ in used..width {
                    out.push(' ');
                }
            }
        }
        if !current.is_plain() {
            out.push_str("\x1b[0m");
        }
    }
}

/// Unicode bidirectional format characters (ALM, LRM, RLM, the embedding /
/// override controls and the isolates). A terminal that applies the bidi
/// algorithm would reorder the text around them, so text could display in
/// a different order than it is read or run.
pub fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(text: &str) -> String {
        let mut out = String::new();
        Line::raw(text).encode(0, &mut out);
        out
    }

    #[test]
    fn encode_drops_control_characters() {
        assert_eq!(encoded("a\x1b[2Jb\x07c\rd"), "a[2Jbcd");
        assert_eq!(encoded("a\tb"), "a    b");
    }

    #[test]
    fn encode_drops_bidi_format_controls() {
        let action =
            "run ls \u{202e}\u{2066}x; rm -rf ~ \u{2069}\u{2066}# list the files \u{2069}\u{202c}";
        assert_eq!(encoded(action), "run ls x; rm -rf ~ # list the files ");
        for ch in [
            '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
            '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            assert!(is_bidi_control(ch), "U+{:04X}", ch as u32);
            assert_eq!(encoded(&format!("a{ch}b")), "ab", "U+{:04X}", ch as u32);
        }
        // Other format characters and right-to-left text itself are kept.
        assert_eq!(
            encoded("\u{200D}\u{05D0}\u{0627}"),
            "\u{200D}\u{05D0}\u{0627}"
        );
    }
}
