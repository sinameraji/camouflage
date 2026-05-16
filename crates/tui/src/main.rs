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
    /// --stdin-events is set, false otherwise. Mutually exclusive with
    /// --responses-fd.
    #[arg(long)]
    emit_responses: Option<bool>,

    /// Write outbound NDJSON events to the given pre-opened file descriptor
    /// instead of stdout. Use when stdout is needed for rendering (e.g. when
    /// the renderer is a child of a host that wants both: TUI on stdout to
    /// the user's terminal, control events on fd 3). The host is responsible
    /// for opening the fd before spawning. Mutually exclusive with
    /// --emit-responses=true.
    #[arg(long, value_name = "FD")]
    responses_fd: Option<i32>,
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
    // Resolve where outbound events should land:
    //   --responses-fd N        → write to fd N (host's choice)
    //   --emit-responses=true   → write to stdout
    //   --emit-responses=false  → don't emit at all
    //   default (with --stdin-events) → write to stdout
    //   default (no flags)      → don't emit
    let emit_responses_default = args.stdin_events;
    let emit_responses = args.emit_responses.unwrap_or(emit_responses_default);
    if args.responses_fd.is_some() && args.emit_responses == Some(true) {
        anyhow::bail!("--responses-fd and --emit-responses=true are mutually exclusive");
    }

    rt.block_on(app::run(app::Config {
        store,
        stdin_events: args.stdin_events,
        replay: args.replay,
        fps: args.fps.max(1).min(120),
        row_cap: args.row_cap,
        emit_responses,
        responses_fd: args.responses_fd,
    }))
}
