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
//! (`SessionStarted`, `SessionEnded`, `PatchProposed`, `PatchApplied`,
//! `PermissionRequested`, `PermissionGranted`, `PermissionDenied`,
//! `SessionCompacted`, `ViewportMarker`) are accepted with any JSON value as
//! payload; the validator only enforces shape where the protocol defines one.

use camouflage_protocol::{payloads, EventType};
use serde::Deserialize;

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
}

#[derive(Debug, Deserialize)]
struct Envelope {
    event_type: EventType,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

/// Validate a single NDJSON line. The `line` index is purely for error
/// messages; pass `1` for the first line and increment as you go.
pub fn validate_line(line: usize, raw: &str) -> Result<EventType, ValidationError> {
    // First pass: parse as raw JSON to detect "not even JSON" cases distinctly
    // from "JSON but missing event_type".
    if let Err(e) = serde_json::from_str::<serde_json::Value>(raw) {
        return Err(ValidationError::NotJson { line, source: e });
    }
    let env: Envelope = serde_json::from_str(raw)
        .map_err(|e| ValidationError::BadEventType { line, source: e })?;
    let payload = env.payload.unwrap_or(serde_json::Value::Null);
    validate_payload(env.event_type, payload).map_err(|e| ValidationError::BadPayload {
        line,
        event_type: env.event_type,
        source: e,
    })?;
    Ok(env.event_type)
}

fn validate_payload(event_type: EventType, payload: serde_json::Value) -> Result<(), serde_json::Error> {
    use EventType::*;
    match event_type {
        UserMessageCreated => {
            serde_json::from_value::<payloads::UserMessage>(payload)?;
        }
        AssistantStreamStarted => {
            serde_json::from_value::<payloads::AssistantStreamStarted>(payload)?;
        }
        AssistantTokenDelta => {
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
        ShowToast => {
            serde_json::from_value::<payloads::ShowToast>(payload)?;
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
        CancelRequested => {
            // No typed payload (CancelRequested is bodyless); accept any value.
        }
        // Events without typed payloads — any JSON value is accepted.
        SessionStarted
        | SessionEnded
        | PatchProposed
        | PatchApplied
        | PermissionRequested
        | PermissionGranted
        | PermissionDenied
        | SessionCompacted
        | TranscriptCleared
        | ViewportMarker
        | Splash => {}
    }
    Ok(())
}

/// Validation summary returned by `validate_stream`.
#[derive(Debug, Default)]
pub struct ValidationReport {
    pub lines_total: usize,
    pub lines_valid: usize,
    pub errors: Vec<ValidationError>,
}

impl ValidationReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Validate every non-blank line in `input`. Errors are collected; the report
/// always includes per-line counts so callers can print a summary.
pub fn validate_stream(input: &str) -> ValidationReport {
    let mut report = ValidationReport::default();
    for (i, raw) in input.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        report.lines_total += 1;
        match validate_line(i + 1, raw) {
            Ok(_) => report.lines_valid += 1,
            Err(e) => report.errors.push(e),
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
        assert!(matches!(err, ValidationError::NotJson { line: 7, .. }), "{err:?}");
    }

    #[test]
    fn rejects_unknown_event_type() {
        let line = r#"{"event_type":"NotARealEvent","payload":{}}"#;
        let err = validate_line(1, line).unwrap_err();
        assert!(matches!(err, ValidationError::BadEventType { .. }), "{err:?}");
    }

    #[test]
    fn rejects_user_message_missing_text() {
        let line = r#"{"event_type":"UserMessageCreated","payload":{"not_text":"x"}}"#;
        let err = validate_line(1, line).unwrap_err();
        assert!(
            matches!(err, ValidationError::BadPayload { event_type: EventType::UserMessageCreated, .. }),
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
        let line = r#"{"event_type":"AssistantTokenDelta","payload":{"stream_id":"s1","token":42}}"#;
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
        assert_eq!(report.errors.len(), 2);
        assert!(matches!(report.errors[0], ValidationError::NotJson { .. }));
        assert!(matches!(report.errors[1], ValidationError::BadPayload { .. }));
    }
}
