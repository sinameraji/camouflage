/**
 * Camouflage event protocol — JS runtime helpers.
 *
 * Pure ESM. No dependencies. Pairs with index.d.ts for types.
 */

export const SCHEMA_VERSION = 1;

const KNOWN_TYPES = new Set([
  "SessionStarted", "SessionEnded", "SessionCompacted",
  "UserMessageCreated",
  "AssistantStreamStarted", "AssistantTokenDelta", "AssistantMessageCompleted",
  "ToolExecutionStarted", "ToolExecutionStdout", "ToolExecutionStderr", "ToolExecutionFinished",
  "PatchProposed", "PatchApplied",
  "PermissionRequested", "PermissionGranted", "PermissionDenied",
  "RuntimeError", "StatusUpdate", "BackgroundTaskUpdate", "ViewportMarker",
  "UserInputSubmitted", "PermissionResponse",
  "SlashCommandsRegistered", "MentionCandidatesRegistered",
  "ShowSelectList", "SelectListResponse",
  "ShowConfirm", "ConfirmResponse",
  "ShowToast", "ShowTable", "ShowKeyValueView",
  "ShowForm", "FormResponse",
  "ShowWizard", "WizardCompleted", "WizardCancelled",
  "ModeChangeRequested", "CancelRequested",
]);

/**
 * Yield one parsed Event per non-blank line from a Readable stream.
 * Malformed lines are skipped (matching the lenient Rust renderer).
 *
 * @param {import("node:stream").Readable} src
 * @yields {object} Event
 */
export async function* reader(src) {
  src.setEncoding("utf8");
  let buf = "";
  for await (const chunk of src) {
    buf += chunk;
    let nl;
    while ((nl = buf.indexOf("\n")) !== -1) {
      const line = buf.slice(0, nl);
      buf = buf.slice(nl + 1);
      if (!line.trim()) continue;
      try {
        const ev = JSON.parse(line);
        if (ev && typeof ev.event_type === "string" && KNOWN_TYPES.has(ev.event_type)) {
          yield ev;
        }
      } catch {
        // skip; mirror the renderer's lenient behaviour
      }
    }
  }
  if (buf.trim()) {
    try {
      const ev = JSON.parse(buf);
      if (ev && typeof ev.event_type === "string" && KNOWN_TYPES.has(ev.event_type)) {
        yield ev;
      }
    } catch {}
  }
}

/**
 * Strict per-line validation. Returns null on success; an error message
 * on failure. Mirrors camouflage-validate's logic at the type-and-shape
 * level (does not enforce every payload field requirement; the Rust
 * validator remains the strict gate for CI).
 *
 * @param {string} line
 * @returns {string | null}
 */
export function validate(line) {
  let v;
  try { v = JSON.parse(line); }
  catch (e) { return `not valid JSON: ${e instanceof Error ? e.message : String(e)}`; }
  if (!v || typeof v !== "object") return "top-level value is not an object";
  if (typeof v.event_type !== "string") return "missing event_type";
  if (!KNOWN_TYPES.has(v.event_type)) return `unknown event_type: ${v.event_type}`;
  return null;
}

/**
 * Stringify an Event into one NDJSON line (no trailing newline).
 *
 * @param {object} ev
 * @returns {string}
 */
export function encode(ev) {
  return JSON.stringify(ev);
}
