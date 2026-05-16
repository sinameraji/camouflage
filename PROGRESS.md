# Camouflage — Progress Tracker

**Single source of truth for "where are we."** Read this first when resuming work, especially after a conversation compaction.

Specs: [`docs/specs/MVP_BUILD_PROMPT.md`](docs/specs/MVP_BUILD_PROMPT.md), [`docs/specs/PRODUCT_SPEC_AND_ROADMAP.md`](docs/specs/PRODUCT_SPEC_AND_ROADMAP.md).

---

## Current stage

**v0.1 MVP — IN PROGRESS (~85% complete)**

Code-complete and tests pass. Offline benchmark hits all four numeric targets. Outstanding items are manual terminal verification, a 5-hour soak run, and a few v0.2-adjacent gaps (lazy paging on scroll, input-latency measurement under a real terminal).

---

## Milestone summary

| Version | Title                            | Status        | Notes                                     |
|---------|----------------------------------|---------------|-------------------------------------------|
| v0.1    | MVP Event-Native TUI             | IN PROGRESS   | ~85% — see breakdown below                |
| v0.2    | Replay & Timeline Inspection     | NOT STARTED   |                                           |
| v0.3    | Renderer Abstraction             | NOT STARTED   |                                           |
| v0.4    | Advanced TUI UX                  | NOT STARTED   |                                           |
| v0.5    | DevTools Layer                   | NOT STARTED   |                                           |
| v0.6    | Ecosystem Layer                  | NOT STARTED   |                                           |
| v0.7    | Desktop Runtime                  | NOT STARTED   |                                           |

**Totals:** 1 / 7 versions in progress, 0 / 7 complete.

---

## v0.1 detailed checklist

| # | Milestone                                         | Status            | Where                                            |
|---|---------------------------------------------------|-------------------|--------------------------------------------------|
| 1 | Workspace + dependency budget (≤15)               | DONE              | `Cargo.toml` — 12 direct deps                    |
| 2 | `protocol` crate (Event + 18 EventTypes)          | DONE              | `crates/protocol/src/lib.rs` + tests             |
| 3 | `store` crate (SQLite WAL, append-only)           | DONE              | `crates/store/src/lib.rs` + 5 tests              |
| 4 | `renderer` crate (bounded model, viewport math)   | DONE              | `crates/renderer/src/{model,viewport}.rs`        |
| 5 | `headless` crate (NDJSON decoder)                 | DONE              | `crates/headless/src/lib.rs`                     |
| 6 | `tui` binary (ratatui + crossterm)                | DONE              | `crates/tui/src/{main,app,draw,input}.rs`        |
| 7 | Persist-before-render pipeline                    | DONE              | `crates/tui/src/app.rs` mpsc + 16ms batch        |
| 8 | Auto-follow + scroll keys                         | DONE              | `viewport.rs` + `input.rs`                       |
| 9 | Frame throttling (60 FPS, dirty-flag)             | DONE              | `crates/tui/src/app.rs` ticker                   |
| 10| Resize handling                                   | DONE              | `crates/tui/src/app.rs` Resize branch            |
| 11| Alternate screen + panic restore                  | DONE              | `crates/tui/src/app.rs` setup/teardown/hook      |
| 12| `bench` binary                                    | DONE (w/ caveat)  | `crates/bench/src/main.rs`                       |
| 13| `fake-agent` example                              | DONE              | `examples/fake-agent/src/main.rs`                |
| 14| README / ARCHITECTURE / DEPENDENCIES / BENCHMARKS | DONE              | repo root                                        |
| 15| Manual-test playbook                              | DONE (doc only)   | `docs/manual-tests.md`                           |
| 16| 4 numeric perf targets on offline bench           | DONE              | replay 72ms, frame 0.009ms, RSS 187MB, 591k w/s  |
| 17| Lazy-page older rows from store on scroll-up      | NOT DONE          | Rows past 2000-cap unreachable. v0.2 candidate.  |
| 18| 5-hour soak run                                   | NOT DONE          | Needs `fake-agent --duration 18000 --fast`       |
| 19| Terminal-in-the-loop p95 input latency measurement| NOT DONE          | Needs pty harness                                |
| 20| Manual TUI verification per `docs/manual-tests.md`| NOT DONE          | Awaits human run-through                         |
| 21| Bench RSS caveat fix (stream into store, not Vec) | NOT DONE          | Documented in `BENCHMARKS.md`                    |

**v0.1: 16 / 21 complete.**

---

## How to use this file

- **At the start of any work session**, open this file first. The "Current stage" line is the elevator pitch.
- **When a milestone changes state**, edit its row here AND the inline `[STATUS]` marker in the relevant spec under `docs/specs/`.
- **When a new milestone surfaces**, append it to the appropriate version's checklist with status `NOT DONE`.
- **Commit this file** alongside the code changes that move it forward — git history then explains "how" while this file explains "what's left."

## Session log

Brief one-liners per session. Keep this short — git log has the detail.

| Date       | Summary                                                                                  |
|------------|------------------------------------------------------------------------------------------|
| 2026-05-15 | Scaffolded workspace; v0.1 code-complete; all unit tests pass; bench hits 4/4 targets.   |
| 2026-05-16 | Initialized git, imported specs into `docs/specs/`, added PROGRESS.md tracker.           |
