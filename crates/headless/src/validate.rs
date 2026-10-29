//! Strict NDJSON validator.
//!
//! Unlike the lenient `run_reader` path (which turns every parse error into a
//! `RuntimeError` event so the renderer keeps moving), this validator is
//! *strict*: any malformed line, unknown `event_type`, or payload that fails to
//! deserialise into the typed struct for its event type is reported as an
//! error. Designed for CI / pre-commit hooks where a regression in a host
//! adapter should fail loudly.
//!
//! Events whose payloads do not have a dedicated typed struct
//! (`SessionEnded`, `TranscriptCleared`, `ActivityBrowserOpen`) are accepted
//! with any JSON value as payload; the validator only enforces shape where
//! the protocol defines one.
//!
//! A stream may carry several sessions at once: events from different
//! `session_id`s interleave, and `seq` is only required to increase *within*
//! a session. `validate_stream` tracks the last `seq` per session and reports
//! a duplicate or backwards `seq` as an error, because the store's
//! uniqueness is `(session_id, seq)`.

use camouflage_protocol::{payloads, EventType};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("line {line}: not valid JSON: {source}")]
    NotJson {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("line {line}: missing or unknown event_type: {source}")]
    BadEventType {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("line {line}: {event_type:?} payload failed to validate: {source}")]
    BadPayload {
        line: usize,
        event_type: EventType,
        #[source]
        source: serde_json::Error,
    },
    #[error("line {line}: session {session_id} seq {seq} is not after {previous}")]
    SeqOutOfOrder {
        line: usize,
        session_id: String,
        seq: i64,
        previous: i64,
    },
}

#[derive(Debug, Deserialize)]
struct Envelope {
    event_type: EventType,
    #[serde(default)]
    payload: Option<serde_json::Value>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    seq: Option<i64>,
}

/// What a validated line says about the stream, beyond its event type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMeta {
    pub event_type: EventType,
    /// The session the line belongs to, when the host set one.
    pub session_id: Option<String>,
    /// The host's sequence number, when it set one.
    pub seq: Option<i64>,
}

/// Validate a single NDJSON line. The `line` index is purely for error
/// messages; pass `1` for the first line and increment as you go.
pub fn validate_line(line: usize, raw: &str) -> Result<EventType, ValidationError> {
    validate_line_meta(line, raw).map(|meta| meta.event_type)
}

/// Like [`validate_line`], but also returns the envelope fields a
/// multi-session stream needs (`session_id`, `seq`).
pub fn validate_line_meta(line: usize, raw: &str) -> Result<LineMeta, ValidationError> {
    // First pass: parse as raw JSON to detect "not even JSON" cases distinctly
    // from "JSON but missing event_type".
    if let Err(e) = serde_json::from_str::<serde_json::Value>(raw) {
        return Err(ValidationError::NotJson { line, source: e });
    }
    let env: Envelope =
        serde_json::from_str(raw).map_err(|e| ValidationError::BadEventType { line, source: e })?;
    let payload = env.payload.unwrap_or(serde_json::Value::Null);
    validate_payload(env.event_type, payload).map_err(|e| ValidationError::BadPayload {
        line,
        event_type: env.event_type,
        source: e,
    })?;
    Ok(LineMeta {
        event_type: env.event_type,
        session_id: env.session_id,
        seq: env.seq,
    })
}

fn or_empty(v: serde_json::Value) -> serde_json::Value {
    if v.is_null() {
        serde_json::json!({})
    } else {
        v
    }
}

pub(crate) fn validate_payload(
    event_type: EventType,
    payload: serde_json::Value,
) -> Result<(), serde_json::Error> {
    use EventType::*;
    match event_type {
        UserMessageCreated => {
            serde_json::from_value::<payloads::UserMessage>(payload)?;
        }
        AssistantStreamStarted => {
            serde_json::from_value::<payloads::AssistantStreamStarted>(payload)?;
        }
        AssistantTokenDelta | AssistantReasoningDelta => {
            serde_json::from_value::<payloads::AssistantTokenDelta>(payload)?;
        }
        AssistantMessageCompleted => {
            serde_json::from_value::<payloads::AssistantMessageCompleted>(payload)?;
        }
        ToolExecutionStarted => {
            serde_json::from_value::<payloads::ToolStarted>(payload)?;
        }
        ToolExecutionStdout | ToolExecutionStderr => {
            serde_json::from_value::<payloads::ToolOutput>(payload)?;
        }
        ToolExecutionFinished => {
            serde_json::from_value::<payloads::ToolFinished>(payload)?;
        }
        RuntimeError => {
            serde_json::from_value::<payloads::RuntimeError>(payload)?;
        }
        StatusUpdate => {
            serde_json::from_value::<payloads::StatusUpdate>(payload)?;
        }
        BackgroundTaskUpdate => {
            serde_json::from_value::<payloads::BackgroundTaskUpdate>(payload)?;
        }
        TodoListUpdate => {
            serde_json::from_value::<payloads::TodoListUpdate>(payload)?;
        }
        UserInputSubmitted => {
            serde_json::from_value::<payloads::UserInputSubmitted>(payload)?;
        }
        PermissionResponse => {
            serde_json::from_value::<payloads::PermissionResponse>(payload)?;
        }
        SlashCommandsRegistered => {
            serde_json::from_value::<payloads::SlashCommandsRegistered>(payload)?;
        }
        MentionCandidatesRegistered => {
            serde_json::from_value::<payloads::MentionCandidatesRegistered>(payload)?;
        }
        ShowSelectList => {
            serde_json::from_value::<payloads::ShowSelectList>(payload)?;
        }
        SelectListResponse => {
            serde_json::from_value::<payloads::SelectListResponse>(payload)?;
        }
        ShowConfirm => {
            serde_json::from_value::<payloads::ShowConfirm>(payload)?;
        }
        ConfirmResponse => {
            serde_json::from_value::<payloads::ConfirmResponse>(payload)?;
        }
        ShowTable => {
            serde_json::from_value::<payloads::ShowTable>(payload)?;
        }
        ShowKeyValueView => {
            serde_json::from_value::<payloads::ShowKeyValueView>(payload)?;
        }
        ShowForm => {
            serde_json::from_value::<payloads::ShowForm>(payload)?;
        }
        FormResponse => {
            serde_json::from_value::<payloads::FormResponse>(payload)?;
        }
        ShowWizard => {
            serde_json::from_value::<payloads::ShowWizard>(payload)?;
        }
        WizardCompleted => {
            serde_json::from_value::<payloads::WizardCompleted>(payload)?;
        }
        WizardCancelled => {
            serde_json::from_value::<payloads::WizardCancelled>(payload)?;
        }
        ModeChangeRequested => {
            serde_json::from_value::<payloads::ModeChangeRequested>(payload)?;
        }
        ShowToast => {
            serde_json::from_value::<payloads::ShowToast>(payload)?;
        }
        CancelRequested => {
            // No typed payload (CancelRequested is bodyless); accept any value.
        }
        MentionQuery => {
            serde_json::from_value::<payloads::MentionQuery>(payload)?;
        }
        TerminalSuspend | TerminalResume => {
            serde_json::from_value::<payloads::TerminalHandoff>(payload)?;
        }
        TerminalSuspended => {
            serde_json::from_value::<payloads::TerminalSuspended>(payload)?;
        }
        PatchProposed => {
            serde_json::from_value::<payloads::PatchProposed>(payload)?;
        }
        PatchApplied => {
            serde_json::from_value::<payloads::PatchApplied>(payload)?;
        }
        PermissionRequested => {
            serde_json::from_value::<payloads::PermissionRequested>(payload)?;
        }
        PermissionGranted => {
            serde_json::from_value::<payloads::PermissionGranted>(payload)?;
        }
        PermissionDenied => {
            serde_json::from_value::<payloads::PermissionDenied>(payload)?;
        }
        Splash => {
            serde_json::from_value::<payloads::Splash>(payload)?;
        }
        // All-optional payloads: a missing payload is the same as `{}`.
        SessionStarted => {
            serde_json::from_value::<payloads::SessionStarted>(or_empty(payload))?;
        }
        SessionCompacted => {
            serde_json::from_value::<payloads::SessionCompacted>(or_empty(payload))?;
        }
        ViewportMarker => {
            serde_json::from_value::<payloads::ViewportMarker>(or_empty(payload))?;
        }
        ActivityUpdate => {
            serde_json::from_value::<payloads::ActivityItem>(payload)?;
        }
        ActivityLog => {
            serde_json::from_value::<payloads::ActivityLog>(payload)?;
        }
        ActivityRemoved => {
            serde_json::from_value::<payloads::ActivityRemoved>(payload)?;
        }
        ActivitySnapshot => {
            serde_json::from_value::<payloads::ActivitySnapshot>(payload)?;
        }
        ActivityStopRequested => {
            serde_json::from_value::<payloads::ActivityStopRequested>(payload)?;
        }
        ActivityViewChanged => {
            serde_json::from_value::<payloads::ActivityViewChanged>(payload)?;
        }
        SessionStatus => {
            serde_json::from_value::<payloads::SessionStatus>(payload)?;
        }
        // Bodyless events — any JSON value is accepted.
        SessionEnded | TranscriptCleared | ActivityBrowserOpen => {}
    }
    Ok(())
}

/// Validation summary returned by `validate_stream`.
#[derive(Debug, Default)]
pub struct ValidationReport {
    pub lines_total: usize,
    pub lines_valid: usize,
    /// Distinct `session_id`s seen in the stream.
    pub sessions: usize,
    pub errors: Vec<ValidationError>,
}

impl ValidationReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Validate every non-blank line in `input`. Errors are collected; the report
/// always includes per-line counts so callers can print a summary.
///
/// Sessions may interleave: `seq` is checked per `session_id`, never across
/// the whole stream.
pub fn validate_stream(input: &str) -> ValidationReport {
    let mut report = ValidationReport::default();
    let mut last_seq: HashMap<String, i64> = HashMap::new();
    for (i, raw) in input.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let line = i + 1;
        report.lines_total += 1;
        let meta = match validate_line_meta(line, raw) {
            Ok(meta) => meta,
            Err(e) => {
                report.errors.push(e);
                continue;
            }
        };
        if let Some(sid) = &meta.session_id {
            if !last_seq.contains_key(sid) {
                report.sessions += 1;
            }
        }
        let mut valid = true;
        if let (Some(sid), Some(seq)) = (&meta.session_id, meta.seq) {
            if let Some(&previous) = last_seq.get(sid) {
                if seq <= previous {
                    report.errors.push(ValidationError::SeqOutOfOrder {
                        line,
                        session_id: sid.clone(),
                        seq,
                        previous,
                    });
                    valid = false;
                }
            }
            if valid {
                last_seq.insert(sid.clone(), seq);
            }
        }
        if valid {
            report.lines_valid += 1;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_well_formed_user_message() {
        let line = r#"{"event_type":"UserMessageCreated","payload":{"text":"hi"}}"#;
        let kind = validate_line(1, line).unwrap();
        assert_eq!(kind, EventType::UserMessageCreated);
    }

    #[test]
    fn accepts_session_started_with_empty_payload() {
        let line = r#"{"event_type":"SessionStarted","payload":{}}"#;
        validate_line(1, line).unwrap();
    }

    #[test]
    fn accepts_status_update_with_segments() {
        let line = r#"{"event_type":"StatusUpdate","payload":{"segments":{"mode":"edit","phase":"thinking"}}}"#;
        validate_line(1, line).unwrap();
    }

    #[test]
    fn rejects_not_json() {
        let err = validate_line(7, "this is not json").unwrap_err();
        assert!(
            matches!(err, ValidationError::NotJson { line: 7, .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_unknown_event_type() {
        let line = r#"{"event_type":"NotARealEvent","payload":{}}"#;
        let err = validate_line(1, line).unwrap_err();
        assert!(
            matches!(err, ValidationError::BadEventType { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_user_message_missing_text() {
        let line = r#"{"event_type":"UserMessageCreated","payload":{"not_text":"x"}}"#;
        let err = validate_line(1, line).unwrap_err();
        assert!(
            matches!(
                err,
                ValidationError::BadPayload {
                    event_type: EventType::UserMessageCreated,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_tool_finished_missing_exit_code() {
        let line = r#"{"event_type":"ToolExecutionFinished","payload":{"tool_id":"t1"}}"#;
        let err = validate_line(1, line).unwrap_err();
        assert!(matches!(err, ValidationError::BadPayload { .. }), "{err:?}");
    }

    #[test]
    fn rejects_token_delta_wrong_type() {
        let line =
            r#"{"event_type":"AssistantTokenDelta","payload":{"stream_id":"s1","token":42}}"#;
        let err = validate_line(1, line).unwrap_err();
        assert!(matches!(err, ValidationError::BadPayload { .. }), "{err:?}");
    }

    #[test]
    fn stream_report_counts_and_collects() {
        let input = "\
{\"event_type\":\"SessionStarted\"}
not json
{\"event_type\":\"UserMessageCreated\",\"payload\":{\"text\":\"hi\"}}

{\"event_type\":\"UserMessageCreated\",\"payload\":{}}
{\"event_type\":\"SessionEnded\"}
";
        let report = validate_stream(input);
        assert_eq!(report.lines_total, 5);
        assert_eq!(report.lines_valid, 3);
        assert_eq!(report.sessions, 0);
        assert_eq!(report.errors.len(), 2);
        assert!(matches!(report.errors[0], ValidationError::NotJson { .. }));
        assert!(matches!(
            report.errors[1],
            ValidationError::BadPayload { .. }
        ));
    }

    #[test]
    fn accepts_session_status() {
        let line = r#"{"event_type":"SessionStatus","session_id":"a","payload":{"state":"blocked","title":"camouflage","detail":"waiting for approval","cost_usd":0.42,"context_used":48000,"context_limit":200000,"pr_url":"https://example.com/pr/1","updated_ms":1747521837000}}"#;
        let meta = validate_line_meta(1, line).unwrap();
        assert_eq!(meta.event_type, EventType::SessionStatus);
        assert_eq!(meta.session_id.as_deref(), Some("a"));
    }

    #[test]
    fn rejects_session_status_with_unknown_state() {
        let line = r#"{"event_type":"SessionStatus","payload":{"state":"sleeping"}}"#;
        let err = validate_line(1, line).unwrap_err();
        assert!(
            matches!(
                err,
                ValidationError::BadPayload {
                    event_type: EventType::SessionStatus,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_session_status_without_state() {
        let line = r#"{"event_type":"SessionStatus","payload":{"title":"camouflage"}}"#;
        let err = validate_line(1, line).unwrap_err();
        assert!(matches!(err, ValidationError::BadPayload { .. }), "{err:?}");
    }

    #[test]
    fn interleaved_sessions_keep_per_session_seq() {
        // Two sessions, each with its own 0,1,2 sequence, interleaved in one
        // stream. Global ordering would flag this; per-session ordering must
        // not.
        let input = "\
{\"event_type\":\"SessionStarted\",\"session_id\":\"a\",\"seq\":0}
{\"event_type\":\"SessionStarted\",\"session_id\":\"b\",\"seq\":0}
{\"event_type\":\"SessionStatus\",\"session_id\":\"a\",\"seq\":1,\"payload\":{\"state\":\"working\"}}
{\"event_type\":\"SessionStatus\",\"session_id\":\"b\",\"seq\":1,\"payload\":{\"state\":\"blocked\"}}
{\"event_type\":\"SessionEnded\",\"session_id\":\"a\",\"seq\":2}
{\"event_type\":\"SessionEnded\",\"session_id\":\"b\",\"seq\":2}
";
        let report = validate_stream(input);
        assert!(report.ok(), "errors: {:#?}", report.errors);
        assert_eq!(report.lines_total, 6);
        assert_eq!(report.lines_valid, 6);
        assert_eq!(report.sessions, 2);
    }

    #[test]
    fn rejects_duplicate_seq_within_one_session() {
        let input = "\
{\"event_type\":\"SessionStarted\",\"session_id\":\"a\",\"seq\":0}
{\"event_type\":\"SessionEnded\",\"session_id\":\"a\",\"seq\":0}
";
        let report = validate_stream(input);
        assert_eq!(report.lines_valid, 1);
        assert!(matches!(
            report.errors[0],
            ValidationError::SeqOutOfOrder {
                line: 2,
                seq: 0,
                previous: 0,
                ..
            }
        ));
    }

    #[test]
    fn multi_session_fixture_validates() {
        let body = crate::fixtures::load_fixture("multi-session").expect("fixture exists");
        let report = validate_stream(&body);
        assert!(report.ok(), "errors: {:#?}", report.errors);
        assert_eq!(report.sessions, 2);
    }
}
