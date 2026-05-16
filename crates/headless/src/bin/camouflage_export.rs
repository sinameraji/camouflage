//! `camouflage-export` — dump a stored session to portable NDJSON.
//!
//! Inverse of `camouflage-record`. Reads events from a SQLite store and
//! writes one full Event JSON per line on stdout, preserving order. The
//! output is consumable by `camouflage-tui --stdin-events`,
//! `camouflage-record`, `camouflage-broadcast`, `camouflage-validate`,
//! or any third-party tool — it's just NDJSON.
//!
//! Use cases:
//! - Attach a session to a bug report
//! - Diff two sessions for regression testing
//! - Round-trip a session through `camouflage-validate` after a protocol
//!   change to confirm payload-shape compatibility
//!
//! Usage:
//!     camouflage-export --store run.db --session <uuid>
//!     camouflage-export --store run.db --session <uuid> > session.ndjson
//!     camouflage-export --store run.db --list   # show session ids + counts

use anyhow::{Context, Result};
use camouflage_protocol::Event;
use camouflage_store::{EventStore, SqliteStore};
use clap::Parser;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "camouflage-export", about = "Export a stored session to NDJSON.")]
struct Args {
    /// SQLite store path.
    #[arg(long)]
    store: PathBuf,
    /// Session UUID to export. Required unless --list is set.
    #[arg(long)]
    session: Option<Uuid>,
    /// List session ids + event counts found in the store, then exit.
    #[arg(long)]
    list: bool,
    /// Emit shorthand `{event_type, payload}` lines (omit id/seq/timestamp).
    /// Default is full Event records so the export is self-describing.
    #[arg(long)]
    shorthand: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let store = SqliteStore::open(&args.store).with_context(|| {
        format!("opening store at {}", args.store.display())
    })?;

    if args.list {
        return list_sessions(&store);
    }

    let session = args.session.ok_or_else(|| {
        anyhow::anyhow!("--session <uuid> required (or pass --list to discover ids)")
    })?;
    let events = store.load_session(session).context("loading session")?;
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    for ev in &events {
        let line = if args.shorthand {
            serde_json::to_string(&serde_json::json!({
                "event_type": ev.event_type.as_str(),
                "payload": ev.payload,
            }))?
        } else {
            serde_json::to_string(ev)?
        };
        match writeln!(w, "{line}") {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
    w.flush().ok();
    eprintln!("camouflage-export: {} events from session {}", events.len(), session);
    Ok(())
}

fn list_sessions(store: &SqliteStore) -> Result<()> {
    // SqliteStore doesn't currently expose a "list sessions" method; we open
    // the connection directly via a lightweight helper. To keep the binary
    // dep-light and avoid widening the store API for one call site, we use
    // SqliteStore::open with the same path and run a raw query via a
    // re-open. The store path is small; this is a one-shot tool.
    use rusqlite::Connection;
    let dummy = store as *const _; // suppress unused warning
    let _ = dummy;
    // Re-open read-only to enumerate sessions. SqliteStore guarantees the
    // file exists and the schema is set up.
    let path = std::env::args()
        .position(|a| a == "--store")
        .and_then(|i| std::env::args().nth(i + 1))
        .expect("--store argv re-read");
    let conn = Connection::open(&path).context("re-open for list")?;
    let mut stmt = conn.prepare(
        "SELECT session_id, COUNT(*), MIN(timestamp_ms), MAX(timestamp_ms)
         FROM events GROUP BY session_id ORDER BY MAX(timestamp_ms) DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    println!("session_id                                  events  first_ts        last_ts");
    for row in rows {
        let (sid, n, first, last) = row?;
        println!("{:<44} {:>6}  {:>14}  {:>14}", sid, n, first, last);
    }
    Ok(())
}

// Silence unused-import lint when --list is the only path that uses Event.
#[allow(dead_code)]
fn _unused(_: &Event) {}
