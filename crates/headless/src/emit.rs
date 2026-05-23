//! Outbound NDJSON emitter.
//!
//! The renderer is bidirectional: most events flow host → renderer, but
//! [`UserInputSubmitted`] and [`PermissionResponse`] flow renderer → host.
//! This module wraps a [`tokio::sync::mpsc::Sender<OutgoingEvent>`] and
//! provides a writer task that flushes pending events to a `Write`
//! destination (stdout by default).

use camouflage_protocol::Event;
use std::io::{BufWriter, Write};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration, MissedTickBehavior};

#[derive(Debug, Clone)]
pub struct OutgoingEvent(pub Event);

/// Spawn a writer task that drains `rx` and serialises each event as one
/// NDJSON line to `writer`. Flushes every 16 ms so a busy host gets prompt
/// delivery of key responses (permission, input submit).
pub fn spawn_writer<W>(writer: W, mut rx: mpsc::Receiver<OutgoingEvent>)
where
    W: Write + Send + 'static,
{
    let writer = Arc::new(std::sync::Mutex::new(BufWriter::new(writer)));
    let w = writer.clone();
    tokio::spawn(async move {
        let mut flush = interval(Duration::from_millis(16));
        flush.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                maybe = rx.recv() => {
                    match maybe {
                        Some(OutgoingEvent(ev)) => {
                            let event_name = ev.event_type.as_str();
                            if let Ok(s) = serde_json::to_string(&ev) {
                                if let Ok(mut g) = w.lock() {
                                    let _ = writeln!(*g, "{}", s);
                                }
                            }
                            // drain any queued additional events synchronously
                            while let Ok(OutgoingEvent(ev2)) = rx.try_recv() {
                                if let Ok(s) = serde_json::to_string(&ev2) {
                                    if let Ok(mut g) = w.lock() {
                                        let _ = writeln!(*g, "{}", s);
                                    }
                                }
                            }
                            // Flush immediately — Esc/Ctrl+C emit
                            // CancelRequested and the host needs it
                            // *now*, not in up to 16ms. The previous
                            // tick-only flush was the difference between
                            // an Esc that aborted in 5ms vs one that
                            // looked dead for a full frame.
                            let flush_res = if let Ok(mut g) = w.lock() {
                                g.flush()
                            } else {
                                Ok(())
                            };
                            if std::env::var_os("CAMOUFLAGE_DEBUG_ESC").is_some() {
                                eprintln!(
                                    "[camouflage:esc] writer flushed {} (flush={})",
                                    event_name,
                                    if flush_res.is_ok() { "ok" } else { "err" }
                                );
                            }
                        }
                        None => break,
                    }
                }
                _ = flush.tick() => {
                    if let Ok(mut g) = w.lock() {
                        let _ = g.flush();
                    }
                }
            }
        }
        if let Ok(mut g) = w.lock() {
            let _ = g.flush();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use camouflage_protocol::{EventType, SCHEMA_VERSION};
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    /// In-memory writer used as the test sink. Implements Write by appending
    /// to a shared Vec we can inspect after the writer task drains.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }

    #[tokio::test]
    async fn writer_emits_ndjson() {
        let sink = Sink::default();
        let captured = sink.0.clone();
        let (tx, rx) = mpsc::channel::<OutgoingEvent>(4);
        spawn_writer(sink, rx);
        let ev = Event {
            id: Uuid::nil(),
            session_id: Uuid::nil(),
            seq: 0,
            timestamp_ms: 0,
            schema_version: SCHEMA_VERSION,
            event_type: EventType::UserInputSubmitted,
            payload: serde_json::json!({"text": "hi"}),
        };
        tx.send(OutgoingEvent(ev)).await.unwrap();
        drop(tx);
        // Allow the writer task to drain.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let out = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(out.contains("UserInputSubmitted"), "got: {out}");
        assert!(out.contains(r#""text":"hi""#));
        assert!(out.ends_with('\n'));
    }
}
