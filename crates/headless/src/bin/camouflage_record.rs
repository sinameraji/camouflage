//! `camouflage-record` — headless runtime (persist NDJSON, no rendering).
//!
//! Reads Camouflage NDJSON from stdin and persists every event into a SQLite
//! store via the same `EventStore::append_batch` path the TUI uses. No
//! terminal output; useful for:
//!
//! - capturing a session for later replay by `camouflage-tui --load <db>`
//!   (when that path lands in a future slice)
//! - running an agent under CI where you want the event log but no UI
//! - feeding multiple downstream consumers off one SQLite file
//!
//! Pipeline:
//!     kimiflare --emit-events -p "..." | camouflage-record --store run.db
//!
//! Per the architecture invariant "persist before render/broadcast", this is
//! the persistence half of the pipeline with the render half removed.

use anyhow::{Context, Result};
use camouflage_headless::{run_reader, NdjsonDecoder};
use camouflage_protocol::Event;
use camouflage_store::{EventStore, SqliteStore};
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::BufReader;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "camouflage-record", about = "Persist Camouflage NDJSON to a SQLite store (no UI).")]
struct Args {
    /// SQLite store path. Created if it does not exist.
    #[arg(long)]
    store: PathBuf,
    /// Override the default session id (random UUID) used for events that
    /// don't carry one. Useful when correlating multiple stdin sources into
    /// one session.
    #[arg(long)]
    session_id: Option<Uuid>,
    /// Flush every N events. Larger = faster, smaller = more durable on crash.
    #[arg(long, default_value_t = 64)]
    batch_size: usize,
    /// Print a one-line summary on stderr at end of stream.
    #[arg(long)]
    summary: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let session_id = args.session_id.unwrap_or_else(Uuid::new_v4);
    let store = Arc::new(SqliteStore::open(&args.store).with_context(|| {
        format!("opening store at {}", args.store.display())
    })?);

    let (tx, mut rx) = mpsc::channel::<Event>(1024);
    let decoder = NdjsonDecoder::new(session_id);
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);

    let reader_task = tokio::spawn(async move {
        let _ = run_reader(reader, decoder, tx).await;
    });

    let store_writer = store.clone();
    let mut total: u64 = 0;
    let mut batch: Vec<Event> = Vec::with_capacity(args.batch_size);

    // Drain the channel with a small periodic flush so partial batches don't
    // sit indefinitely on a slow producer.
    let mut flush_tick = tokio::time::interval(Duration::from_millis(50));
    flush_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            ev = rx.recv() => match ev {
                Some(e) => {
                    batch.push(e);
                    total += 1;
                    if batch.len() >= args.batch_size {
                        let to_write = std::mem::take(&mut batch);
                        let s = store_writer.clone();
                        tokio::task::spawn_blocking(move || s.append_batch(&to_write))
                            .await
                            .expect("blocking task")
                            .context("append_batch")?;
                    }
                }
                None => break,
            },
            _ = flush_tick.tick() => {
                if !batch.is_empty() {
                    let to_write = std::mem::take(&mut batch);
                    let s = store_writer.clone();
                    tokio::task::spawn_blocking(move || s.append_batch(&to_write))
                        .await
                        .expect("blocking task")
                        .context("append_batch")?;
                }
            }
        }
    }
    if !batch.is_empty() {
        let s = store_writer.clone();
        tokio::task::spawn_blocking(move || s.append_batch(&batch))
            .await
            .expect("blocking task")
            .context("final append_batch")?;
    }
    let _ = reader_task.await;

    if args.summary {
        let last_seq = store
            .latest_seq(session_id)
            .unwrap_or(-1);
        eprintln!(
            "camouflage-record: session={} events={} last_seq={} store={}",
            session_id,
            total,
            last_seq,
            args.store.display()
        );
    }
    Ok(())
}
