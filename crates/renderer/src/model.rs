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
///
/// Two row buffers exist:
/// - `rows` (live, bounded VecDeque): the newest `row_cap` rows produced by
///   `apply()`. Older rows are evicted from this buffer.
/// - `history` (paged, unbounded Vec): older rows reconstructed on demand
///   from the persistence layer via `prepend_history`. Rendered above
///   `rows`. Cleared on jump-to-latest to bound memory.
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
    /// Reconstructed older rows, displayed *before* `rows` in the viewport.
    /// Grows as the user scrolls upward; cleared on `clear_history`.
    history: Vec<Row>,
}

impl RenderModel {
    pub fn new() -> Self {
        Self::with_cap(DEFAULT_ROW_CAP)
    }

    pub fn with_cap(cap: usize) -> Self {
        // Use `min(cap, 1024)` for the initial allocation to avoid allocating
        // gigabytes when `cap == usize::MAX` (used by history reconstruction).
        let initial = cap.min(1024).max(16);
        Self {
            rows: VecDeque::with_capacity(initial),
            row_cap: cap.max(16),
            total_rows: 0,
            active_stream_row: None,
            active_stream_id: None,
            tools: HashMap::new(),
            dirty: true,
            history: Vec::new(),
        }
    }

    /// Cap used by [`with_cap`] for fully unbounded reconstruction
    /// (used by history reconstruction in the TUI).
    pub fn unbounded() -> Self {
        Self::with_cap(usize::MAX)
    }

    /// Prepend already-reconstructed rows to the history buffer. Caller is
    /// responsible for passing rows in increasing-seq order and for ensuring
    /// they belong strictly before any row already in this model.
    pub fn prepend_history(&mut self, rows: Vec<Row>) {
        if rows.is_empty() {
            return;
        }
        // history is stored in display order (oldest first). Prepend means
        // these new rows go BEFORE existing history rows.
        let mut new_history = rows;
        new_history.extend(self.history.drain(..));
        self.history = new_history;
        self.dirty = true;
    }

    /// Drop all history rows (e.g. on jump-to-latest). Memory-bounding the
    /// model is only meaningful if this is called sometimes.
    pub fn clear_history(&mut self) {
        if !self.history.is_empty() {
            self.history.clear();
            self.dirty = true;
        }
    }

    pub fn history_rows(&self) -> &[Row] {
        &self.history
    }

    /// Combined visible row count: history + live.
    pub fn combined_len(&self) -> usize {
        self.history.len() + self.rows.len()
    }

    /// First seq currently visible in the model (history's first row, or
    /// live's first row if no history). None if both are empty.
    pub fn earliest_visible_seq(&self) -> Option<i64> {
        if let Some(r) = self.history.first() {
            Some(r.seq)
        } else {
            self.rows.front().map(|r| r.seq)
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

/// Reconstruct render rows from a slice of events using an unbounded
/// temporary `RenderModel`. Used to backfill the history buffer when the
/// user scrolls past the live ring buffer.
pub fn reconstruct_rows(events: &[Event]) -> Vec<Row> {
    let mut tmp = RenderModel::unbounded();
    for ev in events {
        tmp.apply(ev);
    }
    tmp.rows.into_iter().collect()
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
    fn reconstruct_rows_matches_live_apply() {
        // Apply a mixed event sequence to a normal model and to
        // reconstruct_rows; they should produce identical row text.
        let events: Vec<Event> = vec![
            ev(0, EventType::SessionStarted, json!({})),
            ev(1, EventType::UserMessageCreated, json!({"text":"hi"})),
            ev(2, EventType::AssistantStreamStarted, json!({"stream_id":"s"})),
            ev(3, EventType::AssistantTokenDelta, json!({"stream_id":"s","token":"hel"})),
            ev(4, EventType::AssistantTokenDelta, json!({"stream_id":"s","token":"lo"})),
            ev(5, EventType::AssistantMessageCompleted, json!({"stream_id":"s"})),
            ev(6, EventType::ToolExecutionStarted, json!({"tool_id":"t","tool":"bash","command":"ls"})),
            ev(7, EventType::ToolExecutionStdout, json!({"tool_id":"t","chunk":"a"})),
            ev(8, EventType::ToolExecutionFinished, json!({"tool_id":"t","exit_code":0})),
        ];
        let mut live = RenderModel::new();
        for e in &events {
            live.apply(e);
        }
        let reconstructed = reconstruct_rows(&events);
        let live_rows: Vec<_> = live.rows().iter().cloned().collect();
        assert_eq!(reconstructed.len(), live_rows.len());
        for (a, b) in reconstructed.iter().zip(live_rows.iter()) {
            assert_eq!(a.text, b.text);
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.seq, b.seq);
        }
    }

    #[test]
    fn history_prepend_and_clear() {
        let mut m = RenderModel::with_cap(50);
        // Live: 30 rows.
        for i in 100..130 {
            m.apply(&ev(i, EventType::UserMessageCreated, json!({"text": format!("u{i}")})));
        }
        assert_eq!(m.combined_len(), 30);
        // Now simulate scrolling up: reconstruct 50 older rows from a fake
        // event slice and prepend.
        let older: Vec<Event> = (50..100)
            .map(|i| ev(i, EventType::UserMessageCreated, json!({"text": format!("o{i}")})))
            .collect();
        let older_rows = reconstruct_rows(&older);
        assert_eq!(older_rows.len(), 50);
        m.prepend_history(older_rows);
        assert_eq!(m.combined_len(), 80);
        assert_eq!(m.history_rows().len(), 50);
        assert_eq!(m.earliest_visible_seq(), Some(50));
        // Another older chunk; should prepend BEFORE existing history.
        let oldest: Vec<Event> = (0..50)
            .map(|i| ev(i, EventType::UserMessageCreated, json!({"text": format!("oldest{i}")})))
            .collect();
        m.prepend_history(reconstruct_rows(&oldest));
        assert_eq!(m.combined_len(), 130);
        assert_eq!(m.earliest_visible_seq(), Some(0));
        assert_eq!(m.history_rows().first().unwrap().text, "oldest0");
        // Clear history.
        m.clear_history();
        assert_eq!(m.combined_len(), 30);
        assert_eq!(m.history_rows().len(), 0);
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
