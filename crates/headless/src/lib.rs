//! NDJSON event ingestion + outbound emission.
//!
//! Reads newline-delimited JSON events from any AsyncBufRead source. Lines
//! may be either full `Event` records or shorthand `{event_type, payload}`
//! objects which we normalize (filling in `id`, `seq`, `session_id`, `timestamp_ms`).

pub mod emit;
pub mod fixtures;
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
    /// Forward-compat: an event_type the running renderer doesn't know
    /// about. Callers may choose to skip these silently instead of
    /// surfacing them as RuntimeError rows, which is important whenever
    /// a host ships ahead of the renderer binary.
    #[error("unknown event_type: {0}")]
    UnknownEventType(String),
}

#[derive(Debug, Deserialize)]
struct Shorthand {
    /// Parsed as a raw string and converted into `EventType` inside
    /// `parse_line` so unknown variants can be reported with a typed
    /// error (and skipped) instead of taking down the whole event.
    event_type: String,
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
        let Some(event_type) = EventType::from_str(&s.event_type) else {
            return Err(NdjsonError::UnknownEventType(s.event_type));
        };
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
            event_type,
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
            Err(NdjsonError::UnknownEventType(name)) => {
                // Forward-compat: a newer host shipped an event type
                // this renderer build doesn't recognise. Silently skip
                // instead of polluting the transcript with a noisy
                // RuntimeError row the user can't act on. Surface via
                // an env-gated stderr line for diagnostics.
                if std::env::var_os("CAMOUFLAGE_DEBUG_NDJSON").is_some() {
                    eprintln!("[camouflage] skipping unknown event_type: {name}");
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

    #[tokio::test]
    async fn unknown_event_type_is_skipped_not_surfaced() {
        // A host shipped with a Camouflage variant this renderer doesn't
        // know about. The stream must keep flowing; only known events
        // reach the channel.
        let d = NdjsonDecoder::new(Uuid::nil());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let input = "{\"event_type\":\"FutureMagic\"}\n{\"event_type\":\"SessionEnded\"}\n"
            .as_bytes();
        let reader = BufReader::new(input);
        tokio::spawn(async move { run_reader(reader, d, tx).await.unwrap() });
        let next = rx.recv().await.unwrap();
        assert_eq!(next.event_type, EventType::SessionEnded);
        // No second event — the unknown one was dropped silently.
        assert!(rx.recv().await.is_none());
    }
}
