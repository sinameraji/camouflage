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
import { encode, validate } from "./types.js";

/**
 * Default executable name. Looked up on PATH unless `opts.bin` is set
 * to an absolute or relative path.
 */
const DEFAULT_BIN = "camouflage-tui";

class CamouflageHandle extends EventEmitter {
  constructor(child, stdin) {
    super();
    this._child = child;
    this._stdin = stdin;
    this._closed = false;
    this._closing = null; // Promise once close() begins
  }

  /**
   * Emit one event into the renderer. Returns synchronously; under the
   * hood this writes one NDJSON line to the renderer's stdin. Throws
   * if the renderer has exited.
   *
   * @param {string} event_type
   * @param {object} [payload]
   */
  emit(event_type, payload = {}) {
    if (this._closed) {
      throw new Error("camouflage: emit() after close()");
    }
    if (typeof event_type !== "string" || !event_type) {
      throw new TypeError("camouflage: event_type must be a non-empty string");
    }
    const line = encode({ event_type, payload });
    // EventEmitter.emit and our wire-emit are distinct names internally;
    // ours uses _writeLine to avoid shadowing.
    return this._writeLine(line);
  }

  /**
   * Emit a pre-built Event object. Slight efficiency win for hot paths
   * because the caller can stringify once and reuse.
   *
   * @param {{event_type: string, payload?: object}} ev
   */
  emitEvent(ev) {
    if (this._closed) {
      throw new Error("camouflage: emitEvent() after close()");
    }
    return this._writeLine(encode(ev));
  }

  _writeLine(line) {
    if (!this._stdin.writable) {
      throw new Error("camouflage: renderer stdin is no longer writable");
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
  const args = ["--stdin-events", "--emit-responses", ...(opts.args || [])];
  const stderrMode = opts.inheritStderr === false ? "pipe" : "inherit";

  const child = spawn(bin, args, {
    stdio: ["pipe", "pipe", stderrMode],
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
  // renderer stdout, parse, and re-emit on the handle.
  const rl = createInterface({ input: child.stdout });
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

function spawnError(err, bin) {
  if (err && err.code === "ENOENT") {
    return new Error(
      `camouflage: could not find renderer binary "${bin}". ` +
      `Install it via \`cargo install --path crates/tui\` from a checkout of ` +
      `https://github.com/sinameraji/camouflage, or pass { bin: "/absolute/path/to/camouflage-tui" } to mount().`,
    );
  }
  return err;
}
