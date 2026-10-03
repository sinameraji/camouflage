//! Replay recorded host sessions through the inline session and compare the
//! final screen with a saved copy, so a renderer change that would alter
//! how a real host (autopilot) looks fails here first.
//!
//! Fixtures live in `fixtures/inline/*.ndjson`, one `{"t": ms, "in" | "out":
//! event}` line per event, as written by the SDK with `CAMOUFLAGE_RECORD`.
//! `in` events are what the host sent; `out` events are what the user did
//! (a submitted prompt is typed and entered again, an answered picker is
//! closed). The screen (everything printed, then the live area) is compared
//! with `<name>.screen.txt`; run with `UPDATE_GOLDEN=1` to rewrite those.

use camouflage_inline::session::{Key, Session};
use camouflage_inline::Theme;
use camouflage_protocol::{Event, EventType};
use serde_json::Value;
use std::path::{Path, PathBuf};

const COLS: usize = 100;
const ROWS: usize = 32;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/inline")
}

fn replay(path: &Path) -> String {
    let mut s = Session::new("autopilot", Theme::default(), COLS - 1, ROWS);
    let mut printed: Vec<String> = Vec::new();
    let mut now = 0;
    for line in std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.trim().is_empty()) {
        let row: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{}: bad line ({e}): {line}", path.display()));
        now = row["t"].as_i64().unwrap_or(now);
        if let Some(ev) = row.get("in") {
            let Some(event_type) = ev["event_type"].as_str().and_then(EventType::from_str) else { continue };
            let event = Event {
                id: Default::default(),
                session_id: Default::default(),
                seq: 0,
                timestamp_ms: now,
                schema_version: 1,
                event_type,
                payload: ev.get("payload").cloned().unwrap_or(Value::Null),
            };
            s.apply(&event, now);
        } else if let Some(ev) = row.get("out") {
            match ev["event_type"].as_str() {
                Some("UserInputSubmitted") => {
                    for ch in ev["payload"]["text"].as_str().unwrap_or("").chars() {
                        s.key(Key::Text(ch.to_string()), now);
                    }
                    s.key(Key::Enter { shift: false, alt: false }, now);
                }
                Some(kind @ ("SelectListResponse" | "ConfirmResponse" | "FormResponse" | "PermissionResponse" | "WizardCancelled")) => {
                    // Keep what the dialog looked like when the user answered.
                    printed.extend(s.take_history(now).iter().map(|l| l.plain_text()));
                    printed.push(format!("──── live area when the user sent {kind} ────"));
                    printed.extend(s.live_frame(now).lines.iter().map(|l| l.plain_text()));
                    printed.push("────".into());
                    s.key(Key::Esc, now);
                }
                _ => {}
            }
            s.take_outbound();
        }
        printed.extend(s.take_history(now).iter().map(|l| l.plain_text()));
    }
    let mut screen: Vec<String> = printed;
    screen.push("──── live area ────".into());
    screen.extend(s.live_frame(now).lines.iter().map(|l| l.plain_text()));
    screen.iter().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n") + "\n"
}

#[test]
fn recorded_sessions_render_as_before() {
    let mut names: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ndjson"))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no fixtures in {}", fixtures().display());
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut failures = Vec::new();
    for path in names {
        let got = replay(&path);
        let golden = path.with_extension("screen.txt");
        if update || !golden.exists() {
            std::fs::write(&golden, &got).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&golden).unwrap();
        if got != want {
            let first = got.lines().zip(want.lines()).position(|(a, b)| a != b).unwrap_or_else(|| got.lines().count().min(want.lines().count()));
            failures.push(format!(
                "{}: screen changed at line {}\n  want: {:?}\n  got:  {:?}\n(rerun with UPDATE_GOLDEN=1 if the change is intended, and review the diff)",
                path.file_name().unwrap().to_string_lossy(),
                first + 1,
                want.lines().nth(first).unwrap_or(""),
                got.lines().nth(first).unwrap_or(""),
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
