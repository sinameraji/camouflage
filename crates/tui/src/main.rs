mod inline_app;
mod platform;
mod persist;
mod replay;
mod transcript;

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

    /// Replay an existing session from the SQLite store. The transcript is
    /// printed through the inline renderer and the process exits.
    #[arg(long, value_name = "SESSION_ID", conflicts_with = "play")]
    replay: Option<Uuid>,

    /// Play back a shareable .camo file (header line + NDJSON events).
    /// The file may also be plain NDJSON without a header. The transcript is
    /// printed through the inline renderer and the process exits.
    #[arg(long, value_name = "FILE", conflicts_with = "replay")]
    play: Option<PathBuf>,

    /// Path to SQLite database. Defaults to $HOME/.camouflage/sessions.db.
    #[arg(long)]
    db: Option<PathBuf>,

    /// Don't save sessions at all (for hosts that keep their own history).
    #[arg(long)]
    no_store: bool,

    /// Delete saved sessions idle for longer than this many days, at
    /// startup. 0 keeps everything. Also CAMOUFLAGE_RETENTION_DAYS.
    #[arg(long, value_name = "DAYS", env = "CAMOUFLAGE_RETENTION_DAYS", default_value_t = 30)]
    retention_days: u32,

    /// Deprecated: the inline renderer has no frame ticker. Accepted and
    /// ignored so older hosts and scripts keep working.
    #[arg(long, hide = true)]
    fps: Option<u32>,

    /// Deprecated: the inline renderer keeps no row buffer. Accepted and
    /// ignored so older hosts and scripts keep working.
    #[arg(long, hide = true)]
    row_cap: Option<usize>,

    /// Emit outbound NDJSON events (UserInputSubmitted, PermissionResponse)
    /// to stdout. Required by hosts that consume user actions back from the
    /// renderer (host-style adapters). Defaults to true when
    /// --stdin-events is set, false otherwise. Mutually exclusive with
    /// --responses-fd.
    /// A bare `--emit-responses` means true (the Node SDK has always passed
    /// it that way).
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    emit_responses: Option<bool>,

    /// Write outbound NDJSON events to the given pre-opened file descriptor
    /// instead of stdout. Use when stdout is needed for rendering (e.g. when
    /// the renderer is a child of a host that wants both: TUI on stdout to
    /// the user's terminal, control events on fd 3). The host is responsible
    /// for opening the fd before spawning. Mutually exclusive with
    /// --emit-responses=true.
    #[arg(long, value_name = "FD")]
    responses_fd: Option<i32>,

    /// Brand shown in the header. Hosts that embed the renderer should pass
    /// their own app name here (the Node SDK does this automatically). When
    /// omitted, falls back to the working-directory folder name. The
    /// renderer never brands itself as "Camouflage".
    #[arg(long, value_name = "NAME")]
    app_title: Option<String>,

    /// Rendering mode. `inline` is the only mode; the old full-screen UI was
    /// removed. The flag is kept so hosts that pass `--ui inline` keep
    /// working, and `--ui fullscreen` fails with a clear message.
    #[arg(long, value_enum, default_value_t = UiMode::Inline)]
    ui: UiMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_responses_accepts_a_bare_flag() {
        let a = Args::try_parse_from(["camouflage-tui", "--stdin-events", "--emit-responses"]).unwrap();
        assert_eq!(a.emit_responses, Some(true));
        let b = Args::try_parse_from(["camouflage-tui", "--emit-responses=false"]).unwrap();
        assert_eq!(b.emit_responses, Some(false));
        let c = Args::try_parse_from(["camouflage-tui", "--emit-responses", "--ui", "inline"]).unwrap();
        assert_eq!((c.emit_responses, c.ui), (Some(true), UiMode::Inline));
    }

    #[test]
    fn inline_is_the_default_mode() {
        assert_eq!(Args::try_parse_from(["camouflage-tui"]).unwrap().ui, UiMode::Inline);
        assert_eq!(Args::try_parse_from(["camouflage-tui", "--ui", "fullscreen"]).unwrap().ui, UiMode::Fullscreen);
    }

    #[test]
    fn replay_and_play_are_mutually_exclusive() {
        let err = Args::try_parse_from([
            "camouflage-tui",
            "--replay",
            "00000000-0000-0000-0000-000000000000",
            "--play",
            "x.camo",
        ])
        .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn deprecated_flags_still_parse() {
        let a = Args::try_parse_from(["camouflage-tui", "--fps", "30", "--row-cap", "2000"]).unwrap();
        assert_eq!(a.fps, Some(30));
        assert_eq!(a.row_cap, Some(2000));
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum UiMode {
    Inline,
    Fullscreen,
}

/// Resolve the header brand: explicit `--app-title`, else the basename of
/// the current working directory. Empty string if neither is available
/// (the header then shows the session id alone).
fn resolve_app_title(flag: Option<String>) -> String {
    if let Some(t) = flag {
        let t = t.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

fn default_db_path() -> PathBuf {
    if let Some(home) = platform::home_dir() {
        home.join(".camouflage").join("sessions.db")
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
    if args.ui == UiMode::Fullscreen {
        anyhow::bail!(
            "--ui fullscreen was removed; the inline renderer is the only UI now. \
             Use --ui inline (the default), or --replay/--play to print a stored transcript."
        );
    }
    let db_path = args.db.unwrap_or_else(default_db_path);

    let store = if args.no_store {
        camouflage_store::SqliteStore::open_in_memory().context("opening in-memory event store")?
    } else {
        camouflage_store::SqliteStore::open(&db_path).context("opening event store")?
    };

    // Replay and play are non-interactive: render the transcript through the
    // inline block renderer and exit. They never touch the live loop.
    if args.replay.is_some() || args.play.is_some() {
        if args.no_store && args.replay.is_some() {
            anyhow::bail!("--replay needs the session store; drop --no-store");
        }
        let (w, h) = inline_app::terminal_size();
        let cfg = replay::ReplayConfig {
            app_title: resolve_app_title(args.app_title),
            theme: inline_app::detect_theme(),
            width: (w as usize).saturating_sub(1),
            height: h as usize,
        };
        let lines = match (args.replay, args.play) {
            (Some(sid), _) => replay::replay_session(&store, sid, &cfg)?,
            (_, Some(path)) => replay::replay_file(&path, &cfg)?,
            _ => unreachable!(),
        };
        let mut out = std::io::stdout();
        replay::write_transcript(&mut out, &lines, cfg.width)?;
        return Ok(());
    }

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

    let result = rt.block_on(inline_app::run(inline_app::InlineConfig {
        // Inline mode never reads the store back; with --no-store it
        // keeps nothing at all (not even in memory).
        store: (!args.no_store).then_some(store),
        retention_days: Some(args.retention_days),
        stdin_events: args.stdin_events,
        emit_responses,
        responses_fd: args.responses_fd,
        app_title: resolve_app_title(args.app_title),
    }));
    // Don't let `rt`'s Drop block on the spawn_blocking thread that owns
    // `tokio::io::stdin()`. That read() is uninterruptible from userspace,
    // so a clean teardown would otherwise hang until the host closes the
    // pipe — which manifests as the user having to press Ctrl+C a third
    // time (now SIGINT, since raw mode is already off) to actually exit.
    rt.shutdown_background();
    result
}
