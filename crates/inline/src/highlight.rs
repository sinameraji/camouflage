//! Syntax highlighting mapped onto the terminal's 16-color palette.
//!
//! syntect parses the code; instead of using its RGB themes we map scope names
//! to ANSI colors, so code blocks follow the user's terminal theme like the
//! rest of the output. Loading the syntax set costs a few milliseconds, so it
//! happens on the first code block, not at startup.

use crate::style::{Color, Span, Style};
use std::sync::OnceLock;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn find_syntax(lang: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let lang = lang.trim().to_ascii_lowercase();
    // The default grammars have no TypeScript; JavaScript's is close enough.
    let lang = match lang.as_str() {
        "ts" | "tsx" | "typescript" | "mts" | "cts" | "jsx" => "js",
        "sh" | "shell" | "zsh" | "console" => "bash",
        "yml" => "yaml",
        "rs" => "rust",
        "py" => "python",
        other => other,
    };
    if lang.is_empty() {
        return None;
    }
    set.find_syntax_by_token(lang)
}

/// Pick a color for the innermost scope we recognize.
fn style_for(stack: &ScopeStack) -> Style {
    for scope in stack.as_slice().iter().rev() {
        if let Some(style) = style_for_scope(*scope) {
            return style;
        }
    }
    Style::new()
}

fn style_for_scope(scope: Scope) -> Option<Style> {
    let name = scope.build_string();
    let starts = |p: &str| name.starts_with(p);
    Some(if starts("comment") {
        Style::new().dim().italic()
    } else if starts("string") {
        Style::new().fg(Color::Green)
    } else if starts("constant.numeric") || starts("constant.language") || starts("constant.character") {
        Style::new().fg(Color::Yellow)
    } else if starts("keyword") || starts("storage") {
        Style::new().fg(Color::Magenta)
    } else if starts("entity.name.function") || starts("support.function") || starts("meta.function-call") && name.contains("entity") {
        Style::new().fg(Color::Blue)
    } else if starts("entity.name.type") || starts("entity.name.class") || starts("support.type") || starts("support.class") || starts("entity.other.inherited-class") {
        Style::new().fg(Color::Cyan)
    } else if starts("entity.name.tag") {
        Style::new().fg(Color::Blue)
    } else if starts("entity.other.attribute-name") || starts("variable.parameter") {
        Style::new().fg(Color::Cyan)
    } else if starts("markup.inserted") {
        Style::new().fg(Color::Green)
    } else if starts("markup.deleted") {
        Style::new().fg(Color::Red)
    } else {
        return None;
    })
}

/// Highlight a block of code. Returns one span list per source line. Unknown
/// languages come back unstyled.
pub fn highlight(code: &str, lang: &str) -> Vec<Vec<Span>> {
    let Some(syntax) = find_syntax(lang) else {
        return code.lines().map(|l| vec![Span::raw(l)]).collect();
    };
    let set = syntaxes();
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out = Vec::new();
    for line in code.lines() {
        let with_nl = format!("{line}\n");
        let ops = match state.parse_line(&with_nl, set) {
            Ok(ops) => ops,
            Err(_) => {
                out.push(vec![Span::raw(line)]);
                continue;
            }
        };
        let mut spans: Vec<Span> = Vec::new();
        let mut pos = 0;
        let push = |spans: &mut Vec<Span>, text: &str, style: Style| {
            let text = text.trim_end_matches('\n');
            if text.is_empty() {
                return;
            }
            match spans.last_mut() {
                Some(last) if last.style == style => last.text.push_str(text),
                _ => spans.push(Span::styled(text, style)),
            }
        };
        for (idx, op) in ops {
            if idx > pos {
                push(&mut spans, &with_nl[pos..idx], style_for(&stack));
                pos = idx;
            }
            let _ = stack.apply(&op);
        }
        if pos < with_nl.len() {
            push(&mut spans, &with_nl[pos..], style_for(&stack));
        }
        out.push(spans);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_keywords_strings_and_comments() {
        let lines = highlight("const x = \"hi\" // note", "ts");
        let spans = &lines[0];
        let find = |t: &str| spans.iter().find(|s| s.text.contains(t)).map(|s| s.style);
        assert_eq!(find("const").unwrap().fg, Color::Magenta);
        assert_eq!(find("hi").unwrap().fg, Color::Green);
        assert!(find("note").unwrap().dim);
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "const x = \"hi\" // note");
    }

    #[test]
    fn unknown_languages_are_plain() {
        let lines = highlight("just text", "nope");
        assert_eq!(lines, vec![vec![Span::raw("just text")]]);
    }
}
