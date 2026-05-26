# Camouflage browser viewer (v0.3)

A single-file HTML viewer that connects to `camouflage-broadcast` over
WebSocket and renders the live event stream.

## Quick start

```bash
# Terminal 1: pump events to the broadcast server
cargo run --release -p kimiflare-mock | \
  cargo run --release -p camouflage-headless --bin camouflage-broadcast -- --port 8080

# Terminal 2: serve the viewer (any static server works)
python3 -m http.server --directory viewer 8000
# then open http://localhost:8000
```

Or just open `viewer/index.html` directly in a browser — it defaults to
`ws://localhost:8080` and will reconnect when you press the button.

## Why a single static file

No build step, no framework, no transpile. Designed for debugging — you
should be able to drop it on any HTTP server (`python -m http.server`,
GitHub Pages, an S3 bucket) and have a working viewer pointed at any
reachable `camouflage-broadcast` instance.

## What it renders

Mirrors the TUI at the level a debugger needs:

- **Transcript**: rows for user, assistant (with token streaming),
  tool execution (collapsed by default with `✓`/`✗` + exit code),
  patches, permissions, errors, markers.
- **Status bar** at the bottom: segments from `StatusUpdate` in the
  conventional order (`mode · phase · elapsed · tokens · cost · branch · warn`).
- **Task ribbon** above the transcript: live `BackgroundTaskUpdate`
  entries; `done` tasks fade after 1.5s.
- **Permission box**: highlighted callout when a `PermissionRequested`
  is unfulfilled. The viewer accepts a response (allow / deny / cancel)
  and emits a `PermissionResponse` back over the same WebSocket frame.

## Wire protocol

One Camouflage event per WebSocket text frame, exactly the same NDJSON
shape consumed by `camouflage-tui --stdin-events`. Late-joining clients
receive the broadcast server's in-memory replay buffer first, then live
events.
