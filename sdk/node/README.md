# camouflage-tui

Node SDK for [Camouflage](https://github.com/sinameraji/camouflage) — a high-performance terminal renderer for AI agent applications.

If you're building an LLM-powered CLI tool and need streaming chat UI, tool execution display, permission modals, forms, and more — without fighting React Ink's re-render performance — this is for you.

```bash
npm install camouflage-tui
```

The package includes a platform-specific optional dependency containing the pre-built renderer (macOS, Linux, or Windows). No Rust toolchain, network download, or install script is required; this also works with `npm install --ignore-scripts`. If your platform package is missing, `mount()` reports which package to install. The SDK never falls back to an unchecked `camouflage-tui` from `PATH`.

## Quick start

```js
import { mount } from "camouflage-tui";

const cam = await mount({ ui: "inline" });

cam.send("SessionStarted", { title: "my-agent", detail: ["model · ~/project"], accent: "orange" });
cam.send("UserMessageCreated", { text: "investigate failing test" });
cam.send("AssistantStreamStarted", { stream_id: "s1" });
cam.send("AssistantTokenDelta", { stream_id: "s1", token: "Looking " });
cam.send("AssistantTokenDelta", { stream_id: "s1", token: "into it..." });
cam.send("AssistantMessageCompleted", { stream_id: "s1" });

// Listen for user input typed into the renderer
cam.on("userInput", (text) => {
  console.log("user said:", text);
});

// Listen for permission responses
cam.on("permissionResponse", ({ request_id, choice, feedback }) => {
  console.log(`${request_id} → ${choice}`);
});

await cam.close();
```

## API

### `mount(opts?) => Promise<CamouflageHandle>`

Spawns the renderer and resolves once the child has spawned. Rejects with
a clear error if the matching platform binary package isn't installed.

```ts
interface MountOptions {
  bin?: string;              // default: matching platform optional dependency
  args?: string[];           // extra args after --stdin-events --emit-responses
  env?: NodeJS.ProcessEnv;   // merged with process.env
  inheritStderr?: boolean;   // default true; false → "stderr" event
  renderToTerminal?: boolean; // true → stdout goes to terminal, responses on fd 3
  ui?: "inline" | "fullscreen"; // default "inline" (since 2.4.0); see below
}
```

`ui: "inline"` prints finished output into the terminal's normal scrollback
and redraws only a small live region (the reply being streamed, the
spinner, the input box). The transcript stays after exit, text selection
works normally, and an idle renderer uses no CPU. It draws on the terminal
(`/dev/tty`) even in the default piped mode, so it works with a plain
`mount({ ui: "inline" })`.

### Helpers that wait for the user

`selectList`, `confirm`, `permission`, `form` and `wizard` send a prompt and
resolve with the user's answer. If the renderer exits first they resolve
as cancelled (`permission` resolves `{ choice: "deny" }`), so a host never
hangs on a dead renderer.

```js
import { permission } from "camouflage-tui";

const { choice } = await permission(cam, {
  request_id: "r1",
  tool: "edit",
  action: "edit src/auth/session.ts",
  diff: { path: "src/auth/session.ts", before, after },
});
```

### `CamouflageHandle`

Extends `EventEmitter`. Send events with `send()`; subscribe to outbound events with `on()`.

| Method                          | What                                            |
|---------------------------------|-------------------------------------------------|
| `send(event_type, payload?)`    | Write one event into the renderer.              |
| `sendEvent({ event_type, payload? })` | Write a pre-built `Event`.                |
| `close()` → `Promise<number>`   | Graceful shutdown; resolves with exit code.     |
| `kill(signal?)`                 | Forceful — only if `close()` is wedged.         |

| Event                | Payload                                           |
|----------------------|---------------------------------------------------|
| `"userInput"`        | `string` — user typed into the input box         |
| `"permissionResponse"` | `{ request_id, choice, feedback? }`             |
| `"selectListResponse"` | `{ id, value?, cancelled }`                     |
| `"confirmResponse"`   | `{ id, value?, cancelled }`                     |
| `"formResponse"`      | `{ id, values?, cancelled }`                    |
| `"wizardCompleted"`   | `{ id, results }`                               |
| `"wizardCancelled"`   | `{ id, at_step }`                               |
| `"cancelRequested"`   | `{}` — user pressed Esc                          |
| `"event"`            | any outbound `Event` (raw)                        |
| `"exit"`             | `{ code, signal }` — renderer exited             |

### Convenience helpers

```js
import { mount, selectList, confirm, table, form, wizard } from "camouflage-tui";

const cam = await mount();

// Show a filterable list picker
const choice = await selectList(cam, {
  id: "model",
  prompt: "Choose a model",
  options: [
    { value: "gpt-4", label: "GPT-4" },
    { value: "claude", label: "Claude" },
  ],
});

// Show a yes/no confirmation
const ok = await confirm(cam, { id: "deploy", prompt: "Deploy to production?" });

// Show a multi-field form
const creds = await form(cam, {
  id: "login",
  title: "API credentials",
  fields: [
    { name: "key", label: "API Key", kind: "text" },
    { name: "secret", label: "Secret", kind: "password" },
  ],
});
```

### Types-only import

Don't need the runtime binding? Import just the protocol types and the
NDJSON parser helpers — zero subprocess overhead:

```ts
import { Event, reader, validate, encode } from "camouflage-tui/types";
```

## Migration from Ink

If you're replacing a React/Ink TUI:

1. **Mount Camouflage** instead of `render(<App/>)`.
2. **Replace state setters** (`setEvents([...e, newEv])`, `setTurnPhase("thinking")`, etc.) with `cam.send(...)` calls.
3. **Replace `<TextInput onSubmit>`** with `cam.on("userInput", ...)`.
4. **Replace permission UI state** with `cam.on("permissionResponse", ...)`.
5. **Delete `<App/>`, the Ink dependency, and any per-component layout code** — the renderer owns layout.

## Native binary

The renderer is shipped in five optional npm packages, one per supported
OS/CPU target. npm selects the matching package using its `os` and `cpu`
metadata, and `camouflage-tui` resolves that package directly at runtime.
No postinstall script, download, or network access is required; installs
with `--ignore-scripts` work normally. The release workflow publishes all
platform packages before the SDK package so its exact-version optional
dependencies are available as soon as the SDK is published. Each package's
version and the SDK's dependency versions are kept in sync by release-please.

Supported targets are macOS arm64/x64, Linux arm64/x64, and Windows x64.
If npm omitted optional dependencies, `mount()` reports the missing package
and the remedy. To use a renderer you built yourself, pass
`mount({ bin: "/path/to/camouflage-tui" })`.

## Versioning

Versions track the main Camouflage workspace. Within `SCHEMA_VERSION: 1`,
payload fields only grow — never renamed, never semantically changed — so a
newer renderer is always backward compatible with an older SDK.

## License

Apache-2.0.
