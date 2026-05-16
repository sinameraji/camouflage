# Dependencies

Direct dependencies, with rationale. Budget: ≤15 for v0.1.

| Crate                 | Used by              | Why                                          | Alternatives considered           |
|-----------------------|----------------------|----------------------------------------------|-----------------------------------|
| ratatui               | tui                  | Terminal UI; minimal, modern, diff-based     | tui-rs (unmaintained), cursive    |
| crossterm             | tui                  | Cross-platform terminal backend for ratatui  | termion (Linux-only)              |
| tokio                 | tui, headless        | Async runtime; needed for concurrent I/O     | smol, std threads                 |
| serde                 | protocol, …          | Event (de)serialisation                      | manual JSON parsing               |
| serde_json            | protocol, …          | NDJSON transport, payload values             | simd-json (heavier)               |
| rusqlite              | store                | SQLite with `bundled` so no system dep       | sqlx (async, but heavier deps)    |
| uuid                  | protocol, …          | Stable event/session identifiers             | nanoid (less universal)           |
| clap                  | tui, bench, fake-agent | Argument parsing                           | argh, lexopt                      |
| anyhow                | tui, bench, fake-agent | Application error type                     | eyre                              |
| thiserror             | protocol, store, headless | Library error enums                      | hand-rolled `Error`               |
| tracing               | tui, store           | Structured logging                           | log + env_logger                  |
| tracing-subscriber    | tui                  | Tracing -> stderr formatter                  | fmt by hand                       |
| libc                  | tui                  | Open /dev/tty + apply termios (raw mode) so the TUI works when stdin is a pipe of NDJSON events | nix (heavier) |
| unicode-width         | tui                  | Display-width-aware truncation of long rows / status segments on narrow terminals | char-count fallback (wrong for emoji + CJK) |

**Total direct deps: 14** (under the 15 budget).

## Rules

1. Every new direct dependency must be justified in this table.
2. Prefer `bundled` features that eliminate system requirements.
3. No embedded JS runtime.
4. No hidden network calls.
5. `cargo audit` in CI (future).

## Transitive surface

`rusqlite` (with `bundled`) and `ratatui` pull the largest trees. Both are widely used, actively maintained, and have no native runtime dependencies once `bundled` is set.
