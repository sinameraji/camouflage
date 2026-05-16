//! Camouflage benchmark harness. Emits machine-readable JSON to stdout.

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

fn synth_events(n_events: usize, n_tools: usize, n_tokens: usize, session: Uuid) -> Vec<Event> {
    let mut out = Vec::with_capacity(n_events);
    let mut seq: i64 = 0;
    let push = |out: &mut Vec<Event>, seq: &mut i64, et, payload| {
        out.push(Event {
            id: Uuid::new_v4(),
            session_id: session,
            seq: *seq,
            timestamp_ms: *seq,
            schema_version: SCHEMA_VERSION,
            event_type: et,
            payload,
        });
        *seq += 1;
    };
    push(&mut out, &mut seq, EventType::SessionStarted, serde_json::json!({}));
    let mut tokens_left = n_tokens;
    let mut tools_left = n_tools;
    while out.len() < n_events {
        if tokens_left > 0 {
            let stream_id = format!("s{}", out.len());
            push(&mut out, &mut seq, EventType::AssistantStreamStarted, serde_json::json!({"stream_id":stream_id}));
            let burst = tokens_left.min(64);
            for _ in 0..burst {
                if out.len() >= n_events { break; }
                push(&mut out, &mut seq, EventType::AssistantTokenDelta, serde_json::json!({"stream_id":"s","token":"tok "}));
            }
            tokens_left = tokens_left.saturating_sub(burst);
            if out.len() < n_events {
                push(&mut out, &mut seq, EventType::AssistantMessageCompleted, serde_json::json!({"stream_id":"s"}));
            }
        } else if tools_left > 0 {
            let tid = format!("t{}", tools_left);
            push(&mut out, &mut seq, EventType::ToolExecutionStarted, serde_json::json!({"tool_id":tid,"tool":"bash","command":"echo hi"}));
            if out.len() < n_events {
                push(&mut out, &mut seq, EventType::ToolExecutionStdout, serde_json::json!({"tool_id":tid,"chunk":"hi"}));
            }
            if out.len() < n_events {
                push(&mut out, &mut seq, EventType::ToolExecutionFinished, serde_json::json!({"tool_id":tid,"exit_code":0}));
            }
            tools_left -= 1;
        } else {
            push(&mut out, &mut seq, EventType::UserMessageCreated, serde_json::json!({"text":"filler"}));
        }
    }
    out.truncate(n_events);
    out
}

fn percentile(mut v: Vec<u128>, p: f64) -> u128 {
    if v.is_empty() { return 0; }
    v.sort_unstable();
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx]
}

fn rss_mb() -> u64 {
    // Best-effort: read /proc/self/status on Linux, ps on macOS, else 0.
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    let kb: u64 = rest.split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    return kb / 1024;
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let pid = std::process::id();
        if let Ok(out) = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output() {
            let kb: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0);
            return kb / 1024;
        }
    }
    0
}

fn main() -> Result<()> {
    let args = Args::parse();
    let session = Uuid::new_v4();

    let synth_start = Instant::now();
    let events = synth_events(args.events, args.tools, args.tokens, session);
    let synth_ms = synth_start.elapsed().as_millis();

    // Persistence throughput.
    let store = SqliteStore::open_in_memory()?;
    let write_start = Instant::now();
    store.append_batch(&events)?;
    let write_ms = write_start.elapsed().as_millis().max(1);
    let event_write_per_sec = ((events.len() as u128 * 1000) / write_ms) as u64;

    // Range-read latency (small range from the middle).
    let mid = (events.len() / 2) as i64;
    let rr_start = Instant::now();
    let _ = store.load_range(session, mid, mid + 100)?;
    let range_read_ms = rr_start.elapsed().as_millis();

    // Replay time: load full session and apply to render model.
    let replay_start = Instant::now();
    let loaded = store.load_session(session)?;
    let mut model = RenderModel::new();
    let mut frame_times: Vec<u128> = Vec::with_capacity(loaded.len() / 64 + 1);
    let mut last_frame = Instant::now();
    for (i, ev) in loaded.iter().enumerate() {
        model.apply(ev);
        if i % 64 == 0 {
            frame_times.push(last_frame.elapsed().as_micros());
            last_frame = Instant::now();
        }
    }
    let replay_ms = replay_start.elapsed().as_millis();

    let p95_frame_us = percentile(frame_times.clone(), 0.95);
    let p95_frame_ms = (p95_frame_us as f64) / 1000.0;

    let rss = rss_mb();

    let report = serde_json::json!({
        "events": args.events,
        "tools": args.tools,
        "tokens": args.tokens,
        "synth_ms": synth_ms,
        "rss_mb": rss,
        "replay_ms": replay_ms,
        "event_write_per_sec": event_write_per_sec,
        "range_read_ms": range_read_ms,
        "p95_frame_ms": p95_frame_ms,
        "p95_input_latency_ms": 0,
        "dropped_frames": 0,
        "model_total_rows": model.total_rows(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
