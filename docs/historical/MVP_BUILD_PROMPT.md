# Camouflage MVP — Event-Native Rendering Runtime for AI Agents

> 📜 **HISTORICAL — original build prompt from 2026-05-16.**
> Kept for archeology. Camouflage shipped through v0.6 and beyond since
> this was written; the live state of the project is in
> [`PROGRESS.md`](../../PROGRESS.md) and [`ARCHITECTURE.md`](../../ARCHITECTURE.md).
> Inline `[DONE]` / `[TODO]` / `[NEEDS MANUAL VERIFY]` markers in this
> file are stale and not maintained — do not trust them.

## Objective

Build the MVP of **Camouflage**, a high-performance event-native rendering/runtime layer for long-running AI agent applications.

Camouflage is **not**:

- an AI agent
- an orchestration framework
- a planning system
- a model provider SDK
- a LangChain competitor
- an MCP implementation
- an IDE

Camouflage **is**:

- an event protocol
- a persistence layer
- a viewport-virtualized rendering runtime
- a replay/timeline system
- a terminal-native UI infrastructure layer
- a high-performance replacement candidate for React Ink-style agent interfaces

The core hypothesis:

> React/Ink retained-state rendering architectures degrade during long-running streaming AI agent sessions. Event-native viewport rendering with bounded memory usage can remain performant indefinitely.

This project exists to test that hypothesis.

---

# Product Scope

## Camouflage Owns

- event schema
- event transport
- event persistence
- viewport rendering
- replay system
- timeline system
- streaming renderer
- memory-bounded virtualization
- terminal ownership
- scrollback behavior
- performance instrumentation

## Camouflage Does Not Own

- AI models
- LLM provider integrations
- agent reasoning
- prompting
- tool orchestration logic
- autonomy
- planning systems
- MCP semantics
- memory strategy
- agent framework behavior

Host applications own those concerns.

Camouflage only renders, persists, replays, and inspects what the host application emits.

---

# Primary Integration Target

The first integration target is **KimiFlare**, the existing coding agent/harness currently using React Ink.

The target integration path is:

```txt
KimiFlare agent runtime
→ emits structured NDJSON events
→ Camouflage persists and renders those events
→ user interacts through Camouflage TUI
```

This MVP must make it possible to test whether Camouflage is materially better than the current Ink UI without rewriting KimiFlare’s agent logic.

---

# Success Definition

**Status: [PARTIAL]** — items 1, 2, 4, 6, 7, 8 implemented in code; items 3 (RSS bound over 5h) and 5 (scroll predictability) need a soak run and manual terminal verification. See `PROGRESS.md`.

MVP succeeds only if:

1. KimiFlare can emit NDJSON events into Camouflage.
2. Long sessions remain smooth.
3. Memory remains bounded.
4. Replay works.
5. Scrolling works predictably during streaming.
6. Terminal flicker is eliminated or materially reduced.
7. Input remains responsive during streaming.
8. The UI feels materially more stable than Ink during long-running sessions.

---

# Technical Stack

Language:

- Rust

Terminal UI:

- ratatui
- crossterm

Storage:

- SQLite via rusqlite

Serialization:

- serde
- serde_json

Async runtime:

- tokio

CLI:

- clap

Logging:

- tracing
- tracing-subscriber

---

# Dependency Policy

Use the minimum possible dependencies.

Allowed initial direct dependencies:

- ratatui
- crossterm
- tokio
- serde
- serde_json
- rusqlite
- uuid
- clap
- anyhow
- thiserror
- tracing
- tracing-subscriber

Do not add dependencies casually.

Every dependency must be documented in `DEPENDENCIES.md` with:

- why it exists
- alternatives considered
- maintenance status
- security considerations

Direct dependency target for MVP:

```txt
<= 15 direct dependencies
```

Hard rule:

> Every dependency must justify its existence.

---

# Workspace Structure

Create a Rust workspace:

```txt
camouflage/
  Cargo.toml
  README.md
  ARCHITECTURE.md
  DEPENDENCIES.md
  BENCHMARKS.md
  crates/
    protocol/
    store/
    renderer/
    tui/
    headless/
    bench/
  examples/
    fake-agent/
  docs/
```

---

# Crate: `protocol` **[DONE]**

Implementation: `crates/protocol/src/lib.rs`. All 18 `EventType` variants implemented, typed payload structs, JSON roundtrip tests for every variant.

Define the canonical event schema.

Every event must include:

```rust
pub struct Event {
    pub id: Uuid,
    pub session_id: Uuid,
    pub seq: i64,
    pub timestamp_ms: i64,
    pub schema_version: u32,
    pub event_type: EventType,
    pub payload: serde_json::Value,
}
```

Event schema requirements:

- append-only
- serializable
- replayable
- stable
- versioned
- suitable for NDJSON transport
- suitable for SQLite persistence

## Required Event Types

Implement at minimum:

```txt
SessionStarted
SessionEnded
UserMessageCreated
AssistantStreamStarted
AssistantTokenDelta
AssistantMessageCompleted
ToolExecutionStarted
ToolExecutionStdout
ToolExecutionStderr
ToolExecutionFinished
PatchProposed
PatchApplied
PermissionRequested
PermissionGranted
PermissionDenied
RuntimeError
SessionCompacted
ViewportMarker
```

## Serialization Requirements

- All events must serialize to JSON.
- All events must deserialize from JSON.
- Event JSON must be stable across runs.
- Add tests for every event type.
- Add snapshot tests for representative JSON fixtures.

---

# Crate: `store` **[DONE]**

Implementation: `crates/store/src/lib.rs`. WAL mode, append-only, unique index on `(session_id, seq)`. All five `EventStore` methods. Tests: single/batch append, 100k insert, range read, latest_seq, replay-order determinism.

Implement SQLite event persistence.

## Requirements

- SQLite WAL mode enabled.
- Append-only writes.
- Events indexed by `(session_id, seq)`.
- Event payload stored as JSON text.
- Range reads supported.
- Replay supported.
- Latest sequence lookup supported.
- Batch inserts supported.

## API

Implement an interface equivalent to:

```rust
trait EventStore {
    fn append_event(&self, event: Event) -> Result<()>;
    fn append_batch(&self, events: Vec<Event>) -> Result<()>;
    fn load_session(&self, session_id: Uuid) -> Result<Vec<Event>>;
    fn load_range(&self, session_id: Uuid, from_seq: i64, to_seq: i64) -> Result<Vec<Event>>;
    fn latest_seq(&self, session_id: Uuid) -> Result<i64>;
}
```

## Persistence Rules

```txt
SQLite WAL mode required.
Append-only writes only.
Events must be persisted before broadcast/render.
Compaction must not block streaming.
Replay must reconstruct deterministic event order.
```

## Store Tests

Add tests for:

1. Single event append/load.
2. Batch append/load.
3. 100k event insert.
4. Range read.
5. Latest sequence lookup.
6. Replay order correctness.

---

# Crate: `renderer` **[DONE — with one v0.2 gap]**

Implementation: `crates/renderer/src/{viewport,model}.rs`. Pure logic. Bounded ring buffer (default 2000 rows), one active-stream row mutated in place, collapsed tool map. Tests: token-deltas-single-row, bounded eviction, tool collapse.

Gap: lazy-load older rows from store on scroll-up not yet implemented — once rows are evicted past the 2000-row cap, scrolling past them shows nothing. Tracked as v0.2 follow-up.

Implement the viewport rendering engine.

This crate should contain rendering data structures and logic independent of terminal backend.

## Core Renderer Model

Camouflage is **not** a React-style retained UI tree.

It is:

- event-driven
- viewport-driven
- renderer-owned
- bounded-memory
- append-log-oriented

The renderer must treat the event log as source of truth and maintain only enough state to render the current viewport.

## Renderer Must Store Only

- viewport height
- viewport width
- visible row cache
- scroll offset
- auto-follow state
- input buffer
- active stream buffer
- active tool states
- small bounded render cache

## Renderer Must Not Store

- the full transcript
- all historical events
- unbounded token history
- a giant retained UI tree
- all rendered rows forever

## Viewport State

Implement viewport state equivalent to:

```rust
pub struct ViewportState {
    pub session_id: Uuid,
    pub viewport_height: u16,
    pub viewport_width: u16,
    pub scroll_offset: i64,
    pub auto_follow: bool,
    pub visible_start_seq: i64,
    pub visible_end_seq: i64,
}
```

## Required Renderer Capabilities

- virtualized transcript rendering
- lazy loading older rows from store
- collapsed tool blocks
- active stream rendering
- scrollback rendering
- replay rendering
- status rendering
- input rendering
- viewport preservation during resize

---

# Terminal Rendering Guarantees **[PARTIAL]**

Implemented in `crates/tui/src/{app,draw,input}.rs`: alternate screen, panic-restore hook, renderer-as-sole-stdout-writer, auto-follow rules, 60 FPS dirty-flag throttling, viewport-preserving resize.

Needs manual terminal verification (see `docs/manual-tests.md`): 50k-token scroll-without-flicker, input responsiveness, damage-region profiling.

This section is mandatory. Do not treat it as optional polish.

## Terminal Ownership Rules

Camouflage fully owns terminal rendering while active.

Renderer is the **only subsystem** allowed to write to stdout/stderr while the TUI is running.

All of the following must become events before rendering:

- model streams
- tool logs
- child process stdout
- child process stderr
- errors
- diagnostics
- progress updates
- permission prompts

Direct terminal writes outside the renderer are forbidden.

## Alternate Screen Mode

The TUI must use alternate screen mode, like `vim`, `lazygit`, `htop`, or `helix`.

Requirements:

- clean entry
- clean exit
- terminal restored on quit
- terminal restored on panic where reasonably possible
- no raw stdout pollution

## Scrollback Architecture

Terminal scrollback is **not** the source of truth.

Camouflage maintains its own virtualized scrollback viewport.

Scrolling must load events/rows from the persistence layer, not from terminal history.

## Auto-Follow Rules

Implement exact behavior:

```txt
If user is at bottom:
  auto_follow = true

If user scrolls upward:
  auto_follow = false

While auto_follow = false:
  incoming events are persisted
  incoming events do not move the viewport
  renderer displays “new output below” indicator

User may restore follow mode via:
  End key
  Ctrl+E
  explicit jump-to-latest action
```

This behavior is non-negotiable.

The viewport must never be yanked back to the bottom while the user is reading older output.

## Frame Throttling

Renderer must throttle screen updates.

Token streaming may arrive at arbitrary rates.

Renderer should batch updates and render at:

```txt
30–60 FPS maximum
```

Renderer must not redraw the terminal for every token delta.

## Damage-Region Rendering

Renderer should redraw only changed regions when possible.

Examples of changed regions:

- active assistant stream block
- input bar
- status line
- updated tool row
- “new output below” indicator

Renderer should avoid full-screen redraws unless required by resize, mode switch, or full replay.

## Resize Handling

Terminal resize events must:

- preserve viewport position
- preserve input state
- preserve active stream state
- recalculate visible rows
- avoid transcript corruption
- avoid forced jump to bottom unless auto-follow is true

Resize must not:

- reset scroll position
- corrupt transcript rendering
- duplicate rows
- drop input
- cause uncontrolled flicker

## Mixed Output Ban

No subsystem may bypass the renderer.

Bad:

```txt
tool process prints directly to terminal
model stream writes directly to stdout
renderer writes separately
```

Good:

```txt
tool output → Event → Store → Renderer
model token → Event → Store → Renderer
error → Event → Store → Renderer
```

---

# Crate: `tui` **[DONE]**

Implementation: `crates/tui/`. Binary `camouflage-tui` with `--stdin-events`, `--replay <uuid>`, `--db <path>`, `--fps <n>`. Layout matches spec. All required keybindings. Persist-before-render via mpsc + 16ms batch flush.

Implement terminal UI using `ratatui` + `crossterm`.

## Required Layout

```txt
┌──────────────────────────────────────────────┐
│ Camouflage                                   │
├──────────────────────────────────────────────┤
│ transcript viewport                          │
│ virtualized rows only                        │
│ collapsed tool blocks                        │
│ active streaming block                       │
├──────────────────────────────────────────────┤
│ status: idle | streaming | tool | error       │
├──────────────────────────────────────────────┤
│ input:                                       │
└──────────────────────────────────────────────┘
```

## Required Keyboard Controls

```txt
Enter       submit fake prompt / confirm input
Esc         cancel active stream
Up/Down     scroll line
PgUp/PgDn   scroll page
End         jump to latest and enable auto-follow
Ctrl+E      jump to latest and enable auto-follow
r           replay current session
b           run benchmark stream
q           quit
```

## Required TUI Behavior

- Do not store full transcript in TUI state.
- Input must stay responsive while events stream.
- Tool calls render collapsed by default.
- Streaming assistant output mutates one active render block.
- Scrolling up disables auto-follow.
- New output indicator appears while scrolled up.
- End/Ctrl+E restores auto-follow.
- Resize does not corrupt layout.
- No direct stdout writes outside renderer.

---

# Crate: `headless` **[DONE]**

Implementation: `crates/headless/src/lib.rs`. `NdjsonDecoder` accepts shorthand `{event_type, payload}` lines and fills in missing fields. Parse errors → `RuntimeError` events (never panic).

Implement NDJSON event input/output.

## Required Mode

```bash
camouflage-tui --stdin-events
```

This mode should:

- read newline-delimited JSON events from stdin
- validate event schema
- persist events
- render events in real time

This is the primary integration mechanism for KimiFlare MVP testing.

## Example Input

```json
{"event_type":"SessionStarted","payload":{}}
{"event_type":"UserMessageCreated","payload":{"text":"fix auth bug"}}
{"event_type":"AssistantTokenDelta","payload":{"token":"I"}}
{"event_type":"AssistantTokenDelta","payload":{"token":" can"}}
{"event_type":"ToolExecutionStarted","payload":{"tool":"bash","command":"npm test"}}
```

---

# Crate: `bench` **[DONE — with caveat]**

Implementation: `crates/bench/src/main.rs`. JSON to stdout. Baseline 2026-05-15 in `BENCHMARKS.md`. Caveat: materialises 100k events into a Vec for the write phase, dominating RSS measurement. Streaming-into-store rewrite tracked in `PROGRESS.md`.

Implement benchmark suite.

Required command:

```bash
camouflage-bench \
  --events 100000 \
  --tools 1000 \
  --tokens 50000
```

Measure:

- RSS memory
- replay time
- frame time
- input latency
- event throughput
- SQLite write throughput
- SQLite range-read latency
- dropped frames
- render batches per second

Output machine-readable JSON.

Example:

```json
{
  "events": 100000,
  "rss_mb": 123,
  "replay_ms": 1800,
  "event_write_per_sec": 40000,
  "range_read_ms": 12,
  "p95_frame_ms": 11,
  "p95_input_latency_ms": 18,
  "dropped_frames": 0
}
```

---

# Example: `fake-agent` **[DONE]**

Implementation: `examples/fake-agent/src/main.rs`. Flags `--tokens`, `--tools`, `--duration`, `--seed`, `--fast`. Handles `BrokenPipe` cleanly.

Build an example generator that simulates a long-running AI agent session.

It should simulate:

- user messages
- assistant streaming tokens
- tool calls
- stdout/stderr logs
- patches
- errors
- long sessions
- large histories
- scroll-up while streaming
- resize while streaming

It must not require a real AI model.

Purpose:

- benchmark rendering behavior
- test scroll behavior
- test flicker behavior
- test replay behavior
- test event persistence

---

# Required Benchmarks **[PARTIAL]**

Baseline 2026-05-15 (Apple Silicon, release): replay 72ms ✅, p95 frame 0.009ms ✅, RSS 187MB ✅ (bench-vec-dominated), writes 591k/s. p95 input latency under streaming **NOT MEASURED** (offline bench has no terminal in loop). 5-hour soak **NOT RUN**.

MVP target metrics:

```txt
RSS memory after 5-hour simulated session:
  < 200MB

100k-event replay:
  < 5 seconds

p95 input latency while streaming:
  < 25ms

p95 render frame:
  < 16ms

Renderer memory growth:
  bounded / near-constant
```

If any target is missed, document why in `BENCHMARKS.md`.

---

# Acceptance Tests

- **Event/Persistence [DONE]**: 5/5 covered in `crates/store` tests + protocol roundtrip.
- **Rendering/Viewport [PARTIAL]**: 3/5 in `crates/renderer` tests (single-row mutation, tool collapse, bounded memory). Items 1+2 architecturally enforced but lack explicit property tests.
- **Terminal/Scrolling [NEEDS MANUAL VERIFY]**: documented in `docs/manual-tests.md`; no pty harness yet.

Implement or manually document tests for:

## Event/Persistence

1. Event serialization works.
2. Event deserialization works.
3. SQLite append works.
4. SQLite batch append works.
5. 100k event persistence works.
6. Replay order is deterministic.

## Rendering/Viewport

1. Renderer does not load full transcript.
2. Renderer only renders visible viewport plus bounded cache.
3. Tool logs collapse by default.
4. Active stream mutates one render block.
5. Large transcripts remain scrollable.

## Terminal/Scrolling

1. Stream 50k tokens while continuously scrolling.
   - Expected: stable viewport, no jumpiness, no flicker.

2. Stream while typing rapidly.
   - Expected: responsive cursor, no input lag.

3. Resize terminal during stream.
   - Expected: no corruption, no viewport reset unless auto-follow is true.

4. Emit large tool logs.
   - Expected: collapsed rendering, bounded memory.

5. Scroll upward while events continue arriving.
   - Expected: viewport frozen, “new output below” indicator shown.

6. Press End/Ctrl+E after scrolling up.
   - Expected: jump to latest and resume auto-follow.

7. Child process emits stdout/stderr.
   - Expected: captured as events, never raw printed.

---

# Architecture Rules

1. Event log is source of truth.
2. Renderer is a subscriber.
3. Persistence happens before render/broadcast.
4. Renderer cannot mutate canonical state.
5. UI state must remain bounded.
6. Streaming must not block input.
7. Replay must reconstruct session deterministically.
8. Terminal scrollback is not trusted.
9. Renderer is the only writer to terminal.
10. Auto-follow must be explicit and predictable.

---

# Integration Design

Camouflage must support this architecture:

```txt
KimiFlare
→ emits NDJSON events
→ Camouflage renders them
```

without requiring:

- orchestration rewrite
- model rewrite
- tool rewrite
- MCP rewrite
- memory rewrite

Only rendering/runtime integration.

## KimiFlare Required Integration Mode

KimiFlare should eventually support:

```bash
kimiflare --emit-events
```

which emits NDJSON events instead of directly rendering with Ink.

Camouflage should support:

```bash
kimiflare --emit-events | camouflage-tui --stdin-events
```

This is the first real-world proof point.

---

# Required Deliverables

- `README.md`
- `ARCHITECTURE.md`
- `DEPENDENCIES.md`
- `BENCHMARKS.md`
- `crates/protocol`
- `crates/store`
- `crates/renderer`
- `crates/tui`
- `crates/headless`
- `crates/bench`
- `examples/fake-agent`

## README Must Include

- what Camouflage is
- what Camouflage is not
- how to run demo
- how to run benchmark
- how to feed events from an external Node/TS app
- how this differs from React Ink

## ARCHITECTURE Must Explain

- why event log is source of truth
- why renderer is a subscriber
- how viewport rendering works
- how scrollback works
- how auto-follow works
- how terminal ownership works
- how this differs from React Ink
- how to integrate with KimiFlare

## BENCHMARKS Must Include

- benchmark command
- hardware used
- RSS memory
- replay time
- frame latency
- input latency
- dropped frames
- known bottlenecks

---

# Definition of Done **[PARTIAL]**

All three commands run. Numeric targets met on offline bench (see `BENCHMARKS.md`). Terminal behaviours require manual verification per `docs/manual-tests.md`.

MVP is complete only when all of this works:

```bash
cargo run -p camouflage-tui --example fake-agent
cargo run -p camouflage-bench -- --events 100000 --tools 1000 --tokens 50000
cargo run -p camouflage-tui -- --stdin-events
```

and benchmark results show:

```txt
RSS memory < 200MB
100k event replay < 5s
p95 frame time < 16ms
p95 input latency < 25ms
```

and terminal behavior satisfies:

```txt
scrolling does not jump
streaming does not flicker
input remains responsive
resize does not corrupt UI
stdout/stderr does not bypass renderer
```

If these targets are missed, document why and identify the bottleneck.
