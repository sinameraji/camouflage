# Camouflage — Product Specification & Long-Term Roadmap

> 📜 **HISTORICAL — original roadmap from 2026-05-16.**
> Kept for archeology. **v0.2 through v0.6 have all shipped and are
> tagged on GitHub**, despite the inline `[NOT STARTED]` markers below.
> The live state of the project is in
> [`PROGRESS.md`](../../PROGRESS.md) (work tracker) and
> [`ARCHITECTURE.md`](../../ARCHITECTURE.md) (current design).
> Per-version `[NOT STARTED]` / `[PARTIAL]` markers in this file are
> stale and not maintained — do not trust them.

## Product Definition

Camouflage is a high-performance event-native rendering/runtime layer for long-running AI agent applications.

It provides:

- event protocol
- event persistence
- replay
- virtualization
- scrollback ownership
- terminal rendering
- timeline inspection
- streaming UI infrastructure
- performance instrumentation

for AI agent products.

Camouflage is renderer-focused infrastructure, not an AI agent framework.

---

# One-Line Pitch

> Camouflage is a high-performance event-native rendering engine for long-running AI agent interfaces.

---

# Problem Statement

Current AI agent interfaces are typically built using:

- React
- Ink
- Electron
- webviews
- retained-state architectures

These architectures often degrade under:

- long-running sessions
- streaming updates
- append-heavy transcripts
- concurrent tool execution
- massive histories
- frequent terminal redraws
- mixed stdout/stderr output
- user scrolling during model/tool streaming

Common problems:

- memory accumulation
- rerender storms
- sluggish input
- poor replayability
- terminal flicker
- broken scroll behavior
- auto-scroll fighting the user
- giant retained UI trees
- difficult observability

AI agents behave more like:

- event streams
- logs
- distributed systems
- terminal applications
- observability traces

than traditional CRUD apps.

Camouflage exists to solve this mismatch.

---

# Core Thesis

AI agent sessions are fundamentally event streams.

Therefore, AI agent interfaces should be built around:

- append-only events
- viewport rendering
- bounded memory
- deterministic replay
- terminal ownership
- renderer isolation

not around giant retained UI trees.

---

# Product Philosophy

1. Event log is source of truth.
2. Renderer is a subscriber.
3. Sessions must be replayable.
4. UI memory must remain bounded.
5. Rendering must be viewport-based.
6. Persistence must be append-only.
7. Terminal scrollback is not trusted.
8. Renderer is the only writer to terminal.
9. Integration must be incremental.
10. Agent orchestration remains external.

---

# Core Architecture

```txt
Host Agent
  → emits events
  → Camouflage Runtime
  → Persistence + Virtualization + Rendering
  → TUI / GUI / Inspector / Replay Viewer
```

Host agents own:

- prompts
- reasoning
- tools
- orchestration
- memory
- MCP
- LLM providers

Camouflage owns:

- events
- rendering
- persistence
- replay
- timeline inspection
- scroll behavior
- performance instrumentation

---

# Non-Goals

Camouflage will not become:

- an AI model company
- an orchestration framework
- a planning system
- a memory engine
- a LangChain competitor
- an agent runtime platform
- an IDE
- a model provider abstraction layer

This boundary is strategic.

Camouflage should be usable by:

- Claude Code-like products
- OpenCode-like products
- Goose-like products
- internal enterprise agents
- custom coding agents
- terminal AI tools

without requiring those products to replace their agent logic.

---

# Adoption Strategy

Adoption must be incremental.

A host application should only need to emit structured events.

Primary integration model:

```txt
Agent application
→ emits NDJSON
→ Camouflage renders
```

Example:

```bash
your-agent --emit-events | camouflage-tui --stdin-events
```

Companies should not need to:

- rewrite their agent
- replace their orchestration
- replace their tools
- replace MCP integrations
- replace their model providers

to try Camouflage.

---

# Version Roadmap

## v0.1 — MVP Event-Native TUI  **[IN PROGRESS — ~85% complete, see PROGRESS.md]**

### Goal

Prove the Ink replacement hypothesis.

### Build

- Rust workspace
- event protocol
- SQLite persistence
- NDJSON input
- ratatui TUI
- viewport rendering
- stream rendering
- internal scrollback
- auto-follow behavior
- fake long-session generator
- benchmark suite

### Must Have

- virtualized transcript viewport
- collapsed tool blocks
- active streaming block
- replay from SQLite
- scroll while streaming
- resize while streaming
- no direct stdout writes outside renderer
- frame throttling
- bounded memory

### Metrics

```txt
RSS memory after 5-hour simulated session:
  < 200MB

100k-event replay:
  < 5 seconds

p95 input latency while streaming:
  < 25ms

p95 render frame:
  < 16ms
```

### Definition of Success

A host application can emit events into Camouflage and the resulting UI is materially smoother and more stable than the existing Ink UI.

---

## v0.2 — Replay & Timeline Inspection  **[NOT STARTED]**

### Goal

Make AI sessions inspectable.

### Features

- timeline scrubber
- event inspector
- replay controls
- search
- event filtering
- patch history
- tool timing view
- runtime error inspection
- event snapshots
- replay speed controls

### User Stories

As a developer, I can:

- replay an agent session from the beginning
- jump to a failed tool call
- inspect patch history
- inspect stdout/stderr for a tool call
- filter only errors, tools, or patches
- search historical events

### Definition of Success

Developers use Camouflage not only as a UI, but as a debugging surface for agent behavior.

---

## v0.3 — Renderer Abstraction  **[NOT STARTED]**

### Goal

Support multiple frontends using the same event stream.

### Features

- renderer protocol
- websocket transport
- headless runtime mode
- browser replay viewer
- remote renderer support
- event schema validator
- event fixture system

### Supported Renderers

- TUI renderer
- headless JSON renderer
- browser replay viewer
- future desktop renderer

### Definition of Success

The same event stream can power multiple UIs without changing the host agent.

---

## v0.4 — Advanced TUI UX  **[NOT STARTED]**

### Goal

Build the best terminal UX in the AI agent category.

### Features

- split panes
- vim motions
- command palette
- diff viewer
- sticky tool panels
- session tabs
- timeline minimap
- live metrics
- stream profiler
- tool latency visualization
- collapsible sections
- keyboard-first navigation

### Design Inspiration

- Lazygit
- Helix
- Zellij
- Bloomberg terminals
- cockpit/game UIs
- serious native developer tools

### Definition of Success

The TUI feels faster, more stable, and more inspectable than React Ink or Electron-style agent interfaces.

---

## v0.5 — DevTools Layer  **[NOT STARTED]**

### Goal

Become observability infrastructure for AI agents.

### Features

- event tracing
- timeline debugging
- performance profiling
- renderer profiling
- latency inspection
- event validation
- crash replay
- deterministic replay
- regression fixtures
- session export/import

### Definition of Success

Developers use Camouflage to debug and profile their agent interfaces, even if they do not use the full TUI.

---

## v0.6 — Ecosystem Layer  **[NOT STARTED]**

### Goal

Become adoptable infrastructure for external agent products.

### Features

- Node SDK
- Rust SDK
- protocol docs
- integration adapters
- renderer plugin API
- benchmark suite
- schema validation tooling
- CI benchmark runner
- migration guide from Ink
- migration guide from Electron/webviews

### Definition of Success

External projects integrate Camouflage as their rendering/replay layer.

---

## v0.7 — Desktop Runtime  **[NOT STARTED]**

### Goal

High-performance native desktop experience.

### Features

- native desktop shell
- GPU rendering exploration
- detached replay viewer
- session archive browser
- advanced profiling
- remote stream synchronization
- persistent local session library

### Definition of Success

Camouflage powers native desktop AI agent clients while preserving the same event model.

---

# Terminal Rendering Model

Camouflage should behave like a serious terminal application.

While active:

- it owns the terminal
- it uses alternate screen mode
- it does not trust terminal scrollback
- it maintains its own viewport
- it is the only writer to terminal output

## Scroll Behavior

Auto-follow rules:

```txt
At bottom:
  follow new output

User scrolls up:
  freeze viewport
  show “new output below”

User presses End/Ctrl+E:
  jump to latest
  resume following
```

The viewport must never yank the user back to the bottom while they are reading older output.

## Flicker Prevention

Camouflage must:

- batch token updates
- throttle rendering to 30–60 FPS
- avoid redrawing the full screen unnecessarily
- isolate child process output
- render only changed regions where possible
- preserve viewport during resize

---

# Core Event Schema

Events must be:

- append-only
- serializable
- replayable
- deterministic
- versioned

Core event categories:

- session lifecycle
- user messages
- assistant streaming
- tool execution
- stdout/stderr
- patches
- permissions
- runtime errors
- viewport markers
- compaction/snapshots

---

# Persistence

Initial storage engine:

- SQLite

Requirements:

- WAL mode
- append-only writes
- indexed replay
- snapshots
- range reads
- deterministic reconstruction

---

# Rendering

Rendering must:

- virtualize transcript
- bound memory usage
- avoid full rerenders
- keep input responsive
- lazy-load history
- support collapsed blocks
- support active stream block mutation
- preserve scroll position

---

# Replay

Replay system must support:

- scrubbing
- inspection
- debugging
- timeline reconstruction
- deterministic event order
- eventually branch replay

---

# Benchmark Suite

Camouflage must ship benchmark tooling.

Required metrics:

- memory usage
- frame latency
- replay speed
- event throughput
- input responsiveness
- persistence throughput
- dropped frames
- scroll stability

Example benchmark:

```bash
camouflage-bench --events 100000 --tools 1000 --tokens 50000
```

---

# Security Philosophy

1. Minimal dependencies.
2. No embedded JS runtime.
3. No hidden network calls.
4. Signed releases eventually.
5. Reproducible builds eventually.
6. `cargo-audit` in CI.
7. Dependency review process.
8. No arbitrary plugin execution in MVP.

---

# Dependency Budget

Direct dependencies target:

```txt
v0.1: <= 15
v1.0: < 25
```

Every dependency must be documented.

---

# Competitive Positioning

Camouflage does not compete with:

- Claude Code
- Cursor
- OpenAI
- LangChain
- coding agents themselves

Camouflage competes with:

- retained transcript UIs
- sluggish long-session terminal interfaces
- flickering React Ink apps
- uninspectable agent sessions
- poor replay/debug tooling

Camouflage becomes:

- rendering infrastructure
- replay infrastructure
- observability infrastructure

for AI agents.

---

# Key Product Wedge

The first wedge is simple:

> Replace Ink for long-running AI agent sessions and prove better memory, scrolling, replay, and responsiveness.

Everything else comes after that.

---

# Long-Term Vision

Camouflage becomes the default event/rendering substrate for:

- coding agents
- terminal AI apps
- local-first AI interfaces
- replayable agent sessions
- agent observability tools

The core moat is:

- bounded-memory rendering
- replayability
- virtualization
- scroll correctness
- terminal-native UX
- observability

not:

- models
- prompts
- orchestration
- agent planning
