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
  | "TodoListUpdate"
  | "ViewportMarker"
  | "UserInputSubmitted"
  | "PermissionResponse"
  | "SlashCommandsRegistered"
  | "MentionCandidatesRegistered"
  | "ShowSelectList"
  | "SelectListResponse"
  | "ShowConfirm"
  | "ConfirmResponse"
  | "ShowTable"
  | "ShowKeyValueView"
  | "ShowForm"
  | "FormResponse"
  | "ShowWizard"
  | "WizardCompleted"
  | "WizardCancelled"
  | "ModeChangeRequested"
  | "CancelRequested"
  | "TranscriptCleared"
  | "Splash"
  | "ShowToast"
  | "MentionQuery"
  | "AssistantReasoningDelta"
  | "TerminalSuspend"
  | "TerminalResume"
  | "TerminalSuspended"
  | "ActivityUpdate"
  | "ActivityLog"
  | "ActivityRemoved"
  | "ActivitySnapshot"
  | "ActivityBrowserOpen"
  | "ActivityStopRequested"
  | "ActivityViewChanged";

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
export type AssistantMessageCompleted = {
  stream_id: string;
  /** Optional final text; replaces what was streamed (e.g. after cleanup). */
  text?: string;
};

export type ToolStarted = {
  tool_id: string;
  /** Tool name shown in bold, e.g. "Read" or "Bash". */
  tool: string;
  /** Arguments shown after the name, e.g. a path or a command line. */
  command: string;
  /** When the tool started (epoch ms). Defaults to when the event arrives. */
  started_at_ms?: number;
};
export type ToolOutput = { tool_id: string; chunk: string };
export type ToolFinished = {
  tool_id: string;
  exit_code: number;
  /** Overrides the status derived from exit_code. */
  status?: "done" | "error" | "cancelled" | "rejected";
  /** One-line result under the tool row, e.g. "142 lines" or "14 passed". */
  summary?: string;
  /** Full output; replaces anything streamed via ToolExecutionStdout. */
  output?: string;
  /** Output lines shown before "ctrl+o to expand". Default 0 (12 on error). */
  preview?: number;
  /** Highlight the output as this language (e.g. "ts" for a written file). */
  output_lang?: string;
  /** Show this diff under the tool row. */
  diff?: DiffPayload;
};

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
  /** Prompt title, e.g. "edit src/auth/session.ts". */
  action: string;
  detail?: string;
  /** Diff preview for edits. */
  diff?: DiffPayload;
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

/**
 * Status segments. Well-known keys: `mode` ("edit" | "plan" | "auto"),
 * `phase` ("thinking" | "streaming" | "tool" | "running" shows the spinner;
 * anything else, e.g. "idle", ends the turn and settles spinners),
 * `activity` (spinner verb, e.g. "Reading files"), `model`, `tokens`,
 * `cost`, `branch`, `warn`. Other keys are shown in the footer. An empty
 * value removes a segment.
 */
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

export type TodoStatus = "pending" | "in_progress" | "completed";
export type TodoItem = {
  id: string;
  title: string;
  status: TodoStatus;
  /** Epoch ms when the task began. Used for elapsed-time ticker. */
  started_at_ms?: number;
  /** Tokens consumed on completion. Shown as `· 1.2k tok`. */
  token_delta?: number;
  /** 0.0..=1.0 for in-progress tasks. Shown as progress bar/percentage. */
  progress?: number;
};
export type TodoListUpdate = { todos: TodoItem[] };

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
export type MentionCandidatesRegistered = {
  candidates: MentionCandidate[];
  /** Set when answering a MentionQuery: the directory part these entries are
   *  for (e.g. "../"). Without it the list replaces the default candidates. */
  for_query?: string;
};

export type SelectListOption = {
  value: string;
  label: string;
  /** Dim text after the label; also searched. */
  description?: string;
  /** Group header shown above the first option of each group (hidden while searching). */
  section?: string;
  /** Values drawn in aligned columns after the label, e.g. ["256k", "$0.60 / $2.50"]. */
  columns?: string[];
  /** Draws an on/off badge, e.g. for enabled skills. */
  state?: "on" | "off";
  /** Extra text the search matches against. */
  keywords?: string;
};
export type ShowSelectList = {
  id: string;
  prompt: string;
  /** A dim line under the prompt. */
  subtitle?: string;
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

export type FormFieldKind = "text" | "password" | "multiline";
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

/** Optional welcome and branding for the inline renderer. */
export type SessionStarted = {
  /** First welcome line, shown in the accent color. Defaults to the app title. */
  title?: string;
  /** Dim lines under the title (model, directory, hints). */
  detail?: string[];
  /** Accent color: a terminal color name ("orange", "blue", …) or "#rrggbb". */
  accent?: string;
  /** Your agent's name, used in prompts like "tell <name> what to do". */
  assistant_label?: string;
  user_label?: string;
};

/** Multi-line text (may contain ANSI colors) printed as-is, e.g. a logo. */
export type Splash = { text: string };

/** A short message shown in the footer (inline) or as a toast (full screen). */
export type ShowToast = {
  text: string;
  kind?: "info" | "warn" | "error" | "success";
  ttl_ms?: number;
};

/**
 * Renderer → host: the user is typing a path after `@` (`./`, `../`, `~`,
 * `/`). Answer with MentionCandidatesRegistered `{ for_query, candidates }`
 * listing that directory; folder tokens end in "/".
 */
export type MentionQuery = { query: string };

/** v2.4+: `TerminalSuspend` / `TerminalResume` payload. */
export type TerminalHandoff = { id: string };
/** v2.4+: renderer → host. `supported: false` means run the child without a terminal. */
export type TerminalSuspended = { id: string; supported: boolean };

/** v2.4+: one step of an activity item (agents' plans). */
export type ActivityStep = { title: string; status?: "pending" | "running" | "done" | "failed" };
/**
 * v2.4+: a background job or agent the host owns. Send it with
 * `ActivityUpdate` (fields left out keep their previous values) or inside
 * `ActivitySnapshot`. The renderer only displays it.
 */
export type ActivityItem = {
  /** Stable id; later updates, logs and removals refer to it. */
  id: string;
  kind?: "job" | "agent";
  /** The command, task or agent name. */
  title?: string;
  status?: "running" | "waiting" | "needs_attention" | "done" | "failed" | "stopped";
  /** Whether the user may ask to stop it (they confirm first). */
  stoppable?: boolean;
  /** One short line: current step, exit code, what it's waiting for. */
  summary?: string;
  /** 0..1 when known. */
  progress?: number;
  started_at_ms?: number;
  /** Last change; for finished items, when they finished. */
  updated_at_ms?: number;
  steps?: ActivityStep[];
};
/** v2.4+: output for an item; may span lines or end mid-line. ANSI is stripped. */
export type ActivityLog = { id: string; chunk: string; stream?: "stdout" | "stderr" };
export type ActivityRemoved = { id: string };
/** v2.4+: the complete list; items not in it are dropped. */
export type ActivitySnapshot = { items: ActivityItem[] };
/** v2.4+: renderer → host, after the user confirmed. Report the outcome with ActivityUpdate. */
export type ActivityStopRequested = { id: string };
/** v2.4+: renderer → host. Stream an item's output while its details are open, for example. */
export type ActivityViewChanged = { view: "list" | "detail" | "closed"; id?: string };

/** A diff to show inline: full before/after text, or a unified diff. */
export type DiffPayload = {
  path: string;
  before?: string;
  after?: string;
  unified?: string;
};

// Tagged union --------------------------------------------------------------

export type Event = EnvelopeMeta &
  (
    | { event_type: "SessionStarted"; payload?: SessionStarted }
    | { event_type: "SessionEnded"; payload?: Record<string, never> }
    | { event_type: "SessionCompacted"; payload: SessionCompacted }
    | { event_type: "UserMessageCreated"; payload: UserMessage }
    | { event_type: "AssistantStreamStarted"; payload: AssistantStreamStarted }
    | { event_type: "AssistantTokenDelta"; payload: AssistantTokenDelta }
    /** v2.4+: the model's reasoning, shown when the user presses Ctrl+R (inline UI). */
    | { event_type: "AssistantReasoningDelta"; payload: AssistantTokenDelta }
    /** v2.4+: hand the terminal to a child process; see `suspendTerminal()`. */
    | { event_type: "TerminalSuspend"; payload: TerminalHandoff }
    | { event_type: "TerminalResume"; payload: TerminalHandoff }
    | { event_type: "TerminalSuspended"; payload: TerminalSuspended }
    /** v2.4+: background jobs and agents (see ActivityItem). */
    | { event_type: "ActivityUpdate"; payload: ActivityItem }
    | { event_type: "ActivityLog"; payload: ActivityLog }
    | { event_type: "ActivityRemoved"; payload: ActivityRemoved }
    | { event_type: "ActivitySnapshot"; payload: ActivitySnapshot }
    | { event_type: "ActivityBrowserOpen"; payload?: Record<string, never> }
    | { event_type: "ActivityStopRequested"; payload: ActivityStopRequested }
    | { event_type: "ActivityViewChanged"; payload: ActivityViewChanged }
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
    | { event_type: "TodoListUpdate"; payload: TodoListUpdate }
    | { event_type: "ViewportMarker"; payload: ViewportMarker }
    | { event_type: "UserInputSubmitted"; payload: UserInputSubmitted }
    | { event_type: "PermissionResponse"; payload: PermissionResponse }
    | { event_type: "SlashCommandsRegistered"; payload: SlashCommandsRegistered }
    | { event_type: "MentionCandidatesRegistered"; payload: MentionCandidatesRegistered }
    | { event_type: "ShowSelectList"; payload: ShowSelectList }
    | { event_type: "SelectListResponse"; payload: SelectListResponse }
    | { event_type: "ShowConfirm"; payload: ShowConfirm }
    | { event_type: "ConfirmResponse"; payload: ConfirmResponse }
    | { event_type: "ShowTable"; payload: ShowTable }
    | { event_type: "ShowKeyValueView"; payload: ShowKeyValueView }
    | { event_type: "ShowForm"; payload: ShowForm }
    | { event_type: "FormResponse"; payload: FormResponse }
    | { event_type: "ShowWizard"; payload: ShowWizard }
    | { event_type: "WizardCompleted"; payload: WizardCompleted }
    | { event_type: "WizardCancelled"; payload: WizardCancelled }
    | { event_type: "ModeChangeRequested"; payload: ModeChangeRequested }
    | { event_type: "CancelRequested"; payload?: Record<string, never> }
    | { event_type: "TranscriptCleared"; payload?: Record<string, never> }
    | { event_type: "Splash"; payload: Splash }
    | { event_type: "ShowToast"; payload: ShowToast }
    | { event_type: "MentionQuery"; payload: MentionQuery }
  );

/** The payload type for a given event type, e.g. `PayloadOf<"ShowToast">`. */
export type PayloadOf<T extends EventType> = Extract<Event, { event_type: T }> extends { payload?: infer P } ? P : never;

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
