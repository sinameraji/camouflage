# Background activity (jobs and agents)

Long-running work the host owns: shell jobs (a dev server, a test watcher)
and worker agents. The inline renderer shows it without making users ask:
a footer badge, a browser on **Ctrl+B**, live output for each item, a stop
request, and a transcript line when something starts, finishes, fails or
needs attention.

The host owns every process. Camouflage only displays what the host
reports and turns keys into requests; it never starts, stops or attaches to
anything. Output is a followed log, not an interactive terminal (no stdin,
no PTY).

Try it with mock data:

```bash
cargo build --release -p camouflage-tui
CAMOUFLAGE_BIN=target/release/camouflage-tui node examples/activity-demo/demo.mjs
```

## What the user sees

- **Badge** in the footer whenever there are items: `◆ 2 jobs · 1 agent ·
  ctrl+b`, yellow with `· 1 needs attention` when something is waiting on
  the user, `◇ 3 finished` once nothing is active. It's static: a job that
  runs for hours costs no wakeups.
- **Browser** (Ctrl+B on an empty input, or the host sends
  `ActivityBrowserOpen`, e.g. for `/jobs`): agents, then jobs; within each,
  items needing attention, then running, then finished. ↑↓ select, Enter
  details, `s` stop, Esc close. It replaces the input box in the live area,
  so the transcript is untouched and comes back exactly as it was.
- **Details**: status and elapsed time, the summary, a progress bar, up to
  six steps, then the output, following the newest lines. ↑↓ / PgUp / PgDn
  scroll back, End follows again, `s` stop, Esc back to the list.
- **Stop**: only for items marked `stoppable` that are still active. The
  user confirms (`y`), the renderer sends `ActivityStopRequested` and shows
  "stopping…" until the host reports the new status.
- **Transcript lines** (no browser needed): `· Started job · npm run dev`,
  `✓ Job finished · …`, `✗ Job failed · … · exit 1`, `! Agent needs
  attention · … · approve a deploy`, `· Stopped job · …`.
- Elapsed times tick once a second only while the browser is open.

## Protocol (v2.4+)

Host → renderer:

| Event | Payload | Meaning |
|---|---|---|
| `ActivityUpdate` | `ActivityItem` | Add or replace one item. Fields left out keep their values; output is kept. |
| `ActivityLog` | `{ id, chunk, stream? }` | Append output. Chunks may span lines or end mid-line; ANSI is stripped; the last 2000 lines are kept. |
| `ActivityRemoved` | `{ id }` | Drop an item. |
| `ActivitySnapshot` | `{ items: ActivityItem[] }` | The complete list. Items not in it are dropped, so nothing stale survives a reconnect. Known items' status changes are announced; new ones aren't. |
| `ActivityBrowserOpen` | none | Open the browser (route `/jobs` or `/agents` here). |

Renderer → host:

| Event | Payload | Meaning |
|---|---|---|
| `ActivityStopRequested` | `{ id }` | The user confirmed a stop. Stop it (or don't) and report the result with `ActivityUpdate`. |
| `ActivityViewChanged` | `{ view: "list" \| "detail" \| "closed", id? }` | Where the browser is. Use it to stream an item's output only while its details are open. |

`ActivityItem`:

```ts
{
  id: string;                 // stable; updates, logs and removals use it
  kind?: "job" | "agent";
  title?: string;             // command, task or agent name
  status?: "running" | "waiting" | "needs_attention" | "done" | "failed" | "stopped";
  stoppable?: boolean;
  summary?: string;           // one line: current step, exit code, what it waits for
  progress?: number;          // 0..1
  started_at_ms?: number;
  updated_at_ms?: number;     // for finished items: when they finished
  steps?: { title: string; status?: "pending" | "running" | "done" | "failed" }[];
}
```

Active statuses are `running`, `waiting` and `needs_attention`; the badge
counts those. SDK: `cam.on("activityStopRequested", …)` and
`cam.on("activityViewChanged", …)`.

## What autopilot needs to add

The renderer side is complete; autopilot's bridge doesn't send any of this
yet. In `src/ui/app-bridge.ts` / `src/camouflage-view.ts`:

1. **Snapshot**: add `activity: ActivityItem[]` to `AppSnapshot`, built from
   the job manager (`src/jobs/manager.ts`) and the agent supervisor
   (`src/agent/supervisor.ts`). Map job state to `status` (running → running,
   exit 0 → done with `summary: "exit 0"`, non-zero → failed, cancelled →
   stopped), `stoppable: true` for running jobs, `title` = the command.
   Agents: `title` = task, `summary` = current step, `steps` from the
   worker's plan, `needs_attention` when it waits on the user.
2. **Sync**: in `View.sync`, diff `activity` against what was sent: send
   `ActivityUpdate` for new or changed items and `ActivityRemoved` for gone
   ones. Send one `ActivitySnapshot` right after mount.
3. **Output**: jobs write to log files. On `activityViewChanged` with
   `view: "detail"`, tail that job's log and send `ActivityLog` chunks (send
   the last ~200 lines first); stop tailing on `list` / `closed`. Agents can
   send recent output the same way.
4. **Actions**: add `stopActivity(id)` to `AppActions` (cancel the job via
   the manager, or the worker via the supervisor) and call it from
   `cam.on("activityStopRequested")`.
5. **Commands**: make `/jobs` (and `/agents`) send `ActivityBrowserOpen`
   under Camouflage instead of printing a list.
