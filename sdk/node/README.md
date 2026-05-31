# camouflage-tui

Node SDK for [Camouflage](https://github.com/sinameraji/camouflage) — a high-performance terminal renderer for AI agent applications.

If you're building an LLM-powered CLI tool and need streaming chat UI, tool execution display, permission modals, forms, and more — without fighting React Ink's re-render performance — this is for you.

```bash
npm install camouflage-tui
```

The `postinstall` script downloads a pre-built native binary for your platform (macOS, Linux). No Rust toolchain required.

## Quick start

```js
import { mount } from "camouflage-tui";

const cam = await mount();

cam.send("SessionStarted", {});
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
a friendly error if the binary can't be found.

```ts
interface MountOptions {
  bin?: string;              // default: bundled binary, then PATH lookup
  args?: string[];           // extra args after --stdin-events --emit-responses
  env?: NodeJS.ProcessEnv;   // merged with process.env
  inheritStderr?: boolean;   // default true; false → "stderr" event
  renderToTerminal?: boolean; // true → stdout goes to terminal, responses on fd 3
}
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
import { mount, selectList, confirm, toast, table, form, wizard } from "camouflage-tui";

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

// Show a brief notification
toast(cam, "Build succeeded");

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

The terminal renderer is a pre-built Rust binary. On install, the
`postinstall` script downloads the binary for your platform from this
package's GitHub Release. It resolves the release tag from the package
version, preferring the component-tagged release
(`camouflage-tui-v<version>`, as produced by release-please) and falling
back to the legacy `v<version>` tag, so both old and new published
versions resolve to a real asset. If the download fails (offline,
unsupported platform), it prints instructions to build from source — the
package install itself never fails.

## Versioning

Versions track the main Camouflage workspace. Within `SCHEMA_VERSION: 1`,
payload fields only grow — never renamed, never semantically changed — so a
newer renderer is always backward compatible with an older SDK.

## License

Apache-2.0.
