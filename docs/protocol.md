# Camouflage Event Protocol — v1 (SCHEMA_VERSION = 1)

Reference for any host writing an adapter that produces Camouflage NDJSON.
Source of truth: [`crates/protocol/src/lib.rs`](../crates/protocol/src/lib.rs).
The JSON examples below match the Rust serde derives exactly; if you find
a discrepancy, the Rust types win — please open an issue.

---

## On-the-wire shape

Each line of input to `camouflage-tui --stdin-events` (or any other
Camouflage consumer) is one JSON object. The minimum required fields:

```json
{ "event_type": "...", "payload": { ... } }
```

The renderer fills in `id` (random UUID), `session_id` (auto-assigned),
`seq` (monotonic), `timestamp_ms` (now), and `schema_version` (1) if
omitted. Full form (what `camouflage-record` persists and
`camouflage-export` emits):

```json
{
  "id": "f0a1...",
  "session_id": "a2b3...",
  "seq": 42,
  "timestamp_ms": 1747521837000,
  "schema_version": 1,
  "event_type": "AssistantTokenDelta",
  "payload": { "stream_id": "s1", "token": "hello " }
}
```

---

## Direction

Most events flow **host → renderer** (inbound). Two are **renderer → host**
(outbound), emitted on stdout by `camouflage-tui --stdin-events
--emit-responses`:

| Event | Direction |
|---|---|
| `UserInputSubmitted` | Outbound |
| `PermissionResponse` | Outbound |
| *(everything else)* | Inbound |

---

## Event types — payload reference

### Session lifecycle

#### `SessionStarted`
```json
{ "event_type": "SessionStarted", "payload": {} }
```
Marks the beginning of a logical session. Renderer resets phase + clears
ephemeral state. Payload is currently empty; future additions will be
additive.

#### `SessionEnded`
```json
{ "event_type": "SessionEnded", "payload": {} }
```
Marks clean termination. Renderer resets `phase` segment to `"idle"`.

#### `SessionCompacted`
```json
{ "event_type": "SessionCompacted", "payload": { "old_seq": 1000, "new_seq": 1 } }
```
Indicates the host compacted the message history. Renderer renders a
marker row. Payload shape is informational; renderer does not currently
re-key seqs.

---

### Conversation

#### `UserMessageCreated`
```json
{ "event_type": "UserMessageCreated", "payload": { "text": "investigate failing test" } }
```
A user message added to the transcript by the host. Distinct from
`UserInputSubmitted` (which is renderer → host).

#### `AssistantStreamStarted`
```json
{ "event_type": "AssistantStreamStarted", "payload": { "stream_id": "s1" } }
```
A new assistant message has begun streaming. `stream_id` keys subsequent
`AssistantTokenDelta` and `AssistantMessageCompleted`.

#### `AssistantTokenDelta`
```json
{ "event_type": "AssistantTokenDelta", "payload": { "stream_id": "s1", "token": "hello " } }
```
A single token (or chunk) appended to the active stream. Renderer
concatenates onto the active assistant row in place.

#### `AssistantMessageCompleted`
```json
{ "event_type": "AssistantMessageCompleted", "payload": { "stream_id": "s1" } }
```
Closes the stream identified by `stream_id`. Renderer stops the spinner
and finalises the row.

---

### Tool execution

#### `ToolExecutionStarted`
```json
{
  "event_type": "ToolExecutionStarted",
  "payload": { "tool_id": "t1", "tool": "bash", "command": "npm test" }
}
```
The host began invoking a tool. Renderer adds a collapsed tool row.
`tool_id` correlates subsequent stdout/stderr/finished events.

#### `ToolExecutionStdout`, `ToolExecutionStderr`
```json
{ "event_type": "ToolExecutionStdout", "payload": { "tool_id": "t1", "chunk": "FAIL src/auth/login.test.ts\n" } }
{ "event_type": "ToolExecutionStderr", "payload": { "tool_id": "t1", "chunk": "warn\n" } }
```
Streaming chunks from a running tool. Renderer accumulates byte counts;
content is stored but not shown inline by default (`i` opens the
inspector to see raw events).

#### `ToolExecutionFinished`
```json
{
  "event_type": "ToolExecutionFinished",
  "payload": { "tool_id": "t1", "exit_code": 0 }
}
```
Closes a tool execution. Renderer flips the row's spinner to `✓` (exit
0) or `✗` (non-zero), and shows the final byte counts.

---

### Patches

#### `PatchProposed`
```json
{
  "event_type": "PatchProposed",
  "payload": {
    "path": "src/auth/login.ts",
    "added": 3,
    "removed": 1,
    "diff": "@@ -1,3 +1,4 @@\n use foo;\n-let x = 1;\n+let x = 2;\n+let y = 3;\n"
  }
}
```
The host proposes an edit. The optional `diff` field (unified diff
format) is split per-line into color-coded `RowKind::Diff` rows in the
renderer, truncated to 40 lines.

#### `PatchApplied`
```json
{ "event_type": "PatchApplied", "payload": { "path": "src/auth/login.ts" } }
```
The patch landed. Renderer renders a system confirmation row.

---

### Permissions

#### `PermissionRequested` (inbound)
```json
{
  "event_type": "PermissionRequested",
  "payload": {
    "request_id": "perm-1",
    "tool": "edit",
    "action": "apply patch to src/auth/login.ts",
    "detail": "3 lines changed in the token-validation branch"
  }
}
```
Renderer displays the inline permission widget (`[1]/[2]/[3]/[Esc]`).
User response comes back as `PermissionResponse` (outbound).

#### `PermissionGranted`, `PermissionDenied` (inbound)
```json
{ "event_type": "PermissionGranted", "payload": { "request_id": "perm-1" } }
{ "event_type": "PermissionDenied",  "payload": { "request_id": "perm-1" } }
```
Host-side acknowledgement of a permission decision (whether autonomous
or user-driven). Renderer renders a system row.

#### `PermissionResponse` (outbound — renderer → host)
```json
{
  "event_type": "PermissionResponse",
  "payload": {
    "request_id": "perm-1",
    "choice": "allow_once",      // or "allow_session" | "deny"
    "feedback": null              // optional free-text
  }
}
```
Emitted on stdout (when `--emit-responses` is set) when the user picks
a button in the permission widget.

---

### Status & background tasks

#### `StatusUpdate`
```json
{
  "event_type": "StatusUpdate",
  "payload": {
    "segments": {
      "mode": "edit",
      "phase": "thinking",
      "elapsed": "1m 23s",
      "tokens": "in 12k",
      "cost": "$0.03",
      "branch": "main",
      "warn": ""
    }
  }
}
```
Status-bar segment updates. Renderer maintains a key→value map; an
empty value removes the segment. Conventional keys (renderer treats
them as well-known when composing the line, in this order):
`mode`, `phase`, `elapsed`, `tokens`, `cost`, `branch`, `warn`. Unknown
keys are still displayed in registration order.

#### `BackgroundTaskUpdate`
```json
{
  "event_type": "BackgroundTaskUpdate",
  "payload": {
    "task_id": "skills",
    "label": "indexing skills",
    "state": "running",            // "running" | "done" | "error"
    "progress": 0.5                 // 0.0..=1.0 or omitted
  }
}
```
Long-running background task lifecycle (skill indexing, memory load,
etc.). Renderer shows active tasks in a ribbon above the status bar.
`done` tasks fade after a short delay.

---

### Errors

#### `RuntimeError`
```json
{
  "event_type": "RuntimeError",
  "payload": {
    "message": "daily token budget exhausted",
    "source": "openai",
    "kind": "quota_exhausted",     // optional: "generic" | "api_error" | "service_ended" | "quota_exhausted"
    "severity": "error",           // optional: "info" | "warn" | "error" | "fatal"
    "cta": { "label": "type /report", "action_id": "report" }   // optional
  }
}
```
Renderer picks a draw style from `kind`. `cta` (when present) shows a
call-to-action beneath the error row.

---

### Renderer → host (outbound)

#### `UserInputSubmitted`
```json
{ "event_type": "UserInputSubmitted", "payload": { "text": "investigate failing test" } }
```
Emitted on stdout when the user hits Enter in the input box. Hosts
should accept this on stdin (or fd 3 with `--responses-fd`) and route
it into their agent's input pipeline.

---

### Misc

#### `ViewportMarker`
```json
{ "event_type": "ViewportMarker", "payload": { "label": "end of warmup" } }
```
Display-only marker row. Useful for bookmarking phases of a session.

---

## Wire-protocol guarantees

1. **Additive evolution**. Within `schema_version: 1`, payload fields
   only get added — never renamed, never have semantics changed. New
   fields are always optional with sensible defaults.
2. **Lenient inbound**. The renderer's `run_reader` path turns parse
   errors into `RuntimeError` events so the UI keeps moving. For
   strict validation, use `camouflage-validate`.
3. **Outbound is line-delimited JSON on stdout** when
   `--emit-responses` is set. Hosts piping into Camouflage should also
   consume Camouflage's stdout to receive `UserInputSubmitted` and
   `PermissionResponse`.

---

## Reference implementations

- **`examples/kimiflare-mock`** — exercises every event type as a
  synthetic event generator. Use this to verify a new consumer.
- **`examples/fake-agent`** — minimal load generator for the bench
  binary.
- **Real KimiFlare adapter** — lives off-tree in `~/kimi-code-clone-3`
  on branch `camouflage-adapter` (see `PROGRESS.md`).

## Conformance test

Any adapter should round-trip cleanly through:

```
your-adapter | camouflage-validate
```

If `--validate` exits 0, your wire output is protocol-conformant.
For visual conformance, additionally compare snapshots:

```
your-adapter > session.ndjson
camouflage-replay-check session.ndjson > golden.json
# (later, after adapter changes)
camouflage-replay-check session.ndjson --golden golden.json
```
