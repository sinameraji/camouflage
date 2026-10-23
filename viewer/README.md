# Camouflage browser viewer (v0.3)

A single-file HTML viewer that connects to `camouflage-broadcast` over
WebSocket and renders the live event stream.

## Quick start

```bash
# Terminal 1: pump events to the broadcast server
cargo run --release -p host-mock | \
  cargo run --release -p camouflage-headless --bin camouflage-broadcast -- --port 8080

# camouflage-broadcast prints an access token on stderr at startup, e.g.
#   camouflage-broadcast: connect with ws://127.0.0.1:8080/?token=<token>
#   camouflage-broadcast: viewer: open viewer/index.html#ws=ws://127.0.0.1:8080&token=<token>

# Terminal 2: serve the viewer (any static server works)
python3 -m http.server --directory viewer 8000
# then open http://localhost:8000/#token=<token>
```

Or open `viewer/index.html#token=<token>` directly in a browser. The viewer
defaults to `ws://localhost:8080`; add `ws=<url>` to the fragment for another
server, or paste the full `ws://…/?token=…` URL into the box and press
Connect.

## Access token

`camouflage-broadcast` replays the whole session to every client, and any
web page you visit can open a WebSocket to `localhost`. So the server only
accepts clients that present its access token, as a `?token=` query
parameter or an `Authorization: Bearer` header; anything else gets
`401 Unauthorized`. The token is random for each run and printed only on
stderr. To keep a stable URL across runs, set one with `--token <value>` or
the `CAMOUFLAGE_BROADCAST_TOKEN` environment variable (use a long random
value). The viewer reads the token from the page's `#fragment`, which
browsers never send to the server hosting the viewer.

## Why a single static file

No build step, no framework, no transpile. Designed for debugging — you
should be able to drop it on any HTTP server (`python -m http.server`,
GitHub Pages, an S3 bucket) and have a working viewer pointed at any
reachable `camouflage-broadcast` instance whose token you have.

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
(with a valid token) receive the broadcast server's in-memory replay
buffer first, then live events.
