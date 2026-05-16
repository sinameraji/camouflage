//! NDJSON event ingestion + outbound emission.
//!
//! Reads newline-delimited JSON events from any AsyncBufRead source. Lines
//! may be either full `Event` records or shorthand `{event_type, payload}`
//! objects which we normalize (filling in `id`, `seq`, `session_id`, `timestamp_ms`).

pub mod emit;
pub mod validate;

use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
use serde::Deserialize;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncBufRead};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum NdjsonError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct Shorthand {
    event_type: EventType,
    #[serde(default)]
    payload: Option<serde_json::Value>,
    #[serde(default)]
    id: Option<Uuid>,
    #[serde(default)]
    session_id: Option<Uuid>,
    #[serde(default)]
    seq: Option<i64>,
    #[serde(default)]
    timestamp_ms: Option<i64>,
    #[serde(default)]
    schema_version: Option<u32>,
}

/// Stateful decoder that assigns sequence numbers / a session id when the
/// upstream emitter does not provide them. Cheap to construct.
pub struct NdjsonDecoder {
    default_session: Uuid,
    seq: AtomicI64,
}

impl NdjsonDecoder {
    pub fn new(default_session: Uuid) -> Self {
        Self {
            default_session,
            seq: AtomicI64::new(0),
        }
    }

    pub fn parse_line(&self, line: &str) -> Result<Event, NdjsonError> {
        let s: Shorthand = serde_json::from_str(line)?;
        let seq = s.seq.unwrap_or_else(|| self.seq.fetch_add(1, Ordering::Relaxed));
        // Keep the local counter ahead of any externally-supplied seq.
        if let Some(seq_v) = s.seq {
            self.seq.fetch_max(seq_v + 1, Ordering::Relaxed);
        }
        Ok(Event {
            id: s.id.unwrap_or_else(Uuid::new_v4),
            session_id: s.session_id.unwrap_or(self.default_session),
            seq,
            timestamp_ms: s.timestamp_ms.unwrap_or_else(now_ms),
            schema_version: s.schema_version.unwrap_or(SCHEMA_VERSION),
            event_type: s.event_type,
            payload: s.payload.unwrap_or(serde_json::Value::Null),
        })
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Spawn a task that reads `reader` line-by-line and forwards `Event`s into `tx`.
/// JSON errors are surfaced as `RuntimeError` events (never panic).
pub async fn run_reader<R>(
    reader: R,
    decoder: NdjsonDecoder,
    tx: tokio::sync::mpsc::Sender<Event>,
) -> Result<(), NdjsonError>
where
    R: AsyncBufRead + Unpin,
{
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        match decoder.parse_line(&line) {
            Ok(ev) => {
                if tx.send(ev).await.is_err() {
                    break;
                }
            }
            Err(e) => {
                let err_ev = Event {
                    id: Uuid::new_v4(),
                    session_id: decoder.default_session,
                    seq: decoder.seq.fetch_add(1, Ordering::Relaxed),
                    timestamp_ms: now_ms(),
                    schema_version: SCHEMA_VERSION,
                    event_type: EventType::RuntimeError,
                    payload: serde_json::json!({
                        "message": format!("ndjson parse error: {e}"),
                        "source": "headless",
                    }),
                };
                if tx.send(err_ev).await.is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn shorthand_line_decodes() {
        let d = NdjsonDecoder::new(Uuid::nil());
        let ev = d
            .parse_line(r#"{"event_type":"UserMessageCreated","payload":{"text":"hi"}}"#)
            .unwrap();
        assert_eq!(ev.event_type, EventType::UserMessageCreated);
        assert_eq!(ev.payload["text"], "hi");
        assert_eq!(ev.seq, 0);
    }

    #[tokio::test]
    async fn seq_increments() {
        let d = NdjsonDecoder::new(Uuid::nil());
        let a = d.parse_line(r#"{"event_type":"SessionStarted"}"#).unwrap();
        let b = d.parse_line(r#"{"event_type":"SessionEnded"}"#).unwrap();
        assert_eq!(a.seq, 0);
        assert_eq!(b.seq, 1);
    }

    #[tokio::test]
    async fn parse_errors_become_runtime_errors() {
        let d = NdjsonDecoder::new(Uuid::nil());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let input = "not json\n{\"event_type\":\"SessionEnded\"}\n".as_bytes();
        let reader = BufReader::new(input);
        tokio::spawn(async move { run_reader(reader, d, tx).await.unwrap() });
        let e1 = rx.recv().await.unwrap();
        assert_eq!(e1.event_type, EventType::RuntimeError);
        let e2 = rx.recv().await.unwrap();
        assert_eq!(e2.event_type, EventType::SessionEnded);
    }
}
