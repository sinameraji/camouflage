use camouflage_protocol::{Event, EventType};
use std::collections::{HashMap, VecDeque};

/// Maximum number of rendered rows the model retains. Older rows are evicted;
/// they remain in the persistence layer and are paged back in on scroll.
pub const DEFAULT_ROW_CAP: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    System,
    User,
    Assistant,
    Tool,
    Error,
    Marker,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub seq: i64,
    pub kind: RowKind,
    pub text: String,
    /// For tool rows, the tool_id this row tracks.
    pub tool_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolState {
    pub tool_id: String,
    pub tool: String,
    pub command: String,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub finished: bool,
    pub exit_code: Option<i32>,
    pub row_index_hint: Option<usize>,
}

/// Bounded render model. Stores only enough state to draw the current viewport
/// plus a small scrollback cache. Never holds the full transcript.
pub struct RenderModel {
    rows: VecDeque<Row>,
    row_cap: usize,
    /// Total rows ever appended (rows.len() may be smaller after eviction).
    total_rows: i64,
    /// Active streaming assistant block, if any. Lives in `rows` at index `active_stream_row`.
    active_stream_row: Option<usize>,
    active_stream_id: Option<String>,
    /// Active tool states keyed by tool_id (collapsed by default).
    tools: HashMap<String, ToolState>,
    /// Dirty flag for damage tracking — TUI clears after draw.
    dirty: bool,
}

impl RenderModel {
    pub fn new() -> Self {
        Self::with_cap(DEFAULT_ROW_CAP)
    }

    pub fn with_cap(cap: usize) -> Self {
        Self {
            rows: VecDeque::with_capacity(cap),
            row_cap: cap.max(16),
            total_rows: 0,
            active_stream_row: None,
            active_stream_id: None,
            tools: HashMap::new(),
            dirty: true,
        }
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn rows(&self) -> &VecDeque<Row> {
        &self.rows
    }

    pub fn total_rows(&self) -> i64 {
        self.total_rows
    }

    pub fn tools(&self) -> &HashMap<String, ToolState> {
        &self.tools
    }

    fn push_row(&mut self, row: Row) -> usize {
        // Evict oldest before pushing if at cap. Adjust active_stream_row index.
        if self.rows.len() == self.row_cap {
            self.rows.pop_front();
            if let Some(idx) = self.active_stream_row.as_mut() {
                if *idx == 0 {
                    // Active stream was evicted — should not happen in practice
                    // because the active row is always near the tail.
                    self.active_stream_row = None;
                    self.active_stream_id = None;
                } else {
                    *idx -= 1;
                }
            }
        }
        self.rows.push_back(row);
        self.total_rows += 1;
        self.dirty = true;
        self.rows.len() - 1
    }

    /// Apply an event to the render model. Returns true if the model changed.
    pub fn apply(&mut self, ev: &Event) -> bool {
        match ev.event_type {
            EventType::SessionStarted => {
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: "session started".to_string(),
                    tool_id: None,
                });
            }
            EventType::SessionEnded => {
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: "session ended".to_string(),
                    tool_id: None,
                });
            }
            EventType::UserMessageCreated => {
                let text = ev
                    .payload
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::User,
                    text,
                    tool_id: None,
                });
            }
            EventType::AssistantStreamStarted => {
                let sid = ev
                    .payload
                    .get("stream_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let idx = self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::Assistant,
                    text: String::new(),
                    tool_id: None,
                });
                self.active_stream_row = Some(idx);
                self.active_stream_id = Some(sid);
            }
            EventType::AssistantTokenDelta => {
                let token = ev
                    .payload
                    .get("token")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if let Some(idx) = self.active_stream_row {
                    if let Some(row) = self.rows.get_mut(idx) {
                        row.text.push_str(token);
                        self.dirty = true;
                    }
                } else {
                    // Token without a preceding StreamStarted — open one implicitly.
                    let idx = self.push_row(Row {
                        seq: ev.seq,
                        kind: RowKind::Assistant,
                        text: token.to_string(),
                        tool_id: None,
                    });
                    self.active_stream_row = Some(idx);
                }
            }
            EventType::AssistantMessageCompleted => {
                self.active_stream_row = None;
                self.active_stream_id = None;
                self.dirty = true;
            }
            EventType::ToolExecutionStarted => {
                let tool_id = ev
                    .payload
                    .get("tool_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool = ev
                    .payload
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let command = ev
                    .payload
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let display = format!("▸ {} {}", tool, command);
                let idx = self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::Tool,
                    text: display,
                    tool_id: Some(tool_id.clone()),
                });
                self.tools.insert(
                    tool_id.clone(),
                    ToolState {
                        tool_id,
                        tool,
                        command,
                        stdout_bytes: 0,
                        stderr_bytes: 0,
                        finished: false,
                        exit_code: None,
                        row_index_hint: Some(idx),
                    },
                );
            }
            EventType::ToolExecutionStdout | EventType::ToolExecutionStderr => {
                // Collapsed by default — accumulate byte counts but don't
                // create per-chunk rows. Bytes flow through persistence intact.
                let tool_id = ev
                    .payload
                    .get("tool_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let chunk_len = ev
                    .payload
                    .get("chunk")
                    .and_then(|v| v.as_str())
                    .map(|s| s.len())
                    .unwrap_or(0);
                if let Some(state) = self.tools.get_mut(tool_id) {
                    if matches!(ev.event_type, EventType::ToolExecutionStdout) {
                        state.stdout_bytes += chunk_len;
                    } else {
                        state.stderr_bytes += chunk_len;
                    }
                    self.dirty = true;
                }
            }
            EventType::ToolExecutionFinished => {
                let tool_id = ev
                    .payload
                    .get("tool_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let exit = ev
                    .payload
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .map(|i| i as i32);
                if let Some(state) = self.tools.get_mut(&tool_id) {
                    state.finished = true;
                    state.exit_code = exit;
                    let summary = format!(
                        "✓ {} {} (exit={}, stdout={}B, stderr={}B)",
                        state.tool,
                        state.command,
                        exit.map(|i| i.to_string()).unwrap_or_else(|| "?".into()),
                        state.stdout_bytes,
                        state.stderr_bytes,
                    );
                    if let Some(idx) = state.row_index_hint {
                        // The hint may be stale after eviction; compare seq if reachable.
                        if let Some(row) = self.rows.get_mut(idx) {
                            if row.tool_id.as_deref() == Some(tool_id.as_str()) {
                                row.text = summary.clone();
                                self.dirty = true;
                                return true;
                            }
                        }
                    }
                    self.push_row(Row {
                        seq: ev.seq,
                        kind: RowKind::Tool,
                        text: summary,
                        tool_id: Some(tool_id),
                    });
                }
            }
            EventType::PatchProposed => {
                let path = ev
                    .payload
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: format!("patch proposed: {}", path),
                    tool_id: None,
                });
            }
            EventType::PatchApplied => {
                let path = ev
                    .payload
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: format!("patch applied: {}", path),
                    tool_id: None,
                });
            }
            EventType::PermissionRequested => {
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: "permission requested".into(),
                    tool_id: None,
                });
            }
            EventType::PermissionGranted => {
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: "permission granted".into(),
                    tool_id: None,
                });
            }
            EventType::PermissionDenied => {
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: "permission denied".into(),
                    tool_id: None,
                });
            }
            EventType::RuntimeError => {
                let msg = ev
                    .payload
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::Error,
                    text: msg.to_string(),
                    tool_id: None,
                });
            }
            EventType::SessionCompacted => {
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: "session compacted".into(),
                    tool_id: None,
                });
            }
            EventType::ViewportMarker => {
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::Marker,
                    text: ev
                        .payload
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    tool_id: None,
                });
            }
        }
        true
    }
}

impl Default for RenderModel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
    use serde_json::json;
    use uuid::Uuid;

    fn ev(seq: i64, et: EventType, payload: serde_json::Value) -> Event {
        Event {
            id: Uuid::new_v4(),
            session_id: Uuid::nil(),
            seq,
            timestamp_ms: 0,
            schema_version: SCHEMA_VERSION,
            event_type: et,
            payload,
        }
    }

    #[test]
    fn token_deltas_mutate_single_row() {
        let mut m = RenderModel::new();
        m.apply(&ev(0, EventType::AssistantStreamStarted, json!({"stream_id":"s"})));
        for t in ["he", "ll", "o"] {
            m.apply(&ev(1, EventType::AssistantTokenDelta, json!({"stream_id":"s","token":t})));
        }
        m.apply(&ev(2, EventType::AssistantMessageCompleted, json!({"stream_id":"s"})));
        // Only one assistant row, regardless of token count.
        let assistant_rows: Vec<_> = m.rows().iter().filter(|r| r.kind == RowKind::Assistant).collect();
        assert_eq!(assistant_rows.len(), 1);
        assert_eq!(assistant_rows[0].text, "hello");
    }

    #[test]
    fn bounded_memory_evicts_old_rows() {
        let mut m = RenderModel::with_cap(50);
        for i in 0..1_000 {
            m.apply(&ev(i, EventType::UserMessageCreated, json!({"text": format!("{i}")})));
        }
        assert_eq!(m.rows().len(), 50);
        assert_eq!(m.total_rows(), 1_000);
    }

    #[test]
    fn tool_collapses_to_summary() {
        let mut m = RenderModel::new();
        m.apply(&ev(0, EventType::ToolExecutionStarted, json!({
            "tool_id":"t1","tool":"bash","command":"npm test"
        })));
        for _ in 0..10_000 {
            m.apply(&ev(1, EventType::ToolExecutionStdout, json!({
                "tool_id":"t1","chunk":"x"
            })));
        }
        m.apply(&ev(2, EventType::ToolExecutionFinished, json!({
            "tool_id":"t1","exit_code":0
        })));
        let tool_rows: Vec<_> = m.rows().iter().filter(|r| r.kind == RowKind::Tool).collect();
        assert_eq!(tool_rows.len(), 1);
        assert!(tool_rows[0].text.contains("stdout=10000B"));
        assert!(tool_rows[0].text.contains("exit=0"));
    }
}
