#!/usr/bin/env node
// Test harness only: reports the arguments it was started with as one
// UserInputSubmitted event, so tests can check what mount() passes.
process.stdout.write(
  JSON.stringify({ event_type: "UserInputSubmitted", payload: { text: process.argv.slice(2).join(" ") } }) + "\n",
);
process.stdin.resume();
process.stdin.on("end", () => process.exit(0));
