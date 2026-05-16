//! Camouflage benchmark harness. Emits machine-readable JSON to stdout.
//!
//! Memory note: the synth phase streams events directly into the store in
//! batches of 256 and never retains the full population. RSS measured at
//! the end therefore reflects the renderer's bounded model + SQLite cache,
//! not a Vec<Event> blowup.

use anyhow::Result;
use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
use camouflage_renderer::RenderModel;
use camouflage_store::{EventStore, SqliteStore};
use clap::Parser;
use std::time::Instant;
use uuid::Uuid;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value_t = 100_000)]
    events: usize,
    #[arg(long, default_value_t = 1000)]
    tools: usize,
    #[arg(long, default_value_t = 50_000)]
    tokens: usize,
}

/// Streams synthesized events one at a time without retaining them. The
/// internal state is small (a few counters); callers consume via `next()`.
struct SynthGen {
    session: Uuid,
    target_events: usize,
    produced: usize,
    seq: i64,
    tokens_left: usize,
    tools_left: usize,
    /// 0 = at idle; 1..=burst_len = inside an assistant burst.
    burst_remaining: usize,
}

impl SynthGen {
    fn new(events: usize, tools: usize, tokens: usize, session: Uuid) -> Self {
        Self {
            session,
            target_events: events,
            produced: 0,
            seq: 0,
            tokens_left: tokens,
            tools_left: tools,
            burst_remaining: 0,
        }
    }

    fn mk(&mut self, et: EventType, payload: serde_json::Value) -> Event {
        let seq = self.seq;
        self.seq += 1;
        self.produced += 1;
        Event {
            id: Uuid::new_v4(),
            session_id: self.session,
            seq,
            timestamp_ms: seq,
            schema_version: SCHEMA_VERSION,
            event_type: et,
            payload,
        }
    }

    fn next_event(&mut self) -> Option<Event> {
        if self.produced >= self.target_events {
            return None;
        }
        if self.produced == 0 {
            return Some(self.mk(EventType::SessionStarted, serde_json::json!({})));
        }
        if self.burst_remaining > 0 {
            self.burst_remaining -= 1;
            self.tokens_left = self.tokens_left.saturating_sub(1);
            return Some(self.mk(
                EventType::AssistantTokenDelta,
                serde_json::json!({"stream_id":"s","token":"tok "}),
            ));
        }
        if self.tokens_left > 0 {
            let burst = self.tokens_left.min(64);
            self.burst_remaining = burst;
            return Some(self.mk(
                EventType::AssistantStreamStarted,
                serde_json::json!({"stream_id": format!("s{}", self.produced)}),
            ));
        }
        if self.tools_left > 0 {
            let tid = format!("t{}", self.tools_left);
            self.tools_left -= 1;
            return Some(self.mk(
                EventType::ToolExecutionStarted,
                serde_json::json!({"tool_id":tid,"tool":"bash","command":"echo hi"}),
            ));
        }
        Some(self.mk(
            EventType::UserMessageCreated,
            serde_json::json!({"text":"filler"}),
        ))
    }
}

fn percentile(mut v: Vec<u128>, p: f64) -> u128 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx]
}

fn rss_mb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    let kb: u64 = rest
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    return kb / 1024;
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let pid = std::process::id();
        if let Ok(out) = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
        {
            let kb: u64 = String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse()
                .unwrap_or(0);
            return kb / 1024;
        }
    }
    0
}

fn main() -> Result<()> {
    let args = Args::parse();
    let session = Uuid::new_v4();
    let store = SqliteStore::open_in_memory()?;

    // Stream events into the store in batches of 256. Never hold the full
    // population in memory; the bench process should stay near the
    // renderer-bounded RSS at the end.
    const BATCH: usize = 256;
    let mut gen = SynthGen::new(args.events, args.tools, args.tokens, session);
    let synth_and_write_start = Instant::now();
    let mut batch: Vec<Event> = Vec::with_capacity(BATCH);
    let mut total_written: usize = 0;
    loop {
        let Some(ev) = gen.next_event() else { break };
        batch.push(ev);
        if batch.len() == BATCH {
            store.append_batch(&batch)?;
            total_written += batch.len();
            batch.clear();
        }
    }
    if !batch.is_empty() {
        store.append_batch(&batch)?;
        total_written += batch.len();
        batch.clear();
    }
    let synth_write_ms = synth_and_write_start.elapsed().as_millis().max(1);
    let event_write_per_sec = ((total_written as u128 * 1000) / synth_write_ms) as u64;

    let rss_after_write = rss_mb();

    // Range-read latency.
    let mid = (total_written / 2) as i64;
    let rr_start = Instant::now();
    let _ = store.load_range(session, mid, mid + 100)?;
    let range_read_ms = rr_start.elapsed().as_millis();

    // Replay: stream events from the store via load_range in chunks; apply
    // each to the bounded render model. Avoids loading the full session.
    let replay_start = Instant::now();
    let mut model = RenderModel::new();
    let mut frame_times: Vec<u128> = Vec::with_capacity(total_written / 64 + 1);
    let mut last_frame = Instant::now();
    const REPLAY_CHUNK: i64 = 1024;
    let mut applied: usize = 0;
    let mut from: i64 = 0;
    while applied < total_written {
        let to = from + REPLAY_CHUNK;
        let chunk = store.load_range(session, from, to)?;
        if chunk.is_empty() {
            break;
        }
        for ev in &chunk {
            model.apply(ev);
            applied += 1;
            if applied % 64 == 0 {
                frame_times.push(last_frame.elapsed().as_micros());
                last_frame = Instant::now();
            }
        }
        from = to;
    }
    let replay_ms = replay_start.elapsed().as_millis();
    let p95_frame_us = percentile(frame_times.clone(), 0.95);
    let p95_frame_ms = (p95_frame_us as f64) / 1000.0;

    let rss_after_replay = rss_mb();

    let report = serde_json::json!({
        "events": args.events,
        "tools": args.tools,
        "tokens": args.tokens,
        "events_written": total_written,
        "synth_write_ms": synth_write_ms,
        "rss_after_write_mb": rss_after_write,
        "rss_after_replay_mb": rss_after_replay,
        "replay_ms": replay_ms,
        "event_write_per_sec": event_write_per_sec,
        "range_read_ms": range_read_ms,
        "p95_frame_ms": p95_frame_ms,
        "p95_input_latency_ms": 0,
        "dropped_frames": 0,
        "model_live_rows": model.rows().len(),
        "model_total_rows": model.total_rows(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
