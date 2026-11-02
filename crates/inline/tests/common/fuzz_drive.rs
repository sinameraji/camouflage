//! Shared driver for the `cargo fuzz` targets and the always-on smoke test.
//!
//! Turns arbitrary bytes into a sequence of protocol events (or key presses)
//! and runs them through a [`Session`]. Both the fuzz targets and
//! `tests/fuzz_smoke.rs` use this, so the protection that runs on every
//! platform in CI is the same code the fuzzer explores on nightly.
//!
//! The contract is simple: **no input may panic**. A malformed payload, an
//! unknown field, a truncated diff, or a key pressed at the wrong moment must
//! be ignored, not crash the renderer.

// Each fuzz target includes this whole module but uses only one entry point.
#![allow(dead_code)]

use camouflage_inline::session::{Key, Session};
use camouflage_inline::Theme;
use camouflage_protocol::{Event, EventType};
use serde_json::{json, Value};

/// Every inbound event type the session handles, plus the no-ops, so the
/// fuzzer can reach each arm of `Session::apply`.
pub const INBOUND: &[EventType] = &[
    EventType::SessionStarted,
    EventType::SessionEnded,
    EventType::Splash,
    EventType::UserMessageCreated,
    EventType::AssistantStreamStarted,
    EventType::AssistantReasoningDelta,
    EventType::AssistantTokenDelta,
    EventType::AssistantMessageCompleted,
    EventType::ToolExecutionStarted,
    EventType::ToolExecutionStdout,
    EventType::ToolExecutionStderr,
    EventType::ToolExecutionFinished,
    EventType::PatchProposed,
    EventType::PermissionRequested,
    EventType::ShowSelectList,
    EventType::ShowConfirm,
    EventType::ShowTable,
    EventType::ShowKeyValueView,
    EventType::ShowForm,
    EventType::ShowWizard,
    EventType::RuntimeError,
    EventType::SessionCompacted,
    EventType::ShowToast,
    EventType::StatusUpdate,
    EventType::TodoListUpdate,
    EventType::BackgroundTaskUpdate,
    EventType::SlashCommandsRegistered,
    EventType::MentionCandidatesRegistered,
    EventType::ActivityUpdate,
    EventType::ActivitySnapshot,
    EventType::ActivityLog,
    EventType::ActivityRemoved,
    EventType::ActivityBrowserOpen,
    EventType::TranscriptCleared,
    EventType::CancelRequested,
    EventType::TerminalSuspend,
    EventType::TerminalResume,
];

fn event(kind: EventType, payload: Value, now: i64) -> Event {
    Event {
        id: Default::default(),
        session_id: Default::default(),
        seq: 0,
        timestamp_ms: now,
        schema_version: 1,
        event_type: kind,
        payload,
    }
}

/// A payload built from the fuzz bytes. Structured JSON is used when the
/// bytes happen to parse as an object; otherwise every string field the
/// session reads is filled with the same text, so the fuzzer reaches the
/// payload-dependent branches instead of bouncing off `Value::Null`.
fn payload_for(bytes: &[u8], now: i64) -> Value {
    if let Ok(v) = serde_json::from_slice::<Value>(bytes) {
        if v.is_object() {
            return v;
        }
    }
    let text = String::from_utf8_lossy(bytes).into_owned();
    let option = json!({ "value": text, "label": text, "description": text });
    // Built as a map rather than one `json!` literal: the literal is deep
    // enough to hit the macro recursion limit.
    let mut m = serde_json::Map::new();
    for key in [
        "text", "token", "chunk", "tool", "command", "tool_id", "stream_id", "id",
        "request_id", "path", "message", "title", "label", "prompt", "action", "summary",
        "output", "unified", "before", "after", "task_id", "kind", "state", "severity",
        "accent", "assistant_label", "default",
    ] {
        m.insert(key.into(), Value::String(text.clone()));
    }
    m.insert("detail".into(), json!([text]));
    m.insert("exit_code".into(), json!(bytes.first().copied().unwrap_or(0) as i64 - 128));
    m.insert("started_at_ms".into(), json!(now));
    m.insert("progress".into(), json!(0.5));
    m.insert("allow_cancel".into(), json!(true));
    m.insert("allow_filter".into(), json!(true));
    m.insert("synthetic".into(), json!(false));
    m.insert("diff".into(), json!({ "path": text, "unified": text }));
    m.insert("options".into(), json!([option]));
    m.insert(
        "fields".into(),
        json!([{ "name": text, "label": text, "kind": text, "default": text, "required": true }]),
    );
    m.insert(
        "steps".into(),
        json!([{ "kind": text, "id": text, "prompt": text, "title": text, "options": [option] }]),
    );
    m.insert("todos".into(), json!([{ "title": text, "status": text }]));
    m.insert("items".into(), json!([{ "label": text, "value": text }]));
    m.insert("columns".into(), json!([{ "name": text, "label": text }]));
    m.insert("rows".into(), json!([{ "text": text }]));
    m.insert("candidates".into(), json!([{ "token": text, "kind": text }]));
    m.insert(
        "commands".into(),
        json!([{ "name": text, "description": text, "args_hint": text }]),
    );
    m.insert("segments".into(), json!({ "phase": text, "model": text, "mode": text }));
    Value::Object(m)
}

fn new_session() -> Session {
    Session::new("autopilot", Theme::default(), 80, 24)
}

/// Feed `data` to `Session::apply` as a sequence of events.
///
/// Each event is `[type tag][payload length][payload bytes]`; the tag picks
/// the event type and the length frames the payload, so one input can drive
/// a whole session rather than a single event.
pub fn drive_apply(data: &[u8]) {
    let mut s = new_session();
    let mut now = 0i64;
    let mut rest = data;
    while rest.len() >= 2 {
        let tag = rest[0];
        let len = rest[1] as usize;
        rest = &rest[2..];
        let take = len.min(rest.len());
        let (payload_bytes, tail) = rest.split_at(take);
        rest = tail;

        now += 1;
        let kind = INBOUND[tag as usize % INBOUND.len()];
        s.apply(&event(kind, payload_for(payload_bytes, now), now), now);
        // Everything the app does after an event: print, draw, schedule.
        let _ = s.take_history(now);
        let _ = s.live_frame(now);
        let _ = s.next_wakeup(now);
        let _ = s.take_outbound();
        if s.exit {
            break;
        }
    }
}

/// Seed a session with a live turn and an open prompt, so keys have
/// something to act on instead of landing on an empty editor.
fn seed(s: &mut Session) {
    s.apply(
        &event(
            EventType::SessionStarted,
            json!({ "title": "autopilot", "detail": ["x"], "assistant_label": "autopilot" }),
            0,
        ),
        0,
    );
    s.apply(
        &event(EventType::StatusUpdate, json!({ "segments": { "phase": "thinking" } }), 0),
        0,
    );
    s.apply(
        &event(EventType::AssistantStreamStarted, json!({ "stream_id": "s" }), 0),
        0,
    );
    s.apply(
        &event(
            EventType::ToolExecutionStarted,
            json!({ "tool_id": "t", "tool": "Read", "command": "x" }),
            0,
        ),
        0,
    );
    s.apply(
        &event(
            EventType::ShowSelectList,
            json!({
                "id": "p",
                "prompt": "Pick",
                "options": [{ "value": "a", "label": "a" }, { "value": "b", "label": "b" }],
            }),
            0,
        ),
        0,
    );
    let _ = s.take_history(0);
    let _ = s.take_outbound();
}

fn key_for(chunk: &[u8]) -> Key {
    let b = chunk[0];
    let c = chunk.get(1).copied().unwrap_or(b);
    match b % 20 {
        0 => Key::Text((c as char).to_string()),
        1 => Key::Paste(String::from_utf8_lossy(chunk).into_owned()),
        2 => Key::Enter { shift: c % 2 == 0, alt: c % 3 == 0 },
        3 => Key::Esc,
        4 => Key::Tab,
        5 => Key::BackTab,
        6 => Key::Up,
        7 => Key::Down,
        8 => Key::Left { word: c % 2 == 0 },
        9 => Key::Right { word: c % 2 == 0 },
        10 => Key::Home,
        11 => Key::End,
        12 => Key::Backspace { word: c % 2 == 0 },
        13 => Key::Delete { word: c % 2 == 0 },
        14 => Key::PageUp,
        15 => Key::PageDown,
        16 => Key::Ctrl((b'a' + (c % 26)) as char),
        17 => Key::Ctrl('c'),
        18 => Key::Ctrl('o'),
        _ => Key::Ctrl('b'),
    }
}

/// Feed `data` to `Session::key` as a sequence of key presses.
pub fn drive_key(data: &[u8]) {
    let mut s = new_session();
    seed(&mut s);
    let mut now = 0i64;
    for chunk in data.chunks(2) {
        now += 1;
        s.key(key_for(chunk), now);
        let _ = s.take_history(now);
        let _ = s.live_frame(now);
        let _ = s.next_wakeup(now);
        let _ = s.take_outbound();
        if s.exit {
            break;
        }
    }
}
