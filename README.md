# Camouflage

A high-performance terminal renderer for AI agent applications. A React Ink alternative built for streaming, persistence, and replay.

<!-- TODO: Add terminal GIF/screenshot here -->

## Why not Ink?

If you're building an LLM-powered CLI tool — streaming responses, tool execution, permission prompts — React Ink starts to hurt:

- **Full-tree re-render on every commit**: Each state update triggers a React reconciliation pass, Yoga layout recalculation, and a recursive walk of the entire node tree to rebuild the output buffer. Ink throttles terminal writes to 30 FPS, but the tree walk and buffer rebuild happen on every React commit.
- **No scrollable viewport**: Ink manages cursor position, but has no built-in scrolling. When content exceeds terminal height, rendering breaks ([ink#359](https://github.com/vadimdemedes/ink/issues/359)). Historical output from `<Static>` is pushed to terminal scrollback with no way to navigate it programmatically.
- **No persistence**: Close the terminal, lose the session. There's no built-in way to save or restore state.
- **No replay**: Can't record, inspect, or debug past sessions.

Camouflage takes a different approach:

| Concern            | React Ink                                          | Camouflage                                |
|--------------------|-------------------------------------------------------|-------------------------------------------|
| State model        | retained React component tree                         | append-only event log + bounded view      |
| Rendering          | full tree walk + buffer rebuild per React commit (output throttled to 30 FPS) | mutates one active row in place, 60 FPS   |
| Memory             | application must manage its own transcript state      | bounded (2000-row cap, older rows paged from SQLite) |
| Viewport           | no built-in scrolling; overflows break rendering      | own viewport with scroll, search, bookmarks |
| Persistence        | none                                                  | SQLite WAL, every event persisted before render |
| Replay             | not supported                                         | first-class, deterministic from event log |

## Install

```bash
npm install camouflage-tui
```

Pre-built binaries are downloaded automatically for macOS and Linux. No Rust toolchain required.

## Quick start

```js
import { mount } from "camouflage-tui";

const cam = await mount();

// Start a session
cam.send("SessionStarted", {});

// Show a user message
cam.send("UserMessageCreated", { text: "What files changed?" });

// Stream an assistant response
cam.send("AssistantStreamStarted", { stream_id: "s1" });
cam.send("AssistantTokenDelta", { stream_id: "s1", token: "Let me " });
cam.send("AssistantTokenDelta", { stream_id: "s1", token: "check..." });
cam.send("AssistantMessageCompleted", { stream_id: "s1" });

// Listen for user input
cam.on("userInput", (text) => {
  console.log("user said:", text);
});

await cam.close();
```

## How it works

Camouflage is **event-driven**, not component-driven. Your application emits NDJSON events; Camouflage persists, renders, and manages the terminal. You never touch the terminal directly.

```
Your app (Node/Python/Rust/Go)
    │  emit NDJSON events
    ▼
camouflage-tui (Rust binary)
    ├─ Persists to SQLite (append-only, WAL)
    ├─ Renders to terminal (ratatui, 60 FPS)
    └─ Sends user responses back to your app
```

This means:
- **Any language** can drive the TUI — just emit JSON lines
- **Process isolation** — renderer crash doesn't kill your app
- **Deterministic replay** — replay any session from the event log

## Built-in components

No need to build UI from scratch. Camouflage includes:

| Component | What it does |
|-----------|-------------|
| **Transcript** | Streaming chat with user/assistant/tool/system rows |
| **Status bar** | Configurable segments (mode, phase, tokens, cost, etc.) |
| **Task ribbon** | Background task progress indicators |
| **Permission widget** | Inline approve/deny modal with feedback |
| **SelectList** | Filterable dropdown picker |
| **Confirm** | Yes/no dialog |
| **Form** | Multi-field text/password input |
| **Table** | Tabular data display |
| **Wizard** | Multi-step flow composing select/confirm/form steps |
| **Diff viewer** | Unified-diff rendering with syntax coloring |

Each component is triggered by sending an event and (for interactive ones) listening for the response:

```js
import { selectList, confirm, form } from "camouflage-tui";

const model = await selectList(cam, {
  id: "model-picker",
  prompt: "Choose a model",
  options: [
    { value: "gpt-4", label: "GPT-4" },
    { value: "claude", label: "Claude" },
  ],
});

const confirmed = await confirm(cam, {
  id: "deploy",
  prompt: "Deploy to production?",
});
```

## Event protocol

Camouflage uses a simple NDJSON wire protocol with 40+ event types. The minimum:

```json
{"event_type":"UserMessageCreated","payload":{"text":"hello"}}
{"event_type":"AssistantStreamStarted","payload":{"stream_id":"s1"}}
{"event_type":"AssistantTokenDelta","payload":{"stream_id":"s1","token":"hi"}}
{"event_type":"AssistantMessageCompleted","payload":{"stream_id":"s1"}}
```

Fields like `id`, `session_id`, `seq`, and `timestamp_ms` are auto-filled if absent.

See [docs/protocol.md](docs/protocol.md) for the full reference.

## Theming

7 built-in themes, switchable at runtime with the `T` key:

- default-dark, default-light
- dracula, nord, gruvbox-dark, tokyo-night

Themes use 24-bit RGB semantic colors (accent, user, assistant, tool, error, diff colors, etc.).

## Keyboard controls

| Key            | Action                                  |
|----------------|-----------------------------------------|
| Enter          | Submit input                            |
| Esc            | Cancel active stream                    |
| Up/Down        | Scroll line by line                     |
| PgUp/PgDn      | Scroll page                             |
| End / Ctrl+E   | Jump to latest, re-engage auto-follow   |
| T              | Cycle theme                             |
| ?              | Help overlay                            |
| Ctrl+F         | Search                                  |
| /              | Slash command picker                    |
| @              | Mention picker                          |
| r              | Replay current session                  |
| q / Ctrl+C     | Quit                                    |

## Replay & persistence

Every event is persisted to SQLite before rendering. Sessions can be replayed deterministically:

```bash
camouflage-tui --replay <SESSION_UUID>
```

Replay mode includes play/pause (Space), step forward/backward (arrows), speed control (+/-), and timeline jumping (1-9 for 10%-90%).

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design. The workspace contains:

| Crate | What |
|-------|------|
| `protocol` | Event schema (serde types) |
| `store` | SQLite WAL persistence |
| `renderer` | Pure render logic (RenderModel + Snapshot) |
| `tui` | Terminal UI (ratatui + crossterm) |
| `headless` | Non-TUI tools (record, broadcast, export, validate) |
| `sdk` | Rust SDK facade |

Plus `sdk/node/` for the Node.js binding.

## Building from source

If you prefer to build the renderer yourself:

```bash
git clone https://github.com/sinameraji/camouflage.git
cd camouflage
cargo install --path crates/tui
```

Or pipe NDJSON directly:

```bash
cargo run --release -p fake-agent -- --tokens 50000 --tools 200 | \
  cargo run --release -p camouflage-tui -- --stdin-events
```

## License

Apache-2.0
