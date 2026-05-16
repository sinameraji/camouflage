//! KimiFlare-style event generator.
//!
//! Emits NDJSON that exercises the v0.1.5 surface area in roughly the order a
//! real coding-agent harness would:
//!
//! - StatusUpdate segments for mode / phase / elapsed / tokens / cost / branch
//! - BackgroundTaskUpdate "skills indexing" running → done
//! - AssistantStream with spinner pre-token, then streaming, then completion
//! - ToolExecution with mid-execution stdout, then completion
//! - PermissionRequested mid-stream (so the inline widget renders)
//! - RuntimeError with kind=quota_exhausted and a CTA
//!
//! Use:
//!
//!     cargo run --release -p kimiflare-mock | \
//!         cargo run --release -p camouflage-tui -- --stdin-events
//!
//! The mock pauses between events so a human can see the transitions; pass
//! `--fast` for an unpaced burst (useful in pty harnesses).

use anyhow::Result;
use clap::Parser;
use std::io::{BufWriter, Write};
use std::thread;
use std::time::Duration;

#[derive(Parser, Debug)]
struct Args {
    /// Emit as fast as possible (no inter-event sleeps).
    #[arg(long)]
    fast: bool,
    /// Skip the permission-requested step.
    #[arg(long)]
    no_permission: bool,
    /// Skip the error step.
    #[arg(long)]
    no_error: bool,
}

fn emit<W: Write>(w: &mut W, line: &serde_json::Value) -> Result<bool> {
    let s = serde_json::to_string(line)?;
    match writeln!(w, "{}", s) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(false),
        Err(e) => Err(e.into()),
    }
}

macro_rules! emit_or_break {
    ($w:expr, $val:expr) => {
        if !emit($w, &$val)? { return Ok(()); }
    };
}

fn main() -> Result<()> {
    let args = Args::parse();
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    let pause = if args.fast { Duration::ZERO } else { Duration::from_millis(120) };
    let long_pause = if args.fast { Duration::ZERO } else { Duration::from_millis(700) };

    // 1. Session boot + base status.
    emit_or_break!(&mut w, serde_json::json!({"event_type":"SessionStarted","payload":{}}));
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"StatusUpdate",
        "payload":{"segments":{
            "mode":"edit","phase":"idle","elapsed":"0s",
            "tokens":"in 8k","cost":"$0.00","branch":"main"
        }}
    }));
    w.flush().ok();
    thread::sleep(long_pause);

    // 2. Background task ribbon: skills indexing.
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"BackgroundTaskUpdate",
        "payload":{"task_id":"skills","label":"indexing skills","state":"running","progress":0.0}
    }));
    for i in 1..=4 {
        emit_or_break!(&mut w, serde_json::json!({
            "event_type":"BackgroundTaskUpdate",
            "payload":{"task_id":"skills","label":"indexing skills","state":"running","progress": i as f32 * 0.25 }
        }));
        w.flush().ok();
        thread::sleep(pause);
    }
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"BackgroundTaskUpdate",
        "payload":{"task_id":"skills","label":"indexing skills","state":"done"}
    }));
    thread::sleep(pause);

    // 3. User message.
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"UserMessageCreated",
        "payload":{"text":"investigate failing test in src/auth/login.ts"}
    }));
    thread::sleep(pause);

    // 4. Assistant thinking → streaming. Phase update first, then the
    //    AssistantStreamStarted is emitted before any tokens (spinner state).
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"StatusUpdate","payload":{"segments":{"phase":"thinking","elapsed":"3s"}}
    }));
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"AssistantStreamStarted","payload":{"stream_id":"s1"}
    }));
    w.flush().ok();
    thread::sleep(long_pause); // Hold the spinner state briefly.
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"StatusUpdate","payload":{"segments":{"phase":"streaming"}}
    }));
    for tok in [
        "Looking ", "at ", "the ", "auth flow. ", "I'll ", "run ", "the ",
        "failing ", "test ", "to ", "see ", "the ", "exact ", "error.\n",
    ] {
        emit_or_break!(&mut w, serde_json::json!({
            "event_type":"AssistantTokenDelta","payload":{"stream_id":"s1","token":tok}
        }));
        w.flush().ok();
        thread::sleep(pause / 2);
    }
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"AssistantMessageCompleted","payload":{"stream_id":"s1"}
    }));

    // 5. Tool execution: a single bash run with stdout streaming.
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"StatusUpdate","payload":{"segments":{"phase":"tool"}}
    }));
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"ToolExecutionStarted",
        "payload":{"tool_id":"t1","tool":"bash","command":"npm test src/auth/login.test.ts"}
    }));
    w.flush().ok();
    thread::sleep(long_pause); // Hold the spinner state briefly.
    for chunk in ["FAIL ", "src/auth/login.test.ts\n", "Expected 200, got 401\n"] {
        emit_or_break!(&mut w, serde_json::json!({
            "event_type":"ToolExecutionStdout","payload":{"tool_id":"t1","chunk":chunk}
        }));
        w.flush().ok();
        thread::sleep(pause / 2);
    }
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"ToolExecutionFinished","payload":{"tool_id":"t1","exit_code":1}
    }));

    // 6. PermissionRequested mid-flow (renders the inline widget).
    if !args.no_permission {
        emit_or_break!(&mut w, serde_json::json!({
            "event_type":"StatusUpdate","payload":{"segments":{"phase":"thinking"}}
        }));
        emit_or_break!(&mut w, serde_json::json!({
            "event_type":"PermissionRequested",
            "payload":{
                "request_id":"perm-1",
                "tool":"edit",
                "action":"apply patch to src/auth/login.ts (3 lines changed)",
                "detail":"changes the token-validation branch"
            }
        }));
        w.flush().ok();
        // Hold the modal on screen so a human can interact, but don't block
        // forever — sleep a short window then auto-resolve.
        thread::sleep(if args.fast { Duration::ZERO } else { Duration::from_secs(4) });
        emit_or_break!(&mut w, serde_json::json!({
            "event_type":"PermissionGranted","payload":{"request_id":"perm-1"}
        }));
    }

    // 7. Quota error with CTA.
    if !args.no_error {
        emit_or_break!(&mut w, serde_json::json!({
            "event_type":"RuntimeError",
            "payload":{
                "message":"daily token budget exhausted",
                "source":"openai",
                "kind":"quota_exhausted",
                "severity":"error",
                "cta":{"label":"type /report to file a ticket","action_id":"report"}
            }
        }));
    }

    // 8. Final idle status.
    emit_or_break!(&mut w, serde_json::json!({
        "event_type":"StatusUpdate",
        "payload":{"segments":{
            "phase":"idle","elapsed":"42s","tokens":"in 14k (2k cached)",
            "cost":"$0.04","warn":"ctx 82% — /compact recommended"
        }}
    }));
    emit_or_break!(&mut w, serde_json::json!({"event_type":"SessionEnded","payload":{}}));
    let _ = w.flush();
    Ok(())
}
