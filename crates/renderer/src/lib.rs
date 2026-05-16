//! Backend-agnostic viewport state and render model.
//!
//! Pure logic — no terminal I/O. Owns bounded memory only:
//! a ring buffer of recent rendered rows, one active stream buffer,
//! and a small map of active tool states.

pub mod viewport;
pub mod model;

pub use model::{RenderModel, Row, RowKind, ToolState};
pub use viewport::ViewportState;
