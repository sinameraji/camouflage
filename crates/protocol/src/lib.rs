//! Camouflage event protocol.
//!
//! Events are the canonical source of truth. Every event is append-only,
//! serializable to JSON, and replayable. The renderer is a subscriber.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub id: Uuid,
    pub session_id: Uuid,
    pub seq: i64,
    pub timestamp_ms: i64,
    pub schema_version: u32,
    pub event_type: EventType,
    pub payload: serde_json::Value,
}

impl Event {
    pub fn new(session_id: Uuid, seq: i64, event_type: EventType, payload: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            seq,
            timestamp_ms: now_ms(),
            schema_version: SCHEMA_VERSION,
            event_type,
            payload,
        }
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EventType {
    SessionStarted,
    SessionEnded,
    UserMessageCreated,
    AssistantStreamStarted,
    AssistantTokenDelta,
    AssistantMessageCompleted,
    ToolExecutionStarted,
    ToolExecutionStdout,
    ToolExecutionStderr,
    ToolExecutionFinished,
    PatchProposed,
    PatchApplied,
    PermissionRequested,
    PermissionGranted,
    PermissionDenied,
    RuntimeError,
    SessionCompacted,
    ViewportMarker,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::SessionStarted => "SessionStarted",
            EventType::SessionEnded => "SessionEnded",
            EventType::UserMessageCreated => "UserMessageCreated",
            EventType::AssistantStreamStarted => "AssistantStreamStarted",
            EventType::AssistantTokenDelta => "AssistantTokenDelta",
            EventType::AssistantMessageCompleted => "AssistantMessageCompleted",
            EventType::ToolExecutionStarted => "ToolExecutionStarted",
            EventType::ToolExecutionStdout => "ToolExecutionStdout",
            EventType::ToolExecutionStderr => "ToolExecutionStderr",
            EventType::ToolExecutionFinished => "ToolExecutionFinished",
            EventType::PatchProposed => "PatchProposed",
            EventType::PatchApplied => "PatchApplied",
            EventType::PermissionRequested => "PermissionRequested",
            EventType::PermissionGranted => "PermissionGranted",
            EventType::PermissionDenied => "PermissionDenied",
            EventType::RuntimeError => "RuntimeError",
            EventType::SessionCompacted => "SessionCompacted",
            EventType::ViewportMarker => "ViewportMarker",
        }
    }
}

pub mod payloads {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct UserMessage {
        pub text: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AssistantStreamStarted {
        pub stream_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AssistantTokenDelta {
        pub stream_id: String,
        pub token: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AssistantMessageCompleted {
        pub stream_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ToolStarted {
        pub tool_id: String,
        pub tool: String,
        pub command: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ToolOutput {
        pub tool_id: String,
        pub chunk: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ToolFinished {
        pub tool_id: String,
        pub exit_code: i32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct RuntimeError {
        pub message: String,
        pub source: Option<String>,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(event_type: EventType, payload: serde_json::Value) -> Event {
        Event {
            id: Uuid::nil(),
            session_id: Uuid::nil(),
            seq: 1,
            timestamp_ms: 0,
            schema_version: SCHEMA_VERSION,
            event_type,
            payload,
        }
    }

    #[test]
    fn roundtrip_all_event_types() {
        let types = [
            EventType::SessionStarted,
            EventType::SessionEnded,
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
        ];
        assert_eq!(types.len(), 18);
        for t in types {
            let ev = sample(t, json!({"k": "v"}));
            let s = serde_json::to_string(&ev).unwrap();
            let back: Event = serde_json::from_str(&s).unwrap();
            assert_eq!(ev, back);
        }
    }

    #[test]
    fn token_delta_payload_roundtrip() {
        let p = payloads::AssistantTokenDelta {
            stream_id: "s1".into(),
            token: "hello".into(),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: payloads::AssistantTokenDelta = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
