#!/usr/bin/env node
/**
 * Test harness only — stands in for `camouflage-tui` in binding tests.
 *
 * Behaviour: reads NDJSON lines from stdin and for each known event_type,
 * echoes it back on stdout (simulating what camouflage-tui would emit as
 * outbound traffic). This lets us validate that the Node binding's
 * spawn → write → readline parse → typed-event pipeline works end-to-end
 * without needing a real Rust binary or a TTY.
 *
 * The real binding does NOT echo inbound back — but for test purposes,
 * any inbound event is "valid outbound" since both directions use the
 * same NDJSON shape.
 */
import { createInterface } from "node:readline";

const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;
  try {
    JSON.parse(trimmed); // sanity check
    process.stdout.write(trimmed + "\n");
  } catch {
    // skip malformed
  }
});
rl.on("close", () => process.exit(0));
