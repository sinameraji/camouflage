//! Event-fixture helpers.
//!
//! The workspace ships golden NDJSON files in `<repo>/fixtures/` (see the
//! workspace README). They serve three purposes:
//!
//! 1. Regression catch — `validate_all_fixtures()` is wired into CI as a unit
//!    test that ensures every checked-in stream still validates against the
//!    current protocol. If you change a payload struct in a non-additive way,
//!    a fixture will fail to validate and you'll know immediately.
//! 2. Cross-crate reuse — any test (store, renderer, tui) can call
//!    `load_fixture("host-mock")` to get a deterministic event stream
//!    instead of hand-rolling NDJSON inside the test source.
//! 3. Adapter contract — when a host adapter outputs a known
//!    fixture for a known input, drift in its output is caught by a simple
//!    diff against the checked-in fixture.
//!
//! Fixture paths are resolved relative to `CARGO_MANIFEST_DIR/../../fixtures`,
//! which works for every crate in the workspace.

use crate::validate::{validate_stream, ValidationReport};
use std::path::{Path, PathBuf};

/// Absolute path to the workspace `fixtures/` directory.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

/// Read a fixture by stem (e.g. `"host-mock"` → `fixtures/host-mock.ndjson`).
pub fn load_fixture(name: &str) -> std::io::Result<String> {
    let path = fixtures_dir().join(format!("{name}.ndjson"));
    std::fs::read_to_string(path)
}

/// Enumerate every `*.ndjson` fixture in the directory, in sorted order.
pub fn list_fixtures() -> std::io::Result<Vec<PathBuf>> {
    let mut out: Vec<_> = std::fs::read_dir(fixtures_dir())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("ndjson"))
        .collect();
    out.sort();
    Ok(out)
}

/// Validate every fixture and return one report per file. Fails on I/O error
/// but not on validation errors — the caller decides whether to assert on them.
pub fn validate_all_fixtures() -> std::io::Result<Vec<(PathBuf, ValidationReport)>> {
    list_fixtures()?
        .into_iter()
        .map(|p| {
            let body = std::fs::read_to_string(&p)?;
            Ok((p, validate_stream(&body)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression gate: every fixture must validate cleanly against the
    /// current protocol. If this fails, either the fixture is stale (re-capture
    /// it) or a payload struct changed in a breaking way (consider a non-
    /// additive change carefully).
    #[test]
    fn all_fixtures_validate() {
        let reports = validate_all_fixtures().expect("fixtures dir readable");
        assert!(!reports.is_empty(), "no fixtures found in {:?}", fixtures_dir());
        for (path, report) in &reports {
            assert!(
                report.ok(),
                "fixture {:?} failed validation:\n  {} of {} lines valid\n  errors: {:#?}",
                path,
                report.lines_valid,
                report.lines_total,
                report.errors
            );
        }
    }

    #[test]
    fn load_fixture_by_name() {
        let body = load_fixture("host-mock").expect("fixture exists");
        assert!(body.contains("\"event_type\""), "fixture looks empty");
    }

    #[test]
    fn multi_session_fixture_interleaves_two_sessions() {
        use camouflage_protocol::EventType;
        use std::collections::HashMap;

        let body = load_fixture("multi-session").expect("fixture exists");
        let report = validate_stream(&body);
        assert!(report.ok(), "{:#?}", report.errors);

        let mut sessions: HashMap<String, Vec<i64>> = HashMap::new();
        let mut statuses = 0;
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            let sid = v["session_id"].as_str().unwrap().to_string();
            sessions.entry(sid).or_default().push(v["seq"].as_i64().unwrap());
            if v["event_type"] == "SessionStatus" {
                statuses += 1;
            }
        }
        assert_eq!(sessions.len(), 2, "fixture should carry two sessions");
        assert!(statuses >= 2, "fixture should carry SessionStatus events");
        for (sid, seqs) in &sessions {
            let mut sorted = seqs.clone();
            sorted.sort_unstable();
            assert_eq!(seqs, &sorted, "session {sid} seqs are not increasing");
        }
        // The two sessions' events are genuinely interleaved, not blocks.
        let order: Vec<String> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v["session_id"].as_str().unwrap().to_string()
            })
            .collect();
        assert_ne!(order[0], order[1], "sessions should alternate");
        assert_eq!(EventType::SessionStatus.as_str(), "SessionStatus");
    }

    #[test]
    fn all_event_types_fixture_covers_every_variant() {
        use camouflage_protocol::EventType;
        let body = load_fixture("all-event-types").expect("fixture exists");
        let variants = [
            EventType::SessionStarted,
            EventType::SessionEnded,
            EventType::SessionStatus,
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
            EventType::ShowTable,
            EventType::ShowKeyValueView,
            EventType::ShowForm,
            EventType::FormResponse,
            EventType::ShowWizard,
            EventType::WizardCompleted,
            EventType::WizardCancelled,
            EventType::ModeChangeRequested,
            EventType::CancelRequested,
        ];
        for v in variants {
            let needle = format!("\"event_type\":\"{}\"", v.as_str());
            assert!(
                body.contains(&needle),
                "all-event-types fixture missing variant {:?} (looked for {:?})",
                v,
                needle
            );
        }
    }
}
