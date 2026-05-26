# Camouflage Architecture

## Core thesis

AI agent sessions are event streams. Camouflage models them as such: an
append-only event log is the canonical source of truth, and every other
component — persistence, renderer, replay viewer — is a subscriber.

```
Host agent (KimiFlare, custom, etc.)
        │   emits NDJSON
        ▼
   headless decoder
        │   Event
        ▼
   persist queue ──► SqliteStore (WAL, append-only)
        │
        ▼
   render queue ──► RenderModel (bounded) ──► ratatui draw
```

## Invariants

1. **Event log is source of truth.** The renderer never mutates canonical state.
2. **Persist before render.** Every event is written to SQLite before being shown.
3. **Bounded renderer memory.** `RenderModel` holds a capped `VecDeque<Row>`; older rows are evicted and paged back from the store when the user scrolls.
4. **Renderer-only terminal writes.** Tool stdout/stderr, model tokens, errors — all become events. Direct stdout/stderr writes during a TUI session are forbidden.
5. **Auto-follow is explicit.** Re-engages only when the user is already at the bottom on new output, or after End / Ctrl+E.

## Viewport rendering

The renderer keeps:

- a bounded ring buffer of `Row` records (default cap: 2000)
- the index of the active streaming assistant row (one row, mutated in place)
- a map of in-flight `ToolState` (collapsed by default; tracks byte counts)
- a `dirty` flag for damage-region rendering

Drawing happens on a `tokio::time::interval` tick (default 60 FPS). The tick checks `model.dirty()` and only redraws when the model has changed. Token bursts that arrive faster than the tick rate coalesce into a single frame.

## Scrollback

Terminal scrollback is not trusted. Camouflage uses crossterm's alternate screen and maintains its own viewport over the row buffer. `ViewportState::scroll_offset` is the distance (in rows) from the bottom; `0` means pinned. Scrolling up sets `auto_follow = false`; scrolling back down to 0 sets it back to `true`. End / Ctrl+E forces both.

While `auto_follow = false`, new events still persist and apply to the model, but the visible window does not move. The status line shows `new output below ↓`.

## Persist-before-render

The TUI runs three coroutines:

1. **Ingestion** — NDJSON decoder on stdin (when `--stdin-events`) or replay loader, both producing `Event` records into a normalisation channel that assigns monotonic `seq` per session.
2. **Persistence** — a writer task batches events from the persistence channel (up to 512 per batch or 16 ms flush window), commits a single SQLite transaction, then forwards the events to the render channel.
3. **Render loop** — the main task selects between key events, persisted events, and the frame ticker. Persisted events update `RenderModel`; the ticker draws if dirty.

This guarantees that anything visible on screen has already been durably written.

## Replay

`--replay <SESSION_UUID>` is implemented by calling `load_session(sid)` and feeding events through `RenderModel::apply` before entering the event loop. Once loaded, the user can scroll, search (Ctrl+F, shipped in v0.2 Slice D), or continue the session.

The bounded row cap means very long sessions don't fit in the rendered buffer simultaneously — the bottom 2000 rows are kept; scrolling up lazily pages older ranges from `load_range` via `prepend_history` (shipped in v0.1 milestone #17).

## How this differs from React Ink

Ink renders a retained tree of React components into a virtual terminal, diffing on every state change. For long sessions:

- transcript components accumulate and never unmount (memory grows linearly)
- token streams trigger re-renders of the whole transcript subtree
- there is no event log, so replay is impossible
- tool stdout typically bypasses Ink and writes directly, breaking scrollback

Camouflage inverts the relationship: the event log is canonical and the renderer is a thin, bounded subscriber. There is no virtual DOM; ratatui's diff is line-oriented and only redraws changed regions.

## Integrating with KimiFlare

KimiFlare keeps its agent runtime, prompts, tools, and orchestration. It only needs to:

1. Add a `--emit-events` mode that writes NDJSON to stdout instead of rendering with Ink.
2. Pipe into `camouflage-tui --stdin-events`.

No SDK is required; the contract is the line protocol described in `README.md`.
