# Camouflage UI Components Catalog

> 📜 **HISTORICAL — original component-catalog spec from 2026-05-17.**
> All seven components (CC-1 through CC-7: `SelectList`, `Confirm`,
> `Toast`, `Wizard`, `Form`, `Table`, `KeyValueView`) have shipped.
> Canonical reference for their on-the-wire shape is now
> [`docs/protocol.md`](../protocol.md) (see `Show*` event entries) and
> [`crates/protocol/src/lib.rs`](../../crates/protocol/src/lib.rs).
> This file is kept for the original design reasoning, which still
> reads useful as a "why we built it this way" record.

## Why this exists

Camouflage today ships a fixed set of UI widgets (status bar, task ribbon, permission widget, overlays for help / metrics / tool-output / slash picker / mention picker). Each one is hardcoded in the renderer and driven by a specific event type.

Hosts that need richer UI — a host's session picker, checkpoint browser, multi-step onboarding wizard, settings form — currently have no way to ask the renderer to show one. They have to fall back on their own UI library (React/Ink), which defeats the point of adopting Camouflage.

The catalog adds a layer of **declarative UI primitives**: hosts emit `Show<Component>` events with payload describing what to render and a unique `id`; the renderer paints the component using its own theme/layout; user interaction is reported back via `<Component>Response` events keyed by the same `id`.

This makes Camouflage a *terminal UI library* — shadcn/ui for TUIs — instead of just a transcript renderer.

## Design principles

1. **Declarative, not imperative.** Hosts say *what* to show, never *how* to draw. No `MoveCursor(x,y)` or `DrawText(s, style)` events.
2. **Renderer owns layout + theme.** A `SelectList` looks like SelectList-in-Dracula when the user's theme is Dracula. Hosts pass data, not styling.
3. **Cross-language.** Same wire shape (NDJSON) every other Camouflage event uses. Python/Go/etc. hosts get the catalog for free.
4. **id-keyed instances.** Every Show event carries a host-chosen `id`. Multiple components can be active simultaneously (a `Confirm` inside a `Wizard` step). Responses route back via id.
5. **Modal by default; inline opt-in.** Most components are modal overlays that take focus. `Toast`, `Table`, `KeyValueView` can be inline; mark with `inline: true`.
6. **Each component is one slice.** Protocol event(s) + model state + renderer overlay + Snapshot field + Node SDK types + a host-side wiring against a real component being replaced. The host wiring is the validation.

## The catalog

### CC-1 — `SelectList`

The highest-leverage primitive. One shape covers ~6 host React components (session picker, checkpoint picker, theme picker, resume picker, model picker, slash picker). The existing hardcoded slash picker is structurally a SelectList — we generalize it.

**Show event:**

```json
{
  "event_type": "ShowSelectList",
  "payload": {
    "id": "resume-picker-1747",
    "prompt": "Resume which session?",
    "options": [
      { "value": "sess-abc", "label": "fix auth — 12 turns, 2h ago" },
      { "value": "sess-def", "label": "refactor router — 8 turns, yesterday" }
    ],
    "default": "sess-abc",
    "allow_filter": true,
    "allow_cancel": true
  }
}
```

**Response (outbound):**

```json
{ "event_type": "SelectListResponse", "payload": { "id": "resume-picker-1747", "value": "sess-abc" } }
```

Or, when the user cancels (Esc or `Ctrl+C`):

```json
{ "event_type": "SelectListResponse", "payload": { "id": "resume-picker-1747", "cancelled": true } }
```

**Rendering:** centered modal overlay (~60% width). Up/Down to navigate, Enter to select, Esc to cancel. When `allow_filter: true`, typing characters narrows the list (substring match on `label`).

### CC-2 — `Confirm`

Tiny scope; covers many small modals. The existing `PermissionRequested` is structurally a special case of `Confirm` with stable buttons; we keep it as a named type for ergonomics.

**Show:**

```json
{
  "event_type": "ShowConfirm",
  "payload": {
    "id": "save-before-quit",
    "prompt": "Save before quitting?",
    "yes_label": "Save",
    "no_label": "Discard",
    "default": "yes"
  }
}
```

**Response:** `{ "event_type": "ConfirmResponse", "payload": { "id": "...", "value": true } }`. Cancel → `{ "id": "...", "cancelled": true }`.

### CC-3 — `Toast`

Trivial scope; display-only. No outbound event.

```json
{
  "event_type": "ShowToast",
  "payload": {
    "text": "Authenticated.",
    "kind": "success",
    "ttl_ms": 2000
  }
}
```

`kind`: `"info" | "success" | "warn" | "error"`. Auto-fades after `ttl_ms` (default 3000).

### CC-4 — `Wizard`

Multi-step flow; composes Select/Confirm/Form.

```json
{
  "event_type": "ShowWizard",
  "payload": {
    "id": "onboarding",
    "steps": [
      { "kind": "select", "id": "auth-method", "prompt": "Auth method", "options": [ ... ] },
      { "kind": "form",   "id": "credentials", "fields": [ ... ] },
      { "kind": "confirm","id": "review",      "prompt": "Looks right?" }
    ]
  }
}
```

Renderer drives the user through steps sequentially. Per-step values accumulate into `results`. Complete: `WizardCompleted { id, results }`. Cancel: `WizardCancelled { id, at_step }`.

### CC-5 — `Form`

Multi-field input.

```json
{
  "event_type": "ShowForm",
  "payload": {
    "id": "settings",
    "title": "Cloud configuration",
    "fields": [
      { "name": "token", "label": "API token", "type": "password", "required": true },
      { "name": "endpoint", "label": "Endpoint", "type": "text", "default": "https://..." },
      { "name": "use_cache", "label": "Enable cache", "type": "boolean", "default": true }
    ]
  }
}
```

Field types: `text | password | boolean | int | float | select`. Response: `FormSubmitted { id, values: {...} }` or `FormCancelled { id }`.

### CC-6 — `Table`

Display-oriented; optional row selection.

```json
{
  "event_type": "ShowTable",
  "payload": {
    "id": "usage-week",
    "title": "Cost — last 7 days",
    "columns": [
      { "name": "day", "label": "Day" },
      { "name": "tokens", "label": "Tokens" },
      { "name": "cost", "label": "Cost ($)", "align": "right" }
    ],
    "rows": [
      { "day": "Mon", "tokens": 12000, "cost": 0.04 },
      { "day": "Tue", "tokens": 18000, "cost": 0.07 }
    ],
    "selectable": true,
    "inline": false
  }
}
```

When `selectable: true`, Enter on a focused row fires `RowSelected { id, row }`. Otherwise display-only.

### CC-7 — `KeyValueView`

Static label/value list — "session details", welcome screen, "about" panel.

```json
{
  "event_type": "ShowKeyValueView",
  "payload": {
    "id": "session-detail",
    "title": "Session sess-abc",
    "items": [
      { "label": "Started",   "value": "2 hours ago" },
      { "label": "Model",     "value": "your-model-v1" },
      { "label": "Turns",     "value": "12" },
      { "label": "Tokens in", "value": "48,392" }
    ],
    "inline": false
  }
}
```

Display-only.

## Cross-component conventions

- **Cancellation:** every modal component honours `Esc` and `Ctrl+C`. Response carries `cancelled: true` when applicable. Hosts that disallow cancel set `allow_cancel: false` in the Show payload.
- **Stacking:** components stack as a focus stack. Newer modals overlay older ones; closing returns focus to the prior one.
- **Style:** all components draw using the active theme. Borders / accents / selection highlights come from `Theme.{overlay_border, accent, ...}`.
- **Inline mode:** when `inline: true` is honoured (Toast, Table, KeyValueView), the component renders into the transcript flow as a special row group rather than as a modal overlay.
- **Per-id state:** the renderer's model maintains an `active_components: Map<id, ComponentState>` so multiple instances coexist. State is cleared when the response is emitted (or on `Esc`).

## Shipping order

Each is one self-contained slice, validated against a real host driver before the next one starts:

1. **CC-1 `SelectList`** — driver: replace the host's resume picker. After this, the existing hardcoded slash-picker becomes a special case of `SelectList` and can be deprecated.
2. **CC-2 `Confirm`** — driver: `--ui camouflage` quit confirmation.
3. **CC-3 `Toast`** — driver: the host's "Authenticated" / "Saved" feedback.
4. **CC-4 `Wizard`** — driver: host onboarding flow.
5. **CC-5 `Form`** — driver: settings form (or whatever the host needs first).
6. **CC-6 `Table`** — driver: a host `cost` weekly view.
7. **CC-7 `KeyValueView`** — driver: host welcome screen.

PROGRESS.md tracks per-slice status under "Components catalog (the missing UI primitives layer)".
