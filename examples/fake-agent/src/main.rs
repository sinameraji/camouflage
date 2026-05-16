//! Emits a simulated long-running AI agent session as NDJSON on stdout.

use anyhow::Result;
use clap::Parser;
use std::io::{BufWriter, Write};
use std::thread;
use std::time::Duration;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value_t = 50_000)]
    tokens: usize,
    #[arg(long, default_value_t = 200)]
    tools: usize,
    /// Approximate session duration in seconds (paces output).
    #[arg(long)]
    duration: Option<u64>,
    /// Deterministic seed (used to shape the event mix).
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Emit as fast as possible (no sleeps).
    #[arg(long)]
    fast: bool,
}

/// Returns `Ok(false)` if the consumer closed the pipe (EPIPE) — caller should stop cleanly.
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

    let total = args.tokens + args.tools * 3 + 10;
    let sleep = if args.fast {
        Duration::ZERO
    } else if let Some(secs) = args.duration {
        Duration::from_nanos(((secs as u128 * 1_000_000_000) / total as u128) as u64)
    } else {
        Duration::from_micros(50)
    };

    let mut rng = args.seed;
    let mut next = || {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        rng
    };

    emit_or_break!(&mut w, serde_json::json!({"event_type":"SessionStarted","payload":{}}));
    emit_or_break!(&mut w, serde_json::json!({"event_type":"UserMessageCreated","payload":{"text":"investigate failing test in auth flow"}}));

    let mut tokens_left = args.tokens;
    let mut tools_left = args.tools;
    let mut stream_n = 0u64;
    let mut tool_n = 0u64;

    while tokens_left > 0 || tools_left > 0 {
        let pick = next() % 5;
        if pick < 3 && tokens_left > 0 {
            stream_n += 1;
            let sid = format!("s{}", stream_n);
            emit_or_break!(&mut w, serde_json::json!({
                "event_type":"AssistantStreamStarted","payload":{"stream_id":sid}
            }));
            let burst = (next() as usize % 200 + 50).min(tokens_left);
            for i in 0..burst {
                emit_or_break!(&mut w, serde_json::json!({
                    "event_type":"AssistantTokenDelta",
                    "payload":{"stream_id":sid,"token":if i % 7 == 6 {"\n"} else {"tok "}}
                }));
                if !sleep.is_zero() && i % 64 == 0 { thread::sleep(sleep); }
            }
            emit_or_break!(&mut w, serde_json::json!({
                "event_type":"AssistantMessageCompleted","payload":{"stream_id":sid}
            }));
            tokens_left -= burst;
        } else if tools_left > 0 {
            tool_n += 1;
            let tid = format!("t{}", tool_n);
            emit_or_break!(&mut w, serde_json::json!({
                "event_type":"ToolExecutionStarted",
                "payload":{"tool_id":tid,"tool":"bash","command":"npm test --silent"}
            }));
            let chunks = (next() as usize % 20) + 1;
            for _ in 0..chunks {
                emit_or_break!(&mut w, serde_json::json!({
                    "event_type":"ToolExecutionStdout",
                    "payload":{"tool_id":tid,"chunk":"ok\n"}
                }));
            }
            emit_or_break!(&mut w, serde_json::json!({
                "event_type":"ToolExecutionFinished",
                "payload":{"tool_id":tid,"exit_code":0}
            }));
            tools_left -= 1;
        }
        if !sleep.is_zero() { thread::sleep(sleep); }
    }

    emit_or_break!(&mut w, serde_json::json!({"event_type":"SessionEnded","payload":{}}));
    if let Err(e) = w.flush() {
        if e.kind() != std::io::ErrorKind::BrokenPipe { return Err(e.into()); }
    }
    Ok(())
}
