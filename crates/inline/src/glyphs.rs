//! Vetted one-cell glyphs used by the inline renderer.

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const ASCII_SPINNER: [&str; 10] = ["|", "/", "-", "\\", "|", "/", "-", "\\", "|", "/"];

/// Fixed Unicode symbols used by inline live rows and activity chrome.
pub const ALLOWED: &[&str] = &[
    "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "━", "╌", "◆", "●", "·", "›", "■", "✗", "✓",
    "▍", "…", "○", "⊘", "█", "░",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyphs {
    pub ascii: bool,
    pub spinner: [&'static str; 10],
    pub plan: &'static str,
    pub progress_done: &'static str,
    pub progress_active: &'static str,
    pub progress_pending: &'static str,
    pub agent: &'static str,
    pub read: &'static str,
    pub edit: &'static str,
    pub shell: &'static str,
    pub code: &'static str,
    pub error: &'static str,
    pub success: &'static str,
    pub declined: &'static str,
    pub radio_on: &'static str,
    pub radio_off: &'static str,
    pub bar_full: &'static str,
    pub bar_empty: &'static str,
    pub caret: &'static str,
    pub ellipsis: &'static str,
}

impl Glyphs {
    pub const fn unicode() -> Self {
        Self {
            ascii: false,
            spinner: SPINNER,
            plan: "◆",
            progress_done: "━",
            progress_active: "━",
            progress_pending: "╌",
            agent: "●",
            read: "·",
            edit: "◆",
            shell: "›",
            code: "■",
            error: "✗",
            success: "✓",
            declined: "⊘",
            radio_on: "●",
            radio_off: "○",
            bar_full: "█",
            bar_empty: "░",
            caret: "▍",
            ellipsis: "…",
        }
    }

    pub const fn ascii() -> Self {
        Self {
            ascii: true,
            spinner: ASCII_SPINNER,
            plan: "#",
            progress_done: "#",
            progress_active: "#",
            progress_pending: "-",
            agent: "@",
            read: ".",
            edit: "*",
            shell: ">",
            code: "#",
            error: "x",
            success: "v",
            declined: "x",
            radio_on: "*",
            radio_off: "o",
            bar_full: "#",
            bar_empty: "-",
            caret: ">",
            ellipsis: ".",
        }
    }

    pub fn detect() -> Self {
        let force = std::env::var("CAMOUFLAGE_ASCII").as_deref() == Ok("1");
        let term = std::env::var("TERM").unwrap_or_default();
        let locale = std::env::var("LC_ALL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("LANG").ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if force
            || term == "linux"
            || (!locale.is_empty() && !locale.contains("utf-8") && !locale.contains("utf8"))
        {
            Self::ascii()
        } else {
            Self::unicode()
        }
    }

    pub fn fixed_glyphs(&self) -> Vec<&'static str> {
        let mut v = self.spinner.to_vec();
        v.extend([
            self.plan,
            self.progress_done,
            self.progress_active,
            self.progress_pending,
            self.agent,
            self.read,
            self.edit,
            self.shell,
            self.code,
            self.error,
            self.success,
            self.declined,
            self.radio_on,
            self.radio_off,
            self.bar_full,
            self.bar_empty,
            self.caret,
            self.ellipsis,
        ]);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn unicode_glyphs_are_one_cell_and_allowlisted() {
        for g in Glyphs::unicode().fixed_glyphs() {
            assert_eq!(UnicodeWidthStr::width(g), 1, "{g:?}");
            assert!(g.is_ascii() || ALLOWED.contains(&g), "{g:?}");
        }
    }

    #[test]
    fn ascii_table_is_cell_parity_and_ascii() {
        let unicode = Glyphs::unicode().fixed_glyphs();
        let ascii = Glyphs::ascii().fixed_glyphs();
        assert_eq!(unicode.len(), ascii.len());
        for (u, a) in unicode.into_iter().zip(ascii) {
            assert_eq!(
                UnicodeWidthStr::width(u),
                UnicodeWidthStr::width(a),
                "{u:?} => {a:?}"
            );
            assert!(a.is_ascii());
        }
    }
}
