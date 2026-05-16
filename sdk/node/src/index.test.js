import { test } from "node:test";
import assert from "node:assert/strict";
import { Readable } from "node:stream";
import { reader, validate, encode } from "./index.js";

test("validate accepts a well-formed line", () => {
  const v = validate('{"event_type":"UserMessageCreated","payload":{"text":"hi"}}');
  assert.equal(v, null);
});

test("validate rejects unknown event_type", () => {
  const v = validate('{"event_type":"NotAThing","payload":{}}');
  assert.match(v ?? "", /unknown event_type/);
});

test("validate rejects non-JSON", () => {
  const v = validate("not json");
  assert.match(v ?? "", /not valid JSON/);
});

test("reader parses NDJSON line-by-line and skips malformed", async () => {
  const src = Readable.from([
    '{"event_type":"SessionStarted","payload":{}}\n',
    "not json\n",
    '{"event_type":"UserMessageCreated","payload":{"text":"hello"}}\n',
    '{"event_type":"SessionEnded","payload":{}}\n',
  ]);
  const out = [];
  for await (const ev of reader(src)) out.push(ev);
  assert.deepEqual(
    out.map((e) => e.event_type),
    ["SessionStarted", "UserMessageCreated", "SessionEnded"],
  );
});

test("encode roundtrips through reader", async () => {
  const ev = {
    event_type: "StatusUpdate",
    payload: { segments: { phase: "thinking", mode: "edit" } },
  };
  const src = Readable.from([encode(ev) + "\n"]);
  const out = [];
  for await (const e of reader(src)) out.push(e);
  assert.equal(out.length, 1);
  assert.equal(out[0].payload.segments.phase, "thinking");
});
