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
    /// v0.5+ — Host → renderer: the agent's todo/plan checklist. Carries the
    /// FULL list every time (last-write-wins, no per-item deltas) — an empty
    /// list clears it. Rendered as a vertical checklist (☐ pending / ◐ in
    /// progress / ☑ completed), distinct from the `BackgroundTaskUpdate` job
    /// ribbon.
    TodoListUpdate,
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
    /// `docs/historical/components-catalog.md`). User's pick is reported back
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
    /// v0.4.6+ (CC-6) — Host → renderer: show a tabular data view as a
    /// modal. Display-only in this version; interactive row selection
    /// + inline mode are follow-ups.
    ShowTable,
    /// v0.4.6+ (CC-7) — Host → renderer: show a label/value list (session
    /// details, welcome screen, "about" panel). Display-only.
    ShowKeyValueView,
    /// v0.4.6+ (CC-5) — Host → renderer: show a multi-field form modal.
    /// User submits → FormResponse with field values, or cancels → same
    /// event with `cancelled: true`.
    ShowForm,
    /// v0.4.6+ (CC-5) — Renderer → host: outcome of a `ShowForm`.
    FormResponse,
    /// v0.4.6+ (CC-4) — Host → renderer: multi-step flow that composes
    /// Select / Confirm / Form into a guided wizard. Renderer drives each
    /// step in order; per-step results accumulate; final results come
    /// back as `WizardCompleted`. Cancel anywhere → `WizardCancelled`.
    ShowWizard,
    /// v0.4.6+ (CC-4) — Renderer → host: all steps completed.
    WizardCompleted,
    /// v0.4.6+ (CC-4) — Renderer → host: user cancelled mid-wizard.
    WizardCancelled,
    /// v0.4.7+ — Renderer → host: user pressed Tab/Shift+Tab on an empty
    /// input. Hosts that want mode cycling (e.g. an
    /// edit/plan/auto toggle) handle this and emit a StatusUpdate to reflect
    /// the new mode. Payload: { direction: "next" | "prev" }.
    ModeChangeRequested,
    /// v0.4.7+ — Renderer → host: user wants the current operation
    /// cancelled (Ctrl+C or Esc pressed). Hosts typically call
    /// `controller.abort()` on the in-flight agent turn. No payload.
    CancelRequested,
    /// v0.4.8+ — Host → renderer: wipe the visible transcript (used by
    /// `/clear` in host CLIs). Auto-follow resumes at the bottom; status
    /// bar and registered slash-commands persist. No payload.
    TranscriptCleared,
    /// v0.4.9+ — Host → renderer: pin a multi-line ANSI splash above the
    /// transcript (e.g. the host's CLI logo + version line). Renderer
    /// keeps it visible until the user submits their first prompt, then
    /// drops it so it doesn't eat transcript real-estate forever.
    /// Payload: `{ "text": "...multi-line ANSI string..." }`.
    Splash,
    /// v0.5+ — Host → renderer: show a transient toast notification near the
    /// bottom of the screen. Unlike `RuntimeError` (a permanent transcript
    /// row) a toast auto-dismisses after `ttl_ms` and never consumes
    /// transcript history — the right surface for ephemeral feedback like
    /// "mode: plan", "saved", "interrupted". Payload:
    /// `{ "text": String, "kind"?: "info"|"warn"|"error"|"success", "ttl_ms"?: u64 }`.
    ShowToast,
    /// v2.3+ — Renderer → host: the user is typing a path mention
    /// (`@./`, `@../`, `@~/`, `@/`). Payload `{ "query": "<text after @>" }`.
    /// The host answers with `MentionCandidatesRegistered` carrying
    /// `for_query` (the directory part it listed) and that directory's
    /// entries; folder tokens end in `/`. Sent once per directory.
    MentionQuery,
    /// v2.4+ — Host → renderer: a chunk of the model's reasoning for a
    /// stream, sent before (or between) its `AssistantTokenDelta`s. Payload
    /// is the same as `AssistantTokenDelta`. Hidden until the user presses
    /// Ctrl+R; renderers that don't show reasoning ignore it.
    AssistantReasoningDelta,
    /// v2.4+ — Host → renderer: give the terminal to a child process (e.g.
    /// a `!` shell command). The inline renderer clears its live area,
    /// leaves raw mode, stops reading keys, then answers
    /// `TerminalSuspended`. Payload `{ "id": "<any>" }`.
    TerminalSuspend,
    /// v2.4+ — Host → renderer: take the terminal back after a
    /// `TerminalSuspend`; the renderer re-enters raw mode and redraws below
    /// whatever the child printed. Payload `{ "id": "<any>" }`.
    TerminalResume,
    /// v2.4+ — Renderer → host: the terminal is free. Payload
    /// `{ "id": "<same id>", "supported": bool }`; `supported: false` means
    /// this renderer can't hand the terminal over (Windows, full-screen
    /// mode), so the host should run the child without a terminal.
    TerminalSuspended,
    /// v2.4+ — Host → renderer: add or replace one background activity item
    /// (a job or an agent). Payload: `payloads::ActivityItem`. Fields left
    /// out keep their previous values; output is kept.
    ActivityUpdate,
    /// v2.4+ — Host → renderer: append output to an item (`{id, chunk}`).
    ActivityLog,
    /// v2.4+ — Host → renderer: drop an item (`{id}`).
    ActivityRemoved,
    /// v2.4+ — Host → renderer: the complete list; items not in it are
    /// dropped (resync after reconnects so nothing stale stays on screen).
    ActivitySnapshot,
    /// v2.4+ — Host → renderer: open the activity browser (e.g. for `/jobs`).
    ActivityBrowserOpen,
    /// v2.4+ — Renderer → host: the user confirmed stopping an item (`{id}`).
    /// The host decides; report the result with `ActivityUpdate`.
    ActivityStopRequested,
    /// v2.4+ — Renderer → host: the browser opened, closed, or shows an
    /// item's details (`{view: "list" | "detail" | "closed", id?}`), e.g. to
    /// stream an item's output only while it's on screen.
    ActivityViewChanged,
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
            EventType::TodoListUpdate => "TodoListUpdate",
            EventType::UserInputSubmitted => "UserInputSubmitted",
            EventType::PermissionResponse => "PermissionResponse",
            EventType::SlashCommandsRegistered => "SlashCommandsRegistered",
            EventType::MentionCandidatesRegistered => "MentionCandidatesRegistered",
            EventType::ShowSelectList => "ShowSelectList",
            EventType::SelectListResponse => "SelectListResponse",
            EventType::ShowConfirm => "ShowConfirm",
            EventType::ConfirmResponse => "ConfirmResponse",
            EventType::ShowTable => "ShowTable",
            EventType::ShowKeyValueView => "ShowKeyValueView",
            EventType::ShowForm => "ShowForm",
            EventType::FormResponse => "FormResponse",
            EventType::ShowWizard => "ShowWizard",
            EventType::WizardCompleted => "WizardCompleted",
            EventType::WizardCancelled => "WizardCancelled",
            EventType::ModeChangeRequested => "ModeChangeRequested",
            EventType::CancelRequested => "CancelRequested",
            EventType::TranscriptCleared => "TranscriptCleared",
            EventType::Splash => "Splash",
            EventType::ShowToast => "ShowToast",
            EventType::MentionQuery => "MentionQuery",
            EventType::AssistantReasoningDelta => "AssistantReasoningDelta",
            EventType::TerminalSuspend => "TerminalSuspend",
            EventType::TerminalResume => "TerminalResume",
            EventType::TerminalSuspended => "TerminalSuspended",
            EventType::ActivityUpdate => "ActivityUpdate",
            EventType::ActivityLog => "ActivityLog",
            EventType::ActivityRemoved => "ActivityRemoved",
            EventType::ActivitySnapshot => "ActivitySnapshot",
            EventType::ActivityBrowserOpen => "ActivityBrowserOpen",
            EventType::ActivityStopRequested => "ActivityStopRequested",
            EventType::ActivityViewChanged => "ActivityViewChanged",
        }
    }

    /// Inverse of `as_str`. Returns `None` for variants this binary
    /// doesn't know about — callers (e.g. the NDJSON decoder) use this
    /// to skip forward-shipped events instead of failing the whole
    /// stream.
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "SessionStarted" => Self::SessionStarted,
            "SessionEnded" => Self::SessionEnded,
            "UserMessageCreated" => Self::UserMessageCreated,
            "AssistantStreamStarted" => Self::AssistantStreamStarted,
            "AssistantTokenDelta" => Self::AssistantTokenDelta,
            "AssistantMessageCompleted" => Self::AssistantMessageCompleted,
            "ToolExecutionStarted" => Self::ToolExecutionStarted,
            "ToolExecutionStdout" => Self::ToolExecutionStdout,
            "ToolExecutionStderr" => Self::ToolExecutionStderr,
            "ToolExecutionFinished" => Self::ToolExecutionFinished,
            "PatchProposed" => Self::PatchProposed,
            "PatchApplied" => Self::PatchApplied,
            "PermissionRequested" => Self::PermissionRequested,
            "PermissionGranted" => Self::PermissionGranted,
            "PermissionDenied" => Self::PermissionDenied,
            "RuntimeError" => Self::RuntimeError,
            "SessionCompacted" => Self::SessionCompacted,
            "ViewportMarker" => Self::ViewportMarker,
            "StatusUpdate" => Self::StatusUpdate,
            "BackgroundTaskUpdate" => Self::BackgroundTaskUpdate,
            "TodoListUpdate" => Self::TodoListUpdate,
            "UserInputSubmitted" => Self::UserInputSubmitted,
            "PermissionResponse" => Self::PermissionResponse,
            "SlashCommandsRegistered" => Self::SlashCommandsRegistered,
            "MentionCandidatesRegistered" => Self::MentionCandidatesRegistered,
            "ShowSelectList" => Self::ShowSelectList,
            "SelectListResponse" => Self::SelectListResponse,
            "ShowConfirm" => Self::ShowConfirm,
            "ConfirmResponse" => Self::ConfirmResponse,
            "ShowTable" => Self::ShowTable,
            "ShowKeyValueView" => Self::ShowKeyValueView,
            "ShowForm" => Self::ShowForm,
            "FormResponse" => Self::FormResponse,
            "ShowWizard" => Self::ShowWizard,
            "WizardCompleted" => Self::WizardCompleted,
            "WizardCancelled" => Self::WizardCancelled,
            "ModeChangeRequested" => Self::ModeChangeRequested,
            "CancelRequested" => Self::CancelRequested,
            "TranscriptCleared" => Self::TranscriptCleared,
            "Splash" => Self::Splash,
            "ShowToast" => Self::ShowToast,
            "MentionQuery" => Self::MentionQuery,
            "AssistantReasoningDelta" => Self::AssistantReasoningDelta,
            "TerminalSuspend" => Self::TerminalSuspend,
            "TerminalResume" => Self::TerminalResume,
            "TerminalSuspended" => Self::TerminalSuspended,
            "ActivityUpdate" => Self::ActivityUpdate,
            "ActivityLog" => Self::ActivityLog,
            "ActivityRemoved" => Self::ActivityRemoved,
            "ActivitySnapshot" => Self::ActivitySnapshot,
            "ActivityBrowserOpen" => Self::ActivityBrowserOpen,
            "ActivityStopRequested" => Self::ActivityStopRequested,
            "ActivityViewChanged" => Self::ActivityViewChanged,
            _ => return None,
        })
    }

    /// Direction this event flows on the wire.
    pub fn direction(&self) -> Direction {
        match self {
            EventType::UserInputSubmitted
            | EventType::PermissionResponse
            | EventType::SelectListResponse
            | EventType::ConfirmResponse
            | EventType::FormResponse
            | EventType::WizardCompleted
            | EventType::WizardCancelled
            | EventType::ModeChangeRequested
            | EventType::CancelRequested
            | EventType::MentionQuery
            | EventType::TerminalSuspended
            | EventType::ActivityStopRequested
            | EventType::ActivityViewChanged => Direction::Outbound,
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
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct UserMessage {
        pub text: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct AssistantStreamStarted {
        pub stream_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct AssistantTokenDelta {
        pub stream_id: String,
        pub token: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct AssistantMessageCompleted {
        pub stream_id: String,
        /// Final text; replaces what was streamed (e.g. after cleanup).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub text: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ToolStarted {
        pub tool_id: String,
        /// Tool name, e.g. `Read` or `Bash`.
        pub tool: String,
        /// Arguments shown after the name, e.g. a path or a command line.
        pub command: String,
        /// When the tool started (epoch ms). Defaults to when the event arrives.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub started_at_ms: Option<i64>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ToolOutput {
        pub tool_id: String,
        pub chunk: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ToolFinished {
        pub tool_id: String,
        pub exit_code: i32,
        /// Overrides the status derived from `exit_code`: `done`, `error`,
        /// `cancelled` or `rejected`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub status: Option<String>,
        /// One-line result, e.g. `142 lines` (shown with Ctrl+O, and for errors).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub summary: Option<String>,
        /// Full output; replaces anything streamed via ToolExecutionStdout.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub output: Option<String>,
        /// Output lines shown collapsed. Default 0 (12 on error).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub preview: Option<u32>,
        /// Highlight the output as this language (e.g. `ts`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub output_lang: Option<String>,
        /// A diff to show under the tool row.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub diff: Option<DiffPayload>,
    }

    /// A diff to show inline: full before/after text, or a unified diff.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct DiffPayload {
        pub path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub before: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub unified: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct PatchProposed {
        pub path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub added: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub removed: Option<u32>,
        /// Unified-diff text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub diff: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct PatchApplied {
        pub path: String,
    }

    /// Host asks the user to allow a tool call.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct PermissionRequested {
        pub request_id: String,
        pub tool: String,
        /// Prompt title, e.g. `edit src/auth/session.ts`.
        pub action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub detail: Option<String>,
        /// Diff preview for edits.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub diff: Option<DiffPayload>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct PermissionGranted {
        pub request_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct PermissionDenied {
        pub request_id: String,
    }

    /// Optional welcome and branding for the inline renderer.
    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct SessionStarted {
        /// First welcome line, in the accent color. Defaults to the app title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub title: Option<String>,
        /// Dim lines under the title (model, directory, hints).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub detail: Option<Vec<String>>,
        /// A terminal color name (`orange`, `blue`, …) or `#rrggbb`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub accent: Option<String>,
        /// The agent's name, used in prompts like "tell <name> what to do".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub assistant_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub user_label: Option<String>,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct SessionCompacted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub old_seq: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub new_seq: Option<i64>,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ViewportMarker {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
    }

    /// Multi-line text (may contain ANSI colors) printed as-is, e.g. a logo.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct Splash {
        pub text: String,
    }

    /// v2.4+ — `TerminalSuspend` / `TerminalResume` payload.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct TerminalHandoff {
        pub id: String,
    }

    /// v2.4+ — one step of an activity item (agents' plans).
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ActivityStep {
        pub title: String,
        /// `pending`, `running`, `done` or `failed`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub status: Option<String>,
    }

    /// v2.4+ — a background job or agent the host owns (`ActivityUpdate`,
    /// and each entry of `ActivitySnapshot`).
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ActivityItem {
        /// Stable id; later updates, logs and removals refer to it.
        pub id: String,
        /// `job` or `agent`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub kind: Option<String>,
        /// The command, task or agent name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub title: Option<String>,
        /// `running`, `waiting`, `needs_attention`, `done`, `failed` or `stopped`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub status: Option<String>,
        /// Whether the user may ask to stop it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub stoppable: Option<bool>,
        /// One short line: current step, exit code, what it's waiting for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub summary: Option<String>,
        /// 0.0..=1.0 when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub progress: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub started_at_ms: Option<i64>,
        /// Last change; for finished items, when they finished.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub updated_at_ms: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub steps: Option<Vec<ActivityStep>>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ActivityLog {
        pub id: String,
        /// Output text; may span lines or end mid-line. ANSI is stripped.
        pub chunk: String,
        /// `stdout` or `stderr`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub stream: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ActivityRemoved {
        pub id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ActivitySnapshot {
        pub items: Vec<ActivityItem>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ActivityStopRequested {
        pub id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ActivityViewChanged {
        /// `list`, `detail` or `closed`.
        pub view: String,
        /// The item, for `detail`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub id: Option<String>,
    }

    /// v2.4+ — renderer → host answer to `TerminalSuspend`.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct TerminalSuspended {
        pub id: String,
        /// False when this renderer can't lend the terminal (Windows,
        /// full-screen mode); the host runs the child without one.
        pub supported: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
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
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(rename_all = "snake_case")]
    pub enum RuntimeErrorKind {
        Generic,
        ApiError,
        ServiceEnded,
        QuotaExhausted,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(rename_all = "snake_case")]
    pub enum Severity {
        Info,
        Warn,
        Error,
        Fatal,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
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
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct StatusUpdate {
        pub segments: std::collections::BTreeMap<String, String>,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(rename_all = "snake_case")]
    pub enum BackgroundTaskState {
        Running,
        Done,
        Error,
    }

    /// v0.1.5+ — background task lifecycle (skill indexing, memory load…).
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct BackgroundTaskUpdate {
        pub task_id: String,
        pub label: String,
        pub state: BackgroundTaskState,
        /// 0.0..=1.0 if known, else None.
        #[serde(default)]
        pub progress: Option<f32>,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(rename_all = "snake_case")]
    pub enum TodoStatus {
        Pending,
        InProgress,
        Completed,
    }

    /// One item in a `TodoListUpdate`.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct TodoItem {
        pub id: String,
        pub title: String,
        pub status: TodoStatus,
        /// v0.5.1+ — when the task began (epoch ms). Used for elapsed-time ticker.
        #[serde(default)]
        pub started_at_ms: Option<i64>,
        /// v0.5.1+ — tokens consumed on completion. Shown as `· 1.2k tok`.
        #[serde(default)]
        pub token_delta: Option<u32>,
        /// v0.5.1+ — 0.0..=1.0 for in-progress tasks. Shown as progress bar/percentage.
        #[serde(default)]
        pub progress: Option<f32>,
    }

    /// v0.5+ — the agent's todo/plan checklist. The host sends the FULL list
    /// on every update (last-write-wins); an empty `todos` clears the panel.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct TodoListUpdate {
        pub todos: Vec<TodoItem>,
    }

    /// v0.1.5+ — renderer → host: user submitted text from the input box.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct UserInputSubmitted {
        pub text: String,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(rename_all = "snake_case")]
    pub enum PermissionChoice {
        AllowOnce,
        AllowSession,
        Deny,
    }

    /// v0.1.5+ — renderer → host: response to a PermissionRequested event.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct PermissionResponse {
        /// Matches `request_id` from the original PermissionRequested payload.
        pub request_id: String,
        pub choice: PermissionChoice,
        #[serde(default)]
        pub feedback: Option<String>,
    }

    /// v0.4.5+ — one entry in a SlashCommandsRegistered payload.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
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
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct SlashCommandsRegistered {
        pub commands: Vec<SlashCommand>,
    }

    /// v0.4.5+ — one entry in a MentionCandidatesRegistered payload.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
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
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct MentionCandidatesRegistered {
        pub candidates: Vec<MentionCandidate>,
        /// v2.3+ — set when answering a `MentionQuery`: the directory part
        /// these entries list (e.g. `../`). Without it, the candidates
        /// replace the default list.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub for_query: Option<String>,
    }

    /// v2.3+ — renderer → host: the text after `@` while the user types a
    /// path mention.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct MentionQuery {
        pub query: String,
    }

    /// v0.4.6+ (CC-1) — one option in a `ShowSelectList`.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct SelectListOption {
        /// Stable opaque token returned to the host in `SelectListResponse`.
        /// Hosts typically use a session id, file path, command name, etc.
        pub value: String,
        /// Human-readable label shown in the picker.
        pub label: String,
        /// Optional one-line description shown dimmed to the right of `label`.
        #[serde(default)]
        pub description: Option<String>,
        /// v2.3+ — group header shown above the first option of each group.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub section: Option<String>,
        /// v2.3+ — values drawn in aligned columns after the label.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub columns: Option<Vec<String>>,
        /// v2.3+ — `on` / `off` badge, e.g. for enabled skills.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub state: Option<String>,
        /// v2.3+ — extra text the search matches against.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub keywords: Option<String>,
    }

    /// v0.4.6+ (CC-1) — host asks the renderer to show a modal select list.
    /// See `docs/historical/components-catalog.md` for the full design.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ShowSelectList {
        /// Host-chosen unique id. Reported back in `SelectListResponse` so
        /// the host can correlate the response with the request. Multiple
        /// SelectLists can stack — each has its own id.
        pub id: String,
        /// Prompt shown above the option list (e.g. "Resume which session?").
        pub prompt: String,
        /// v2.3+ — a dim line under the prompt.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub subtitle: Option<String>,
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
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct SelectListResponse {
        pub id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub value: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub cancelled: bool,
    }

    /// v0.4.6+ (CC-2) — host asks the renderer to show a Yes/No modal.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
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
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ConfirmResponse {
        pub id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub value: Option<bool>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub cancelled: bool,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(rename_all = "snake_case")]
    pub enum TableAlign {
        Left,
        Right,
        Center,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct TableColumn {
        /// Key into each row object.
        pub name: String,
        /// Display label shown in the header. Defaults to `name`.
        #[serde(default)]
        pub label: Option<String>,
        /// Cell alignment. Defaults to left.
        #[serde(default)]
        pub align: Option<TableAlign>,
    }

    /// v0.4.6+ (CC-6) — show tabular data in a modal. Display-only in
    /// this version. Rows are arbitrary JSON objects; each column reads
    /// `row[name]` and stringifies it.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ShowTable {
        pub id: String,
        #[serde(default)]
        pub title: Option<String>,
        pub columns: Vec<TableColumn>,
        pub rows: Vec<serde_json::Value>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct KeyValueItem {
        pub label: String,
        pub value: String,
    }

    /// v0.4.6+ (CC-7) — show a label/value list as a modal. Display-only.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ShowKeyValueView {
        pub id: String,
        #[serde(default)]
        pub title: Option<String>,
        pub items: Vec<KeyValueItem>,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(rename_all = "snake_case")]
    pub enum FormFieldKind {
        Text,
        Password,
        /// v2.4+ — multi-line text: Enter adds a line, Tab moves on.
        Multiline,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct FormField {
        pub name: String,
        pub label: String,
        #[serde(default)]
        pub kind: Option<FormFieldKind>,
        #[serde(default)]
        pub default: Option<String>,
        #[serde(default)]
        pub placeholder: Option<String>,
        #[serde(default)]
        pub required: bool,
    }

    /// v0.4.6+ (CC-5) — show a multi-field form modal.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ShowForm {
        pub id: String,
        #[serde(default)]
        pub title: Option<String>,
        pub fields: Vec<FormField>,
        #[serde(default = "default_true")]
        pub allow_cancel: bool,
    }

    /// v0.4.6+ (CC-5) — outbound result of `ShowForm`.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct FormResponse {
        pub id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub values: Option<std::collections::BTreeMap<String, String>>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub cancelled: bool,
    }

    /// v0.4.6+ (CC-4) — one step of a Wizard. Tagged union on `kind`.
    /// Each variant carries the per-step payload that the renderer uses
    /// to install the appropriate sub-modal (SelectList / Confirm / Form).
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum WizardStep {
        Select(WizardSelectStep),
        Confirm(WizardConfirmStep),
        Form(WizardFormStep),
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct WizardSelectStep {
        pub id: String,
        pub prompt: String,
        pub options: Vec<SelectListOption>,
        #[serde(default)]
        pub default: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct WizardConfirmStep {
        pub id: String,
        pub prompt: String,
        #[serde(default)]
        pub yes_label: Option<String>,
        #[serde(default)]
        pub no_label: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct WizardFormStep {
        pub id: String,
        #[serde(default)]
        pub title: Option<String>,
        pub fields: Vec<FormField>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ShowWizard {
        pub id: String,
        #[serde(default)]
        pub title: Option<String>,
        pub steps: Vec<WizardStep>,
        #[serde(default = "default_true")]
        pub allow_cancel: bool,
    }

    /// v0.4.6+ (CC-4) — per-step result value carried back in
    /// `WizardCompleted.results`. String for select; bool for confirm;
    /// JSON object for form.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(untagged)]
    pub enum WizardStepResult {
        Select(String),
        Confirm(bool),
        Form(std::collections::BTreeMap<String, String>),
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct WizardCompleted {
        pub id: String,
        /// Step id → result for each completed step.
        pub results: std::collections::BTreeMap<String, WizardStepResult>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct WizardCancelled {
        pub id: String,
        /// 0-based index of the step the user was on when they cancelled.
        pub at_step: usize,
    }

    /// v0.4.7+ — outbound. User pressed Tab (direction=next) or
    /// Shift+Tab (direction=prev) on an empty input. The host decides
    /// what that means and updates the `mode` status segment to reflect
    /// the new mode.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ModeChangeRequested {
        pub direction: ModeChangeDirection,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(rename_all = "snake_case")]
    pub enum ModeChangeDirection {
        Next,
        Prev,
    }

    /// v0.5+ — severity/visual treatment for a `ShowToast`. Distinct from
    /// `Severity` (used by `RuntimeError`) because toasts add a `Success`
    /// affordance and omit `Fatal`.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    #[serde(rename_all = "snake_case")]
    pub enum ToastKind {
        Info,
        Warn,
        Error,
        Success,
    }

    /// v0.5+ — host asks the renderer to show a transient toast. The toast
    /// auto-dismisses after `ttl_ms` (renderer applies a default when
    /// omitted) and never enters the transcript.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[cfg_attr(test, derive(schemars::JsonSchema))]
    pub struct ShowToast {
        pub text: String,
        /// Visual treatment. Defaults to `Info` when omitted.
        #[serde(default)]
        pub kind: Option<ToastKind>,
        /// Auto-dismiss delay in milliseconds. Renderer applies a default
        /// (and clamps to a sane ceiling) when omitted.
        #[serde(default)]
        pub ttl_ms: Option<u64>,
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
            EventType::TodoListUpdate,
            EventType::UserInputSubmitted,
            EventType::PermissionResponse,
            EventType::SlashCommandsRegistered,
            EventType::MentionCandidatesRegistered,
            EventType::ShowSelectList,
            EventType::SelectListResponse,
            EventType::ShowConfirm,
            EventType::ConfirmResponse,
            EventType::ShowTable,
            EventType::ShowKeyValueView,
            EventType::ShowForm,
            EventType::FormResponse,
            EventType::ShowWizard,
            EventType::WizardCompleted,
            EventType::WizardCancelled,
            EventType::ModeChangeRequested,
            EventType::CancelRequested,
            EventType::TranscriptCleared,
            EventType::Splash,
            EventType::ShowToast,
            EventType::MentionQuery,
            EventType::AssistantReasoningDelta,
            EventType::TerminalSuspend,
            EventType::TerminalResume,
            EventType::TerminalSuspended,
            EventType::ActivityUpdate,
            EventType::ActivityLog,
            EventType::ActivityRemoved,
            EventType::ActivitySnapshot,
            EventType::ActivityBrowserOpen,
            EventType::ActivityStopRequested,
            EventType::ActivityViewChanged,
        ];
        assert_eq!(types.len(), 53);
        for t in types {
            let ev = sample(t, json!({"k": "v"}));
            let s = serde_json::to_string(&ev).unwrap();
            let back: Event = serde_json::from_str(&s).unwrap();
            assert_eq!(ev, back);
        }
    }

    /// The Node SDK's type lists are hand-written; fail the build when they
    /// fall behind this enum (autopilot once deleted `TranscriptCleared`
    /// calls because the SDK types didn't list it).
    #[test]
    fn node_sdk_knows_every_event_type() {
        let runtime = include_str!("../../../sdk/node/src/types.js");
        let types = include_str!("../../../sdk/node/src/types.d.ts");
        let all = [
            EventType::SessionStarted, EventType::SessionEnded, EventType::UserMessageCreated,
            EventType::AssistantStreamStarted, EventType::AssistantTokenDelta, EventType::AssistantMessageCompleted,
            EventType::ToolExecutionStarted, EventType::ToolExecutionStdout, EventType::ToolExecutionStderr,
            EventType::ToolExecutionFinished, EventType::PatchProposed, EventType::PatchApplied,
            EventType::PermissionRequested, EventType::PermissionGranted, EventType::PermissionDenied,
            EventType::RuntimeError, EventType::SessionCompacted, EventType::ViewportMarker,
            EventType::StatusUpdate, EventType::BackgroundTaskUpdate, EventType::TodoListUpdate,
            EventType::UserInputSubmitted, EventType::PermissionResponse, EventType::SlashCommandsRegistered,
            EventType::MentionCandidatesRegistered, EventType::ShowSelectList, EventType::SelectListResponse,
            EventType::ShowConfirm, EventType::ConfirmResponse, EventType::ShowTable, EventType::ShowKeyValueView,
            EventType::ShowForm, EventType::FormResponse, EventType::ShowWizard, EventType::WizardCompleted,
            EventType::WizardCancelled, EventType::ModeChangeRequested, EventType::CancelRequested,
            EventType::TranscriptCleared, EventType::Splash, EventType::ShowToast,
            EventType::MentionQuery, EventType::AssistantReasoningDelta,
            EventType::TerminalSuspend, EventType::TerminalResume, EventType::TerminalSuspended,
            EventType::ActivityUpdate, EventType::ActivityLog, EventType::ActivityRemoved, EventType::ActivitySnapshot, EventType::ActivityBrowserOpen, EventType::ActivityStopRequested, EventType::ActivityViewChanged,
        ];
        for t in all {
            let quoted = format!("\"{}\"", t.as_str());
            assert!(runtime.contains(&quoted), "sdk/node/src/types.js is missing {quoted}");
            assert!(types.contains(&format!("| {quoted}")), "sdk/node/src/types.d.ts EventType is missing {quoted}");
            assert!(types.contains(&format!("event_type: {quoted}")), "sdk/node/src/types.d.ts Event union is missing {quoted}");
        }
    }

    /// Fields of a Rust payload as the wire sees them: name → optional
    /// (serde `default` / `Option`), from its JSON schema.
    fn rust_fields<T: schemars::JsonSchema>() -> std::collections::BTreeMap<String, bool> {
        let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap();
        let required: Vec<String> = schema["required"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        schema["properties"]
            .as_object()
            .map(|props| props.keys().map(|k| (k.clone(), !required.contains(k))).collect())
            .unwrap_or_default()
    }

    /// Object types in types.d.ts (`export type X = { ... }`): name →
    /// (field → optional). Comments are ignored; nested braces are skipped.
    fn ts_object_types(src: &str) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, bool>> {
        let mut no_comments = String::new();
        let mut rest = src;
        while let Some(i) = rest.find("/*") {
            no_comments.push_str(&rest[..i]);
            rest = rest[i..].find("*/").map(|j| &rest[i + j + 2..]).unwrap_or("");
        }
        no_comments.push_str(rest);
        let src: String = no_comments.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
        let mut out = std::collections::BTreeMap::new();
        let mut at = 0;
        while let Some(i) = src[at..].find("export type ") {
            let start = at + i + "export type ".len();
            at = start;
            let Some(eq) = src[start..].find('=') else { break };
            let name = src[start..start + eq].trim().to_string();
            let after = src[start + eq + 1..].trim_start();
            if !after.starts_with('{') || name.contains('<') {
                continue;
            }
            let open = src.len() - after.len();
            let (mut depth, mut end) = (0i32, open);
            for (k, ch) in src[open..].char_indices() {
                match ch {
                    '{' | '(' | '[' | '<' => depth += 1,
                    '}' | ')' | ']' | '>' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + k;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let body = &src[open + 1..end];
            let mut fields = std::collections::BTreeMap::new();
            let (mut depth, mut piece) = (0i32, String::new());
            for ch in body.chars().chain(std::iter::once(';')) {
                match ch {
                    '{' | '(' | '[' | '<' => depth += 1,
                    '}' | ')' | ']' | '>' => depth -= 1,
                    _ => {}
                }
                if (ch == ';' || ch == '\n') && depth == 0 {
                    let p = piece.trim();
                    if let Some(colon) = p.find(':') {
                        let key = p[..colon].trim();
                        let (key, optional) = match key.strip_suffix('?') {
                            Some(k) => (k.trim(), true),
                            None => (key, false),
                        };
                        if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                            fields.insert(key.to_string(), optional);
                        }
                    }
                    piece.clear();
                } else {
                    piece.push(ch);
                }
            }
            out.insert(name, fields);
            at = end;
        }
        out
    }

    /// The SDK's payload types are hand-written for their docs; Rust is the
    /// source of truth. Every object type in types.d.ts must match its Rust
    /// payload field for field, including which fields are optional.
    #[test]
    fn node_sdk_payload_types_match_rust() {
        use payloads::*;
        let ts = ts_object_types(include_str!("../../../sdk/node/src/types.d.ts"));
        let mut rust: std::collections::BTreeMap<&str, std::collections::BTreeMap<String, bool>> = std::collections::BTreeMap::new();
        macro_rules! payloads {
            ($($t:ident),* $(,)?) => { $( rust.insert(stringify!($t), rust_fields::<$t>()); )* };
        }
        payloads!(
            UserMessage, AssistantStreamStarted, AssistantTokenDelta, AssistantMessageCompleted,
            ToolStarted, ToolOutput, ToolFinished, DiffPayload, PatchProposed, PatchApplied,
            PermissionRequested, PermissionGranted, PermissionDenied, RuntimeError, Cta,
            StatusUpdate, BackgroundTaskUpdate, TodoItem, TodoListUpdate, SessionStarted,
            SessionCompacted, ViewportMarker, UserInputSubmitted, PermissionResponse,
            SlashCommand, SlashCommandsRegistered, MentionCandidate, MentionCandidatesRegistered,
            MentionQuery, SelectListOption, ShowSelectList, SelectListResponse, ShowConfirm,
            ConfirmResponse, TableColumn, ShowTable, KeyValueItem, ShowKeyValueView, FormField,
            ShowForm, FormResponse, ShowWizard, WizardCompleted, WizardCancelled,
            ModeChangeRequested, ShowToast, Splash, TerminalHandoff, TerminalSuspended,
            ActivityStep, ActivityItem, ActivityLog, ActivityRemoved, ActivitySnapshot,
            ActivityStopRequested, ActivityViewChanged,
        );
        let mut problems = Vec::new();
        for (name, fields) in &ts {
            if name == "EnvelopeMeta" {
                continue;
            }
            match rust.get(name.as_str()) {
                None => problems.push(format!("types.d.ts has {name} but no Rust payload of that name")),
                Some(r) if r != fields => problems.push(format!("{name}: Rust {r:?} vs types.d.ts {fields:?}")),
                _ => {}
            }
        }
        for name in rust.keys() {
            if !ts.contains_key(*name) {
                problems.push(format!("Rust payload {name} is missing from types.d.ts"));
            }
        }
        assert!(problems.is_empty(), "SDK types drifted from the protocol:\n{}", problems.join("\n"));
    }

    #[test]
    fn direction_classification() {
        assert_eq!(EventType::SessionStarted.direction(), Direction::Inbound);
        assert_eq!(EventType::StatusUpdate.direction(), Direction::Inbound);
        assert_eq!(EventType::UserInputSubmitted.direction(), Direction::Outbound);
        assert_eq!(EventType::PermissionResponse.direction(), Direction::Outbound);
    }

    #[test]
    fn permission_response_choice_wire_format() {
        // Host-side parsers key off these exact strings. Pin them down so a
        // rename in PermissionChoice can't silently break "allow for the
        // turn" / "allow for the session" semantics across the boundary.
        for (choice, expected) in [
            (payloads::PermissionChoice::AllowOnce, "allow_once"),
            (payloads::PermissionChoice::AllowSession, "allow_session"),
            (payloads::PermissionChoice::Deny, "deny"),
        ] {
            let p = payloads::PermissionResponse {
                request_id: "req-1".into(),
                choice,
                feedback: None,
            };
            let s = serde_json::to_string(&p).unwrap();
            assert!(
                s.contains(&format!("\"choice\":\"{}\"", expected)),
                "expected choice={} in {}", expected, s
            );
            let back: payloads::PermissionResponse = serde_json::from_str(&s).unwrap();
            assert_eq!(p, back);
        }
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
    fn show_toast_payload_roundtrip() {
        // Full payload round-trips, and the kind enum uses snake_case wire
        // strings the host (kimiflare) already emits.
        let p = payloads::ShowToast {
            text: "mode: plan".into(),
            kind: Some(payloads::ToastKind::Warn),
            ttl_ms: Some(3500),
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"kind\":\"warn\""), "got {s}");
        let back: payloads::ShowToast = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);

        // Minimal payload (text only) — kind/ttl_ms optional for hosts that
        // don't care about styling or timing.
        let raw = r#"{"text":"saved"}"#;
        let m: payloads::ShowToast = serde_json::from_str(raw).unwrap();
        assert_eq!(m.text, "saved");
        assert!(m.kind.is_none());
        assert!(m.ttl_ms.is_none());

        for (kind, wire) in [
            (payloads::ToastKind::Info, "info"),
            (payloads::ToastKind::Warn, "warn"),
            (payloads::ToastKind::Error, "error"),
            (payloads::ToastKind::Success, "success"),
        ] {
            let s = serde_json::to_string(&kind).unwrap();
            assert_eq!(s, format!("\"{wire}\""));
        }
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
