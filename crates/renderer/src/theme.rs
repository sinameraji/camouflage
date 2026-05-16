//! Renderer themes.
//!
//! A `Theme` is a small bag of semantic colors that the TUI uses for row
//! kinds, status/accent surfaces, and overlay borders. Themes are wire-
//! independent (the JSON `Snapshot` deliberately ships plain text + plain
//! row kinds so browser/SDK consumers can apply their own theming).
//!
//! Three themes ship built-in: `default-dark`, `default-light`, and
//! `dracula`. Hosts and end users can add more by dropping JSON files
//! into a future `themes/` directory loaded at startup (deferred to a
//! follow-up — the structure here is JSON-compatible via serde so the
//! loader is a one-liner when needed).

use serde::{Deserialize, Serialize};

/// 24-bit RGB color. Renderers map this to whatever their backend wants
/// (ratatui::Color::Rgb for the TUI, hex string for browser, etc.).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    /// Header line accent (e.g. "Camouflage" title).
    pub accent: Rgb,
    /// User input prefix and submitted user rows.
    pub user: Rgb,
    /// Assistant text default colour (markdown spans inherit from this).
    pub assistant: Rgb,
    /// Tool execution rows.
    pub tool: Rgb,
    /// Error rows (RuntimeError, kind-specific blocks).
    pub error: Rgb,
    /// System / meta rows.
    pub system: Rgb,
    /// Bookmark / viewport markers.
    pub marker: Rgb,
    /// Diff line additions.
    pub diff_add: Rgb,
    /// Diff line removals.
    pub diff_remove: Rgb,
    /// Diff hunk header (`@@`).
    pub diff_hunk: Rgb,
    /// Inline-code span foreground.
    pub code_fg: Rgb,
    /// Inline-code span background.
    pub code_bg: Rgb,
    /// Spinner glyph while streaming / tool in flight.
    pub spinner: Rgb,
    /// Overlay border (help, metrics, permission).
    pub overlay_border: Rgb,
}

impl Theme {
    pub fn builtin(name: &str) -> Option<&'static Theme> {
        BUILTIN.iter().find(|t| t.name == name)
    }

    /// Returns the next theme name in cycle order. Used by the TUI's `T`
    /// keybind.
    pub fn next_after(name: &str) -> &'static str {
        let i = BUILTIN.iter().position(|t| t.name == name).unwrap_or(0);
        let next = (i + 1) % BUILTIN.len();
        &BUILTIN[next].name
    }

    pub fn builtin_names() -> Vec<&'static str> {
        BUILTIN.iter().map(|t| t.name.as_str()).collect()
    }
}

// Note: lazy_static would be cleaner but we don't want the dep. The themes
// are constructed at first access; the cost is one Vec allocation per call
// for builtin_names() and a linear scan in builtin() — both rare paths.
fn theme(name: &str, builder: impl FnOnce() -> Theme) -> Theme {
    let mut t = builder();
    t.name = name.to_string();
    t
}

static BUILTIN: std::sync::LazyLock<Vec<Theme>> = std::sync::LazyLock::new(|| {
    vec![
        theme("default-dark", || Theme {
            name: String::new(),
            accent:         Rgb(0x58, 0xa6, 0xff),
            user:           Rgb(0xd2, 0xa8, 0xff),
            assistant:      Rgb(0xc9, 0xd1, 0xd9),
            tool:           Rgb(0x79, 0xc0, 0xff),
            error:          Rgb(0xff, 0x7b, 0x72),
            system:         Rgb(0x76, 0x83, 0x90),
            marker:         Rgb(0x69, 0x39, 0xff),
            diff_add:       Rgb(0x56, 0xd3, 0x64),
            diff_remove:    Rgb(0xff, 0x7b, 0x72),
            diff_hunk:      Rgb(0x79, 0xc0, 0xff),
            code_fg:        Rgb(0x79, 0xc0, 0xff),
            code_bg:        Rgb(0x1c, 0x21, 0x28),
            spinner:        Rgb(0xf0, 0x88, 0x3e),
            overlay_border: Rgb(0xf0, 0x88, 0x3e),
        }),
        theme("default-light", || Theme {
            name: String::new(),
            accent:         Rgb(0x09, 0x69, 0xda),
            user:           Rgb(0x82, 0x50, 0xdf),
            assistant:      Rgb(0x1f, 0x23, 0x28),
            tool:           Rgb(0x05, 0x50, 0xae),
            error:          Rgb(0xcf, 0x22, 0x2e),
            system:         Rgb(0x59, 0x63, 0x6e),
            marker:         Rgb(0x82, 0x50, 0xdf),
            diff_add:       Rgb(0x1a, 0x7f, 0x37),
            diff_remove:    Rgb(0xcf, 0x22, 0x2e),
            diff_hunk:      Rgb(0x05, 0x50, 0xae),
            code_fg:        Rgb(0x05, 0x50, 0xae),
            code_bg:        Rgb(0xf6, 0xf8, 0xfa),
            spinner:        Rgb(0x9a, 0x67, 0x00),
            overlay_border: Rgb(0x9a, 0x67, 0x00),
        }),
        theme("dracula", || Theme {
            name: String::new(),
            accent:         Rgb(0xbd, 0x93, 0xf9),
            user:           Rgb(0xff, 0x79, 0xc6),
            assistant:      Rgb(0xf8, 0xf8, 0xf2),
            tool:           Rgb(0x8b, 0xe9, 0xfd),
            error:          Rgb(0xff, 0x55, 0x55),
            system:         Rgb(0x62, 0x72, 0xa4),
            marker:         Rgb(0xbd, 0x93, 0xf9),
            diff_add:       Rgb(0x50, 0xfa, 0x7b),
            diff_remove:    Rgb(0xff, 0x55, 0x55),
            diff_hunk:      Rgb(0x8b, 0xe9, 0xfd),
            code_fg:        Rgb(0xf1, 0xfa, 0x8c),
            code_bg:        Rgb(0x28, 0x2a, 0x36),
            spinner:        Rgb(0xff, 0xb8, 0x6c),
            overlay_border: Rgb(0xbd, 0x93, 0xf9),
        }),
    ]
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_lookup() {
        assert!(Theme::builtin("default-dark").is_some());
        assert!(Theme::builtin("dracula").is_some());
        assert!(Theme::builtin("nope").is_none());
    }

    #[test]
    fn cycle_wraps() {
        let names = Theme::builtin_names();
        let mut cur = names[0];
        for _ in 0..names.len() {
            cur = Theme::next_after(cur);
        }
        assert_eq!(cur, names[0], "cycle should wrap to the start after N steps");
    }

    #[test]
    fn theme_is_serde_compatible() {
        let t = Theme::builtin("default-dark").unwrap();
        let s = serde_json::to_string(t).unwrap();
        let back: Theme = serde_json::from_str(&s).unwrap();
        assert_eq!(back.name, "default-dark");
    }
}
