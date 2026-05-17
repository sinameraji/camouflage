# Camouflage — Progress Tracker

**Single source of truth for "where are we."** Read this first when resuming work, especially after a conversation compaction.

Specs: [`docs/specs/MVP_BUILD_PROMPT.md`](docs/specs/MVP_BUILD_PROMPT.md), [`docs/specs/PRODUCT_SPEC_AND_ROADMAP.md`](docs/specs/PRODUCT_SPEC_AND_ROADMAP.md).

## File layout (read in this order)

1. **Open work** — the long forward-looking checklist. Everything NOT yet done lives here. Each item has an ID, a one-line title, a status, the reasoning/why, and where to look. Pick from the top of the list to work next.
2. **Current stage** — one-paragraph elevator pitch of where the codebase is right now.
3. **Tagged releases** — what's shipped and where to find it on GitHub.
4. **Per-version slice checklists** — backward-looking record of what each version contained.
5. **Session log** — one-liners per work session for archeology.

When a new item surfaces in a conversation (bug report, feature idea, decision), add it to **Open work** immediately with a fresh ID and the reasoning, so it can't get lost.

When an item ships, flip its status to ✅ DONE and add the commit hash; leave it in place rather than deleting (so the document remains a complete history of decisions, not just current state).

---

## Open work

Items below are **not yet done**. Pick the highest-priority one (top of each section) when starting work. Add new items at the bottom of the relevant section as they're discovered.

Status legend: ⬜ TODO · ⏳ IN PROGRESS · ✅ DONE (kept in place for archeology) · ⏸ DEFERRED (intentionally on hold, reason given)

### TUI bugs surfaced by real `--ui camouflage` testing (2026-05-17)

These came out of running `kimiflare --ui camouflage --dangerously-allow-all -p "..."` end-to-end against a real Cloudflare turn. They're real, reproducible, and block daily use.

| ID | Status | Bug | Reasoning / details | Where |
|----|--------|-----|---------------------|-------|
| TBR-1 | ✅ DONE — `dc861bc` | Tool execution rows show raw JSON-stringified arguments spanning many visual lines | `KimiFlare's call.function.arguments is a JSON string. Model put it directly into the row text. Fix: compact_command() helper truncates to 120 chars with ellipsis, collapses whitespace, strips control chars. Full payload retained on ToolState.command for X-overlay.` | `crates/renderer/src/model.rs` |
| TBR-2 | ✅ DONE — `dc861bc` | Spinners keep spinning on stale empty assistant rows after the stream completed | `row_to_line drew a spinner on any empty Assistant row regardless of whether the stream was active. Fix: row_to_line takes has_active_stream; model exposes has_active_stream() / active_stream_row().` | `crates/renderer/src/{model,tui/src/draw}.rs` |
| TBR-3 | ✅ DONE — `dc861bc` | Help/metrics/tool-output overlays only closeable via their toggle key (Shift+? is awful UX) | `Esc should close any open overlay. Currently Esc maps to CancelStream globally. Fix: pre-handler intercepts Esc when an overlay is open and closes it instead.` | `crates/tui/src/app.rs` |
| TBR-4 | ✅ DONE — `dc861bc` | Up/Down arrows in input prompt don't recall prior submitted prompts | `Standard readline expectation. Added input_history Vec capped at 200; SubmitInput pushes; Up/Down walks history when input is focused and no overlay/picker/permission/replay is taking the key.` | `crates/tui/src/app.rs` |
| TBR-5 | ⬜ TODO | Slash picker (`/`) does nothing when typed | `KimiFlare adapter does not yet emit SlashCommandsRegistered. The TUI picker overlay exists (Phase 2.6, commit 48fb2c7) but only triggers when commands are registered. Adapter fix: enumerate KimiFlare's 28 slash commands (src/commands/builtins.ts) and emit on session start.` | `~/kimi-code-clone-3/src/{ui-mode,emit-mode}.ts` |
| TBR-6 | ⬜ TODO | Can't scroll up to see entire session — earlier rows disappear | `Possibly the live-buffer 2000-row cap kicked in mid-session AND lazy paging from store isn't loading the older rows back. Need to repro and check: (a) is the renderer persisting events to the store under --ui camouflage, (b) does the scroll-up history paging fire, (c) does prepend_history correctly insert the older rows. Could also be a viewport math edge case.` | `crates/renderer/src/{model,viewport}.rs`, `crates/tui/src/app.rs` history worker |
| TBR-7 | ⬜ TODO | "Formatting looks off" — needs a more specific repro | `User's screenshot shows markdown rendering working on most assistant text but tool-arg rows still dominated the screen. With TBR-1 fixed this should be much less bad. Ask user to retest and screenshot any remaining issues.` | n/a |

### Components catalog (the missing UI primitives layer)

Recognized 2026-05-17. Today Camouflage exposes a fixed set of widgets (StatusBar, TaskRibbon, PermissionWidget, overlays for help/metrics/tools/slash/mention). Hosts that need richer UI (session picker, checkpoint picker, multi-step wizard, settings form) currently have no way to ask the renderer to show one. This is the missing primitives layer that makes Camouflage a *UI library* (like shadcn/ui for terminals) rather than just an event-driven transcript renderer.

Design: each component is one inbound event (`Show<Component>`) carrying a unique `id`, plus an optional outbound response event (`<Component>Response`) keyed by the same id. Renderer owns layout/theme; host owns the data and the eventual handling of the response.

The full catalog conversation from 2026-05-17 lives in [`docs/specs/components-catalog.md`](docs/specs/components-catalog.md) (to be created in the next slice). Quick reference of components in priority order:

| ID | Component | Status | KimiFlare consumers it unblocks | Reasoning |
|----|-----------|--------|----------------------------------|-----------|
| CC-1 | `SelectList` | ✅ DONE — `c7480b4 → aac4094` (camouflage) + `6556771` (kimi-code-clone-3) | session picker, checkpoint picker, theme picker, slash picker (generalised), resume picker, model picker | Highest leverage — single shape covers ~6 KimiFlare React components. Shipped in 5 commits: protocol+payloads, model+Snapshot, TUI overlay+keys+outbound emit, Node SDK helper, KimiFlare `kimiflare resume` driver. Validated end-to-end with real session data. Existing hardcoded slash picker (Phase 2.6) is structurally a special-case SelectList — generalising it (deleting `SlashCommandsRegistered` in favor of `ShowSelectList`) is a follow-up cleanup. |
| CC-2 | `Confirm` | ✅ DONE — `7bc8d43` | "save before quit?", "delete session?", any yes/no modal | Tiny scope, covers many small modals. The existing `PermissionRequested` is structurally a special-case Confirm; we keep it as the well-known type. |
| CC-3 | `Toast` | ✅ DONE — `8f3f0d4` | "Saved", "Authenticated", brief feedback | Trivial scope, immediate visible value. No outbound event — display-only with optional TTL. |
| CC-4 | `Wizard` | ⬜ TODO | onboarding flow, LSP wizard, command wizard | Multi-step; composes 1+2+3. |
| CC-5 | `Form` | ⬜ TODO | settings configuration, cloud token + endpoint configuration | Bigger; can wait for a real driver. |
| CC-6 | `Table` | ✅ DONE — `e4c1d25` (display-only modal; selectable rows + inline mode are follow-ups) | usage stats, cost attribution, session-list with metadata | Display-oriented; optional `RowSelected` outbound for interactive use. |
| CC-7 | `KeyValueView` | ✅ DONE — `<this commit>` (display-only modal) | "session details" inspector pane, welcome screen | Display-oriented. |

Each component ships as a self-contained slice (one commit series): protocol event(s) → model state → renderer overlay → Snapshot projection field → Node SDK types update → KimiFlare adapter wires it against an actual component being replaced. The KimiFlare wiring is what proves the design.

### KimiFlare adapter follow-ups (on `~/kimi-code-clone-3` `camouflage-adapter` branch)

| ID | Status | Item | Reasoning |
|----|--------|------|-----------|
| ADP-1 | ⬜ TODO | Emit `SlashCommandsRegistered` listing KimiFlare's 28 slash commands | Unblocks TBR-5. Source list in `~/kimi-code-clone-3/src/commands/builtins.ts`. |
| ADP-2 | ⬜ TODO | Wire `--ui camouflage` to use a SelectList for the resume picker (first CC-1 driver) | After CC-1 ships, replace KimiFlare's `src/ui/resume-picker.tsx` with a `ShowSelectList` emit + `SelectListResponse` handler. Validates the catalog design. |
| ADP-3 | ⬜ TODO | KimiFlare → npm-published `camouflage` package | Currently uses `file:../camouflage/sdk/node` for local development. Switch to a real npm version once we publish. |
| ADP-4 | ⏸ DEFERRED | Cost segment in `StatusUpdate` (mode/phase/elapsed/tokens/cost/branch) | Requires KimiFlare's cost-attribution machinery (complex; lives in `src/cost-attribution/`). Land after CC-1 → CC-3 demonstrate the catalog works. |
| ADP-5 | ⏸ DEFERRED | Mode cycling (`edit` / `plan` / `auto` + Shift+Tab) | Hardcoded to "edit" in `runUiMode`. Needs a host-protocol addition (`ModeChanged` event) and KimiFlare's mode controller. |

### v0.5.5 (DevTools follow-ups)

| ID | Status | Item | Reasoning |
|----|--------|------|-----------|
| DT-1 | ⬜ TODO | Event tracing — structured trace file for offline analysis | Jaeger-compatible JSON output of every event with timing. |
| DT-2 | ⬜ TODO | Latency inspection — recv → applied → drawn timing histogram | Surfaced in metrics overlay or a separate report. |
| DT-3 | ⬜ TODO | Per-event renderer profiling | Time-per-event-kind histograms. |

### v0.6.5 (Ecosystem follow-ups)

| ID | Status | Item | Reasoning |
|----|--------|------|-----------|
| ECO-1 | ⬜ TODO | Integration adapters bundle | Reference adapters for non-KimiFlare hosts (e.g., a Python adapter, Go adapter). |
| ECO-2 | ⬜ TODO | Renderer plugin API | Allow hosts to register custom event types + custom render handlers. Possibly subsumed by the components catalog. |
| ECO-3 | ⬜ TODO | Benchmark suite extensions + CI benchmark runner | Track perf regressions on every push. |
| ECO-4 | ⬜ TODO | Migration guide (Ink → Camouflage) | Step-by-step "how to replace Ink" doc. Should reference the KimiFlare adapter's Option-B migration as the worked example. |
| ECO-5 | ⬜ TODO | Migration guide (Electron/webviews → Camouflage) | Same pattern, different starting point. |
| ECO-6 | ⬜ TODO | Publish `camouflage` to npm + prebuilt binary downloads via `postinstall` | Currently consumers need `cargo install` and a manual PATH entry. Need GitHub Actions per-platform builds + release uploads. |

### v0.4.5+ (deferred Advanced TUI UX items)

| ID | Status | Item | Reasoning |
|----|--------|------|-----------|
| UX-1 | ⬜ TODO | Split panes | Side-by-side transcript/inspector or two sessions. |
| UX-2 | ⬜ TODO | Vim motions | hjkl, w/b/e in inspector mode. |
| UX-3 | ⬜ TODO | Session tabs | Multiple sessions in one TUI; tab switcher. |
| UX-4 | ⬜ TODO | Sticky tool panels | Pin a tool's output to the bottom of the transcript. |
| UX-5 | ⬜ TODO | Timeline minimap | Compressed event-density view in a sidebar. |
| UX-6 | ⬜ TODO | Stream profiler | Visual representation of event throughput per turn. |
| UX-7 | ⬜ TODO | Hierarchical help menu | Replaces flat help overlay with categorized navigation. |
| UX-8 | ⬜ TODO | Queued-prompts row | Visible "you have N prompts queued" row above input. |
| UX-9 | ⬜ TODO | Repeated-call warning on tool rows | "agent called the same tool 5x in a row" hint. |
| UX-10 | ⬜ TODO | Port KimiFlare's remaining 7 themes (we have 6 of 13) | Catppuccin Latte/Mocha, Everforest dark/light, Kanagawa, One Dark, Solarized dark/light. Data entry. |

### v0.7 (Desktop Runtime — entire version still untouched)

| ID | Status | Item |
|----|--------|------|
| DR-1 | ⬜ TODO | Native desktop shell |
| DR-2 | ⬜ TODO | GPU rendering exploration |
| DR-3 | ⬜ TODO | Detached replay viewer (extends `viewer/index.html` to load `.ndjson` files via drag-drop or URL param) |
| DR-4 | ⬜ TODO | Session archive browser |
| DR-5 | ⬜ TODO | Remote stream synchronization |
| DR-6 | ⬜ TODO | Persistent local session library |

### Maintenance / housekeeping

| ID | Status | Item |
|----|--------|------|
| MN-1 | ⬜ TODO | 5-hour soak run (`scripts/soak.py` is ready; 30 min validated; full 18000 s when idle laptop available) |

---

## Current stage

**Node SDK (`camouflage` npm package) shipped + `--ui camouflage` mode in KimiFlare.** The user can now run `kimiflare --ui camouflage -p "..."` and have Camouflage render directly in their terminal — single command, single process tree, no piping. This is the integration shape that paves the way to deleting Ink entirely (Option B).

Five-commit series on Camouflage main (`ebf748d` → `d826f89`) + camouflage-tui `--responses-fd` flag (`97af9dd`) + KimiFlare `--ui camouflage` rewrite on `camouflage-adapter` branch in `~/kimi-code-clone-3` (`2f1c7de`).

**Ink-replacement-ready milestone shipped (tagged `v0.4.5`).** Three phases of focused work closed the gap between Camouflage and KimiFlare's React/Ink UI:

- **Phase 1** (5 slices on `~/kimi-code-clone-3` `camouflage-adapter` branch): adapter now emits `ToolExecutionStdout/Stderr`, supports `--multi-turn` mode, emits `StatusUpdate` (mode/phase/elapsed/tokens/branch), wires bidirectional `PermissionResponse`, emits `BackgroundTaskUpdate` for skills/memory/agent tasks.
- **Phase 2** (4 slices on this repo): slash-command picker (`/`), `@`-mention picker, expandable tool-output overlay (`X`), permission widget with free-text feedback.
- **Phase 3** (this repo): TB1/TB2/TB3 TUI bugs cleared, +3 themes (6 total: dark/light/dracula/nord/gruvbox-dark/tokyo-night).

KimiFlare can now realistically swap out Ink for Camouflage end-to-end. Remaining gaps are nice-to-haves (more themes, hierarchical help menu, queued-prompts row, etc.) — see "What's left".

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
| v0.5    | DevTools Layer                   | ✅ DONE (partial) | Slices A–D: camouflage-export, crash-replay dump on panic, golden-snapshot regression tests, camouflage-replay-check CLI. Tagged `v0.5.0`. Event tracing + latency inspection + per-event profiling → v0.5.5. |
| v0.6    | Ecosystem Layer                  | ✅ DONE (partial) | Slices A–C: Rust SDK facade crate (`camouflage`), `docs/protocol.md` reference, Node SDK (TS types + NDJSON reader). Integration adapters / plugin API / benchmark extensions / migration guides → v0.6.5. |
| v0.7    | Desktop Runtime                  | NOT STARTED   |                                           |

**Totals:** 7 / 8 versions code-complete (v0.1.0, v0.1.5, v0.2.0, v0.3, v0.4 partial, v0.5 partial, v0.6 partial). v0.4.5 / v0.5.5 / v0.6.5 deferred items + v0.7 remain.

## v0.6 slice checklist

| Slice | Subject | Status |
|-------|---------|--------|
| A | Rust SDK facade crate (`camouflage` re-exports protocol/store/renderer/headless) | ✅ DONE — `40fc880` |
| B | `docs/protocol.md` — complete event/payload reference | ✅ DONE — `40fc880` |
| C | Node SDK (`sdk/node/`, TS types + NDJSON reader + validate + encode) | ✅ DONE — pending |
| — | Integration adapters (collection of reference adapters) | DEFERRED to v0.6.5 |
| — | Renderer plugin API | DEFERRED to v0.6.5 |
| — | Benchmark suite extensions + CI benchmark runner | DEFERRED to v0.6.5 |
| — | Migration guide (Ink → Camouflage) | DEFERRED to v0.6.5 |
| — | Migration guide (Electron/webviews → Camouflage) | DEFERRED to v0.6.5 |

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

## Historical "What's left" (superseded by Open work above; kept for archeology)

The original headline-scope table from when v0.3–v0.7 were unstarted:

| Version | Title | Original headline scope |
|---------|-------|----------------|
| v0.3 | Renderer Abstraction | Renderer protocol, websocket transport, headless runtime mode, browser replay viewer, event schema validator, event fixture system |
| v0.4 | Advanced TUI UX | Split panes, vim motions, command palette, diff viewer, sticky tool panels, session tabs, timeline minimap, live metrics, stream profiler, theme system, slash-command picker, `@`-mention picker, help menu |
| v0.5 | DevTools Layer | Event tracing, timeline debugging, performance profiling, renderer profiling, latency inspection, event validation, crash replay, deterministic replay, regression fixtures, session export/import |
| v0.6 | Ecosystem Layer | Node SDK, Rust SDK, protocol docs, integration adapters, renderer plugin API, benchmark suite, schema validation tooling, CI benchmark runner, migration guide from Ink, migration guide from Electron/webviews |
| v0.7 | Desktop Runtime | Native desktop shell, GPU rendering exploration, detached replay viewer, session archive browser, advanced profiling, remote stream synchronization, persistent local session library |

For the current state of these items, see **Open work** at the top of this file — each version's deferred items have their own ID prefix (DT-N, ECO-N, UX-N, DR-N, etc.) and explicit reasoning.

## Original TUI bug backlog (superseded by Open work above)

| #   | Bug | Status |
|-----|-----|--------|
| TB1 | "session started" row appears twice on session boot | ✅ FIXED — `55e4958` |
| TB2 | Status bar phase stays `streaming` after `SessionEnded` | ✅ FIXED — `55e4958` |
| TB3 | Spinners animate while user scrolls | ✅ FIXED — `55e4958` |

## Cross-version validation notes

Evidence collected during real testing that informs scope decisions — record here so we don't relitigate at version-cut time.

- **2026-05-16 — v0.4 (Advanced TUI UX) markdown rendering and diff viewer confirmed as critical.** Assistant text rendered with literal `**bold**`, raw backticks, and no paragraph wrap. Long unbroken token sequences cluster into walls of text. Confirmed diff/markdown/theme work belongs in v0.4 — shipped in Slices A/B/E.
- **2026-05-17 — Components catalog gap recognized as the real blocker for Ink replacement.** Adapter completeness (Phase 1) + TUI features (Phase 2) close the gap on the *fixed-shape* surfaces (status bar, tool rows, permission widget) but not on *arbitrary host-defined UI* (session pickers, checkpoint pickers, wizards, etc.). Conclusion: Camouflage needs a primitives layer (`ShowSelectList`, `ShowConfirm`, `ShowWizard`, …) — see Open work → "Components catalog (the missing UI primitives layer)".

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
| 2026-05-16 | v0.5 code-complete (partial): Slices A–D shipped (`ca32cee` → `812fdf8`). `camouflage-export` (stored session → portable NDJSON, with `--list`), crash-replay dump on panic (ring buffer flushed to `crash-<ts>.ndjson`), golden-snapshot regression tests (per-fixture `<name>.snapshot.json` checked-in + integration test gate), `camouflage-replay-check` (deterministic CI helper). 63 workspace tests, all green. Event tracing + latency inspection + per-event profiling deferred to v0.5.5. Tagged `v0.5.0`. |
| 2026-05-16 | v0.6 code-complete (partial): Slices A–C shipped (`40fc880` + Node SDK). New `camouflage` Rust facade crate (re-exports + prelude), exhaustive `docs/protocol.md` event reference, Node SDK at `sdk/node/` (ESM + TS types + NDJSON reader + validate + encode, 5 tests passing). 66 Rust workspace tests + 5 Node tests, all green. Integration adapters + plugin API + benchmark CI + migration guides deferred to v0.6.5. Tagged `v0.6.0`. |
| 2026-05-17 | Ink-replacement-ready milestone shipped (tagged `v0.4.5`). Three-phase push: (1) KimiFlare adapter completeness — 5 slices in `~/kimi-code-clone-3` `camouflage-adapter` branch (`8fc1093` → `a8c9108`): tool stdout/stderr emission, multi-turn mode, StatusUpdate (mode/phase/elapsed/tokens/branch), bidirectional permission flow, BackgroundTaskUpdate. (2) Camouflage TUI features — 4 slices on main (`2ced5a4` → `cbcd655`): expandable tool overlay (X), permission feedback input, slash-command picker (with new `SlashCommandsRegistered` event), `@`-mention picker (with new `MentionCandidatesRegistered` event). (3) TUI bug backlog cleared (`55e4958`): TB1/TB2/TB3 all fixed. +3 themes (nord/gruvbox-dark/tokyo-night). Protocol EventType variants 22 → 24. KimiFlare can now realistically swap Ink for Camouflage. |
| 2026-05-17 | Node SDK shipped — `camouflage` npm package (rename from `camouflage-sdk`). Five-commit series on main (`ebf748d` → `d826f89`) wraps the Rust renderer in a Node-native API: `mount() → send() / on() / close()`, hides all subprocess management and NDJSON plumbing. New `--responses-fd` flag on camouflage-tui (`97af9dd`) lets the renderer write outbound NDJSON to a separate fd so stdout can be reserved for rendering to the user's terminal. KimiFlare's `--ui camouflage` mode (`2f1c7de` on `camouflage-adapter`) is the first consumer: `kimiflare --ui camouflage -p "..."` spawns the renderer in-process, user types follow-ups directly in the TUI, permissions round-trip without a single visible pipe. This is the integration shape that paves the way to deleting Ink entirely (Option B). |
| 2026-05-17 | Real `--ui camouflage` test surfaced 7 bugs (TBR-1..7). TBR-1..4 fixed in `dc861bc` (compact tool commands, suppress stale spinners, Esc closes overlays, Up/Down input history). TBR-5..7 still open (see Open work). Same testing surfaced the components-catalog gap: Camouflage has fixed-shape UI but no primitives for arbitrary host-defined UI like session pickers / wizards / forms / tables. Adopted catalog approach (declarative `Show<Component>` events with `id`-keyed responses); 7 components prioritised (CC-1 SelectList highest). PROGRESS.md restructured to put forward-looking "Open work" at the top with per-item IDs / reasoning / where-to-look so conversations don't get lost. |
| 2026-05-17 | CC-1 SelectList shipped end-to-end. 5 commits: protocol+payloads (`c7480b4`), model state + Snapshot (`ff6505e`), TUI overlay + key handling + outbound emit (`0e42e38`), Node SDK `selectList()` helper + `selectListResponse` event (`aac4094`), KimiFlare `kimiflare resume` subcommand as first real driver (`6556771` on `camouflage-adapter`). Protocol EventType variants 24 → 26. First worked example of the components-catalog migration playbook. |
| 2026-05-17 | CC-2 Confirm + CC-3 Toast shipped (`7bc8d43`, `8f3f0d4`). Protocol EventType variants 26 → 29. Both follow the same one-commit-per-component pattern: protocol+payloads → model+snapshot → TUI overlay → Node SDK helper. Three components live now (SelectList / Confirm / Toast); four left in the catalog (Wizard, Form, Table, KeyValueView). Toast is the first display-only primitive (no outbound response). KimiFlare drivers for CC-2/CC-3 are TODO. |
