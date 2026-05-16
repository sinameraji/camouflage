mod app;
mod draw;
mod input;

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
}

fn default_db_path() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".camouflage").join("sessions.db")
    } else {
        PathBuf::from("camouflage.db")
    }
}

#[tokio::main]
async fn main() -> Result<()> {
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

    app::run(app::Config {
        store,
        stdin_events: args.stdin_events,
        replay: args.replay,
        fps: args.fps.max(1).min(120),
    })
    .await
}
