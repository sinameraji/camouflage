# Manual TUI tests

> 📸 **Many of these now have automated visual coverage.**
> The vhs-based harness at [`tests/visual/`](../tests/visual/) drives the
> TUI through a real PTY and emits PNG frames at named checkpoints —
> see `splash.tape`, `toasts.tape`, `esc-idle.tape`, `esc-stream.tape`,
> `turn-separators.tape`, `kimiflare-splash.tape`,
> `kimiflare-splash-small.tape`, `kimiflare-flow.tape`. Run a scenario
> via `tests/visual/run.sh <name>`. The scenarios below still serve as
> the "ground-truth" interactive checklist (resize, terminal-emulator
> quirks, exit-cleanup), but anything purely visual is faster to verify
> with the harness.

## Quick smoke test — "is it actually rendering?"

```bash
cargo run --release -p fake-agent -- --tokens 50000 --tools 200 --fast \
  | cargo run --release -p camouflage-tui -- --stdin-events
```

You should see (top to bottom):

- `Camouflage session=<8 hex chars>` header
- A transcript: cyan `›` user message, `tok tok …` assistant rows, magenta `⚙ ✓ bash npm test …` collapsed tool rows, gray `· session ended` at the end
- A **status line** like ` idle [follow] rows=12 total=12` — if you don't see this, the layout has regressed
- An `┌─ input ─┐` box at the bottom with a `›` prompt

Interactions to verify:

- Type characters → they appear after `›` in the input box
- `Enter` → input line clears, a new `›` row appears in the transcript with your text
- `Up` / `PgUp` → viewport freezes; status changes to `[scrolled]` and ` | new output below ↓` appears
- `End` or `Ctrl+E` → snaps back to bottom, status returns to `[follow]`
- `q` or `Ctrl+C` → clean exit, terminal restored to your shell prompt with no garbage



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
