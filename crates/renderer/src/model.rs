use camouflage_protocol::{Event, EventType};
use std::collections::{BTreeMap, HashMap, VecDeque};

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
    /// v0.4+ — a single line of a unified diff hunk. The first character of
    /// `Row.text` is the diff marker (`+`, `-`, ` `, `@`, or empty for the
    /// header). Renderers color-code based on that marker.
    Diff,
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
    /// v0.2+: wall-clock timestamp from the ToolExecutionStarted event.
    pub started_ms: i64,
    /// v0.2+: wall-clock timestamp from the ToolExecutionFinished event.
    pub finished_ms: Option<i64>,
    /// v0.4.5+: most recent output captured for the X-overlay. Truncated
    /// to TOOL_OUTPUT_CAP bytes (head + tail with a "… N bytes elided"
    /// marker) so a 100MB tool output doesn't blow up the model.
    pub recent_stdout: String,
    pub recent_stderr: String,
}

/// Per-tool stdout/stderr capture cap, in bytes. Above this we keep the
/// head + tail and elide the middle. Tuned so a typical compiler error
/// fits whole but a giant log dump still costs only a few KB of memory.
pub const TOOL_OUTPUT_CAP: usize = 8_192;

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
    /// v0.1.5+ — host-supplied status-bar segments. Renderer draws these in
    /// a fixed conventional order plus any extras in registration order.
    status_segments: BTreeMap<String, String>,
    /// v0.1.5+ — most recent PermissionRequested whose response is still
    /// pending. The TUI renders an inline modal instead of the input box.
    pending_permission: Option<PendingPermission>,
    /// v0.1.5+ — active background tasks shown in the ribbon above the
    /// status line. Insertion order preserved for stable display.
    background_tasks: Vec<BackgroundTask>,
    /// v0.4.5+ — slash-commands the host advertises. The TUI shows a
    /// picker when the input buffer starts with `/`. Last-write-wins.
    slash_commands: Vec<SlashCmdEntry>,
    /// v0.4.5+ — `@`-mention candidates the host advertises. The TUI
    /// shows a picker when the input cursor is just after an `@`.
    mention_candidates: Vec<MentionEntry>,
    /// TB1 fix (v0.4.5+): dedupe `session started` rows. The TUI used to
    /// synthesize a SessionStarted at boot AND the host typically emits
    /// its own, producing two identical rows. This flag flips on the
    /// first SessionStarted; subsequent ones are no-ops at the row
    /// level (still persisted by the caller).
    session_started_seen: bool,
    /// CC-1 (v0.4.6+): currently-open SelectList instance, if any. The
    /// renderer draws a modal overlay while this is `Some` and routes
    /// ↑/↓/Enter/Esc/typed-chars into picker actions. Cleared when the
    /// user submits or cancels.
    active_select_list: Option<SelectListState>,
    /// CC-2 (v0.4.6+): currently-open Confirm modal, if any.
    active_confirm: Option<ConfirmState>,
}

/// CC-2 — per-instance state for an open Confirm modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmState {
    pub id: String,
    pub prompt: String,
    pub yes_label: String,
    pub no_label: String,
    /// true = yes is selected, false = no.
    pub selected_yes: bool,
    pub allow_cancel: bool,
}

/// CC-1 — per-instance state for an open SelectList modal. Mirrors the
/// `ShowSelectList` payload plus runtime UI state (selected index, filter
/// buffer). Multiple SelectLists could in principle stack; this minimal
/// implementation supports one at a time and ignores subsequent Show
/// events until the open one is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectListState {
    pub id: String,
    pub prompt: String,
    pub options: Vec<SelectListEntry>,
    /// Index into `options` of the currently-highlighted entry.
    pub selected: usize,
    /// Substring filter — typed chars accumulate here, Backspace removes.
    /// Filtering is done at draw-time via [`SelectListState::filtered_indices`].
    pub filter: String,
    pub allow_filter: bool,
    pub allow_cancel: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectListEntry {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

impl SelectListState {
    /// Indices into `options` that match the current `filter` (substring
    /// match on `label`, case-insensitive). When filter is empty, returns
    /// all indices in order.
    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.options.len()).collect();
        }
        let needle = self.filter.to_lowercase();
        self.options
            .iter()
            .enumerate()
            .filter(|(_, e)| e.label.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCmdEntry {
    pub name: String,
    pub description: String,
    pub args_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionEntry {
    pub token: String,
    pub label: Option<String>,
    pub kind: Option<String>,
}

/// State for an in-flight permission request. Cleared once the renderer
/// emits a `PermissionResponse` outbound.
#[derive(Debug, Clone)]
pub struct PendingPermission {
    pub request_id: String,
    pub tool: String,
    pub action: String,
    pub detail: String,
}

/// A row in the background task ribbon, populated by `BackgroundTaskUpdate`
/// events. KimiFlare uses this for skill indexing, memory loading, etc.
#[derive(Debug, Clone)]
pub struct BackgroundTask {
    pub task_id: String,
    pub label: String,
    pub state: BackgroundTaskState,
    pub progress: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundTaskState {
    Running,
    Done,
    Error,
}

impl BackgroundTaskState {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "done" => Some(Self::Done),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
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
            status_segments: BTreeMap::new(),
            pending_permission: None,
            background_tasks: Vec::new(),
            slash_commands: Vec::new(),
            mention_candidates: Vec::new(),
            session_started_seen: false,
            active_select_list: None,
            active_confirm: None,
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

    /// Status-bar segments as currently set by the host.
    pub fn status_segments(&self) -> &BTreeMap<String, String> {
        &self.status_segments
    }

    /// Currently-pending permission request, if any. The TUI renders the
    /// modal widget while this is `Some`.
    pub fn pending_permission(&self) -> Option<&PendingPermission> {
        self.pending_permission.as_ref()
    }

    /// Active background tasks (ribbon between transcript and status).
    /// CC-1 — currently-open SelectList, if any.
    pub fn active_select_list(&self) -> Option<&SelectListState> {
        self.active_select_list.as_ref()
    }

    /// CC-1 — mutable access to the currently-open SelectList. Used by the
    /// TUI to advance the selection, edit the filter, etc.
    pub fn active_select_list_mut(&mut self) -> Option<&mut SelectListState> {
        self.active_select_list.as_mut()
    }

    /// CC-1 — clear the currently-open SelectList (called on submit/cancel
    /// after the response has been emitted).
    pub fn clear_select_list(&mut self) {
        if self.active_select_list.take().is_some() {
            self.dirty = true;
        }
    }

    /// CC-2 — currently-open Confirm, if any.
    pub fn active_confirm(&self) -> Option<&ConfirmState> {
        self.active_confirm.as_ref()
    }

    pub fn active_confirm_mut(&mut self) -> Option<&mut ConfirmState> {
        self.active_confirm.as_mut()
    }

    pub fn clear_confirm(&mut self) {
        if self.active_confirm.take().is_some() {
            self.dirty = true;
        }
    }

    pub fn slash_commands(&self) -> &[SlashCmdEntry] {
        &self.slash_commands
    }

    pub fn mention_candidates(&self) -> &[MentionEntry] {
        &self.mention_candidates
    }

    /// Whether an AssistantStream is currently active (started, not yet
    /// completed). Draw layer checks this to suppress the "empty
    /// assistant row → spinner" branch on stale rows whose stream has
    /// already closed but whose text happens to be empty.
    pub fn has_active_stream(&self) -> bool {
        self.active_stream_row.is_some()
    }

    /// Live-buffer index (within `rows()`) of the currently-active
    /// assistant stream row, if any. Returns None when no stream is in
    /// flight or when the row has been evicted out of the live buffer.
    pub fn active_stream_row(&self) -> Option<usize> {
        self.active_stream_row
    }

    pub fn background_tasks(&self) -> &[BackgroundTask] {
        &self.background_tasks
    }

    /// Called by the TUI when the user has answered. The pending state clears
    /// regardless of the choice; the outbound PermissionResponse event has
    /// already been emitted to the host by the caller.
    pub fn clear_pending_permission(&mut self) {
        if self.pending_permission.is_some() {
            self.pending_permission = None;
            self.dirty = true;
        }
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
                // TB1 fix: don't emit duplicate "session started" rows when
                // the TUI's boot-synth race-condition fires after the host's
                // SessionStarted (or vice-versa).
                if !self.session_started_seen {
                    self.session_started_seen = true;
                    self.push_row(Row {
                        seq: ev.seq,
                        kind: RowKind::System,
                        text: "session started".to_string(),
                        tool_id: None,
                    });
                }
            }
            EventType::SessionEnded => {
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: "session ended".to_string(),
                    tool_id: None,
                });
                // TB2 fix: when a session ends, reset phase to "idle" so the
                // status bar doesn't lie if the host's last StatusUpdate was
                // mid-stream. Other segments are left intact (final tokens /
                // cost / branch remain visible).
                self.status_segments
                    .insert("phase".to_string(), "idle".to_string());
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
                let command_raw = ev
                    .payload
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                // Strip leading/trailing whitespace and collapse newlines so
                // hosts that pass JSON-stringified tool arguments (e.g.
                // KimiFlare's `call.function.arguments`) don't blow up
                // into a multi-row text dump. Full payload remains visible
                // via the X-overlay (tool-output captured) or `i`-inspector.
                let command_display = compact_command(command_raw);
                let command = command_raw.to_string();
                let display = format!("▸ {} {}", tool, command_display);
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
                        started_ms: ev.timestamp_ms,
                        finished_ms: None,
                        recent_stdout: String::new(),
                        recent_stderr: String::new(),
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
                let chunk_str = ev
                    .payload
                    .get("chunk")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if let Some(state) = self.tools.get_mut(tool_id) {
                    if matches!(ev.event_type, EventType::ToolExecutionStdout) {
                        state.stdout_bytes += chunk_len;
                        append_capped(&mut state.recent_stdout, chunk_str);
                    } else {
                        state.stderr_bytes += chunk_len;
                        append_capped(&mut state.recent_stderr, chunk_str);
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
                    state.finished_ms = Some(ev.timestamp_ms);
                    let elapsed_ms = (ev.timestamp_ms - state.started_ms).max(0);
                    let elapsed_str = format_elapsed_ms(elapsed_ms);
                    let summary = format!(
                        "✓ {} {} ({}, exit={}, stdout={}B, stderr={}B)",
                        state.tool,
                        state.command,
                        elapsed_str,
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
                let added = ev.payload.get("added").and_then(|v| v.as_i64()).unwrap_or(0);
                let removed = ev.payload.get("removed").and_then(|v| v.as_i64()).unwrap_or(0);
                // Header row always present.
                let header = if added > 0 || removed > 0 {
                    format!("patch proposed: {} (+{} -{})", path, added, removed)
                } else {
                    format!("patch proposed: {}", path)
                };
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: header,
                    tool_id: None,
                });
                // Optional `diff` field carries a unified diff body. Split
                // into per-line Diff rows, truncated to the v0.4 budget so
                // a 5k-line patch doesn't blow the row cap.
                if let Some(diff) = ev.payload.get("diff").and_then(|v| v.as_str()) {
                    const MAX_DIFF_ROWS: usize = 40;
                    let lines: Vec<&str> = diff.lines().collect();
                    let shown = lines.len().min(MAX_DIFF_ROWS);
                    for line in &lines[..shown] {
                        self.push_row(Row {
                            seq: ev.seq,
                            kind: RowKind::Diff,
                            text: (*line).to_string(),
                            tool_id: None,
                        });
                    }
                    if lines.len() > shown {
                        self.push_row(Row {
                            seq: ev.seq,
                            kind: RowKind::Diff,
                            text: format!("… {} more lines (truncated)", lines.len() - shown),
                            tool_id: None,
                        });
                    }
                }
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
                let request_id = ev
                    .payload
                    .get("request_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool = ev
                    .payload
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let action = ev
                    .payload
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("permission requested")
                    .to_string();
                let detail = ev
                    .payload
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: format!("permission requested: {} ({})", action, tool),
                    tool_id: None,
                });
                self.pending_permission = Some(PendingPermission {
                    request_id,
                    tool,
                    action,
                    detail,
                });
                self.dirty = true;
            }
            EventType::PermissionGranted => {
                self.pending_permission = None;
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::System,
                    text: "permission granted".into(),
                    tool_id: None,
                });
            }
            EventType::PermissionDenied => {
                self.pending_permission = None;
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
                // v0.1.5+: optional `kind`, `severity`, `cta` on the payload.
                // The renderer composes a short row prefix from kind so the
                // TUI can pick a visual treatment without re-parsing here.
                let kind = ev
                    .payload
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("generic");
                let cta_label = ev
                    .payload
                    .get("cta")
                    .and_then(|v| v.get("label"))
                    .and_then(|v| v.as_str());
                let mut text = match kind {
                    "api_error" => format!("[api] {}", msg),
                    "service_ended" => format!("[service ended] {}", msg),
                    "quota_exhausted" => format!("[quota] {}", msg),
                    _ => msg.to_string(),
                };
                if let Some(cta) = cta_label {
                    text.push_str(&format!("  ({})", cta));
                }
                self.push_row(Row {
                    seq: ev.seq,
                    kind: RowKind::Error,
                    text,
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
            EventType::StatusUpdate => {
                if let Some(segs) = ev.payload.get("segments").and_then(|v| v.as_object()) {
                    for (k, v) in segs {
                        if let Some(s) = v.as_str() {
                            if s.is_empty() {
                                self.status_segments.remove(k);
                            } else {
                                self.status_segments.insert(k.clone(), s.to_string());
                            }
                        }
                    }
                    self.dirty = true;
                }
            }
            EventType::BackgroundTaskUpdate => {
                let task_id = ev
                    .payload
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let label = ev
                    .payload
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let state = ev
                    .payload
                    .get("state")
                    .and_then(|v| v.as_str())
                    .and_then(BackgroundTaskState::from_str)
                    .unwrap_or(BackgroundTaskState::Running);
                let progress = ev
                    .payload
                    .get("progress")
                    .and_then(|v| v.as_f64())
                    .map(|f| f as f32);
                // Done/Error → remove from ribbon. Running → upsert.
                match state {
                    BackgroundTaskState::Done | BackgroundTaskState::Error => {
                        self.background_tasks.retain(|t| t.task_id != task_id);
                    }
                    BackgroundTaskState::Running => {
                        if let Some(existing) =
                            self.background_tasks.iter_mut().find(|t| t.task_id == task_id)
                        {
                            existing.label = label;
                            existing.state = state;
                            existing.progress = progress;
                        } else {
                            self.background_tasks.push(BackgroundTask {
                                task_id,
                                label,
                                state,
                                progress,
                            });
                        }
                    }
                }
                self.dirty = true;
            }
            EventType::SlashCommandsRegistered => {
                self.slash_commands.clear();
                if let Some(arr) = ev.payload.get("commands").and_then(|v| v.as_array()) {
                    for v in arr {
                        let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("");
                        if name.is_empty() {
                            continue;
                        }
                        self.slash_commands.push(SlashCmdEntry {
                            name: name.to_string(),
                            description: v.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            args_hint: v.get("args_hint").and_then(|x| x.as_str()).map(str::to_string),
                        });
                    }
                }
                self.dirty = true;
            }
            EventType::MentionCandidatesRegistered => {
                self.mention_candidates.clear();
                if let Some(arr) = ev.payload.get("candidates").and_then(|v| v.as_array()) {
                    for v in arr {
                        let token = v.get("token").and_then(|x| x.as_str()).unwrap_or("");
                        if token.is_empty() {
                            continue;
                        }
                        self.mention_candidates.push(MentionEntry {
                            token: token.to_string(),
                            label: v.get("label").and_then(|x| x.as_str()).map(str::to_string),
                            kind: v.get("kind").and_then(|x| x.as_str()).map(str::to_string),
                        });
                    }
                }
                self.dirty = true;
            }
            // Outbound events never reach apply() in practice, but be tolerant
            // (a replayed session log might contain them).
            EventType::UserInputSubmitted | EventType::PermissionResponse => {}
            // CC-1 — when replaying a stored session, a SelectListResponse
            // means the SelectList resolved; clear the active state so the
            // snapshot reflects the post-resolution UI.
            EventType::SelectListResponse => {
                self.active_select_list = None;
                self.dirty = true;
            }
            // CC-2 — same pattern as SelectListResponse: replayed responses
            // resolve the modal.
            EventType::ConfirmResponse => {
                self.active_confirm = None;
                self.dirty = true;
            }
            EventType::ShowConfirm => {
                if self.active_confirm.is_some() {
                    return false;
                }
                let id = ev.payload.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let prompt = ev.payload.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if id.is_empty() {
                    return false;
                }
                let yes_label = ev
                    .payload
                    .get("yes_label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Yes")
                    .to_string();
                let no_label = ev
                    .payload
                    .get("no_label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No")
                    .to_string();
                let selected_yes = ev
                    .payload
                    .get("default")
                    .and_then(|v| v.as_str())
                    .map(|s| s != "no")
                    .unwrap_or(true);
                let allow_cancel = ev
                    .payload
                    .get("allow_cancel")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                self.active_confirm = Some(ConfirmState {
                    id,
                    prompt,
                    yes_label,
                    no_label,
                    selected_yes,
                    allow_cancel,
                });
                self.dirty = true;
            }
            EventType::ShowSelectList => {
                // Ignore if another SelectList is already open — the renderer
                // is single-instance for this primitive; queueing is host
                // concern. Hosts should resolve the pending one (cancel or
                // wait for response) before showing another.
                if self.active_select_list.is_some() {
                    return false;
                }
                let id = ev.payload.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let prompt = ev.payload.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if id.is_empty() {
                    return false; // malformed; host must supply an id
                }
                let options: Vec<SelectListEntry> = ev
                    .payload
                    .get("options")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|opt| {
                                let value = opt.get("value").and_then(|v| v.as_str())?;
                                let label = opt.get("label").and_then(|v| v.as_str())?;
                                let description = opt
                                    .get("description")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string);
                                Some(SelectListEntry {
                                    value: value.to_string(),
                                    label: label.to_string(),
                                    description,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if options.is_empty() {
                    return false; // nothing to pick from; no-op
                }
                let default = ev.payload.get("default").and_then(|v| v.as_str());
                let selected = default
                    .and_then(|d| options.iter().position(|o| o.value == d))
                    .unwrap_or(0);
                let allow_filter = ev
                    .payload
                    .get("allow_filter")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let allow_cancel = ev
                    .payload
                    .get("allow_cancel")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                self.active_select_list = Some(SelectListState {
                    id,
                    prompt,
                    options,
                    selected,
                    filter: String::new(),
                    allow_filter,
                    allow_cancel,
                });
                self.dirty = true;
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

/// Format milliseconds as a short human-friendly elapsed string:
/// "234ms", "1.2s", "1m 23s".
/// Append `chunk` to `buf`, keeping the total under [`TOOL_OUTPUT_CAP`] by
/// eliding the middle if it would overflow. Always preserves the head
/// (first N/2 bytes) + tail (last N/2 bytes) with a `… N bytes elided`
/// marker — useful when the user scrolls up to see early errors.
fn append_capped(buf: &mut String, chunk: &str) {
    if buf.len() + chunk.len() <= TOOL_OUTPUT_CAP {
        buf.push_str(chunk);
        return;
    }
    let combined = format!("{buf}{chunk}");
    let total = combined.len();
    let head_len = TOOL_OUTPUT_CAP / 2;
    let tail_len = TOOL_OUTPUT_CAP / 2;
    let head_end = combined
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= head_len)
        .last()
        .unwrap_or(0);
    let tail_start = combined
        .char_indices()
        .rev()
        .map(|(i, _)| i)
        .take_while(|&i| total - i <= tail_len)
        .last()
        .unwrap_or(total);
    let head = &combined[..head_end];
    let tail = &combined[tail_start..];
    let elided = total.saturating_sub(head.len() + tail.len());
    *buf = format!("{head}\n… {elided} bytes elided …\n{tail}");
}

/// Compact a tool's command-line into something safe to render on a single
/// row of a transcript. Collapses whitespace, escapes literal control
/// chars that show as garbage in a TUI, and truncates to ~120 visible
/// chars with an ellipsis. Used for the `▸ tool …` display string; the
/// original full payload is preserved on `ToolState.command`.
pub fn compact_command(raw: &str) -> String {
    // Collapse any whitespace run (including \n / \r / \t) to a single
    // space, drop other control chars. Tracks UTF-8 codepoints so we
    // truncate on a char boundary.
    let mut out = String::with_capacity(raw.len().min(140));
    let mut prev_space = false;
    let mut count: usize = 0;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                count += 1;
                prev_space = true;
            }
            continue;
        }
        if ch.is_control() {
            continue;
        }
        prev_space = false;
        if count >= 120 {
            out.push('…');
            return out;
        }
        out.push(ch);
        count += 1;
    }
    // Trim trailing space we may have left while iterating.
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

pub fn format_elapsed_ms(ms: i64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let s = ms / 1000;
        format!("{}m {}s", s / 60, s % 60)
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
    fn patch_proposed_without_diff_emits_header_only() {
        let mut m = RenderModel::new();
        m.apply(&ev(
            0,
            EventType::PatchProposed,
            json!({"path":"src/x.rs","added":3,"removed":1}),
        ));
        let rows: Vec<_> = m.rows().iter().collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, RowKind::System);
        assert!(rows[0].text.contains("src/x.rs"));
        assert!(rows[0].text.contains("+3 -1"));
    }

    #[test]
    fn patch_proposed_with_diff_splits_into_diff_rows() {
        let diff = "@@ -1,3 +1,4 @@\n use foo;\n-let x = 1;\n+let x = 2;\n+let y = 3;\n more";
        let mut m = RenderModel::new();
        m.apply(&ev(
            0,
            EventType::PatchProposed,
            json!({"path":"src/x.rs","added":2,"removed":1,"diff":diff}),
        ));
        let kinds: Vec<_> = m.rows().iter().map(|r| r.kind.clone()).collect();
        // header (System) + 6 diff lines
        assert_eq!(kinds[0], RowKind::System);
        assert!(kinds[1..].iter().all(|k| *k == RowKind::Diff));
        assert_eq!(kinds.len(), 7);
        // Spot-check marker preservation: lines start with @, space, -, +, +, space
        let firsts: Vec<char> = m.rows()
            .iter()
            .skip(1)
            .map(|r| r.text.chars().next().unwrap_or(' '))
            .collect();
        assert_eq!(firsts, vec!['@', ' ', '-', '+', '+', ' ']);
    }

    #[test]
    fn patch_proposed_truncates_long_diffs() {
        let big: String = (0..100).map(|i| format!("+line {i}\n")).collect();
        let mut m = RenderModel::new();
        m.apply(&ev(
            0,
            EventType::PatchProposed,
            json!({"path":"src/x.rs","diff":big}),
        ));
        let diff_rows: Vec<_> = m.rows().iter().filter(|r| r.kind == RowKind::Diff).collect();
        // 40 diff lines + 1 truncation footer
        assert_eq!(diff_rows.len(), 41);
        assert!(diff_rows.last().unwrap().text.contains("more lines"));
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
    fn status_update_merges_and_removes_segments() {
        let mut m = RenderModel::new();
        m.apply(&ev(0, EventType::StatusUpdate, json!({
            "segments": {"mode": "edit", "phase": "thinking", "tokens": "in 12k"}
        })));
        assert_eq!(m.status_segments().get("mode").map(|s| s.as_str()), Some("edit"));
        assert_eq!(m.status_segments().len(), 3);

        // Override and remove (empty value = remove).
        m.apply(&ev(1, EventType::StatusUpdate, json!({
            "segments": {"phase": "streaming", "tokens": ""}
        })));
        assert_eq!(m.status_segments().get("phase").map(|s| s.as_str()), Some("streaming"));
        assert!(m.status_segments().get("tokens").is_none());
        assert_eq!(m.status_segments().len(), 2);
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
