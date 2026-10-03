# Inline replay fixtures

Recorded host sessions that `crates/inline/tests/replay.rs` replays through
the inline renderer. Each `<name>.screen.txt` is the expected screen:
everything printed, every dialog as it looked when the user answered it,
and the final live area. A renderer change that alters any of it fails the
test; if the change is intended, rerun with `UPDATE_GOLDEN=1` and review the
diff of the `.screen.txt` files.

## Recording a session

Any host using the Node SDK can record:

```bash
CAMOUFLAGE_RECORD=/tmp/session.ndjson autopilot
```

Each line is `{"t": ms since mount, "in" | "out": event}`: `in` is what the
host sent, `out` is what the renderer reported (submitted prompts, picker
answers). Before committing a recording, remove anything private (paths,
usage numbers, file contents), then drop it here as `<name>.ndjson` and run
the test once to write its `.screen.txt`.

- `autopilot-pickers`: `/help`, `/model`, `/hooks` from a real autopilot run.
- `autopilot-shell-and-cost`: `! git status --short` (terminal handoff) and
  `/cost` (usage numbers replaced).
- `coding-turn`: a synthetic autopilot-shaped turn: reads, a long test run,
  a failing typecheck, an edit and a markdown reply.
