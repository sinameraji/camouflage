//! Renderer-agnostic view state.
//!
//! A `Snapshot` is a small, serde-derived projection of `RenderModel` that any
//! non-TUI renderer (browser viewer, SDK consumer, future native shell) can
//! consume without re-implementing the protocol semantics in its own language.
//! The TUI does NOT use this — it reads `RenderModel` fields directly for
//! per-frame performance — but everything else should subscribe to a stream
//! of snapshots over the wire instead of re-deriving them from raw events.
//!
//! The shape is intentionally narrow: only what a UI needs to draw. Per-cell
//! rendering decisions (color, wrap, spinner glyph) belong in the renderer
//! itself, not in the snapshot.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotRowKind {
    System,
    User,
    Assistant,
    Tool,
    Error,
    Marker,
    Diff,
    Control,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotRow {
    pub seq: i64,
    pub kind: SnapshotRowKind,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotTaskState {
    Running,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotTask {
    pub task_id: String,
    pub label: String,
    pub state: SnapshotTaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotTodoStatus {
    Pending,
    InProgress,
    Completed,
}

/// One item in the agent's todo/plan checklist (see `TodoListUpdate`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotTodo {
    pub id: String,
    pub title: String,
    pub status: SnapshotTodoStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_delta: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotPermission {
    pub request_id: String,
    pub tool: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// CC-1 — projection of an active SelectList for non-Rust frontends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotSelectListOption {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotSelectList {
    pub id: String,
    pub prompt: String,
    pub options: Vec<SnapshotSelectListOption>,
    pub selected: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub filter: String,
    pub allow_filter: bool,
    pub allow_cancel: bool,
}

/// CC-2 — projection of an active Confirm modal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotConfirm {
    pub id: String,
    pub prompt: String,
    pub yes_label: String,
    pub no_label: String,
    pub selected_yes: bool,
    pub allow_cancel: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotTableAlign {
    Left,
    Right,
    Center,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotTableColumn {
    pub name: String,
    pub label: String,
    pub align: SnapshotTableAlign,
}

/// CC-6 — projection of an active Table modal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotTable {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub columns: Vec<SnapshotTableColumn>,
    /// Pre-stringified cells (rows × columns).
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotKeyValueItem {
    pub label: String,
    pub value: String,
}

/// CC-7 — projection of an active KeyValueView modal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotKeyValueView {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub items: Vec<SnapshotKeyValueItem>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotFormFieldKind {
    Text,
    Password,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotFormField {
    pub name: String,
    pub label: String,
    pub kind: SnapshotFormFieldKind,
    pub value: String,
    pub required: bool,
}

/// CC-5 — projection of an active Form modal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotForm {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub fields: Vec<SnapshotFormField>,
    pub focused: usize,
    pub allow_cancel: bool,
}

/// A full UI snapshot. Renderers should treat the document as
/// last-write-wins: every field reflects current state, not deltas.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub total_rows: i64,
    pub rows: Vec<SnapshotRow>,
    /// Status-bar segments in their preferred draw order.
    pub status: BTreeMap<String, String>,
    pub tasks: Vec<SnapshotTask>,
    /// The agent's todo/plan checklist. Empty when none is active.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<SnapshotTodo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_permission: Option<SnapshotPermission>,
    /// CC-1 — present when a SelectList modal is currently active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_list: Option<SnapshotSelectList>,
    /// CC-2 — present when a Confirm modal is currently active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<SnapshotConfirm>,
    /// CC-6 — present when a Table modal is currently active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<SnapshotTable>,
    /// CC-7 — present when a KeyValueView modal is currently active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_value_view: Option<SnapshotKeyValueView>,
    /// CC-5 — present when a Form modal is currently active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<SnapshotForm>,
}
