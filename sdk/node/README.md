# camouflage-sdk (Node)

TypeScript / JavaScript SDK for the [Camouflage](https://github.com/sinameraji/camouflage)
event protocol. Pure ESM, zero runtime dependencies, ships TS types.

## Install

```bash
npm install camouflage-sdk
```

## Quick start — consuming a Camouflage stream

```ts
import { reader } from "camouflage-sdk";

for await (const ev of reader(process.stdin)) {
  if (ev.event_type === "AssistantTokenDelta") {
    process.stdout.write(ev.payload.token);
  }
}
```

## Emitting a Camouflage stream from a Node agent

```ts
import { encode } from "camouflage-sdk";

console.log(encode({ event_type: "SessionStarted", payload: {} }));
console.log(encode({
  event_type: "UserMessageCreated",
  payload: { text: "investigate failing test" },
}));
console.log(encode({
  event_type: "AssistantTokenDelta",
  payload: { stream_id: "s1", token: "Hello " },
}));
// pipe stdout through `camouflage-tui --stdin-events`
```

The TypeScript types in `index.d.ts` are a tagged union on `event_type`,
so a `switch` is exhaustive — your TS compiler will catch a missed
event type.

## Strict validation

`validate(line)` returns `null` on success or a string error message:

```ts
import { validate } from "camouflage-sdk";
const err = validate(line);
if (err) throw new Error(`bad event: ${err}`);
```

This is a quick type/shape check. For full CI-grade validation
(payload-field types, enum value lists), run the Rust
`camouflage-validate` binary on your output — it's the source of truth.

## Protocol reference

The complete event-type and payload reference is in
[docs/protocol.md](https://github.com/sinameraji/camouflage/blob/main/docs/protocol.md)
in the main Camouflage repo. The types in this package mirror the Rust
types in `crates/protocol/src/lib.rs` exactly.

## Versioning

Within `schema_version: 1`, payload fields only grow — never renamed,
never semantically changed. New fields are always optional with sensible
defaults, so existing code keeps working across SDK upgrades.

## License

Apache-2.0.
