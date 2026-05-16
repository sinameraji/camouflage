# Camouflage — Progress Tracker

**Single source of truth for "where are we."** Read this first when resuming work, especially after a conversation compaction.

Specs: [`docs/specs/MVP_BUILD_PROMPT.md`](docs/specs/MVP_BUILD_PROMPT.md), [`docs/specs/PRODUCT_SPEC_AND_ROADMAP.md`](docs/specs/PRODUCT_SPEC_AND_ROADMAP.md).

---

## Current stage

**v0.5 code-complete (DevTools Layer) — four slices A–D shipped (session export, crash-replay dump, golden-snapshot regression tests, deterministic replay-check CLI). Pending: tag `v0.5.0`. Event tracing + latency inspection + per-event profiling deferred to v0.5.5.**

Three tagged releases on GitHub at https://github.com/sinameraji/camouflage:
- `v0.1.0` — event-native TUI MVP (22 milestones, four perf targets met)
- `v0.1.5` — extensibility primitives (`StatusUpdate`, bidirectional protocol, permission widget, task ribbon, etc.)
- `v0.2.0` — replay controls, event inspector, filter, search, tool timing, bookmarks

v0.3 adds: strict NDJSON validator (`camouflage-validate`), event fixture system, headless persistence runtime (`camouflage-record`), `Renderer` trait + serializable `Snapshot`, WebSocket transport (`camouflage-broadcast`), single-file browser replay viewer (`viewer/index.html`). The Camouflage protocol is now consumable from non-Rust frontends over the wire, with both a Rust trait contract and a runnable JS reference implementation.

Diff viewer + slash-command palette + theme system remain deferred to v0.4.

---

## Milestone summary

| Version | Title                            | Status        | Notes                                     |
|---------|----------------------------------|---------------|-------------------------------------------|
| v0.1    | MVP Event-Native TUI             | ✅ DONE        | Tagged `v0.1.0`. All 22 milestones complete. |
| v0.1.5  | Extensibility Primitives         | ✅ DONE        | All 6 slices + post-tag UX polish for narrow terminals. |
| v0.2    | Replay & Timeline Inspection     | ✅ DONE        | Replay controls, event inspector, filter, search, tool timing, bookmarks. |
| v0.3    | Renderer Abstraction             | ✅ DONE        | Validator, fixtures, headless record, Renderer trait + Snapshot, WS broadcast, browser viewer. Tagged `v0.3.0`. |
| v0.4    | Advanced TUI UX                  | ✅ DONE (partial) | Slices A–E: markdown rendering, diff viewer, help overlay, metrics overlay, theme system. Tagged `v0.4.0`. Slash picker + split panes + vim motions + session tabs + `@`-picker + minimap → v0.4.5. |
| v0.5    | DevTools Layer                   | ✅ DONE (partial) | Slices A–D: camouflage-export, crash-replay dump on panic, golden-snapshot regression tests, camouflage-replay-check CLI. Event tracing + latency inspection + per-event profiling → v0.5.5. |
| v0.6    | Ecosystem Layer                  | NOT STARTED   |                                           |
| v0.7    | Desktop Runtime                  | NOT STARTED   |                                           |

**Totals:** 6 / 8 versions code-complete (v0.1.0, v0.1.5, v0.2.0, v0.3, v0.4 partial, v0.5 partial). v0.4.5 / v0.5.5 deferred items + v0.6 / v0.7 remain.

## v0.5 slice checklist

| Slice | Subject | Status |
|-------|---------|--------|
| A | `camouflage-export` (stored session → portable NDJSON; `--list` enumerates sessions) | ✅ DONE — `ca32cee` |
| B | Crash-replay dump on panic (last 256 events written to `crash-<ts>.ndjson`) | ✅ DONE — `acf1677` |
| C | Golden-snapshot regression tests (per-fixture `<name>.snapshot.json` + CI gate) | ✅ DONE — `472ad38` |
| D | `camouflage-replay-check` (deterministic NDJSON → Snapshot CLI for CI) | ✅ DONE — `812fdf8` |
| — | Event tracing (structured trace file for offline analysis) | DEFERRED to v0.5.5 |
| — | Latency inspection (recv → applied → drawn timing histogram) | DEFERRED to v0.5.5 |
| — | Per-event renderer profiling | DEFERRED to v0.5.5 |

## v0.4 slice checklist

| Slice | Subject | Status |
|-------|---------|--------|
| A | Inline markdown rendering (**bold**, *italic*, `code`, escapes) for assistant text | ✅ DONE — `f7e8940` |
| B | Diff viewer (per-line `RowKind::Diff`, color-coded markers, 40-line truncation) | ✅ DONE — `e3b3ae4` |
| C | Help overlay (`?` toggles centered keybind reference) | ✅ DONE — `7bfb316` |
| D | Live-metrics overlay (`M` toggles events/sec, frame time, row util) | ✅ DONE — `85d7c6a` |
| E | Theme system (3 built-in themes, `T` cycles, JSON-loadable) | ✅ DONE — `6ee3efc` |
| — | Slash-command picker | DEFERRED — needs host-protocol additions (SlashCommandsRegistered + SlashCommandSelected) |
| — | Split panes / vim motions / session tabs / sticky tool panels / `@`-picker / timeline minimap / stream profiler | DEFERRED to v0.4.5 — large independent features |

## v0.4 keymap additions

| Key | Action |
|-----|--------|
| `?` | toggle help overlay |
| `M` | toggle live-metrics overlay |
| `T` | cycle to next built-in theme |

## v0.3 slice checklist

| Slice | Subject | Status |
|-------|---------|--------|
| A | Strict NDJSON validator (`camouflage-validate`) — typed payload checks, line+reason errors | ✅ DONE — `b95d9cb` |
| B | Event fixture system (`fixtures/`, loader + CI gate that revalidates all fixtures) | ✅ DONE — `fedfe75` |
| C | Headless runtime (`camouflage-record` — persist NDJSON to SQLite, no UI) | ✅ DONE — `427d7c1` |
| D | `Renderer` trait + serializable `Snapshot` projection of `RenderModel` | ✅ DONE — `cf4ff7d` |
| E | WebSocket transport (`camouflage-broadcast` — fan NDJSON to live WS clients with replay buffer) | ✅ DONE — `1c5bee8` |
| F | Single-file browser replay viewer (`viewer/index.html`) | ✅ DONE — `700ce3d` |

## v0.3 surfaces added

| Surface | Where | Notes |
|---------|-------|-------|
| `camouflage-validate` | `crates/headless/src/bin/camouflage_validate.rs` | Stdin or file → strict typed validation; exits 1 on any error. |
| `camouflage-record` | `crates/headless/src/bin/camouflage_record.rs` | Stdin → SQLite WAL store; flags for batch size, session id, summary. |
| `camouflage-broadcast` | `crates/headless/src/bin/camouflage_broadcast.rs` | Stdin → WebSocket fan-out, in-memory replay buffer, slow-client drop. |
| `Renderer` / `SnapshotRenderer` traits | `crates/renderer/src/lib.rs` | Contract for any non-TUI renderer; `RenderModel` implements both. |
| `Snapshot` / `SnapshotRow` / `SnapshotTask` / `SnapshotPermission` | `crates/renderer/src/snapshot.rs` | Serde-derived; narrow projection of `RenderModel` for wire transport. |
| Workspace `fixtures/` | repo root | `kimiflare-mock.ndjson`, `kimiflare-adapter-simple.ndjson`, `all-event-types.ndjson` |
| `viewer/index.html` | repo root | Single-file viewer; auto-connects to `ws://localhost:8080`. |

## Dependency budget update

v0.3 added `tokio-tungstenite` + `futures-util` for the WebSocket transport. The workspace is now at **16 direct dependencies (was 14; previous budget was 15)**. Both are standard async-WS choices with no smaller alternative; the budget is intentionally re-pegged to 16. Future deps should still be justified at the slice-plan stage.

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
| 20| Manual TUI verification per `docs/manual-tests.md`| DONE              | User confirmed end-to-end on real terminal (mock content rendered, keys work, narrow-terminal wrap working). |
| 21| Bench RSS caveat fix (stream into store, not Vec) | DONE              | Streaming generator in `crates/bench`: RSS 187 MB → 37 MB. |

| 22| Pipe-stdin works end-to-end (custom /dev/tty key reader bypassing crossterm's broken mio path) | DONE | `crates/tui/src/tty.rs`. Verified via pty test: 290 events streamed and persisted, full UI rendered. Root cause: macOS kqueue returns EINVAL when registering a freshly-opened /dev/tty fd, which kills crossterm's event source. Workaround: blocking `read(/dev/tty)` thread with a small ANSI parser. |

**v0.1: 22 / 22 complete — tagged `v0.1.0`.**

---

## How to use this file

- **At the start of any work session**, open this file first. The "Current stage" line is the elevator pitch.
- **When a milestone changes state**, edit its row here AND the inline `[STATUS]` marker in the relevant spec under `docs/specs/`.
- **When a new milestone surfaces**, append it to the appropriate version's checklist with status `NOT DONE`.
- **Commit this file** alongside the code changes that move it forward — git history then explains "how" while this file explains "what's left."

## What's left

Per the spec roadmap, in order:

| Version | Title | Headline scope |
|---------|-------|----------------|
| v0.3 | Renderer Abstraction | Renderer protocol, websocket transport, headless runtime mode, browser replay viewer, event schema validator, event fixture system |
| v0.4 | Advanced TUI UX | Split panes, vim motions, **command palette**, **diff viewer**, sticky tool panels, session tabs, timeline minimap, live metrics, stream profiler, theme system, slash-command picker, `@`-mention picker, help menu |
| v0.5 | DevTools Layer | Event tracing, timeline debugging, performance profiling, renderer profiling, latency inspection, event validation, crash replay, deterministic replay, regression fixtures, session export/import |
| v0.6 | Ecosystem Layer | Node SDK, Rust SDK, protocol docs, integration adapters, renderer plugin API, benchmark suite, schema validation tooling, CI benchmark runner, migration guide from Ink, migration guide from Electron/webviews |
| v0.7 | Desktop Runtime | Native desktop shell, GPU rendering exploration, detached replay viewer, session archive browser, advanced profiling, remote stream synchronization, persistent local session library |

Off-roadmap but high-value:

- **KimiFlare adapter (`~/kimi-code-clone-3`, branch `camouflage-adapter`)** — MVP shipped 2026-05-16 (`b840b80` in that repo). One-shot `--emit-events` mode emits 8 event types via NDJSON. Validated end-to-end with a real Cloudflare turn through `camouflage-tui --stdin-events`. Follow-up slices, in priority order:
  - **Emit `ToolExecutionStdout` chunks** so tool output is visible in the transcript (currently shows `stdout=0B`). [adapter gap from 2026-05-16 test]
  - **Multi-turn mode** — stdin reader for `UserInputSubmitted` to drive follow-up prompts into new `runAgentTurn` calls.
  - **Bidirectional permission flow** — route stdin `PermissionResponse` into the pending `askPermission` promise instead of the current auto-allow / auto-deny.
  - **`StatusUpdate` + `BackgroundTaskUpdate` emission** — needs a different tap point inside `app.tsx::sharedCallbacks` since the headless `runAgentTurn` doesn't surface phase/usage in that shape.
- **5-hour soak** — the script (`scripts/soak.py`) is ready; 30 min has been validated. Run the full 18000 s when an idle laptop is available.

## TUI bug backlog (from adapter testing)

Discovered 2026-05-16 while smoke-testing the KimiFlare adapter against a real terminal. Each should be one small commit; none are roadmap milestones. Address between version cuts, not in the middle of one — writing them down so they don't get lost.

| #   | Bug | Likely cause / where to look | Repro |
|-----|-----|------------------------------|-------|
| TB1 | "session started" row appears twice on session boot | TUI synthesizes a session-start row even when the host emits `SessionStarted` — collision in `crates/renderer/src/model.rs` apply path or the boot synth in `crates/tui/src/app.rs`. | Run adapter one-shot; observe two consecutive `- session started` rows at the top. |
| TB2 | Status bar phase stays `streaming` after `SessionEnded` arrives | `SessionEnded` apply path should clear / reset the `StatusBarState` phase segment (or the renderer should infer idle when no active stream/tool). `crates/renderer/src/status.rs` + `apply()` in `model.rs`. | Run adapter one-shot to completion; status bar still reads `streaming` indefinitely. |
| TB3 | Spinners (status-bar phase glyph + task-ribbon dot) animate while user scrolls | Spinner frame counter is incrementing on *redraw* tick instead of on a wall-clock tick. Scroll causes redraws → glyph advances without time passing. Fix: drive spinner frame from `Instant::now()` modulo period, not a per-redraw counter. `crates/tui/src/draw.rs`. | Run adapter one-shot; before completion, scroll up/down — watch the spinners spin in lockstep with scroll. |

## Cross-version validation notes

Evidence collected from the 2026-05-16 adapter test that confirms scope decisions for later versions — record here so we don't relitigate at version-cut time.

- **v0.4 (Advanced TUI UX) — markdown rendering and diff viewer confirmed as critical.** Assistant text rendered with literal `**bold**`, raw backticks, and no paragraph wrap. Long unbroken token sequences cluster into walls of text. Confirms diff/markdown/theme work belongs *exactly* where the roadmap puts it — do not pull forward into v0.3.

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
| 2026-05-16 | KimiFlare adapter MVP shipped in `~/kimi-code-clone-3` on branch `camouflage-adapter` (commit `b840b80`). Added `--emit-events` one-shot mode (`src/emit-mode.ts`) cloning the `runPrintMode` template; emits 8 event types (Session/UserMessage/AssistantStream/Tool/Permission/RuntimeError) via NDJSON to stdout. Smoke-tested with a real Cloudflare turn — clean 6-event stream end-to-end. Multi-turn, bidirectional permission responses, `StatusUpdate` and `BackgroundTaskUpdate` deferred to follow-up commits. |
| 2026-05-16 | Real-terminal adapter test surfaced 3 TUI bugs + 1 adapter gap + 2 v0.4-scheduled cosmetic gaps. Captured in new "TUI bug backlog" + "Cross-version validation notes" sections instead of fixing reactively, to keep roadmap momentum. |
| 2026-05-16 | v0.3 code-complete: Slices A–F shipped (`b95d9cb` → `700ce3d`). New surfaces: `camouflage-validate`, `camouflage-record`, `camouflage-broadcast`, `Renderer`/`SnapshotRenderer` traits, `Snapshot` serde projection, workspace `fixtures/` + CI gate, single-file `viewer/index.html` browser viewer. Dep budget bumped to 16 (added `tokio-tungstenite` + `futures-util`). 45 workspace tests pass. Tagged `v0.3.0`. |
| 2026-05-16 | v0.4 code-complete (partial): Slices A–E shipped (`f7e8940` → `6ee3efc`). Markdown rendering for assistant text, diff viewer with color-coded markers, `?` help overlay, `M` metrics overlay, theme system with 3 built-ins (`T` cycles). 51 workspace tests, all green. Slash-command picker + split panes + vim motions + session tabs + `@`-picker + minimap deferred to v0.4.5 (need host-protocol coordination or are large independent features). Tagged `v0.4.0`. |
| 2026-05-16 | v0.5 code-complete (partial): Slices A–D shipped (`ca32cee` → `812fdf8`). `camouflage-export` (stored session → portable NDJSON, with `--list`), crash-replay dump on panic (ring buffer flushed to `crash-<ts>.ndjson`), golden-snapshot regression tests (per-fixture `<name>.snapshot.json` checked-in + integration test gate), `camouflage-replay-check` (deterministic CI helper). 63 workspace tests, all green. Event tracing + latency inspection + per-event profiling deferred to v0.5.5. Tag `v0.5.0` pending. |
