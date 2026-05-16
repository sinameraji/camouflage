mod app;
mod draw;
mod input;
mod tty;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "camouflage-tui", about = "Camouflage event-native TUI")]
struct Args {
    /// Read newline-delimited JSON events from stdin.
    #[arg(long)]
    stdin_events: bool,

    /// Replay an existing session from the SQLite store.
    #[arg(long, value_name = "SESSION_ID")]
    replay: Option<Uuid>,

    /// Path to SQLite database. Defaults to $HOME/.camouflage/sessions.db.
    #[arg(long)]
    db: Option<PathBuf>,

    /// Target frame rate (frames per second).
    #[arg(long, default_value_t = 60)]
    fps: u32,

    /// Override the live-buffer row cap (default 2000). Lower values force
    /// history paging earlier — useful for testing.
    #[arg(long)]
    row_cap: Option<usize>,

    /// Emit outbound NDJSON events (UserInputSubmitted, PermissionResponse)
    /// to stdout. Required by hosts that consume user actions back from the
    /// renderer (e.g. KimiFlare-style adapters). Defaults to true when
    /// --stdin-events is set.
    #[arg(long)]
    emit_responses: Option<bool>,
}

fn default_db_path() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".camouflage").join("sessions.db")
    } else {
        PathBuf::from("camouflage.db")
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let db_path = args.db.unwrap_or_else(default_db_path);

    let store =
        camouflage_store::SqliteStore::open(&db_path).context("opening event store")?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let emit_responses = args.emit_responses.unwrap_or(args.stdin_events);

    rt.block_on(app::run(app::Config {
        store,
        stdin_events: args.stdin_events,
        replay: args.replay,
        fps: args.fps.max(1).min(120),
        row_cap: args.row_cap,
        emit_responses,
    }))
}
