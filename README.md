# Camouflage

Event-native rendering/runtime layer for long-running AI agent applications.

## What Camouflage is

- An event protocol (append-only, serializable, replayable) — 39 event types covering chat, tools, permissions, slash commands, mentions, background tasks
- A persistence layer (SQLite, WAL) — every event flushed before render
- A viewport-virtualized terminal renderer (ratatui) with bounded memory
- A catalog of overlay primitives — SelectList, Confirm, Form, Wizard, Table, KeyValueView, Toast — for hosts to compose drill-down UX
- A replay/timeline substrate with playback controls + inline event inspector
- A Node SDK (`camouflage` on npm) that wraps the renderer subprocess and the protocol
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

## Node SDK

```js
import { mount, selectList, confirm, form, toast } from "camouflage";

const cam = await mount({ renderToTerminal: true });

cam.emit("UserMessageCreated", { text: "hello" });

const choice = await selectList(cam, {
  title: "Pick a theme",
  options: themes.map(t => ({ value: t.id, label: t.name })),
});

if (await confirm(cam, { title: "Apply?", yes: "Apply", no: "Cancel" })) {
  toast(cam, { kind: "info", text: `Theme set to ${choice}`, ttl_ms: 3000 });
}
```

The SDK exposes `mount`, `selectList`, `confirm`, `form`, `wizard`, `table`, `keyValueView`, and `toast`. Each modal helper returns a Promise that resolves with the user's response — host code stays straight-line and avoids reconciliation overhead.

## Feeding events from an external Node/TS app

Emit one JSON object per line on stdout. The minimum required fields:

```json
{"event_type":"UserMessageCreated","payload":{"text":"hello"}}
{"event_type":"AssistantStreamStarted","payload":{"stream_id":"s1"}}
{"event_type":"AssistantTokenDelta","payload":{"stream_id":"s1","token":"hi"}}
{"event_type":"AssistantMessageCompleted","payload":{"stream_id":"s1"}}
```

`id`, `session_id`, `seq`, and `timestamp_ms` are filled in by Camouflage if absent.

### Several sessions in one stream

One NDJSON stream may carry several sessions. Events from different
`session_id`s may interleave in any order; `seq` is per session, not per
stream, so each session's events number `0, 1, 2, …` independently.
`SessionStarted` introduces a session and `SessionEnded` removes it. A
single-session host changes nothing — omit `session_id` and every event
belongs to the one session Camouflage created for you.

A host that renders several sessions (the deck) reports each one's state
with `SessionStatus`. The event's `session_id` says which session it
describes; the payload is last-write-wins, so send the whole state each
time:

```json
{"event_type":"SessionStatus","session_id":"11111111-1111-4111-8111-111111111111","payload":{"state":"blocked","title":"camouflage","place":"this computer","branch":"aster/154f59c9","detail":"waiting for approval to run tests","cost_usd":0.31,"context_used":48000,"context_limit":200000,"pr_url":"https://github.com/sinameraji/camouflage/pull/149","updated_ms":1793000001100}}
```

`state` is one of `working`, `blocked`, `idle`, `done`, `failed`. Every
other field is optional: `title` (e.g. the repo), `place`, `branch`,
`detail` (why blocked/failed), `cost_usd`, `context_used` /
`context_limit`, `pr_url`, and `updated_ms`. Renderers that show one
session ignore it.

See [`fixtures/multi-session.ndjson`](fixtures/multi-session.ndjson) for a
two-session interleaved stream, and `docs/protocol.md` for the full
reference.

## UI components

Surfaces the renderer draws natively:

- **Transcript** — user / assistant / system / tool rows with hand-rolled markdown (paragraphs, fenced ``` code, bulleted + numbered lists)
- **Busy-turn summary** — turn-scoped elapsed time and token count, compact plan progress, live agents, a bounded trail of reads/edits/shell actions, and the latest action; Ctrl+O expands tool detail
- **Tool view** — per-tool status glyphs (✓ done, ✗ error, ⊘ cancelled, ⊗ rejected), duration, repeated-signature warning, optional captured stdout overlay
- **Permission modal** — vertical 1/2/3 picker with ↑↓ / j/k navigation, `?` help overlay, inline unified `+/-` diff for file edits, free-text feedback that rides along with the response
- **Task ribbon** — background tasks with a celebration window so completed entries linger ~2s with a strikethrough ✓ before sweeping
- **Slash command picker** — fuzzy filter with `[builtin] [project] [global]` source badges
- **Mention picker** — `Recent` then `All files` sections, recent entries marked with `↻ @`
- **Status bar** — mode · timer · cost · branch, plus warn segments (e.g. `/compact recommended` past 80% of the context limit)
- **Toasts** — bottom-right, multiple stack, TTL-driven sweep
- **Catalog overlays** — SelectList, Confirm, Form (text/secret/multiline fields), Wizard (multi-step), Table, KeyValueView; compose host-side for drill-down menus and wizards
- **Replay UI** — play/pause/step/speed controls, scrub timeline, side-panel event inspector with kind filter

## Keyboard controls

Transcript / input:

| Key            | Action                                  |
|----------------|-----------------------------------------|
| Enter          | submit input as `UserMessageCreated`    |
| Esc            | cancel active stream                    |
| ↑ / ↓          | scroll line (frozen anchor while away from bottom) |
| PgUp / PgDn    | scroll page                             |
| Home / End     | scroll to top / jump to latest          |
| Ctrl+E         | jump to latest (or end-of-line when typing) |
| Ctrl+A         | start-of-line when typing               |
| Ctrl+W / Ctrl+U| delete word / line before cursor        |
| Ctrl+F         | open inline search                      |
| Ctrl+C         | quit                                    |

Overlays (empty input only, so they don't fire mid-prose):

| Key | Action                                |
|-----|---------------------------------------|
| `?` | toggle help overlay                   |
| `M` | toggle live metrics (events/s, frame, RSS) |
| `T` | cycle theme                           |
| `X` | toggle most-recent tool's stdout/stderr overlay |

Permission modal:

| Key            | Action                                  |
|----------------|-----------------------------------------|
| 1 / 2 / 3      | allow once / allow session / deny       |
| ↑ / ↓ / j / k  | move highlight                          |
| Enter          | confirm highlighted option              |
| `?`            | toggle help overlay                     |
| Esc            | deny                                    |
| (any other)    | buffered as feedback on the response    |

Replay (with empty input, `--replay` mode):

| Key      | Action                  |
|----------|-------------------------|
| Space    | play / pause            |
| ← / →    | step backward / forward |
| `+` / `-`| faster / slower         |
| `0`      | restart                 |

## How this differs from React Ink

| Concern          | React Ink                              | Camouflage                                |
|------------------|----------------------------------------|-------------------------------------------|
| State model      | retained UI tree                       | append-only event log + bounded view      |
| Memory           | grows with transcript length           | bounded (row cap, evict-then-page-back)   |
| Streaming        | re-renders on each token               | mutates one active row, 30–60 FPS throttle|
| Modals           | nested React subtrees                  | one overlay slot, ≈hundreds of bytes      |
| Persistence      | none                                   | SQLite WAL, persisted before render       |
| Replay           | not supported                          | first-class, deterministic                |
| Terminal scrollback | trusted                             | not trusted — own viewport                |
| Output ownership | mixed (tool stdout, model, UI)         | renderer is sole writer; everything else is an event |

See `ARCHITECTURE.md` for details and `BENCHMARKS.md` for the headline numbers.
