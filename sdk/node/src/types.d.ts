/**
 * Camouflage event protocol — TypeScript types.
 *
 * Mirrors crates/protocol/src/lib.rs. See docs/protocol.md for the full
 * spec.
 *
 * All payload structs are typed; the inbound `Event` type is a tagged
 * union on `event_type` so a TS host's `switch` exhausts all cases.
 */

export const SCHEMA_VERSION = 1;

export type EventType =
  | "SessionStarted"
  | "SessionEnded"
  | "SessionCompacted"
  | "UserMessageCreated"
  | "AssistantStreamStarted"
  | "AssistantTokenDelta"
  | "AssistantMessageCompleted"
  | "ToolExecutionStarted"
  | "ToolExecutionStdout"
  | "ToolExecutionStderr"
  | "ToolExecutionFinished"
  | "PatchProposed"
  | "PatchApplied"
  | "PermissionRequested"
  | "PermissionGranted"
  | "PermissionDenied"
  | "RuntimeError"
  | "StatusUpdate"
  | "BackgroundTaskUpdate"
  | "ViewportMarker"
  | "UserInputSubmitted"
  | "PermissionResponse"
  | "SlashCommandsRegistered"
  | "MentionCandidatesRegistered"
  | "ShowSelectList"
  | "SelectListResponse"
  | "ShowConfirm"
  | "ConfirmResponse"
  | "ShowToast"
  | "ShowTable"
  | "ShowKeyValueView"
  | "ShowForm"
  | "FormResponse"
  | "ShowWizard"
  | "WizardCompleted"
  | "WizardCancelled"
  | "ModeChangeRequested"
  | "CancelRequested";

export type Direction = "inbound" | "outbound";

export interface EnvelopeMeta {
  id?: string;            // UUID
  session_id?: string;    // UUID
  seq?: number;
  timestamp_ms?: number;
  schema_version?: number;
}

// Payloads ------------------------------------------------------------------

export type UserMessage = { text: string };
export type AssistantStreamStarted = { stream_id: string };
export type AssistantTokenDelta = { stream_id: string; token: string };
export type AssistantMessageCompleted = { stream_id: string };

export type ToolStarted = { tool_id: string; tool: string; command: string };
export type ToolOutput = { tool_id: string; chunk: string };
export type ToolFinished = { tool_id: string; exit_code: number };

export type PatchProposed = {
  path: string;
  added?: number;
  removed?: number;
  /** Unified-diff string; renderer splits into per-line Diff rows. */
  diff?: string;
};
export type PatchApplied = { path: string };

export type PermissionRequested = {
  request_id: string;
  tool: string;
  action: string;
  detail?: string;
};
export type PermissionGranted = { request_id: string };
export type PermissionDenied = { request_id: string };

export type RuntimeErrorKind =
  | "generic"
  | "api_error"
  | "service_ended"
  | "quota_exhausted";
export type Severity = "info" | "warn" | "error" | "fatal";
export type Cta = { label: string; action_id: string };
export type RuntimeError = {
  message: string;
  source?: string;
  kind?: RuntimeErrorKind;
  severity?: Severity;
  cta?: Cta;
};

export type StatusUpdate = {
  segments: Record<string, string>;
};

export type BackgroundTaskState = "running" | "done" | "error";
export type BackgroundTaskUpdate = {
  task_id: string;
  label: string;
  state: BackgroundTaskState;
  progress?: number;
};

export type SessionCompacted = { old_seq?: number; new_seq?: number };
export type ViewportMarker = { label?: string };

export type UserInputSubmitted = { text: string };
export type PermissionChoice = "allow_once" | "allow_session" | "deny";
export type PermissionResponse = {
  request_id: string;
  choice: PermissionChoice;
  feedback?: string;
};

export type SlashCommand = {
  name: string;
  description?: string;
  args_hint?: string;
};
export type SlashCommandsRegistered = { commands: SlashCommand[] };

export type MentionCandidate = {
  token: string;
  label?: string;
  kind?: string;
};
export type MentionCandidatesRegistered = { candidates: MentionCandidate[] };

export type SelectListOption = {
  value: string;
  label: string;
  description?: string;
};
export type ShowSelectList = {
  id: string;
  prompt: string;
  options: SelectListOption[];
  default?: string;
  allow_filter?: boolean;
  allow_cancel?: boolean;
};
export type SelectListResponse = {
  id: string;
  value?: string;
  cancelled?: boolean;
};

export type ShowConfirm = {
  id: string;
  prompt: string;
  yes_label?: string;
  no_label?: string;
  default?: "yes" | "no";
  allow_cancel?: boolean;
};
export type ConfirmResponse = {
  id: string;
  value?: boolean;
  cancelled?: boolean;
};

export type ToastKind = "info" | "success" | "warn" | "error";
export type ShowToast = {
  text: string;
  kind?: ToastKind;
  ttl_ms?: number;
};

export type TableAlign = "left" | "right" | "center";
export type TableColumn = {
  name: string;
  label?: string;
  align?: TableAlign;
};
export type ShowTable = {
  id: string;
  title?: string;
  columns: TableColumn[];
  /** Each row is a JSON object keyed by column `name`. */
  rows: Record<string, unknown>[];
};

export type KeyValueItem = { label: string; value: string };
export type ShowKeyValueView = {
  id: string;
  title?: string;
  items: KeyValueItem[];
};

export type FormFieldKind = "text" | "password";
export type FormField = {
  name: string;
  label: string;
  kind?: FormFieldKind;
  default?: string;
  placeholder?: string;
  required?: boolean;
};
export type ShowForm = {
  id: string;
  title?: string;
  fields: FormField[];
  allow_cancel?: boolean;
};
export type FormResponse = {
  id: string;
  values?: Record<string, string>;
  cancelled?: boolean;
};

export type WizardStep =
  | { kind: "select"; id: string; prompt: string; options: SelectListOption[]; default?: string }
  | { kind: "confirm"; id: string; prompt: string; yes_label?: string; no_label?: string }
  | { kind: "form"; id: string; title?: string; fields: FormField[] };

export type ShowWizard = {
  id: string;
  title?: string;
  steps: WizardStep[];
  allow_cancel?: boolean;
};

export type WizardStepResult =
  | string                       // from a select step
  | boolean                      // from a confirm step
  | Record<string, string>;      // from a form step

export type WizardCompleted = {
  id: string;
  results: Record<string, WizardStepResult>;
};

export type WizardCancelled = {
  id: string;
  at_step: number;
};

export type ModeChangeRequested = {
  direction: "next" | "prev";
};

// Tagged union --------------------------------------------------------------

export type Event = EnvelopeMeta &
  (
    | { event_type: "SessionStarted"; payload?: Record<string, never> }
    | { event_type: "SessionEnded"; payload?: Record<string, never> }
    | { event_type: "SessionCompacted"; payload: SessionCompacted }
    | { event_type: "UserMessageCreated"; payload: UserMessage }
    | { event_type: "AssistantStreamStarted"; payload: AssistantStreamStarted }
    | { event_type: "AssistantTokenDelta"; payload: AssistantTokenDelta }
    | { event_type: "AssistantMessageCompleted"; payload: AssistantMessageCompleted }
    | { event_type: "ToolExecutionStarted"; payload: ToolStarted }
    | { event_type: "ToolExecutionStdout"; payload: ToolOutput }
    | { event_type: "ToolExecutionStderr"; payload: ToolOutput }
    | { event_type: "ToolExecutionFinished"; payload: ToolFinished }
    | { event_type: "PatchProposed"; payload: PatchProposed }
    | { event_type: "PatchApplied"; payload: PatchApplied }
    | { event_type: "PermissionRequested"; payload: PermissionRequested }
    | { event_type: "PermissionGranted"; payload: PermissionGranted }
    | { event_type: "PermissionDenied"; payload: PermissionDenied }
    | { event_type: "RuntimeError"; payload: RuntimeError }
    | { event_type: "StatusUpdate"; payload: StatusUpdate }
    | { event_type: "BackgroundTaskUpdate"; payload: BackgroundTaskUpdate }
    | { event_type: "ViewportMarker"; payload: ViewportMarker }
    | { event_type: "UserInputSubmitted"; payload: UserInputSubmitted }
    | { event_type: "PermissionResponse"; payload: PermissionResponse }
    | { event_type: "SlashCommandsRegistered"; payload: SlashCommandsRegistered }
    | { event_type: "MentionCandidatesRegistered"; payload: MentionCandidatesRegistered }
    | { event_type: "ShowSelectList"; payload: ShowSelectList }
    | { event_type: "SelectListResponse"; payload: SelectListResponse }
    | { event_type: "ShowConfirm"; payload: ShowConfirm }
    | { event_type: "ConfirmResponse"; payload: ConfirmResponse }
    | { event_type: "ShowToast"; payload: ShowToast }
    | { event_type: "ShowTable"; payload: ShowTable }
    | { event_type: "ShowKeyValueView"; payload: ShowKeyValueView }
    | { event_type: "ShowForm"; payload: ShowForm }
    | { event_type: "FormResponse"; payload: FormResponse }
    | { event_type: "ShowWizard"; payload: ShowWizard }
    | { event_type: "WizardCompleted"; payload: WizardCompleted }
    | { event_type: "WizardCancelled"; payload: WizardCancelled }
    | { event_type: "ModeChangeRequested"; payload: ModeChangeRequested }
    | { event_type: "CancelRequested"; payload?: Record<string, never> }
  );

// Reader --------------------------------------------------------------------

import type { Readable } from "node:stream";

/**
 * Read NDJSON from a Readable stream and yield one parsed Event per line.
 * Malformed lines are skipped (matching the lenient Rust renderer);
 * use `validate()` for a strict equivalent.
 *
 * @example
 *   import { reader } from "camouflage-sdk";
 *   for await (const ev of reader(process.stdin)) {
 *     if (ev.event_type === "AssistantTokenDelta") {
 *       process.stdout.write(ev.payload.token);
 *     }
 *   }
 */
export function reader(src: Readable): AsyncIterableIterator<Event>;

/**
 * Strict per-line validation: returns null if the line parses + the
 * event_type is known. Returns a string error message otherwise. Mirrors
 * the camouflage-validate binary's logic.
 */
export function validate(line: string): string | null;

/** Helper: stringify an Event into one NDJSON line, no trailing newline. */
export function encode(ev: Event): string;
