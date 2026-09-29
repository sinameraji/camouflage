//! Parse text containing ANSI SGR escapes (colored tool output, host splash
//! banners) into styled lines. Other escape sequences are dropped, so host
//! text can never move the cursor and break the live region.

use crate::style::{Color, Line, Span, Style};

pub fn parse_ansi(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut line = Line::new();
    let mut style = Style::new();
    let mut buf = String::new();
    let mut chars = text.chars().peekable();

    let flush = |line: &mut Line, buf: &mut String, style: Style| {
        if !buf.is_empty() {
            line.spans.push(Span::styled(std::mem::take(buf), style));
        }
    };

    while let Some(c) = chars.next() {
        match c {
            '\x1b' => {
                if chars.peek() == Some(&'[') {
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
                    if fin == Some('m') {
                        flush(&mut line, &mut buf, style);
                        style = apply_sgr(style, &params);
                    }
                } else if chars.peek() == Some(&']') {
                    // OSC (e.g. hyperlinks): skip to BEL or ST.
                    chars.next();
                    while let Some(p) = chars.next() {
                        if p == '\x07' {
                            break;
                        }
                        if p == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
            '\n' => {
                flush(&mut line, &mut buf, style);
                lines.push(std::mem::take(&mut line));
            }
            '\r' => {}
            '\t' => buf.push_str("    "),
            c if c.is_control() => {}
            c => buf.push(c),
        }
    }
    flush(&mut line, &mut buf, style);
    if !line.spans.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// Strip all escapes, keeping plain text.
pub fn strip_ansi(text: &str) -> String {
    parse_ansi(text).iter().map(|l| l.plain_text()).collect::<Vec<_>>().join("\n")
}

fn basic(n: u16) -> Color {
    match n {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ => Color::White,
    }
}

fn bright(n: u16) -> Color {
    match n {
        0 => Color::BrightBlack,
        1 => Color::BrightRed,
        2 => Color::BrightGreen,
        3 => Color::BrightYellow,
        4 => Color::BrightBlue,
        5 => Color::BrightMagenta,
        6 => Color::BrightCyan,
        _ => Color::BrightWhite,
    }
}

fn apply_sgr(mut s: Style, params: &str) -> Style {
    let nums: Vec<u16> = if params.is_empty() {
        vec![0]
    } else {
        params.split([';', ':']).map(|p| p.parse().unwrap_or(0)).collect()
    };
    let mut i = 0;
    while i < nums.len() {
        match nums[i] {
            0 => s = Style::new(),
            1 => s.bold = true,
            2 => s.dim = true,
            3 => s.italic = true,
            4 => s.underline = true,
            7 => s.reverse = true,
            9 => s.strike = true,
            22 => {
                s.bold = false;
                s.dim = false;
            }
            23 => s.italic = false,
            24 => s.underline = false,
            27 => s.reverse = false,
            29 => s.strike = false,
            n @ 30..=37 => s.fg = basic(n - 30),
            39 => s.fg = Color::Default,
            n @ 40..=47 => s.bg = basic(n - 40),
            49 => s.bg = Color::Default,
            n @ 90..=97 => s.fg = bright(n - 90),
            n @ 100..=107 => s.bg = bright(n - 100),
            n @ (38 | 48) => {
                let color = match nums.get(i + 1) {
                    Some(5) => {
                        let c = Color::Indexed(*nums.get(i + 2).unwrap_or(&0) as u8);
                        i += 2;
                        c
                    }
                    Some(2) => {
                        let g = |k: usize| *nums.get(i + k).unwrap_or(&0) as u8;
                        let c = Color::Rgb(g(2), g(3), g(4));
                        i += 4;
                        c
                    }
                    _ => Color::Default,
                };
                if n == 38 {
                    s.fg = color;
                } else {
                    s.bg = color;
                }
            }
            _ => {}
        }
        i += 1;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colors_and_resets() {
        let lines = parse_ansi("\x1b[32m✔\x1b[0m passed\n\x1b[1;31mfail\x1b[m");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].style.fg, Color::Green);
        assert_eq!(lines[0].spans[1].text, " passed");
        assert!(lines[0].spans[1].style.is_plain());
        assert!(lines[1].spans[0].style.bold);
        assert_eq!(lines[1].spans[0].style.fg, Color::Red);
    }

    #[test]
    fn parses_truecolor_and_256() {
        let l = &parse_ansi("\x1b[38;2;255;128;0mx\x1b[48;5;236my")[0];
        assert_eq!(l.spans[0].style.fg, Color::Rgb(255, 128, 0));
        assert_eq!(l.spans[1].style.bg, Color::Indexed(236));
    }

    #[test]
    fn drops_cursor_movement_and_osc() {
        let text = "a\x1b[2Kb\x1b[1Ac\x1b]8;;https://x\x07link\x1b]8;;\x07";
        assert_eq!(strip_ansi(text), "abclink");
    }
}
