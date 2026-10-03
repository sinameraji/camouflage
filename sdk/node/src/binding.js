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
import { existsSync, readFileSync, appendFileSync } from "node:fs";
import { join, dirname, basename, parse as parsePath } from "node:path";
import { fileURLToPath } from "node:url";
import { encode, validate } from "./types.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const LOCAL_BIN = join(__dirname, "..", "bin", process.platform === "win32" ? "camouflage-tui.exe" : "camouflage-tui");
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
    if (this._record) this._record("in", line);
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
    defaultArgs = ["--stdin-events", "--emit-responses=true"];
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
  const uiArgs = opts.ui && !opts.skipDefaultArgs && !userArgs.includes("--ui") ? ["--ui", opts.ui] : [];
  const args = [...defaultArgs, ...uiArgs, ...titleArgs, ...userArgs];

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
  // CAMOUFLAGE_RECORD=<file>: append every event to and from the renderer
  // as `{"t": ms since mount, "in" | "out": event}` lines, for replay tests
  // and bug reports. Off unless set.
  const recordPath = opts.record ?? process.env.CAMOUFLAGE_RECORD;
  if (recordPath) {
    const t0 = Date.now();
    handle._record = (dir, line) => {
      try {
        appendFileSync(recordPath, `{"t":${Date.now() - t0},"${dir}":${line}}\n`);
      } catch {
        // Recording is best-effort; never break the host over it.
      }
    };
  }

  // If the renderer exits, writes to its stdin fail with EPIPE. Unhandled,
  // that 'error' event would crash the host process; treat it as closed.
  child.stdin.on("error", (err) => {
    handle._closed = true;
    if (err && err.code !== "EPIPE") handle.emit("invalid", { line: "", error: String(err) });
  });

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
    if (handle._record) handle._record("out", trimmed);
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
    } else if (ev.event_type === "MentionQuery") {
      handle.emit("mentionQuery", { query: ev.payload?.query ?? "" });
    } else if (ev.event_type === "ActivityStopRequested") {
      handle.emit("activityStopRequested", { id: ev.payload?.id });
    } else if (ev.event_type === "ActivityViewChanged") {
      handle.emit("activityViewChanged", { view: ev.payload?.view, id: ev.payload?.id });
    } else if (ev.event_type === "TerminalSuspended") {
      handle.emit("terminalSuspended", { id: ev.payload?.id, supported: ev.payload?.supported === true });
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
 * Resolve with the first `eventName` response whose id matches, or with
 * `onExit` if the renderer exits first, so callers never hang on a dead
 * renderer.
 */
function awaitResponse(cam, eventName, matches, onExit) {
  return new Promise((resolve) => {
    if (cam._closed) {
      resolve(onExit);
      return;
    }
    const cleanup = () => {
      cam.off(eventName, listener);
      cam.off("exit", exitListener);
    };
    const listener = (resp) => {
      if (!matches(resp)) return;
      cleanup();
      resolve(resp);
    };
    const exitListener = () => {
      cleanup();
      resolve(onExit);
    };
    cam.on(eventName, listener);
    cam.on("exit", exitListener);
  });
}

/**
 * Convenience helper: emit a ShowSelectList and resolve to the user's
 * SelectListResponse for that id. Resolves `{ cancelled: true }` if the
 * renderer exits first.
 *
 * @param {CamouflageHandle} cam
 * @param {{id: string, prompt: string, options: object[], default?: string, allow_filter?: boolean, allow_cancel?: boolean}} spec
 * @returns {Promise<{id: string, value?: string, cancelled: boolean}>}
 */
export function selectList(cam, spec) {
  const done = awaitResponse(cam, "selectListResponse", (r) => r.id === spec.id, { id: spec.id, cancelled: true });
  cam.send("ShowSelectList", spec);
  return done;
}

/**
 * Convenience helper: emit a ShowConfirm and resolve to the user's
 * ConfirmResponse for that id. Resolves `{ cancelled: true }` if the
 * renderer exits first.
 *
 * @param {CamouflageHandle} cam
 * @param {{id: string, prompt: string, yes_label?: string, no_label?: string, default?: "yes"|"no", allow_cancel?: boolean}} spec
 * @returns {Promise<{id: string, value?: boolean, cancelled: boolean}>}
 */
export function confirm(cam, spec) {
  const done = awaitResponse(cam, "confirmResponse", (r) => r.id === spec.id, { id: spec.id, cancelled: true });
  cam.send("ShowConfirm", spec);
  return done;
}

/**
 * Hand the terminal to a child process (a `!` shell command, an editor, a
 * login prompt). The inline renderer clears its live area, leaves raw mode
 * and stops reading keys before this resolves. Run the child with
 * `stdio: "inherit"`, then call `resumeTerminal(cam)`.
 *
 * Resolves `{ supported: false }` when the renderer can't hand the terminal
 * over (Windows, full-screen mode, renderers older than 2.4.0-beta.7, or no
 * answer within `timeoutMs`); run the child without a terminal then.
 *
 * @param {CamouflageHandle} cam
 * @param {{timeoutMs?: number}} [opts]
 * @returns {Promise<{supported: boolean}>}
 */
export function suspendTerminal(cam, opts = {}) {
  const id = `suspend-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const answer = awaitResponse(cam, "terminalSuspended", (r) => r.id === id, { id, supported: false });
  const timeout = new Promise((resolve) => {
    const t = setTimeout(() => resolve({ id, supported: false }), opts.timeoutMs ?? 1000);
    t.unref?.();
  });
  cam.send("TerminalSuspend", { id });
  return Promise.race([answer, timeout]).then((r) => ({ supported: r.supported === true }));
}

/**
 * Take the terminal back after `suspendTerminal`: the renderer re-enters
 * raw mode and redraws below whatever the child printed.
 *
 * @param {CamouflageHandle} cam
 */
export function resumeTerminal(cam) {
  cam.send("TerminalResume", { id: "resume" });
}

/**
 * Ask the user for permission and resolve to their answer. Resolves
 * `{ choice: "deny" }` if the renderer exits first.
 *
 * @param {CamouflageHandle} cam
 * @param {{request_id: string, tool: string, action: string, detail?: string, diff?: object}} spec
 * @returns {Promise<{request_id: string, choice: "allow_once"|"allow_session"|"deny", feedback?: string}>}
 */
export function permission(cam, spec) {
  const done = awaitResponse(
    cam,
    "permissionResponse",
    (r) => r.request_id === spec.request_id,
    { request_id: spec.request_id, choice: "deny", feedback: "" },
  );
  cam.send("PermissionRequested", spec);
  return done;
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
  const done = awaitResponse(cam, "formResponse", (r) => r.id === spec.id, { id: spec.id, cancelled: true });
  cam.send("ShowForm", spec);
  return done;
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
  const completed = awaitResponse(cam, "wizardCompleted", (r) => r.id === spec.id, null);
  const cancelled = awaitResponse(cam, "wizardCancelled", (r) => r.id === spec.id, null);
  cam.send("ShowWizard", spec);
  return Promise.race([
    completed.then((r) => r ?? { id: spec.id, cancelled: true, at_step: 0 }),
    cancelled.then((r) => ({ id: spec.id, cancelled: true, at_step: r?.at_step ?? 0 })),
  ]);
}

/**
 * Convenience helper: set the agent's todo/plan checklist. Replaces the
 * entire list (last-write-wins); pass an empty array to clear the panel.
 *
 * @param {CamouflageHandle} cam
 * @param {{id: string, title: string, status: "pending"|"in_progress"|"completed", started_at_ms?: number, token_delta?: number, progress?: number}[]} todos
 */
export function tasksSet(cam, todos) {
  cam.send("TodoListUpdate", { todos });
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
