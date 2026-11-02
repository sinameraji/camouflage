//! `--replay` / `--play`: render a recorded session through the inline
//! renderer.
//!
//! The full-screen replay viewer is gone. A replay is now what the inline
//! design says it should be: read the events from SQLite (or a `.camo` /
//! NDJSON file), apply them to a [`Session`], and print the finished blocks
//! into the terminal's normal scrollback. Nothing is redrawn and nothing is
//! kept in memory after the transcript is written.

use anyhow::{Context, Result};
use camouflage_headless::{NdjsonDecoder, NdjsonError};
use camouflage_inline::session::Session;
use camouflage_inline::{Line, Theme};
use camouflage_protocol::Event;
use camouflage_store::{EventStore, SqliteStore};
use std::io::Write;
use std::path::Path;
use uuid::Uuid;

pub struct ReplayConfig {
    pub app_title: String,
    pub theme: Theme,
    pub width: usize,
    pub height: usize,
}

/// Render a stored session. Errors when the id is unknown or the session is
/// empty, so a typo doesn't silently print nothing.
pub fn replay_session(store: &SqliteStore, session_id: Uuid, cfg: &ReplayConfig) -> Result<Vec<Line>> {
    let events = store
        .load_session(session_id)
        .with_context(|| format!("loading session {session_id}"))?;
    if events.is_empty() {
        anyhow::bail!("session {session_id} not found (or empty)");
    }
    Ok(transcript(&events, cfg))
}

/// Render a `.camo` container or plain NDJSON file.
pub fn replay_file(path: &Path, cfg: &ReplayConfig) -> Result<Vec<Line>> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let events = parse_ndjson(&body, path)?;
    if events.is_empty() {
        anyhow::bail!("{} contains no events", path.display());
    }
    Ok(transcript(&events, cfg))
}

/// Apply every event to a fresh session and return the whole transcript.
/// `TranscriptCleared` drops what was printed before it, exactly as the live
/// renderer clears the screen.
pub fn transcript(events: &[Event], cfg: &ReplayConfig) -> Vec<Line> {
    let mut session = Session::new(&cfg.app_title, cfg.theme.clone(), cfg.width, cfg.height);
    // A replay is a transcript, not a live turn: show tool output in full.
    session.set_expanded(true);
    let mut out = Vec::new();
    let mut now = 0;
    for ev in events {
        now = now.max(ev.timestamp_ms);
        session.apply(ev, now);
        if session.clear_screen {
            session.clear_screen = false;
            out.clear();
        }
        out.extend(session.take_history(now));
    }
    out.extend(session.finish_transcript(now));
    out
}

/// Parse a `.camo` file (optional `{"camo_version": …}` header) or plain
/// NDJSON. Unknown event types are skipped, matching the live renderer.
fn parse_ndjson(body: &str, path: &Path) -> Result<Vec<Event>> {
    let decoder = NdjsonDecoder::new(Uuid::nil());
    let mut events = Vec::new();
    for (n, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if n == 0
            && serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v.get("camo_version").cloned())
                .is_some()
        {
            continue;
        }
        match decoder.parse_line(line) {
            Ok(ev) => events.push(ev),
            Err(NdjsonError::UnknownEventType(_)) => continue,
            Err(e) => anyhow::bail!("{}:{}: {e}", path.display(), n + 1),
        }
    }
    Ok(events)
}

/// Write the transcript to `out`, one line per row, with no live region.
pub fn write_transcript(out: &mut impl Write, lines: &[Line], width: usize) -> Result<()> {
    let mut buf = String::new();
    for line in lines {
        line.encode(width, &mut buf);
        buf.push_str("\x1b[0m\n");
    }
    out.write_all(buf.as_bytes())?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use camouflage_protocol::{EventType, SCHEMA_VERSION};
    use serde_json::json;

    fn ev(seq: i64, event_type: EventType, payload: serde_json::Value) -> Event {
        Event {
            id: Uuid::nil(),
            session_id: Uuid::nil(),
            seq,
            timestamp_ms: seq * 10,
            schema_version: SCHEMA_VERSION,
            event_type,
            payload,
        }
    }

    fn cfg() -> ReplayConfig {
        ReplayConfig {
            app_title: "autopilot".into(),
            theme: Theme::dark(),
            width: 80,
            height: 24,
        }
    }

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(Line::plain_text).collect()
    }

    #[test]
    fn a_replay_prints_the_whole_conversation() {
        let events = vec![
            ev(0, EventType::SessionStarted, json!({})),
            ev(1, EventType::UserMessageCreated, json!({ "text": "fix the bug" })),
            ev(2, EventType::AssistantStreamStarted, json!({ "stream_id": "s" })),
            ev(3, EventType::AssistantTokenDelta, json!({ "stream_id": "s", "token": "Done." })),
            ev(4, EventType::AssistantMessageCompleted, json!({ "stream_id": "s" })),
        ];
        let lines = text(&transcript(&events, &cfg()));
        let joined = lines.join("\n");
        assert!(joined.contains("fix the bug"), "{joined}");
        assert!(joined.contains("Done."), "{joined}");
    }

    #[test]
    fn a_replay_shows_tool_output_in_full() {
        let events = vec![
            ev(0, EventType::SessionStarted, json!({})),
            ev(1, EventType::ToolExecutionStarted, json!({ "tool_id": "t", "tool": "Bash", "command": "ls" })),
            ev(2, EventType::ToolExecutionFinished, json!({
                "tool_id": "t",
                "exit_code": 0,
                "output": "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\n"
            })),
        ];
        let joined = text(&transcript(&events, &cfg())).join("\n");
        assert!(joined.contains("a"), "{joined}");
        assert!(joined.contains("p"), "the last output line is shown: {joined}");
        assert!(!joined.contains("ctrl+o"), "a replay has no live area to expand: {joined}");
    }

    #[test]
    fn transcript_cleared_drops_earlier_output() {
        let events = vec![
            ev(0, EventType::SessionStarted, json!({})),
            ev(1, EventType::UserMessageCreated, json!({ "text": "old prompt" })),
            ev(2, EventType::TranscriptCleared, json!({})),
            ev(3, EventType::UserMessageCreated, json!({ "text": "new prompt" })),
        ];
        let joined = text(&transcript(&events, &cfg())).join("\n");
        assert!(!joined.contains("old prompt"), "{joined}");
        assert!(joined.contains("new prompt"), "{joined}");
    }

    #[test]
    fn a_camo_header_is_skipped() {
        let body = concat!(
            "{\"camo_version\":1}\n",
            "{\"event_type\":\"UserMessageCreated\",\"payload\":{\"text\":\"hi\"}}\n",
        );
        let events = parse_ndjson(body, Path::new("x.camo")).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::UserMessageCreated);
    }
}
