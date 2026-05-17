import { EventEmitter } from "node:events";
import type { Event } from "./types.js";

export interface MountOptions {
  /** Executable name or path. Defaults to "camouflage-tui" (PATH lookup). */
  bin?: string;
  /** Extra args to pass to the renderer. `--stdin-events --emit-responses`
   *  are always appended by the binding. */
  args?: string[];
  /** Environment overrides for the child. Merged with process.env. */
  env?: NodeJS.ProcessEnv;
  /** If true (default), renderer stderr is forwarded to Node's stderr.
   *  If false, captured and exposed via the "stderr" event. */
  inheritStderr?: boolean;
  /** When true, skip the default `--stdin-events --emit-responses` args.
   *  Only used to point the binding at a non-Camouflage binary for tests
   *  or specialised harnesses. */
  skipDefaultArgs?: boolean;
  /** When true, the renderer's stdout and stderr go directly to the user's
   *  terminal (TUI rendering is visible to the user), and outbound
   *  NDJSON arrives on fd 3 via the new --responses-fd path. Use this when
   *  building a "host CLI that internally drives Camouflage" — e.g.
   *  KimiFlare's eventual Ink replacement. Default false: backward-
   *  compatible programmatic mode where both directions ride on the
   *  pipes the binding manages. */
  renderToTerminal?: boolean;
}

export interface PermissionResponseEvent {
  request_id: string;
  choice: "allow_once" | "allow_session" | "deny";
  feedback?: string;
}

export interface SelectListResponseEvent {
  id: string;
  value?: string;
  cancelled: boolean;
}

export interface ConfirmResponseEvent {
  id: string;
  value?: boolean;
  cancelled: boolean;
}

/**
 * Convenience helper: emit a ShowConfirm and resolve to the user's
 * ConfirmResponse for that id.
 */
export function confirm(
  cam: CamouflageHandle,
  spec: {
    id: string;
    prompt: string;
    yes_label?: string;
    no_label?: string;
    default?: "yes" | "no";
    allow_cancel?: boolean;
  },
): Promise<ConfirmResponseEvent>;

/**
 * Convenience helper: show a brief inline toast. Display-only — no
 * response.
 */
export function toast(
  cam: CamouflageHandle,
  spec: string | {
    text: string;
    kind?: "info" | "success" | "warn" | "error";
    ttl_ms?: number;
  },
): void;

/** Convenience helper: show a tabular data view. Display-only. */
export function table(
  cam: CamouflageHandle,
  spec: {
    id: string;
    title?: string;
    columns: {
      name: string;
      label?: string;
      align?: "left" | "right" | "center";
    }[];
    rows: Record<string, unknown>[];
  },
): void;

/** Convenience helper: show a label/value list. Display-only. */
export function keyValueView(
  cam: CamouflageHandle,
  spec: {
    id: string;
    title?: string;
    items: { label: string; value: string }[];
  },
): void;

export interface FormResponseEvent {
  id: string;
  values?: Record<string, string>;
  cancelled: boolean;
}

/**
 * Convenience helper: show a multi-field form and resolve to the user's
 * FormResponse for that id.
 */
export function form(
  cam: CamouflageHandle,
  spec: {
    id: string;
    title?: string;
    fields: {
      name: string;
      label: string;
      kind?: "text" | "password";
      default?: string;
      placeholder?: string;
      required?: boolean;
    }[];
    allow_cancel?: boolean;
  },
): Promise<FormResponseEvent>;

export interface WizardCompletedEvent {
  id: string;
  results: Record<string, unknown>;
}

export interface WizardCancelledEvent {
  id: string;
  at_step: number;
}

export type WizardResolved =
  | (WizardCompletedEvent & { cancelled?: undefined })
  | (WizardCancelledEvent & { cancelled: true });

/**
 * Convenience helper: show a multi-step wizard. Resolves to either
 * `{ id, results }` (completion) or `{ id, cancelled: true, at_step }`
 * (user cancelled).
 */
export function wizard(
  cam: CamouflageHandle,
  spec: {
    id: string;
    title?: string;
    steps: (
      | { kind: "select"; id: string; prompt: string; options: { value: string; label: string; description?: string }[]; default?: string }
      | { kind: "confirm"; id: string; prompt: string; yes_label?: string; no_label?: string }
      | { kind: "form"; id: string; title?: string; fields: { name: string; label: string; kind?: "text" | "password"; default?: string; placeholder?: string; required?: boolean }[] }
    )[];
    allow_cancel?: boolean;
  },
): Promise<WizardResolved>;

/**
 * Convenience helper: emit a `ShowSelectList` and return a Promise that
 * resolves to the user's `SelectListResponseEvent` for that id. The host
 * picks the id; the helper subscribes once, filters by id, and unsubscribes
 * automatically.
 *
 * @example
 *   const choice = await selectList(cam, {
 *     id: "resume-picker",
 *     prompt: "Resume which session?",
 *     options: [{ value: "a", label: "Session A" }],
 *   });
 *   if (choice.cancelled) return;
 *   console.log("user picked:", choice.value);
 */
export function selectList(
  cam: CamouflageHandle,
  spec: {
    id: string;
    prompt: string;
    options: { value: string; label: string; description?: string }[];
    default?: string;
    allow_filter?: boolean;
    allow_cancel?: boolean;
  },
): Promise<SelectListResponseEvent>;

export interface InvalidEvent {
  line: string;
  error: string;
}

export interface ExitEvent {
  code: number | null;
  signal: NodeJS.Signals | null;
}

export interface CamouflageHandle extends EventEmitter {
  /** Send one event INTO the renderer. */
  send(event_type: string, payload?: object): boolean;
  /** Send a pre-built Event object. */
  sendEvent(ev: { event_type: string; payload?: object }): boolean;
  /** Gracefully close: end stdin, wait for child exit, resolve with code. */
  close(): Promise<number>;
  /** Force-kill the renderer. */
  kill(signal?: NodeJS.Signals): void;

  // Typed event subscriptions (EventEmitter overrides):
  on(event: "userInput", listener: (text: string) => void): this;
  on(event: "permissionResponse", listener: (resp: PermissionResponseEvent) => void): this;
  on(event: "selectListResponse", listener: (resp: SelectListResponseEvent) => void): this;
  on(event: "confirmResponse", listener: (resp: ConfirmResponseEvent) => void): this;
  on(event: "formResponse", listener: (resp: FormResponseEvent) => void): this;
  on(event: "wizardCompleted", listener: (resp: WizardCompletedEvent) => void): this;
  on(event: "wizardCancelled", listener: (resp: WizardCancelledEvent) => void): this;
  on(event: "modeChangeRequested", listener: (resp: { direction: "next" | "prev" }) => void): this;
  on(event: "event", listener: (ev: Event) => void): this;
  on(event: "invalid", listener: (info: InvalidEvent) => void): this;
  on(event: "stderr", listener: (chunk: string) => void): this;
  on(event: "exit", listener: (info: ExitEvent) => void): this;
  on(event: string, listener: (...args: any[]) => void): this;
}

/**
 * Spawn the Camouflage renderer and return a handle for emitting events
 * and listening to renderer→host outbound events.
 *
 * @example
 *   import { mount } from "camouflage";
 *   const cam = await mount();
 *   cam.send("SessionStarted", {});
 *   cam.on("userInput", (text) => console.log("got input:", text));
 *   // ... when done:
 *   await cam.close();
 */
export function mount(opts?: MountOptions): Promise<CamouflageHandle>;
