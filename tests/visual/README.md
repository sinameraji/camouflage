# Visual TUI tests

Scripts a real PTY against `camouflage-tui` via [vhs](https://github.com/charmbracelet/vhs) and emits PNG frames at named checkpoints. Frames are images, so a human (or a multimodal LLM) can verify rendering rather than just inspecting text snapshots.

## Why

Bugs like literal-`\e[31m`-text-instead-of-color, misaligned toast borders, or "Esc didn't actually clear the splash" only show up to a human eye. Text snapshots miss them. These tests close the loop: run a scenario, look at the PNG, judge it.

## Setup

```sh
brew install vhs   # also pulls ffmpeg + ttyd
```

## Run

```sh
tests/visual/run.sh splash
tests/visual/run.sh toasts
tests/visual/run.sh esc-idle
tests/visual/run.sh esc-stream
tests/visual/run.sh host-flow
```

Outputs land in `tests/visual/out/<scenario>/`:
- `01-*.png`, `02-*.png` — checkpoint frames in order
- `session.gif` — full session as a GIF (handy for scrubbing)

## Adding a scenario

1. Drop an NDJSON fixture in `fixtures/` (existing protocol; see `fixtures/all-event-types.ndjson` for the full event vocabulary).
2. Copy `tests/visual/splash.tape` to `tests/visual/<name>.tape`, edit the fixture path, keystroke sequence, and screenshot checkpoints.
3. Run `tests/visual/run.sh <name>` and confirm the frames look right.

## How it works

- `feed.sh` slow-feeds the fixture into the TUI's stdin (`FEED_DELAY_MS`, default 100 ms per line) so frames catch intermediate states.
- The TUI reads NDJSON from stdin (`--stdin-events`), reads keys from the PTY vhs gives it, and renders to that same PTY.
- vhs records the PTY into a GIF and emits PNGs at each `Screenshot` directive.
- Four channels: events-in (stdin), keys-in (PTY), render-out (PTY), responses-out (--responses-fd if needed) — they don't collide, which is what makes this work.

## Not yet

- Golden-image pixel diffs in CI (add once tapes stabilize and rendering is deterministic).
- Host-side integration scenarios (would live in the adapter repo).
