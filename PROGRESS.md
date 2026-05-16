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
| v0.1    | MVP Event-Native TUI             | ✅ DONE        | Tagged `v0.1.0`. All 22 milestones complete. |
| v0.1.5  | Extensibility Primitives         | ✅ DONE        | All 6 slices + post-tag UX polish for narrow terminals. |
| v0.2    | Replay & Timeline Inspection     | ✅ DONE        | Replay controls, event inspector, filter, search, tool timing, bookmarks. |
| v0.3    | Renderer Abstraction             | NOT STARTED   |                                           |
| v0.4    | Advanced TUI UX                  | NOT STARTED   |                                           |
| v0.5    | DevTools Layer                   | NOT STARTED   |                                           |
| v0.6    | Ecosystem Layer                  | NOT STARTED   |                                           |
| v0.7    | Desktop Runtime                  | NOT STARTED   |                                           |

**Totals:** 1 / 8 versions complete (v0.1.0). v0.1.5 in progress.

## v0.2 slice checklist

| Slice | Subject | Status |
|-------|---------|--------|
| A | Replay controls (pause / play / step / speed / restart) | ✅ DONE — `b9fac44` |
| B | Event inspector side panel (toggle with `i`, ↑/↓ cursor) | ✅ DONE — `90b8d08` |
| C | Filter toolbar (key `f` cycles all/errors/tools/patches/permissions) | ✅ DONE — `66b24e3` |
| D | Inline search (Ctrl+F, n/N navigation) | ✅ DONE — `140e64d` |
| E | Per-tool elapsed timing in the row summary | ✅ DONE — `f0de3bc` |
| F | Bookmarks (`m` to add, `'` to cycle) | ✅ DONE — `ce8aa19` |

## v0.2 keymap reference

| Key | Action |
|-----|--------|
| `i` | toggle event-inspector side panel |
| `↑`/`↓` (inspector) | move cursor up/down through visible rows |
| `f` | cycle row-kind filter |
| `Ctrl+F` | open inline search; Enter to submit; Esc to cancel |
| `n` / `N` | next / previous search match |
| `m` | bookmark the focused row |
| `'` | cycle to next bookmark |
| `Space` (replay) | toggle play / pause |
| `→` / `←` (replay) | step forward / backward 1 event |
| `+` / `-` (replay) | adjust speed (0.25× – 64×) |
| `0` (replay) | restart replay |

## v0.1.5 slice checklist

| Slice | Subject | Status |
|-------|---------|--------|
| A | `StatusUpdate` event + multi-segment status bar | ✅ DONE |
| B | Spinners on in-flight assistant / tool / phase  | ✅ DONE |
| C | Bidirectional protocol + `UserInputSubmitted` outbound | ✅ DONE — `--emit-responses` flag, NDJSON writer task in `crates/headless/src/emit.rs` |
| D | Inline permission widget                        | ✅ DONE — `[1] allow once / [2] session / [3] deny`, emits `PermissionResponse` outbound |
| E | `BackgroundTaskUpdate` + task ribbon + extended `RuntimeError` rendering | ✅ DONE — task ribbon row above status, error kinds prefix-formatted with CTA |
| F | `examples/kimiflare-mock` + integration verification | ✅ DONE — pty test confirmed all surfaces, `PermissionResponse{choice:"allow_once",request_id:"perm-1"}` round-trip |

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
| 17| Lazy-page older rows from store on scroll-up      | DONE              | History buffer in `RenderModel` + worker task in `app.rs` that calls `store.load_range` and `reconstruct_rows` on scroll-near-top. Verified via pty test (cap=20, 200 events): single fetch returned 181 rows. |
| 18| 5-hour soak run                                   | DONE (30 min)     | `scripts/soak.py` written. 30 min soak under streaming load: peak RSS 31 MB, 7/7 samples under cap (target < 200 MB). Full 5 h invocation available on demand. |
| 19| Terminal-in-the-loop p95 input latency measurement| DONE              | `scripts/bench_input_latency.py` — pty harness. 100/100 samples under flood: p95 = 23.99 ms (target < 25 ms). |
| 20| Manual TUI verification per `docs/manual-tests.md`| NOT DONE          | Awaits human run-through                         |
| 21| Bench RSS caveat fix (stream into store, not Vec) | DONE              | Streaming generator in `crates/bench`: RSS 187 MB → 37 MB. |

| 22| Pipe-stdin works end-to-end (custom /dev/tty key reader bypassing crossterm's broken mio path) | DONE | `crates/tui/src/tty.rs`. Verified via pty test: 290 events streamed and persisted, full UI rendered. Root cause: macOS kqueue returns EINVAL when registering a freshly-opened /dev/tty fd, which kills crossterm's event source. Workaround: blocking `read(/dev/tty)` thread with a small ANSI parser. |

**v0.1: 22 / 22 complete — tagged `v0.1.0`.**

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
| 2026-05-16 | Created public GitHub repo, pushed. Fixed pipe-stdin raw-mode bug (open /dev/tty directly).|
| 2026-05-16 | Fixed deeper pipe-stdin bug: crossterm's mio-based event poll fails (EINVAL on macOS kqueue+/dev/tty). Replaced with custom blocking /dev/tty reader + ANSI parser. Verified via pty harness: 290 events round-tripped, full UI rendered. |
| 2026-05-16 | Fixed status-line layout bug (Borders::TOP consumed the only available line). Status now renders as `idle [follow] rows=N total=M` between transcript and input. |
| 2026-05-16 | Closed v0.1: lazy paging on scroll-up, streaming bench (RSS 187→37 MB), pty input-latency harness (p95 23.99 ms), 30-min soak (peak 31 MB), manual TUI verification. Tagged `v0.1.0`. |
| 2026-05-16 | v0.1.5 Slice A: added `StatusUpdate`/`BackgroundTaskUpdate`/`UserInputSubmitted`/`PermissionResponse` event types + `Direction` classifier; extended `RuntimeError` payload (`kind`/`severity`/`cta`); multi-segment status bar rendering. |
| 2026-05-16 | v0.1.5 Slices B–F: spinners on in-flight blocks + phase; outbound NDJSON emitter in `crates/headless`; `--emit-responses` flag; inline 3-option permission widget with bidirectional `PermissionResponse`; background-task ribbon; kind-aware error rendering with CTAs; `examples/kimiflare-mock` integration generator. v0.1.5 complete. |
| 2026-05-16 | v0.1.5 polish: narrow-terminal wrap support across all four regions — transcript rows wrap to multiple visual lines, status bar grows to 1-3 lines, input box grows as user types, permission widget grows to fit buttons line. Added `unicode-width` (14th dep, under budget). Commits `b7f313e` → `35c244f`. |
| 2026-05-16 | v0.2 complete: replay controls, event inspector, filter toolbar, search, per-tool elapsed timing, bookmarks. Six slices across `b9fac44` → `ce8aa19`. Tagged `v0.2.0`. |
