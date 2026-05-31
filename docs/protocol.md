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

Most events flow **host → renderer** (inbound). A handful are **renderer
→ host** (outbound), emitted on stdout by `camouflage-tui
--stdin-events --emit-responses` (or fd 3 with `--responses-fd 3` when
stdout is reserved for rendering):

| Event | Direction |
|---|---|
| `UserInputSubmitted` | Outbound |
| `PermissionResponse` | Outbound |
| `SelectListResponse` | Outbound (CC-1) |
| `ConfirmResponse` | Outbound (CC-2) |
| `FormResponse` | Outbound (CC-5) |
| `WizardCompleted` | Outbound (CC-4) |
| `WizardCancelled` | Outbound (CC-4) |
| `ModeChangeRequested` | Outbound |
| `CancelRequested` | Outbound |
| *(everything else)* | Inbound |

---

## Event types — payload reference

### Session lifecycle

#### `SessionStarted`
```json
{
  "event_type": "SessionStarted",
  "payload": {
    "user_label": "You",
    "assistant_label": "your-agent"
  }
}
```
Marks the beginning of a logical session. Renderer resets phase + clears
ephemeral state.

Optional payload fields (v0.5+):
- `user_label` (string) — label rendered before each user turn in the
  transcript. Defaults to `"You"`. Trimmed; empty strings are ignored.
- `assistant_label` (string) — label rendered before each assistant
  turn (e.g. `"your-agent"`, `"Claude"`). Defaults to `"Assistant"`.

Both labels can be re-set on any subsequent `SessionStarted` so an
adapter can update branding mid-session without restarting the TUI.

#### `Splash` (v0.4.9+)
```json
{ "event_type": "Splash", "payload": { "text": "ANSI logo text…" } }
```
Host-supplied splash banner pinned above the transcript until the user
submits their first prompt. `text` may contain ANSI SGR escapes (24-bit
RGB fg/bg, default-bg reset). Visually-empty leading/trailing rows are
stripped by the renderer; the remaining height is capped at
`min(area.height - 6, 30)` rows to leave room for the transcript.
Subsequent `Splash` events replace the pinned content (the host owns
its lifecycle).

#### `TranscriptCleared`
```json
{ "event_type": "TranscriptCleared", "payload": {} }
```
The host wiped the message history (e.g. `/clear`). Renderer drops all
ring-buffer rows and pins a "history floor" so scrolling up won't refetch
the wiped rows from the persisted store.

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

### Pickers & registries (v0.4.6+)

#### `SlashCommandsRegistered`
```json
{
  "event_type": "SlashCommandsRegistered",
  "payload": {
    "commands": [
      { "name": "compact", "description": "compact the session" },
      { "name": "clear", "description": "clear the transcript" }
    ]
  }
}
```
Host registers the slash-command list used by the renderer's `/` picker
overlay. Re-registering replaces the list. Each entry may carry an
optional `source` badge (e.g. `"project"`, `"global"`) shown in the
picker so users can distinguish overrides.

#### `MentionCandidatesRegistered`
```json
{
  "event_type": "MentionCandidatesRegistered",
  "payload": {
    "candidates": [
      { "token": "src/auth/login.ts", "kind": "file" },
      { "token": "AuthGuard", "kind": "symbol" }
    ]
  }
}
```
Host registers candidates for the `@`-mention picker. Re-registering
replaces the list.

---

### Components catalog (v0.4.6+, see [`docs/historical/components-catalog.md`](historical/components-catalog.md) for design)

Each component is a single inbound `Show<Component>` event carrying a
unique `id`, optionally paired with an outbound `<Component>Response`
event keyed by the same `id`. Renderer owns layout/theme; host owns the
data and the eventual handling of the response.

#### `ShowSelectList` (CC-1) + `SelectListResponse` (outbound)
```json
{
  "event_type": "ShowSelectList",
  "payload": {
    "id": "resume-1",
    "prompt": "Resume which session?",
    "options": [
      { "value": "sess-abc", "label": "fix auth" },
      { "value": "sess-def", "label": "refactor router" }
    ],
    "default": "sess-abc",
    "allow_filter": true,
    "allow_cancel": true
  }
}
```
```json
{
  "event_type": "SelectListResponse",
  "payload": { "id": "resume-1", "value": "sess-abc" }
}
```
Renderer pops a modal SelectList overlay. User's pick (or cancel) comes
back keyed by `id`. Used by a host's resume picker, session picker,
theme picker, model picker.

#### `ShowConfirm` (CC-2) + `ConfirmResponse` (outbound)
```json
{
  "event_type": "ShowConfirm",
  "payload": {
    "id": "quit-1",
    "prompt": "Save before quitting?",
    "yes_label": "Save",
    "no_label": "Discard",
    "default": "yes"
  }
}
```
```json
{ "event_type": "ConfirmResponse", "payload": { "id": "quit-1", "value": true } }
```
Two-button yes/no modal. `PermissionRequested` is structurally a
specialised `ShowConfirm`; both ship as their own events because
permissions have additional UX (free-text feedback, multi-action).

#### `ShowToast` (CC-3)
```json
{
  "event_type": "ShowToast",
  "payload": { "text": "Saved", "kind": "success", "ttl_ms": 1500 }
}
```
Brief non-modal notification. `kind` is one of `info` / `success` /
`warn` / `error`. `ttl_ms` defaults to 3000. Toast auto-dismisses on TTL
expiry; no response event.

#### `ShowTable` (CC-6)
```json
{
  "event_type": "ShowTable",
  "payload": {
    "id": "usage-1",
    "title": "Cost — last 7 days",
    "columns": [
      { "name": "day", "label": "Day" },
      { "name": "cost", "label": "Cost ($)", "align": "right" }
    ],
    "rows": [
      { "day": "Mon", "cost": "0.04" },
      { "day": "Tue", "cost": "0.07" }
    ]
  }
}
```
Display-only tabular data in a modal. `align` is one of `left` /
`right` / `center`. Selectable rows + inline mode are deferred follow-ups.

#### `ShowKeyValueView` (CC-7)
```json
{
  "event_type": "ShowKeyValueView",
  "payload": {
    "id": "session-1",
    "title": "Session details",
    "items": [
      { "label": "Started", "value": "2h ago" },
      { "label": "Model", "value": "your-model-v1" }
    ]
  }
}
```
Display-only label/value inspector pane.

#### `ShowForm` (CC-5) + `FormResponse` (outbound)
```json
{
  "event_type": "ShowForm",
  "payload": {
    "id": "settings",
    "title": "Settings",
    "fields": [
      { "name": "endpoint", "label": "API endpoint", "default": "https://api.example.com" },
      { "name": "token", "label": "API token", "kind": "password", "required": true }
    ]
  }
}
```
```json
{
  "event_type": "FormResponse",
  "payload": {
    "id": "settings",
    "values": { "endpoint": "https://api.example.com", "token": "hidden" }
  }
}
```
Multi-field form. Up/Down or Tab navigates fields. `kind` may be
`text` / `password` (boolean / select are deferred follow-ups).

#### `ShowWizard` (CC-4) + `WizardCompleted` / `WizardCancelled` (outbound)
```json
{
  "event_type": "ShowWizard",
  "payload": {
    "id": "onboard",
    "title": "Onboarding",
    "steps": [
      { "kind": "confirm", "id": "agree", "prompt": "Accept terms?" },
      { "kind": "form", "id": "creds", "fields": [ { "name": "email", "label": "Email" } ] }
    ]
  }
}
```
```json
{
  "event_type": "WizardCompleted",
  "payload": {
    "id": "onboard",
    "results": { "agree": true, "creds": { "email": "x@y.z" } }
  }
}
{ "event_type": "WizardCancelled", "payload": { "id": "onboard-2", "at_step": 1 } }
```
Multi-step flow composing Select / Confirm / Form steps. The renderer
intercepts the sub-modal responses and threads them into `results`
keyed by each step's `id`.

---

### Renderer → host (outbound)

#### `UserInputSubmitted`
```json
{ "event_type": "UserInputSubmitted", "payload": { "text": "investigate failing test" } }
```
Emitted on stdout when the user hits Enter in the input box. Hosts
should accept this on stdin (or fd 3 with `--responses-fd`) and route
it into their agent's input pipeline.

#### `ModeChangeRequested`
```json
{ "event_type": "ModeChangeRequested", "payload": { "direction": "next" } }
```
User hit Shift+Tab to cycle modes. `direction` is `next` or `prev`.
The host owns the mode list (e.g. `edit` / `plan` / `auto`) and is
expected to acknowledge by emitting an updated `StatusUpdate` whose
`segments.mode` reflects the new value.

#### `CancelRequested`
```json
{ "event_type": "CancelRequested", "payload": {} }
```
User hit Esc during an active stream / tool. Host should abort the
in-flight operation. The renderer also closes any open overlay locally
on Esc; this event fires only when nothing local was cancellable.

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

- **`examples/host-mock`** — exercises every event type as a
  synthetic event generator. Use this to verify a new consumer.
- **`examples/fake-agent`** — minimal load generator for the bench
  binary.
- **Real host adapter** — lives off-tree in the host adapter repo
  (see `PROGRESS.md`).

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
