//! The always-on half of the fuzzing story.
//!
//! `cargo fuzz` needs nightly and only runs in one CI job, so the same
//! guarantee — no host input can panic the renderer — is also checked here
//! with a fixed, deterministic corpus. Every platform runs this on every PR;
//! the fuzz targets in `crates/inline/fuzz` explore the same code paths with
//! real coverage guidance.

#[path = "common/fuzz_drive.rs"]
mod fuzz_drive;

use camouflage_inline::session::Session;
use camouflage_inline::Theme;
use camouflage_protocol::Event;
use serde_json::{json, Value};

/// xorshift64: deterministic, no dependency, good enough to spread bytes.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}

#[test]
fn arbitrary_event_streams_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..2_000 {
        let len = (rng.next() % 512) as usize;
        fuzz_drive::drive_apply(&rng.bytes(len));
    }
}

#[test]
fn arbitrary_key_streams_never_panic() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for _ in 0..2_000 {
        let len = (rng.next() % 256) as usize;
        fuzz_drive::drive_key(&rng.bytes(len));
    }
}

/// Payloads a host should never send but a buggy one might: wrong types,
/// missing fields, truncated diffs, nulls where objects belong.
fn hostile_payloads() -> Vec<Value> {
    vec![
        Value::Null,
        json!({}),
        json!({ "text": 1, "token": [], "options": "nope" }),
        json!({ "options": [null, 1, "x", { "value": null }] }),
        json!({ "fields": [null, { "name": null, "kind": 7 }] }),
        json!({ "steps": [null, { "kind": "select" }, { "kind": "confirm" }] }),
        json!({ "diff": { "unified": "@@ garbage\n-not a diff" } }),
        json!({ "diff": "not an object" }),
        json!({ "segments": { "phase": 5, "mode": [] } }),
        json!({ "todos": [null, { "title": null, "status": 7 }] }),
        json!({ "columns": [], "rows": [{}] }),
        json!({ "items": [null, { "label": 1 }] }),
        json!({ "id": "x", "kind": "agent", "status": "running", "steps": [null] }),
        json!({ "candidates": [null, { "token": 3 }] }),
        json!({ "commands": [null, { "name": 9 }] }),
        json!({ "exit_code": "not a number", "started_at_ms": -1 }),
        json!({ "text": "x".repeat(10_000) }),
    ]
}

#[test]
fn every_event_type_survives_hostile_payloads() {
    for kind in fuzz_drive::INBOUND {
        for payload in hostile_payloads() {
            let mut s = Session::new("autopilot", Theme::default(), 80, 24);
            let ev = Event {
                id: Default::default(),
                session_id: Default::default(),
                seq: 0,
                timestamp_ms: 0,
                schema_version: 1,
                event_type: *kind,
                payload,
            };
            s.apply(&ev, 0);
            let _ = s.take_history(0);
            let _ = s.live_frame(0);
            let _ = s.next_wakeup(0);
            let _ = s.take_outbound();
        }
    }
}

/// A prompt answered before it is armed, a form cancelled mid-edit, a wizard
/// stepped past its end: keys at the wrong moment must be dropped, not panic.
#[test]
fn keys_at_the_wrong_moment_never_panic() {
    use camouflage_inline::session::Key;
    let keys = [
        Key::Enter { shift: false, alt: false },
        Key::Esc,
        Key::Tab,
        Key::Down,
        Key::Up,
        Key::Text("y".into()),
        Key::Backspace { word: false },
        Key::Ctrl('c'),
    ];
    let mut s = Session::new("autopilot", Theme::default(), 80, 24);
    for (i, key) in keys.iter().enumerate() {
        s.key(key.clone(), i as i64);
        let _ = s.live_frame(i as i64);
        let _ = s.take_outbound();
    }
}
