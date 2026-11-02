import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { mount } from "./index.js";

const FAKE = join(dirname(fileURLToPath(import.meta.url)), "__fake-renderer.js");

/**
 * Mount the fake renderer (echoes every inbound NDJSON line back on stdout)
 * and collect everything it echoes. The fake stands in for camouflage-tui:
 * same wire protocol, no TTY required.
 */
async function mountRecording(t) {
  const cam = await mount({
    bin: process.execPath,
    args: [FAKE],
    skipDefaultArgs: true,
  });
  // Always reap the fake renderer, even when an assertion fails — a leaked
  // child keeps the event loop alive and hangs the whole test run.
  t.after(() => cam.close());
  const events = [];
  cam.on("event", (ev) => events.push(ev));
  const types = () => events.map((e) => e.event_type);
  const ofType = (type) => events.filter((e) => e.event_type === type);
  const waitFor = async (predicate, deadlineMs = 1000) => {
    const start = Date.now();
    while (Date.now() - start < deadlineMs) {
      if (predicate()) return;
      await new Promise((r) => setTimeout(r, 10));
    }
  };
  return { cam, events, types, ofType, waitFor };
}

test("turn.stream() drives start → delta → completed in order", async (t) => {
  const { cam, types, waitFor } = await mountRecording(t);
  const turn = cam.turn();
  const s = turn.stream();

  assert.equal(s.settled, false);
  s.write("hel");
  s.write("lo");
  s.reasoning("thinking…");
  s.end();

  await waitFor(() => types().filter((t) => t === "AssistantMessageCompleted").length === 1);
  assert.deepEqual(types(), [
    "AssistantStreamStarted",
    "AssistantTokenDelta",
    "AssistantTokenDelta",
    "AssistantReasoningDelta",
    "AssistantMessageCompleted",
  ]);
  assert.equal(s.settled, true);
  // Idempotent: a second end() sends nothing and doesn't throw.
  assert.equal(s.end(), false);
  assert.equal(types().length, 5);
  await cam.close();
});

test("turn.tool() drives start → stdout → finish, with ok mapped to done", async (t) => {
  const { cam, ofType, waitFor } = await mountRecording(t);
  const turn = cam.turn();
  const tool = turn.tool({ name: "Bash", args: "npm test", started_at_ms: 1_700_000_000_000 });

  tool.stdout("1..5\n");
  tool.stderr("warn\n");
  tool.finish({ status: "ok", summary: "5 passed", diff: { path: "a.ts", before: "a", after: "b" } });

  await waitFor(() => ofType("ToolExecutionFinished").length === 1);
  const [started] = ofType("ToolExecutionStarted");
  assert.equal(started.payload.tool, "Bash");
  assert.equal(started.payload.command, "npm test");
  assert.equal(started.payload.started_at_ms, 1_700_000_000_000);

  const [stdout, stderr] = [ofType("ToolExecutionStdout")[0], ofType("ToolExecutionStderr")[0]];
  assert.equal(stdout.payload.chunk, "1..5\n");
  assert.equal(stderr.payload.chunk, "warn\n");

  const [finished] = ofType("ToolExecutionFinished");
  assert.equal(finished.payload.status, "done");
  assert.equal(finished.payload.exit_code, 0);
  assert.equal(finished.payload.summary, "5 passed");
  assert.deepEqual(finished.payload.diff, { path: "a.ts", before: "a", after: "b" });
  assert.equal(tool.settled, true);
  await cam.close();
});

test("turn.end() auto-settles an open stream and an open tool", async (t) => {
  const { cam, ofType, types, waitFor } = await mountRecording(t);
  const turn = cam.turn();

  // The host forgets every finish event: no s.end(), no tool.finish().
  const s = turn.stream();
  s.write("half a rep");
  const tool = turn.tool({ name: "Read", args: "src/a.ts" });
  tool.stdout("chunk");

  turn.end();

  await waitFor(() => ofType("ToolExecutionFinished").length === 1);
  const completed = ofType("AssistantMessageCompleted");
  assert.equal(completed.length, 1, "open stream must be completed by turn.end()");
  assert.equal(completed[0].payload.stream_id, s.id);

  const finished = ofType("ToolExecutionFinished");
  assert.equal(finished.length, 1, "open tool must be finished by turn.end()");
  assert.equal(finished[0].payload.tool_id, tool.id);
  assert.equal(finished[0].payload.status, "done");

  // The busy phase is cleared so the renderer's spinner cannot outlive the turn.
  const status = ofType("StatusUpdate");
  assert.equal(status.length, 1);
  assert.deepEqual(status[0].payload.segments, { phase: "idle" });

  assert.equal(s.settled, true);
  assert.equal(tool.settled, true);
  assert.equal(turn.settled, true);

  // Everything settled exactly once — no duplicate settle events.
  assert.equal(types().filter((t) => t === "AssistantMessageCompleted").length, 1);
  assert.equal(types().filter((t) => t === "ToolExecutionFinished").length, 1);
  await cam.close();
});

test("turn.end() is idempotent and rejects new work afterwards", async (t) => {
  const { cam, types, waitFor } = await mountRecording(t);
  const turn = cam.turn();
  turn.stream();

  assert.equal(turn.end(), true);
  assert.equal(turn.end(), false, "a second end() must not settle again");
  await waitFor(() => types().includes("AssistantMessageCompleted"));
  assert.equal(types().filter((t) => t === "StatusUpdate").length, 1);

  assert.throws(() => turn.stream(), /already ended/);
  assert.throws(() => turn.tool({ name: "Bash" }), /already ended/);
  assert.throws(() => turn.permission({ title: "run" }), /already ended/);
  await cam.close();
});

test("turn.end() settles a pending permission as deny", async (t) => {
  const { cam, ofType, waitFor } = await mountRecording(t);
  const turn = cam.turn();

  const answer = turn.permission({ title: "edit src/auth/session.ts", tool: "edit", diff: { path: "a.ts" } });
  await waitFor(() => ofType("PermissionRequested").length === 1);
  const [requested] = ofType("PermissionRequested");
  assert.equal(requested.payload.action, "edit src/auth/session.ts");
  assert.equal(requested.payload.tool, "edit");
  assert.equal(typeof requested.payload.request_id, "string");

  turn.end(); // nobody answered
  const resolved = await answer;
  assert.equal(resolved.choice, "deny");
  assert.equal(resolved.request_id, requested.payload.request_id);
  await cam.close();
});

test("a permission answered before end() keeps the user's answer", async (t) => {
  const { cam, waitFor } = await mountRecording(t);
  const turn = cam.turn();
  const answer = turn.permission({ request_id: "r1", tool: "bash", action: "run npm test" });
  // Stand in for the user.
  cam.send("PermissionResponse", { request_id: "r1", choice: "allow_session", feedback: "" });
  const resolved = await answer;
  turn.end();
  assert.equal(resolved.choice, "allow_session");
  await waitFor(() => turn.settled);
  assert.equal((await answer).choice, "allow_session", "end() must not overwrite the answer");
  await cam.close();
});

test("renderer exit auto-settles every open handle and promise", async (t) => {
  const { cam } = await mountRecording(t);
  const turn = cam.turn();
  const s = turn.stream();
  s.write("streaming…");
  const tool = turn.tool({ name: "Bash", args: "sleep 100" });
  tool.stdout("working");
  const answer = turn.permission({ title: "run it", tool: "bash" });

  assert.equal(turn.settled, false);
  cam.kill("SIGKILL");

  const resolved = await answer; // must not hang on a dead renderer
  assert.equal(resolved.choice, "deny");
  assert.equal(s.settled, true, "open stream settles on renderer exit");
  assert.equal(tool.settled, true, "open tool settles on renderer exit");
  assert.equal(turn.settled, true, "turn settles on renderer exit");
});

test("turns opened after an exit settle immediately too", async (t) => {
  const { cam } = await mountRecording(t);
  cam.kill("SIGKILL");
  await new Promise((r) => cam.on("exit", r));

  const turn = cam.turn();
  const s = turn.stream();
  const answer = turn.permission({ title: "anything" });
  // send() is a forgiving no-op on a dead renderer; the handle still settles.
  assert.equal(s.end(), false);
  assert.equal((await answer).choice, "deny");
  turn.end();
  assert.equal(turn.settled, true);
});

test("raw send() stays available alongside handles", async (t) => {
  const { cam, types, waitFor } = await mountRecording(t);
  const turn = cam.turn();
  const s = turn.stream();

  cam.send("Splash", { text: "logo" }); // escape hatch, interleaved freely
  s.write("hi");
  turn.end();

  await waitFor(() => types().includes("AssistantMessageCompleted"));
  assert.ok(types().includes("Splash"));
  await cam.close();
});

test("unknown tool statuses throw instead of settling silently", async (t) => {
  const { cam } = await mountRecording(t);
  const turn = cam.turn();
  const tool = turn.tool({ name: "Bash" });
  assert.throws(() => tool.finish({ status: "meh" }), /unknown tool status/);
  assert.throws(() => turn.end({ toolStatus: "meh" }), /unknown tool status/);
  assert.equal(tool.settled, false, "a rejected finish must leave the handle open");
  await cam.close();
});

test("turn ids and stream ids are unique and host-facing only", async (t) => {
  const { cam, ofType, waitFor } = await mountRecording(t);
  const a = cam.turn({ id: "my-turn" });
  const b = cam.turn();
  assert.equal(a.id, "my-turn");
  assert.notEqual(a.id, b.id);

  const s1 = a.stream();
  const s2 = a.stream();
  assert.notEqual(s1.id, s2.id);
  a.end();
  b.end();

  await waitFor(() => ofType("AssistantStreamStarted").length === 2);
  const ids = ofType("AssistantStreamStarted").map((e) => e.payload.stream_id);
  assert.deepEqual(ids, [s1.id, s2.id]);
  // The turn id never leaks onto the wire.
  assert.ok(!JSON.stringify(ofType("AssistantStreamStarted")).includes("my-turn"));
  await cam.close();
});
