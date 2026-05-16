//! `camouflage-replay-check` — deterministic replay for CI.
//!
//! Reads an NDJSON event stream (file or stdin), applies every event to a
//! fresh RenderModel, and either:
//!
//! 1. Prints the resulting Snapshot as JSON on stdout (no golden given), or
//! 2. Compares it byte-for-byte to a `--golden <path>` file and exits non-zero
//!    on mismatch.
//!
//! Pipeline:
//!     camouflage-replay-check session.ndjson > golden.json     # capture
//!     camouflage-replay-check session.ndjson --golden golden.json   # check
//!     cat session.ndjson | camouflage-replay-check --golden golden.json
//!
//! Use cases:
//! - Catch regressions in a host adapter (the renderer output for a known
//!   input must not drift between adapter versions).
//! - Pin a renderer behaviour for a v1.0 release.
//! - Diff two captured sessions on the rendered-state level rather than the
//!   per-event level.

use anyhow::{Context, Result};
use camouflage_renderer::golden::{snapshot_of_ndjson, snapshot_to_golden_json};
use clap::Parser;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "camouflage-replay-check",
    about = "Apply NDJSON to RenderModel and either dump or check the resulting Snapshot."
)]
struct Args {
    /// NDJSON input file. If omitted, reads from stdin.
    input: Option<PathBuf>,
    /// Golden snapshot JSON to compare against. If omitted, the actual
    /// snapshot is written to stdout (useful for capturing a baseline).
    #[arg(long)]
    golden: Option<PathBuf>,
    /// When set with --golden, overwrite the golden file instead of comparing.
    #[arg(long)]
    update_golden: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("camouflage-replay-check: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let args = Args::parse();
    let mut buf = String::new();
    match &args.input {
        Some(p) => buf = std::fs::read_to_string(p)
            .with_context(|| format!("reading {}", p.display()))?,
        None => {
            std::io::stdin().read_to_string(&mut buf).context("read stdin")?;
        }
    }
    let snap = snapshot_of_ndjson(&buf);
    let actual = snapshot_to_golden_json(&snap);

    match args.golden {
        Some(path) if args.update_golden => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&path, &actual)
                .with_context(|| format!("writing {}", path.display()))?;
            eprintln!("camouflage-replay-check: wrote {}", path.display());
            Ok(ExitCode::SUCCESS)
        }
        Some(path) => {
            let expected = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            if actual.trim() == expected.trim() {
                eprintln!(
                    "camouflage-replay-check: OK ({} rows, {} status segments)",
                    snap.total_rows,
                    snap.status.len()
                );
                Ok(ExitCode::SUCCESS)
            } else {
                let first_diff = actual
                    .lines()
                    .zip(expected.lines())
                    .enumerate()
                    .find(|(_, (a, e))| a != e)
                    .map(|(i, (a, e))| format!("line {}: actual={a:?} expected={e:?}", i + 1))
                    .unwrap_or_else(|| "differ at trailing length".into());
                eprintln!(
                    "camouflage-replay-check: MISMATCH against {}\n  {first_diff}",
                    path.display()
                );
                Ok(ExitCode::FAILURE)
            }
        }
        None => {
            print!("{}", actual);
            Ok(ExitCode::SUCCESS)
        }
    }
}
