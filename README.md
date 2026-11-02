# Camouflage

Event-native rendering/runtime layer for long-running AI agent applications.

## What Camouflage is

- An event protocol (append-only, serializable, replayable) — 54 event types covering chat, tools, permissions, slash commands, mentions, background tasks, and per-session status
- A persistence layer (SQLite, WAL) — every event flushed before render
- An inline terminal renderer: finished output is printed once into the terminal's normal scrollback, and only a small live region is redrawn
- A catalog of overlay primitives — SelectList, Confirm, Form, Wizard, Table, KeyValueView, Toast — for hosts to compose drill-down UX
- Replay (`--replay`, `--play`) and a searchable Ctrl+O transcript view, both built from SQLite
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

Replay a stored session (the transcript is printed and the process exits):

```bash
cargo run --release -p camouflage-tui -- --replay <SESSION_UUID>
```

Play a shareable `.camo` file (header line + NDJSON, or plain NDJSON):

```bash
cargo run --release -p camouflage-tui -- --play session.camo
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

### Several sessions on one stream

One stream may carry more than one session. Events from different
`session_id`s interleave freely; `seq` is **per session**, so each session
counts from its own `0`. `SessionStarted` introduces a session and
`SessionEnded` removes it. Single-session hosts change nothing — omit
`session_id` and Camouflage assigns one.

`SessionStatus` carries the state a deck or sidebar shows for one session:

```json
{"session_id":"11111111-1111-4111-8111-111111111111","seq":1,"event_type":"SessionStatus","payload":{"state":"blocked","title":"camouflage","place":"this computer","branch":"feat/deck","detail":"waiting for approval: run tests","cost_usd":0.42,"context_used":12000,"context_limit":200000,"pr_url":"https://github.com/sinameraji/camouflage/pull/1","updated_ms":1762000000000}}
```

`state` is required and is one of `working` | `blocked` | `idle` | `done` |
`failed`. Every other field is optional and omitted when unknown; hosts send
the full status each time (last-write-wins). See
[`docs/protocol.md`](docs/protocol.md#multi-session-streams) and
[`fixtures/multi-session.ndjson`](fixtures/multi-session.ndjson) for a
complete interleaved example.

## UI components

Surfaces the renderer draws natively:

- **Transcript** — user / assistant / system / tool rows with markdown (paragraphs, fenced ``` code, bulleted + numbered lists, tables) and syntax highlighting
- **Busy-turn summary** — turn-scoped elapsed time and token count, compact plan progress, live agents, a bounded trail of reads/edits/shell actions, and the latest action
- **Tool view** — per-tool status glyphs (✓ done, ✗ error, ⊘ cancelled, ⊗ rejected), duration, repeated-signature warning, inline diffs, and collapsed output
- **Permission modal** — vertical 1/2/3 picker with ↑↓ / j/k navigation, `?` help overlay, inline unified `+/-` diff for file edits, free-text feedback that rides along with the response
- **Task ribbon** — background tasks with a celebration window so completed entries linger ~2s with a strikethrough ✓ before sweeping
- **Slash command picker** — fuzzy filter with `[builtin] [project] [global]` source badges
- **Mention picker** — `Recent` then `All files` sections, recent entries marked with `↻ @`
- **Status bar** — mode · timer · cost · branch, plus warn segments (e.g. `/compact recommended` past 80% of the context limit)
- **Toasts** — bottom-right, multiple stack, TTL-driven sweep
- **Catalog overlays** — SelectList, Confirm, Form (text/secret/multiline fields), Wizard (multi-step), Table, KeyValueView; compose host-side for drill-down menus and wizards
- **Transcript view** — Ctrl+O pages the whole session from SQLite in the alternate screen, with `/` search and `n`/`N` navigation

## Keyboard controls

Input and transcript:

| Key            | Action                                  |
|----------------|-----------------------------------------|
| Enter          | submit input as `UserMessageCreated`    |
| Esc            | cancel active stream                    |
| ↑ / ↓          | prompt history                          |
| PgUp / PgDn    | scroll the terminal's own scrollback    |
| Ctrl+O         | open the full transcript view           |
| Ctrl+R         | show / hide model reasoning             |
| Ctrl+B         | background jobs and agents browser      |
| Ctrl+C         | interrupt a turn; twice to exit         |
| Ctrl+D         | exit when the input is empty            |
| Shift+Tab      | switch modes                            |
| `\` + Enter    | newline                                 |

Transcript view:

| Key            | Action                                  |
|----------------|-----------------------------------------|
| ↑ / ↓, j / k   | scroll one line                         |
| PgUp / PgDn    | scroll a page                           |
| Home / End     | top / bottom                            |
| `/`            | search                                  |
| `n` / `N`      | next / previous match                   |
| `q` / Esc      | close                                   |

Permission modal:

| Key            | Action                                  |
|----------------|-----------------------------------------|
| 1 / 2 / 3      | allow once / allow session / deny       |
| ↑ / ↓ / j / k  | move highlight                          |
| Enter          | confirm highlighted option              |
| `?`            | toggle help overlay                     |
| Esc            | deny                                    |
| (any other)    | buffered as feedback on the response    |

## How this differs from React Ink

| Concern          | React Ink                              | Camouflage                                |
|------------------|----------------------------------------|-------------------------------------------|
| State model      | retained UI tree                       | append-only event log + live region       |
| Memory           | grows with transcript length           | flat: printed blocks are dropped          |
| Streaming        | re-renders on each token               | prints finished paragraphs, redraws only the live area |
| Modals           | nested React subtrees                  | one overlay slot, ≈hundreds of bytes      |
| Persistence      | none                                   | SQLite WAL, persisted before render       |
| Replay           | not supported                          | first-class, deterministic                |
| Terminal scrollback | trusted                             | used for finished output; live area is redrawn |
| Output ownership | mixed (tool stdout, model, UI)         | renderer is sole writer; everything else is an event |

See `ARCHITECTURE.md` for details and `BENCHMARKS.md` for the headline numbers.
