//! Golden-snapshot regression testing.
//!
//! For each `fixtures/*.ndjson` we ship, an optional matching
//! `fixtures/golden/<name>.snapshot.json` captures the expected `Snapshot`
//! after applying every event in order to a fresh `RenderModel`.
//!
//! The test `golden_snapshots_match` (in `crates/renderer/tests/`) loads
//! every fixture, applies it, computes a Snapshot, and asserts it equals
//! the checked-in golden. This catches *any* renderer regression that
//! changes externally-visible output — a stricter gate than the existing
//! "fixture lines validate" check.
//!
//! To regenerate goldens after an intentional change, set the env var
//! `CAMOUFLAGE_UPDATE_GOLDENS=1` and re-run `cargo test`.
//!
//! Golden JSON is pretty-printed so diffs in PRs are reviewable.

use crate::{RenderModel, Renderer, Snapshot, SnapshotRenderer};
use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
use serde::Deserialize;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct EventLine {
    event_type: EventType,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

/// Apply a single NDJSON stream to a fresh RenderModel and return the
/// resulting Snapshot. Lines that fail to parse are skipped silently to
/// mirror the renderer's lenient on-wire behaviour — strict validation
/// is the validator's job.
pub fn snapshot_of_ndjson(ndjson: &str) -> Snapshot {
    let mut model = RenderModel::new();
    let mut seq: i64 = 0;
    for line in ndjson.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: Result<EventLine, _> = serde_json::from_str(line);
        let Ok(p) = parsed else { continue };
        let ev = Event {
            id: Uuid::nil(),
            session_id: Uuid::nil(),
            seq,
            timestamp_ms: seq, // deterministic
            schema_version: SCHEMA_VERSION,
            event_type: p.event_type,
            payload: p.payload.unwrap_or(serde_json::Value::Null),
        };
        model.apply(&ev);
        seq += 1;
    }
    model.snapshot()
}

/// Convenience: read a fixture NDJSON file and produce its snapshot.
pub fn snapshot_of_fixture_path(path: &Path) -> std::io::Result<Snapshot> {
    let body = std::fs::read_to_string(path)?;
    Ok(snapshot_of_ndjson(&body))
}

/// Stable, pretty-printed JSON suitable for checking in as a golden file.
pub fn snapshot_to_golden_json(snap: &Snapshot) -> String {
    serde_json::to_string_pretty(snap).expect("Snapshot is always serializable")
}
