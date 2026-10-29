//! NDJSON event ingestion + outbound emission.
//!
//! Reads newline-delimited JSON events from any AsyncBufRead source. Lines
//! may be either full `Event` records or shorthand `{event_type, payload}`
//! objects which we normalize (filling in `id`, `seq`, `session_id`, `timestamp_ms`).

pub mod broadcast_auth;
pub mod emit;
pub mod fixtures;
pub mod validate;

use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum NdjsonError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Forward-compat: an event_type the running renderer doesn't know
    /// about. Callers can surface a diagnostic while continuing the stream.
    #[error("unknown event_type: {0}")]
    UnknownEventType(String),
    #[error("schema version mismatch: host uses {received}, renderer supports {supported}")]
    SchemaVersionMismatch { received: u32, supported: u32 },
    #[error("invalid payload for {event_type:?}: {source}")]
    InvalidPayload {
        event_type: EventType,
        #[source]
        source: serde_json::Error,
    },
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
///
/// One stream may carry several sessions: events from different
/// `session_id`s interleave, and each session keeps its own `seq` counter
/// (the store's uniqueness is `(session_id, seq)`). A single-session host
/// sees exactly the old behaviour — one counter, starting at 0.
pub struct NdjsonDecoder {
    default_session: Uuid,
    /// Next `seq` per session.
    seq: Mutex<HashMap<Uuid, i64>>,
}

impl NdjsonDecoder {
    pub fn new(default_session: Uuid) -> Self {
        Self {
            default_session,
            seq: Mutex::new(HashMap::new()),
        }
    }

    /// The next `seq` for `session_id`, advancing that session's counter.
    fn next_seq(&self, session_id: Uuid) -> i64 {
        let mut seqs = self.seq.lock().unwrap();
        let next = seqs.entry(session_id).or_insert(0);
        let seq = *next;
        *next += 1;
        seq
    }

    /// Keep a session's counter ahead of an externally-supplied `seq`.
    fn observe_seq(&self, session_id: Uuid, seq: i64) {
        let mut seqs = self.seq.lock().unwrap();
        let next = seqs.entry(session_id).or_insert(0);
        *next = (*next).max(seq + 1);
    }

    pub fn parse_line(&self, line: &str) -> Result<Event, NdjsonError> {
        let s: Shorthand = serde_json::from_str(line)?;
        let Some(event_type) = EventType::from_str(&s.event_type) else {
            return Err(NdjsonError::UnknownEventType(s.event_type));
        };
        let schema_version = s.schema_version.unwrap_or(SCHEMA_VERSION);
        // Zero was used by pre-versioned event producers and remains an
        // explicit legacy sentinel. Every nonzero version must agree.
        if schema_version != 0 && schema_version != SCHEMA_VERSION {
            return Err(NdjsonError::SchemaVersionMismatch {
                received: schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        let payload = s.payload.unwrap_or(serde_json::Value::Null);
        crate::validate::validate_payload(event_type, payload.clone())
            .map_err(|source| NdjsonError::InvalidPayload { event_type, source })?;
        let session_id = s.session_id.unwrap_or(self.default_session);
        let seq = match s.seq {
            Some(seq) => {
                self.observe_seq(session_id, seq);
                seq
            }
            None => self.next_seq(session_id),
        };
        Ok(Event {
            id: s.id.unwrap_or_else(Uuid::new_v4),
            session_id,
            seq,
            timestamp_ms: s.timestamp_ms.unwrap_or_else(now_ms),
            schema_version: if schema_version == 0 {
                SCHEMA_VERSION
            } else {
                schema_version
            },
            event_type,
            payload,
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
    let mut warned_unknown = HashSet::new();
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
                // One visible diagnostic per unknown type keeps a newer host
                // from silently losing events without flooding the transcript.
                if warned_unknown.insert(name.clone()) {
                    let message = format!(
                        "Ignored unknown event {name:?}. The host may need a newer Camouflage renderer."
                    );
                    if tx.send(diagnostic_event(&decoder, message)).await.is_err() {
                        break;
                    }
                }
            }
            Err(e) => {
                let message = format!("Ignored invalid event from host: {e}");
                if tx.send(diagnostic_event(&decoder, message)).await.is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn diagnostic_event(decoder: &NdjsonDecoder, message: String) -> Event {
    Event {
        id: Uuid::new_v4(),
        session_id: decoder.default_session,
        seq: decoder.next_seq(decoder.default_session),
        timestamp_ms: now_ms(),
        schema_version: SCHEMA_VERSION,
        event_type: EventType::RuntimeError,
        payload: serde_json::json!({
            "message": message,
            "source": "protocol",
            "severity": "warn",
        }),
    }
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
    async fn schema_and_payload_errors_become_visible_warnings() {
        let d = NdjsonDecoder::new(Uuid::nil());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let input = concat!(
            "{\"event_type\":\"SessionStarted\",\"schema_version\":2,\"payload\":{}}\n",
            "{\"event_type\":\"UserMessageCreated\",\"payload\":{\"text\":false}}\n",
            "{\"event_type\":\"SessionEnded\"}\n",
        )
        .as_bytes();
        let reader = BufReader::new(input);
        tokio::spawn(async move { run_reader(reader, d, tx).await.unwrap() });

        let version_warning = rx.recv().await.unwrap();
        assert_eq!(version_warning.event_type, EventType::RuntimeError);
        assert!(version_warning.payload["message"]
            .as_str()
            .unwrap()
            .contains("schema version mismatch"));
        assert_eq!(version_warning.payload["severity"], "warn");

        let payload_warning = rx.recv().await.unwrap();
        assert_eq!(payload_warning.event_type, EventType::RuntimeError);
        assert!(payload_warning.payload["message"]
            .as_str()
            .unwrap()
            .contains("invalid payload for UserMessageCreated"));
        assert_eq!(payload_warning.payload["severity"], "warn");

        assert_eq!(rx.recv().await.unwrap().event_type, EventType::SessionEnded);
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn unknown_event_type_is_reported_once_and_stream_continues() {
        let d = NdjsonDecoder::new(Uuid::nil());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let input = "{\"event_type\":\"FutureMagic\"}\n{\"event_type\":\"FutureMagic\"}\n{\"event_type\":\"SessionEnded\"}\n".as_bytes();
        let reader = BufReader::new(input);
        tokio::spawn(async move { run_reader(reader, d, tx).await.unwrap() });
        let warning = rx.recv().await.unwrap();
        assert_eq!(warning.event_type, EventType::RuntimeError);
        assert!(warning.payload["message"]
            .as_str()
            .unwrap()
            .contains("FutureMagic"));
        assert_eq!(warning.payload["severity"], "warn");
        let next = rx.recv().await.unwrap();
        assert_eq!(next.event_type, EventType::SessionEnded);
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn rejects_mismatched_schema_versions_and_bad_typed_payloads() {
        let decoder = NdjsonDecoder::new(Uuid::nil());
        let mismatch = decoder
            .parse_line(r#"{"event_type":"SessionStarted","schema_version":2,"payload":{}}"#)
            .unwrap_err();
        assert!(matches!(
            mismatch,
            NdjsonError::SchemaVersionMismatch { received: 2, .. }
        ));

        let invalid = decoder
            .parse_line(r#"{"event_type":"UserMessageCreated","payload":{"text":false}}"#)
            .unwrap_err();
        assert!(matches!(
            invalid,
            NdjsonError::InvalidPayload {
                event_type: EventType::UserMessageCreated,
                ..
            }
        ));
    }

    #[test]
    fn accepts_legacy_zero_schema_version() {
        let decoder = NdjsonDecoder::new(Uuid::nil());
        let event = decoder
            .parse_line(r#"{"event_type":"SessionEnded","schema_version":0}"#)
            .unwrap();
        assert_eq!(event.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn seq_is_per_session_when_sessions_interleave() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let d = NdjsonDecoder::new(Uuid::nil());
        let line = |sid: Uuid| {
            format!(r#"{{"event_type":"UserMessageCreated","session_id":"{sid}","payload":{{"text":"hi"}}}}"#)
        };
        let seqs: Vec<(Uuid, i64)> = [a, b, a, b, a]
            .into_iter()
            .map(|sid| {
                let ev = d.parse_line(&line(sid)).unwrap();
                (ev.session_id, ev.seq)
            })
            .collect();
        assert_eq!(seqs, vec![(a, 0), (b, 0), (a, 1), (b, 1), (a, 2)]);
    }

    #[test]
    fn external_seq_only_advances_its_own_session() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let d = NdjsonDecoder::new(Uuid::nil());
        let with_seq = |sid: Uuid, seq: i64| {
            format!(r#"{{"event_type":"SessionEnded","session_id":"{sid}","seq":{seq}}}"#)
        };
        assert_eq!(d.parse_line(&with_seq(a, 41)).unwrap().seq, 41);
        // b's counter is untouched by a's explicit seq.
        let b0 = d
            .parse_line(&format!(r#"{{"event_type":"SessionEnded","session_id":"{b}"}}"#))
            .unwrap();
        assert_eq!(b0.seq, 0);
        // a continues after the explicit seq.
        let a1 = d
            .parse_line(&format!(r#"{{"event_type":"SessionEnded","session_id":"{a}"}}"#))
            .unwrap();
        assert_eq!(a1.seq, 42);
    }

    #[test]
    fn interleaved_fixture_decodes_with_per_session_seq() {
        let body = crate::fixtures::load_fixture("multi-session").expect("fixture exists");
        let d = NdjsonDecoder::new(Uuid::nil());
        let mut per_session: HashMap<Uuid, Vec<i64>> = HashMap::new();
        let mut statuses = 0;
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            let ev = d.parse_line(line).expect("fixture line decodes");
            per_session.entry(ev.session_id).or_default().push(ev.seq);
            if ev.event_type == EventType::SessionStatus {
                statuses += 1;
            }
        }
        assert_eq!(per_session.len(), 2, "fixture should carry two sessions");
        for (sid, seqs) in &per_session {
            assert_eq!(
                seqs,
                &(0..seqs.len() as i64).collect::<Vec<_>>(),
                "session {sid} seqs should be 0..n"
            );
        }
        assert!(statuses >= 2, "fixture should carry SessionStatus events");
    }
}
