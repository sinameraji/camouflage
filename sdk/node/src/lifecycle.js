/**
 * Camouflage lifecycle API — turn / stream / tool handles that auto-settle.
 *
 * The raw protocol is ~41 low-level events and the *host* has to get the
 * ordering right: start a stream before its deltas, finish every tool, end
 * every stream, clear the busy phase. Forget one finish event and the
 * renderer keeps a spinner running forever (the root of several past
 * "stuck spinner" bugs).
 *
 * This module flips that around: the host opens a handle, writes meaning
 * into it, and closing the handle — or the whole turn — settles everything
 * underneath it.
 *
 *   const turn = cam.turn();
 *   const s = turn.stream();            // AssistantStreamStarted
 *   s.write(token);                     // AssistantTokenDelta
 *   s.end();                            // AssistantMessageCompleted
 *
 *   const tool = turn.tool({ name, args });   // ToolExecutionStarted
 *   tool.stdout(chunk);                      // ToolExecutionStdout
 *   tool.finish({ status: "ok", diff });     // ToolExecutionFinished
 *
 *   const ok = await turn.permission({ title, diff });  // PermissionRequested
 *   turn.end();     // settles every open stream/tool/permission + phase
 *
 * Guarantees:
 *   - Closing a handle is idempotent; a second `end()`/`finish()` no-ops.
 *   - `turn.end()` settles everything still open under that turn, so a
 *     missing finish event can never leave UI state running.
 *   - When the renderer exits, every open handle settles and every pending
 *     `turn.permission()` resolves `{ choice: "deny" }` — no host hangs.
 *   - Raw `cam.send()` stays available as the escape hatch; handles and
 *     raw sends interleave freely.
 */

/** Process-unique, human-readable ids (host bookkeeping only). */
let seq = 0;
function nextId(prefix) {
  seq += 1;
  return `${prefix}-${Date.now().toString(36)}-${seq.toString(36)}`;
}

/** Host-friendly tool statuses → the protocol's ToolFinished.status. */
const TOOL_STATUSES = new Map([
  ["ok", "done"],
  ["done", "done"],
  ["error", "error"],
  ["failed", "error"],
  ["cancelled", "cancelled"],
  ["canceled", "cancelled"],
  ["rejected", "rejected"],
]);

/**
 * Normalize a host-facing status to the wire value. Unknown statuses throw:
 * silently settling a tool as something the host didn't mean is worse than
 * a loud error.
 *
 * @param {string} [status]
 * @returns {"done"|"error"|"cancelled"|"rejected"}
 */
function normalizeToolStatus(status) {
  if (status == null) return "done";
  const normalized = TOOL_STATUSES.get(String(status).toLowerCase());
  if (!normalized) {
    throw new TypeError(
      `camouflage: unknown tool status ${JSON.stringify(status)} ` +
      `(expected one of: ${[...new Set(TOOL_STATUSES.keys())].join(", ")})`,
    );
  }
  return normalized;
}

/** One assistant message being streamed. Settles with AssistantMessageCompleted. */
class StreamHandle {
  constructor(turn, id) {
    this._turn = turn;
    this.id = id;
    this._open = true;
  }

  /** True once the stream has been ended (or auto-settled). */
  get settled() {
    return !this._open;
  }

  /**
   * Append text to the stream (AssistantTokenDelta). Returns false once the
   * stream has ended — a late token is dropped, never thrown.
   *
   * @param {unknown} token
   */
  write(token) {
    if (!this._open) return false;
    return this._turn._send("AssistantTokenDelta", {
      stream_id: this.id,
      token: token == null ? "" : String(token),
    });
  }

  /**
   * Append the model's reasoning (AssistantReasoningDelta), shown when the
   * user presses Ctrl+R in the inline UI.
   *
   * @param {unknown} token
   */
  reasoning(token) {
    if (!this._open) return false;
    return this._turn._send("AssistantReasoningDelta", {
      stream_id: this.id,
      token: token == null ? "" : String(token),
    });
  }

  /**
   * Finish the stream (AssistantMessageCompleted). Idempotent.
   *
   * @param {{text?: string}} [opts] `text` replaces what was streamed.
   * @returns {boolean}
   */
  end(opts = {}) {
    if (!this._open) return false;
    this._open = false;
    this._turn._forget(this);
    const payload = { stream_id: this.id };
    if (opts && opts.text !== undefined) payload.text = opts.text;
    return this._turn._send("AssistantMessageCompleted", payload);
  }
}

/** One tool execution. Settles with ToolExecutionFinished. */
class ToolHandle {
  constructor(turn, spec) {
    this._turn = turn;
    this.id = spec.id ?? spec.tool_id ?? nextId("tool");
    this._open = true;
    this._spec = spec;
  }

  /** True once the tool has been finished (or auto-settled). */
  get settled() {
    return !this._open;
  }

  /** Announce the tool (ToolExecutionStarted). Called by `turn.tool()`. */
  _start() {
    const spec = this._spec;
    const payload = {
      tool_id: this.id,
      tool: String(spec.name ?? spec.tool ?? ""),
      command: String(spec.args ?? spec.command ?? ""),
    };
    if (spec.started_at_ms != null) payload.started_at_ms = spec.started_at_ms;
    this._spec = null;
    return this._turn._send("ToolExecutionStarted", payload);
  }

  /**
   * Stream a chunk of the tool's output (ToolExecutionStdout).
   * @param {unknown} chunk
   */
  stdout(chunk) {
    return this._io("ToolExecutionStdout", chunk);
  }

  /**
   * Stream a chunk of the tool's stderr (ToolExecutionStderr).
   * @param {unknown} chunk
   */
  stderr(chunk) {
    return this._io("ToolExecutionStderr", chunk);
  }

  /** Alias for `stdout()` — the common case. */
  write(chunk) {
    return this._io("ToolExecutionStdout", chunk);
  }

  _io(event_type, chunk) {
    if (!this._open) return false;
    return this._turn._send(event_type, {
      tool_id: this.id,
      chunk: chunk == null ? "" : String(chunk),
    });
  }

  /**
   * Finish the tool (ToolExecutionFinished). Idempotent.
   *
   * @param {object} [spec]
   * @param {string} [spec.status] "ok" (default), "error", "cancelled" or
   *   "rejected" — "ok"/"done" both mean success.
   * @param {number} [spec.exit_code] Defaults to 0 (1 when status is an error).
   * @param {string} [spec.summary] One-line result under the tool row.
   * @param {string} [spec.output] Full output; replaces streamed chunks.
   * @param {number} [spec.preview] Lines shown before "ctrl+o to expand".
   * @param {string} [spec.output_lang] Highlight the output as this language.
   * @param {object} [spec.diff] Diff shown under the tool row.
   * @returns {boolean}
   */
  finish(spec = {}) {
    if (!this._open) return false;
    // Validate before mutating: a bad status must leave the handle open.
    const status = normalizeToolStatus(spec.status);
    this._open = false;
    this._turn._forget(this);
    const payload = {
      tool_id: this.id,
      exit_code: spec.exit_code ?? (status === "error" ? 1 : 0),
      status,
    };
    for (const key of ["summary", "output", "preview", "output_lang", "diff"]) {
      if (spec[key] !== undefined) payload[key] = spec[key];
    }
    return this._turn._send("ToolExecutionFinished", payload);
  }
}

/**
 * One unit of work ("a turn"). Owns the streams, tools and permission
 * requests opened under it; `end()` settles all of them.
 */
class Turn {
  constructor(cam, opts, hooks) {
    this._cam = cam;
    this._hooks = hooks;
    /** Host-facing id; never sent on the wire (the protocol has no turn event). */
    this.id = opts.id ?? nextId("turn");
    this._settleToolStatus = normalizeToolStatus(opts.settleToolStatus);
    this._children = new Set();
    /** request_id → settle() for a permission still waiting on the user. */
    this._pending = new Map();
    this._ended = false;
  }

  /** True once `end()` has run (directly or via auto-settle). */
  get settled() {
    return this._ended;
  }

  _assertOpen() {
    if (this._ended) {
      throw new Error(
        `camouflage: turn ${this.id} has already ended — ` +
        `start a new one with cam.turn()`,
      );
    }
  }

  _send(event_type, payload) {
    return this._cam.send(event_type, payload);
  }

  _forget(child) {
    this._children.delete(child);
  }

  /**
   * Open an assistant stream (AssistantStreamStarted).
   *
   * @param {{id?: string}} [opts] `id` defaults to a generated stream id.
   * @returns {StreamHandle}
   */
  stream(opts = {}) {
    this._assertOpen();
    const handle = new StreamHandle(this, opts.id ?? opts.stream_id ?? nextId("stream"));
    this._children.add(handle);
    this._send("AssistantStreamStarted", { stream_id: handle.id });
    return handle;
  }

  /**
   * Start a tool (ToolExecutionStarted).
   *
   * @param {object} [spec]
   * @param {string} [spec.name] Tool name shown in bold, e.g. "Bash".
   * @param {string} [spec.args] Arguments shown after the name.
   * @param {string} [spec.id] Stable tool id; generated when omitted.
   * @param {number} [spec.started_at_ms] When the tool started (epoch ms).
   * @returns {ToolHandle}
   */
  tool(spec = {}) {
    this._assertOpen();
    const handle = new ToolHandle(this, spec);
    this._children.add(handle);
    handle._start();
    return handle;
  }

  /**
   * Ask the user for permission (PermissionRequested) and resolve to their
   * answer. Resolves `{ choice: "deny" }` if the renderer exits first or if
   * the turn ends before anyone answers — a caller never hangs.
   *
   * @param {object} [spec]
   * @param {string} [spec.title] Prompt title, e.g. "edit src/auth/session.ts".
   * @param {string} [spec.tool] Tool being authorized, e.g. "bash".
   * @param {string} [spec.detail] Dim line under the title.
   * @param {object} [spec.diff] Diff preview for edits.
   * @param {string} [spec.request_id] Generated when omitted.
   * @returns {Promise<{request_id: string, choice: "allow_once"|"allow_session"|"deny", feedback?: string}>}
   */
  permission(spec = {}) {
    this._assertOpen();
    const request_id = spec.request_id ?? nextId("perm");
    const payload = { request_id };
    const tool = spec.tool ?? spec.name;
    if (tool != null) payload.tool = String(tool);
    const action = spec.title ?? spec.action;
    if (action != null) payload.action = String(action);
    if (spec.detail != null) payload.detail = String(spec.detail);
    if (spec.diff != null) payload.diff = spec.diff;

    let resolveFn;
    const result = new Promise((resolve) => { resolveFn = resolve; });
    let done = false;
    const finish = (value) => {
      if (done) return;
      done = true;
      this._pending.delete(request_id);
      resolveFn(value);
    };
    // What `end()` (or a renderer exit) settles to while unanswered.
    this._pending.set(request_id, () => {
      finish({ request_id, choice: "deny", feedback: "" });
    });
    if (this._hooks.askPermission) {
      this._hooks.askPermission(this._cam, payload).then(finish);
    }
    return result;
  }

  /**
   * End the turn: settle every stream, tool and permission still open under
   * it, then clear the busy phase so no spinner is left running.
   *
   * Idempotent — calling it twice (or after everything already settled)
   * sends nothing further and returns false.
   *
   * @param {object} [opts]
   * @param {"ok"|"done"|"error"|"cancelled"|"rejected"} [opts.toolStatus]
   *   Status given to tools still open at end(). Default "done".
   * @param {string|null} [opts.phase="idle"] Value for the `phase` status
   *   segment; the renderer settles its spinner when phase is not busy.
   *   Pass `null` to leave the phase alone (e.g. when another turn owns it).
   * @returns {boolean} true if this call did the settling.
   */
  end(opts = {}) {
    if (this._ended) return false;
    // Validate before mutating: a bad status must leave the turn open.
    const toolStatus = opts.toolStatus !== undefined
      ? normalizeToolStatus(opts.toolStatus)
      : this._settleToolStatus;
    this._ended = true;

    for (const child of [...this._children]) {
      if (child instanceof StreamHandle) child.end();
      else child.finish({ status: toolStatus });
    }
    this._children.clear();

    for (const settle of [...this._pending.values()]) settle();
    this._pending.clear();

    const phase = opts.phase === undefined ? "idle" : opts.phase;
    if (phase !== null) {
      this._send("StatusUpdate", { segments: { phase } });
    }

    this._hooks.onEnd?.(this);
    return true;
  }
}

/**
 * Build the lifecycle facade bound to one handle.
 *
 * @param {object} cam      A CamouflageHandle (needs `.send()`).
 * @param {{permission?: Function}} [helpers]
 * @returns {{turn: (opts?: object) => Turn, settleAll: () => void, openTurns: () => number}}
 */
export function createLifecycle(cam, helpers = {}) {
  const turns = new Set();

  const hooks = {
    askPermission: helpers.permission,
    onEnd: (turn) => turns.delete(turn),
  };

  return {
    /**
     * Open a new turn. Every stream, tool and permission created under it
     * settles when `turn.end()` runs — or when the renderer exits.
     *
     * @param {{id?: string, settleToolStatus?: string}} [opts]
     * @returns {Turn}
     */
    turn(opts = {}) {
      const turn = new Turn(cam, opts ?? {}, hooks);
      turns.add(turn);
      return turn;
    },
    /** Settle every turn still open (used on renderer exit). Idempotent. */
    settleAll() {
      for (const turn of [...turns]) turn.end();
    },
    /** How many turns are still open. */
    openTurns() {
      return turns.size;
    },
  };
}

export { StreamHandle, ToolHandle, Turn, normalizeToolStatus };
