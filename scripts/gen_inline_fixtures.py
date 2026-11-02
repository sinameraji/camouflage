#!/usr/bin/env python3
"""Generate the inline replay fixture corpus in `fixtures/inline/`.

Each fixture is an autopilot-shaped session: the same `{"t": ms, "in" | "out":
event}` lines a real host writes with `CAMOUFLAGE_RECORD`, covering long turns,
resizes, interrupts, every picker and form, permission prompts with diffs, and
`/clear`. They are generated rather than recorded so the corpus stays
reviewable and reproducible; a real recording drops in the same way (see
`fixtures/inline/README.md`).

Usage:
    python3 scripts/gen_inline_fixtures.py           # write the fixtures
    python3 scripts/gen_inline_fixtures.py --check   # fail if they are stale

The `.screen.txt` goldens are written by the replay test itself
(`UPDATE_GOLDEN=1 cargo test -p camouflage-inline --test replay`).
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "fixtures" / "inline"

COMMANDS = [
    ("help", "Show the commands and shortcuts", None),
    ("model", "Switch the model", "[name]"),
    ("hooks", "Manage hooks", None),
    ("sessions", "Resume a previous session", None),
    ("theme", "Pick a colour theme", None),
    ("agents", "Show running agents", None),
    ("mcp", "Manage MCP servers", None),
    ("permissions", "Review allowed tools", None),
    ("cost", "Show token usage and cost", None),
    ("clear", "Clear the transcript", None),
    ("compact", "Compact the conversation", None),
    ("settings", "Open settings", None),
]

MENTIONS = [
    "crates/inline/src/session.rs",
    "crates/inline/src/blocks.rs",
    "crates/inline/src/chrome.rs",
    "crates/inline/tests/replay.rs",
    "fixtures/inline/README.md",
    "scripts/gen_inline_fixtures.py",
    "Cargo.toml",
    "README.md",
]

DIFF_UNIFIED = """\
@@ -12,7 +12,9 @@ impl Session {
     pub fn apply(&mut self, ev: &Event, now: i64) {
         let p = &ev.payload;
-        match ev.event_type {
+        // Every host event lands here; a malformed payload must not panic.
+        let Some(kind) = ev.event_type else { return };
+        match kind {
             EventType::SessionStarted => {
                 self.start(p, now);
             }
"""


class Rec:
    """Builds one fixture's event lines."""

    def __init__(self) -> None:
        self.lines: list[dict] = []
        self.t = 0

    def at(self, t: int) -> "Rec":
        self.t = t
        return self

    def wait(self, ms: int) -> "Rec":
        self.t += ms
        return self

    def send(self, event_type: str, payload: dict | None = None) -> "Rec":
        self.lines.append(
            {"t": self.t, "in": {"event_type": event_type, "payload": payload or {}}}
        )
        return self

    def user(self, text: str) -> "Rec":
        self.lines.append(
            {"t": self.t, "out": {"event_type": "UserInputSubmitted", "payload": {"text": text}}}
        )
        return self

    def answer(self, event_type: str, payload: dict | None = None) -> "Rec":
        self.lines.append(
            {"t": self.t, "out": {"event_type": event_type, "payload": payload or {}}}
        )
        return self

    def resize(self, cols: int, rows: int) -> "Rec":
        """The terminal changed size; the harness calls `Session::resize`."""
        self.lines.append({"t": self.t, "resize": [cols, rows]})
        return self

    def key(self, name: str) -> "Rec":
        """A key press that isn't a prompt answer (ctrl+r, ctrl+o, ?, ...)."""
        self.lines.append({"t": self.t, "key": name})
        return self

    def snapshot(self, label: str) -> "Rec":
        """Keep the live area as it looks now, for mid-turn-only states."""
        self.lines.append({"t": self.t, "snapshot": label})
        return self

    # -- common shapes ---------------------------------------------------

    def start(self, project: str = "~/project", branch: str = "main") -> "Rec":
        self.send(
            "SessionStarted",
            {
                "title": "autopilot 2.11.0",
                "detail": [f"gpt-6-luna · {project} · {branch}", "/ for commands · @ to mention files"],
                "accent": "orange",
                "assistant_label": "autopilot",
            },
        )
        self.wait(20)
        self.send(
            "SlashCommandsRegistered",
            {"commands": [{"name": n, "description": d, "args_hint": h} for n, d, h in COMMANDS]},
        )
        self.wait(10)
        self.send(
            "MentionCandidatesRegistered",
            {"candidates": [{"token": m, "kind": "file"} for m in MENTIONS]},
        )
        self.wait(10)
        self.send(
            "StatusUpdate",
            {"segments": {"model": "gpt-6-luna", "tokens": "12.4k tokens", "cost": "$0.91"}},
        )
        self.wait(10)
        return self

    def thinking(self) -> "Rec":
        return self.send("StatusUpdate", {"segments": {"phase": "thinking"}})

    def idle(self) -> "Rec":
        return self.send("StatusUpdate", {"segments": {"phase": ""}})

    def tool(self, tid: str, name: str, command: str, ms: int = 900, **finish) -> "Rec":
        self.send(
            "ToolExecutionStarted",
            {"tool_id": tid, "tool": name, "command": command, "started_at_ms": self.t},
        )
        self.wait(ms)
        payload = {"tool_id": tid, "exit_code": finish.pop("exit_code", 0)}
        payload.update(finish)
        self.send("ToolExecutionFinished", payload)
        self.wait(30)
        return self

    def reply(self, sid: str, text: str, chunk: int = 40) -> "Rec":
        self.send("AssistantStreamStarted", {"stream_id": sid})
        for i in range(0, len(text), chunk):
            self.wait(12)
            self.send("AssistantTokenDelta", {"stream_id": sid, "token": text[i : i + chunk]})
        self.wait(20)
        self.send("AssistantMessageCompleted", {"stream_id": sid, "text": text})
        self.wait(20)
        return self


# ---------------------------------------------------------------------------
# Turns
# ---------------------------------------------------------------------------


def turn_long_tools() -> Rec:
    r = Rec().start()
    r.wait(40).user("audit the inline renderer for panics")
    r.wait(60).thinking()
    r.reply("s1", "I'll start by mapping the session state machine, then look for unwraps.")
    r.tool("t1", "Read", "crates/inline/src/session.rs", 1400, summary="4431 lines")
    r.tool("t2", "Grep", "unwrap\\(\\) (crates/inline/src)", 700, summary="38 matches")
    r.tool("t3", "Read", "crates/inline/src/blocks.rs", 900, summary="1185 lines")
    r.tool("t4", "Read", "crates/inline/src/chrome.rs", 800, summary="702 lines")
    r.tool("t5", "Grep", "\\[0\\]|\\[1\\] (crates/inline/src)", 600, summary="12 matches")
    r.reply("s2", "Most of the indexing is guarded. The risky spots are the prompt option lookups.")
    r.tool("t6", "Bash", "cargo test -p camouflage-inline --lib", 5200, summary="212 passed")
    r.tool("t7", "Edit", "crates/inline/src/session.rs", 400, summary="Updated session.rs · +6 −2")
    r.tool("t8", "Bash", "cargo clippy -p camouflage-inline -- -D warnings", 3100, summary="clean")
    r.reply("s3", "Done. Three option lookups now clamp instead of indexing, and the suite is green.")
    r.wait(80).idle()
    return r


def turn_long_output() -> Rec:
    r = Rec().start()
    r.wait(40).user("run the full test suite and show me the tail")
    r.wait(50).thinking()
    lines = [f"test result: ok. {i} passed; 0 failed; 0 ignored" for i in range(1, 60)]
    r.send(
        "ToolExecutionStarted",
        {"tool_id": "t1", "tool": "Bash", "command": "cargo test --workspace", "started_at_ms": r.t},
    )
    for i in range(0, len(lines), 6):
        r.wait(120)
        r.send("ToolExecutionStdout", {"tool_id": "t1", "chunk": "\n".join(lines[i : i + 6]) + "\n"})
    r.wait(400)
    r.send(
        "ToolExecutionFinished",
        {"tool_id": "t1", "exit_code": 0, "summary": "59 suites passed", "preview": 8},
    )
    r.wait(40)
    r.reply("s1", "All 59 suites pass. Nothing to fix.")
    r.wait(60).idle()
    return r


def turn_resize_narrow() -> Rec:
    r = Rec().start()
    r.wait(40).user("summarise the diff")
    r.wait(50).thinking()
    r.reply("s1", "The change folds a run of one tool into a single row so long code steps stop flooding the transcript.")
    r.tool("t1", "Read", "crates/inline/src/compact.rs", 700, summary="706 lines")
    # The host reports the terminal shrinking mid-turn.
    r.wait(30).resize(60, 24)
    r.tool("t2", "Edit", "crates/inline/src/compact.rs", 500, summary="Updated compact.rs · +18 −4")
    r.reply("s2", "Narrow terminals now wrap the summary instead of truncating it.")
    r.wait(60).idle()
    return r


def turn_resize_wide() -> Rec:
    r = Rec().start()
    r.wait(40).user("show the plan")
    r.wait(50).thinking()
    r.send(
        "TodoListUpdate",
        {
            "todos": [
                {"title": "Read the replay harness", "status": "completed"},
                {"title": "Add the soak assertion", "status": "in_progress"},
                {"title": "Add the fuzz target", "status": "pending"},
            ]
        },
    )
    r.wait(30).resize(160, 48)
    r.tool("t1", "Read", "crates/inline/tests/replay.rs", 600, summary="105 lines")
    r.reply("s1", "The plan is on track; the soak assertion is next.")
    r.wait(60).idle()
    return r


def turn_interrupt_esc() -> Rec:
    r = Rec().start()
    r.wait(40).user("rewrite the whole renderer")
    r.wait(50).thinking()
    r.reply("s1", "That's a large change; let me start with the block renderer.")
    r.send(
        "ToolExecutionStarted",
        {"tool_id": "t1", "tool": "Bash", "command": "cargo test --workspace", "started_at_ms": r.t},
    )
    r.wait(1200)
    # The user hits Esc: the host reports the cancel and settles.
    r.send("CancelRequested", {})
    r.wait(40)
    r.send("ToolExecutionFinished", {"tool_id": "t1", "status": "cancelled", "summary": "Stopped"})
    r.wait(60).idle()
    return r


def turn_interrupt_ctrlc() -> Rec:
    r = Rec().start()
    r.wait(40).user("keep going")
    r.wait(50).thinking()
    r.send("AssistantStreamStarted", {"stream_id": "s1"})
    r.wait(30).send("AssistantTokenDelta", {"stream_id": "s1", "token": "Working on the "})
    r.wait(30).send("AssistantTokenDelta", {"stream_id": "s1", "token": "second half of the "})
    r.wait(30).send("AssistantTokenDelta", {"stream_id": "s1", "token": "refactor now."})
    r.wait(200)
    r.send("CancelRequested", {})
    r.wait(40).idle()
    return r


def turn_error_retry() -> Rec:
    r = Rec().start()
    r.wait(40).user("fix the failing test")
    r.wait(50).thinking()
    r.tool("t1", "Bash", "cargo test -p camouflage-inline", 2400, exit_code=101, summary="Exit code 101")
    r.send(
        "RuntimeError",
        {
            "message": "upstream 529 · the model is overloaded",
            "severity": "warn",
            "cta": {"label": "retrying in 4s"},
        },
    )
    r.wait(4000)
    r.thinking()
    r.tool("t2", "Bash", "cargo test -p camouflage-inline", 2100, summary="212 passed")
    r.reply("s1", "The retry succeeded; the suite is green again.")
    r.wait(60).idle()
    return r


def turn_plan_progress() -> Rec:
    r = Rec().start()
    r.wait(40).user("add the fuzz target")
    r.wait(50).thinking()
    steps = [
        ("Survey existing fuzz setup", "in_progress"),
        ("Write the apply target", "pending"),
        ("Write the key target", "pending"),
        ("Wire it into CI", "pending"),
    ]
    r.send("TodoListUpdate", {"todos": [{"title": t, "status": s} for t, s in steps]})
    r.wait(30)
    r.tool("t1", "Grep", "cargo-fuzz (.)", 500, summary="no matches")
    steps[0] = (steps[0][0], "completed")
    steps[1] = (steps[1][0], "in_progress")
    r.send("TodoListUpdate", {"todos": [{"title": t, "status": s} for t, s in steps]})
    r.wait(30)
    r.tool("t2", "Write", "crates/inline/fuzz/fuzz_targets/session_apply.rs", 300, summary="Created")
    steps[1] = (steps[1][0], "completed")
    steps[2] = (steps[2][0], "in_progress")
    r.send("TodoListUpdate", {"todos": [{"title": t, "status": s} for t, s in steps]})
    r.wait(30)
    r.reply("s1", "Both targets are in place; CI wiring is next.")
    r.wait(60).idle()
    return r


def turn_reasoning() -> Rec:
    r = Rec().start()
    r.wait(40).user("why does the footer flicker?")
    r.wait(50).thinking()
    r.send("AssistantStreamStarted", {"stream_id": "s1"})
    for tok in [
        "The footer is redrawn every frame ",
        "because status_until keeps it visible ",
        "for four seconds after a change. ",
        "The flicker is the spinner frame index ",
        "advancing while the footer is static.",
    ]:
        r.wait(25).send("AssistantReasoningDelta", {"stream_id": "s1", "token": tok})
    r.wait(30)
    r.reply("s1", "The footer redraws on every spinner tick. Freezing the frame index while idle removes the flicker.")
    r.wait(60).idle()
    return r


def turn_reasoning_expanded() -> Rec:
    r = Rec().start()
    r.wait(40).user("why does the footer flicker?")
    r.wait(50).thinking()
    r.send("AssistantStreamStarted", {"stream_id": "s1"})
    for tok in [
        "The footer is redrawn every frame ",
        "because status_until keeps it visible ",
        "for four seconds after a change.",
    ]:
        r.wait(25).send("AssistantReasoningDelta", {"stream_id": "s1", "token": tok})
    r.wait(30)
    # Ctrl+R shows the reasoning above the reply, and keeps it in history.
    r.key("ctrl+r")
    r.wait(20).snapshot("reasoning shown")
    r.reply("s1", "The footer redraws on every spinner tick; freezing the frame index while idle fixes it.")
    r.wait(60).idle()
    return r


def turn_tools_expanded() -> Rec:
    r = Rec().start()
    r.wait(40).user("run the suite")
    r.wait(50).thinking()
    r.send(
        "ToolExecutionStarted",
        {"tool_id": "t1", "tool": "Bash", "command": "cargo test --workspace", "started_at_ms": r.t},
    )
    r.wait(200)
    r.send("ToolExecutionStdout", {"tool_id": "t1", "chunk": "running 212 tests\n"})
    r.wait(200)
    # Ctrl+O swaps the compact rows for the full tool rows.
    r.key("ctrl+o")
    r.wait(20).snapshot("tool output expanded")
    r.key("ctrl+o")
    r.wait(200)
    r.send(
        "ToolExecutionFinished",
        {"tool_id": "t1", "exit_code": 0, "summary": "212 passed", "preview": 2},
    )
    r.wait(40)
    r.reply("s1", "All 212 tests pass.")
    r.wait(60).idle()
    return r


def turn_shortcuts() -> Rec:
    r = Rec().start()
    r.wait(40).user("what can I press?")
    r.wait(50).thinking()
    r.reply("s1", "Press ? for the full list.")
    r.wait(40).idle()
    r.wait(200)
    r.key("?")
    r.wait(20).snapshot("shortcuts panel")
    r.key("ctrl+u")
    r.wait(40)
    return r


def turn_markdown() -> Rec:
    r = Rec().start()
    r.wait(40).user("write up the fixture plan")
    r.wait(50).thinking()
    text = """\
## Fixture plan

The replay harness already diffs the final screen, so the corpus only needs
**coverage**, not cleverness.

- long turns with many tools
- resizes mid-turn
- interrupts from both keys
- every picker and form
- permission prompts with diffs
- `/clear`

### Budget

| Area | Fixtures | Notes |
| --- | --- | --- |
| turns | 20 | tools, streaming, errors |
| pickers | 9 | one per command |
| forms | 3 | text, multiline, password |

```rust
#[test]
fn recorded_sessions_render_as_before() {
    // every fixture, every run
}
```

> A renderer change that alters any of it fails here first.
"""
    r.reply("s1", text)
    r.wait(60).idle()
    return r


def turn_streaming_long() -> Rec:
    r = Rec().start()
    r.wait(40).user("explain the compact region")
    r.wait(50).thinking()
    paragraphs = [
        "The compact region replaces the tool rows and the plan while a turn is running. ",
        "Each tool contributes one bead, so a long run of reads stays a single line. ",
        "When the turn settles the region is cleared and the finished blocks print once. ",
        "That keeps the live frame a fixed height no matter how long the turn gets. ",
    ]
    r.send("AssistantStreamStarted", {"stream_id": "s1"})
    for p in paragraphs:
        for i in range(0, len(p), 24):
            r.wait(18).send("AssistantTokenDelta", {"stream_id": "s1", "token": p[i : i + 24]})
        r.wait(40)
    r.wait(60)
    r.send("AssistantMessageCompleted", {"stream_id": "s1", "text": "".join(paragraphs)})
    r.wait(60).idle()
    return r


def turn_background_tasks() -> Rec:
    r = Rec().start()
    r.wait(40).user("index the repo and run the soak")
    r.wait(50).thinking()
    r.send(
        "BackgroundTaskUpdate",
        {"task_id": "skills", "label": "indexing skills", "state": "running", "progress": 0.0},
    )
    r.wait(200)
    r.send(
        "BackgroundTaskUpdate",
        {"task_id": "soak", "label": "soak 10m", "state": "running", "progress": 0.1},
    )
    r.wait(400)
    r.send(
        "BackgroundTaskUpdate",
        {"task_id": "skills", "label": "indexing skills", "state": "done", "progress": 1.0},
    )
    r.wait(300)
    r.send(
        "BackgroundTaskUpdate",
        {"task_id": "soak", "label": "soak 10m", "state": "error", "progress": 0.4},
    )
    r.wait(40)
    r.reply("s1", "Skills indexed. The soak failed its RSS cap at 214MB.")
    r.wait(60).idle()
    return r


def turn_agents_browser() -> Rec:
    r = Rec().start()
    r.wait(40).user("fan out the review across three agents")
    r.wait(50).thinking()
    for i, title in enumerate(["review session.rs", "review blocks.rs", "review chrome.rs"], 1):
        r.send(
            "ActivityUpdate",
            {
                "id": f"agent-{i}",
                "kind": "agent",
                "title": title,
                "status": "running",
                "stoppable": True,
                "started_at_ms": r.t,
            },
        )
        r.wait(60)
    r.wait(400)
    r.send("ActivityBrowserOpen", {})
    r.wait(200)
    for i in range(1, 4):
        r.send(
            "ActivityUpdate",
            {
                "id": f"agent-{i}",
                "kind": "agent",
                "status": "done",
                "summary": "no findings",
                "progress": 1.0,
            },
        )
        r.wait(80)
    r.wait(60).idle()
    return r


def turn_terminal_handoff() -> Rec:
    r = Rec().start()
    r.wait(40).user("! git status --short")
    r.wait(30)
    r.send("TerminalSuspend", {})
    r.wait(1200)
    r.send("TerminalResume", {})
    r.wait(30)
    r.send(
        "ToolExecutionStarted",
        {"tool_id": "t1", "tool": "Bash", "command": "git status --short", "started_at_ms": r.t},
    )
    r.wait(300)
    r.send(
        "ToolExecutionFinished",
        {
            "tool_id": "t1",
            "exit_code": 0,
            "summary": "3 files changed",
            "output": " M crates/inline/src/session.rs\n M crates/inline/tests/replay.rs\n?? fixtures/inline/turn-long-tools.ndjson\n",
            "preview": 4,
        },
    )
    r.wait(40)
    r.reply("s1", "Three files changed, one new fixture.")
    r.wait(60).idle()
    return r


def turn_cost_status() -> Rec:
    r = Rec().start()
    r.wait(40).user("/cost")
    r.wait(30)
    r.send(
        "ShowKeyValueView",
        {
            "title": "Usage",
            "items": [
                {"label": "Model", "value": "gpt-6-luna"},
                {"label": "Input", "value": "44.1k tokens"},
                {"label": "Output", "value": "9.8k tokens"},
                {"label": "Cost", "value": "$3.43"},
            ],
        },
    )
    r.wait(40)
    r.send(
        "StatusUpdate",
        {"segments": {"model": "gpt-6-luna", "tokens": "53.9k tokens", "cost": "$3.43"}},
    )
    r.wait(60).idle()
    return r


def turn_mentions() -> Rec:
    r = Rec().start()
    r.wait(40).user("look at @crates/inline/src/session.rs")
    r.wait(30)
    r.send(
        "MentionCandidatesRegistered",
        {
            "candidates": [
                {"token": "crates/inline/src/session.rs", "kind": "file"},
                {"token": "crates/inline/src/blocks.rs", "kind": "file"},
                {"token": "crates/inline/src/chrome.rs", "kind": "file"},
            ]
        },
    )
    r.wait(30).thinking()
    r.tool("t1", "Read", "crates/inline/src/session.rs", 900, summary="4431 lines")
    r.reply("s1", "The session state machine is 4,431 lines; the prompt paths are the interesting part.")
    r.wait(60).idle()
    return r


def turn_compacted() -> Rec:
    r = Rec().start()
    r.wait(40).user("keep going")
    r.wait(50).thinking()
    r.reply("s1", "Continuing from the compacted summary.")
    r.wait(30)
    r.send("SessionCompacted", {})
    r.wait(40)
    r.tool("t1", "Read", "docs/inline-redesign.md", 600, summary="318 lines")
    r.reply("s2", "The redesign notes still match the implementation.")
    r.wait(60).idle()
    return r


def turn_toast() -> Rec:
    r = Rec().start()
    r.wait(40).user("switch to plan mode")
    r.wait(30)
    r.send("ShowToast", {"text": "Plan mode", "kind": "success", "ttl_ms": 2500})
    r.wait(30)
    r.send("StatusUpdate", {"segments": {"mode": "plan"}})
    r.wait(400)
    r.send("ShowToast", {"text": "No background jobs or agents", "kind": "warn", "ttl_ms": 2500})
    r.wait(60).idle()
    return r


def turn_table_and_kv() -> Rec:
    r = Rec().start()
    r.wait(40).user("/sessions")
    r.wait(30)
    r.send(
        "ShowTable",
        {
            "title": "Recent sessions",
            "columns": [
                {"name": "id", "label": "ID"},
                {"name": "title", "label": "Title"},
                {"name": "updated", "label": "Updated"},
            ],
            "rows": [
                {"id": "a1f2", "title": "inline replay fixtures", "updated": "2m ago"},
                {"id": "b7c9", "title": "compact region beads", "updated": "1h ago"},
                {"id": "d3e4", "title": "fuzz target", "updated": "yesterday"},
            ],
        },
    )
    r.wait(40)
    r.send(
        "ShowKeyValueView",
        {
            "title": "Session a1f2",
            "items": [
                {"label": "Model", "value": "gpt-6-luna"},
                {"label": "Branch", "value": "main"},
                {"label": "Turns", "value": "18"},
            ],
        },
    )
    r.wait(60).idle()
    return r


def turn_patch_proposed() -> Rec:
    r = Rec().start()
    r.wait(40).user("guard the event type")
    r.wait(50).thinking()
    r.send(
        "PatchProposed",
        {"path": "crates/inline/src/session.rs", "diff": DIFF_UNIFIED, "added": 6, "removed": 2},
    )
    r.wait(40)
    r.reply("s1", "Proposed a guard so a malformed event type returns early instead of panicking.")
    r.wait(60).idle()
    return r


# ---------------------------------------------------------------------------
# Pickers
# ---------------------------------------------------------------------------


def picker_help() -> Rec:
    r = Rec().start()
    r.wait(40).user("/help")
    r.wait(30)
    r.send(
        "ShowSelectList",
        {
            "id": "help",
            "prompt": "Commands",
            "subtitle": "Type to filter · Enter to run",
            "options": [
                {"value": n, "label": f"/{n}", "description": d}
                for n, d, _ in COMMANDS
            ],
            "allow_cancel": True,
            "allow_filter": True,
        },
    )
    r.wait(200)
    r.answer("SelectListResponse", {"id": "help", "value": "help"})
    r.wait(60).idle()
    return r


def picker_model() -> Rec:
    r = Rec().start()
    r.wait(40).user("/model")
    r.wait(30)
    r.send(
        "ShowSelectList",
        {
            "id": "model",
            "prompt": "Switch model",
            "subtitle": "Current: gpt-6-luna",
            "options": [
                {"value": "gpt-6-luna", "label": "gpt-6-luna", "description": "fast · 400k context", "section": "OpenAI"},
                {"value": "gpt-6-sol", "label": "gpt-6-sol", "description": "deep reasoning · 400k context", "section": "OpenAI"},
                {"value": "claude-opus-5", "label": "claude-opus-5", "description": "long turns · 1M context", "section": "Anthropic"},
                {"value": "claude-sonnet-5", "label": "claude-sonnet-5", "description": "balanced · 1M context", "section": "Anthropic"},
                {"value": "deepseek-v4", "label": "deepseek-v4", "description": "cheap · 128k context", "section": "DeepSeek"},
            ],
            "default": "gpt-6-luna",
            "allow_cancel": True,
            "allow_filter": True,
        },
    )
    r.wait(200)
    r.answer("SelectListResponse", {"id": "model", "value": "gpt-6-sol"})
    r.wait(30)
    r.send("StatusUpdate", {"segments": {"model": "gpt-6-sol"}})
    r.wait(60).idle()
    return r


def picker_hooks() -> Rec:
    r = Rec().start()
    r.wait(40).user("/hooks")
    r.wait(30)
    r.send(
        "ShowSelectList",
        {
            "id": "hooks",
            "prompt": "Hooks",
            "subtitle": "Run on every turn",
            "options": [
                {"value": "pre-tool", "label": "pre-tool", "description": "scripts/pre-tool.sh", "state": "on"},
                {"value": "post-edit", "label": "post-edit", "description": "scripts/fmt.sh", "state": "on"},
                {"value": "stop", "label": "stop", "description": "scripts/notify.sh", "state": "off"},
            ],
            "allow_cancel": True,
            "allow_filter": True,
        },
    )
    r.wait(200)
    r.answer("SelectListResponse", {"id": "hooks", "value": "stop"})
    r.wait(60).idle()
    return r


def picker_sessions() -> Rec:
    r = Rec().start()
    r.wait(40).user("/sessions")
    r.wait(30)
    r.send(
        "ShowSelectList",
        {
            "id": "sessions",
            "prompt": "Resume a session",
            "options": [
                {
                    "value": "a1f2",
                    "label": "inline replay fixtures",
                    "description": "2m ago",
                    "columns": ["18 turns", "main"],
                },
                {
                    "value": "b7c9",
                    "label": "compact region beads",
                    "description": "1h ago",
                    "columns": ["42 turns", "feat/compact"],
                },
                {
                    "value": "d3e4",
                    "label": "fuzz target",
                    "description": "yesterday",
                    "columns": ["7 turns", "main"],
                },
            ],
            "allow_cancel": True,
            "allow_filter": True,
        },
    )
    r.wait(200)
    r.answer("SelectListResponse", {"id": "sessions", "value": "a1f2"})
    r.wait(60).idle()
    return r


def picker_theme() -> Rec:
    r = Rec().start()
    r.wait(40).user("/theme")
    r.wait(30)
    r.send(
        "ShowSelectList",
        {
            "id": "theme",
            "prompt": "Colour theme",
            "options": [
                {"value": "dark", "label": "dark", "description": "default", "state": "on"},
                {"value": "light", "label": "light", "description": "for bright terminals", "state": "off"},
                {"value": "ansi", "label": "ansi", "description": "use the terminal palette", "state": "off"},
                {"value": "mono", "label": "mono", "description": "no colour", "state": "off"},
            ],
            "default": "dark",
            "allow_cancel": True,
            "allow_filter": True,
        },
    )
    r.wait(200)
    r.answer("SelectListResponse", {"id": "theme", "value": "ansi"})
    r.wait(60).idle()
    return r


def picker_agents() -> Rec:
    r = Rec().start()
    r.wait(40).user("/agents")
    r.wait(30)
    r.send(
        "ShowSelectList",
        {
            "id": "agents",
            "prompt": "Agents",
            "subtitle": "3 running",
            "options": [
                {"value": "agent-1", "label": "review session.rs", "description": "running · 2m", "columns": ["12 tools"]},
                {"value": "agent-2", "label": "review blocks.rs", "description": "running · 1m", "columns": ["8 tools"]},
                {"value": "agent-3", "label": "review chrome.rs", "description": "running · 40s", "columns": ["3 tools"]},
            ],
            "allow_cancel": True,
            "allow_filter": True,
        },
    )
    r.wait(200)
    r.answer("SelectListResponse", {"id": "agents", "value": "agent-1"})
    r.wait(60).idle()
    return r


def picker_mcp() -> Rec:
    r = Rec().start()
    r.wait(40).user("/mcp")
    r.wait(30)
    r.send(
        "ShowSelectList",
        {
            "id": "mcp",
            "prompt": "MCP servers",
            "options": [
                {"value": "github", "label": "github", "description": "connected · 14 tools", "section": "Connected", "state": "on"},
                {"value": "linear", "label": "linear", "description": "connected · 6 tools", "section": "Connected", "state": "on"},
                {"value": "sentry", "label": "sentry", "description": "not configured", "section": "Available", "state": "off"},
            ],
            "allow_cancel": True,
            "allow_filter": True,
        },
    )
    r.wait(200)
    r.answer("SelectListResponse", {"id": "mcp", "value": "sentry"})
    r.wait(60).idle()
    return r


def picker_permissions() -> Rec:
    r = Rec().start()
    r.wait(40).user("/permissions")
    r.wait(30)
    r.send(
        "ShowSelectList",
        {
            "id": "permissions",
            "prompt": "Allowed tools",
            "subtitle": "This session",
            "options": [
                {"value": "Read", "label": "Read", "description": "always", "state": "on"},
                {"value": "Grep", "label": "Grep", "description": "always", "state": "on"},
                {"value": "Bash", "label": "Bash", "description": "ask every time", "state": "off"},
                {"value": "Edit", "label": "Edit", "description": "ask every time", "state": "off"},
            ],
            "allow_cancel": True,
            "allow_filter": True,
        },
    )
    r.wait(200)
    r.answer("SelectListResponse", {"id": "permissions", "value": "Bash"})
    r.wait(60).idle()
    return r


def picker_filter() -> Rec:
    r = Rec().start()
    r.wait(40).user("/model")
    r.wait(30)
    r.send(
        "ShowSelectList",
        {
            "id": "model",
            "prompt": "Switch model",
            "options": [
                {"value": "gpt-6-luna", "label": "gpt-6-luna", "description": "fast"},
                {"value": "gpt-6-sol", "label": "gpt-6-sol", "description": "deep reasoning"},
                {"value": "claude-opus-5", "label": "claude-opus-5", "description": "long turns"},
                {"value": "claude-sonnet-5", "label": "claude-sonnet-5", "description": "balanced"},
                {"value": "deepseek-v4", "label": "deepseek-v4", "description": "cheap"},
            ],
            "allow_cancel": True,
            "allow_filter": True,
        },
    )
    r.wait(200)
    r.answer("SelectListResponse", {"id": "model", "value": "claude-opus-5", "filter": "claude"})
    r.wait(60).idle()
    return r


# ---------------------------------------------------------------------------
# Forms and wizards
# ---------------------------------------------------------------------------


def form_settings() -> Rec:
    r = Rec().start()
    r.wait(40).user("/settings")
    r.wait(30)
    r.send(
        "ShowForm",
        {
            "id": "settings",
            "title": "Settings",
            "fields": [
                {"name": "model", "label": "Model", "kind": "text", "default": "gpt-6-luna", "required": True},
                {"name": "temperature", "label": "Temperature", "kind": "text", "placeholder": "0.2"},
                {"name": "max_tokens", "label": "Max output tokens", "kind": "text", "placeholder": "8192"},
            ],
            "allow_cancel": True,
        },
    )
    r.wait(300)
    r.answer("FormResponse", {"id": "settings", "values": {"model": "gpt-6-luna"}})
    r.wait(60).idle()
    return r


def form_multiline() -> Rec:
    r = Rec().start()
    r.wait(40).user("/settings")
    r.wait(30)
    r.send(
        "ShowForm",
        {
            "id": "settings",
            "title": "System prompt",
            "fields": [
                {
                    "name": "prompt",
                    "label": "Prompt",
                    "kind": "multiline",
                    "default": "You are autopilot.\nBe concise.",
                    "required": True,
                },
                {"name": "note", "label": "Note", "kind": "text", "placeholder": "optional"},
            ],
            "allow_cancel": True,
        },
    )
    r.wait(300)
    r.answer("FormResponse", {"id": "settings", "values": {"prompt": "You are autopilot."}})
    r.wait(60).idle()
    return r


def form_password() -> Rec:
    r = Rec().start()
    r.wait(40).user("/mcp")
    r.wait(30)
    r.send(
        "ShowForm",
        {
            "id": "mcp-auth",
            "title": "Connect sentry",
            "fields": [
                {"name": "token", "label": "API token", "kind": "password", "required": True},
                {"name": "org", "label": "Organisation", "kind": "text", "placeholder": "acme"},
            ],
            "allow_cancel": True,
        },
    )
    r.wait(300)
    r.answer("FormResponse", {"id": "mcp-auth", "values": {"token": "***", "org": "acme"}})
    r.wait(60).idle()
    return r


def wizard_setup() -> Rec:
    r = Rec().start()
    r.wait(40).user("/init")
    r.wait(30)
    r.send(
        "ShowWizard",
        {
            "id": "init",
            "title": "Set up autopilot",
            "allow_cancel": True,
            "steps": [
                {
                    "kind": "select",
                    "id": "init-model",
                    "prompt": "Which model?",
                    "options": [
                        {"value": "gpt-6-luna", "label": "gpt-6-luna", "description": "fast"},
                        {"value": "claude-opus-5", "label": "claude-opus-5", "description": "long turns"},
                    ],
                },
                {
                    "kind": "confirm",
                    "id": "init-hooks",
                    "prompt": "Install the format-on-save hook?",
                    "yes_label": "Install",
                    "no_label": "Skip",
                },
                {
                    "kind": "form",
                    "id": "init-name",
                    "title": "Project name",
                    "fields": [
                        {"name": "name", "label": "Name", "kind": "text", "default": "camouflage", "required": True}
                    ],
                },
            ],
        },
    )
    r.wait(300)
    r.answer("SelectListResponse", {"id": "init-model", "value": "gpt-6-luna"})
    r.wait(200)
    r.answer("ConfirmResponse", {"id": "init-hooks", "value": True})
    r.wait(200)
    r.answer("FormResponse", {"id": "init-name", "values": {"name": "camouflage"}})
    r.wait(60).idle()
    return r


def wizard_cancel() -> Rec:
    r = Rec().start()
    r.wait(40).user("/init")
    r.wait(30)
    r.send(
        "ShowWizard",
        {
            "id": "init",
            "title": "Set up autopilot",
            "allow_cancel": True,
            "steps": [
                {
                    "kind": "select",
                    "id": "init-model",
                    "prompt": "Which model?",
                    "options": [
                        {"value": "gpt-6-luna", "label": "gpt-6-luna"},
                        {"value": "claude-opus-5", "label": "claude-opus-5"},
                    ],
                },
                {"kind": "confirm", "id": "init-hooks", "prompt": "Install the hook?"},
            ],
        },
    )
    r.wait(300)
    r.answer("WizardCancelled", {"id": "init"})
    r.wait(60).idle()
    return r


# ---------------------------------------------------------------------------
# Permissions
# ---------------------------------------------------------------------------


def permission_diff() -> Rec:
    r = Rec().start()
    r.wait(40).user("guard the event type")
    r.wait(50).thinking()
    r.send(
        "PermissionRequested",
        {
            "request_id": "req-1",
            "tool": "Edit",
            "action": "edit crates/inline/src/session.rs",
            "detail": "crates/inline/src/session.rs",
            "diff": {"path": "crates/inline/src/session.rs", "unified": DIFF_UNIFIED},
        },
    )
    r.wait(600)
    r.answer("PermissionResponse", {"request_id": "req-1", "choice": "allow_once"})
    r.wait(40)
    r.tool("t1", "Edit", "crates/inline/src/session.rs", 400, summary="Updated session.rs · +6 −2")
    r.reply("s1", "Applied the guard.")
    r.wait(60).idle()
    return r


def permission_plain() -> Rec:
    r = Rec().start()
    r.wait(40).user("run the tests")
    r.wait(50).thinking()
    r.send(
        "PermissionRequested",
        {
            "request_id": "req-2",
            "tool": "Bash",
            "action": "run cargo test --workspace",
            "detail": "cargo test --workspace",
        },
    )
    r.wait(600)
    r.answer("PermissionResponse", {"request_id": "req-2", "choice": "allow_session"})
    r.wait(40)
    r.tool("t1", "Bash", "cargo test --workspace", 2400, summary="59 suites passed")
    r.reply("s1", "Everything passes.")
    r.wait(60).idle()
    return r


def permission_deny() -> Rec:
    r = Rec().start()
    r.wait(40).user("delete the build directory")
    r.wait(50).thinking()
    r.send(
        "PermissionRequested",
        {
            "request_id": "req-3",
            "tool": "Bash",
            "action": "run rm -rf target",
            "detail": "rm -rf target",
        },
    )
    r.wait(600)
    r.answer("PermissionResponse", {"request_id": "req-3", "choice": "deny"})
    r.wait(40)
    r.send("CancelRequested", {})
    r.wait(40)
    r.reply("s1", "Understood — I won't touch the build directory.")
    r.wait(60).idle()
    return r


# ---------------------------------------------------------------------------
# /clear
# ---------------------------------------------------------------------------


def clear_transcript() -> Rec:
    r = Rec().start()
    r.wait(40).user("first turn")
    r.wait(50).thinking()
    r.reply("s1", "First answer, which /clear will wipe.")
    r.wait(40)
    r.tool("t1", "Read", "README.md", 500, summary="212 lines")
    r.wait(40).idle()
    r.wait(200).user("/clear")
    r.wait(30)
    r.send("TranscriptCleared", {})
    r.wait(40)
    r.user("second turn")
    r.wait(50).thinking()
    r.reply("s2", "Second answer, after the clear.")
    r.wait(60).idle()
    return r


SCENARIOS = {
    "turn-long-tools": turn_long_tools,
    "turn-long-output": turn_long_output,
    "turn-resize-narrow": turn_resize_narrow,
    "turn-resize-wide": turn_resize_wide,
    "turn-interrupt-esc": turn_interrupt_esc,
    "turn-interrupt-ctrlc": turn_interrupt_ctrlc,
    "turn-error-retry": turn_error_retry,
    "turn-plan-progress": turn_plan_progress,
    "turn-reasoning": turn_reasoning,
    "turn-reasoning-expanded": turn_reasoning_expanded,
    "turn-tools-expanded": turn_tools_expanded,
    "turn-shortcuts": turn_shortcuts,
    "turn-markdown": turn_markdown,
    "turn-streaming-long": turn_streaming_long,
    "turn-background-tasks": turn_background_tasks,
    "turn-agents-browser": turn_agents_browser,
    "turn-terminal-handoff": turn_terminal_handoff,
    "turn-cost-status": turn_cost_status,
    "turn-mentions": turn_mentions,
    "turn-compacted": turn_compacted,
    "turn-toast": turn_toast,
    "turn-table-and-kv": turn_table_and_kv,
    "turn-patch-proposed": turn_patch_proposed,
    "picker-help": picker_help,
    "picker-model": picker_model,
    "picker-hooks": picker_hooks,
    "picker-sessions": picker_sessions,
    "picker-theme": picker_theme,
    "picker-agents": picker_agents,
    "picker-mcp": picker_mcp,
    "picker-permissions": picker_permissions,
    "picker-filter": picker_filter,
    "form-settings": form_settings,
    "form-multiline": form_multiline,
    "form-password": form_password,
    "wizard-setup": wizard_setup,
    "wizard-cancel": wizard_cancel,
    "permission-diff": permission_diff,
    "permission-plain": permission_plain,
    "permission-deny": permission_deny,
    "clear-transcript": clear_transcript,
}


def render(name: str) -> str:
    lines = SCENARIOS[name]().lines
    return "".join(json.dumps(line, separators=(",", ":")) + "\n" for line in lines)


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--check", action="store_true", help="fail if a fixture is stale")
    args = p.parse_args()

    stale = []
    for name in SCENARIOS:
        path = OUT / f"{name}.ndjson"
        body = render(name)
        if args.check:
            if not path.exists() or path.read_text() != body:
                stale.append(path.name)
        else:
            path.write_text(body)
    if stale:
        print("stale fixtures (rerun without --check): " + ", ".join(stale), file=sys.stderr)
        return 1
    if not args.check:
        print(f"wrote {len(SCENARIOS)} fixtures to {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
