/**
 * Camouflage lifecycle API — turn / stream / tool handles that auto-settle.
 *
 * See lifecycle.js for the runtime behaviour and the guarantees these
 * types describe: closing a handle (or the turn) settles everything under
 * it, and a renderer exit settles everything still open.
 */

import type { DiffPayload } from "./types.js";
import type { PermissionResponseEvent } from "./binding.js";

/** Host-facing tool statuses; `"ok"` and `"done"` both mean success. */
export type ToolStatus = "ok" | "done" | "error" | "failed" | "cancelled" | "rejected";

export interface TurnOptions {
  /** Stable id for this turn. Host bookkeeping only — never sent on the wire. */
  id?: string;
  /** Status given to tools still open when `turn.end()` runs. Default `"done"`. */
  settleToolStatus?: ToolStatus;
}

export interface StreamOptions {
  /** Stream id; generated when omitted. */
  id?: string;
}

export interface StreamEndOptions {
  /** Replaces whatever was streamed (e.g. after cleanup). */
  text?: string;
}

/** One assistant message being streamed. */
export class StreamHandle {
  /** The stream id used in every event for this stream. */
  readonly id: string;
  /** True once the stream has been ended (or auto-settled). */
  readonly settled: boolean;
  /** Append text (AssistantTokenDelta). False once ended — never throws. */
  write(token: unknown): boolean;
  /** Append reasoning (AssistantReasoningDelta), shown on Ctrl+R. */
  reasoning(token: unknown): boolean;
  /** Finish the stream (AssistantMessageCompleted). Idempotent. */
  end(opts?: StreamEndOptions): boolean;
}

export interface ToolSpec {
  /** Stable tool id; generated when omitted. */
  id?: string;
  /** Alias for `id`. */
  tool_id?: string;
  /** Tool name shown in bold, e.g. `"Bash"`. */
  name?: string;
  /** Alias for `name`. */
  tool?: string;
  /** Arguments shown after the name, e.g. a path or a command line. */
  args?: string;
  /** Alias for `args`. */
  command?: string;
  /** When the tool started (epoch ms). Defaults to when the handle opens. */
  started_at_ms?: number;
}

export interface ToolFinishSpec {
  status?: ToolStatus;
  /** Defaults to 0 (1 when the status is an error). */
  exit_code?: number;
  /** One-line result under the tool row, e.g. "142 lines". */
  summary?: string;
  /** Full output; replaces anything streamed via `stdout()`/`stderr()`. */
  output?: string;
  /** Output lines shown before "ctrl+o to expand". */
  preview?: number;
  /** Highlight the output as this language. */
  output_lang?: string;
  /** Diff shown under the tool row. */
  diff?: DiffPayload;
}

/** One tool execution. */
export class ToolHandle {
  /** The tool id used in every event for this tool. */
  readonly id: string;
  /** True once the tool has been finished (or auto-settled). */
  readonly settled: boolean;
  /** Stream output (ToolExecutionStdout). */
  stdout(chunk: unknown): boolean;
  /** Stream stderr (ToolExecutionStderr). */
  stderr(chunk: unknown): boolean;
  /** Alias for `stdout()` — the common case. */
  write(chunk: unknown): boolean;
  /** Finish the tool (ToolExecutionFinished). Idempotent. */
  finish(spec?: ToolFinishSpec): boolean;
}

export interface PermissionSpec {
  /** Prompt title, e.g. `"edit src/auth/session.ts"`. */
  title?: string;
  /** Tool being authorized, e.g. `"bash"`. */
  tool?: string;
  /** Alias for `title`. */
  action?: string;
  /** Alias for `tool`. */
  name?: string;
  /** Dim line under the title. */
  detail?: string;
  /** Diff preview for edits. */
  diff?: DiffPayload;
  /** Generated when omitted. */
  request_id?: string;
}

export interface TurnEndOptions {
  /** Status given to tools still open at `end()`. Default: the turn's `settleToolStatus`. */
  toolStatus?: ToolStatus;
  /**
   * Value for the `phase` status segment. The renderer settles its spinner
   * when phase is not busy, so the default `"idle"` clears it. Pass `null`
   * to leave the phase alone (e.g. when another turn owns it).
   */
  phase?: string | null;
}

/** One unit of work: streams, tools and permission requests that settle together. */
export class Turn {
  /** Host-facing id; never sent on the wire (the protocol has no turn event). */
  readonly id: string;
  /** True once `end()` has run (directly or via auto-settle). */
  readonly settled: boolean;
  /** Open an assistant stream (AssistantStreamStarted). */
  stream(opts?: StreamOptions): StreamHandle;
  /** Start a tool (ToolExecutionStarted). */
  tool(spec?: ToolSpec): ToolHandle;
  /**
   * Ask the user for permission (PermissionRequested). Resolves `{ choice: "deny" }`
   * if the renderer exits first or the turn ends before anyone answers.
   */
  permission(spec?: PermissionSpec): Promise<PermissionResponseEvent>;
  /**
   * Settle every stream, tool and permission still open under this turn,
   * then clear the busy phase. Idempotent.
   */
  end(opts?: TurnEndOptions): boolean;
}

export interface Lifecycle {
  /** Open a new turn. */
  turn(opts?: TurnOptions): Turn;
  /** Settle every turn still open (used on renderer exit). Idempotent. */
  settleAll(): void;
  /** How many turns are still open. */
  openTurns(): number;
}

/** Build the lifecycle facade bound to one handle. Used by `mount()`. */
export function createLifecycle(
  cam: { send(event_type: string, payload?: object): boolean },
  helpers?: { permission?: Function },
): Lifecycle;

/** Normalize a host-facing status to the wire value; throws on unknown values. */
export function normalizeToolStatus(status?: string): "done" | "error" | "cancelled" | "rejected";
