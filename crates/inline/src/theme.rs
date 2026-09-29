//! Semantic styles. Hosts pick one accent; everything else comes from the
//! terminal's own palette, so the output matches the user's theme.

use crate::style::{Color, Style};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    /// The host's brand color: spinner, selection, links in prompts.
    pub accent: Color,
    /// Background of the user-prompt band. `None` draws no band.
    pub user_band: Option<Color>,
    /// Background tints for added and removed diff lines.
    pub diff_add_bg: Option<Color>,
    pub diff_del_bg: Option<Color>,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::dark()
    }
}

impl Theme {
    /// Defaults for dark terminals. 256-color grays are subtle on nearly every
    /// dark palette; hosts can override.
    pub fn dark() -> Self {
        Theme {
            accent: Color::Yellow,
            user_band: Some(Color::Indexed(236)),
            diff_add_bg: Some(Color::Indexed(22)),
            diff_del_bg: Some(Color::Indexed(52)),
        }
    }

    pub fn light() -> Self {
        Theme {
            accent: Color::Blue,
            user_band: Some(Color::Indexed(254)),
            diff_add_bg: Some(Color::Indexed(194)),
            diff_del_bg: Some(Color::Indexed(224)),
        }
    }

    pub fn with_accent(mut self, accent: Color) -> Self {
        self.accent = accent;
        self
    }

    pub fn text(&self) -> Style { Style::new() }
    pub fn dim(&self) -> Style { Style::new().dim() }
    pub fn accent(&self) -> Style { Style::new().fg(self.accent) }
    pub fn strong(&self) -> Style { Style::new().bold() }
    pub fn ok(&self) -> Style { Style::new().fg(Color::Green) }
    pub fn err(&self) -> Style { Style::new().fg(Color::Red) }
    pub fn warn(&self) -> Style { Style::new().fg(Color::Yellow) }
    pub fn inline_code(&self) -> Style { Style::new().fg(Color::Blue) }
}
