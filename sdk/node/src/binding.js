/**
 * Camouflage runtime binding — spawns the Rust `camouflage-tui` renderer
 * as a child process and exposes a Node-native, pipe-free API.
 *
 * Consumers use this via `import { mount } from "camouflage"` and never
 * see NDJSON, stdio, or subprocesses. When/if we ship a NAPI build of
 * the renderer, the import surface stays identical — only this module's
 * internals swap.
 */

import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { EventEmitter } from "node:events";
import { existsSync, readFileSync } from "node:fs";
import { join, dirname, basename, parse as parsePath } from "node:path";
import { fileURLToPath } from "node:url";
import { encode, validate } from "./types.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const LOCAL_BIN = join(__dirname, "..", "bin", "camouflage-tui");
const DEFAULT_BIN = existsSync(LOCAL_BIN) ? LOCAL_BIN : "camouflage-tui";

/**
 * Best-effort detection of the host application's name, used as the header
 * brand so the renderer shows the *client's* identity — never "Camouflage".
 *
 * Walks up from the host's entry script (process.argv[1]) to the nearest
 * package.json and returns its `name`. Falls back to that package's folder
 * name, then to the current working directory's folder name. Returns null
 * if nothing can be determined (the renderer then shows the session id
 * alone). Hosts can always override via `mount({ appTitle })`.
 *
 * @returns {string|null}
 */
function detectAppTitle() {
  const entry = process.argv[1];
  let dir = entry ? dirname(entry) : process.cwd();
  const { root } = parsePath(dir);
  while (true) {
    const pkgPath = join(dir, "package.json");
    if (existsSync(pkgPath)) {
      try {
        const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
        if (pkg && typeof pkg.name === "string" && pkg.name.trim()) {
          return pkg.name.trim();
        }
      } catch {
        // unreadable/invalid package.json — keep walking up
      }
      return basename(dir) || null;
    }
    if (dir === root) break;
    dir = dirname(dir);
  }
  return basename(process.cwd()) || null;
}

class CamouflageHandle extends EventEmitter {
  constructor(child, stdin) {
    super();
    this._child = child;
    this._stdin = stdin;
    this._closed = false;
    this._closing = null; // Promise once close() begins
  }

  /**
   * Send one event INTO the renderer. Returns synchronously; under the
   * hood this writes one NDJSON line to the renderer's stdin. Throws
   * if the renderer has exited.
   *
   * NOTE: this method is named `send` (not `emit`) to avoid shadowing
   * EventEmitter's `emit()`, which the binding uses internally to
   * dispatch outbound events to consumer listeners.
   *
   * @param {string} event_type
   * @param {object} [payload]
   */
  send(event_type, payload = {}) {
    if (this._closed) {
      // Forgiving: when the renderer has already exited (user pressed
      // `q`, child crashed, etc.), `send()` no-ops silently and returns
      // false. Throwing here would kill the host process during normal
      // cleanup (the finally block typically tries to send a final
      // StatusUpdate + SessionEnded). Consumers that care can check
      // the boolean return or listen for the "exit" event.
      return false;
    }
    if (typeof event_type !== "string" || !event_type) {
      throw new TypeError("camouflage: event_type must be a non-empty string");
    }
    const line = encode({ event_type, payload });
    try {
      return this._writeLine(line);
    } catch (err) {
      // EPIPE / write-on-closed-stream race: same idea — swallow.
      if (err && (err.code === "EPIPE" || /writable/i.test(String(err.message)))) {
        return false;
      }
      throw err;
    }
  }

  /**
   * Send a pre-built Event object. Slight efficiency win for hot paths
   * because the caller can stringify once and reuse.
   *
   * @param {{event_type: string, payload?: object}} ev
   */
  sendEvent(ev) {
    if (this._closed) {
      return false;
    }
    return this._writeLine(encode(ev));
  }

  _writeLine(line) {
    if (!this._stdin.writable) {
      // Stream closed mid-flight (child exited between our last check
      // and now). Same forgiving policy as send(): no-op and return.
      this._closed = true;
      return false;
    }
    return this._stdin.write(line + "\n");
  }

  /**
   * Gracefully close the binding: closes stdin (the renderer will
   * finish processing buffered events, then exit), waits for the
   * child to exit, resolves with its exit code.
   */
  async close() {
    if (this._closing) return this._closing;
    this._closing = (async () => {
      this._closed = true;
      if (this._stdin.writable) {
        try { this._stdin.end(); } catch { /* ignore */ }
      }
      // Wait for the child to exit on its own (renderer reads stdin
      // until EOF). Soft timeout falls back to SIGTERM, then SIGKILL.
      const code = await waitForExit(this._child, 5000);
      return code;
    })();
    return this._closing;
  }

  /**
   * Force-kill the renderer immediately. Use only when close() doesn't
   * resolve (e.g. the renderer is wedged).
   */
  kill(signal = "SIGTERM") {
    this._closed = true;
    try { this._child.kill(signal); } catch { /* ignore */ }
  }
}

/**
 * Wait for a child process to exit. After `softTimeoutMs`, sends SIGTERM;
 * after another 2s, SIGKILL. Resolves with the exit code (or -1 if
 * killed without one).
 */
function waitForExit(child, softTimeoutMs) {
  return new Promise((resolve) => {
    if (child.exitCode != null) return resolve(child.exitCode);
    let resolved = false;
    const finalize = (code) => {
      if (resolved) return;
      resolved = true;
      resolve(code ?? -1);
    };
    child.once("exit", (code) => finalize(code));
    const term = setTimeout(() => {
      try { child.kill("SIGTERM"); } catch { /* ignore */ }
      const kill = setTimeout(() => {
        try { child.kill("SIGKILL"); } catch { /* ignore */ }
      }, 2000);
      // Defensive: if the kill timeout fires, still resolve.
      child.once("exit", () => clearTimeout(kill));
    }, softTimeoutMs);
    child.once("exit", () => clearTimeout(term));
  });
}

/**
 * Spawn the Camouflage renderer and return a CamouflageHandle.
 *
 * @param {object} [opts]
 * @param {string} [opts.bin]      Executable name or path. Defaults to
 *                                 "camouflage-tui" (PATH lookup).
 * @param {string} [opts.appTitle] Brand shown in the renderer header. When
 *                                 omitted, auto-detected from the host's
 *                                 package.json `name` (falling back to the
 *                                 folder name). The renderer never brands
 *                                 itself as "Camouflage".
 * @param {string[]} [opts.args]   Extra args to pass to the renderer.
 *                                 The binding always appends `--stdin-events
 *                                 --emit-responses` so outbound events flow
 *                                 back to us.
 * @param {object} [opts.env]      Environment overrides for the child.
 * @param {boolean} [opts.inheritStderr=true]
 *                                 If true, renderer stderr is forwarded to
 *                                 Node's stderr (useful for logs/diagnostics).
 *                                 If false, stderr is captured and exposed
 *                                 via the "stderr" event.
 * @returns {Promise<CamouflageHandle>} Resolves once the child has spawned.
 */
export async function mount(opts = {}) {
  const bin = opts.bin || DEFAULT_BIN;
  // Two integration modes:
  //
  // Default (programmatic, e.g. tests, daemons, anything not user-facing):
  //   stdio = [pipe, pipe, inherit|pipe]
  //   args  = --stdin-events --emit-responses
  //   stdout carries outbound NDJSON; we parse it line-by-line.
  //
  // renderToTerminal: true (the "host wraps the renderer" mode for
  // Option-B-style integrations where the host wraps the renderer):
  //   stdio = [pipe, inherit, inherit, pipe]
  //   args  = --stdin-events --responses-fd 3
  //   stdout goes directly to the user's terminal (rendering is visible);
  //   outbound NDJSON arrives on fd 3 (a separate pipe back to us).
  const renderToTerminal = !!opts.renderToTerminal;
  const stderrMode = opts.inheritStderr === false ? "pipe" : "inherit";

  let defaultArgs;
  let stdio;
  if (opts.skipDefaultArgs) {
    defaultArgs = [];
    stdio = ["pipe", "pipe", stderrMode];
  } else if (renderToTerminal) {
    defaultArgs = ["--stdin-events", "--responses-fd", "3"];
    stdio = ["pipe", "inherit", "inherit", "pipe"];
  } else {
    defaultArgs = ["--stdin-events", "--emit-responses"];
    stdio = ["pipe", "pipe", stderrMode];
  }
  // Brand the header with the host app's name (explicit > auto-detected).
  // Skipped when the caller manages args themselves (skipDefaultArgs) or
  // already passes --app-title in opts.args.
  const userArgs = opts.args || [];
  const titleArgs = [];
  if (!opts.skipDefaultArgs && !userArgs.includes("--app-title")) {
    const title = opts.appTitle != null ? String(opts.appTitle).trim() : detectAppTitle();
    if (title) {
      titleArgs.push("--app-title", title);
    }
  }
  const args = [...defaultArgs, ...titleArgs, ...userArgs];

  const child = spawn(bin, args, {
    stdio,
    env: opts.env ? { ...process.env, ...opts.env } : process.env,
  });

  // Surface spawn failures (binary missing, etc.) as a rejected mount().
  await new Promise((resolve, reject) => {
    const onError = (err) => {
      reject(spawnError(err, bin));
    };
    const onSpawn = () => {
      child.off("error", onError);
      resolve();
    };
    child.once("error", onError);
    child.once("spawn", onSpawn);
  });

  const handle = new CamouflageHandle(child, child.stdin);

  // Stream outbound events (UserInputSubmitted, PermissionResponse) from
  // whichever stream the renderer is writing them to. In renderToTerminal
  // mode that's fd 3 (child.stdio[3]); otherwise it's stdout.
  const outboundStream = renderToTerminal ? child.stdio[3] : child.stdout;
  if (!outboundStream) {
    throw new Error("camouflage: outbound stream is null — did the spawn succeed?");
  }
  const rl = createInterface({ input: outboundStream });
  rl.on("line", (line) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    const err = validate(trimmed);
    if (err) {
      // Renderer should only emit well-formed NDJSON. Surface but don't
      // throw — let the consumer decide what to do with malformed lines.
      handle.emit("invalid", { line: trimmed, error: err });
      return;
    }
    let ev;
    try { ev = JSON.parse(trimmed); }
    catch { return; /* unreachable: validate would have caught */ }
    // Translate well-known outbound events into ergonomic Node events.
    if (ev.event_type === "UserInputSubmitted") {
      handle.emit("userInput", ev.payload?.text ?? "");
    } else if (ev.event_type === "PermissionResponse") {
      handle.emit("permissionResponse", {
        request_id: ev.payload?.request_id,
        choice: ev.payload?.choice,
        feedback: ev.payload?.feedback,
      });
    } else if (ev.event_type === "SelectListResponse") {
      handle.emit("selectListResponse", {
        id: ev.payload?.id,
        value: ev.payload?.value,
        cancelled: !!ev.payload?.cancelled,
      });
    } else if (ev.event_type === "ConfirmResponse") {
      handle.emit("confirmResponse", {
        id: ev.payload?.id,
        value: ev.payload?.value,
        cancelled: !!ev.payload?.cancelled,
      });
    } else if (ev.event_type === "FormResponse") {
      handle.emit("formResponse", {
        id: ev.payload?.id,
        values: ev.payload?.values,
        cancelled: !!ev.payload?.cancelled,
      });
    } else if (ev.event_type === "WizardCompleted") {
      handle.emit("wizardCompleted", {
        id: ev.payload?.id,
        results: ev.payload?.results ?? {},
      });
    } else if (ev.event_type === "WizardCancelled") {
      handle.emit("wizardCancelled", {
        id: ev.payload?.id,
        at_step: ev.payload?.at_step,
      });
    } else if (ev.event_type === "ModeChangeRequested") {
      handle.emit("modeChangeRequested", {
        direction: ev.payload?.direction,
      });
    } else if (ev.event_type === "CancelRequested") {
      if (process.env.CAMOUFLAGE_DEBUG_ESC) {
        process.stderr.write(`[sdk:esc] CancelRequested line arrived, emitting cancelRequested\n`);
      }
      handle.emit("cancelRequested", {});
    }
    // Always also emit the raw Event for advanced consumers.
    handle.emit("event", ev);
  });

  if (stderrMode === "pipe" && child.stderr) {
    child.stderr.on("data", (buf) => handle.emit("stderr", buf.toString()));
  }

  // Surface unexpected exits as an "exit" event AND mark the handle closed.
  child.once("exit", (code, signal) => {
    handle._closed = true;
    handle.emit("exit", { code, signal });
  });

  return handle;
}

/**
 * Convenience helper: emit a ShowSelectList and resolve to the user's
 * SelectListResponse for that id. Subscribes once, unsubscribes after
 * the response arrives.
 *
 * @param {CamouflageHandle} cam
 * @param {{id: string, prompt: string, options: object[], default?: string, allow_filter?: boolean, allow_cancel?: boolean}} spec
 * @returns {Promise<{id: string, value?: string, cancelled: boolean}>}
 */
export function selectList(cam, spec) {
  return new Promise((resolve) => {
    const listener = (resp) => {
      if (resp.id !== spec.id) return;
      cam.off("selectListResponse", listener);
      resolve(resp);
    };
    cam.on("selectListResponse", listener);
    cam.send("ShowSelectList", spec);
  });
}

/**
 * Convenience helper: emit a ShowConfirm and resolve to the user's
 * ConfirmResponse for that id.
 *
 * @param {CamouflageHandle} cam
 * @param {{id: string, prompt: string, yes_label?: string, no_label?: string, default?: "yes"|"no", allow_cancel?: boolean}} spec
 * @returns {Promise<{id: string, value?: boolean, cancelled: boolean}>}
 */
export function confirm(cam, spec) {
  return new Promise((resolve) => {
    const listener = (resp) => {
      if (resp.id !== spec.id) return;
      cam.off("confirmResponse", listener);
      resolve(resp);
    };
    cam.on("confirmResponse", listener);
    cam.send("ShowConfirm", spec);
  });
}

/**
 * Convenience helper: show a brief inline toast. Display-only — no
 * response. Returns synchronously.
 *
 * @param {CamouflageHandle} cam
 * @param {string | {text: string, kind?: "info"|"success"|"warn"|"error", ttl_ms?: number}} spec
 *   Either a plain string (defaults to info, 3000ms) or a full spec object.
 */
export function toast(cam, spec) {
  const payload = typeof spec === "string" ? { text: spec } : spec;
  cam.send("ShowToast", payload);
}

/**
 * Convenience helper: show a tabular data view. Display-only.
 *
 * @param {CamouflageHandle} cam
 * @param {{id: string, title?: string, columns: object[], rows: object[]}} spec
 */
export function table(cam, spec) {
  cam.send("ShowTable", spec);
}

/**
 * Convenience helper: show a label/value list. Display-only.
 *
 * @param {CamouflageHandle} cam
 * @param {{id: string, title?: string, items: {label: string, value: string}[]}} spec
 */
export function keyValueView(cam, spec) {
  cam.send("ShowKeyValueView", spec);
}

/**
 * Convenience helper: show a multi-field form and resolve to the user's
 * FormResponse for that id.
 *
 * @param {CamouflageHandle} cam
 * @param {{id: string, title?: string, fields: object[], allow_cancel?: boolean}} spec
 * @returns {Promise<{id: string, values?: Record<string, string>, cancelled: boolean}>}
 */
export function form(cam, spec) {
  return new Promise((resolve) => {
    const listener = (resp) => {
      if (resp.id !== spec.id) return;
      cam.off("formResponse", listener);
      resolve(resp);
    };
    cam.on("formResponse", listener);
    cam.send("ShowForm", spec);
  });
}

/**
 * Convenience helper: emit a ShowWizard and resolve to the user's
 * completion or cancellation. Resolves with either:
 *   { id, results }           — all steps completed
 *   { id, cancelled: true, at_step }   — user cancelled
 *
 * @param {CamouflageHandle} cam
 * @param {{id: string, title?: string, steps: object[], allow_cancel?: boolean}} spec
 * @returns {Promise<{id: string, results?: Record<string, any>, cancelled?: boolean, at_step?: number}>}
 */
export function wizard(cam, spec) {
  return new Promise((resolve) => {
    const onCompleted = (resp) => {
      if (resp.id !== spec.id) return;
      cleanup();
      resolve(resp);
    };
    const onCancelled = (resp) => {
      if (resp.id !== spec.id) return;
      cleanup();
      resolve({ id: resp.id, cancelled: true, at_step: resp.at_step });
    };
    const cleanup = () => {
      cam.off("wizardCompleted", onCompleted);
      cam.off("wizardCancelled", onCancelled);
    };
    cam.on("wizardCompleted", onCompleted);
    cam.on("wizardCancelled", onCancelled);
    cam.send("ShowWizard", spec);
  });
}

function spawnError(err, bin) {
  if (err && err.code === "ENOENT") {
    return new Error(
      `camouflage: could not find renderer binary "${bin}". ` +
      `Try reinstalling: npm rebuild camouflage-tui\n` +
      `Or install from source: cargo install --git https://github.com/sinameraji/camouflage camouflage-tui\n` +
      `Or pass { bin: "/absolute/path/to/camouflage-tui" } to mount().`,
    );
  }
  return err;
}
