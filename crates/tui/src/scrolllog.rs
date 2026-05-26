//! Diagnostic telemetry. Two independent log streams gated by env vars:
//!   - `CAMOUFLAGE_SCROLL_LOG`  — render/scroll math (used by draw.rs)
//!   - `CAMOUFLAGE_EVENT_LOG`   — every inbound event applied to the
//!                                model + every outbound event sent
//!                                to the host. Use when scroll telemetry
//!                                isn't enough to explain "I didn't see
//!                                my message" type bugs.
//!
//! Both writers append one JSON object per line and are cheap no-ops
//! when the env var is unset.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn open_log(env_var: &str) -> Option<Mutex<std::fs::File>> {
    std::env::var(env_var).ok().and_then(|path| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()
            .map(Mutex::new)
    })
}

static SCROLL_LOG: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
static EVENT_LOG: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
static SCREEN_LOG: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();

fn scroll_handle() -> Option<&'static Mutex<std::fs::File>> {
    SCROLL_LOG.get_or_init(|| open_log("CAMOUFLAGE_SCROLL_LOG")).as_ref()
}

fn event_handle() -> Option<&'static Mutex<std::fs::File>> {
    EVENT_LOG.get_or_init(|| open_log("CAMOUFLAGE_EVENT_LOG")).as_ref()
}

fn screen_handle() -> Option<&'static Mutex<std::fs::File>> {
    SCREEN_LOG.get_or_init(|| open_log("CAMOUFLAGE_SCREEN_LOG")).as_ref()
}

pub fn enabled() -> bool {
    scroll_handle().is_some()
}

pub fn event_enabled() -> bool {
    event_handle().is_some()
}

pub fn screen_enabled() -> bool {
    screen_handle().is_some()
}

fn now_iso() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}.{:03}", d.as_secs(), d.subsec_millis())
}

fn write_to(m: &Mutex<std::fs::File>, mut payload: serde_json::Value) {
    if let serde_json::Value::Object(ref mut o) = payload {
        o.insert("ts".into(), serde_json::Value::String(now_iso()));
    }
    if let Ok(mut f) = m.lock() {
        let line = serde_json::to_string(&payload).unwrap_or_default();
        let _ = writeln!(f, "{}", line);
        let _ = f.flush();
    }
}

pub fn log_json(payload: serde_json::Value) {
    if let Some(m) = scroll_handle() {
        write_to(m, payload);
    }
}

pub fn log_event(payload: serde_json::Value) {
    if let Some(m) = event_handle() {
        write_to(m, payload);
    }
}

/// Dump the rendered contents of `rect` from `buf` as plain text, one
/// row per line, with a header noting the rect's coordinates and the
/// current frame. Used to capture literal screen content so we can see
/// exactly what the user saw (no inference required).
pub fn log_screen(
    rect: ratatui::layout::Rect,
    buf: &ratatui::buffer::Buffer,
    label: &str,
    frame: u64,
) {
    let Some(m) = screen_handle() else { return };
    let mut text = String::new();
    text.push_str(&format!(
        "=== frame={} ts={} label={} rect=(x={},y={},w={},h={}) ===\n",
        frame, now_iso(), label, rect.x, rect.y, rect.width, rect.height,
    ));
    for row in 0..rect.height {
        let y = rect.y + row;
        let mut line = String::new();
        let mut col = 0u16;
        while col < rect.width {
            let cell = &buf[(rect.x + col, y)];
            let sym = cell.symbol();
            line.push_str(sym);
            // Advance by the cell's display width. ratatui marks the
            // trailing half of a wide grapheme as an empty cell, so
            // pushing the symbol once and skipping by `width` keeps
            // alignment.
            let w = unicode_width::UnicodeWidthStr::width(sym).max(1) as u16;
            col += w;
        }
        // Right-trim spaces so blank tails don't bloat the log.
        let trimmed = line.trim_end_matches(' ').to_string();
        text.push_str(&format!("{:>3} | {}\n", row, trimmed));
    }
    text.push('\n');
    if let Ok(mut f) = m.lock() {
        let _ = f.write_all(text.as_bytes());
        let _ = f.flush();
    }
}
