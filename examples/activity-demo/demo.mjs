// Background-activity demo: mock jobs and agents driving the inline
// renderer, the way a host like autopilot would.
//
//   cargo build --release -p camouflage-tui
//   CAMOUFLAGE_BIN=target/release/camouflage-tui node examples/activity-demo/demo.mjs
//
// Press ctrl+b (with an empty input) to open the browser; enter for details,
// s to stop the dev server, esc to go back. ctrl+c twice quits.
import { mount } from "../../sdk/node/src/index.js";

const cam = await mount({ ui: "inline", appTitle: "demo", ...(process.env.CAMOUFLAGE_BIN ? { bin: process.env.CAMOUFLAGE_BIN } : {}) });
const send = (type, payload) => cam.send(type, payload);
const now = () => Date.now();

send("SessionStarted", { title: "activity demo", detail: ["ctrl+b opens background activity"] });

// A long-running job that keeps printing.
send("ActivityUpdate", { id: "dev", kind: "job", title: "npm run dev", status: "running", stoppable: true, started_at_ms: now(), summary: "listening on :3000" });
let req = 0;
const devLog = setInterval(() => send("ActivityLog", { id: "dev", chunk: `GET /api/items ${200 + (req++ % 3 === 0 ? 4 : 0)} ${10 + (req % 40)}ms\n` }), 700);

// A job that finishes.
send("ActivityUpdate", { id: "test", kind: "job", title: "npm test -- --watch=false", status: "running", stoppable: true, started_at_ms: now() });
let n = 0;
const testLog = setInterval(() => {
  n++;
  send("ActivityLog", { id: "test", chunk: `✓ suite ${n} (${n * 3} tests)\n` });
  if (n === 8) {
    clearInterval(testLog);
    send("ActivityUpdate", { id: "test", status: "done", summary: "24 passed", updated_at_ms: now() });
  }
}, 900);

// An agent with steps that later needs attention.
const steps = ["Read the auth module", "Find session handling", "Draft the fix", "Run the tests"];
send("ActivityUpdate", { id: "worker", kind: "agent", title: "research: session expiry bug", status: "running", started_at_ms: now(), steps: steps.map((title, i) => ({ title, status: i === 0 ? "running" : "pending" })) });
let step = 0;
const agent = setInterval(() => {
  step++;
  if (step < steps.length) {
    send("ActivityUpdate", { id: "worker", progress: step / steps.length, summary: steps[step], steps: steps.map((title, i) => ({ title, status: i < step ? "done" : i === step ? "running" : "pending" })) });
    send("ActivityLog", { id: "worker", chunk: `step ${step}: ${steps[step]}\n` });
  } else {
    clearInterval(agent);
    send("ActivityUpdate", { id: "worker", status: "needs_attention", summary: "wants to edit src/auth/session.ts" });
  }
}, 2500);

cam.on("activityStopRequested", ({ id }) => {
  // The host owns the process: stop it, then report what happened.
  if (id === "dev") clearInterval(devLog);
  send("ActivityLog", { id, chunk: "^C\n" });
  send("ActivityUpdate", { id, status: "stopped", summary: "stopped by you", updated_at_ms: now() });
});
cam.on("activityViewChanged", (v) => {
  // e.g. only stream a job's output while its details are open.
  void v;
});
cam.on("exit", () => process.exit(0));
