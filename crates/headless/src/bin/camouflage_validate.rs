//! `camouflage-validate` — strict NDJSON validator.
//!
//! Reads NDJSON from stdin or a file, validates every line against the
//! Camouflage event protocol, and prints a summary. Exits 0 on success,
//! 1 on any validation error, 2 on I/O error.
//!
//! Usage:
//!     cargo run -p camouflage-headless --bin camouflage-validate -- [path]
//!     cat events.ndjson | camouflage-validate
//!     camouflage-validate fixtures/host-mock.ndjson

use camouflage_headless::validate::{validate_stream, ValidationReport};
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let path = std::env::args().nth(1).map(PathBuf::from);
    let mut buf = String::new();
    let read_result = match &path {
        Some(p) => std::fs::read_to_string(p).map(|s| {
            buf = s;
        }),
        None => std::io::stdin().read_to_string(&mut buf).map(|_| ()),
    };
    if let Err(e) = read_result {
        eprintln!("camouflage-validate: read error: {e}");
        return ExitCode::from(2);
    }

    let report = validate_stream(&buf);
    print_report(&report, path.as_deref().and_then(|p| p.to_str()));
    if report.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_report(report: &ValidationReport, source: Option<&str>) {
    let src = source.unwrap_or("<stdin>");
    for err in &report.errors {
        eprintln!("{src}: {err}");
    }
    let invalid = report.lines_total - report.lines_valid;
    eprintln!(
        "{src}: {} lines, {} valid, {} invalid",
        report.lines_total, report.lines_valid, invalid
    );
}
