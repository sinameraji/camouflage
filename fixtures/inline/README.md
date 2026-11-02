# Inline replay fixtures

Recorded host sessions that `crates/inline/tests/replay.rs` replays through
the inline renderer. Each `<name>.screen.txt` is the expected screen:
everything printed, every dialog as it looked when the user answered it,
and the final live area. A renderer change that alters any of it fails the
test; if the change is intended, rerun with `UPDATE_GOLDEN=1` and review the
diff of the `.screen.txt` files.

The corpus is the primary regression suite for the inline renderer. The VHS
visual harness is for things a screen diff can't express (colour, cursor
position, real terminal quirks); anything that shows up as text belongs here.

## What the test checks

Every fixture is held to three things, not just the golden:

1. **The screen** matches `<name>.screen.txt`.
2. **The idle soak**: after the last event the session is not busy and
   `next_wakeup` returns `None` — an idle renderer must ask for no frames at
   all, forever. A fixture that leaves a spinner, a toast or a background
   task running fails here.
3. **The frame-time budget**: `live_frame` costs the same after 100k lines of
   transcript as after 1k. Printed blocks are dropped, so a regression that
   keeps them shows up as a ratio, not a slow test.

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

Two extra line shapes are understood by the harness:

- `{"t": ms, "resize": [cols, rows]}` — the terminal changed size.
- `{"t": ms, "key": "ctrl+r"}` — a key press that isn't a prompt answer
  (`esc`, `enter`, `tab`, `up`, `down`, `ctrl+o`, `ctrl+b`, `ctrl+u`, `?`, …).
- `{"t": ms, "snapshot": "label"}` — keep the live area as it looks now, for
  states that only exist mid-turn (expanded tools, the shortcuts panel).
- an `out` event whose payload carries `"filter": "text"` — the user typed
  that filter into the open picker before answering, so the golden records
  the list narrowed.

A recording must end with the turn settled (no running tool, no open prompt,
no background task) or the idle soak fails. That is also what a real session
looks like when the user stops typing.

## The corpus

`scripts/gen_inline_fixtures.py` writes the fixtures below. They are
autopilot-shaped rather than captured from a live run, so the corpus is
reviewable and reproducible; a real recording drops in the same way and is
welcome in place of a generated one. Rerun the script after editing it, then
`UPDATE_GOLDEN=1 cargo test -p camouflage-inline --test replay`.

Turns:

- `turn-long-tools` — a long turn: reads, greps, a test run, an edit, a reply.
- `turn-long-output` — a tool that streams 59 lines of output.
- `turn-resize-narrow` / `turn-resize-wide` — the terminal resized mid-turn.
- `turn-interrupt-esc` / `turn-interrupt-ctrlc` — both interrupt paths.
- `turn-error-retry` — a failing tool, a runtime error, then a retry.
- `turn-plan-progress` — a todo list advancing through three states.
- `turn-reasoning` — reasoning deltas, hidden unless Ctrl+R.
- `turn-reasoning-expanded` — the same turn with Ctrl+R pressed.
- `turn-tools-expanded` — Ctrl+O showing full tool output mid-turn.
- `turn-shortcuts` — the `?` shortcuts panel.
- `turn-markdown` — headings, lists, a table, a fenced block, a quote.
- `turn-streaming-long` — a reply streamed paragraph by paragraph.
- `turn-background-tasks` — a task finishing and one failing.
- `turn-agents-browser` — three agents, then the activity browser.
- `turn-terminal-handoff` — `! git status --short` through a suspend/resume.
- `turn-cost-status` — `/cost` as a key/value view plus a footer update.
- `turn-mentions` — `@` candidates and a mention in the prompt.
- `turn-compacted` — a compaction notice mid-session.
- `turn-toast` — success and warning toasts.
- `turn-table-and-kv` — `ShowTable` and `ShowKeyValueView`.
- `turn-patch-proposed` — a proposed patch with a unified diff.

Pickers (one per command):

- `picker-help`, `picker-model`, `picker-hooks`, `picker-sessions`,
  `picker-theme`, `picker-agents`, `picker-mcp`, `picker-permissions`.
- `picker-filter` — a picker narrowed by a typed filter.

Forms and wizards:

- `form-settings` — three text fields.
- `form-multiline` — a multiline field.
- `form-password` — a password field.
- `wizard-setup` — select, confirm and form steps in one flow.
- `wizard-cancel` — cancelled at the first step.

Permissions:

- `permission-diff` — an edit prompt with a diff, approved.
- `permission-plain` — a bash prompt, approved for the session.
- `permission-deny` — denied, which interrupts the turn.

Other:

- `clear-transcript` — `/clear` mid-session.
- `autopilot-pickers` — `/help`, `/model`, `/hooks` from a real autopilot run.
- `autopilot-shell-and-cost` — `! git status --short` (terminal handoff) and
  `/cost` (usage numbers replaced).
- `coding-turn` — a synthetic autopilot-shaped turn: reads, a long test run,
  a failing typecheck, an edit and a markdown reply.

## Fuzzing

`crates/inline/fuzz` holds two `cargo fuzz` targets over `Session::apply` and
`Session::key`. They share their driver with `tests/fuzz_smoke.rs`, which
runs the same code on stable in every CI job, so the "no host input may
panic" guarantee holds even where nightly isn't available.

```bash
cd crates/inline/fuzz
cargo +nightly fuzz run session_apply -- -max_total_time=60
cargo +nightly fuzz run session_key -- -max_total_time=60
```

CI runs both for a fixed 60-second budget on every PR.
