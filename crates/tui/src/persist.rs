//! What the inline renderer saves to the session store.
//!
//! Saving every event as it arrives grew a real user's store to 400 MB:
//! three quarters of it a host re-sending an identical slash-command list,
//! the rest mostly one row per streamed token. Like Claude Code's
//! transcripts, the store keeps conversations, not keystrokes:
//!
//! - streamed reply and reasoning tokens become one delta per stream,
//!   written just before the stream's `AssistantMessageCompleted`;
//! - a tool's stdout/stderr chunks become one chunk (its last 64 KB), written
//!   just before `ToolExecutionFinished`;
//! - a slash-command or mention list, or a status update, identical to the
//!   last one saved is skipped; mention answers for a typed path and live
//!   activity output aren't saved at all.
//!
//! A replay still shows every reply and tool, just not token by token. A
//! crash loses at most the stream that was in flight.

use camouflage_protocol::{Event, EventType};
use std::collections::HashMap;

/// Tool output kept per tool when coalescing.
const TOOL_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub struct Coalescer {
    /// (event type, stream/tool id) → (first event of the run, text so far).
    pending: HashMap<(EventType, String), (Event, String)>,
    /// Order the runs started in, so flushes keep that order.
    order: Vec<(EventType, String)>,
    /// Last saved payload per registration/status event type.
    last: HashMap<EventType, String>,
}

fn field(ev: &Event, key: &str) -> String {
    ev.payload.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

impl Coalescer {
    /// Take one event; return what should be saved now, in order.
    pub fn push(&mut self, ev: Event) -> Vec<Event> {
        use EventType::*;
        match ev.event_type {
            AssistantTokenDelta | AssistantReasoningDelta => {
                let key = (ev.event_type, field(&ev, "stream_id"));
                self.append(key, ev, "token", usize::MAX);
                Vec::new()
            }
            ToolExecutionStdout | ToolExecutionStderr => {
                let key = (ev.event_type, field(&ev, "tool_id"));
                self.append(key, ev, "chunk", TOOL_OUTPUT_BYTES);
                Vec::new()
            }
            AssistantMessageCompleted => {
                let id = field(&ev, "stream_id");
                let mut out = self.take(&[(AssistantReasoningDelta, id.clone()), (AssistantTokenDelta, id)]);
                out.push(ev);
                out
            }
            ToolExecutionFinished => {
                let id = field(&ev, "tool_id");
                let mut out = self.take(&[(ToolExecutionStdout, id.clone()), (ToolExecutionStderr, id)]);
                out.push(ev);
                out
            }
            ActivityLog | MentionQuery => Vec::new(),
            MentionCandidatesRegistered if ev.payload.get("for_query").is_some() => Vec::new(),
            SlashCommandsRegistered | MentionCandidatesRegistered | StatusUpdate => {
                let body = ev.payload.to_string();
                if self.last.get(&ev.event_type) == Some(&body) {
                    return Vec::new();
                }
                self.last.insert(ev.event_type, body);
                vec![ev]
            }
            _ => vec![ev],
        }
    }

    /// Everything still buffered (streams that never completed), e.g. on
    /// shutdown.
    pub fn flush(&mut self) -> Vec<Event> {
        let keys = self.order.clone();
        self.take(&keys)
    }

    fn append(&mut self, key: (EventType, String), ev: Event, field_name: &str, cap: usize) {
        let piece = field(&ev, field_name);
        match self.pending.get_mut(&key) {
            Some((_, text)) => {
                text.push_str(&piece);
                if text.len() > cap {
                    let mut cut = text.len() - cap;
                    while !text.is_char_boundary(cut) {
                        cut += 1;
                    }
                    text.drain(..cut);
                }
            }
            None => {
                self.order.push(key.clone());
                self.pending.insert(key, (ev, piece));
            }
        }
    }

    fn take(&mut self, keys: &[(EventType, String)]) -> Vec<Event> {
        let mut out = Vec::new();
        for key in keys {
            if let Some((mut first, text)) = self.pending.remove(key) {
                let name = if matches!(key.0, EventType::ToolExecutionStdout | EventType::ToolExecutionStderr) { "chunk" } else { "token" };
                first.payload[name] = serde_json::Value::String(text);
                out.push(first);
            }
        }
        self.order.retain(|k| self.pending.contains_key(k));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: i64, t: EventType, payload: serde_json::Value) -> Event {
        Event { id: Default::default(), session_id: Default::default(), seq, timestamp_ms: seq, schema_version: 1, event_type: t, payload }
    }

    #[test]
    fn a_streamed_reply_is_saved_as_one_delta_then_completion() {
        let mut c = Coalescer::default();
        assert_eq!(c.push(ev(0, EventType::AssistantStreamStarted, json!({"stream_id": "s"}))).len(), 1);
        for (i, tok) in ["Hel", "lo ", "world"].iter().enumerate() {
            assert!(c.push(ev(1 + i as i64, EventType::AssistantTokenDelta, json!({"stream_id": "s", "token": tok}))).is_empty());
        }
        c.push(ev(4, EventType::AssistantReasoningDelta, json!({"stream_id": "s", "token": "think"})));
        let out = c.push(ev(5, EventType::AssistantMessageCompleted, json!({"stream_id": "s"})));
        let kinds: Vec<_> = out.iter().map(|e| (e.event_type, e.seq)).collect();
        assert_eq!(kinds, vec![(EventType::AssistantReasoningDelta, 4), (EventType::AssistantTokenDelta, 1), (EventType::AssistantMessageCompleted, 5)]);
        assert_eq!(out[1].payload["token"], "Hello world");
    }

    #[test]
    fn tool_output_is_one_bounded_chunk() {
        let mut c = Coalescer::default();
        c.push(ev(0, EventType::ToolExecutionStarted, json!({"tool_id": "t"})));
        for i in 0..2000 {
            c.push(ev(1 + i, EventType::ToolExecutionStdout, json!({"tool_id": "t", "chunk": format!("line {i} {}\n", "x".repeat(40))})));
        }
        let out = c.push(ev(9999, EventType::ToolExecutionFinished, json!({"tool_id": "t", "exit_code": 0})));
        assert_eq!(out.len(), 2);
        let chunk = out[0].payload["chunk"].as_str().unwrap();
        assert!(chunk.len() <= TOOL_OUTPUT_BYTES);
        assert!(chunk.contains("line 1999"), "the newest output is kept");
        assert!(!chunk.contains("line 0 "), "the oldest output is dropped");
    }

    #[test]
    fn repeated_registrations_and_statuses_are_saved_once() {
        let mut c = Coalescer::default();
        let cmds = json!({"commands": [{"name": "help"}]});
        assert_eq!(c.push(ev(0, EventType::SlashCommandsRegistered, cmds.clone())).len(), 1);
        for i in 1..100 {
            assert!(c.push(ev(i, EventType::SlashCommandsRegistered, cmds.clone())).is_empty());
        }
        assert_eq!(c.push(ev(100, EventType::SlashCommandsRegistered, json!({"commands": []}))).len(), 1, "a changed list is saved");
        let st = json!({"segments": {"phase": "idle"}});
        assert_eq!(c.push(ev(101, EventType::StatusUpdate, st.clone())).len(), 1);
        assert!(c.push(ev(102, EventType::StatusUpdate, st)).is_empty());
        assert!(c.push(ev(103, EventType::ActivityLog, json!({"id": "j", "chunk": "x"}))).is_empty());
        assert!(c.push(ev(104, EventType::MentionCandidatesRegistered, json!({"for_query": "../", "candidates": []}))).is_empty());
    }

    #[test]
    fn unfinished_streams_are_flushed_in_order() {
        let mut c = Coalescer::default();
        c.push(ev(1, EventType::AssistantTokenDelta, json!({"stream_id": "a", "token": "x"})));
        c.push(ev(2, EventType::ToolExecutionStdout, json!({"tool_id": "t", "chunk": "y"})));
        let out = c.flush();
        assert_eq!(out.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2]);
        assert!(c.flush().is_empty());
    }
}
