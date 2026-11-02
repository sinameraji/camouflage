# Camouflage Architecture

## Core thesis

AI agent sessions are event streams. Camouflage models them as such: an
append-only event log is the canonical source of truth, and every other
component — persistence, the inline renderer, replay, the transcript view —
is a subscriber.

```
Host agent (any coding-agent host, custom, etc.)
        │   emits NDJSON
        ▼
   headless decoder
        │   Event
        ▼
   persist queue ──► SqliteStore (WAL, append-only)
        │
        ▼
   inline session ──► finished blocks printed once into scrollback
                  └─► small live region redrawn in place
```

## Invariants

1. **Event log is source of truth.** The renderer never mutates canonical state.
2. **Persist before render.** Every event is written to SQLite before being shown.
3. **Printed output is final.** A finished block is written into the terminal's
   normal scrollback once and dropped from memory. Only the unfinished block,
   the input and the footer stay live.
4. **Idle means zero work.** The loop wakes for host events, keys and the
   session's own next deadline. With nothing animating there is no timer.
5. **Renderer-only terminal writes.** Tool stdout/stderr, model tokens, errors —
   all become events. Direct stdout/stderr writes during a session are forbidden.

## Inline rendering

`crates/inline` owns the renderer:

- `Session` applies events and holds only unfinished blocks.
- `take_history` renders finished blocks into `Line`s and drops them.
- `live_frame` renders the unfinished block, the input box and the footer.
- `InlineTerminal` prints history above the live region and redraws only the
  live rows that changed.

The live area must always be shorter than the screen. A streaming reply is
printed paragraph by paragraph; running tool output shows only its last few
lines; the busy-turn summary is capped at four rows plus one spinner row.

## Persist-before-render

The TUI runs three coroutines:

1. **Ingestion** — NDJSON decoder on stdin (when `--stdin-events`) or the
   replay loader, both producing `Event` records with monotonic `seq`.
2. **Persistence** — a writer thread batches events (one transaction per
   250 ms), commits them, and answers `Flush` requests from the transcript
   view.
3. **Render loop** — the main task selects between host events, key events and
   the session's next deadline. It draws once per burst, not once per event.

This guarantees that anything visible on screen has already been durably
written.

## Replay

`--replay <SESSION_UUID>` loads the session with `load_session`, applies every
event to a fresh `Session` with tool output expanded, and prints the finished
blocks into scrollback. `--play <FILE>` does the same for a `.camo` container
or plain NDJSON file. Both are non-interactive: the transcript is the output.

## Transcript view

Ctrl+O opens a full-screen pager over the same blocks, built from SQLite. The
app flushes the store writer, loads the session, renders it with tool output
expanded, and pages it in the alternate screen. `/` searches, `n`/`N` move
between matches, `q` closes. Nothing is written to scrollback, so the live
area is unchanged when the view closes.

## How this differs from React Ink

Ink renders a retained tree of React components into a virtual terminal,
diffing on every state change. For long sessions:

- transcript components accumulate and never unmount (memory grows linearly)
- token streams trigger re-renders of the whole transcript subtree
- there is no event log, so replay is impossible
- tool stdout typically bypasses Ink and writes directly, breaking scrollback

Camouflage inverts the relationship: the event log is canonical and the
renderer is a thin subscriber. Finished output leaves the renderer entirely,
so drawing cost depends on the live area, not the length of the session.

## Integrating with a host application

A host keeps its agent runtime, prompts, tools, and orchestration. It only needs to:

1. Add a `--emit-events` mode that writes NDJSON to stdout instead of rendering with Ink.
2. Pipe into `camouflage-tui --stdin-events`.

No SDK is required; the contract is the line protocol described in `README.md`.
