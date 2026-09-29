//! Inline terminal rendering for Camouflage.
//!
//! Finished output is printed once into the terminal's normal scrollback;
//! only a small live region at the bottom is ever redrawn. See
//! `docs/inline-redesign.md` §3 for the design and its rules.

pub mod style;
pub mod term;
pub mod width;

pub use style::{Color, Line, Span, Style};
pub use term::{InlineTerminal, LiveFrame};
pub use width::{str_width, truncate, wrap};
