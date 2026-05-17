//! Camouflage event protocol.
//!
//! Events are the canonical source of truth. Every event is append-only,
//! serializable to JSON, and replayable. The renderer is a subscriber.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub id: Uuid,
    pub session_id: Uuid,
    pub seq: i64,
    pub timestamp_ms: i64,
    pub schema_version: u32,
    pub event_type: EventType,
    pub payload: serde_json::Value,
}

impl Event {
    pub fn new(session_id: Uuid, seq: i64, event_type: EventType, payload: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            seq,
            timestamp_ms: now_ms(),
            schema_version: SCHEMA_VERSION,
            event_type,
            payload,
        }
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EventType {
    SessionStarted,
    SessionEnded,
    UserMessageCreated,
    AssistantStreamStarted,
    AssistantTokenDelta,
    AssistantMessageCompleted,
    ToolExecutionStarted,
    ToolExecutionStdout,
    ToolExecutionStderr,
    ToolExecutionFinished,
    PatchProposed,
    PatchApplied,
    PermissionRequested,
    PermissionGranted,
    PermissionDenied,
    RuntimeError,
    SessionCompacted,
    ViewportMarker,
    /// v0.1.5+ — Host → renderer: status-bar segment key/value updates.
    StatusUpdate,
    /// v0.1.5+ — Host → renderer: background task lifecycle (skills index,
    /// memory load, etc.) shown in the task ribbon above the status line.
    BackgroundTaskUpdate,
    /// v0.1.5+ — Renderer → host: user submitted input from the input box.
    UserInputSubmitted,
    /// v0.1.5+ — Renderer → host: user's response to a PermissionRequested.
    PermissionResponse,
    /// v0.4.5+ — Host → renderer: registers the list of slash-commands the
    /// host accepts (e.g. /compact, /clear, /help). The TUI shows a picker
    /// overlay when the user types `/` at the start of an input buffer.
    /// Selection is delivered back via the existing `UserInputSubmitted`
    /// path (`/commandname args…`) — no new outbound event is needed.
    SlashCommandsRegistered,
    /// v0.4.5+ — Host → renderer: registers `@`-mention candidates (e.g.
    /// file paths, symbol names). When the user types `@` mid-input, the
    /// picker fuzzy-matches against these. Same submission story as
    /// SlashCommandsRegistered.
    MentionCandidatesRegistered,
    /// v0.4.6+ (CC-1) — Host → renderer: render a modal SelectList. The
    /// first of the "components catalog" primitives (see
    /// `docs/specs/components-catalog.md`). User's pick is reported back
    /// via `SelectListResponse` keyed by the same `id`.
    ShowSelectList,
    /// v0.4.6+ (CC-1) — Renderer → host: outcome of a `ShowSelectList`.
    /// Payload carries either `value` (a successful pick) or
    /// `cancelled: true` (user dismissed with Esc / Ctrl+C).
    SelectListResponse,
    /// v0.4.6+ (CC-2) — Host → renderer: show a Yes/No modal confirmation.
    /// The user's choice comes back via `ConfirmResponse`. Lighter sibling
    /// of `PermissionRequested` (which keeps its own type because it
    /// carries permission-specific extras).
    ShowConfirm,
    /// v0.4.6+ (CC-2) — Renderer → host: outcome of a `ShowConfirm`.
    /// Payload carries either `value: bool` or `cancelled: true`.
    ConfirmResponse,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::SessionStarted => "SessionStarted",
            EventType::SessionEnded => "SessionEnded",
            EventType::UserMessageCreated => "UserMessageCreated",
            EventType::AssistantStreamStarted => "AssistantStreamStarted",
            EventType::AssistantTokenDelta => "AssistantTokenDelta",
            EventType::AssistantMessageCompleted => "AssistantMessageCompleted",
            EventType::ToolExecutionStarted => "ToolExecutionStarted",
            EventType::ToolExecutionStdout => "ToolExecutionStdout",
            EventType::ToolExecutionStderr => "ToolExecutionStderr",
            EventType::ToolExecutionFinished => "ToolExecutionFinished",
            EventType::PatchProposed => "PatchProposed",
            EventType::PatchApplied => "PatchApplied",
            EventType::PermissionRequested => "PermissionRequested",
            EventType::PermissionGranted => "PermissionGranted",
            EventType::PermissionDenied => "PermissionDenied",
            EventType::RuntimeError => "RuntimeError",
            EventType::SessionCompacted => "SessionCompacted",
            EventType::ViewportMarker => "ViewportMarker",
            EventType::StatusUpdate => "StatusUpdate",
            EventType::BackgroundTaskUpdate => "BackgroundTaskUpdate",
            EventType::UserInputSubmitted => "UserInputSubmitted",
            EventType::PermissionResponse => "PermissionResponse",
            EventType::SlashCommandsRegistered => "SlashCommandsRegistered",
            EventType::MentionCandidatesRegistered => "MentionCandidatesRegistered",
            EventType::ShowSelectList => "ShowSelectList",
            EventType::SelectListResponse => "SelectListResponse",
            EventType::ShowConfirm => "ShowConfirm",
            EventType::ConfirmResponse => "ConfirmResponse",
        }
    }

    /// Direction this event flows on the wire.
    pub fn direction(&self) -> Direction {
        match self {
            EventType::UserInputSubmitted
            | EventType::PermissionResponse
            | EventType::SelectListResponse
            | EventType::ConfirmResponse => Direction::Outbound,
            _ => Direction::Inbound,
        }
    }
}

/// On-wire direction relative to the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Host → renderer (NDJSON read from stdin).
    Inbound,
    /// Renderer → host (NDJSON written to stdout).
    Outbound,
}

pub mod payloads {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct UserMessage {
        pub text: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AssistantStreamStarted {
        pub stream_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AssistantTokenDelta {
        pub stream_id: String,
        pub token: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AssistantMessageCompleted {
        pub stream_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ToolStarted {
        pub tool_id: String,
        pub tool: String,
        pub command: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ToolOutput {
        pub tool_id: String,
        pub chunk: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ToolFinished {
        pub tool_id: String,
        pub exit_code: i32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct RuntimeError {
        pub message: String,
        #[serde(default)]
        pub source: Option<String>,
        /// v0.1.5+ — categorises the error so the renderer picks an
        /// appropriate visual treatment. Optional for backward compat.
        #[serde(default)]
        pub kind: Option<RuntimeErrorKind>,
        #[serde(default)]
        pub severity: Option<Severity>,
        /// Call-to-action shown beneath the error (e.g. "type /report").
        #[serde(default)]
        pub cta: Option<Cta>,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum RuntimeErrorKind {
        Generic,
        ApiError,
        ServiceEnded,
        QuotaExhausted,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum Severity {
        Info,
        Warn,
        Error,
        Fatal,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct Cta {
        pub label: String,
        pub action_id: String,
    }

    /// v0.1.5+ — host updates one or more status-bar segments.
    ///
    /// Renderer maintains a key→value map. A segment with an empty value is
    /// removed. Conventional keys (renderer treats them as well-known when
    /// composing the status line in order):
    ///
    /// - `mode`     — short badge (`edit`, `plan`, `auto`)
    /// - `phase`    — `idle`, `thinking`, `streaming`, `tool`, `error`
    /// - `elapsed`  — already-formatted elapsed time (e.g. `1m 23s`)
    /// - `tokens`   — e.g. `in 12k`
    /// - `cost`     — e.g. `$0.03`
    /// - `branch`   — git branch
    /// - `warn`     — extra warning text (shown in yellow)
    ///
    /// Unknown keys are still displayed (in registration order) so hosts can
    /// freely extend the bar.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct StatusUpdate {
        pub segments: std::collections::BTreeMap<String, String>,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum BackgroundTaskState {
        Running,
        Done,
        Error,
    }

    /// v0.1.5+ — background task lifecycle (skill indexing, memory load…).
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct BackgroundTaskUpdate {
        pub task_id: String,
        pub label: String,
        pub state: BackgroundTaskState,
        /// 0.0..=1.0 if known, else None.
        #[serde(default)]
        pub progress: Option<f32>,
    }

    /// v0.1.5+ — renderer → host: user submitted text from the input box.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct UserInputSubmitted {
        pub text: String,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum PermissionChoice {
        AllowOnce,
        AllowSession,
        Deny,
    }

    /// v0.1.5+ — renderer → host: response to a PermissionRequested event.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct PermissionResponse {
        /// Matches `request_id` from the original PermissionRequested payload.
        pub request_id: String,
        pub choice: PermissionChoice,
        #[serde(default)]
        pub feedback: Option<String>,
    }

    /// v0.4.5+ — one entry in a SlashCommandsRegistered payload.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct SlashCommand {
        /// The literal command name *without* the leading slash, e.g. `compact`.
        pub name: String,
        /// One-line description shown in the picker.
        #[serde(default)]
        pub description: String,
        /// Optional arg hint shown after the name, e.g. `<path>`.
        #[serde(default)]
        pub args_hint: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct SlashCommandsRegistered {
        pub commands: Vec<SlashCommand>,
    }

    /// v0.4.5+ — one entry in a MentionCandidatesRegistered payload.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct MentionCandidate {
        /// What gets inserted into the input (e.g. `src/auth/login.ts`).
        pub token: String,
        /// Optional human-readable label shown alongside; defaults to `token`.
        #[serde(default)]
        pub label: Option<String>,
        /// Optional category tag (e.g. `file`, `symbol`, `commit`).
        #[serde(default)]
        pub kind: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct MentionCandidatesRegistered {
        pub candidates: Vec<MentionCandidate>,
    }

    /// v0.4.6+ (CC-1) — one option in a `ShowSelectList`.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct SelectListOption {
        /// Stable opaque token returned to the host in `SelectListResponse`.
        /// Hosts typically use a session id, file path, command name, etc.
        pub value: String,
        /// Human-readable label shown in the picker.
        pub label: String,
        /// Optional one-line description shown dimmed to the right of `label`.
        #[serde(default)]
        pub description: Option<String>,
    }

    /// v0.4.6+ (CC-1) — host asks the renderer to show a modal select list.
    /// See `docs/specs/components-catalog.md` for the full design.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ShowSelectList {
        /// Host-chosen unique id. Reported back in `SelectListResponse` so
        /// the host can correlate the response with the request. Multiple
        /// SelectLists can stack — each has its own id.
        pub id: String,
        /// Prompt shown above the option list (e.g. "Resume which session?").
        pub prompt: String,
        pub options: Vec<SelectListOption>,
        /// Initial selection (must match an option `value`). Defaults to
        /// the first option when omitted or unmatched.
        #[serde(default)]
        pub default: Option<String>,
        /// When true, the user can type characters to filter the visible
        /// list by substring match on `label`. Default true.
        #[serde(default = "default_true")]
        pub allow_filter: bool,
        /// When true, Esc / Ctrl+C dismisses without selecting (response
        /// carries `cancelled: true`). Default true.
        #[serde(default = "default_true")]
        pub allow_cancel: bool,
    }

    fn default_true() -> bool { true }

    /// v0.4.6+ (CC-1) — outbound result of `ShowSelectList`. Exactly one
    /// of `value` or `cancelled` is set per response.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct SelectListResponse {
        pub id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub value: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub cancelled: bool,
    }

    /// v0.4.6+ (CC-2) — host asks the renderer to show a Yes/No modal.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ShowConfirm {
        pub id: String,
        pub prompt: String,
        /// Label for the affirmative button. Default "Yes".
        #[serde(default)]
        pub yes_label: Option<String>,
        /// Label for the negative button. Default "No".
        #[serde(default)]
        pub no_label: Option<String>,
        /// Which button is initially selected. "yes" or "no". Default "yes".
        #[serde(default)]
        pub default: Option<String>,
        /// When true, Esc / Ctrl+C dismisses without choosing (response
        /// carries `cancelled: true`). Default true.
        #[serde(default = "default_true")]
        pub allow_cancel: bool,
    }

    /// v0.4.6+ (CC-2) — outbound result of `ShowConfirm`. Exactly one of
    /// `value` (bool) or `cancelled` is set.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ConfirmResponse {
        pub id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub value: Option<bool>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub cancelled: bool,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(event_type: EventType, payload: serde_json::Value) -> Event {
        Event {
            id: Uuid::nil(),
            session_id: Uuid::nil(),
            seq: 1,
            timestamp_ms: 0,
            schema_version: SCHEMA_VERSION,
            event_type,
            payload,
        }
    }

    #[test]
    fn roundtrip_all_event_types() {
        let types = [
            EventType::SessionStarted,
            EventType::SessionEnded,
            EventType::UserMessageCreated,
            EventType::AssistantStreamStarted,
            EventType::AssistantTokenDelta,
            EventType::AssistantMessageCompleted,
            EventType::ToolExecutionStarted,
            EventType::ToolExecutionStdout,
            EventType::ToolExecutionStderr,
            EventType::ToolExecutionFinished,
            EventType::PatchProposed,
            EventType::PatchApplied,
            EventType::PermissionRequested,
            EventType::PermissionGranted,
            EventType::PermissionDenied,
            EventType::RuntimeError,
            EventType::SessionCompacted,
            EventType::ViewportMarker,
            EventType::StatusUpdate,
            EventType::BackgroundTaskUpdate,
            EventType::UserInputSubmitted,
            EventType::PermissionResponse,
            EventType::SlashCommandsRegistered,
            EventType::MentionCandidatesRegistered,
            EventType::ShowSelectList,
            EventType::SelectListResponse,
            EventType::ShowConfirm,
            EventType::ConfirmResponse,
        ];
        assert_eq!(types.len(), 28);
        for t in types {
            let ev = sample(t, json!({"k": "v"}));
            let s = serde_json::to_string(&ev).unwrap();
            let back: Event = serde_json::from_str(&s).unwrap();
            assert_eq!(ev, back);
        }
    }

    #[test]
    fn direction_classification() {
        assert_eq!(EventType::SessionStarted.direction(), Direction::Inbound);
        assert_eq!(EventType::StatusUpdate.direction(), Direction::Inbound);
        assert_eq!(EventType::UserInputSubmitted.direction(), Direction::Outbound);
        assert_eq!(EventType::PermissionResponse.direction(), Direction::Outbound);
    }

    #[test]
    fn status_update_payload_roundtrip() {
        let mut segs = std::collections::BTreeMap::new();
        segs.insert("mode".to_string(), "edit".to_string());
        segs.insert("phase".to_string(), "thinking".to_string());
        let p = payloads::StatusUpdate { segments: segs };
        let s = serde_json::to_string(&p).unwrap();
        let back: payloads::StatusUpdate = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn runtime_error_with_kind_roundtrip() {
        let p = payloads::RuntimeError {
            message: "rate limit".into(),
            source: Some("openai".into()),
            kind: Some(payloads::RuntimeErrorKind::QuotaExhausted),
            severity: Some(payloads::Severity::Error),
            cta: Some(payloads::Cta { label: "type /report".into(), action_id: "report".into() }),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: payloads::RuntimeError = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn runtime_error_legacy_payload_roundtrip() {
        // A legacy RuntimeError with only message + source (no kind/severity/cta)
        // must still deserialise — additive field guarantee.
        let raw = r#"{"message":"oops","source":"x"}"#;
        let p: payloads::RuntimeError = serde_json::from_str(raw).unwrap();
        assert_eq!(p.message, "oops");
        assert!(p.kind.is_none());
        assert!(p.severity.is_none());
        assert!(p.cta.is_none());
    }

    #[test]
    fn token_delta_payload_roundtrip() {
        let p = payloads::AssistantTokenDelta {
            stream_id: "s1".into(),
            token: "hello".into(),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: payloads::AssistantTokenDelta = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
