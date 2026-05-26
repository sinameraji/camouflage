# camouflage (Node SDK)

Node-native API for the [Camouflage](https://github.com/sinameraji/camouflage)
event-native rendering runtime. Spawn the renderer, send events into it,
subscribe to renderer→host outbound events. No subprocess management, no
NDJSON, no stdio — the binding hides all of that.

```bash
npm install camouflage
```

Prereq: the `camouflage-tui` binary must be on `PATH`, or you pass an
absolute path via `{ bin: "/path/to/camouflage-tui" }`. Build it from a
checkout of the main repo:

```bash
cargo install --path crates/tui
```

(Prebuilt binaries via `postinstall` are deferred until we have a real
distribution story — tracked as **ECO-6** in
[`PROGRESS.md`](../../PROGRESS.md).)

## Quick start

```ts
import { mount } from "camouflage";

const cam = await mount();

cam.send("SessionStarted", {});
cam.send("UserMessageCreated", { text: "investigate failing test" });
cam.send("AssistantStreamStarted", { stream_id: "s1" });
cam.send("AssistantTokenDelta", { stream_id: "s1", token: "Hello, " });
cam.send("AssistantTokenDelta", { stream_id: "s1", token: "world." });
cam.send("AssistantMessageCompleted", { stream_id: "s1" });

// listen for user input typed into the renderer
cam.on("userInput", (text) => {
  console.log("user said:", text);
});

// listen for permission responses
cam.on("permissionResponse", ({ request_id, choice, feedback }) => {
  console.log(`${request_id} → ${choice}${feedback ? ` (${feedback})` : ""}`);
});

await cam.close(); // graceful: flush stdin, wait for exit
```

## API

### `mount(opts?) => Promise<CamouflageHandle>`

Spawns the renderer and resolves once the child has spawned. Rejects with
a friendly error if the binary can't be found.

```ts
interface MountOptions {
  bin?: string;              // default "camouflage-tui" (PATH lookup)
  args?: string[];           // extra args after --stdin-events --emit-responses
  env?: NodeJS.ProcessEnv;   // merged with process.env
  inheritStderr?: boolean;   // default true; false → "stderr" event
  skipDefaultArgs?: boolean; // test-only; skip --stdin-events --emit-responses
}
```

### `CamouflageHandle`

Extends `EventEmitter`. Send events with `send()`/`sendEvent()`; subscribe
to outbound events with `on()`.

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
| `"event"`            | any outbound `Event` (raw, including the two above) |
| `"invalid"`          | `{ line, error }` — renderer emitted bad NDJSON  |
| `"stderr"`           | `string` — only when `inheritStderr: false`      |
| `"exit"`             | `{ code, signal }` — renderer exited             |

### Types-only import

Don't need the runtime binding? Import just the protocol types and the
NDJSON parser helpers — zero subprocess overhead:

```ts
import { Event, reader, validate, encode } from "camouflage/types";
```

## Migration from Ink

If you're replacing a React/Ink TUI:

1. **Mount Camouflage** instead of `render(<App/>)`.
2. **Replace state setters** (`setEvents([...e, newEv])`, `setTurnPhase("thinking")`, etc.) with `cam.send(...)` calls. Each event you previously stuffed into React state becomes a `send()` of the equivalent Camouflage event.
3. **Replace `<TextInput onSubmit>`** with `cam.on("userInput", ...)`.
4. **Replace permission UI state** with `cam.on("permissionResponse", ...)`.
5. **Delete `<App/>`, the Ink dependency, and any per-component layout code** once nothing reads from React state anymore. The renderer owns layout.

For a real-world reference, see `~/kimi-code-clone-3` (branch
`camouflage-adapter`) — the `src/emit-mode.ts` module is being rewritten
on top of this binding as the canonical example.

## Versioning

Versions track the main Camouflage workspace. `camouflage@0.4.6` works
with `camouflage-tui` ≥ v0.4.5 (i.e., the v0.4.5 tag and forward).
Within `SCHEMA_VERSION: 1`, payload fields only grow — never renamed,
never semantically changed — so a newer renderer is always backward
compatible with an older binding.

## License

Apache-2.0.
