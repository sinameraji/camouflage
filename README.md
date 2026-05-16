# Camouflage

Event-native rendering/runtime layer for long-running AI agent applications.

## What Camouflage is

- An event protocol (append-only, serializable, replayable)
- A persistence layer (SQLite, WAL)
- A viewport-virtualized terminal renderer (ratatui)
- A replay/timeline substrate
- A high-performance candidate replacement for React Ink in agent UIs

## What Camouflage is not

- An AI agent, orchestration framework, planner, LLM SDK, or MCP implementation.

Host applications own those concerns. Camouflage only renders, persists, replays, and inspects what the host emits.

## Quick start

Pipe NDJSON events from any source into the TUI:

```bash
cargo run --release -p fake-agent -- --tokens 50000 --tools 200 | \
  cargo run --release -p camouflage-tui -- --stdin-events
```

Run the benchmark:

```bash
cargo run --release -p camouflage-bench -- --events 100000 --tools 1000 --tokens 50000
```

Replay a stored session:

```bash
cargo run --release -p camouflage-tui -- --replay <SESSION_UUID>
```

## Feeding events from an external Node/TS app

Emit one JSON object per line on stdout. The minimum required fields:

```json
{"event_type":"UserMessageCreated","payload":{"text":"hello"}}
{"event_type":"AssistantStreamStarted","payload":{"stream_id":"s1"}}
{"event_type":"AssistantTokenDelta","payload":{"stream_id":"s1","token":"hi"}}
{"event_type":"AssistantMessageCompleted","payload":{"stream_id":"s1"}}
```

`id`, `session_id`, `seq`, and `timestamp_ms` are filled in by Camouflage if absent.

## Keyboard controls

| Key            | Action                                  |
|----------------|-----------------------------------------|
| Enter          | submit input as `UserMessageCreated`    |
| Esc            | cancel active stream                    |
| Up/Down        | scroll line                             |
| PgUp/PgDn      | scroll page                             |
| End / Ctrl+E   | jump to latest, re-engage auto-follow   |
| r              | replay current session                  |
| q / Ctrl+C     | quit                                    |

## How this differs from React Ink

| Concern          | React Ink                              | Camouflage                                |
|------------------|----------------------------------------|-------------------------------------------|
| State model      | retained UI tree                       | append-only event log + bounded view      |
| Memory           | grows with transcript length           | bounded (row cap, evict-then-page-back)   |
| Streaming        | re-renders on each token               | mutates one active row, 30–60 FPS throttle|
| Persistence      | none                                   | SQLite WAL, persisted before render       |
| Replay           | not supported                          | first-class, deterministic                |
| Terminal scrollback | trusted                             | not trusted — own viewport                |
| Output ownership | mixed (tool stdout, model, UI)         | renderer is sole writer; everything else is an event |

See `ARCHITECTURE.md` for details.
