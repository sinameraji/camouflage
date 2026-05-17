/**
 * Camouflage — Node SDK entry point.
 *
 * This package wraps the Rust `camouflage-tui` renderer in a Node-native
 * API. Consumers should import from here. The wire-protocol types and the
 * NDJSON parsing helpers re-export from `./types.js` for convenience —
 * advanced users can also `import { ... } from "camouflage/types"` for a
 * dependency-light import that doesn't pull the subprocess management code.
 */

export {
  SCHEMA_VERSION,
  reader,
  validate,
  encode,
} from "./types.js";

export { mount, selectList, confirm, toast, table, keyValueView } from "./binding.js";
