/**
 * Camouflage — Node SDK entry point.
 *
 * Wraps the Rust `camouflage-tui` renderer in a Node-native API. Wire-
 * protocol types and parse helpers live in `./types`; the runtime binding
 * (mount + subprocess management) lands alongside.
 */

export {
  SCHEMA_VERSION,
  EventType,
  Direction,
  EnvelopeMeta,
  // Payloads
  UserMessage,
  AssistantStreamStarted,
  AssistantTokenDelta,
  AssistantMessageCompleted,
  ToolStarted,
  ToolOutput,
  ToolFinished,
  PatchProposed,
  PatchApplied,
  PermissionRequested,
  PermissionGranted,
  PermissionDenied,
  RuntimeErrorKind,
  Severity,
  Cta,
  RuntimeError,
  StatusUpdate,
  BackgroundTaskState,
  BackgroundTaskUpdate,
  SessionCompacted,
  ViewportMarker,
  UserInputSubmitted,
  PermissionChoice,
  PermissionResponse,
  SlashCommand,
  SlashCommandsRegistered,
  MentionCandidate,
  MentionCandidatesRegistered,
  // Tagged union
  Event,
  // Helpers
  reader,
  validate,
  encode,
} from "./types.js";
