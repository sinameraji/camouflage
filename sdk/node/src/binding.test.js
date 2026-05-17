import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { mount, selectList, confirm } from "./index.js";

const FAKE = join(dirname(fileURLToPath(import.meta.url)), "__fake-renderer.js");

/**
 * Mount the fake renderer (a tiny Node script that echoes any inbound
 * NDJSON line back to its stdout). The fake stands in for the real
 * camouflage-tui binary — same wire protocol, but no TTY requirement.
 */
async function mountFake() {
  return mount({
    bin: process.execPath,
    args: [FAKE],
    skipDefaultArgs: true, // fake doesn't take --stdin-events / --emit-responses
  });
}

test("mount() spawns and resolves a handle", async () => {
  const cam = await mountFake();
  assert.equal(typeof cam.send, "function");
  assert.equal(typeof cam.close, "function");
  await cam.close();
});

test("mount() rejects when the binary is missing", async () => {
  await assert.rejects(
    () => mount({ bin: "/this/does/not/exist/camouflage-xxxxxx", skipDefaultArgs: true }),
    /could not find renderer binary|ENOENT/,
  );
});

test("emit() writes one line per call; outbound events round-trip", async () => {
  const cam = await mountFake();
  const received = [];
  cam.on("event", (ev) => received.push(ev));

  cam.send("SessionStarted", {});
  cam.send("UserMessageCreated", { text: "hello" });
  cam.send("AssistantTokenDelta", { stream_id: "s1", token: "hi" });

  // Wait for the fake to echo back
  await waitUntil(() => received.length >= 3, 1000);
  assert.equal(received.length, 3);
  assert.deepEqual(
    received.map((e) => e.event_type),
    ["SessionStarted", "UserMessageCreated", "AssistantTokenDelta"],
  );
  await cam.close();
});

test("userInput convenience event fires for UserInputSubmitted", async () => {
  const cam = await mountFake();
  let got = null;
  cam.on("userInput", (text) => { got = text; });
  cam.send("UserInputSubmitted", { text: "follow-up question" });
  await waitUntil(() => got !== null, 500);
  assert.equal(got, "follow-up question");
  await cam.close();
});

test("permissionResponse convenience event fires", async () => {
  const cam = await mountFake();
  let got = null;
  cam.on("permissionResponse", (r) => { got = r; });
  cam.send("PermissionResponse", { request_id: "perm-1", choice: "allow_once" });
  await waitUntil(() => got !== null, 500);
  assert.deepEqual(got, { request_id: "perm-1", choice: "allow_once", feedback: undefined });
  await cam.close();
});

test("send() throws after close()", async () => {
  const cam = await mountFake();
  await cam.close();
  assert.throws(() => cam.send("SessionStarted", {}), /after close/);
});

test("close() resolves with the child's exit code", async () => {
  const cam = await mountFake();
  const code = await cam.close();
  assert.equal(code, 0);
});

test("selectList() helper round-trips via the fake echo renderer", async () => {
  // The fake echoes any inbound NDJSON back on stdout. Sending a ShowSelectList
  // makes the fake "respond" with that exact event — not a real
  // SelectListResponse. To simulate the real flow we instead send the response
  // directly: the helper subscribes to selectListResponse before sending
  // ShowSelectList, so any matching SelectListResponse echoed back resolves it.
  const cam = await mountFake();
  // Don't await — race the helper against a manually-sent response.
  const p = selectList(cam, {
    id: "test-1",
    prompt: "Pick one",
    options: [{ value: "a", label: "A" }, { value: "b", label: "B" }],
  });
  // Echo what'll be parsed as the response.
  cam.send("SelectListResponse", { id: "test-1", value: "b" });
  const resp = await p;
  assert.equal(resp.id, "test-1");
  assert.equal(resp.value, "b");
  assert.equal(resp.cancelled, false);
  await cam.close();
});

test("confirm() helper round-trips via the fake echo renderer", async () => {
  const cam = await mountFake();
  const p = confirm(cam, { id: "quit-1", prompt: "Save?" });
  cam.send("ConfirmResponse", { id: "quit-1", value: true });
  const resp = await p;
  assert.equal(resp.id, "quit-1");
  assert.equal(resp.value, true);
  assert.equal(resp.cancelled, false);
  await cam.close();
});

test("send() rejects bad event_type argument", async () => {
  const cam = await mountFake();
  assert.throws(() => cam.send("", {}), /non-empty string/);
  assert.throws(() => cam.send(null, {}), /non-empty string/);
  await cam.close();
});

/**
 * Poll-wait until `predicate()` returns truthy or the deadline elapses.
 * Resolves either way; the caller asserts.
 */
async function waitUntil(predicate, deadlineMs) {
  const start = Date.now();
  while (Date.now() - start < deadlineMs) {
    if (predicate()) return;
    await new Promise((r) => setTimeout(r, 10));
  }
}
