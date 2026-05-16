# Manual TUI tests

Run each scenario and verify behavior. These exercise terminal-side behavior
not covered by unit tests (which are pure-logic against `RenderModel` and
`ViewportState`).

## 1. Stream 50k tokens while scrolling

```bash
cargo run --release -p fake-agent -- --tokens 50000 --tools 0 | \
  cargo run --release -p camouflage-tui -- --stdin-events
```

While tokens arrive, press `Up` repeatedly. Expected:
- viewport freezes at the scrolled position
- status line shows `[scrolled] ... | new output below ↓`
- no flicker, no jump to bottom

Press `End`. Expected: jump to latest, status returns to `[follow]`.

## 2. Type rapidly while streaming

Same command. Type a sentence while tokens flow. Expected:
- characters appear in the input box without lag
- input frame redraws without disturbing the transcript

## 3. Resize during stream

Same command. Resize the terminal window. Expected:
- transcript reflows, no row duplication or corruption
- if scrolled up, position is preserved
- if at bottom, follows new output

## 4. Large tool output

```bash
cargo run --release -p fake-agent -- --tokens 0 --tools 200 | \
  cargo run --release -p camouflage-tui -- --stdin-events
```

Expected: tools render as a single collapsed row each with `(exit=0, stdout=…B)`. No per-chunk rows.

## 5. Replay

After running scenario 1 and quitting, note the session id is in the SQLite db at `~/.camouflage/sessions.db`. List sessions:

```bash
sqlite3 ~/.camouflage/sessions.db \
  "SELECT session_id, COUNT(*) FROM events GROUP BY session_id"
```

Replay:

```bash
cargo run --release -p camouflage-tui -- --replay <SESSION_UUID>
```

Expected: transcript reconstructs deterministically; `r` re-runs replay.

## 6. Panic restore

While the TUI is running, send `SIGABRT` to the process (or trigger any panic). Expected: terminal returns to normal cooked mode; cursor visible; no lingering raw-mode artefacts.
